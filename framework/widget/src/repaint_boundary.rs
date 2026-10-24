//! Explicit retained-paint promotion for stable widget subtrees.
//!
//! [`RepaintBoundary`] is an optimization hint, not a promise that a widget
//! receives a permanent GPU texture. Stable, bounded, retention-safe elements
//! can enter the scene tree automatically; this boundary is the explicit
//! priority hint for a subtree whose pixels should be isolated even when it is
//! small or dynamic. Oversized or unsupported paint stays on the ordinary
//! command path with identical widget semantics.

use std::marker::PhantomData;

use aimer_attribute::position::Vec2d;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_events::element::ElementEvent;

use crate::base::BuildContext;
use crate::components::element::{Element, EventDispatchContext};
use crate::components::event_element::{EventElement, EventResult};
use crate::components::layout_element::LayoutElement;
use crate::components::rebuildable::Rebuildable;
use crate::components::visitor_element::VisitorElement;
use crate::widget::child_builder::ChildBuilder;
use crate::widget::stateful::{State, StateUpdater, StatefulElement, StatefulWidget};
use crate::{AnyElement, AnyWidget, Drawable, RequiredChild, Widget};

/// Marks one subtree as a priority candidate for retained compositor paint.
///
/// The boundary remains transparent to layout, focus, hit testing, events,
/// and reconciliation. It only changes the paint policy: if the child opts
/// into the framework's paint-only contract and its recorded commands are
/// safe, later frames can compose a renderer-owned surface without walking or
/// rerasterizing the child. Otherwise the child is drawn normally. Elements
/// that satisfy the same stable/bounded contract without this boundary may be
/// retained automatically when their command stream is large enough to justify
/// the extra surface.
///
/// This is intentionally not a one-widget/one-texture rule. The compositor may
/// keep the child as replayable commands or decline promotion altogether when
/// that is safer or cheaper.
///
/// # Examples
///
/// ```
/// use aimer_widget::RepaintBoundary;
///
/// # struct Panel;
/// # impl aimer_widget::PortableWidget for Panel {}
/// # impl aimer_widget::Widget for Panel {
/// #     fn to_element(self, _ctx: &aimer_widget::base::BuildContext) -> aimer_widget::AnyElement {
/// #         unreachable!("example only")
/// #     }
/// # }
/// let panel = RepaintBoundary::new().child(Panel);
/// ```
#[derive(aimer_macro::PortableWidget)]
#[portable_widget(id = "aimer_widget::RepaintBoundary", schema_only)]
pub struct RepaintBoundary<W = RequiredChild> {
    #[portable_child]
    child: ChildBuilder,
    #[portable_skip]
    marker: PhantomData<W>,
}

impl RepaintBoundary {
    /// Creates an incomplete boundary builder.
    #[inline]
    pub fn new() -> Self {
        Self {
            child: ChildBuilder::required(),
            marker: PhantomData,
        }
    }
}

impl Default for RepaintBoundary {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<W> RepaintBoundary<W> {
    /// Attaches the child and completes this builder.
    #[inline]
    pub fn child<C: Widget + 'static>(self, child: C) -> RepaintBoundary<C> {
        RepaintBoundary {
            child: ChildBuilder::from_widget(child),
            marker: PhantomData,
        }
    }

    /// Attaches a child and erases the completed boundary widget.
    #[inline]
    pub fn box_child<C: Widget + 'static>(self, child: C) -> AnyWidget {
        self.child(child).boxed()
    }
}

impl<W: Widget + 'static> StatefulWidget for RepaintBoundary<W> {
    type State = RepaintBoundaryState<W>;

    fn create_state(self) -> Self::State {
        RepaintBoundaryState {
            child: self.child,
            marker: PhantomData,
        }
    }
}

impl<W: Widget + 'static> Widget for RepaintBoundary<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "RepaintBoundary", None)
            .0
            .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "RepaintBoundary"
    }
}

/// Retained state that keeps the boundary child in a [`ChildBuilder`].
#[doc(hidden)]
pub struct RepaintBoundaryState<W: Widget + 'static> {
    child: ChildBuilder,
    marker: PhantomData<W>,
}

impl<W: Widget + 'static> State<RepaintBoundary<W>> for RepaintBoundaryState<W> {
    fn init_state(&mut self, _updater: StateUpdater<Self>) {}

    fn adopt_config_from(&mut self, new: Self) {
        self.child = new.child;
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        RepaintBoundaryTarget {
            child: self.child.clone().to_element(ctx),
        }
    }
}

/// Transparent retained element used as the stateful paint owner's child.
struct RepaintBoundaryTarget {
    child: AnyElement,
}

impl crate::widget::PortableWidget for RepaintBoundaryTarget {}

impl Widget for RepaintBoundaryTarget {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        self.child
    }

    fn debug_name(&self) -> &'static str {
        "RepaintBoundary"
    }
}

impl VisitorElement for RepaintBoundaryTarget {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "RepaintBoundary"
    }
}

impl Rebuildable for RepaintBoundaryTarget {
    fn rebuild_if_dirty(&self, ctx: &BuildContext) {
        self.child.rebuild_if_dirty(ctx);
    }

