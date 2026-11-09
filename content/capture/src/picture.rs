//! The picture itself: pixels, and what can be done with them.

use std::fmt;

use aimer_platform::capture::Frame;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::imageops::FilterType;
use image::{ExtendedColorType, ImageEncoder, RgbaImage};

/// A picture: red, green, blue and alpha bytes, rows packed, top row first.
///
/// It remembers its `scale` — how many of its pixels make a logical pixel of the
/// app that drew it — so a point in the app's own coordinates can be found in it,
/// and still can after it has been cropped or shrunk.
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    width: u32,
    height: u32,
    scale: f32,
    rgba: Vec<u8>,
}

impl Image {
    /// Wraps `rgba`, which must hold exactly `width * height` pixels of four
    /// bytes. `None` for an empty picture, a scale that is not positive and
    /// finite, or the wrong number of bytes.
    pub fn new(width: u32, height: u32, scale: f32, rgba: Vec<u8>) -> Option<Self> {
        let size = (width as usize).checked_mul(height as usize)?.checked_mul(4)?;
        (width > 0 && height > 0 && scale.is_finite() && scale > 0.0 && rgba.len() == size)
            .then_some(Self { width, height, scale, rgba })
    }

    /// The picture a captured frame holds, with its channels in red, green, blue,
    /// alpha order whichever order the surface stored them in.
    ///
    /// # Panics
    ///
    /// If the frame does not hold `width * height` pixels, which a renderer
    /// never produces.
    pub fn from_frame(frame: &Frame) -> Self {
        let mut rgba = frame.pixels.clone();
        if frame.bgra {
            for pixel in rgba.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
        }
        Self::new(frame.width, frame.height, frame.scale, rgba).expect("a frame holds width x height pixels")
    }

    /// Width in pixels.
    #[inline]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    #[inline]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Pixels per logical pixel of the app that drew it.
    #[inline]
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// The pixels: `width * height * 4` bytes.
    #[inline]
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    /// The part of the picture inside the `width` by `height` pixel rectangle whose
    /// top-left corner is `(x, y)`. A rectangle that hangs over an edge is trimmed
    /// to it; one wholly outside, or empty, is `None`.
    pub fn crop(&self, x: u32, y: u32, width: u32, height: u32) -> Option<Self> {
        if x >= self.width || y >= self.height || width == 0 || height == 0 {
            return None;
        }
        let (width, height) = (width.min(self.width - x), height.min(self.height - y));
        let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
        for row in y..y + height {
            let start = ((row * self.width + x) * 4) as usize;
            rgba.extend_from_slice(&self.rgba[start..start + width as usize * 4]);
        }
        Self::new(width, height, self.scale, rgba)
    }

    /// [`Image::crop`] for a rectangle in the app's logical pixels, taken at this
    /// picture's scale and rounded outwards so nothing inside it is lost. Parts
    /// that fall off the picture are trimmed.
    pub fn crop_logical(&self, x: f32, y: f32, width: f32, height: f32) -> Option<Self> {
        if ![x, y, width, height].iter().all(|v| v.is_finite()) || width <= 0.0 || height <= 0.0 {
            return None;
        }
        let (left, top) = ((x * self.scale).floor().max(0.0), (y * self.scale).floor().max(0.0));
        let (right, bottom) = (((x + width) * self.scale).ceil(), ((y + height) * self.scale).ceil());
        if right <= 0.0 || bottom <= 0.0 || right <= left || bottom <= top {
            return None;
        }
        self.crop(left as u32, top as u32, (right - left) as u32, (bottom - top) as u32)
    }

    /// The picture shrunk so it is at most `max_width` pixels wide, keeping its
    /// shape; its scale shrinks with it. One that is narrow enough already, or a
    /// `max_width` of zero, is returned unchanged: a picture is never enlarged.
    pub fn scaled_to_width(&self, max_width: u32) -> Self {
        if max_width == 0 || self.width <= max_width {
            return self.clone();
        }
        let height = ((self.height as f64 * max_width as f64 / self.width as f64).round() as u32).max(1);
        let source = RgbaImage::from_raw(self.width, self.height, self.rgba.clone()).expect("the size matches the bytes");
        // Catmull-Rom keeps the edges of text and controls sharper than a plain
        // average does, which is what a viewer of a screenshot needs.
        let small = image::imageops::resize(&source, max_width, height, FilterType::CatmullRom);
        Self {
            width: max_width,
            height,
            scale: self.scale * max_width as f32 / self.width as f32,
            rgba: small.into_raw(),
        }
    }

    /// The picture encoded as `format`.
    pub fn encode(&self, format: Format) -> Result<Vec<u8>, EncodeError> {
        let mut out = Vec::new();
        let encoded = match format {
            Format::Png => PngEncoder::new(&mut out).write_image(&self.rgba, self.width, self.height, ExtendedColorType::Rgba8),
            Format::Jpeg { quality } => {
                // JPEG has no alpha: the colour channels are kept and alpha dropped.
                let rgb: Vec<u8> = self.rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
                JpegEncoder::new_with_quality(&mut out, quality.clamp(1, 100)).write_image(
                    &rgb,
                    self.width,
                    self.height,
                    ExtendedColorType::Rgb8,
                )
            }
        };
        encoded.map_err(|e| EncodeError(e.to_string()))?;
        Ok(out)
    }
}

/// How a picture is written out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Lossless, with alpha. The default: exact, but large for a busy window.
    Png,
    /// Lossy and much smaller, without alpha. `quality` is 1 to 100, and is kept
    /// in that range.
    Jpeg {
        /// How faithful, from 1 (smallest) to 100 (largest).
        quality: u8,
    },
}

impl Default for Format {
    fn default() -> Self {
        Self::Png
    }
}

impl Format {
    /// The MIME type of what this produces.
    pub fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg { .. } => "image/jpeg",
        }
    }
}

/// The encoder refused the picture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodeError(String);

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "could not encode the picture: {}", self.0)
    }
}

impl std::error::Error for EncodeError {}

#[cfg(test)]
#[path = "picture_tests.rs"]
mod tests;
