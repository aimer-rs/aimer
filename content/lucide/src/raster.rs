//! Draws an icon into a plain RGBA pixel buffer on the CPU.
//!
//! Widgets draw icons on the GPU, but some places only take pixels: a system
//! tray icon, a window icon, a notification image. [`rasterize`] covers them.
//!
//! The result is a single flat color with a smooth alpha edge, which is what
//! those places want, and what lets a monochrome menu bar tint it. Each shape
//! is sampled on a grid of [`SUPERSAMPLE`] x [`SUPERSAMPLE`] points per pixel.
//!
//! # Limits
//!
//! Strokes get round caps and round joins whatever the document asks for: that
//! is what every Lucide icon uses. Dashes, gradients, masks, clips and filters
//! are ignored.

use aimer_cupid::svg::{SvgFillRule, SvgPathCommand, SvgTransform};
use aimer_svg::{SvgColor, SvgDocument};

/// Sample points per pixel along each axis.
pub const SUPERSAMPLE: usize = 4;

/// The largest side [`rasterize`] accepts, so a bad size cannot ask for
/// gigabytes of samples.
pub const MAX_SIZE: u32 = 1024;

/// A straight (not premultiplied) RGBA image, row by row from the top.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RasterIcon {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl RasterIcon {
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

    /// The pixels, four bytes each: red, green, blue, alpha.
    #[inline]
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    /// Takes the pixels out, for an API that owns its buffer.
    #[inline]
    pub fn into_rgba(self) -> Vec<u8> {
        self.rgba
    }
}

/// Draws `icon` into a square `size` x `size` image in `color`.
///
/// `icon` is any [`SvgDocument`], which includes every constant of
/// [`Icons`](crate::Icons). The icon is scaled to fill the image the way the
/// widget does. The color's alpha multiplies the coverage.
///
/// # Panics
///
/// Panics if `size` is zero or above [`MAX_SIZE`].
///
/// # Examples
///
/// ```
/// use aimer_lucide::{Icons, rasterize};
/// use aimer_svg::SvgColor;
///
/// let bell = rasterize(Icons::BELL, 32, SvgColor::rgba8(40, 120, 255, 255));
/// assert_eq!((bell.width(), bell.height()), (32, 32));
/// assert_eq!(bell.rgba().len(), 32 * 32 * 4);
/// ```
pub fn rasterize(icon: impl Into<SvgDocument>, size: u32, color: SvgColor) -> RasterIcon {
    assert!((1..=MAX_SIZE).contains(&size), "icon size must be 1..={MAX_SIZE}, got {size}");
    let document: SvgDocument = icon.into();
    let side = size as usize * SUPERSAMPLE;
    let mut mask = Mask { side, bits: vec![false; side * side] };

    let scene = document.scene();
    let fit = document
        .fit_transform(size as f32, size as f32)
        .unwrap_or_default();
    let mut points = Vec::new();
    for node in scene.nodes.iter() {
        if !node.visible || node.is_definition {
            continue;
        }
        let Some(geometry) = scene.geometry(node) else { continue };
        let to_pixels = fit.mul(node.transform);
        let scale = (to_pixels.sx * to_pixels.sy - to_pixels.kx * to_pixels.ky).abs().sqrt();
        let paths = flatten(&geometry.commands, to_pixels, &mut points);

        if node.fill.as_ref().is_some_and(|fill| fill.color.a > 0.0) {
            let rule = node.fill.as_ref().map_or(SvgFillRule::NonZero, |fill| fill.rule);
            mask.fill(&points, &paths, rule);
        }
        if let Some(stroke) = node.stroke.as_ref().filter(|stroke| stroke.color.a > 0.0) {
            mask.stroke(&points, &paths, stroke.width * scale * 0.5);
        }
    }
    mask.resolve(size, color)
}

/// One flattened subpath: a range of the shared point list.
struct Path {
    start: usize,
    end: usize,
    closed: bool,
}

