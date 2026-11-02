//! Putting something down.
//!
//! [`DragTarget<T>`] is an ordinary event element that happens to handle
//! [`ElementEvent::DragOver`] and [`ElementEvent::DragDrop`]. It performs no hit
//! testing of its own: the dispatcher's routed pass already found the topmost
//! element under the pointer, honouring clipping, scrolling and z-order, and
//! the target simply consumes what reaches it.
//!
//! A target is bound to one payload type. A drag carrying anything else is not
//! rejected — it is *invisible*: the target ignores the event so it falls
//! through to whatever is underneath, which may well understand it.

use std::cell::{Cell, RefCell};
use std::marker::PhantomData;
use std::rc::Rc;

use aimer_attribute::position::Vec2d;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_events::element::ElementEvent;
use aimer_widget::InteractionBounds;
use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, AnyWidget, Drawable, Element, EventElement, EventResult, EventTreeRole,
    LayoutElement, PointerKey, Rebuildable, RequiredChild, State, StateUpdater, StatefulElement,
    StatefulWidget, VisitorElement, Widget,
};

use crate::DragSession;

/// Marks a [`DragTarget`] that has been given its child and is a [`Widget`].
pub struct HasChild;

/// Decides whether a target would take a particular payload.
pub(crate) type AcceptPredicate<T> = Rc<dyn Fn(&T) -> bool>;

/// Receives an accepted payload.
pub(crate) type AcceptHandler<T> = Rc<dyn Fn(T)>;

/// Builds a target's content from its hover state.
pub(crate) type TargetChild = Rc<dyn Fn(DragTargetState) -> AnyWidget>;

/// The hovered target's identity together with the way to tell it it was left.
type HoveredTarget = Option<(u64, Rc<dyn Fn()>)>;

/// What a drag target knows about the drag currently over it.
///
/// Handed to the child builder on every hover flip, and only on a hover flip: a
/// pointer travelling across a target does not rebuild it, and travelling
/// across one target does not rebuild the others.
///
/// # Examples
///
/// ```
/// use aimer_dnd::DragTargetState;
///
/// let idle = DragTargetState::default();
///
/// assert!(!idle.is_hovered);
/// assert!(!idle.will_accept);
/// ```
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DragTargetState {
    /// Whether a drag of the target's payload type is over it right now.
    pub is_hovered: bool,
    /// Whether releasing here would be accepted.
    ///
    /// A target with no predicate accepts everything of its type, so this
    /// tracks [`DragTargetState::is_hovered`] unless a predicate says
    /// otherwise. A hovered target with `will_accept` false is the "no" the
    /// user should see before they let go.
    pub will_accept: bool,
}

aimer_tls::thread_local! {
    /// The next target identity.
    static NEXT_ID: Cell<u64> = const { Cell::new(1) };
    /// The target the drag is currently over, and how to tell it that it is
    /// not any more.
    ///
    /// Only the topmost target receives a [`ElementEvent::DragOver`], so a
    /// target that has been left hears nothing at all. Rather than broadcast a
    /// leave event to a whole tree of elements that are not targets, the one
    /// hovered target is remembered here and told directly — `O(1)` per move
    /// instead of `O(elements)`.
    static HOVERED: RefCell<HoveredTarget> = const { RefCell::new(None) };
}

/// Hands out one identity per target, so the hovered one can be recognized
/// without comparing element pointers.
pub(crate) fn next_target_id() -> u64 {
    NEXT_ID.with(|next| {
        let id = next.get();
        next.set(id.wrapping_add(1).max(1));
        id
    })
}

/// Records that `id` is now hovered, telling whoever was before it that it is
/// not.
pub(crate) fn enter_hover(id: u64, leave: Rc<dyn Fn()>) {
    let left = HOVERED.with_borrow_mut(|hovered| match hovered.as_ref() {
        Some((current, _)) if *current == id => None,
        _ => hovered.replace((id, leave)),
    });
    if let Some((_, leave)) = left {
        leave();
    }
}

/// Tells the hovered target, if any, that the drag has gone.
pub(crate) fn clear_hover() {
    let left = HOVERED.with_borrow_mut(Option::take);
    if let Some((_, leave)) = left {
        leave();
    }
}

