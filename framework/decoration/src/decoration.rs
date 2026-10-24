use std::rc::Rc;

use crate::BoxDecoration;
use aimer_attribute::{ResolvedSize, Vec2d};
use aimer_canvas::Canvas;
use aimer_rubick::ErasedFrom;

mod any_decor;
pub use any_decor::AnyDecor;
mod collections;
mod components;

#[cfg(test)]
mod components_tests;

// SAFETY: Unsizing the null pointer supplies exactly D's Decoration vtable
// with a null data address. The template is never dereferenced.
unsafe impl<D: Decoration + 'static> ErasedFrom<D> for dyn Decoration {
    const TEMPLATE: *const Self = std::ptr::null::<D>() as *const dyn Decoration;
}

/// Paints a box and optionally describes its child insets and clipping.
///
/// `DecorationBox` uses only painting; `Container` also applies child geometry.
///
/// Decorations record element-local commands through an already open [`Canvas`].
/// Coordinates are logical pixels; layout sizes have already been divided by
/// the device scale. Implementations need neither `Clone` nor thread-safe state.
///
/// # Examples
///
/// ```
/// use aimer_attribute::{ResolvedSize, Vec2d};
/// use aimer_canvas::Canvas;
/// use aimer_container::SizedBox;
/// use aimer_decoration::{Decoration, DecorationBox};
/// use aimer_widget::base::Color;
///
/// struct Background;
/// impl Decoration for Background {
///     fn paint(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, _scale: f32) {
///         canvas.fill_color_rect(origin, size, Color::BLUE);
///     }
/// }
/// let widget = DecorationBox::new()
///     .decoration(Background)
///     .child(SizedBox::new().width(120).height(40));
/// ```
pub trait Decoration {
    /// Records decoration commands behind the child.
    ///
    /// `origin` locates the decorated box inside its retained paint list,
    /// including any left/top paint outsets. `size` is finite and nonnegative,
    /// and `scale` is finite and positive. Add `origin` to local coordinates.
    /// Balance any canvas state scopes; unbalanced scopes panic when the
    /// wrapper commits the list. Do not draw children, open or finish another
    /// canvas, change layout, or perform application side effects here.
    fn paint(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, scale: f32);

    /// Paints a layer and propagates its declared background to later layers.
    /// Composite decorations override this to propagate each leaf exactly once.
    #[doc(hidden)]
    #[inline]
    fn paint_layer(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, scale: f32) {
        self.paint(canvas, origin, size, scale);
        canvas.add_background_color(self.background_color());
    }

    /// Paints with a container's fallback background when no background is supplied.
    ///
    /// The default fills the box before calling [`Decoration::paint`]. A
    /// decoration with a shaped background can override this to incorporate
    /// the fallback into its existing background command.
    fn paint_with_background(
        &self,
        canvas: &Canvas,
        origin: Vec2d,
        size: ResolvedSize,
        scale: f32,
        background: Option<aimer_color::prelude::Color>,
    ) {
        if self.background_color().is_none() {
            if let Some(color) = background {
                canvas.fill_color_rect(origin, size, color);
            }
        }
        self.paint(canvas, origin, size, scale);
    }

    /// Returns conservative logical paint outsets in left/top/right/bottom order.
    ///
    /// Include every pixel outside `size`, such as shadows or outlines, across
    /// all retained revisions. The default confines paint to the box. Negative
    /// or nonfinite values are treated as zero. For unchanged size and scale,
    /// changing these declared bounds requires rebuilding the widget.
    #[inline]
    fn paint_outsets(&self, _size: ResolvedSize, _scale: f32) -> [f32; 4] {
        [0.0; 4]
    }

    /// Returns logical child insets in left/top/right/bottom order.
    ///
    /// The default adds no inset. `size` is the logical decorated box size;
    /// `scale` is finite and positive. Negative or nonfinite results are treated
    /// as zero. Geometry must remain unchanged until the owner is rebuilt.
    /// This method is called during layout and must be cheap and side-effect-free.
    #[inline]
    fn content_insets(&self, _size: ResolvedSize, _scale: f32) -> [f32; 4] {
        [0.0; 4]
    }

