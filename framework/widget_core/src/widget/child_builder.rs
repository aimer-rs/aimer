//! A child subtree a parent can place into the tree more than once.

use std::any::TypeId;
use std::cell::{Cell, OnceCell};
use std::rc::Rc;

use aimer_attribute::position::Vec2d;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_events::element::ElementEvent;
use aimer_events::window::request_animation_frame;
use aimer_focus::FocusNode;

use crate::base::BuildContext;
use crate::diagnostics::ErrorWidget;
use crate::components::drawable::Drawable;
use crate::components::element::{Element, ElementId, EventDispatchContext, VisitorElement};
use crate::components::event_element::{EventElement, EventResult, EventTreeRole};
use crate::components::layout_element::LayoutElement;
use crate::components::rebuildable::Rebuildable;
use crate::key::Key;
use crate::widget::Widget;
use crate::{AnyElement, AnyWidget};

/// A child subtree its parent may place into the tree, any number of times.
///
/// # Why a parent needs one
///
/// A widget describes configuration and is *consumed* by the build that turns it
/// into an element, so a plain child value can serve exactly one build. That is
/// enough for a container, which is rebuilt by its own parent and therefore
/// receives a fresh child widget every time, but not for a widget that rebuilds
/// *itself*: a scrollable may rebuild on offset changes, a theme on every tick
/// of its transition. Each of those rebuilds needs
/// the child's element again, and a consumed widget cannot produce a second one.
///
/// Reproducing the widget is not available either — cloning it would need a
/// [`Clone`] bound the tree does not have, since an erased child is not `Clone`
/// — so the child is **retained** instead. The first build consumes the widget
/// and keeps the element it produced; every later build hands the tree a thin
/// proxy over that same element. A self-rebuilding parent therefore does not
/// rebuild its subtree at all, which is strictly cheaper than rebuilding it on
/// every update.
///
/// # Identity
///
/// Reconciliation matches an old element to a new one by [`Widget::key`], and
/// diagnostics name a node by [`Widget::debug_name`]. Both are copied out of the
/// widget before it is stored, and the proxy reports the retained element's own
/// key and name, so a child reached through a slot keeps the identity it had
/// when it was reached directly.
///
/// # Invariant
///
/// One `ChildBuilder` describes **one position** in the tree. Cloning it is a
/// reference-count bump, which is what lets a state carry it across the rebuilds
/// of its parent, but placing two clones in two different positions would put
/// the same element in the tree twice and is a misuse.
///
/// # Example
///
/// ```
/// use aimer_widget_core::{ChildBuilder, ErrorWidget};
///
/// let child = ChildBuilder::from_widget(ErrorWidget::new("missing route"));
///
/// // The builder is a widget itself, so it can be handed to any parent, and
/// // it reports the name of the widget it was made from.
/// assert_eq!(aimer_widget_core::Widget::debug_name(&child), "ErrorWidget");
/// ```
#[derive(aimer_macro::PortableWidget)]
#[portable_widget(
    id = "aimer_widget::ChildBuilder",
    manual_lowering,
    materializer = materialize_child_builder
)]
pub struct ChildBuilder {
    #[portable_skip]
    source: Source,
    #[portable_skip]
    key: Option<Key>,
    #[portable_skip]
    debug_name: &'static str,
}

/// Materializes the portable form of [`ChildBuilder`] as its child.
///
/// Guest lowering consumes the builder and emits the child directly. This
/// host-side path handles a document that contains an explicit builder node
/// without recreating a native retained-element owner or crossing a closure
/// boundary into the host.
fn materialize_child_builder(
    _document: &crate::portable::__anteros::WidgetDocumentView<'_>,
    _node: crate::portable::__anteros::WidgetNodeView<'_>,
    mut children: Vec<AnyWidget>,
) -> Result<AnyWidget, crate::portable::PortableMaterializeError> {
    if children.len() != 1 {
        return Err(crate::portable::PortableMaterializeError::InvalidChildCount {
            expected: 1,
            actual: children.len(),
        });
    }
    children
        .pop()
        .ok_or(crate::portable::PortableMaterializeError::InvalidChildCount {
            expected: 1,
            actual: 0,
        })
}

/// Where a [`ChildBuilder`] gets its subtree from.
///
/// The un-attached case is the absent handle rather than a variant beside it:
/// both attached forms are one refcounted [`Child`], so `None` lives in that
/// count's null niche and a waiting builder is exactly one word — no
/// allocation, no discriminant. A parent therefore holds one plain field, and
/// the question `None` asks is already settled by its type-state.
type Source = Option<Rc<Child>>;

/// The child a slot and all of its placements share.
struct Child {
    /// Where this child's widget comes from.
    origin: Origin,
    /// The element the first build produced, for a child that is retained.
    ///
    /// Written at most once and never replaced, which is why a [`OnceCell`] is
    /// enough: a placement can hand out a borrow of the child for as long as it
    /// holds its own reference, with no borrow flag on the hot path and no
    /// `unsafe`.
    element: OnceCell<AnyElement>,
    /// Whether the owner declared this child as a named slot, through
    /// `BuildContext::retain`.
    slot: bool,
    /// The chain of elements above this child when the rebuild pass last
    /// descended it inside a replaced subtree; see `rebuild_retained_child`.
    placed_at: Cell<u64>,
}

