use std::rc::Rc;

use aimer_attribute::bounds::Bounds;
use aimer_attribute::position::Vec2d;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_events::element::ElementEvent;
use aimer_macro::Rebuildable;
use aimer_widget::InteractionBounds;
use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, AnyWidget, Drawable, Element, EventElement, EventResult, LayoutElement,
    RequiredChild, VisitorElement, Widget,
};

/// A shared, cheap-to-clone reference to the on-screen rectangle of an
/// [`Anchor`].
///
/// The handle is written by the anchored widget on every layout and paint pass
/// and read by [`crate::Floating`] while it resolves its position, so a panel
/// keeps following its trigger when the trigger moves, resizes, or scrolls.
///
/// A handle that was never painted reports [`None`]; callers must treat that as
/// "position unknown" rather than "at the origin".
///
/// Two handles compare equal only when they refer to the same tracked anchor,
/// never when they merely happen to hold the same rectangle.
///
/// # Example
///
/// ```rust
/// use aimer_attribute::bounds::Bounds;
/// use aimer_modal::AnchorHandle;
///
/// let handle = AnchorHandle::new();
/// assert!(handle.bounds().is_none());
///
/// let tracker = handle.clone();
/// tracker.set_bounds(Bounds::new(10.0, 20.0, 100.0, 30.0));
///
/// assert_eq!(handle.bounds(), Some(Bounds::new(10.0, 20.0, 100.0, 30.0)));
/// ```
#[derive(Clone, Debug)]
pub struct AnchorHandle {
    bounds: Rc<InteractionBounds>,
}

impl PartialEq for AnchorHandle {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.bounds, &other.bounds)
    }
}

impl Default for AnchorHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl AnchorHandle {
    /// Creates a handle that does not track any rectangle yet.
    #[inline]
    pub fn new() -> Self {
        Self {
            bounds: Rc::new(InteractionBounds::new()),
        }
    }

    /// Records the anchor rectangle in logical, viewport-relative units.
    #[inline]
    pub fn set_bounds(&self, bounds: Bounds) {
        self.bounds
            .save(1.0, bounds.x, bounds.y, bounds.width, bounds.height);
    }

    /// Returns the last recorded rectangle, or [`None`] before the first pass.
    #[inline]
    pub fn bounds(&self) -> Option<Bounds> {
        self.bounds.get_bounds()
    }

    /// Returns whether a rectangle has been recorded.
    #[inline]
    pub fn is_tracked(&self) -> bool {
        self.bounds.get_bounds().is_some()
    }
}

/// Tracks the position of its child so a [`crate::Floating`] panel can be
/// pinned to it.
///
/// `Anchor` is layout-transparent: it reports the size, flex factor and content
/// size of its child unchanged, and only records where the child ended up.
///
/// # Example
///
/// ```rust
/// use aimer_container::SizedBox;
/// use aimer_modal::{Anchor, AnchorHandle};
///
/// let handle = AnchorHandle::new();
/// let trigger = Anchor::new().handle(handle.clone())
///                            .child(SizedBox::new().width(120).height(32));
///
/// assert_eq!(trigger.handle_value(), handle);
/// ```
#[derive(aimer_macro::PortableWidget)]
#[portable_widget(id = "aimer_modal::Anchor", schema_only)]
pub struct Anchor<W = RequiredChild> {
    #[portable_child]
    child: W,
    #[portable_skip]
    handle: AnchorHandle,
}

impl Default for Anchor {
    fn default() -> Self {
        Self::new()
    }
}

impl Anchor {
    /// Creates an anchor that owns a freshly created [`AnchorHandle`].
    #[inline]
    pub fn new() -> Self {
        Self {
            child: RequiredChild,
            handle: AnchorHandle::new(),
        }
    }

    /// Reports the tracked rectangle through an externally owned handle.
    #[inline]
    pub fn handle(mut self, handle: AnchorHandle) -> Self {
        self.handle = handle;
        self
    }

    /// Attaches the tracked child and completes this builder.
    #[inline]
    pub fn child<W: Widget>(self, child: W) -> Anchor<W> {
        Anchor {
            child,
            handle: self.handle,
        }
    }

    /// Attaches and erases the tracked child.
    #[inline]
    pub fn box_child<W: Widget + 'static>(self, child: W) -> AnyWidget {
        self.child(child).boxed()
    }
}