    fn mark_needs_rebuild(&self) {
        self.child.mark_needs_rebuild();
    }
}

impl EventElement for RepaintBoundaryTarget {
    fn on_event(&self, _event: &ElementEvent) -> EventResult {
        EventResult::ignored()
    }

    fn on_event_with_context(
        &self,
        event: &ElementEvent,
        context: &mut EventDispatchContext<'_, '_>,
    ) -> EventResult {
        let pos = context.position();
        context.dispatch_child(self.child.as_ref(), pos, event)
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn structural_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn focus_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.child.focus_children(visitor);
    }

    fn hit_test_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn hit_test_children_reversed<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn hit_test_children_at<'a>(
        &'a self,
        pos: Vec2d,
        visitor: &mut dyn FnMut(&'a dyn Element),
    ) {
        self.child.hit_test_children_at(pos, visitor);
    }

    fn hit_test_children_at_reversed<'a>(
        &'a self,
        pos: Vec2d,
        visitor: &mut dyn FnMut(&'a dyn Element),
    ) {
        self.child.hit_test_children_at_reversed(pos, visitor);
    }

    fn has_overlapping_hit_targets(&self) -> bool {
        self.child.has_overlapping_hit_targets()
    }

    fn occludes_pointer_at(&self, pos: Vec2d) -> bool {
        self.child.occludes_pointer_at(pos)
    }

    fn occlusion_children_at<'a>(&'a self, pos: Vec2d, visitor: &mut dyn FnMut(&'a dyn Element)) {
        self.child.occlusion_children_at(pos, visitor);
    }
}

impl LayoutElement for RepaintBoundaryTarget {
    fn pos(&self) -> Option<Vec2d> {
        self.child.pos()
    }

    fn size(&self) -> Option<Size> {
        self.child.size()
    }

    fn layout(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.layout(ctx)
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.computed_size(ctx)
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.content_size(ctx)
    }

    fn layer(&self) -> u32 {
        self.child.layer()
    }

    fn flex(&self) -> Option<f32> {
        self.child.flex()
    }

    fn is_layout_stable(&self) -> bool {
        self.child.is_layout_stable()
    }

    fn get_size_from_child(&self) -> Option<Size> {
        self.child.get_size_from_child()
    }

    fn invalidate_layout(&self) {
        self.child.invalidate_layout();
    }

    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.child.pos_start_end()
    }
}

impl Drawable for RepaintBoundaryTarget {
    fn update(&self, ctx: &BuildContext) {
        self.child.update(ctx);
    }

    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        self.child.sync_paint_geometry(ctx);
    }

}

#[cfg(all(test, feature = "portable-guest"))]
mod portable_tests {
    use super::RepaintBoundary;
    use crate::portable::PortableWidgetSchema;
    use crate::RequiredChild;

    #[test]
    fn repaint_boundary_schema_exposes_one_required_child() {
        let schema = <RepaintBoundary<RequiredChild> as PortableWidgetSchema>::SCHEMA;

        assert_eq!(schema.children(), aimer_anteros::ChildCardinality::exactly(1));
    }
}

#[cfg(all(test, not(target_arch = "wasm32"), not(feature = "portable-guest")))]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::sync::OnceLock;

    use aimer_attribute::size::ResolvedSize;
    use super::*;
    use crate::components::context::WindowHandle;

    fn context() -> BuildContext<'static> {
        let canvas = {
            let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
            aimer_canvas::FrameCanvas::new(inner)
        };
        static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
        let runtime = RUNTIME.get_or_init(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
        });
        let _guard = runtime.enter();
        BuildContext::new(
            canvas,
            ResolvedSize {
                width: 8.0,
                height: 8.0,
            },
            1.0,
            Default::default(),
            Default::default(),
            WindowHandle::headless(Default::default(), 1.0),
            tokio::runtime::Handle::current(),
        )
    }

    struct ProbeWidget {
        draws: Rc<Cell<usize>>,
    }

    struct ProbeElement {
        draws: Rc<Cell<usize>>,
    }

    impl crate::widget::PortableWidget for ProbeWidget {}

    impl Widget for ProbeWidget {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            ProbeElement { draws: self.draws }.boxed()
        }
    }

    impl VisitorElement for ProbeElement {
        fn debug_name(&self) -> &'static str {
            "RepaintBoundaryProbe"
        }
    }

    impl Rebuildable for ProbeElement {}
    impl EventElement for ProbeElement {}
    impl LayoutElement for ProbeElement {
        fn is_layout_stable(&self) -> bool {
            true
        }
    }

    impl Drawable for ProbeElement {
        fn update(&self, _ctx: &BuildContext) {
            self.draws.set(self.draws.get() + 1);
        }
    }

    #[test]
    fn the_boundary_updates_its_child_every_frame() {
        let ctx = context();
        let draws = Rc::new(Cell::new(0));
        let element = RepaintBoundary::new()
            .child(ProbeWidget {
                draws: draws.clone(),
            })
            .to_element(&ctx);

        element.update(&ctx);
        element.update(&ctx);

        assert_eq!(draws.get(), 2);
    }
}