/// How a [`Child`] produces its widget.
enum Origin {
    /// One widget, taken by the first build; the element is then reused.
    Once(Cell<Option<AnyWidget>>),
    /// A closure asked again on every build, because that is what a closure is
    /// for.
    Every(Box<dyn Fn() -> AnyWidget>),
}

impl Child {
    /// Produces this child's element for one placement in the tree.
    #[inline]
    fn build(self: &Rc<Self>, ctx: &BuildContext) -> AnyElement {
        match &self.origin {
            Origin::Every(build) => build().into_element(ctx),
            Origin::Once(widget) => {
                match self.element.get() {
                    None => {
                        let widget = widget
                            .take()
                            .expect("a retained child holds either its widget or its element");
                        self.element
                            .set(widget.into_element(ctx))
                            .unwrap_or_else(|_| unreachable!("the cell was empty one line above"));
                    }
                    // A later placement means the parent rebuilt itself, and a
                    // parent rebuild is exactly when the values it publishes
                    // change — a theme mid-transition, a provider's new value.
                    // The child's element survives, so its state, scroll offsets
                    // and GPU resources do, but the build closures inside it are
                    // asked again so they observe the new context.
                    // Only builds that observed inherited context can differ now, so
                    // the mark is a placement's, which spares the others.
                    Some(element) => {
                        if !crate::components::element::placement_marks_suspended() {
                            crate::components::element::with_placement_mark(|| {
                                element.mark_needs_rebuild()
                            })
                        }
                    }
                }
                RetainedChildElement(Rc::clone(self)).boxed()
            }
        }
    }

    /// Returns the retained element if this child has been built and kept.
    #[inline]
    fn built(&self) -> Option<&AnyElement> {
        self.element.get()
    }
}

impl ChildBuilder {
    /// Stores a widget as a subtree that can be placed repeatedly.
    ///
    /// The widget is moved in, and moved out again by the first build. Attaching
    /// a child therefore costs one allocation and no copy of the widget's
    /// fields.
    ///
    /// # Example
    ///
    /// ```
    /// use aimer_widget_core::{ChildBuilder, ErrorWidget, Widget};
    ///
    /// let child = ChildBuilder::from_widget(ErrorWidget::new("boom"));
    ///
    /// assert_eq!(child.debug_name(), "ErrorWidget");
    /// ```
    #[inline]
    pub fn from_widget<W: Widget + 'static>(widget: W) -> Self {
        let key = widget.key();
        let debug_name = widget.debug_name();
        Self {
            source: Some(Rc::new(Child {
                origin: Origin::Once(Cell::new(Some(widget.boxed()))),
                element: OnceCell::new(),
                slot: false,
                placed_at: Cell::new(0),
            })),
            key,
            debug_name,
        }
    }

    /// Stores a widget as a slot an owner declared with `BuildContext::retain`.
    ///
    /// Behaves as [`ChildBuilder::from_widget`], except that reconciliation
    /// never treats it as the stand-in for a different retained child.
    #[inline]
    pub(crate) fn slot_from_widget<W: Widget + 'static>(widget: W) -> Self {
        let mut builder = Self::from_widget(widget);
        if let Some(child) = builder.source.as_mut().and_then(Rc::get_mut) {
            child.slot = true;
        }
        builder
    }

    /// Stores a closure that produces a fresh subtree on every call.
    ///
    /// Use this when the subtree is not a single value to keep — when it
    /// depends on data read at build time, or when the caller would otherwise
    /// have to keep a widget alive only to describe it. Unlike
    /// [`ChildBuilder::from_widget`], the subtree is rebuilt on every build,
    /// because a closure exists precisely to answer differently next time.
    ///
    /// The builder cannot know a key or a name in advance here, so it reports
    /// none and is named after itself.
    ///
    /// # Example
    ///
    /// ```
    /// use aimer_widget_core::{ChildBuilder, ErrorWidget, Widget};
    ///
    /// let child = ChildBuilder::new(|| ErrorWidget::new("rebuilt").boxed());
    ///
    /// assert_eq!(child.key(), None);
    /// ```
    #[inline]
    pub fn new(build: impl Fn() -> AnyWidget + 'static) -> Self {
        Self {
            source: Some(Rc::new(Child {
                origin: Origin::Every(Box::new(build)),
                element: OnceCell::new(),
                slot: false,
                placed_at: Cell::new(0),
            })),
            key: None,
            debug_name: "ChildBuilder",
        }
    }

    /// The placeholder a parent holds until its child is attached.
    ///
    /// A parent in this framework is completed by its child, and the builder
    /// spells that out in the type: a parent still holding
    /// [`RequiredChild`](crate::RequiredChild) is not a [`Widget`], and only
    /// attaching a child makes it one. This value fills the slot in the
    /// meantime, so a half-built parent stays a plain value instead of an
    /// `Option` that every build would have to unwrap.
    ///
    /// It allocates nothing: a builder waiting for its child holds no child
    /// yet, which matters because parents are constructed on every frame.
    ///
    /// A well-typed tree never builds it. One that somehow does gets an
    /// [`ErrorWidget`] naming the mistake rather than a panic.
    ///
    /// # Example
    ///
    /// ```
    /// use aimer_widget_core::{ChildBuilder, Widget};
    ///
    /// assert_eq!(ChildBuilder::required().debug_name(), "RequiredChild");
    /// ```
    #[inline]
    pub fn required() -> Self {
        Self {
            source: None,
            key: None,
            debug_name: "RequiredChild",
        }
    }

    /// Produces the element for this child's position.
    ///
    /// A stored widget is built on the first call and reused by every later one;
    /// a stored closure is called every time.
    pub fn build(&self, ctx: &BuildContext) -> AnyElement {
        match &self.source {
            Some(child) => child.build(ctx),
            None => ErrorWidget::new("a widget was built before its required child was attached")
                .to_element(ctx),
        }
    }

    /// Consumes the stored child and lowers it into the portable Widget IR.
    ///
    /// A portable build is a one-shot description pass, so it may consume the
    /// child widget instead of retaining a native element. Reusing a builder
    /// that already has another owner, or one that has already produced a
    /// native retained element, is rejected rather than crossing native state
    /// into the guest document.
    #[doc(hidden)]
    #[cfg(feature = "portable-guest")]
    pub fn into_portable_node(
        self,
        ctx: &mut crate::portable::PortableBuildContext,
        source: crate::portable::SourceFingerprint,
    ) -> Result<crate::portable::PortableNodeId, crate::portable::PortableBuildError> {
        let Some(child) = self.source else {
            return Err(ctx.unsupported_widget(self.debug_name, source));
        };
        let child = Rc::try_unwrap(child).map_err(|_| {
            ctx.unsupported_widget("ChildBuilder", source)
        })?;
        let widget = match child.origin {
            Origin::Every(build) => build(),
            Origin::Once(widget) => widget.take().ok_or_else(|| {
                ctx.unsupported_widget("ChildBuilder", source)
            })?,
        };
        widget.into_portable_node(ctx, source)
    }
}