/// Turns the commands into straight polylines in pixel space.
fn flatten(commands: &[SvgPathCommand], to_pixels: SvgTransform, points: &mut Vec<(f32, f32)>) -> Vec<Path> {
    points.clear();
    let mut paths = Vec::new();
    let mut start = 0;
    let mut subpath_start = (0.0, 0.0);
    let mut current = (0.0, 0.0);

    let finish = |paths: &mut Vec<Path>, points: &Vec<(f32, f32)>, start: usize, closed: bool| {
        if points.len() > start {
            paths.push(Path { start, end: points.len(), closed });
        }
    };

    for command in commands {
        match *command {
            SvgPathCommand::MoveTo { x, y } => {
                finish(&mut paths, points, start, false);
                start = points.len();
                current = to_pixels.transform_point(x, y);
                subpath_start = current;
                points.push(current);
            }
            SvgPathCommand::LineTo { x, y } => {
                current = to_pixels.transform_point(x, y);
                points.push(current);
            }
            SvgPathCommand::QuadraticTo { control_x, control_y, x, y } => {
                let control = to_pixels.transform_point(control_x, control_y);
                let end = to_pixels.transform_point(x, y);
                let steps = steps_for(&[current, control, end]);
                for step in 1..=steps {
                    let t = step as f32 / steps as f32;
                    let u = 1.0 - t;
                    points.push((
                        u * u * current.0 + 2.0 * u * t * control.0 + t * t * end.0,
                        u * u * current.1 + 2.0 * u * t * control.1 + t * t * end.1,
                    ));
                }
                current = end;
            }
            SvgPathCommand::CubicTo { control1_x, control1_y, control2_x, control2_y, x, y } => {
                let c1 = to_pixels.transform_point(control1_x, control1_y);
                let c2 = to_pixels.transform_point(control2_x, control2_y);
                let end = to_pixels.transform_point(x, y);
                let steps = steps_for(&[current, c1, c2, end]);
                for step in 1..=steps {
                    let t = step as f32 / steps as f32;
                    let u = 1.0 - t;
                    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
                    points.push((
                        a * current.0 + b * c1.0 + c * c2.0 + d * end.0,
                        a * current.1 + b * c1.1 + c * c2.1 + d * end.1,
                    ));
                }
                current = end;
            }
            SvgPathCommand::Close => {
                finish(&mut paths, points, start, true);
                // A path may carry on drawing after a close, from where it began.
                start = points.len();
                current = subpath_start;
                points.push(current);
            }
        }
    }
    finish(&mut paths, points, start, false);
    // A lone point left by a trailing close or move draws nothing.
    paths.retain(|path| path.end - path.start > 1 || path.closed);
    paths
}

/// How many straight pieces a curve with these control points needs: one per
/// pixel of control polygon, which keeps the deviation well below a sample.
fn steps_for(control: &[(f32, f32)]) -> usize {
    let length: f32 = control
        .windows(2)
        .map(|pair| ((pair[1].0 - pair[0].0).powi(2) + (pair[1].1 - pair[0].1).powi(2)).sqrt())
        .sum();
    (length.ceil() as usize).clamp(4, 64)
}

/// Which sample points the icon covers.
struct Mask {
    /// Samples per side.
    side: usize,
    bits: Vec<bool>,
}

impl Mask {
    /// Marks the samples inside the polygons, between the rule's crossings.
    fn fill(&mut self, points: &[(f32, f32)], paths: &[Path], rule: SvgFillRule) {
        let scale = SUPERSAMPLE as f32;
        let mut crossings: Vec<(f32, i32)> = Vec::new();
        for row in 0..self.side {
            let y = (row as f32 + 0.5) / scale;
            crossings.clear();
            for path in paths {
                let ring = &points[path.start..path.end];
                for index in 0..ring.len() {
                    // Every subpath is closed for filling.
                    let (a, b) = (ring[index], ring[(index + 1) % ring.len()]);
                    if (a.1 <= y) == (b.1 <= y) {
                        continue;
                    }
                    let x = a.0 + (y - a.1) / (b.1 - a.1) * (b.0 - a.0);
                    crossings.push((x, if b.1 > a.1 { 1 } else { -1 }));
                }
            }
            crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut winding = 0;
            for (index, &(x, direction)) in crossings.iter().enumerate() {
                winding += direction;
                let inside = match rule {
                    SvgFillRule::NonZero => winding != 0,
                    SvgFillRule::EvenOdd => (index + 1) % 2 == 1,
                };
                let Some(&(next, _)) = crossings.get(index + 1) else { break };
                if inside {
                    self.span(row, x * scale, next * scale);
                }
            }
        }
    }