/// A region that accepts dropped values of one type.
///
/// `new()` takes nothing and [`DragTarget::child`] comes last: the child
/// builder is what turns this into a [`Widget`].
///
/// # Examples
///
/// ```no_run
/// use aimer_container::{Container, ZeroSizedBox};
/// use aimer_dnd::{DragTarget, DragTargetState};
///
/// #[derive(Clone)]
/// struct CardId(u32);
///
/// let column = DragTarget::<CardId>::new()
///     .will_accept(|id: &CardId| id.0 != 7)
///     .on_accept(|_id: CardId| { /* move the card */ })
///     .child(|state: DragTargetState| {
///         let _highlight = state.is_hovered && state.will_accept;
///         Container::new().child(ZeroSizedBox)
///     });
/// ```
#[derive(aimer_widget::PortableWidget)]
#[portable_widget(id = "aimer_dnd::DragTarget", schema_only)]
pub struct DragTarget<T, C = RequiredChild> {
    #[portable_skip]
    will_accept: Option<AcceptPredicate<T>>,
    #[portable_skip]
    on_accept: Option<AcceptHandler<T>>,
    // The child is rebuilt from live hover state, so it has no single static
    // portable child node. Keep the callback on the native path.
    #[portable_skip]
    child: Option<TargetChild>,
    #[portable_skip]
    _child: PhantomData<C>,
    #[portable_skip]
    _payload: PhantomData<T>,
}

impl<T> Default for DragTarget<T> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<T> DragTarget<T> {
    /// Creates an incomplete target that accepts every `T`.
    #[inline]
    pub fn new() -> Self {
        Self {
            will_accept: None,
            on_accept: None,
            child: None,
            _child: PhantomData,
            _payload: PhantomData,
        }
    }
}

impl<T, C> DragTarget<T, C> {
    /// Decides, per drag, whether this target would take it.
    ///
    /// Called on hover, not on every move. A target with no predicate accepts
    /// every payload of its type.
    #[inline]
    pub fn will_accept<F: Fn(&T) -> bool + 'static>(mut self, will_accept: F) -> Self {
        self.will_accept = Some(Rc::new(will_accept));
        self
    }

    /// Receives the dropped value, exactly once per accepted drop.
    #[inline]
    pub fn on_accept<F: Fn(T) + 'static>(mut self, on_accept: F) -> Self {
        self.on_accept = Some(Rc::new(on_accept));
        self
    }

    /// Builds the target's content from its hover state, completing the
    /// builder.
    #[inline]
    pub fn child<F, W>(self, child: F) -> DragTarget<T, HasChild>
    where
        F: Fn(DragTargetState) -> W + 'static,
        W: Widget + 'static,
    {
        DragTarget {
            will_accept: self.will_accept,
            on_accept: self.on_accept,
            child: Some(Rc::new(move |state| child(state).boxed())),
            _child: PhantomData,
            _payload: PhantomData,
        }
    }
}

impl<T: 'static> Widget for DragTarget<T, HasChild> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::from_widget(self, ctx, "DragTarget", None)
    }

    fn debug_name(&self) -> &'static str {
        "DragTarget"
    }
}

/// The hover state one target keeps between frames.
pub struct DragTargetLiveState<T> {
    id: u64,
    hover: DragTargetState,
    will_accept: Option<AcceptPredicate<T>>,
    on_accept: Option<AcceptHandler<T>>,
    child: Option<TargetChild>,
    updater: StateUpdater<Self>,
}

impl<T: 'static> StatefulWidget for DragTarget<T, HasChild> {
    type State = DragTargetLiveState<T>;

    fn create_state(self) -> Self::State {
        DragTargetLiveState {
            id: next_target_id(),
            hover: DragTargetState::default(),
            will_accept: self.will_accept.clone(),
            on_accept: self.on_accept.clone(),
            child: self.child.clone(),
            updater: StateUpdater::empty(),
        }
    }
}