/// Cloning shares the child instead of duplicating it, which is what lets a
/// state keep it across the rebuilds of its parent.
impl Clone for ChildBuilder {
    #[inline]
    fn clone(&self) -> Self {
        Self {
            source: self.source.clone(),
            key: self.key.clone(),
            debug_name: self.debug_name,
        }
    }
}

impl Widget for ChildBuilder {
    #[inline]
    fn key(&self) -> Option<Key> {
        self.key.clone()
    }

    #[inline]
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        self.build(ctx)
    }

    #[inline]
    fn debug_name(&self) -> &'static str {
        self.debug_name
    }
}

impl crate::widget::PortableWidget for ChildBuilder {
    #[cfg(feature = "portable-guest")]
    fn to_portable_node(
        self,
        ctx: &mut crate::portable::PortableBuildContext,
        source: crate::portable::SourceFingerprint,
    ) -> Result<crate::portable::PortableNodeId, crate::portable::PortableBuildError> {
        self.into_portable_node(ctx, source)
    }
}

/// The element a parent places into the tree for a retained child.
///
/// It owns no subtree of its own: every question is forwarded to the one
/// retained element, so layout, painting, event dispatch, reconciliation
/// identity and diagnostics are the child's own and not the proxy's.
struct RetainedChildElement(Rc<Child>);

impl RetainedChildElement {
    /// Returns the retained child.
    ///
    /// A placement only exists after the child was built, so the child is
    /// always there; the fallback keeps a misuse from panicking during a paint.
    #[inline]
    fn child(&self) -> Option<&dyn Element> {
        self.0.built().map(|element| element.as_ref())
    }
}

impl VisitorElement for RetainedChildElement {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        if let Some(child) = self.child() {
            visitor(child);
        }
    }

    fn debug_name(&self) -> &'static str {
        self.child()
            .map(VisitorElement::debug_name)
            .unwrap_or("RetainedChild")
    }

    fn element_type_id(&self) -> TypeId {
        self.child()
            .map(VisitorElement::element_type_id)
            .unwrap_or_else(TypeId::of::<Self>)
    }

    fn reconciliation_key(&self) -> Option<&Key> {
        self.child().and_then(VisitorElement::reconciliation_key)
    }

    fn retained_identity(&self) -> Option<usize> {
        Some(Rc::as_ptr(&self.0) as usize)
    }

    fn is_retained_slot(&self) -> bool {
        self.0.slot
    }

    fn element_id(&self) -> Option<ElementId> {
        self.child().and_then(VisitorElement::element_id)
    }

    fn set_element_id(&self, id: ElementId) {
        if let Some(child) = self.child() {
            child.set_element_id(id);
        }
    }
}

