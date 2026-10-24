use aimer_attribute::{ResolvedSize, Vec2d};
use aimer_canvas::{
    Canvas, GradientShape, MAX_GRADIENT_STOPS, MaterialDrawRequest, MaterialGradient, MaterialKind,
};
use aimer_color::prelude::Color;

use super::{Decoration, declared_corner_radii};

/// Paints a linear, radial or conic color ramp over a box.
///
/// The ramp is evaluated per pixel on the GPU from its stops, so it stays smooth
/// at any size and scale, costs no texture upload, and re-resolves for free when
/// the box is resized. Colors are blended premultiplied in display space the way
/// CSS gradients are, so a transparent stop fades its neighbor instead of tinting
/// it. Angles follow CSS: `0` points up and angles turn clockwise.
///
/// Corners are square unless a [`BorderRadius`](crate::BorderRadius) or
/// [`BoxDecoration`](crate::BoxDecoration) radius precedes the gradient in the
/// same stack, in which case it follows those per-corner radii, each fitted
/// within half the shorter side. A gradient adds no fill, border, shadow, child
/// inset or clip of its own.
///
/// A gradient holds up to eight stops. Stops stay sorted by offset; adding more
/// than eight, or a stop with a nonfinite offset, is ignored. A ramp with one
/// stop is a solid color and one with none paints nothing.
///
/// # Examples
///
/// ```
/// use aimer_container::SizedBox;
/// use aimer_decoration::{BorderRadius, Decoration, DecorationBox, Gradient};
/// use aimer_widget::base::Color;
///
/// // Sunset, top to bottom, with rounded corners.
/// let sunset = DecorationBox::new()
///     .decoration(vec![
///         BorderRadius::from(16).notype(),
///         Gradient::linear()
///             .stop(0.0, Color::Rgb(255, 94, 98))
///             .stop(1.0, Color::Rgb(255, 195, 113))
///             .notype(),
///     ])
///     .child(SizedBox::new().width(240).height(80));
///
/// // Seven evenly spaced colors, left to right.
/// let rainbow = Gradient::rainbow();
///
/// // A spotlight that fades out toward the edge.
/// let spot = Gradient::radial()
///     .colors([Color::Rgba(255, 255, 255, 200), Color::Rgba(255, 255, 255, 0)]);
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gradient {
    shape: GradientShape,
    /// Linear: CSS angle in degrees. Conic: CSS start angle in degrees.
    angle: f32,
    /// Radial and conic center as a fraction of the box.
    center: [f32; 2],
    /// Radial extent as a multiple of the farthest-corner ellipse.
    radius: f32,
    stop_count: u8,
    offsets: [f32; MAX_GRADIENT_STOPS],
    /// Straight RGBA8, packed `r | g << 8 | b << 16 | a << 24`.
    colors: [u32; MAX_GRADIENT_STOPS],
}

impl Gradient {
    /// The seven colors of [`Gradient::rainbow`], red to violet.
    pub const RAINBOW: [Color; 7] = [
        Color::Hex(0xFFFF0000),
        Color::Hex(0xFFFF7F00),
        Color::Hex(0xFFFFFF00),
        Color::Hex(0xFF00FF00),
        Color::Hex(0xFF0000FF),
        Color::Hex(0xFF4B0082),
        Color::Hex(0xFF9400D3),
    ];

    #[inline]
    fn with_shape(shape: GradientShape, angle: f32) -> Self {
        Self {
            shape,
            angle,
            center: [0.5, 0.5],
            radius: 1.0,
            stop_count: 0,
            offsets: [0.0; MAX_GRADIENT_STOPS],
            colors: [0; MAX_GRADIENT_STOPS],
        }
    }

    /// Creates a stopless linear gradient pointing to the bottom (`180°`).
    ///
    /// Add colors with [`Self::stop`] or [`Self::colors`].
    #[inline]
    pub fn linear() -> Self {
        Self::with_shape(GradientShape::Linear, 180.0)
    }

    /// Creates a stopless radial gradient centered in the box.
    ///
    /// The first stop is the center and the last is the farthest-corner ellipse
    /// around it; see [`Self::center`] and [`Self::radius`].
    #[inline]
    pub fn radial() -> Self {
        Self::with_shape(GradientShape::Radial, 0.0)
    }

    /// Creates a stopless conic gradient centered in the box, starting at the top.
    #[inline]
    pub fn conic() -> Self {
        Self::with_shape(GradientShape::Conic, 0.0)
    }

    /// Creates the seven [`Self::RAINBOW`] colors running left to right.
    #[inline]
    pub fn rainbow() -> Self {
        Self::linear().angle(90.0).colors(Self::RAINBOW)
    }

    /// Adds a color at `offset` along the ramp, keeping the stops sorted.
    ///
    /// The offset is clamped to `0.0..=1.0`; a nonfinite offset, or a ninth
    /// stop, is ignored. Stops at the same offset keep their insertion order, so
    /// a hard edge is two stops at one offset.
    pub fn stop(mut self, offset: f32, color: Color) -> Self {
        let count = usize::from(self.stop_count);
        if !offset.is_finite() || count == MAX_GRADIENT_STOPS {
            return self;
        }
        let offset = offset.clamp(0.0, 1.0);
        let at = self.offsets[..count].partition_point(|existing| *existing <= offset);
        self.offsets.copy_within(at..count, at + 1);
        self.colors.copy_within(at..count, at + 1);
        self.offsets[at] = offset;
        self.colors[at] = pack(color);
        self.stop_count += 1;
        self
    }

