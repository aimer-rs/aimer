//! Glyph rasterization performed by the system on Apple platforms.
//!
//! Cupid rasterizes glyphs itself: outlines come from `glyf` or `CFF `, color
//! artwork from `sbix`, `CBDT` or `COLR`. Apple's own system fonts do not
//! always use those formats. Two of them are load bearing on iOS:
//!
//! * `PingFangUI.ttc` — the only face on the system covering Simplified
//!   Chinese — stores its outlines in a private `hvgl` table and ships neither
//!   `glyf` nor `CFF `;
//! * `AppleColorEmoji-160px.ttc` stores its `sbix` strikes as `emjc`, Apple's
//!   compressed emoji format, where the macOS build of the same font uses
//!   plain `png `.
//!
//! Every table needed to *shape* text is present in both — `cmap`, `hmtx`,
//! `GSUB`, `morx` — only the pixels are unreachable, so shaping, measurement
//! and layout stay with Cupid and just the final rasterization is handed to
//! Core Text, which is the only code that can read these formats.
//!
//! # Glyph identity
//!
//! A [`RasterizedGlyph`] is addressed by glyph id, and glyph ids are per face:
//! asking Core Text for the wrong face of a collection would draw a different
//! character. The face is therefore selected by PostScript name — read from
//! the very face Cupid shaped with — and the collection index is used only
//! when the file exposes no name to match on.
//!
//! # Weight
//!
//! Apple's variable system faces default to a heavier instance than the one
//! their own cascade pairs with regular text, so a variable face is pinned
//! before drawing — to the weight the caller's glyph key asks for, which the
//! fallback pipeline derives from the run's style and from the faces standing
//! beside the glyph. [`NORMAL_TEXT_WEIGHT`] is the neutral request.
//!
//! # Cost
//!
//! Building a `CTFont` parses the file, so fonts are cached per
//! `(file, face, size, weight)`. The cache is thread local, which keeps it
//! lock free and matches how text preparation runs: each worker owns its
//! rasterizer.
//! Everything downstream — the atlas, the bitmap cache, the metrics store —
//! is unaware that the pixels came from the platform.
//!
//! This module is compiled only with the `apple-core-text` compatibility
//! feature. It is deliberately outside the portable Aimer rasterizer.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::text_pipeline::font_resolver::mapped_font_file;
use crate::text_pipeline::glyph_rasterizer::RasterizedGlyph;
use objc2_core_foundation::{CFDictionary, CFNumber, CFRetained, CFString, CFURL, CGPoint, CGRect};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGBitmapContextGetBytesPerRow, CGColorSpace, CGContext, CGGlyph,
    CGImageAlphaInfo,
};
use objc2_core_text::{
    CTFont, CTFontDescriptor, CTFontManagerCreateFontDescriptorsFromURL, CTFontOrientation,
    kCTFontNameAttribute, kCTFontVariationAxisDefaultValueKey, kCTFontVariationAxisIdentifierKey,
    kCTFontVariationAxisMaximumValueKey, kCTFontVariationAxisMinimumValueKey,
};

/// Point size used when only a yes/no answer about a glyph is needed.
///
/// Bounding rectangles scale linearly with the point size, so any positive
/// value decides emptiness identically.
const PROBE_FONT_SIZE: f32 = 32.0;

/// Largest bitmap accepted for a single glyph, per axis.
///
/// This matches the atlas bound: a larger glyph could not be stored anyway,
/// and refusing it here keeps a malformed or hostile font from asking for a
/// multi-gigabyte allocation.
const MAX_GLYPH_EXTENT: u32 = 2048;

/// The `wght` variation axis tag, `u32::from_be_bytes(*b"wght")`.
const WEIGHT_AXIS_TAG: i64 = u32::from_be_bytes(*b"wght") as i64;

/// Weight platform-drawn glyphs default to, on the OpenType `wght` scale.
///
/// Cupid addresses a glyph by `(font, glyph id, size, weight)`, and the weight
/// of the key travels into [`rasterize_glyph`], so a variable face is pinned
/// per request rather than to one constant. This value is the neutral
/// request, used by callers holding no weight of their own — the drawability
/// probe below, and plain text whose run demands nothing else. It matches the
/// pairing Apple uses for regular UI text: the cascade answers
/// `.PingFangUITextSC-Regular` with `wght` pinned to `400`, while the *default
/// instance* of the same variable file sits at `501` — the visibly heavier
/// stroke an unpinned font renders with.
const NORMAL_TEXT_WEIGHT: u16 = 400;

