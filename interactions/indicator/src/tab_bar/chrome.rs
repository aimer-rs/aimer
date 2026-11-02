//! The container a [`TabBar`](super::TabBar) draws behind and around its items.

use std::rc::Rc;

use aimer_attribute::{Dimension, ResolvedSize, Vec2d};
use aimer_canvas::Canvas;
use aimer_container::Container;
use aimer_decoration::Decoration;
use aimer_space::Alignment;
use aimer_style::{LayoutSpacing, Spacing};
use aimer_widget::base::Color;
use aimer_widget::AnyWidget;

/// The space between the bar's edge and its items unless the caller says
/// otherwise.
pub(super) const DEFAULT_PADDING: u32 = 6;

/// Everything about the bar's container except what it is painted with by
/// default, which depends on the bar's colors.
///
/// Held by both the widget and its retained state, so it has to be cheap to
/// clone: the only shared part is the decoration, behind an `Rc`.
#[derive(Clone)]
pub(super) struct BarChrome {
    /// The caller's decoration, replacing the default pill.
    pub(super) decoration: Option<Rc<dyn Decoration>>,
    pub(super) padding: LayoutSpacing,
    pub(super) margin: LayoutSpacing,
    pub(super) width: Dimension,
    pub(super) height: Dimension,
    pub(super) alignment: Option<Alignment>,
    /// Whether the decoration's own clip of its child applies.
    pub(super) clip_content: bool,
}

impl BarChrome {
    pub(super) fn new() -> Self {
        Self {
            decoration: None,
            padding: LayoutSpacing::all(DEFAULT_PADDING),
            margin: LayoutSpacing::default(),
            width: Dimension::Auto,
            height: Dimension::Auto,
            alignment: None,
            clip_content: true,
        }
    }

    /// The outer size of the bar, margin excluded, for items that fill
    /// `content` together.
    ///
    /// Unless an axis was given a pixel size, it is the content plus padding
    /// plus the decoration's content insets, so the items always have the room
    /// they need. Percentage padding and percentage sizes depend on a parent
    /// that is not known yet, so they are left out of the computed size, and a
    /// percentage size is resolved by the container itself.
    pub(super) fn extent(&self, content: ResolvedSize, scale: f32) -> ResolvedSize {
        let padded = ResolvedSize {
            width: content.width + pixels(self.padding.left) + pixels(self.padding.right),
            height: content.height + pixels(self.padding.top) + pixels(self.padding.bottom),
        };
        let [left, top, right, bottom] = self
            .decoration
            .as_ref()
            .map_or([0.0; 4], |decoration| decoration.content_insets(padded, scale))
            .map(|inset| if inset.is_finite() { inset.max(0.0) } else { 0.0 });
        ResolvedSize {
            width: axis(self.width, padded.width + left + right),
            height: axis(self.height, padded.height + top + bottom),
        }
    }

    /// Wraps `child` in the container, painted with `decoration`.
    ///
    /// `size` is what [`extent`](Self::extent) returned. The container is given
    /// it as a pixel size unless the caller asked for a percentage.
    pub(super) fn frame<D: Decoration + 'static>(
        &self,
        decoration: D,
        size: ResolvedSize,
        child: AnyWidget,
    ) -> AnyWidget {
        let container = Container::new()
            .width(match self.width {
                Dimension::Percent(_) => self.width,
                _ => Dimension::Px(size.width),
            })
            .height(match self.height {
                Dimension::Percent(_) => self.height,
                _ => Dimension::Px(size.height),
            })
            .padding(self.padding)
            .margin(self.margin);
        if self.clip_content {
            container.decoration(decoration).box_child(child)
        } else {
            container.decoration(Unclipped(decoration)).box_child(child)
        }
    }
}

/// The logical pixels of a padding side. A percentage has no parent to
/// resolve against here and counts as nothing.
fn pixels(spacing: Spacing) -> f32 {
    match spacing {
        Spacing::Px(px) => px as f32,
        Spacing::Percent(_) | Spacing::None => 0.0,
    }
}

/// An axis's size: the caller's pixel size if it is usable, `computed`
/// otherwise.
fn axis(requested: Dimension, computed: f32) -> f32 {
    match requested {
        Dimension::Px(px) if px.is_finite() && px >= 0.0 => px,
        _ => computed,
    }
}

/// A decoration that lets its child paint beyond the decoration's shape.
///
/// Everything but the clip is forwarded, which is what a lens that is lifted
/// off the end of the bar needs.
pub(super) struct Unclipped<D>(pub(super) D);

impl<D: Decoration> Decoration for Unclipped<D> {
    #[inline]
    fn paint(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, scale: f32) {
        self.0.paint(canvas, origin, size, scale);
    }

    #[inline]
    fn paint_layer(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, scale: f32) {
        self.0.paint_layer(canvas, origin, size, scale);
    }

    #[inline]
    fn paint_with_background(
        &self,
        canvas: &Canvas,
        origin: Vec2d,
        size: ResolvedSize,
        scale: f32,
        background: Option<Color>,
    ) {
        self.0.paint_with_background(canvas, origin, size, scale, background);
    }

    #[inline]
    fn paint_outsets(&self, size: ResolvedSize, scale: f32) -> [f32; 4] {
        self.0.paint_outsets(size, scale)
    }

    #[inline]
    fn content_insets(&self, size: ResolvedSize, scale: f32) -> [f32; 4] {
        self.0.content_insets(size, scale)
    }

    #[inline]
    fn child_clip_radius(&self, _size: ResolvedSize, _scale: f32) -> Option<[f32; 4]> {
        None
    }

    #[inline]
    fn background_color(&self) -> Option<Color> {
        self.0.background_color()
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[inline]
    fn revision(&self) -> u64 {
        self.0.revision()
    }
}

#[cfg(test)]
mod tests {
    use aimer_decoration::BoxDecoration;

    use super::*;

    const SIZE: ResolvedSize = ResolvedSize { width: 200.0, height: 80.0 };

    #[test]
    fn a_box_decoration_clips_its_child_and_the_unclipped_one_does_not() {
        let decoration = BoxDecoration::new().border_radius(20);
        assert!(decoration.child_clip_radius(SIZE, 1.0).is_some());
        assert_eq!(Unclipped(decoration).child_clip_radius(SIZE, 1.0), None);
    }

    #[test]
    fn unclipped_forwards_everything_else() {
        let decoration = BoxDecoration::new().background_color(Color::Rgba(1, 2, 3, 255));
        // `BoxDecoration` has an inherent builder of the same name.
        let color = Decoration::background_color(&decoration);
        let revision = decoration.revision();
        let unclipped = Unclipped(decoration);
        assert_eq!(Decoration::background_color(&unclipped), color);
        assert_eq!(unclipped.revision(), revision);
        assert!(!unclipped.is_empty());
    }

    #[test]
    fn an_unusable_pixel_size_falls_back_to_the_computed_one() {
        assert_eq!(axis(Dimension::Px(-1.0), 7.0), 7.0);
        assert_eq!(axis(Dimension::Px(f32::NAN), 7.0), 7.0);
        assert_eq!(axis(Dimension::Px(0.0), 7.0), 0.0);
    }
}