    /// Returns logical corner radii for clipping the region inside the insets.
    ///
    /// Radii are ordered top-left, top-right, bottom-right, bottom-left.
    /// `None` leaves the child unclipped; `Some([0.0; 4])` clips to a rectangle.
    /// Padding is inside this region. The default is no clip. Geometry must
    /// remain unchanged until the owner is rebuilt.
    #[inline]
    fn child_clip_radius(&self, _size: ResolvedSize, _scale: f32) -> Option<[f32; 4]> {
        None
    }

    /// Returns the background color, if the decoration supplies one.
    ///
    /// Containers use this to select their fallback color and to occlude scroll
    /// events over a painted background. The default supplies no background.
    #[inline]
    fn background_color(&self) -> Option<aimer_color::prelude::Color> {
        None
    }

    /// Returns whether the decoration paints nothing and supplies no geometry.
    ///
    /// The conservative default is `false`. Return `true` only when bypassing
    /// this decoration preserves painting, layout, clipping and interaction.
    #[inline]
    fn is_empty(&self) -> bool {
        false
    }

    /// Identifies the current paint values for retained command reuse.
    ///
    /// The default is suitable for immutable decorations. Interior-mutable
    /// implementations must return a different value when their paint changes
    /// and have their owner request a redraw, for example through
    /// [`aimer_widget::base::WindowHandle::request_redraw`]. Revision checks do
    /// not schedule frames. This query runs during retained traversal and
    /// should be cheap and side-effect-free.
    #[inline]
    fn revision(&self) -> u64 {
        0
    }

    /// Consumes this decoration and erases its concrete type into [`AnyDecor`].
    ///
    /// Small values stay inline. Heap storage uses the active UI allocator, or
    /// Rubick's thread-local pool outside a UI allocation scope. Nothing is
    /// cloned, and an existing `AnyDecor` is returned unchanged.
    ///
    /// ```
    /// use aimer_decoration::{AnyDecor, BorderRadius, BoxShadow, Decoration};
    ///
    /// let layers: Vec<AnyDecor> = vec![
    ///     BorderRadius::from(12).notype(),
    ///     BoxShadow::new().blur(8.0).notype(),
    /// ];
    /// ```
    #[inline]
    fn notype(self) -> AnyDecor
    where
        Self: Sized + 'static,
    {
        AnyDecor::new(self)
    }
}

impl Decoration for BoxDecoration {
    #[inline]
    fn paint(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, scale: f32) {
        self.record_local_v2(
            canvas,
            origin,
            size.width * scale,
            size.height * scale,
            scale,
        );
    }

    #[inline]
    fn paint_with_background(
        &self,
        canvas: &Canvas,
        origin: Vec2d,
        size: ResolvedSize,
        scale: f32,
        background: Option<aimer_color::prelude::Color>,
    ) {
        self.record_local_v2_with_background(
            canvas,
            origin,
            size.width * scale,
            size.height * scale,
            scale,
            background,
        );
    }

    #[inline]
    fn paint_outsets(&self, size: ResolvedSize, scale: f32) -> [f32; 4] {
        self.local_v2_paint_outsets(size.width * scale, size.height * scale, scale)
    }

    #[inline]
    fn content_insets(&self, size: ResolvedSize, _scale: f32) -> [f32; 4] {
        let resolve = |dimension: aimer_attribute::Dimension, parent: f32| {
            match dimension {
                aimer_attribute::Dimension::Auto => 0.0,
                aimer_attribute::Dimension::Px(value) => value,
                aimer_attribute::Dimension::Percent(value) => parent * (value / 100.0),
            }
            .max(0.0)
        };
        [
            resolve(self.border.left.stroke, size.width),
            resolve(self.border.top.stroke, size.height),
            resolve(self.border.right.stroke, size.width),
            resolve(self.border.bottom.stroke, size.height),
        ]
    }