impl<T: 'static> State<DragTarget<T, HasChild>> for DragTargetLiveState<T> {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn adopt_config_from(&mut self, new: Self) {
        self.will_accept = new.will_accept;
        self.on_accept = new.on_accept;
        self.child = new.child;
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        let content = self
            .child
            .as_ref()
            .map(|build| build(self.hover))
            .unwrap_or_else(|| Widget::boxed(aimer_container::ZeroSizedBox));

        TargetGate {
            child: content,
            logic: Rc::new(TargetLogic {
                id: self.id,
                will_accept: self.will_accept.clone(),
                on_accept: self.on_accept.clone(),
                updater: self.updater,
            }),
        }
    }
}

/// The event-handling half of a target, kept apart from the state so a rebuild
/// does not have to recreate the element that owns the bounds.
struct TargetLogic<T> {
    id: u64,
    will_accept: Option<AcceptPredicate<T>>,
    on_accept: Option<AcceptHandler<T>>,
    updater: StateUpdater<DragTargetLiveState<T>>,
}

impl<T: 'static> TargetLogic<T> {
    /// What the state currently says about the drag over this target.
    fn hover(&self) -> DragTargetState {
        self.updater.read(|state| state.hover)
    }

    /// Rebuilds only if the flags actually changed.
    fn set_hover(&self, hover: DragTargetState) {
        if self.hover() == hover {
            return;
        }
        self.updater.set_state(move |state| state.hover = hover);
    }

    /// Whether this target would take the drag in flight, and whether the drag
    /// is even addressed to it.
    fn evaluate(&self) -> Option<bool> {
        DragSession::with_payload::<T, bool>(|payload| {
            self.will_accept
                .as_ref()
                .is_none_or(|predicate| predicate(payload))
        })
    }
}

struct TargetGate<T> {
    child: AnyWidget,
    logic: Rc<TargetLogic<T>>,
}

impl<T: 'static> Widget for TargetGate<T> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        RawDragTarget {
            child: self.child.to_element(ctx),
            logic: self.logic.clone(),
            bounds: InteractionBounds::new(),
        }
        .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "DragTargetGate"
    }
}

impl<T: 'static> aimer_widget::PortableWidget for TargetGate<T> {}

struct RawDragTarget<T> {
    child: AnyElement,
    logic: Rc<TargetLogic<T>>,
    bounds: InteractionBounds,
}

impl<T: 'static> RawDragTarget<T> {
    /// Registers this target as the hovered one and remembers how to unregister
    /// it.
    fn enter(&self, will_accept: bool) {
        let logic = self.logic.clone();
        enter_hover(
            self.logic.id,
            Rc::new(move || {
                logic.set_hover(DragTargetState::default());
            }),
        );
        self.logic.set_hover(DragTargetState {
            is_hovered: true,
            will_accept,
        });
    }
}

impl<T: 'static> VisitorElement for RawDragTarget<T> {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "DragTarget"
    }
}

impl<T: 'static> EventElement for RawDragTarget<T> {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::IndexedTarget
    }

    fn on_event(&self, event: &ElementEvent) -> EventResult {
        match event {
            ElementEvent::DragOver { .. } => {
                let Some(will_accept) = self.logic.evaluate() else {
                    // Somebody else's payload: stay out of its way entirely, so
                    // a target underneath that does understand it still hears.
                    return EventResult::ignored();
                };
                self.enter(will_accept);
                EventResult::consumed().with_redraw()
            }

            ElementEvent::DragDrop { source, id, .. } => {
                let Some(will_accept) = self.logic.evaluate() else {
                    return EventResult::ignored();
                };
                clear_hover();
                if !will_accept {
                    // Refused: the payload stays in the session, which is how
                    // the release is reported as unaccepted and the feedback
                    // knows to spring back.
                    return EventResult::consumed().with_redraw();
                }

                let pointer = PointerKey::new(*source, *id);
                if let Some(payload) = DragSession::take(pointer)
                    && let Ok(value) = payload.downcast::<T>()
                    && let Some(on_accept) = self.logic.on_accept.as_ref()
                {
                    on_accept(value);
                }
                EventResult::consumed().with_redraw()
            }

            ElementEvent::DragLeave { .. } | ElementEvent::Cancel => {
                self.logic.set_hover(DragTargetState::default());
                EventResult::ignored()
            }

            _ => EventResult::ignored(),
        }
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }
}

