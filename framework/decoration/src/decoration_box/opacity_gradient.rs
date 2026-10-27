//! [`OpacityGradient`], an alpha ramp used as a mask.

use aimer_attribute::{ResolvedSize, Vec2d};
use aimer_canvas::Canvas;
use aimer_color::prelude::Color;
use aimer_widget::Widget;

use super::{Decoration, Gradient};
use crate::OpacityMaskBox;

/// An alpha ramp over a box: the mask that fades a child out.
///
/// It takes the same shapes as [`Gradient`] (linear, radial and conic, with the
/// same angle, center and radius), but its stops are opacities, `0.0` for fully
/// transparent and `1.0` for fully opaque, instead of colors. Used on its own
/// the ramp paints black of that opacity; its purpose is to be the decoration of
/// an [`OpacityMaskBox`], which draws its child through the ramp so that the
/// child is faded wherever the ramp is transparent. Where the ramp is opaque the
/// child shows unchanged.
///
/// Like [`Gradient`] it is evaluated per pixel on the GPU, follows the
/// [`BorderRadius`](crate::BorderRadius) that precedes it in a stack, holds up
/// to eight stops and costs no texture upload. Opacities are quantized to eight
/// bits, as the pixels they fade are.
///
/// A ramp with no stops is [empty](Decoration::is_empty), and a box built with it
/// leaves its child as it is.
///
/// # Examples
///
/// ```
/// use aimer_container::SizedBox;
/// use aimer_decoration::{OpacityGradient, OpacityMaskBox};
///
/// // A list that fades out over its bottom fifth.
/// let list = SizedBox::new().width(240).height(320);
/// let faded = OpacityGradient::linear()
///     .stop(0.0, 1.0)
///     .stop(0.8, 1.0)
///     .stop(1.0, 0.0)
///     .child(list);
///
/// // The same, with the box spelled out.
/// let spelled = OpacityMaskBox::new()
///     .decoration(OpacityGradient::linear().opacities([1.0, 0.0]))
///     .child(SizedBox::new().width(240).height(320));
///
/// // A vignette: opaque in the middle, transparent toward the corners.
/// let vignette = OpacityGradient::radial().opacities([1.0, 1.0, 0.0]);
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OpacityGradient {
    ramp: Gradient,
}

impl OpacityGradient {
    #[inline]
    fn from_ramp(ramp: Gradient) -> Self {
        Self { ramp }
    }

    /// Creates a stopless linear ramp pointing to the bottom (`180°`).
    ///
    /// Add opacities with [`Self::stop`] or [`Self::opacities`].
    #[inline]
    pub fn linear() -> Self {
        Self::from_ramp(Gradient::linear())
    }

    /// Creates a stopless radial ramp centered in the box.
    ///
    /// The first stop is the center and the last is the farthest-corner ellipse
    /// around it; see [`Self::center`] and [`Self::radius`].
    #[inline]
    pub fn radial() -> Self {
        Self::from_ramp(Gradient::radial())
    }

    /// Creates a stopless conic ramp centered in the box, starting at the top.
    #[inline]
    pub fn conic() -> Self {
        Self::from_ramp(Gradient::conic())
    }

    /// Adds an opacity at `offset` along the ramp, keeping the stops sorted.
    ///
    /// The offset is clamped to `0.0..=1.0` and the opacity to `0.0..=1.0`. A
    /// nonfinite offset or opacity, or a ninth stop, is ignored. Stops at the same
    /// offset keep their insertion order, so a hard edge is two stops at one
    /// offset.
    #[inline]
    pub fn stop(mut self, offset: f32, opacity: f32) -> Self {
        if let Some(color) = shade(opacity) {
            self.ramp = self.ramp.stop(offset, color);
        }
        self
    }

    /// Replaces every stop with `opacities`, spaced evenly from `0.0` to `1.0`.
    ///
    /// Each opacity is clamped to `0.0..=1.0`. Nonfinite opacities and any beyond
    /// the eighth are ignored; a single opacity is a uniform fade.
    pub fn opacities(mut self, opacities: impl IntoIterator<Item = f32>) -> Self {
        self.ramp = self
            .ramp
            .colors(opacities.into_iter().filter_map(shade));
        self
    }

    /// Sets the direction of a linear ramp in degrees, as CSS does: `0` points
    /// up, `90` right, `180` (the default) down and `270` left.
    ///
    /// See [`Gradient::angle`]. Other shapes ignore it.
    #[inline]
    pub fn angle(mut self, degrees: f32) -> Self {
        self.ramp = self.ramp.angle(degrees);
        self
    }

    /// Sets where a conic ramp starts, in CSS degrees: `0` is the top and the
    /// ramp turns clockwise. See [`Gradient::from_angle`]. Other shapes ignore it.
    #[inline]
    pub fn from_angle(mut self, degrees: f32) -> Self {
        self.ramp = self.ramp.from_angle(degrees);
        self
    }

    /// Sets the center of a radial or conic ramp as a fraction of the box.
    ///
    /// See [`Gradient::center`]. Linear ramps ignore it.
    #[inline]
    pub fn center(mut self, x: f32, y: f32) -> Self {
        self.ramp = self.ramp.center(x, y);
        self
    }

    /// Scales a radial ramp's extent; `1.0` (the default) ends at the farthest
    /// corner. See [`Gradient::radius`]. Other shapes ignore it.
    #[inline]
    pub fn radius(mut self, scale: f32) -> Self {
        self.ramp = self.ramp.radius(scale);
        self
    }

    /// Fades `child` through this ramp.
    ///
    /// Shorthand for [`OpacityMaskBox::new`] with this decoration.
    #[inline]
    pub fn child<W: Widget>(self, child: W) -> OpacityMaskBox<Self, W> {
        OpacityMaskBox::new().decoration(self).child(child)
    }
}

impl Decoration for OpacityGradient {
    #[inline]
    fn paint(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, scale: f32) {
        self.ramp.paint(canvas, origin, size, scale);
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.ramp.is_empty()
    }
}

/// Black with `opacity` as its alpha, or `None` when `opacity` is not finite.
///
/// Gradients blend premultiplied, so a black stop's alpha ramps linearly and
/// its color never reaches the screen: only the alpha is used.
#[inline]
fn shade(opacity: f32) -> Option<Color> {
    opacity
        .is_finite()
        .then(|| Color::Rgba(0, 0, 0, (opacity.clamp(0.0, 1.0) * 255.0).round() as u8))
}