impl LayoutElement for RetainedChildElement {
    #[inline]
    fn is_layout_stable(&self) -> bool {
        self.child()
            .map(LayoutElement::is_layout_stable)
            .unwrap_or(false)
    }

    fn pos(&self) -> Option<Vec2d> {
        self.child().and_then(LayoutElement::pos)
    }

    fn size(&self) -> Option<Size> {
        self.child().and_then(LayoutElement::size)
    }

    fn layout(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child()
            .map(|child| child.layout(ctx))
            .unwrap_or_default()
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child()
            .map(|child| child.computed_size(ctx))
            .unwrap_or_default()
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child()
            .map(|child| child.content_size(ctx))
            .unwrap_or_default()
    }

    fn layer(&self) -> u32 {
        self.child().map(LayoutElement::layer).unwrap_or_default()
    }

    fn flex(&self) -> Option<f32> {
        self.child().and_then(LayoutElement::flex)
    }

    fn get_size_from_child(&self) -> Option<Size> {
        self.child().and_then(LayoutElement::get_size_from_child)
    }

    fn invalidate_layout(&self) {
        if let Some(child) = self.child() {
            child.invalidate_layout();
        }
    }

    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.child().and_then(LayoutElement::pos_start_end)
    }

    fn event_tree_bounds(&self) -> Option<(Vec2d, Vec2d)> {
        self.child().and_then(LayoutElement::event_tree_bounds)
    }
}

impl EventElement for RetainedChildElement {
    fn event_tree_role(&self) -> EventTreeRole {
        self.child()
            .map(|_| EventTreeRole::IndexedHitTestBoundary)
            .unwrap_or(EventTreeRole::Transparent)
    }

    fn focus_node(&self) -> Option<&FocusNode> {
        self.child().and_then(EventElement::focus_node)
    }

    fn autofocus(&self) -> bool {
        self.child().map(EventElement::autofocus).unwrap_or_default()
    }

    fn traps_focus(&self) -> bool {
        self.child()
            .map(EventElement::traps_focus)
            .unwrap_or_default()
    }

    fn on_event(&self, event: &ElementEvent) -> EventResult {
        self.child()
            .map(|child| child.on_event(event))
            .unwrap_or_default()
    }

    fn on_event_with_context(
        &self,
        event: &ElementEvent,
        context: &mut EventDispatchContext<'_, '_>,
    ) -> EventResult {
        let Some(child) = self.child() else {
            return EventResult::ignored();
        };
        let pos = context.position();
        let result = context.dispatch_child(child, pos, event);
        if !result.needs_redraw() {
            return result;
        }
        // A redraw that came back through the child belongs to the element
        // that asked for it, and that element already marked its own retained
        // paint stale. This placement shares the child's identity, so passing
        // the flag on would also mark the whole child stale: a window-sized
        // root would re-record and damage its full bounds on every scroll.
        // The frame is still wanted, so schedule it here.
        request_animation_frame();
        result.without_redraw()
    }

    /// Offers the retained child's *own* children, not the child itself.
    ///
    /// This placement stands in for the child in every way an event cares
    /// about: it reports the child's identity, its bounds and its focus node,
    /// and [`Self::on_event`] hands the event straight to it. Yielding the
    /// child here as well would put it in the walk twice — once on its own
    /// account and once through this placement — so an event it did not consume
    /// would be delivered to it two times. Skipping a level keeps the subtree
    /// below reachable while each element is asked exactly once.
    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        if let Some(child) = self.child() {
            child.event_children(visitor);
        }
    }

    /// Reconciliation and focus traversal need the retained element itself as
    /// the placement's structural child. Descendants are visited after that
    /// element by the normal walk, which keeps positional state handoff aligned
    /// when the retained element owns a child list.
    fn structural_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        if let Some(child) = self.child() {
            visitor(child);
        }
    }

    /// Offers the retained child's own children, for the reason given on
    /// [`Self::event_children`]: this placement is hit-tested with the child's
    /// bounds and answers with the child's handler, so the child must not also
    /// be visited in its own right.
    fn hit_test_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        if let Some(child) = self.child() {
            child.hit_test_children(visitor);
        }
    }

    fn hit_test_children_reversed<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        if let Some(child) = self.child() {
            child.hit_test_children_reversed(visitor);
        }
    }

    fn hit_test_children_at<'a>(
        &'a self,
        _pos: Vec2d,
        _visitor: &mut dyn FnMut(&'a dyn Element),
    ) {
    }

    fn hit_test_children_at_reversed<'a>(
        &'a self,
        pos: Vec2d,
        visitor: &mut dyn FnMut(&'a dyn Element),
    ) {
        if let Some(child) = self.child() {
            child.hit_test_children_at_reversed(pos, visitor);
        }
    }

    fn has_overlapping_hit_targets(&self) -> bool {
        self.child()
            .is_some_and(EventElement::has_overlapping_hit_targets)
    }

    fn occludes_pointer_at(&self, pos: Vec2d) -> bool {
        self.child()
            .is_some_and(|child| child.occludes_pointer_at(pos))
    }

    fn occlusion_children_at<'a>(&'a self, pos: Vec2d, visitor: &mut dyn FnMut(&'a dyn Element)) {
        if let Some(child) = self.child() {
            child.occlusion_children_at(pos, visitor);
        }
    }
}