impl<T: 'static> LayoutElement for RawDragTarget<T> {
    #[inline]
    fn size(&self) -> Option<Size> {
        None
    }

    fn layout(&self, ctx: &BuildContext) -> ResolvedSize {
        let size = self.child.layout(ctx);
        let (abs_x, abs_y) = ctx.canvas.get_transform_translation();
        self.bounds
            .save(ctx.scale, abs_x, abs_y, size.width, size.height);
        size
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.computed_size(ctx)
    }

    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.bounds.pos_start_end()
    }
}

impl<T: 'static> Drawable for RawDragTarget<T> {
    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint_local_v2(&self, _ctx: &BuildContext) {}

    /// Hit-tested against the rectangle this element reports as its own size,
    /// which can be larger than the content box its render node is laid out as.
    #[inline]
    fn retained_v2_interaction_size(&self, ctx: &BuildContext) -> Option<ResolvedSize> {
        Some(self.child.computed_size(ctx))
    }

    #[inline]
    fn adopt_retained_v2_interaction_source(&self, source: aimer_widget::InteractionSource) {
        self.bounds.adopt(source);
    }

    #[inline]
    fn retained_v2_interaction_disagreement(
        &self,
    ) -> Option<aimer_widget::InteractionDisagreement> {
        self.bounds.disagreement("DragTarget")
    }

    fn update(&self, ctx: &BuildContext) {
        let (abs_x, abs_y) = ctx.canvas.get_transform_translation();
        let size = self.child.computed_size(ctx);
        self.bounds
            .save(ctx.scale, abs_x, abs_y, size.width, size.height);
        self.child.update(ctx);
    }

    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        let (abs_x, abs_y) = ctx.canvas.get_transform_translation();
        let size = self.child.computed_size(ctx);
        self.bounds
            .save(ctx.scale, abs_x, abs_y, size.width, size.height);
        self.child.sync_paint_geometry(ctx);
    }
}

impl<T: 'static> Rebuildable for RawDragTarget<T> {}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestElement;

    impl VisitorElement for TestElement {
        fn debug_name(&self) -> &'static str {
            "TestElement"
        }
    }

    impl EventElement for TestElement {}
    impl LayoutElement for TestElement {}
    impl Rebuildable for TestElement {}

    impl Drawable for TestElement {
        fn update(&self, _ctx: &BuildContext) {}
    }

    #[test]
    fn drag_target_opts_into_indexed_event_routing() {
        let target = RawDragTarget::<u32> {
            child: TestElement.boxed(),
            logic: Rc::new(TargetLogic {
                id: 1,
                will_accept: None,
                on_accept: None,
                updater: StateUpdater::empty(),
            }),
            bounds: InteractionBounds::new(),
        };

        assert_eq!(target.event_tree_role(), EventTreeRole::IndexedTarget);
    }

    #[test]
    fn a_new_hover_tells_the_previous_one_it_was_left() {
        clear_hover();
        let left = Rc::new(RefCell::new(Vec::new()));

        let first = left.clone();
        enter_hover(1, Rc::new(move || first.borrow_mut().push(1)));
        assert!(
            left.borrow().is_empty(),
            "the first target has not been left"
        );

        let second = left.clone();
        enter_hover(2, Rc::new(move || second.borrow_mut().push(2)));

        assert_eq!(*left.borrow(), vec![1]);
        clear_hover();
        assert_eq!(*left.borrow(), vec![1, 2]);
    }

    #[test]
    fn re_entering_the_same_target_does_not_leave_it() {
        clear_hover();
        let left = Rc::new(RefCell::new(0usize));

        for _ in 0..3 {
            let counter = left.clone();
            enter_hover(7, Rc::new(move || *counter.borrow_mut() += 1));
        }

        assert_eq!(*left.borrow(), 0);
        clear_hover();
        assert_eq!(*left.borrow(), 1);
    }

    #[test]
    fn completed_drag_target_publishes_a_derived_schema() {
        use aimer_widget::portable::__anteros::ChildCardinality;
        use aimer_widget::portable::PortableWidgetSchema;

        assert_eq!(
            <DragTarget<String, HasChild> as PortableWidgetSchema>::SCHEMA.children(),
            ChildCardinality::none()
        );
    }
}