    #[inline]
    fn child_clip_radius(&self, size: ResolvedSize, scale: f32) -> Option<[f32; 4]> {
        let [left, top, right, bottom] = self.content_insets(size, scale);
        let radii = self
            .border_radius
            .resolve(size.width * scale, size.height * scale, scale)
            .map(|radius| radius / scale);
        Some([
            (radii[0] - top.max(left)).max(0.0),
            (radii[1] - top.max(right)).max(0.0),
            (radii[2] - bottom.max(right)).max(0.0),
            (radii[3] - bottom.max(left)).max(0.0),
        ])
    }

    #[inline]
    fn background_color(&self) -> Option<aimer_color::prelude::Color> {
        self.background_color.get()
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    #[inline]
    fn revision(&self) -> u64 {
        self.background_color
            .get()
            .map_or(0, |color| u64::from(color.as_u32()) + 1)
    }
}

impl<D: Decoration + ?Sized> Decoration for Box<D> {
    #[inline]
    fn paint_layer(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, scale: f32) {
        self.as_ref().paint_layer(canvas, origin, size, scale);
    }

    #[inline]
    fn paint(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, scale: f32) {
        self.as_ref().paint(canvas, origin, size, scale);
    }

    #[inline]
    fn paint_with_background(
        &self,
        canvas: &Canvas,
        origin: Vec2d,
        size: ResolvedSize,
        scale: f32,
        background: Option<aimer_color::prelude::Color>,
    ) {
        self.as_ref()
            .paint_with_background(canvas, origin, size, scale, background);
    }

    #[inline]
    fn paint_outsets(&self, size: ResolvedSize, scale: f32) -> [f32; 4] {
        self.as_ref().paint_outsets(size, scale)
    }

    #[inline]
    fn content_insets(&self, size: ResolvedSize, scale: f32) -> [f32; 4] {
        self.as_ref().content_insets(size, scale)
    }

    #[inline]
    fn child_clip_radius(&self, size: ResolvedSize, scale: f32) -> Option<[f32; 4]> {
        self.as_ref().child_clip_radius(size, scale)
    }

    #[inline]
    fn background_color(&self) -> Option<aimer_color::prelude::Color> {
        self.as_ref().background_color()
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.as_ref().is_empty()
    }

    #[inline]
    fn revision(&self) -> u64 {
        self.as_ref().revision()
    }
}

impl<D: Decoration + ?Sized> Decoration for Rc<D> {
    #[inline]
    fn paint_layer(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, scale: f32) {
        self.as_ref().paint_layer(canvas, origin, size, scale);
    }

    #[inline]
    fn paint(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, scale: f32) {
        self.as_ref().paint(canvas, origin, size, scale);
    }

    #[inline]
    fn paint_with_background(
        &self,
        canvas: &Canvas,
        origin: Vec2d,
        size: ResolvedSize,
        scale: f32,
        background: Option<aimer_color::prelude::Color>,
    ) {
        self.as_ref()
            .paint_with_background(canvas, origin, size, scale, background);
    }

    #[inline]
    fn paint_outsets(&self, size: ResolvedSize, scale: f32) -> [f32; 4] {
        self.as_ref().paint_outsets(size, scale)
    }

    #[inline]
    fn content_insets(&self, size: ResolvedSize, scale: f32) -> [f32; 4] {
        self.as_ref().content_insets(size, scale)
    }

    #[inline]
    fn child_clip_radius(&self, size: ResolvedSize, scale: f32) -> Option<[f32; 4]> {
        self.as_ref().child_clip_radius(size, scale)
    }

    #[inline]
    fn background_color(&self) -> Option<aimer_color::prelude::Color> {
        self.as_ref().background_color()
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.as_ref().is_empty()
    }

    #[inline]
    fn revision(&self) -> u64 {
        self.as_ref().revision()
    }
}