impl Drawable for RetainedChildElement {
    #[inline]
    fn prepare_layout(&self, ctx: &BuildContext) -> bool {
        if let Some(child) = self.child() {
            child.prepare_layout(ctx)
        } else {
            false
        }
    }

    fn update(&self, ctx: &BuildContext) {
        if let Some(child) = self.child() {
            child.update(ctx);
        }
    }

    #[inline]
    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        // The slot has no paint of its own. Keeping it local-v2 lets the
        // retained child keep its independent render node beneath the slot.
        true
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        aimer_canvas::Canvas::of(ctx).finish();
    }

    #[inline]
    fn retained_clip(&self, ctx: &BuildContext) -> Option<aimer_cupid::draw_cmd_v2::Rect> {
        self.child().and_then(|child| child.retained_clip(ctx))
    }

    #[inline]
    fn retained_v2_bounds(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        self.child().map(|child| {
            child
                .retained_v2_bounds(ctx)
                .unwrap_or_else(|| child.content_size(ctx))
        })
    }

    #[inline]
    fn retained_v2_paint_outsets(&self, ctx: &BuildContext) -> Option<[f32; 4]> {
        self.child()
            .and_then(|child| child.retained_v2_paint_outsets(ctx))
    }

    #[inline]
    fn retained_v2_child_context<'a>(
        &self,
        ctx: &BuildContext<'a>,
        child: &dyn Element,
    ) -> Option<BuildContext<'a>> {
        self.child()
            .and_then(|retained| retained.retained_v2_child_context(ctx, child))
    }

    #[inline]
    fn retained_v2_child_context_at<'a>(
        &self,
        ctx: &BuildContext<'a>,
        child: &dyn Element,
        child_index: usize,
    ) -> Option<BuildContext<'a>> {
        self.child().and_then(|retained| {
            retained.retained_v2_child_context_at(ctx, child, child_index)
        })
    }

    #[inline]
    fn retained_v2_child_geometry(
        &self,
        ctx: &BuildContext,
        child: &dyn Element,
    ) -> Option<(
        aimer_cupid::draw_cmd_v2::Rect,
        Option<aimer_cupid::draw_cmd_v2::Rect>,
    )> {
        self.child()
            .and_then(|retained| retained.retained_v2_child_geometry(ctx, child))
    }

    #[inline]
    fn retained_v2_child_geometry_at(
        &self,
        ctx: &BuildContext,
        child: &dyn Element,
        child_index: usize,
    ) -> Option<(
        aimer_cupid::draw_cmd_v2::Rect,
        Option<aimer_cupid::draw_cmd_v2::Rect>,
    )> {
        self.child().and_then(|retained| {
            retained.retained_v2_child_geometry_at(ctx, child, child_index)
        })
    }

    #[inline]
    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        if let Some(child) = self.child() {
            child.sync_paint_geometry(ctx);
        }
    }

}

impl Rebuildable for RetainedChildElement {
    /// A placement is the child it stands for: the mark goes to the child's own
    /// element, which is judged there.
    fn forwards_marks_only(&self) -> bool {
        true
    }

    fn rebuild_if_dirty(&self, ctx: &BuildContext) {
        if let Some(child) = self.child() {
            crate::components::element::rebuild_retained_child(child, &self.0.placed_at, ctx);
        }
    }

    fn mark_needs_rebuild(&self) {
        if let Some(child) = self.child() {
            child.mark_needs_rebuild();
        }
    }

    /// Adopts nothing.
    ///
    /// The element this one replaces is a placement of the *same* retained
    /// child, so there is no state to move: it never left.
    fn adopt_runtime_state_from(&self, _old: &dyn Element) {}
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use aimer_anteros::{ModelLimits, Version, WidgetDocument, WidgetNode, WidgetSchemaId};
    use aimer_attribute::size::ResolvedSize;

    use super::*;

    mod scroll_position;
    mod forwarded_redraw;
    mod context_reads;
    mod retain_slots;
    use crate::base::WindowHandle;
    use crate::components::element::broadcast_event;
    use crate::portable::{
        PortableNativeWidget, PortableWidgetSchema, linked_portable_native_widget_registrations,
    };

    struct Probe {
        builds: Rc<Cell<usize>>,
    }

    /// An element that counts the events offered to it and consumes none of
    /// them, which is the only case where being asked twice is visible.
    struct Counter {
        events: Rc<Cell<usize>>,
    }

