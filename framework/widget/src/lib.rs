extern crate self as aimer_widget;

mod async_builder;
pub mod components;
mod element_invalidation;
mod event_tree;
pub mod focus_scope;
pub mod focusable;
mod frame_work_stats;
mod hover_reconciliation;
pub mod key;
pub mod layout_cache;
pub mod page_storage;
mod paint_damage;
pub mod platform_brightness;
pub mod pointer_claim;
mod pointer_occlusion;
#[doc(hidden)]
pub mod portable;
mod rebuild_stats;
pub mod reconcile;
mod reconciliation_plan;
pub mod repaint_boundary;
pub mod safe_area;
#[doc(hidden)]
pub mod testing;
mod widget;
pub mod window_metrics;

/// An Empty Widget that using as placeholder
///
/// ## Example
///
/// ```rust ignore
/// pub struct MyWidget<W = EmptyWidget> {
///     child: W,
///     // any fields here
/// }
/// ```
pub struct RequiredChild;

#[derive(Default)]
pub struct CalledCounter(Cell<(u32, &'static str)>);

unsafe impl Send for CalledCounter {}
unsafe impl Sync for CalledCounter {}
impl CalledCounter {
    pub const fn new() -> Self {
        Self(Cell::new((0, "NotSet")))
    }

    pub fn increase(&self) {
        self.0.update(|(c, l)| (c + 1, l))
    }

    pub fn reset(&self) {
        self.0.update(|(c, l)| (0, l))
    }

    pub fn set_label(&self, label: &'static str) {
        self.0.update(|(c, _)| (0, label))
    }

    pub fn print(&self) {
        let (c, l) = self.0.get();
        debug!("{}: {}", l, c);
    }
}
pub static CALLED: CalledCounter = CalledCounter::new();

#[doc(hidden)]
pub use crate::components::context::{InheritedStates, RetainedNodeHandle};
pub use crate::components::diagnostics::{
    ErrorElement, ErrorWidget, OverflowEdges, OverflowIndicator, detect_overflow,
};
#[doc(hidden)]
pub use crate::components::drawable::{
    CompositorAnimationDecision, CompositorAnimationFrame, CompositorTransform, Drawable,
};
pub use crate::components::element::{
    AnyElement, Element, ElementId, ElementNodeMap, ElementPath, EventDispatchContext, EventDispatcher,
    begin_event_frame, begin_paint_frame, element_tree_generation, focus_is_settling_between_events,
    frame_instant,
    has_active_v2_render_presentation, has_active_v2_render_tree, keep_ticking_elements_reachable,
    layout_invalidation_generation, live_children_generation, mark_paint_damage,
    mark_paint_damage_full, notify_element_tree_changed, notify_hosted_element_tree_changed,
    notify_layout_changed, notify_live_children_changed, notify_retained_render_structure_changed,
    rebuild_invalidation_generation, retained_render_structure_generation, set_rebuild_source_path,
    take_declined_paint_elements, take_paint_frame_damage, take_unmapped_draws,
    update_v2_render_node_geometry, with_v2_render_tree_context,
};
#[doc(hidden)]
pub use crate::components::element::{
    KeepReachable, ScopedCursor, VisibleWindow, animation_only_targets, has_scoped_rebuilds_since,
    is_animation_only_frame, keep_element_reachable, may_escape_ancestor_clips,
    may_skip_settled_offscreen, note_state_update_marked, preserving_dirty_paths,
    rebuild_descent_count, rebuild_dirty_children, rebuild_keepalive_count,
    rebuild_replaced_subtree, without_placement_marks, rebuild_state_mark_count, rebuild_visit_count,
    request_isolated_animation_frame, request_isolated_subtree_animation_frame,
    retained_element_settled, retained_visible_window, retained_visible_window_of_current,
    run_animation_only_pass, scoped_rebuild_cursor, scoped_rebuilds_since,
    set_animation_only_frame, set_frame_rebuilt_roots, set_scroll_only_frame,
    set_settled_offscreen_skip, stable_generation_cursor, stable_generations_since,
    structural_search_count, traversal_counts, unscoped_element_tree_generation,
    with_v2_animation_only_context,
};
#[cfg(feature = "frame-stats")]
pub use crate::components::element::{
    reset_draw_traversal_count, reset_routed_event_visit_count, take_draw_traversal_count,
    take_routed_event_visit_count,
};
pub use crate::components::event_element::{
    CaptureRequest, EventElement, EventResult, EventTreeRole, FollowUp, PointerKey,
};
pub use crate::components::interaction_bounds::{
    InteractionBounds, InteractionDisagreement, InteractionSource,
};
pub use crate::components::layout_element::LayoutElement;
pub use crate::components::rebuildable::Rebuildable;
pub use crate::components::visitor_element::VisitorElement;
#[doc(hidden)]
pub use crate::element_invalidation::{
    ElementChangeKind, ElementInvalidation, ElementInvalidationBatch, ElementInvalidationBounds,
    ElementInvalidationRevisions,
};
pub use crate::focus_scope::FocusScope;
pub use crate::focusable::{Focusable, FocusableState, RawFocusable};
pub use crate::frame_work_stats::FrameWorkStats;
pub use crate::frame_work_stats::{
    record_element_index_lookup, record_hit_test_visit, record_invalidation_coalesced,
    record_invalidation_queued, record_layout_call, record_paint_call, record_redraw_request,
    record_root_draw_call, record_scroll_event, record_scroll_offset_update, record_scroll_step,
    record_smoothing_step, record_stale_element_id, record_state_update, reset_frame_work_stats,
    take_frame_work_stats,
};
#[doc(hidden)]
pub use crate::hover_reconciliation::{
    mouse_region_hover_reconciliation_deferred, pointer_capture_active,
    request_mouse_region_hover_reconciliation, set_pointer_capture_active,
    with_deferred_mouse_region_hover_reconciliation,
};
#[doc(hidden)]
pub use crate::paint_damage::PaintDamageTracker;
pub use crate::rebuild_stats::RebuildStats;
#[cfg(feature = "frame-stats")]
pub use crate::rebuild_stats::{reset as reset_rebuild_stats, take as take_rebuild_stats};
/// Keyboard focus lives in its own crate; it is re-exported here so
/// `aimer_widget::focus::*` keeps naming the focus system.
pub use aimer_focus as focus;
pub use aimer_focus::{
    FocusBehavior, FocusCallback, FocusCandidate, FocusManager, FocusNode, FocusTransition,
    FocusTrap, FocusTrapId, active_focus_trap,
};
use std::cell::Cell;
use std::ops::Deref;

pub mod base {
    pub use aimer_attribute::dimension::Dimension;
    pub use aimer_attribute::position::Vec2d;
    pub use aimer_attribute::size::{ResolvedSize, Size};
    pub use aimer_color::prelude::*;

    #[doc(hidden)]
    pub use crate::components::context::BuildConsumer;
    pub use crate::components::context::{BuildContext, WindowHandle};
}
pub use crate::async_builder::{
    AsyncBuilder, AsyncSnapshot, AsyncSnapshotError, FutureSnapshotExt, SnapshotTask,
};
pub use crate::components::element::{broadcast_event, dispatch_event, dispatch_focused_event};
pub use crate::key::Key;
pub use crate::layout_cache::LayoutCache;
pub use crate::platform_brightness::{Brightness, platform_brightness, set_platform_brightness};
pub use crate::pointer_claim::{
    claim_pointer, claimed_pointer_count, is_pointer_claimed, release_all_pointers, release_pointer,
};
pub use crate::pointer_occlusion::pointer_occluded_at;
#[doc(hidden)]
pub use crate::pointer_occlusion::request_pointer_view_frame;
pub use crate::reconciliation_plan::{
    ReconciliationMatch, ReconciliationMatchKind, ReconciliationPlan, ReconciliationPlanError,
    plan_element_reconciliation,
};
pub use crate::repaint_boundary::RepaintBoundary;
pub use crate::safe_area::{SafeAreaInsets, safe_area_insets, set_safe_area_insets};
pub use crate::widget::child_builder::ChildBuilder;
#[doc(hidden)]
pub use crate::widget::stateful::{
    State, StateReadGuard, StateUpdater, StatefulElement, StatefulWidget,
};
pub use crate::widget::stateless::{NamedWidget, StatelessElement, StatelessWidget};
pub use crate::widget::{AnyWidget, DynWidget, PortableWidget, Widget};
pub use crate::window_metrics::{WindowMetrics, notify_window_metrics_changed};
pub use aimer_canvas::{TextHorizontalAlign, TextOverflowMode};
pub use aimer_macro::{PortableValue, PortableWidget, main, widget};
use aimer_utils::debug;

/// Carries the live state of an old element subtree into the subtree replacing
/// it, and transfers its logical identities onto the replacement.
///
/// This is the step a rebuild performs between building the replacement and
/// installing it: every state-owning element in `new` that corresponds to one in
/// `old` adopts its runtime state, so a `set_state` above a subtree does not
/// reset the widgets inside it. Identities travel the same way, because the
/// dispatcher's captured pointers and focus records name elements by id: a
/// pointer captured before the rebuild must still reach the element that
/// replaced its owner, or the capture turns into a ghost and swallows every
/// event routed its way. Containers that materialize their children on demand
/// also exchange them here, through
/// [`Rebuildable::take_retained_children`].
///
/// Safe to call on any pair: elements that do not correspond are left alone.
#[inline]
pub fn carry_element_state(old: &dyn Element, new: &dyn Element, ctx: &base::BuildContext) {
    plan_element_reconciliation(old, new)
        .commit(ctx)
        .expect("fresh reconciliation plan must remain valid until commit");
}

#[cfg(test)]
mod tests {
    use std::any::Any;
    use std::cell::Cell;

    use crate::base::{BuildContext, WindowHandle};
    use crate::components::element::test_generation_guard;
    use crate::{
        AnyElement, Drawable, Element, EventElement, LayoutElement, Rebuildable,
        ReconciliationMatchKind, VisitorElement, element_tree_generation,
        plan_element_reconciliation, scoped_rebuild_cursor, scoped_rebuilds_since,
        unscoped_element_tree_generation,
    };

    #[test]
    fn planning_reconciliation_does_not_mutate_either_tree() {
        let old = Branch::new([
            StateLeaf::keyed("first", 11).boxed(),
            StateLeaf::keyed("second", 22).boxed(),
        ]);
        let new = Branch::new([
            StateLeaf::keyed("second", 0).boxed(),
            StateLeaf::keyed("first", 0).boxed(),
        ]);
        let old_ids = child_ids(old.as_ref());
        let new_ids = child_ids(new.as_ref());
        let _generation_guard = test_generation_guard();
        let generation = element_tree_generation();

        let plan = plan_element_reconciliation(old.as_ref(), new.as_ref());

        assert_eq!(child_ids(old.as_ref()), old_ids);
        assert_eq!(child_ids(new.as_ref()), new_ids);
        assert_eq!(leaf_states(new.as_ref()), [0, 0]);
        assert_eq!(element_tree_generation(), generation);
        assert_eq!(
            plan.matches()
                .iter()
                .map(|element_match| element_match.kind())
                .collect::<Vec<_>>(),
            [
                ReconciliationMatchKind::Root,
                ReconciliationMatchKind::Keyed,
                ReconciliationMatchKind::Keyed,
            ]
        );
        plan.validate().unwrap();
    }

    /// A wrapper that keeps its only child as its first field, as `Expanded`
    /// does. The child then sits at the same address as the wrapper itself,
    /// which a pair of addresses cannot tell apart from the wrapper's own pair.
    #[repr(C)]
    struct FirstFieldWrapper {
        child: AnyElement,
    }

    impl VisitorElement for FirstFieldWrapper {
        fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
            visitor(&self.child);
        }

        fn debug_name(&self) -> &'static str {
            "FirstFieldWrapper"
        }
    }

    impl EventElement for FirstFieldWrapper {}
    impl LayoutElement for FirstFieldWrapper {}
    impl Drawable for FirstFieldWrapper {
        fn update(&self, _ctx: &BuildContext) {}
    }
    impl Rebuildable for FirstFieldWrapper {}

    #[test]
    fn a_child_stored_at_its_wrappers_address_is_still_matched() {
        let old = FirstFieldWrapper {
            child: StateLeaf::unkeyed(1).boxed(),
        }
        .boxed();
        let new = FirstFieldWrapper {
            child: StateLeaf::unkeyed(0).boxed(),
        }
        .boxed();
        let old_child = child_ids(old.as_ref());

        let plan = plan_element_reconciliation(old.as_ref(), new.as_ref());
        assert_eq!(
            plan.matches().len(),
            2,
            "the wrapper and the child inside it must both be matched"
        );
        plan.apply_identities();

        assert_eq!(
            child_ids(new.as_ref()),
            old_child,
            "the child keeps its identity"
        );
    }

    #[test]
    fn a_reconciliation_commit_reports_the_replaced_subtree_and_nothing_wider() {
        let old = Branch::new([StateLeaf::unkeyed(1).boxed()]);
        let new = Branch::new([StateLeaf::unkeyed(0).boxed()]);
        let root = old.id();
        let cursor = scoped_rebuild_cursor();
        let unscoped = unscoped_element_tree_generation();
        let generation = element_tree_generation();

        plan_element_reconciliation(old.as_ref(), new.as_ref())
            .commit(&context())
            .unwrap();

        assert!(
            element_tree_generation() > generation,
            "other consumers still see the advance"
        );
        assert_eq!(
            unscoped_element_tree_generation(),
            unscoped,
            "a replaced subtree is not an unscoped change"
        );
        assert_eq!(
            scoped_rebuilds_since(cursor),
            Some(vec![root]),
            "the new root carries the old identity"
        );
    }

    #[test]
    fn reconciliation_commit_applies_keyed_and_positional_matches_atomically() {
        let old = Branch::new([
            StateLeaf::keyed("first", 11).boxed(),
            StateLeaf::keyed("second", 22).boxed(),
            StateLeaf::unkeyed(33).boxed(),
        ]);
        let new = Branch::new([
            StateLeaf::keyed("second", 0).boxed(),
            StateLeaf::keyed("first", 0).boxed(),
            StateLeaf::unkeyed(0).boxed(),
        ]);
        let old_ids = child_ids(old.as_ref());
        let generation = element_tree_generation();
        let plan = plan_element_reconciliation(old.as_ref(), new.as_ref());

        assert_eq!(
            plan.matches()
                .iter()
                .map(|element_match| element_match.kind())
                .collect::<Vec<_>>(),
            [
                ReconciliationMatchKind::Root,
                ReconciliationMatchKind::Keyed,
                ReconciliationMatchKind::Keyed,
                ReconciliationMatchKind::Positional,
            ]
        );

        plan.commit(&context()).unwrap();

        assert_eq!(
            child_ids(new.as_ref()),
            [old_ids[1], old_ids[0], old_ids[2]]
        );
        assert_eq!(leaf_states(new.as_ref()), [11, 22, 33]);
        assert!(element_tree_generation() > generation);
    }

    struct StateLeaf {
        key: Option<crate::Key>,
        state: Cell<u32>,
    }

    impl StateLeaf {
        fn keyed(key: &'static str, state: u32) -> Self {
            Self {
                key: Some(crate::Key::Static(key)),
                state: Cell::new(state),
            }
        }

        fn unkeyed(state: u32) -> Self {
            Self {
                key: None,
                state: Cell::new(state),
            }
        }
    }

    impl VisitorElement for StateLeaf {
        fn debug_name(&self) -> &'static str {
            "StateLeaf"
        }

        fn reconciliation_key(&self) -> Option<&crate::Key> {
            self.key.as_ref()
        }
    }

    impl EventElement for StateLeaf {}
    impl LayoutElement for StateLeaf {}
    impl Drawable for StateLeaf {
        fn update(&self, _ctx: &BuildContext) {}
    }
    impl Rebuildable for StateLeaf {
        fn option_any(&self) -> Option<&dyn Any> {
            Some(self)
        }

        fn adopt_runtime_state_from(&self, old: &dyn Element) {
            let old = old
                .option_any()
                .and_then(|value| value.downcast_ref::<Self>())
                .unwrap();
            self.state.set(old.state.get());
        }
    }

    struct Branch(Vec<AnyElement>);

    impl Branch {
        fn new<const N: usize>(children: [AnyElement; N]) -> AnyElement {
            Self(children.into()).boxed()
        }
    }

    impl VisitorElement for Branch {
        fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
            for child in &self.0 {
                visitor(child.as_ref());
            }
        }

        fn debug_name(&self) -> &'static str {
            "Branch"
        }
    }

    impl EventElement for Branch {}
    impl LayoutElement for Branch {}
    impl Drawable for Branch {
        fn update(&self, _ctx: &BuildContext) {}
    }
    impl Rebuildable for Branch {}

    fn child_ids(element: &dyn Element) -> Vec<crate::ElementId> {
        let mut ids = Vec::new();
        element.visit_children(&mut |child| ids.push(child.id()));
        ids
    }

    fn leaf_states(element: &dyn Element) -> Vec<u32> {
        let mut states = Vec::new();
        element.visit_children(&mut |child| {
            let leaf = child
                .option_any()
                .and_then(|value| value.downcast_ref::<StateLeaf>())
                .unwrap();
            states.push(leaf.state.get());
        });
        states
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn dummy_async_handle() -> tokio::runtime::Handle {
        use std::sync::OnceLock;

        static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
        let runtime = RUNTIME.get_or_init(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
        });
        let _guard = runtime.enter();
        tokio::runtime::Handle::current()
    }

    fn context() -> BuildContext<'static> {
        let canvas = {
            let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
            aimer_canvas::FrameCanvas::new(inner)
        };
        BuildContext::new(
            canvas,
            Default::default(),
            1.0,
            Default::default(),
            Default::default(),
            WindowHandle::headless(Default::default(), 1.0),
            #[cfg(not(target_arch = "wasm32"))]
            dummy_async_handle(),
        )
    }
}