    /// Replaces every stop with `colors`, spaced evenly from `0.0` to `1.0`.
    ///
    /// Colors beyond the eighth are ignored; a single color is a solid fill.
    pub fn colors(mut self, colors: impl IntoIterator<Item = Color>) -> Self {
        self.stop_count = 0;
        let mut packed = [0; MAX_GRADIENT_STOPS];
        let mut count = 0;
        for color in colors.into_iter().take(MAX_GRADIENT_STOPS) {
            packed[count] = pack(color);
            count += 1;
        }
        self.offsets = [0.0; MAX_GRADIENT_STOPS];
        self.colors = packed;
        for index in 1..count {
            self.offsets[index] = index as f32 / (count - 1) as f32;
        }
        self.stop_count = count as u8;
        self
    }

    /// Sets the direction of a linear gradient in degrees, as CSS does.
    ///
    /// `0` points up, `90` right, `180` (the default) down and `270` left.
    /// Finite angles wrap into `0.0..360.0`; a nonfinite angle keeps the
    /// current one. The gradient line is as long as the box's extent along it, so
    /// a diagonal angle reaches the corners exactly. Radial gradients ignore it.
    #[inline]
    pub fn angle(mut self, degrees: f32) -> Self {
        if degrees.is_finite() && self.shape == GradientShape::Linear {
            self.angle = degrees.rem_euclid(360.0);
        }
        self
    }

    /// Sets where a conic gradient starts, in CSS degrees: `0` is the top and
    /// the ramp turns clockwise. Finite angles wrap into `0.0..360.0`; a
    /// nonfinite angle keeps the current one. Other shapes ignore it.
    #[inline]
    pub fn from_angle(mut self, degrees: f32) -> Self {
        if degrees.is_finite() && self.shape == GradientShape::Conic {
            self.angle = degrees.rem_euclid(360.0);
        }
        self
    }

    /// Sets the center of a radial or conic gradient as a fraction of the box:
    /// `(0.0, 0.0)` is the top-left corner and `(1.0, 1.0)` the bottom-right.
    /// A nonfinite coordinate keeps the current one. Linear gradients ignore it.
    #[inline]
    pub fn center(mut self, x: f32, y: f32) -> Self {
        if x.is_finite() {
            self.center[0] = x;
        }
        if y.is_finite() {
            self.center[1] = y;
        }
        self
    }

    /// Scales a radial gradient's extent. `1.0` (the default) ends at the
    /// farthest corner of the box; `0.5` ends halfway there.
    ///
    /// Negative values clamp to zero, which paints only the last stop; a
    /// nonfinite value keeps the current radius. Other shapes ignore it.
    #[inline]
    pub fn radius(mut self, scale: f32) -> Self {
        if scale.is_finite() {
            self.radius = scale.max(0.0);
        }
        self
    }

    /// Resolves the shape against a logical box size, relative to the box origin.
    fn resolve(&self, size: ResolvedSize) -> MaterialGradient {
        let (width, height) = (size.width, size.height);
        let geometry = match self.shape {
            GradientShape::Linear => {
                let (sin, cos) = self.angle.to_radians().sin_cos();
                let (dx, dy) = (sin, -cos);
                let half = (width * sin.abs() + height * cos.abs()) * 0.5;
                let (cx, cy) = (width * 0.5, height * 0.5);
                [cx - dx * half, cy - dy * half, cx + dx * half, cy + dy * half]
            }
            GradientShape::Radial => {
                let (cx, cy) = (self.center[0] * width, self.center[1] * height);
                let corner = std::f32::consts::SQRT_2 * self.radius;
                [
                    cx,
                    cy,
                    cx.max(width - cx) * corner,
                    cy.max(height - cy) * corner,
                ]
            }
            GradientShape::Conic => [
                self.center[0] * width,
                self.center[1] * height,
                // The shader measures from screen-right; CSS from the top.
                (self.angle - 90.0).to_radians(),
                0.0,
            ],
        };
        MaterialGradient {
            shape: self.shape,
            geometry,
            stop_count: self.stop_count,
            offsets: self.offsets,
            colors: self.colors,
        }
    }
}

impl Decoration for Gradient {
    fn paint(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, _scale: f32) {
        if size.width <= 0.0 || size.height <= 0.0 || self.is_empty() {
            return;
        }
        let limit = size.width.min(size.height) * 0.5;
        let mut request = MaterialDrawRequest::from_layout(MaterialKind::Gradient, origin, size);
        request.gradient = self.resolve(size);
        request.opacity = 1.0;
        request.corner_radii =
            declared_corner_radii(canvas, limit.min(MaterialDrawRequest::MAX_CORNER_RADIUS));
        canvas.draw_material(request);
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.stop_count == 0
    }
}

#[inline]
fn pack(color: Color) -> u32 {
    let [alpha, red, green, blue] = color.as_u32().to_be_bytes();
    MaterialGradient::pack_color([red, green, blue, alpha])
}