/// One pixel of slack around the reported bounding box.
///
/// Antialiased coverage reaches marginally beyond the design bounds, and
/// bitmap strikes may be authored slightly larger than the outline box they
/// are attached to. A single pixel is enough to keep edges from being clipped
/// and shifts the glyph's offsets consistently, so positioning is unaffected.
const BOUNDS_PADDING: f32 = 1.0;

/// Identity of a platform font: the file, the face inside it, the point size
/// in tenths — the same quantization [`GlyphKey`] uses — and the `wght` the
/// face is pinned to, so a cache entry corresponds to exactly one
/// rasterization size and weight.
///
/// [`GlyphKey`]: crate::text_pipeline::glyph_rasterizer::GlyphKey
type FontKey = (PathBuf, u32, u32, u16);

/// Fonts built so far, including the files the platform refused (`None`).
type FontCache = HashMap<FontKey, Option<CFRetained<CTFont>>>;

thread_local! {
    /// Fonts already built by this thread, keyed by file, face, size and
    /// weight.
    static FONT_CACHE: RefCell<FontCache> = RefCell::new(HashMap::new());
}

/// Reports whether the platform can draw `glyph_id` of the given face.
///
/// This is the question [`system_fallback`] has to answer before adopting a
/// face whose glyph data Cupid cannot read: a `cmap` entry alone proves
/// nothing, and neither does the absence of `glyf`. A glyph the platform
/// renders as blank — a space, or an unimplemented slot — reports `false`.
///
/// [`system_fallback`]: crate::text_pipeline::system_fallback
pub(crate) fn draws_glyph(path: &Path, collection_index: u32, glyph_id: u16) -> bool {
    // Emptiness does not depend on the weight a glyph is drawn at, so the
    // probe always asks for the neutral one and shares its font.
    with_font(
        path,
        collection_index,
        PROBE_FONT_SIZE,
        NORMAL_TEXT_WEIGHT,
        |font| {
            let bounds = glyph_bounds(font, glyph_id)?;
            (bounds.size.width > 0.0 && bounds.size.height > 0.0).then_some(())
        },
    )
    .is_some()
}

/// Draws `glyph_id` of the given face at `font_size` and returns its pixels.
///
/// `weight` is the OpenType `wght` the glyph must be drawn at; a variable
/// face is pinned to it — clamped into its axis range — while a static face
/// renders its one design regardless. The fallback pipeline derives it from
/// the run being drawn, so a platform glyph matches the stroke of the faces
/// Cupid rasterizes beside it.
///
/// The bitmap is 8-bit coverage for outline faces and non-premultiplied RGBA8
/// for color faces, which is exactly what [`GlyphAtlas`] and its color sibling
/// expect. `advance_width` is passed in rather than queried: shaping, layout
/// and measurement all derive advances from `hmtx`, and a glyph must not
/// advance differently just because the platform drew it.
///
/// Returns `None` when the face cannot be built, when the glyph is blank, or
/// when its bitmap would exceed [`MAX_GLYPH_EXTENT`].
///
/// [`GlyphAtlas`]: crate::text_pipeline::glyph_atlas::GlyphAtlas
pub(crate) fn rasterize_glyph(
    path: &Path,
    collection_index: u32,
    glyph_id: u16,
    font_size: f32,
    weight: u16,
    is_color: bool,
    advance_width: f32,
) -> Option<RasterizedGlyph> {
    with_font(path, collection_index, font_size, weight, |font| {
        let bounds = glyph_bounds(font, glyph_id)?;
        let (left, bottom, width, height) = pixel_box(bounds)?;

        let bitmap = if is_color {
            draw_color(font, glyph_id, left, bottom, width, height)?
        } else {
            draw_coverage(font, glyph_id, left, bottom, width, height)?
        };

        Some(RasterizedGlyph {
            bitmap,
            width,
            height,
            offset_x: left,
            offset_y: bottom,
            advance_width,
            is_color,
        })
    })
}

/// Returns the design bounding rectangle of `glyph_id`, in pixels.
fn glyph_bounds(font: &CTFont, glyph_id: u16) -> Option<CGRect> {
    let glyph: CGGlyph = glyph_id;
    let glyphs = std::ptr::NonNull::from(&glyph);
    // SAFETY: `glyphs` points at one `CGGlyph` and the count says so; passing
    // null for `bounding_rects` requests only the overall rectangle, which is
    // the return value.
    let bounds = unsafe {
        font.bounding_rects_for_glyphs(CTFontOrientation::Default, glyphs, std::ptr::null_mut(), 1)
    };
    (bounds.size.width.is_finite()
        && bounds.size.height.is_finite()
        && bounds.size.width > 0.0
        && bounds.size.height > 0.0)
        .then_some(bounds)
}