    impl VisitorElement for Counter {
        fn debug_name(&self) -> &'static str {
            "Counter"
        }
    }

    impl Rebuildable for Counter {}

    impl Drawable for Counter {
        fn update(&self, _ctx: &BuildContext) {}
    }

    impl LayoutElement for Counter {}

    impl EventElement for Counter {
        fn on_event(&self, _event: &ElementEvent) -> EventResult {
            self.events.set(self.events.get() + 1);
            EventResult::ignored()
        }
    }

    struct Counting {
        events: Rc<Cell<usize>>,
    }

    impl Widget for Counting {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            Counter {
                events: self.events,
            }
            .boxed()
        }

        fn debug_name(&self) -> &'static str {
            "Counting"
        }
    }

    impl crate::widget::PortableWidget for Counting {}

    impl Widget for Probe {
        fn to_element(self, ctx: &BuildContext) -> AnyElement {
            self.builds.set(self.builds.get() + 1);
            ErrorWidget::new("probe").to_element(ctx)
        }

        fn key(&self) -> Option<Key> {
            Some(Key::Static("probe"))
        }

        fn debug_name(&self) -> &'static str {
            "Probe"
        }
    }

    impl crate::widget::PortableWidget for Probe {}

    fn context() -> BuildContext<'static> {
        let canvas = {
            let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
            aimer_canvas::FrameCanvas::new(inner)
        };
        BuildContext::new(
            canvas,
            ResolvedSize::default(),
            1.0,
            Default::default(),
            Default::default(),
            WindowHandle::headless(Default::default(), 1.0),
            tokio::runtime::Handle::current(),
        )
    }

    #[tokio::test]
    async fn a_stored_widget_is_built_once_however_often_it_is_placed() {
        let builds = Rc::new(Cell::new(0));
        let child = ChildBuilder::from_widget(Probe {
            builds: Rc::clone(&builds),
        });
        let ctx = context();

        for _ in 0..8 {
            drop(child.build(&ctx));
        }

        assert_eq!(
            builds.get(),
            1,
            "a parent rebuilding itself must not rebuild its child subtree"
        );
    }

    /// A placement is the child, not a second element above it.
    ///
    /// It reports the child's identity and bounds and forwards to the child's
    /// handler, so a tree walk that also descended into the child would offer
    /// the same event to it twice — one tap arriving as two presses in every
    /// widget built on a retained child.
    #[tokio::test]
    async fn a_walk_offers_the_retained_child_one_event_once() {
        let events = Rc::new(Cell::new(0));
        let child = ChildBuilder::from_widget(Counting {
            events: Rc::clone(&events),
        });
        let ctx = context();
        let placement = child.build(&ctx);

        let _ = broadcast_event(placement.as_ref(), &ElementEvent::FocusGained);

        assert_eq!(events.get(), 1, "a placement must not double the child");
    }

    #[tokio::test]
    async fn every_placement_reaches_the_same_element() {
        let child = ChildBuilder::from_widget(Probe {
            builds: Rc::new(Cell::new(0)),
        });
        let ctx = context();

        let first = child.build(&ctx);
        let second = child.build(&ctx);

        let mut addresses = Vec::new();
        for placement in [&first, &second] {
            placement.visit_children(&mut |retained| {
                addresses.push(retained as *const dyn Element as *const ());
            });
        }

        assert_eq!(addresses.len(), 2);
        assert_eq!(
            addresses[0], addresses[1],
            "every placement must reach the element the first build produced"
        );
    }

    /// Reconciliation must pair a retained placement with the element it
    /// represents before walking that element's descendants.
    #[tokio::test]
    async fn structural_walk_exposes_the_retained_element_before_its_children() {
        struct Parent {
            child: AnyElement,
        }

        impl VisitorElement for Parent {
            fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
                visitor(&self.child);
            }

            fn debug_name(&self) -> &'static str {
                "StructuralParent"
            }
        }

        impl EventElement for Parent {}
        impl LayoutElement for Parent {}
        impl Rebuildable for Parent {}
        impl Drawable for Parent {
            fn update(&self, _ctx: &BuildContext) {}
        }

        struct ParentWidget;

        impl Widget for ParentWidget {
            fn to_element(self, _ctx: &BuildContext) -> AnyElement {
                Parent {
                    child: Counter {
                        events: Rc::new(Cell::new(0)),
                    }
                    .boxed(),
                }
                .boxed()
            }

            fn debug_name(&self) -> &'static str {
                "ParentWidget"
            }
        }

        impl crate::widget::PortableWidget for ParentWidget {}

        let ctx = context();
        let placement = ChildBuilder::from_widget(ParentWidget).build(&ctx);
        let mut names = Vec::new();
        placement.structural_children(&mut |child| names.push(child.debug_name()));

        assert_eq!(
            names,
            vec!["StructuralParent"],
            "a retained placement must pair with its represented element before descending"
        );
    }

    #[tokio::test]
    async fn a_placement_keeps_the_identity_of_the_retained_child() {
        let child = ChildBuilder::from_widget(Probe {
            builds: Rc::new(Cell::new(0)),
        });
        let ctx = context();

        let placement = child.build(&ctx);

        assert_eq!(
            placement.debug_name(),
            "ErrorWidget",
            "diagnostics must name the child, not its holder"
        );
        assert_eq!(
            child.key(),
            Some(Key::Static("probe")),
            "reconciliation matches on the key of the widget, not of its holder"
        );
        assert_eq!(child.debug_name(), "Probe");
    }

    #[tokio::test]
    async fn a_builder_is_a_widget_a_parent_can_take_as_its_child() {
        let builds = Rc::new(Cell::new(0));
        let child = ChildBuilder::from_widget(Probe {
            builds: Rc::clone(&builds),
        });
        let ctx = context();

        drop(Widget::to_element(child, &ctx));

        assert_eq!(builds.get(), 1);
    }

    /// A widget whose element has nothing below it and no rebuild source.
    struct LeafWidget;

    impl Widget for LeafWidget {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            Counter { events: Rc::new(Cell::new(0)) }.boxed()
        }
    }

    impl crate::portable::PortableWidget for LeafWidget {}

    #[tokio::test]
    async fn placing_a_leaf_child_again_leaves_the_dirty_path_index_alone() {
        // An animation rebuilds its subtree on every frame, and every rebuild
        // places each retained child again. Marking a leaf — nothing under it,
        // no source of rebuilds — for rebuild cannot change anything, but it used
        // to throw the index away, so the pass that followed walked the whole page.
        let child = ChildBuilder::from_widget(LeafWidget);
        let ctx = context();
        drop(child.build(&ctx));
        crate::components::element::mark_dirty_paths_ready();

        drop(child.build(&ctx));

        assert!(
            crate::components::element::dirty_paths_are_ready(),
            "a second placement of a leaf discarded the index of dirty paths"
        );
    }

    /// An element that holds one child and does nothing else.
    ///
    /// `forwards` is whether it declares that marking and rebuilding it only
    /// reach its child, so that it cannot hide a producer of rebuilds itself.
    struct Wrapper {
        child: AnyElement,
        forwards: bool,
    }

    impl VisitorElement for Wrapper {
        fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
            visitor(&self.child);
        }

        fn debug_name(&self) -> &'static str {
            "Wrapper"
        }
    }

    impl EventElement for Wrapper {}
    impl LayoutElement for Wrapper {}
    impl Drawable for Wrapper {
        fn update(&self, _ctx: &BuildContext) {}
    }

    impl Rebuildable for Wrapper {
        fn forwards_marks_only(&self) -> bool {
            self.forwards
        }
    }

    /// A widget whose element is a wrapper around `inner`, or around a leaf.
    struct WrapperWidget {
        forwards: bool,
        inner: Option<Box<WrapperWidget>>,
    }

    impl Widget for WrapperWidget {
        fn to_element(self, ctx: &BuildContext) -> AnyElement {
            let child = match self.inner {
                Some(inner) => inner.to_element(ctx),
                None => Counter { events: Rc::new(Cell::new(0)) }.boxed(),
            };
            Wrapper { child, forwards: self.forwards }.boxed()
        }
    }

    impl crate::portable::PortableWidget for WrapperWidget {}

    /// Places `widget` twice, as a parent that rebuilds itself does, and says
    /// whether the second placement left the dirty-path index usable.
    async fn index_survives_a_second_placement(widget: WrapperWidget) -> bool {
        let child = ChildBuilder::from_widget(widget);
        let ctx = context();
        drop(child.build(&ctx));
        crate::components::element::mark_dirty_paths_ready();

        drop(child.build(&ctx));

        crate::components::element::dirty_paths_are_ready()
    }

    #[tokio::test]
    async fn placing_a_forwarding_wrapper_again_leaves_the_dirty_path_index_alone() {
        // A container around a label is what a button holds, and a retained
        // child of that shape is placed again by every frame of an animation.
        // The wrapper only passes the mark on, and what it reaches is a leaf, so
        // there is no producer the index could have lost.
        let nested = WrapperWidget {
            forwards: true,
            inner: Some(Box::new(WrapperWidget { forwards: true, inner: None })),
        };

        assert!(
            index_survives_a_second_placement(nested).await,
            "a wrapper that only forwards discarded the index of dirty paths"
        );
    }

    #[tokio::test]
    async fn placing_a_wrapper_that_may_hide_a_producer_again_discards_the_index() {
        // An element that does not say it only forwards could rebuild by means
        // the index does not know, so a mark on it has to stay conservative.
        let widget = WrapperWidget { forwards: false, inner: None };

        assert!(
            !index_survives_a_second_placement(widget).await,
            "an opaque wrapper must keep discarding the index"
        );
    }

    #[tokio::test]
    async fn a_forwarding_wrapper_over_an_opaque_one_still_discards_the_index() {
        // The mark passes through the forwarder and reaches an element that could
        // be a producer: the forwarder's promise covers only itself.
        let widget = WrapperWidget {
            forwards: true,
            inner: Some(Box::new(WrapperWidget { forwards: false, inner: None })),
        };

        assert!(
            !index_survives_a_second_placement(widget).await,
            "an opaque element below a forwarder was not noticed"
        );
    }

    #[tokio::test]
    async fn a_clone_places_the_same_child() {
        let builds = Rc::new(Cell::new(0));
        let child = ChildBuilder::from_widget(Probe {
            builds: Rc::clone(&builds),
        });
        let carried = child.clone();
        let ctx = context();

        drop(child.build(&ctx));
        drop(carried.build(&ctx));

        assert_eq!(
            builds.get(),
            1,
            "a clone shares the child instead of building another one"
        );
    }

    #[test]
    fn waiting_for_a_child_costs_no_space_and_no_allocation() {
        assert_eq!(
            size_of::<Source>(),
            size_of::<usize>(),
            "an un-attached child has to live in the null niche of the count"
        );
        assert!(ChildBuilder::required().source.is_none());
    }

    #[tokio::test]
    async fn an_unattached_child_reports_the_mistake_instead_of_panicking() {
        let ctx = context();

        let element = ChildBuilder::required().build(&ctx);

        assert_eq!(element.debug_name(), "ErrorWidget");
    }

    #[tokio::test]
    async fn a_closure_produces_a_fresh_widget_for_every_build() {
        let builds = Rc::new(Cell::new(0));
        let child = ChildBuilder::new({
            let builds = Rc::clone(&builds);
            move || {
                Probe {
                    builds: Rc::clone(&builds),
                }
                .boxed()
            }
        });
        let ctx = context();

        drop(child.build(&ctx));
        drop(child.build(&ctx));

        assert_eq!(builds.get(), 2);
        assert_eq!(
            child.debug_name(),
            "ChildBuilder",
            "a closure has no widget to take a name from until it runs"
        );
    }

    #[test]
    fn child_builder_materializes_a_single_child_without_retaining_host_state() {
        const LIMITS: ModelLimits = ModelLimits::new(1_024, 4, 8, 8).max_widget_depth(2);
        let nodes = [WidgetNode::new(WidgetSchemaId::new(1), Version::new(1, 0))];
        let image = WidgetDocument::new(0, 0, 0, &nodes, &[], &[])
            .encode(LIMITS)
            .unwrap();
        let document = aimer_anteros::WidgetDocumentView::decode(&image, LIMITS).unwrap();
        let node = document.node(0).unwrap();
        let materialized = <ChildBuilder as PortableNativeWidget>::materialize_widget(
            &document,
            node,
            vec![ErrorWidget::new("child").boxed()],
        )
        .unwrap();

        assert_eq!(Widget::debug_name(&materialized), "ErrorWidget");
        let widget_type = <ChildBuilder as PortableWidgetSchema>::SCHEMA.widget().id();
        assert!(linked_portable_native_widget_registrations()
            .iter()
            .any(|registration| registration.widget_type() == widget_type));
        assert!(matches!(
            <ChildBuilder as PortableNativeWidget>::materialize_widget(
                &document,
                document.node(0).unwrap(),
                vec![],
            ),
            Err(crate::portable::PortableMaterializeError::InvalidChildCount {
                expected: 1,
                actual: 0,
            }),
        ));
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn child_builder_guest_lowering_consumes_only_a_unique_source() {
        use crate::portable::{
            PortableBuildContext, PortableLimits, PortableWidgetLimits, SourceFingerprint,
            StableId128,
        };

        let mut context = PortableBuildContext::new(
            1,
            1,
            PortableWidgetLimits::new(8, 8, 8, 8, 64, 2_048),
            PortableLimits::new(8, 16, 64, 128, 1_024),
        )
        .unwrap();
        let child = ChildBuilder::from_widget(ErrorWidget::new("child"));
        let root = child
            .into_portable_node(
                &mut context,
                SourceFingerprint::new(StableId128::from_bytes([2; 16])),
            )
            .unwrap();
        let graph = context.finish_graph(root).unwrap();

        assert_eq!(graph.node_count(), 1);
        assert_eq!(
            graph.node(root).unwrap().widget_type(),
            <ErrorWidget as PortableWidgetSchema>::SCHEMA.widget().id(),
        );
    }

    #[cfg(feature = "portable-guest")]
    #[test]
    fn child_builder_guest_lowering_rejects_shared_native_ownership() {
        use crate::portable::{
            PortableBuildContext, PortableLimits, PortableWidgetLimits, SourceFingerprint,
            StableId128,
        };

        let mut context = PortableBuildContext::new(
            1,
            1,
            PortableWidgetLimits::new(8, 8, 8, 8, 64, 2_048),
            PortableLimits::new(8, 16, 64, 128, 1_024),
        )
        .unwrap();
        let child = ChildBuilder::from_widget(ErrorWidget::new("child"));
        let _shared_owner = child.clone();
        let result = child.into_portable_node(
            &mut context,
            SourceFingerprint::new(StableId128::from_bytes([3; 16])),
        );

        assert!(matches!(
            result,
            Err(crate::portable::PortableBuildError::UnsupportedWidget {
                widget: "ChildBuilder",
                ..
            })
        ));
    }
}