impl<W> Anchor<W> {
    /// Returns the handle this anchor writes to.
    #[inline]
    pub fn handle_value(&self) -> AnchorHandle {
        self.handle.clone()
    }
}

impl<W: Widget + 'static> Widget for Anchor<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        RawAnchor {
            child: self.child.to_element(ctx),
            handle: self.handle.clone(),
        }
        .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "Anchor"
    }
}

#[derive(Rebuildable)]
struct RawAnchor {
    child: AnyElement,
    handle: AnchorHandle,
}

impl RawAnchor {
    fn track(&self, ctx: &BuildContext, size: ResolvedSize) {
        let (abs_x, abs_y) = ctx.canvas.get_transform_translation();
        let scale = if ctx.scale > 0.0 { ctx.scale } else { 1.0 };
        self.handle.set_bounds(Bounds::new(
            abs_x / scale,
            abs_y / scale,
            size.width / scale,
            size.height / scale,
        ));
    }
}

impl Drawable for RawAnchor {
    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint_local_v2(&self, _ctx: &BuildContext) {}

    /// The anchor reports its child's rectangle, which a floating panel is
    /// pinned to.
    fn retained_v2_interaction_size(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        Some(self.child.computed_size(ctx))
    }

    #[inline]
    fn adopt_retained_v2_interaction_source(&self, source: aimer_widget::InteractionSource) {
        self.handle.bounds.adopt(source);
    }

    #[inline]
    fn retained_v2_interaction_disagreement(
        &self,
    ) -> Option<aimer_widget::InteractionDisagreement> {
        self.handle.bounds.disagreement("Anchor")
    }

    fn update(&self, ctx: &BuildContext) {
        self.track(ctx, self.child.computed_size(ctx));
        self.child.update(ctx);
    }

    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        self.track(ctx, self.child.computed_size(ctx));
        self.child.sync_paint_geometry(ctx);
    }
}

impl EventElement for RawAnchor {
    fn on_event(&self, _event: &ElementEvent) -> EventResult {
        EventResult::ignored()
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }
}

impl LayoutElement for RawAnchor {
    fn size(&self) -> Option<Size> {
        self.child.size()
    }

    fn layout(&self, ctx: &BuildContext) -> ResolvedSize {
        let size = self.child.layout(ctx);
        self.track(ctx, size);
        size
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.computed_size(ctx)
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.content_size(ctx)
    }

    fn flex(&self) -> Option<f32> {
        self.child.flex()
    }

    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.handle.bounds().map(|bounds| {
            (
                Vec2d {
                    x: bounds.x,
                    y: bounds.y,
                },
                Vec2d {
                    x: bounds.x + bounds.width,
                    y: bounds.y + bounds.height,
                },
            )
        })
    }
}

impl VisitorElement for RawAnchor {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "Anchor"
    }
}

#[cfg(test)]
mod tests {
    use aimer_container::ZeroSizedBox;

    use super::*;

    #[test]
    fn a_fresh_handle_tracks_nothing() {
        let handle = AnchorHandle::new();

        assert!(!handle.is_tracked());
        assert_eq!(handle.bounds(), None);
    }

    #[test]
    fn clones_of_a_handle_observe_the_same_rectangle() {
        let handle = AnchorHandle::new();
        let clone = handle.clone();

        clone.set_bounds(Bounds::new(4.0, 8.0, 16.0, 32.0));

        assert!(handle.is_tracked());
        assert_eq!(handle.bounds(), Some(Bounds::new(4.0, 8.0, 16.0, 32.0)));
    }

    #[test]
    fn the_latest_rectangle_replaces_the_previous_one() {
        let handle = AnchorHandle::new();

        handle.set_bounds(Bounds::new(0.0, 0.0, 10.0, 10.0));
        handle.set_bounds(Bounds::new(5.0, 5.0, 20.0, 20.0));

        assert_eq!(handle.bounds(), Some(Bounds::new(5.0, 5.0, 20.0, 20.0)));
    }

    #[test]
    fn attaching_a_child_keeps_the_configured_handle() {
        let handle = AnchorHandle::new();
        let anchor = Anchor::new().handle(handle.clone()).child(ZeroSizedBox);

        assert_eq!(anchor.handle_value(), handle);
    }

    #[test]
    fn an_anchor_without_a_configured_handle_owns_its_own() {
        let anchor = Anchor::new().child(ZeroSizedBox);

        assert!(!anchor.handle_value().is_tracked());
        assert_ne!(anchor.handle_value(), AnchorHandle::new());
    }
}