/// Converts a design bounding rectangle into a whole-pixel bitmap box.
///
/// Returns the left and bottom edges relative to the pen — the origin
/// [`RasterizedGlyph`] uses — together with the bitmap dimensions.
fn pixel_box(bounds: CGRect) -> Option<(f32, f32, u32, u32)> {
    let left = (bounds.origin.x as f32 - BOUNDS_PADDING).floor();
    let bottom = (bounds.origin.y as f32 - BOUNDS_PADDING).floor();
    let right = (bounds.origin.x + bounds.size.width) as f32 + BOUNDS_PADDING;
    let top = (bounds.origin.y + bounds.size.height) as f32 + BOUNDS_PADDING;

    let width = (right.ceil() - left) as i64;
    let height = (top.ceil() - bottom) as i64;
    if width <= 0
        || height <= 0
        || width > MAX_GLYPH_EXTENT as i64
        || height > MAX_GLYPH_EXTENT as i64
    {
        return None;
    }

    Some((left, bottom, width as u32, height as u32))
}

/// Bytes a bitmap row is padded to before it is handed to Core Graphics.
///
/// A bitmap context is free to lay its rows out on an alignment of its own
/// choosing, and it is documented to perform best on a multiple of 16 bytes.
/// Requesting a row exactly as wide as the glyph therefore invites the
/// framework to pad behind Cupid's back, and a row read back at the wrong
/// stride is skewed by a pixel more on every line — which reads as a thinner,
/// blurrier glyph rather than as a broken one, and strikes only the glyphs
/// whose width happens to be unaligned. Padding up front removes the question.
const ROW_ALIGNMENT: usize = 16;

/// Returns the padded stride, in bytes, of a row of `width` pixels of
/// `bytes_per_pixel`.
///
/// # Examples
///
/// ```ignore
/// assert_eq!(row_stride(38, 1), 48);
/// assert_eq!(row_stride(36, 1), 48);
/// assert_eq!(row_stride(16, 1), 16);
/// ```
#[inline]
fn row_stride(width: u32, bytes_per_pixel: usize) -> usize {
    let row = width as usize * bytes_per_pixel;
    row.div_ceil(ROW_ALIGNMENT) * ROW_ALIGNMENT
}

/// Returns the stride to read a rendered buffer back at.
///
/// `reported` is what the bitmap context says it used, which is the only
/// authority on the matter; `requested` is the fallback for the case where the
/// answer cannot be trusted — it must address whole rows and must not run past
/// the buffer that was handed over.
#[inline]
fn readback_stride(
    reported: usize,
    requested: usize,
    row_bytes: usize,
    height: u32,
    buffer: usize,
) -> usize {
    let fits = reported >= row_bytes && reported.saturating_mul(height as usize) <= buffer;
    if fits { reported } else { requested }
}

/// Copies `height` rows of `row_bytes` out of a buffer laid out at `stride`.
///
/// The stride is the one the bitmap context reports, not the one that was
/// asked for: reading the padding as if it were pixels shifts every row after
/// the first.
fn pack_rows(padded: &[u8], stride: usize, row_bytes: usize, height: u32) -> Vec<u8> {
    let mut packed = Vec::with_capacity(row_bytes * height as usize);
    for row in 0..height as usize {
        let start = row * stride;
        packed.extend_from_slice(&padded[start..start + row_bytes]);
    }
    packed
}

/// Renders `glyph_id` into an 8-bit coverage mask.
fn draw_coverage(
    font: &CTFont,
    glyph_id: u16,
    left: f32,
    bottom: f32,
    width: u32,
    height: u32,
) -> Option<Vec<u8>> {
    let stride = row_stride(width, 1);
    let mut padded = vec![0u8; stride * height as usize];
    // SAFETY: the buffer holds `stride * height` bytes, which is what the
    // 8-bit, one-component-per-pixel layout described here addresses. The
    // context is dropped before the buffer is read back.
    let context = unsafe {
        CGBitmapContextCreate(
            padded.as_mut_ptr().cast(),
            width as usize,
            height as usize,
            8,
            stride,
            None,
            CGImageAlphaInfo::Only.0,
        )
    }?;

    prepare(&context);
    // An alpha-only context keeps coverage, not color, so the fill has to be
    // fully opaque for the glyph to leave anything behind.
    CGContext::set_gray_fill_color(Some(&context), 1.0, 1.0);
    draw(&context, font, glyph_id, left, bottom);
    let stride = readback_stride(
        CGBitmapContextGetBytesPerRow(Some(&context)),
        stride,
        width as usize,
        height,
        padded.len(),
    );
    drop(context);

    Some(pack_rows(&padded, stride, width as usize, height))
}