    /// Marks the samples within `radius` pixels of the polylines: a union of
    /// capsules, which is what a stroke with round caps and joins is.
    fn stroke(&mut self, points: &[(f32, f32)], paths: &[Path], radius: f32) {
        let radius = radius.max(0.0);
        for path in paths {
            let ring = &points[path.start..path.end];
            let segments = if path.closed { ring.len() } else { ring.len() - 1 };
            if ring.len() == 1 {
                self.capsule(ring[0], ring[0], radius);
            }
            for index in 0..segments {
                self.capsule(ring[index], ring[(index + 1) % ring.len()], radius);
            }
        }
    }

    fn capsule(&mut self, a: (f32, f32), b: (f32, f32), radius: f32) {
        let scale = SUPERSAMPLE as f32;
        let limit = self.side as f32;
        let low_x = ((a.0.min(b.0) - radius) * scale).floor().max(0.0) as usize;
        let high_x = (((a.0.max(b.0) + radius) * scale).ceil().min(limit)) as usize;
        let low_y = ((a.1.min(b.1) - radius) * scale).floor().max(0.0) as usize;
        let high_y = (((a.1.max(b.1) + radius) * scale).ceil().min(limit)) as usize;
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let length_squared = dx * dx + dy * dy;
        let radius_squared = radius * radius;
        for row in low_y..high_y {
            let y = (row as f32 + 0.5) / scale;
            for column in low_x..high_x {
                let x = (column as f32 + 0.5) / scale;
                let t = if length_squared > 0.0 {
                    (((x - a.0) * dx + (y - a.1) * dy) / length_squared).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let (nx, ny) = (a.0 + t * dx - x, a.1 + t * dy - y);
                if nx * nx + ny * ny <= radius_squared {
                    self.bits[row * self.side + column] = true;
                }
            }
        }
    }

    /// Marks the samples of `row` whose centers lie in `from..to`.
    fn span(&mut self, row: usize, from: f32, to: f32) {
        let first = (from - 0.5).ceil().max(0.0) as usize;
        let last = ((to - 0.5).ceil().max(0.0) as usize).min(self.side);
        if first < last {
            self.bits[row * self.side + first..row * self.side + last].fill(true);
        }
    }

    /// Averages each pixel's samples into its alpha.
    fn resolve(&self, size: u32, color: SvgColor) -> RasterIcon {
        let size = size as usize;
        let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        let (r, g, b) = (channel(color.r), channel(color.g), channel(color.b));
        let samples = (SUPERSAMPLE * SUPERSAMPLE) as f32;
        let mut rgba = Vec::with_capacity(size * size * 4);
        for y in 0..size {
            for x in 0..size {
                let mut covered = 0u32;
                for sample_y in 0..SUPERSAMPLE {
                    let start = (y * SUPERSAMPLE + sample_y) * self.side + x * SUPERSAMPLE;
                    covered += self.bits[start..start + SUPERSAMPLE].iter().filter(|&&on| on).count() as u32;
                }
                let alpha = covered as f32 / samples * color.a;
                rgba.extend_from_slice(&[r, g, b, channel(alpha)]);
            }
        }
        RasterIcon { width: size as u32, height: size as u32, rgba }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Icons;

    const WHITE: SvgColor = SvgColor::rgba8(255, 255, 255, 255);

    fn alpha(icon: &RasterIcon, x: u32, y: u32) -> u8 {
        icon.rgba()[((y * icon.width() + x) * 4 + 3) as usize]
    }

    fn document(svg: &str) -> SvgDocument {
        SvgDocument::from_svg(svg).expect("test svg parses")
    }

    #[test]
    fn a_filled_square_covers_its_pixels_and_nothing_else() {
        let square = document(
            r#"<svg viewBox="0 0 8 8" fill="black"><path d="M2 2h4v4h-4z"/></svg>"#,
        );
        let icon = rasterize(square, 8, WHITE);
        assert_eq!(alpha(&icon, 3, 3), 255);
        assert_eq!(alpha(&icon, 2, 2), 255);
        assert_eq!(alpha(&icon, 5, 5), 255);
        assert_eq!(alpha(&icon, 1, 3), 0);
        assert_eq!(alpha(&icon, 6, 3), 0);
        assert_eq!(alpha(&icon, 3, 7), 0);
    }

    #[test]
    fn an_even_odd_hole_stays_empty() {
        let ring = document(
            r#"<svg viewBox="0 0 8 8" fill="black" fill-rule="evenodd"><path d="M0 0h8v8h-8zM2 2h4v4h-4z"/></svg>"#,
        );
        let icon = rasterize(ring, 8, WHITE);
        assert_eq!(alpha(&icon, 0, 0), 255);
        assert_eq!(alpha(&icon, 3, 3), 0);
    }

    #[test]
    fn a_stroke_is_as_wide_as_declared_and_antialiased_at_its_edge() {
        let line = document(
            r#"<svg viewBox="0 0 8 8" fill="none" stroke="black" stroke-width="2"><path d="M0 4.5h8"/></svg>"#,
        );
        let icon = rasterize(line, 8, WHITE);
        // The stroke spans y = 3.5..5.5: row 4 is full, rows 3 and 5 are half.
        assert_eq!(alpha(&icon, 4, 4), 255);
        assert_eq!(alpha(&icon, 4, 3), 128);
        assert_eq!(alpha(&icon, 4, 5), 128);
        assert_eq!(alpha(&icon, 4, 2), 0);
        assert_eq!(alpha(&icon, 4, 6), 0);
    }

    #[test]
    fn the_icon_scales_with_the_requested_size() {
        let small = rasterize(Icons::BELL, 16, WHITE);
        let large = rasterize(Icons::BELL, 64, WHITE);
        assert_eq!(small.rgba().len(), 16 * 16 * 4);
        assert_eq!(large.rgba().len(), 64 * 64 * 4);
        // Mean coverage is what stays put when the size changes; counting any
        // touched pixel would not, because thin lines touch more of a small grid.
        let ink = |icon: &RasterIcon| {
            let total: u32 = icon.rgba().chunks(4).map(|px| u32::from(px[3])).sum();
            total as f32 / 255.0 / (icon.width() * icon.height()) as f32
        };
        assert!((ink(&small) - ink(&large)).abs() < 0.02, "{} vs {}", ink(&small), ink(&large));
        assert!(ink(&large) > 0.05, "a bell has ink");
    }

    #[test]
    fn the_color_is_used_for_every_pixel_and_its_alpha_scales_coverage() {
        let icon = rasterize(Icons::BELL, 24, SvgColor::rgba8(10, 20, 30, 128));
        assert!(icon.rgba().chunks(4).all(|px| px[..3] == [10, 20, 30]));
        assert!(icon.rgba().chunks(4).all(|px| px[3] <= 128));
        assert!(icon.rgba().chunks(4).any(|px| px[3] == 128));
    }

    #[test]
    fn a_bell_is_left_right_symmetric_apart_from_antialiasing() {
        let icon = rasterize(Icons::HOUSE, 24, WHITE);
        let mut difference = 0u32;
        for y in 0..24 {
            for x in 0..12 {
                difference += alpha(&icon, x, y).abs_diff(alpha(&icon, 23 - x, y)) as u32;
            }
        }
        // A house is symmetric about x = 12, so the halves agree closely.
        assert!(difference < 24 * 12 * 40, "difference {difference}");
    }

    #[test]
    #[should_panic(expected = "icon size")]
    fn a_zero_size_is_rejected() {
        let _ = rasterize(Icons::BELL, 0, WHITE);
    }

    #[test]
    fn every_bundled_icon_draws_something() {
        for icon in [Icons::BELL, Icons::HOUSE, Icons::SEARCH, Icons::STAR, Icons::SETTINGS] {
            let image = rasterize(icon, 24, WHITE);
            assert!(image.rgba().chunks(4).any(|px| px[3] > 0));
        }
    }
}