/// Renders `glyph_id` into non-premultiplied RGBA8 color artwork.
fn draw_color(
    font: &CTFont,
    glyph_id: u16,
    left: f32,
    bottom: f32,
    width: u32,
    height: u32,
) -> Option<Vec<u8>> {
    let stride = row_stride(width, 4);
    let mut padded = vec![0u8; stride * height as usize];
    let color_space = CGColorSpace::new_device_rgb()?;
    // SAFETY: the buffer holds `stride * height` bytes, matching the 8 bits
    // per component, four components per pixel layout described here. The
    // context is dropped before the buffer is read back.
    let context = unsafe {
        CGBitmapContextCreate(
            padded.as_mut_ptr().cast(),
            width as usize,
            height as usize,
            8,
            stride,
            Some(&color_space),
            CGImageAlphaInfo::PremultipliedLast.0,
        )
    }?;

    prepare(&context);
    draw(&context, font, glyph_id, left, bottom);
    let stride = readback_stride(
        CGBitmapContextGetBytesPerRow(Some(&context)),
        stride,
        width as usize * 4,
        height,
        padded.len(),
    );
    drop(context);

    let mut bitmap = pack_rows(&padded, stride, width as usize * 4, height);
    un_premultiply(&mut bitmap);
    Some(bitmap)
}

/// Applies the rendering settings shared by both bitmap formats.
///
/// Font smoothing is Core Graphics' subpixel antialiasing, which would encode
/// a specific pixel geometry into the atlas; Cupid blends glyph coverage
/// itself, so plain grayscale antialiasing is what it needs.
fn prepare(context: &CGContext) {
    CGContext::set_should_antialias(Some(context), true);
    CGContext::set_allows_font_smoothing(Some(context), false);
    CGContext::set_should_smooth_fonts(Some(context), false);
    CGContext::set_allows_font_subpixel_positioning(Some(context), false);
    CGContext::set_should_subpixel_position_fonts(Some(context), false);
    CGContext::set_allows_font_subpixel_quantization(Some(context), false);
    CGContext::set_should_subpixel_quantize_fonts(Some(context), false);
}

/// Draws the glyph so that its bounding box lands on the bitmap's origin.
///
/// Core Graphics places glyphs relative to the text origin, so shifting the
/// pen by the negated bounding box moves the box onto the bitmap. The context
/// keeps its default bottom-left orientation: bitmap memory starts at the top
/// row regardless, which is the row order the atlas uploads.
fn draw(context: &CGContext, font: &CTFont, glyph_id: u16, left: f32, bottom: f32) {
    let glyph: CGGlyph = glyph_id;
    let position = CGPoint {
        x: -left as f64,
        y: -bottom as f64,
    };
    // SAFETY: both pointers address exactly one element, which is the count
    // passed alongside them.
    unsafe {
        font.draw_glyphs(
            std::ptr::NonNull::from(&glyph),
            std::ptr::NonNull::from(&position),
            1,
            context,
        );
    }
}

/// Converts premultiplied RGBA8 in place into the straight alpha the color
/// atlas stores.
///
/// Core Graphics bitmap contexts only composite premultiplied, while the atlas
/// is documented to hold non-premultiplied bytes; dividing the color channels
/// back out keeps a glyph's edges from darkening when the shader multiplies by
/// alpha again.
fn un_premultiply(bitmap: &mut [u8]) {
    aimer_simd::kernels::unpremultiply_rgba8(bitmap);
}

/// Runs `f` with the platform font for `(path, collection_index, font_size,
/// weight)`.
///
/// The font is built at most once per thread per size and weight; a file the
/// platform refuses is remembered as unusable so it is not reopened on every
/// glyph.
fn with_font<T>(
    path: &Path,
    collection_index: u32,
    font_size: f32,
    weight: u16,
    f: impl FnOnce(&CTFont) -> Option<T>,
) -> Option<T> {
    if font_size <= 0.0 {
        return None;
    }
    let key = (
        path.to_path_buf(),
        collection_index,
        (font_size * 10.0) as u32,
        weight,
    );
    FONT_CACHE.with_borrow_mut(|cache| {
        let font = cache
            .entry(key)
            .or_insert_with(|| create_font(path, collection_index, font_size, weight));
        f(font.as_deref()?)
    })
}

/// Builds the platform font for one face of a font file, pinned to `weight`.
///
/// A variable face is pinned to the requested weight rather than left at the
/// file's default instance — see [`pin_to_weight`].
fn create_font(
    path: &Path,
    collection_index: u32,
    font_size: f32,
    weight: u16,
) -> Option<CFRetained<CTFont>> {
    let descriptor = face_descriptor(path, collection_index)?;
    // SAFETY: a null matrix requests the identity transform.
    let font =
        unsafe { CTFont::with_font_descriptor(&descriptor, font_size as f64, std::ptr::null()) };
    Some(pin_to_weight(&descriptor, font, font_size, weight))
}

/// Returns `font` re-created at `weight` when it is variable.
///
/// A descriptor resolved from a font file alone renders the file's *default
/// instance*, and Apple's variable system faces default to a heavier one than
/// the weight their cascade pairs with regular text: `.PingFangUITextSC`
/// defaults to `wght` `501` while the cascade serves it pinned to `400`. Left
/// unpinned, every ideograph the platform draws lands visibly bolder than the
/// text Cupid rasterizes around it.
///
/// A static face, or one whose default already is the target, is returned
/// unchanged. The target is clamped into the axis range, so a face that
/// cannot express the requested weight comes back as close to it as it can
/// get.
fn pin_to_weight(
    descriptor: &CTFontDescriptor,
    font: CFRetained<CTFont>,
    font_size: f32,
    weight: u16,
) -> CFRetained<CTFont> {
    let Some((minimum, default, maximum)) = weight_axis_range(&font) else {
        return font;
    };
    let target = f64::from(weight).clamp(minimum, maximum);
    if default == target {
        return font;
    }
    let axis = CFNumber::new_i64(WEIGHT_AXIS_TAG);
    // SAFETY: the identifier is the `wght` axis tag as a CFNumber, which is
    // the form `CTFontDescriptorCreateCopyWithVariation` documents.
    let pinned = unsafe { descriptor.copy_with_variation(&axis, target) };
    // SAFETY: a null matrix requests the identity transform.
    unsafe { CTFont::with_font_descriptor(&pinned, font_size as f64, std::ptr::null()) }
}

/// Returns the descriptor of the face Cupid shaped with.
///
/// Descriptors are matched by PostScript name, because that is the identity
/// glyph ids belong to. The collection index serves as the fallback for files
/// whose name table cannot be read — the descriptor array follows the order of
/// the faces in the file.
fn face_descriptor(path: &Path, collection_index: u32) -> Option<CFRetained<CTFontDescriptor>> {
    let url = CFURL::from_file_path(path)?;
    // SAFETY: `url` is a valid file URL.
    let descriptors = unsafe { CTFontManagerCreateFontDescriptorsFromURL(&url) }?;
    // SAFETY: the function is documented to return an array of font
    // descriptors.
    let descriptors = unsafe { descriptors.cast_unchecked::<CTFontDescriptor>() };

    if let Some(expected) = postscript_name(path, collection_index)
        && let Some(matched) = descriptors
            .iter()
            .find(|descriptor| descriptor_name(descriptor).as_deref() == Some(expected.as_str()))
    {
        return Some(matched);
    }

    descriptors.get(collection_index as usize)
}

/// Returns the `wght` axis of `font` as `(minimum, default, maximum)`.
///
/// `None` means the face is static — there is no weight to pin.
fn weight_axis_range(font: &CTFont) -> Option<(f64, f64, f64)> {
    // SAFETY: the function returns an array of axis dictionaries or null.
    let axes = unsafe { font.variation_axes() }?;
    // SAFETY: every entry is an axis dictionary, and the only keys read below
    // are the numeric ones — identifier and range — whose values Core Text
    // documents as `CFNumberRef`.
    let axes = unsafe { axes.cast_unchecked::<CFDictionary<CFString, CFNumber>>() };
    axes.iter().find_map(|axis| {
        let value = |key: &CFString| axis.get(key)?.as_f64();
        // SAFETY: the keys are Core Text constant strings.
        unsafe {
            if value(kCTFontVariationAxisIdentifierKey)? as i64 != WEIGHT_AXIS_TAG {
                return None;
            }
            Some((
                value(kCTFontVariationAxisMinimumValueKey)?,
                value(kCTFontVariationAxisDefaultValueKey)?,
                value(kCTFontVariationAxisMaximumValueKey)?,
            ))
        }
    })
}

/// Returns the PostScript name a descriptor advertises.
fn descriptor_name(descriptor: &CTFontDescriptor) -> Option<String> {
    // SAFETY: `kCTFontNameAttribute` is a Core Text constant string.
    let attribute = unsafe { descriptor.attribute(kCTFontNameAttribute) }?;
    Some(attribute.downcast::<CFString>().ok()?.to_string())
}

/// Returns the PostScript name recorded in the font file itself.
fn postscript_name(path: &Path, collection_index: u32) -> Option<String> {
    let data = mapped_font_file(path)?;
    crate::text_pipeline::aimer_font::SfntFace::from_bytes(data.as_ref(), collection_index)
        .ok()?
        .name(6)
        .ok()
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The face carrying Simplified Chinese on Apple platforms, whose outlines
    /// live in a private table.
    fn simplified_chinese_face() -> Option<(PathBuf, u32, u16)> {
        crate::text_pipeline::apple_fonts::font_paths_for_codepoint(
            '吗',
            crate::text_pipeline::font_resolver::REGULAR_WEIGHT,
        )
        .into_iter()
        .find_map(|path| {
            let data = mapped_font_file(&path)?;
            (0..64).find_map(|index| {
                let face =
                    crate::text_pipeline::aimer_font::SfntFace::from_bytes(data.as_ref(), index)
                        .ok()?;
                let glyph_id = face.glyph_index('吗' as u32).ok().flatten()?;
                (glyph_id != 0).then_some((path.clone(), index, glyph_id))
            })
        })
    }

    // A glyph whose row is not a whole number of alignment units is the case
    // the padding exists for: 吗 is 36 pixels wide and 你 is 38, and only the
    // second one would ever have been laid out at a stride of its own.
    #[test]
    fn a_row_is_padded_to_the_alignment() {
        assert_eq!(row_stride(36, 1), 48);
        assert_eq!(row_stride(38, 1), 48);
        assert_eq!(row_stride(16, 1), 16);
        assert_eq!(row_stride(1, 1), ROW_ALIGNMENT);
        assert_eq!(row_stride(8, 4), 32);
    }

    #[test]
    fn a_padded_row_always_holds_its_pixels() {
        for width in 1..=64u32 {
            for bytes_per_pixel in [1usize, 4] {
                let stride = row_stride(width, bytes_per_pixel);
                assert!(stride >= width as usize * bytes_per_pixel);
                assert_eq!(stride % ROW_ALIGNMENT, 0);
            }
        }
    }

    // Reading a padded buffer at the pixel width instead of the stride shifts
    // every row after the first, which is the skew that thins a glyph.
    #[test]
    fn rows_are_copied_out_from_their_stride() {
        let padded = vec![
            1, 2, 3, 0, 0, // row 0: three pixels then padding
            4, 5, 6, 0, 0, // row 1
        ];

        assert_eq!(pack_rows(&padded, 5, 3, 2), vec![1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn a_tight_buffer_is_copied_unchanged() {
        let tight = vec![1, 2, 3, 4, 5, 6];

        assert_eq!(pack_rows(&tight, 3, 3, 2), tight);
    }

    // The context's own answer wins, because it is the only party that knows
    // how it laid the rows out.
    #[test]
    fn the_reported_stride_is_used_when_it_fits() {
        assert_eq!(readback_stride(48, 48, 38, 10, 48 * 10), 48);
        assert_eq!(readback_stride(64, 48, 38, 10, 64 * 10), 64);
    }

    // ...but never one that cannot hold a row, or that would read past the
    // buffer the context was given — a stride the buffer cannot back is a
    // report Cupid must not act on.
    #[test]
    fn an_impossible_reported_stride_falls_back() {
        assert_eq!(readback_stride(16, 48, 38, 10, 48 * 10), 48);
        assert_eq!(readback_stride(0, 48, 38, 10, 48 * 10), 48);
        assert_eq!(readback_stride(64, 48, 38, 10, 48 * 10), 48);
        assert_eq!(readback_stride(usize::MAX, 48, 38, 10, 48 * 10), 48);
    }

    /// Total coverage `font` deposits for `glyph_id`, in fully-lit pixels.
    fn ink(font: &CTFont, glyph_id: u16) -> f64 {
        let bounds = glyph_bounds(font, glyph_id).expect("the glyph is not blank");
        let (left, bottom, width, height) = pixel_box(bounds).expect("the glyph fits a bitmap");
        let bitmap =
            draw_coverage(font, glyph_id, left, bottom, width, height).expect("the glyph draws");
        bitmap.iter().map(|coverage| *coverage as f64).sum::<f64>() / 255.0
    }

    // The regression behind the "PingFang looks bolder" reports: the variable
    // system faces default to a heavier instance than the weight Apple pairs
    // with regular UI text (`.PingFangUITextSC-Default` answers `wght` 501,
    // the cascade pins 400), and a font rebuilt from the file alone renders
    // that heavy default. The production path must come out lighter than the
    // file's default instance.
    #[test]
    fn a_variable_face_is_drawn_at_the_normal_text_weight() {
        let (path, index, glyph_id) =
            simplified_chinese_face().expect("Apple platforms always carry a Chinese face");
        let descriptor = face_descriptor(&path, index).expect("the face has a descriptor");
        // SAFETY: a null matrix requests the identity transform.
        let unpinned = unsafe { CTFont::with_font_descriptor(&descriptor, 64.0, std::ptr::null()) };
        let Some((minimum, default, _)) = weight_axis_range(&unpinned) else {
            return; // A static face has no heavier default to pin down.
        };
        if default <= f64::from(NORMAL_TEXT_WEIGHT).max(minimum) {
            return; // The default instance is not the heavy one.
        }

        let default_ink = ink(&unpinned, glyph_id);
        let glyph = rasterize_glyph(
            &path,
            index,
            glyph_id,
            64.0,
            NORMAL_TEXT_WEIGHT,
            false,
            64.0,
        )
        .expect("the platform must draw the face");
        let pinned_ink = glyph
            .bitmap
            .iter()
            .map(|coverage| *coverage as f64)
            .sum::<f64>()
            / 255.0;

        assert!(
            pinned_ink < default_ink * 0.95,
            "a glyph drawn for regular text must be lighter than the file's \
             default instance: pinned {pinned_ink:.1} vs default {default_ink:.1}"
        );
    }

    // The weight of the glyph key must reach the platform: a run standing
    // beside a light kana face asks for a lighter instance, a bold style for
    // a heavier one, and each must deposit measurably different coverage.
    #[test]
    fn the_requested_weight_scales_the_ink_a_variable_face_deposits() {
        let (path, index, glyph_id) =
            simplified_chinese_face().expect("Apple platforms always carry a Chinese face");
        let font = create_font(&path, index, 64.0, NORMAL_TEXT_WEIGHT)
            .expect("the platform builds the face");
        let Some((minimum, _, maximum)) = weight_axis_range(&font) else {
            return; // A static face renders one design for every request.
        };
        if minimum > 300.0 || maximum < 600.0 {
            return; // The axis cannot express the weights being compared.
        }

        let coverage = |weight: u16| {
            let glyph = rasterize_glyph(&path, index, glyph_id, 64.0, weight, false, 64.0)
                .expect("the platform must draw the face");
            glyph
                .bitmap
                .iter()
                .map(|coverage| *coverage as f64)
                .sum::<f64>()
                / 255.0
        };

        let light = coverage(300);
        let normal = coverage(NORMAL_TEXT_WEIGHT);
        let heavy = coverage(600);

        assert!(
            light < normal * 0.97 && normal < heavy * 0.97,
            "weights must draw distinguishable strokes: \
             300 → {light:.1}, 400 → {normal:.1}, 600 → {heavy:.1}"
        );
    }

    // The axis reader answers with the file's own numbers, which the pinning
    // decision rests on.
    #[test]
    fn the_weight_axis_reports_a_range_that_holds_the_normal_weight() {
        let (path, index, _) =
            simplified_chinese_face().expect("Apple platforms always carry a Chinese face");
        let font = create_font(&path, index, 32.0, NORMAL_TEXT_WEIGHT)
            .expect("the platform builds the face");
        let Some((minimum, default, maximum)) = weight_axis_range(&font) else {
            return; // A static face exposes no axes.
        };
        assert!(minimum <= default && default <= maximum);
        assert!(
            minimum <= f64::from(NORMAL_TEXT_WEIGHT) && f64::from(NORMAL_TEXT_WEIGHT) <= maximum,
            "the weight axis {minimum}..{maximum} cannot express normal text"
        );
    }

    #[test]
    fn draws_a_glyph_whose_outlines_only_the_platform_can_read() {
        let (path, index, glyph_id) =
            simplified_chinese_face().expect("Apple platforms always carry a Chinese face");

        assert!(
            draws_glyph(&path, index, glyph_id),
            "{path:?}#{index} glyph {glyph_id} reported as undrawable"
        );

        let glyph = rasterize_glyph(
            &path,
            index,
            glyph_id,
            32.0,
            NORMAL_TEXT_WEIGHT,
            false,
            32.0,
        )
        .expect("the platform must draw a face it reports as drawable");
        assert!(!glyph.is_color);
        assert_eq!(glyph.bitmap.len(), (glyph.width * glyph.height) as usize);
        assert!(
            glyph.bitmap.iter().any(|coverage| *coverage > 0),
            "the coverage mask is blank"
        );
        // An ideograph is roughly square and fills its em box.
        assert!(
            glyph.width >= 16 && glyph.height >= 16,
            "{}x{}",
            glyph.width,
            glyph.height
        );
    }

    #[test]
    fn draws_color_emoji_as_straight_alpha_rgba() {
        let path = crate::text_pipeline::apple_fonts::font_paths_for_codepoint(
            '😀',
            crate::text_pipeline::font_resolver::REGULAR_WEIGHT,
        )
        .into_iter()
        .next()
        .expect("emoji resolve to a font file");
        let data = mapped_font_file(&path).expect("the emoji file is readable");
        let face = crate::text_pipeline::aimer_font::SfntFace::from_bytes(data.as_ref(), 0)
            .expect("the emoji file holds a face");
        let glyph_id = face
            .glyph_index('😀' as u32)
            .expect("the emoji cmap should parse")
            .expect("the face maps the emoji");

        let glyph = rasterize_glyph(&path, 0, glyph_id, 32.0, NORMAL_TEXT_WEIGHT, true, 32.0)
            .expect("the platform must draw color emoji");

        assert!(glyph.is_color);
        assert_eq!(
            glyph.bitmap.len(),
            (glyph.width * glyph.height * 4) as usize
        );
        assert!(
            glyph.bitmap.chunks_exact(4).any(|pixel| pixel[3] > 0),
            "the emoji artwork is fully transparent"
        );
        // The artwork is colorful: at least one visible pixel must have
        // channels that differ, which a coverage mask expanded to RGBA could
        // never produce.
        assert!(
            glyph
                .bitmap
                .chunks_exact(4)
                .any(|pixel| pixel[3] > 0 && (pixel[0] != pixel[1] || pixel[1] != pixel[2])),
            "the emoji artwork is monochrome"
        );
    }

    #[test]
    fn scales_with_the_requested_point_size() {
        let (path, index, glyph_id) =
            simplified_chinese_face().expect("Apple platforms always carry a Chinese face");

        let small = rasterize_glyph(
            &path,
            index,
            glyph_id,
            16.0,
            NORMAL_TEXT_WEIGHT,
            false,
            16.0,
        )
        .expect("drawn");
        let large = rasterize_glyph(
            &path,
            index,
            glyph_id,
            64.0,
            NORMAL_TEXT_WEIGHT,
            false,
            64.0,
        )
        .expect("drawn");

        assert!(
            large.width > small.width && large.height > small.height,
            "{}x{} is not larger than {}x{}",
            large.width,
            large.height,
            small.width,
            small.height
        );
    }

    #[test]
    fn keeps_the_advance_width_it_was_given() {
        let (path, index, glyph_id) =
            simplified_chinese_face().expect("Apple platforms always carry a Chinese face");

        let glyph = rasterize_glyph(
            &path,
            index,
            glyph_id,
            24.0,
            NORMAL_TEXT_WEIGHT,
            false,
            17.5,
        )
        .expect("drawn");

        assert_eq!(glyph.advance_width, 17.5);
    }

    #[test]
    fn rejects_a_file_that_holds_no_font() {
        let path = std::env::temp_dir().join("aimer-core-text-raster-not-a-font");
        std::fs::write(&path, b"not a font").expect("the temporary file is writable");

        assert!(!draws_glyph(&path, 0, 1));
        assert!(rasterize_glyph(&path, 0, 1, 16.0, NORMAL_TEXT_WEIGHT, false, 8.0).is_none());

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn rejects_a_non_positive_point_size() {
        let (path, index, glyph_id) =
            simplified_chinese_face().expect("Apple platforms always carry a Chinese face");

        assert!(
            rasterize_glyph(&path, index, glyph_id, 0.0, NORMAL_TEXT_WEIGHT, false, 8.0).is_none()
        );
    }

    #[test]
    fn un_premultiply_preserves_vector_boundaries_and_incomplete_pixels() {
        let pixels = [
            17, 99, 231, 0, 64, 32, 16, 128, 1, 128, 255, 255, 255, 13, 7, 1,
        ];
        let mut bitmap = pixels.repeat(5);
        bitmap.extend_from_slice(&[11, 22, 33]);
        un_premultiply(&mut bitmap);
        let mut expected = [
            17, 99, 231, 0, 128, 64, 32, 128, 1, 128, 255, 255, 255, 255, 255, 1,
        ]
        .repeat(5);
        expected.extend_from_slice(&[11, 22, 33]);
        assert_eq!(bitmap, expected);
    }

    #[test]
    fn un_premultiply_restores_straight_alpha() {
        // Half-transparent pure red, premultiplied: the color channel comes
        // back to full intensity.
        let mut bitmap = vec![128, 0, 0, 128, 255, 255, 255, 255, 0, 0, 0, 0];
        un_premultiply(&mut bitmap);

        assert_eq!(bitmap[0], 255);
        assert_eq!(bitmap[3], 128);
        // Opaque and fully transparent pixels are left untouched.
        assert_eq!(&bitmap[4..8], &[255, 255, 255, 255]);
        assert_eq!(&bitmap[8..], &[0, 0, 0, 0]);
    }
}
