//! Making an arbitrary subtree a keyboard focus target.
//!
//! Aimer ships focusable controls — a text field is one — but a control is not
//! the only thing a user tabs to: a card, a list row, a canvas or a panel may
//! all want the keyboard without being a control at all. [`Focusable`] is the
//! wrapper that grants that to any subtree, and the rest of this module is the
//! element it retains.

use aimer_attribute::position::Vec2d;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_events::element::ElementEvent;
use aimer_focus::{FocusBehavior, FocusCallback, FocusGate, FocusNode};
use aimer_utils::callback::{CallbackExecutor, VoidCallback};
use std::marker::PhantomData;
use std::panic::Location;

use crate::base::BuildContext;
use crate::components::drawable::Drawable;
use crate::components::element::{Element, dispatch_focused_event};
use crate::components::event_element::{EventElement, EventResult, EventTreeRole};
use crate::components::layout_element::LayoutElement;
use crate::components::rebuildable::Rebuildable;
use crate::components::visitor_element::VisitorElement;
use crate::key::Key;
use crate::widget::child_builder::ChildBuilder;
use crate::widget::stateful::{State, StateUpdater, StatefulElement, StatefulWidget};
use crate::{AnyElement, AnyWidget, RequiredChild, Widget};

/// Makes its child a keyboard focus target.
///
/// A focusable region is one that offers a [`FocusNode`] to the framework. That
/// single offer is the whole of the contract: a press that lands on the region
/// moves focus to it, `Tab` and `Shift-Tab` stop at it in tree order, keyboard
/// events are delivered to it while it owns focus, and a press that lands on
/// nothing focusable takes focus away again. Everything else this widget does is
/// handing layout, painting and events straight through to the child.
///
/// # Why it has state
///
/// A focus target *is* its node, and widgets are rebuilt constantly — including
/// as a result of the focus change itself, when a handler calls `set_state` to
/// draw a border. A region that made a fresh node on every build would be a
/// different target each time and would lose focus the moment anything above it
/// changed. The node is therefore created once, in the state, and every rebuild
/// hands the same one back.
///
/// The child is retained for the same reason, through a [`ChildBuilder`]: a
/// rebuild reaches the child's *element* again instead of building another one,
/// so a text field inside a focusable card keeps its text when the card is
/// focused.
///
/// # Choosing a behavior
///
/// [`FocusBehavior`] decides whether the region is offered at all and whether it
/// asks for focus on arrival; see its documentation for the table. The default
/// is [`FocusBehavior::OnPress`].
///
/// # Observing focus
///
/// [`on_focus`](Self::on_focus) and [`on_focus_lost`](Self::on_focus_lost)
/// report one edge each; [`on_focus_change`](Self::on_focus_change) reports both
/// with a `bool`, which is usually what a handler that flips an appearance
/// wants. All three are notifications rather than requests: they never consume
/// the event, so a focusable region nested inside another still hears about its
/// own focus.
///
/// # Examples
///
/// A region that draws a border while it owns the keyboard, driven by the
/// application's own state:
///
/// ```ignore
/// Focusable::new()
///     .on_focus_change(move |focused| updater.set_state(move |s| s.focused = focused))
///     .child(Container::new().box_decoration(decoration(self.focused)).child(body))
/// ```
///
/// A region an application also drives by hand, by keeping the node itself:
///
/// ```ignore
/// // in the state
/// let node = FocusNode::new();
///
/// // in the build
/// Focusable::new().node(node.clone()).child(body)
///
/// // from anywhere, later
/// node.request_focus();
/// ```
#[derive(aimer_macro::PortableWidget)]
#[portable_widget(id = "aimer_widget::Focusable", schema_only)]
pub struct Focusable<W = RequiredChild> {
    #[portable_skip]
    node: Option<FocusNode>,
    #[portable_callback(async)]
    on_focus: VoidCallback,
    #[portable_callback(async)]
    on_focus_lost: VoidCallback,
    #[portable_skip]
    on_focus_change: FocusCallback,
    #[portable_skip]
    behavior: FocusBehavior,
    #[portable_skip]
    gate: FocusGate,
    /// The subtree inside the region, kept as a builder because the region is
    /// rebuilt whenever focus moves and needs the same child each time.
    #[portable_child]
    child: ChildBuilder,
    #[portable_skip]
    widget_key: Option<Key>,
    /// Records which child type completed the builder without storing it.
    ///
    /// The child itself is erased into [`ChildBuilder`], but the parameter has
    /// to survive so that a region without a child stays
    /// `Focusable<RequiredChild>` — a type that is deliberately not a
    /// [`Widget`] — and so that one state type belongs to one child type.
    #[portable_skip]
    marker: PhantomData<W>,
}

impl Focusable {
    /// Creates a focusable region with a node of its own.
    ///
    /// Finish the builder with [`Focusable::child`] or
    /// [`Focusable::box_child`].
    #[inline]
    pub fn new() -> Self {
        Self {
            node: None,
            on_focus: VoidCallback::default(),
            on_focus_lost: VoidCallback::default(),
            on_focus_change: FocusCallback::default(),
            behavior: FocusBehavior::default(),
            gate: FocusGate::default(),
            child: ChildBuilder::required(),
            widget_key: None,
            marker: PhantomData,
        }
    }
}

impl Default for Focusable {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<W> Focusable<W> {
    /// Uses `node` as this region's focus target, instead of one of its own.
    ///
    /// Supply a node kept by a `State` when the application needs to move focus
    /// imperatively — [`FocusNode::request_focus`],
    /// [`FocusNode::unfocus`] — or to ask who owns it. Cloning a node yields
    /// another handle to the same target, not a second target.
    ///
    /// Without this the region creates a node and keeps it; the region is fully
    /// usable either way.
    #[inline]
    pub fn node(mut self, node: FocusNode) -> Self {
        self.node = Some(node);
        self
    }

    /// Sets how the region takes part in focus.
    ///
    /// The default is [`FocusBehavior::OnPress`].
    #[inline]
    pub fn behavior(mut self, behavior: FocusBehavior) -> Self {
        self.behavior = behavior;
        self
    }

    /// Offers the region as a focus target only while `predicate` holds.
    ///
    /// A [behavior](Self::behavior) is chosen when the region is described; a
    /// gate is asked every time the tree gathers its targets, which is what a
    /// region whose eligibility follows live state needs — a selection owns the
    /// keyboard exactly as long as the selection exists, and nothing rebuilds
    /// the region when it ends. The two compose: an ignored region stays hidden
    /// however the gate answers.
    ///
    /// Keep the predicate cheap. It is a read of state that already exists, not
    /// a place to compute one.
    #[inline]
    pub fn focusable_when(mut self, predicate: impl Fn() -> bool + 'static) -> Self {
        self.gate = FocusGate::from(move |()| predicate());
        self
    }

    /// Reports that the region became the focus owner.
    #[inline]
    pub fn on_focus(mut self, on_focus: impl Into<VoidCallback>) -> Self {
        self.on_focus = on_focus.into();
        self
    }

    /// Reports that the region stopped being the focus owner.
    #[inline]
    pub fn on_focus_lost(mut self, on_focus_lost: impl Into<VoidCallback>) -> Self {
        self.on_focus_lost = on_focus_lost.into();
        self
    }

    /// Reports every change of focus ownership, `true` for a gain.
    ///
    /// Runs alongside [`on_focus`](Self::on_focus) and
    /// [`on_focus_lost`](Self::on_focus_lost) rather than instead of them, so a
    /// widget may take whichever shape reads better where it is used.
    #[inline]
    pub fn on_focus_change(mut self, on_focus_change: impl Fn(bool) + 'static) -> Self {
        self.on_focus_change = FocusCallback::from(on_focus_change);
        self
    }

    /// Sets the identity of this region for widget reconciliation.
    #[inline]
    #[track_caller]
    pub fn key(mut self, key: impl Into<Key>) -> Self {
        let caller = Location::caller();
        self.widget_key = Some(key.into().with_location(caller));
        self
    }

    /// Attaches the required child and completes this builder.
    #[inline]
    pub fn child<C: Widget + 'static>(self, child: C) -> Focusable<C> {
        Focusable {
            node: self.node,
            on_focus: self.on_focus,
            on_focus_lost: self.on_focus_lost,
            on_focus_change: self.on_focus_change,
            behavior: self.behavior,
            gate: self.gate,
            child: ChildBuilder::from_widget(child),
            widget_key: self.widget_key,
            marker: PhantomData,
        }
    }

    /// Attaches `child` and erases the completed region's concrete type.
    ///
    /// Exactly equivalent to `self.child(child).boxed()`.
    #[inline]
    pub fn box_child<C: Widget + 'static>(self, child: C) -> AnyWidget {
        self.child(child).boxed()
    }
}

/// The retained side of [`Focusable`]: the focus target itself, and the child
/// it wraps.
pub struct FocusableState<W: Widget + 'static> {
    node: FocusNode,
    /// Whether the node was handed in from outside.
    ///
    /// A rebuild brings a new widget, and only a node that widget was *given*
    /// may replace the retained one. Adopting the node of a region that makes
    /// its own would swap the focus target on every rebuild, which is the very
    /// thing this state exists to prevent.
    supplied: bool,
    on_focus: VoidCallback,
    on_focus_lost: VoidCallback,
    on_focus_change: FocusCallback,
    behavior: FocusBehavior,
    gate: FocusGate,
    child: ChildBuilder,
    /// Keeps one state type per child type, so a region whose child type
    /// changes is rebuilt from scratch rather than adopting the state of a
    /// different region.
    marker: PhantomData<W>,
}

impl<W: Widget + 'static> StatefulWidget for Focusable<W> {
    type State = FocusableState<W>;

    fn create_state(self) -> Self::State {
        FocusableState {
            supplied: self.node.is_some(),
            node: self.node.unwrap_or_default(),
            on_focus: self.on_focus,
            on_focus_lost: self.on_focus_lost,
            on_focus_change: self.on_focus_change,
            behavior: self.behavior,
            gate: self.gate,
            child: self.child,
            marker: PhantomData,
        }
    }
}

impl<W: Widget + 'static> Widget for Focusable<W> {
    fn key(&self) -> Option<Key> {
        self.widget_key.clone()
    }

    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let key = Widget::key(&self);
        StatefulElement::new_with_name(self, ctx, "Focusable", key)
            .0
            .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "Focusable"
    }
}

impl<W: Widget + 'static> State<Focusable<W>> for FocusableState<W> {
    fn init_state(&mut self, _updater: StateUpdater<Self>)
    where
        Self: Sized,
    {
    }

    fn adopt_config_from(&mut self, new: Self) {
        // The node is deliberately not adopted unless the new widget carried
        // one: see `supplied`.
        if new.supplied {
            self.node = new.node;
            self.supplied = true;
        }
        self.on_focus = new.on_focus;
        self.on_focus_lost = new.on_focus_lost;
        self.on_focus_change = new.on_focus_change;
        self.behavior = new.behavior;
        self.gate = new.gate;
        self.child = new.child;
    }

    fn build(&self, _: &BuildContext) -> impl Widget {
        FocusTarget {
            node: self.node.clone(),
            on_focus: self.on_focus.clone(),
            on_focus_lost: self.on_focus_lost.clone(),
            on_focus_change: self.on_focus_change.clone(),
            behavior: self.behavior,
            gate: self.gate.clone(),
            child: self.child.clone(),
        }
    }
}

/// What the state builds: the description of one focus attachment.
struct FocusTarget {
    node: FocusNode,
    on_focus: VoidCallback,
    on_focus_lost: VoidCallback,
    on_focus_change: FocusCallback,
    behavior: FocusBehavior,
    gate: FocusGate,
    child: ChildBuilder,
}

impl Widget for FocusTarget {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let child = self.child.to_element(ctx);
        if self.behavior.is_autofocus() {
            self.node.announce_autofocus();
        }
        RawFocusable {
            node: self.node,
            on_focus: self.on_focus,
            on_focus_lost: self.on_focus_lost,
            on_focus_change: self.on_focus_change,
            behavior: self.behavior,
            gate: self.gate,
            child,
        }
        .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "Focusable"
    }
}

impl crate::widget::PortableWidget for FocusTarget {}

/// The element that carries the focus node.
///
/// [`Focusable`] is the widget-level face of this element, and the one an
/// application wants: it keeps the node across rebuilds and retains the child.
/// This is the mechanism underneath, for a widget that already builds its own
/// element and owns its own node — a text field, a selection region — and would
/// otherwise implement [`EventElement::focus_node`] by hand. Using it costs one
/// element instead of three and gives such a widget exactly the same focus
/// behavior as every other focusable region.
///
/// # Examples
///
/// ```ignore
/// // Inside `Widget::to_element`, where the child element is already built:
/// RawFocusable::new(self.focus_node, child)
///     .focusable_when(move || session.is_focused())
///     .boxed()
/// ```
pub struct RawFocusable {
    node: FocusNode,
    on_focus: VoidCallback,
    on_focus_lost: VoidCallback,
    on_focus_change: FocusCallback,
    behavior: FocusBehavior,
    gate: FocusGate,
    child: AnyElement,
}

impl RawFocusable {
    /// Makes `child` a focus target held by `node`.
    #[inline]
    pub fn new(node: FocusNode, child: AnyElement) -> Self {
        Self {
            node,
            on_focus: VoidCallback::default(),
            on_focus_lost: VoidCallback::default(),
            on_focus_change: FocusCallback::default(),
            behavior: FocusBehavior::default(),
            gate: FocusGate::default(),
            child,
        }
    }

    /// Sets how the region takes part in focus.
    ///
    /// The default is [`FocusBehavior::OnPress`].
    #[inline]
    pub fn behavior(mut self, behavior: FocusBehavior) -> Self {
        self.behavior = behavior;
        if behavior.is_autofocus() {
            self.node.announce_autofocus();
        }
        self
    }

    /// Offers the region as a focus target only while `predicate` holds.
    ///
    /// See [`Focusable::focusable_when`].
    #[inline]
    pub fn focusable_when(mut self, predicate: impl Fn() -> bool + 'static) -> Self {
        self.gate = FocusGate::from(move |()| predicate());
        self
    }

    /// Reports that the region became the focus owner.
    #[inline]
    pub fn on_focus(mut self, on_focus: impl Into<VoidCallback>) -> Self {
        self.on_focus = on_focus.into();
        self
    }

    /// Reports that the region stopped being the focus owner.
    #[inline]
    pub fn on_focus_lost(mut self, on_focus_lost: impl Into<VoidCallback>) -> Self {
        self.on_focus_lost = on_focus_lost.into();
        self
    }

    /// Reports every change of focus ownership, `true` for a gain.
    #[inline]
    pub fn on_focus_change(mut self, on_focus_change: impl Fn(bool) + 'static) -> Self {
        self.on_focus_change = FocusCallback::from(on_focus_change);
        self
    }

    /// Tells whoever is listening that focus arrived or left.
    #[inline]
    fn report(&self, focused: bool) -> EventResult {
        if focused {
            self.on_focus.execute(());
        } else {
            self.on_focus_lost.execute(());
        }
        self.on_focus_change.execute(focused);
        EventResult::redraw()
    }

    /// Offers an event aimed at the focus owner to the subtree that owner is.
    ///
    /// A focusable region is a region, not a control: the node it offers stands
    /// for everything inside it, so what the framework delivers to the owner
    /// has to travel the last step inwards — otherwise a field wrapped in a
    /// focusable region would never see the text it is being typed into. The
    /// subtree is walked innermost first and stops at the first element that
    /// consumes, which is the same rule the dispatcher applies to the tree as a
    /// whole, so a field nested inside another never receives the same text
    /// twice.
    #[inline]
    fn offer_inside(&self, event: &ElementEvent) -> EventResult {
        dispatch_focused_event(self.child.as_ref(), event).without_capture_request()
    }
}

/// Returns whether `event` is one the framework delivers to the focus owner.
///
/// Keyboard text and input-method composition carry no position and are routed
/// by focus alone; a named key is offered to the owner before it is offered to
/// anything under the pointer. Everything else reaches this element by hit
/// testing and is already on its way to the child.
#[inline]
fn is_owner_directed(event: &ElementEvent) -> bool {
    event.is_focus_directed() || matches!(event, ElementEvent::KeyInput { .. })
}

/// A focus attachment is transparent to rebuilding: whatever the child needs
/// from a rebuild, it needs through this element as well.
impl Rebuildable for RawFocusable {
    /// Answers for the child, so that a parent asking whether its subtree keeps
    /// its own state across a rebuild is not told "no" by the wrapper.
    fn is_carry_state(&self) -> bool {
        self.child.is_carry_state()
    }

    /// Publishes whatever the child publishes — a selection scope, an inherited
    /// value — so that descendants rebuilt during state carry see the same
    /// context they see while drawing.
    fn with_rebuild_context(&self, ctx: &BuildContext, callback: &mut dyn FnMut(&BuildContext)) {
        self.child.with_rebuild_context(ctx, callback);
    }
}

impl EventElement for RawFocusable {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::IndexedTarget
    }

    /// This wrapper exposes one child through both structural views.
    #[inline]
    fn structural_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    /// Offers this region as a focus target, unless it is ignored.
    ///
    /// Answering `None` is the whole of [`FocusBehavior::Ignore`]: a target that
    /// is never offered can neither be pressed into focus nor reached by `Tab`,
    /// while the children below it are gathered exactly as before.
    #[inline]
    fn focus_node(&self) -> Option<&FocusNode> {
        let offered = self.behavior.is_focusable() && self.gate.call(()).unwrap_or(true);
        offered.then_some(&self.node)
    }

    #[inline]
    fn autofocus(&self) -> bool {
        self.behavior.is_autofocus()
    }

    /// Reports focus changes, without consuming them.
    ///
    /// A focus notification is news, not a request: consuming it would hide it
    /// from a child that also cares — a text field inside a focusable panel,
    /// say. The redraw is asked for because the region is very likely painted
    /// differently now.
    fn on_event(&self, event: &ElementEvent) -> EventResult {
        match event {
            // A notification is delivered to the owner alone, so it is passed on
            // whether or not the node still reads as focused: losing focus is
            // precisely the moment it does not.
            ElementEvent::FocusGained => self.report(true).merge(self.offer_inside(event)),
            ElementEvent::FocusLost => self.report(false).merge(self.offer_inside(event)),
            // Only the owner is asked, and only while it is the owner: a
            // broadcast reaches this element on its own way through the tree,
            // and forwarding then would offer the child the same event twice.
            _ if is_owner_directed(event) && self.node.has_focus() => self.offer_inside(event),
            _ => EventResult::ignored(),
        }
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }
}

impl Drawable for RawFocusable {
    fn update(&self, ctx: &BuildContext) {
        self.child.update(ctx);
    }

    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint_local_v2(&self, _ctx: &BuildContext) {}

    #[inline]
    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        self.child.sync_paint_geometry(ctx);
    }

}

impl LayoutElement for RawFocusable {
    fn pos(&self) -> Option<Vec2d> {
        self.child.pos()
    }

    /// Lays the child out.
    ///
    /// The default would answer with [`Self::computed_size`], which measures the
    /// child without ever placing it — a wrapper that stops there flattens
    /// whatever it wraps.
    fn layout(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.layout(ctx)
    }

    fn invalidate_layout(&self) {
        self.child.invalidate_layout();
    }

    /// Where the child ended up on screen.
    ///
    /// This is the delegation that matters most, and the easiest to forget: a
    /// press is offered to every element it *might* have landed on, and an
    /// element that reports no bounds is taken to be everywhere. A focusable
    /// element that forgets this is focused by a press anywhere in the window.
    fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
        self.child.pos_start_end()
    }

    fn event_tree_bounds(&self) -> Option<(Vec2d, Vec2d)> {
        None
    }

    fn size(&self) -> Option<Size> {
        self.child.size()
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

    fn get_size_from_child(&self) -> Option<Size> {
        self.child.get_size_from_child()
    }
}

impl VisitorElement for RawFocusable {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "Focusable"
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use aimer_attribute::position::Vec2d;
    use aimer_attribute::size::ResolvedSize;
    use aimer_events::element::{ElementEvent, KeyAction, Modifiers, NamedKey};
    use aimer_events::pointer::{PointerButton, PointerInfo};
    use aimer_focus::{FocusBehavior, FocusNode};

    use super::{Focusable, RawFocusable};
    use crate::base::{BuildContext, WindowHandle};
    use crate::components::element::EventDispatcher;
    use crate::components::event_element::EventResult;
    use crate::{
        AnyElement, Drawable, Element, EventElement, LayoutElement, Rebuildable, State,
        StatefulWidget, VisitorElement, Widget,
    };

    /// How far the test child extends from the origin.
    const BOX_SIDE: f32 = 100.0;

    /// A point inside the child.
    const INSIDE: Vec2d = Vec2d { x: 50.0, y: 50.0 };

    /// A point that no element of the test tree covers.
    const OUTSIDE: Vec2d = Vec2d { x: 400.0, y: 400.0 };

    /// A leaf occupying a known rectangle, so a press can be aimed at it.
    struct Bounded;

    impl VisitorElement for Bounded {
        fn debug_name(&self) -> &'static str {
            "Bounded"
        }
    }

    impl EventElement for Bounded {}
    impl Rebuildable for Bounded {}

    impl Drawable for Bounded {
        fn update(&self, _ctx: &BuildContext) {}

    }

    impl LayoutElement for Bounded {
        fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
            Some((
                Vec2d::default(),
                Vec2d {
                    x: BOX_SIDE,
                    y: BOX_SIDE,
                },
            ))
        }
    }

    /// The widget behind [`Bounded`], counting how often it is asked for an
    /// element.
    struct Boxed {
        builds: Rc<Cell<usize>>,
    }

    impl Widget for Boxed {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            self.builds.set(self.builds.get() + 1);
            Bounded.boxed()
        }

        fn debug_name(&self) -> &'static str {
            "Boxed"
        }
    }

    impl crate::widget::PortableWidget for Boxed {}

    /// A child nobody counts.
    fn child() -> Boxed {
        Boxed {
            builds: Rc::new(Cell::new(0)),
        }
    }

    /// Where the recording child sits inside its parent.
    const CHILD_POS: Vec2d = Vec2d { x: 12.0, y: 34.0 };

    /// A child that consumes the press that lands on it, the way every control
    /// does.
    struct Greedy;

    impl VisitorElement for Greedy {
        fn debug_name(&self) -> &'static str {
            "Greedy"
        }
    }

    impl Rebuildable for Greedy {}

    impl Drawable for Greedy {
        fn update(&self, _ctx: &BuildContext) {}
    }

    impl LayoutElement for Greedy {
        fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
            Some((
                Vec2d::default(),
                Vec2d {
                    x: BOX_SIDE,
                    y: BOX_SIDE,
                },
            ))
        }
    }

    impl EventElement for Greedy {
        fn on_event(&self, event: &ElementEvent) -> EventResult {
            matches!(event, ElementEvent::PointerDown(_)).into()
        }
    }

    /// A child that keeps state of its own and publishes something to the
    /// subtree below it, the way a selection region does.
    struct Carrier {
        published: Rc<Cell<bool>>,
    }

    impl VisitorElement for Carrier {
        fn debug_name(&self) -> &'static str {
            "Carrier"
        }
    }

    impl EventElement for Carrier {}

    impl Drawable for Carrier {
        fn update(&self, _ctx: &BuildContext) {}
    }

    impl LayoutElement for Carrier {}

    impl Rebuildable for Carrier {
        fn is_carry_state(&self) -> bool {
            true
        }

        fn with_rebuild_context(
            &self,
            ctx: &BuildContext,
            callback: &mut dyn FnMut(&BuildContext),
        ) {
            self.published.set(true);
            callback(ctx);
        }
    }

    /// A child that writes down what reached it.
    ///
    /// It behaves the way a text editor does — it consumes what it typed and
    /// treats a focus change as news — so the tests can tell delivery apart
    /// from consumption.
    struct Recorder {
        seen: Rc<RefCell<Vec<&'static str>>>,
        layouts: Rc<Cell<usize>>,
    }

    impl VisitorElement for Recorder {
        fn debug_name(&self) -> &'static str {
            "Recorder"
        }
    }

    impl Rebuildable for Recorder {}

    impl Drawable for Recorder {
        fn update(&self, _ctx: &BuildContext) {}
    }

    impl LayoutElement for Recorder {
        fn pos(&self) -> Option<Vec2d> {
            Some(CHILD_POS)
        }

        fn layout(&self, _ctx: &BuildContext) -> ResolvedSize {
            self.layouts.set(self.layouts.get() + 1);
            ResolvedSize::default()
        }

        fn pos_start_end(&self) -> Option<(Vec2d, Vec2d)> {
            Some((
                Vec2d::default(),
                Vec2d {
                    x: BOX_SIDE,
                    y: BOX_SIDE,
                },
            ))
        }
    }

    impl EventElement for Recorder {
        fn on_event(&self, event: &ElementEvent) -> EventResult {
            let name = match event {
                ElementEvent::FocusGained => "focus",
                ElementEvent::FocusLost => "blur",
                ElementEvent::KeyInput { .. } => "key",
                ElementEvent::TextInput { .. } => "text",
                _ => return EventResult::ignored(),
            };
            self.seen.borrow_mut().push(name);
            matches!(
                event,
                ElementEvent::KeyInput { .. } | ElementEvent::TextInput { .. }
            )
            .into()
        }
    }

    /// The widget behind [`Recorder`].
    struct Recording {
        seen: Rc<RefCell<Vec<&'static str>>>,
        layouts: Rc<Cell<usize>>,
    }

    impl Widget for Recording {
        fn to_element(self, _ctx: &BuildContext) -> AnyElement {
            Recorder {
                seen: self.seen,
                layouts: self.layouts,
            }
            .boxed()
        }

        fn debug_name(&self) -> &'static str {
            "Recording"
        }
    }

    impl crate::widget::PortableWidget for Recording {}

    /// A recording child that only the events it saw are read back from.
    fn recording(seen: &Rc<RefCell<Vec<&'static str>>>) -> Recording {
        Recording {
            seen: Rc::clone(seen),
            layouts: Rc::new(Cell::new(0)),
        }
    }

    /// Types `text` the way a platform input method commits a phrase.
    fn type_text(dispatcher: &mut EventDispatcher, root: &AnyElement, text: &str) -> EventResult {
        dispatcher.dispatch(
            root.as_ref(),
            OUTSIDE,
            &ElementEvent::TextInput {
                text: text.to_owned(),
                action: KeyAction::Pressed,
                modifiers: Modifiers::default(),
            },
        )
    }

    /// Presses a named key, which the dispatcher offers to the focus owner
    /// first.
    fn press_key(dispatcher: &mut EventDispatcher, root: &AnyElement) -> EventResult {
        dispatcher.dispatch(
            root.as_ref(),
            OUTSIDE,
            &ElementEvent::KeyInput {
                key: NamedKey::ArrowLeft,
                action: KeyAction::Pressed,
                modifiers: Modifiers::default(),
            },
        )
    }

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

    /// Presses at `pos`, the way a mouse would.
    fn press(dispatcher: &mut EventDispatcher, root: &AnyElement, pos: Vec2d) {
        let _ = dispatcher.dispatch(
            root.as_ref(),
            pos,
            &ElementEvent::PointerDown(PointerInfo::mouse(pos, PointerButton::Primary)),
        );
    }

    /// Moves the pointer without pressing, which resolves focus without
    /// deciding it.
    fn hover(dispatcher: &mut EventDispatcher, root: &AnyElement, pos: Vec2d) {
        let _ = dispatcher.dispatch(
            root.as_ref(),
            pos,
            &ElementEvent::PointerMove(PointerInfo::mouse(pos, PointerButton::Primary)),
        );
    }

    /// A press on the region makes it the focus owner.
    #[tokio::test]
    async fn a_press_focuses_the_region_it_lands_on() {
        let node = FocusNode::new();
        let ctx = context();
        let root = Focusable::new()
            .node(node.clone())
            .child(child())
            .to_element(&ctx);
        let mut dispatcher = EventDispatcher::new();

        press(&mut dispatcher, &root, INSIDE);

        assert!(node.has_focus(), "a press must focus what it landed on");
    }

    /// A region is only where its child is: a press that missed the child must
    /// not focus it, which is what an element that forgets to report its bounds
    /// gets wrong.
    #[tokio::test]
    async fn a_press_that_missed_the_region_takes_focus_away() {
        let node = FocusNode::new();
        let ctx = context();
        let root = Focusable::new()
            .node(node.clone())
            .child(child())
            .to_element(&ctx);
        let mut dispatcher = EventDispatcher::new();

        press(&mut dispatcher, &root, INSIDE);
        press(&mut dispatcher, &root, OUTSIDE);

        assert!(
            !node.has_focus(),
            "a press on nothing focusable must clear the owner"
        );
    }

    /// An ignored region is never a target, however precisely it is pressed.
    #[tokio::test]
    async fn an_ignored_region_is_not_a_focus_target() {
        let node = FocusNode::new();
        let ctx = context();
        let root = Focusable::new()
            .node(node.clone())
            .behavior(FocusBehavior::Ignore)
            .child(child())
            .to_element(&ctx);
        let mut dispatcher = EventDispatcher::new();

        press(&mut dispatcher, &root, INSIDE);

        assert!(!node.has_focus());
    }

    /// Ignoring hides the region itself, not what it contains.
    #[tokio::test]
    async fn an_ignored_region_still_offers_the_targets_inside_it() {
        let outer = FocusNode::new();
        let inner = FocusNode::new();
        let ctx = context();
        let root = Focusable::new()
            .node(outer.clone())
            .behavior(FocusBehavior::Ignore)
            .child(Focusable::new().node(inner.clone()).child(child()))
            .to_element(&ctx);
        let mut dispatcher = EventDispatcher::new();

        press(&mut dispatcher, &root, INSIDE);

        assert!(inner.has_focus(), "the inner region is still a target");
        assert!(!outer.has_focus());
    }

    /// An automatic region takes focus as soon as it is in the tree, without
    /// anybody pointing at it.
    #[tokio::test]
    async fn an_automatic_region_takes_focus_when_it_appears() {
        let node = FocusNode::new();
        let ctx = context();
        let root = Focusable::new()
            .node(node.clone())
            .behavior(FocusBehavior::Auto)
            .child(child())
            .to_element(&ctx);
        let mut dispatcher = EventDispatcher::new();

        hover(&mut dispatcher, &root, OUTSIDE);

        assert!(
            node.has_focus(),
            "an automatic region asks for focus itself"
        );
    }

    /// The reason this widget has state at all.
    ///
    /// A focus target *is* its node, so a region that made a new one on every
    /// rebuild would be a different target each time — and a rebuild happens
    /// whenever anything above it changes, including as a result of the focus
    /// change itself. The state keeps one node, so the target survives.
    #[tokio::test]
    async fn the_same_target_survives_a_rebuild() {
        let ctx = context();
        let state = Focusable::new().child(child()).create_state();

        let first = state.build(&ctx).to_element(&ctx);
        let second = state.build(&ctx).to_element(&ctx);

        let (Some(first), Some(second)) = (first.focus_node(), second.focus_node()) else {
            panic!("a focusable region must offer a node");
        };
        assert!(
            first.ptr_eq(second),
            "a rebuild must not replace the focus target"
        );
    }

    /// A region that makes its own node keeps it when the widget describing it
    /// is replaced, which is what a rebuild of the parent does.
    ///
    /// The replacement's node is a different target, so adopting it would move
    /// focus off the region every time anything above it changed; the rest of
    /// the replacement's configuration is adopted as usual.
    #[tokio::test]
    async fn a_replaced_region_keeps_its_target_and_takes_the_new_configuration() {
        let mut live = Focusable::new().child(child()).create_state();
        let node = live.node.clone();
        let replacement = Focusable::new()
            .behavior(FocusBehavior::Ignore)
            .child(child())
            .create_state();

        live.adopt_config_from(replacement);

        assert!(
            live.node.ptr_eq(&node),
            "the focus target must not be swapped"
        );
        assert_eq!(live.behavior, FocusBehavior::Ignore);
    }

    /// A node handed in from outside is the application's to choose, so a
    /// replacement carrying one is obeyed.
    #[tokio::test]
    async fn a_replaced_region_adopts_a_node_it_was_given() {
        let mut live = Focusable::new().child(child()).create_state();
        let node = FocusNode::new();
        let replacement = Focusable::new()
            .node(node.clone())
            .child(child())
            .create_state();

        live.adopt_config_from(replacement);

        assert!(live.node.ptr_eq(&node));
    }

    /// A region that rebuilds itself must reach the same child again rather
    /// than build another one, or the subtree would lose its own state
    /// whenever focus moved.
    #[tokio::test]
    async fn a_rebuild_reuses_the_child_element() {
        let builds = Rc::new(Cell::new(0));
        let ctx = context();
        let state = Focusable::new()
            .child(Boxed {
                builds: Rc::clone(&builds),
            })
            .create_state();

        state.build(&ctx).to_element(&ctx);
        state.build(&ctx).to_element(&ctx);

        assert_eq!(builds.get(), 1, "the child must be retained, not rebuilt");
    }

    /// The region is exactly where its child is.
    #[tokio::test]
    async fn the_region_reports_the_bounds_of_its_child() {
        let ctx = context();
        let element = Focusable::new()
            .child(child())
            .create_state()
            .build(&ctx)
            .to_element(&ctx);

        assert_eq!(
            element.pos_start_end(),
            Some((
                Vec2d::default(),
                Vec2d {
                    x: BOX_SIDE,
                    y: BOX_SIDE
                }
            ))
        );
    }

    /// Gaining and losing focus is reported, and reported as news: consuming
    /// the notification would hide it from a child that also cares.
    #[tokio::test]
    async fn focus_changes_are_reported_without_being_consumed() {
        let gained = Rc::new(Cell::new(0));
        let lost = Rc::new(Cell::new(0));
        let changes: Rc<Cell<Option<bool>>> = Rc::new(Cell::new(None));
        let ctx = context();
        let element = Focusable::new()
            .on_focus({
                let gained = Rc::clone(&gained);
                move || gained.set(gained.get() + 1)
            })
            .on_focus_lost({
                let lost = Rc::clone(&lost);
                move || lost.set(lost.get() + 1)
            })
            .on_focus_change({
                let changes = Rc::clone(&changes);
                move |focused| changes.set(Some(focused))
            })
            .child(child())
            .create_state()
            .build(&ctx)
            .to_element(&ctx);

        let gain = element.on_event(&ElementEvent::FocusGained);
        assert_eq!(
            (gained.get(), lost.get(), changes.get()),
            (1, 0, Some(true))
        );
        assert!(!gain.is_consumed(), "a notification is news, not a request");
        assert!(gain.needs_redraw(), "the region is painted differently now");

        let loss = element.on_event(&ElementEvent::FocusLost);
        assert_eq!(
            (gained.get(), lost.get(), changes.get()),
            (1, 1, Some(false))
        );
        assert!(!loss.is_consumed());
    }

    /// The region owns the focus, but it is what the region *contains* that
    /// edits: typed text has to travel the last step inwards or a field wrapped
    /// in a focusable region would be deaf.
    #[tokio::test]
    async fn typed_text_reaches_the_child_of_the_focused_region() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let ctx = context();
        let root = Focusable::new().child(recording(&seen)).to_element(&ctx);
        let mut dispatcher = EventDispatcher::new();

        press(&mut dispatcher, &root, INSIDE);
        let result = type_text(&mut dispatcher, &root, "hi");

        assert_eq!(seen.borrow().as_slice(), ["focus", "text"]);
        assert!(
            result.is_consumed(),
            "the child's answer has to travel back out"
        );
    }

    /// Named keys are offered to the focus owner before anything else, so an
    /// editor inside a focusable region must see them there.
    #[tokio::test]
    async fn a_named_key_reaches_the_child_of_the_focused_region() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let ctx = context();
        let root = Focusable::new().child(recording(&seen)).to_element(&ctx);
        let mut dispatcher = EventDispatcher::new();

        press(&mut dispatcher, &root, INSIDE);
        let result = press_key(&mut dispatcher, &root);

        assert_eq!(seen.borrow().as_slice(), ["focus", "key"]);
        assert!(result.is_consumed());
    }

    /// Nothing keyboard-bound is offered inside a region that does not hold the
    /// focus, which is what keeps a broadcast from being delivered twice.
    #[tokio::test]
    async fn an_unfocused_region_keeps_typed_text_out() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let ctx = context();
        let root = Focusable::new().child(recording(&seen)).to_element(&ctx);
        let mut dispatcher = EventDispatcher::new();

        let _ = type_text(&mut dispatcher, &root, "hi");

        assert!(seen.borrow().is_empty());
    }

    /// A child that reacts to focus — a caret that starts blinking, a field
    /// that opens an input-method session — hears about it through the region
    /// that owns the node.
    #[tokio::test]
    async fn focus_notifications_reach_the_child() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let ctx = context();
        let root = Focusable::new().child(recording(&seen)).to_element(&ctx);
        let mut dispatcher = EventDispatcher::new();

        press(&mut dispatcher, &root, INSIDE);
        press(&mut dispatcher, &root, OUTSIDE);

        assert_eq!(seen.borrow().as_slice(), ["focus", "blur"]);
    }

    /// Eligibility that changes without a rebuild.
    ///
    /// A region whose focusability follows live state — a selection that only
    /// owns the keyboard while it exists — cannot spell that as a fixed
    /// behavior, because nothing rebuilds it when the state turns over.
    #[tokio::test]
    async fn a_gate_decides_focusability_every_time_it_is_asked() {
        let open = Rc::new(Cell::new(false));
        let node = FocusNode::new();
        let ctx = context();
        let root = Focusable::new()
            .node(node.clone())
            .focusable_when({
                let open = Rc::clone(&open);
                move || open.get()
            })
            .child(child())
            .to_element(&ctx);
        let mut dispatcher = EventDispatcher::new();

        press(&mut dispatcher, &root, INSIDE);
        assert!(!node.has_focus(), "a closed gate is not a target");

        open.set(true);
        press(&mut dispatcher, &root, INSIDE);
        assert!(
            node.has_focus(),
            "the gate is asked again, never remembered"
        );
    }

    /// Laying a region out lays its child out.
    ///
    /// Measuring is not laying out: an element that answers with its child's
    /// computed size alone leaves the child never placed, which is how a
    /// wrapper silently flattens whatever it wraps.
    #[tokio::test]
    async fn the_child_is_laid_out_through_the_region() {
        let layouts = Rc::new(Cell::new(0));
        let ctx = context();
        let element = Focusable::new()
            .child(Recording {
                seen: Rc::new(RefCell::new(Vec::new())),
                layouts: Rc::clone(&layouts),
            })
            .create_state()
            .build(&ctx)
            .to_element(&ctx);

        element.layout(&ctx);

        assert_eq!(
            layouts.get(),
            1,
            "the child has to be laid out, not measured"
        );
    }

    /// The region sits exactly where its child sits.
    #[tokio::test]
    async fn the_region_reports_the_position_of_its_child() {
        let ctx = context();
        let element = Focusable::new()
            .child(recording(&Rc::new(RefCell::new(Vec::new()))))
            .to_element(&ctx);

        assert_eq!(element.pos(), Some(CHILD_POS));
    }

    /// An element that already exists is made a target without a widget, a
    /// state or a retained child in between.
    ///
    /// This is what a control that builds its own element uses instead of
    /// answering [`EventElement::focus_node`] itself.
    #[tokio::test]
    async fn an_element_becomes_a_target_without_a_widget_around_it() {
        let node = FocusNode::new();
        let root = RawFocusable::new(node.clone(), Bounded.boxed()).boxed();
        let mut dispatcher = EventDispatcher::new();

        press(&mut dispatcher, &root, INSIDE);

        assert!(node.has_focus());
    }

    /// A press the child takes for itself still focuses the region around it.
    ///
    /// A control inside a focusable region — a text field, a list row's button
    /// — consumes the press that lands on it, and that press still landed
    /// *inside* the region. A router that stopped looking for a focus target
    /// there would find none, and a press that finds no target takes the
    /// keyboard away: clicking the field would blur it.
    #[tokio::test]
    async fn a_press_the_child_consumes_still_focuses_the_region() {
        let node = FocusNode::new();
        let root = RawFocusable::new(node.clone(), Greedy.boxed()).boxed();
        let mut dispatcher = EventDispatcher::new();

        press(&mut dispatcher, &root, INSIDE);

        assert!(node.has_focus());
    }

    /// An attachment is not a layer of its own as far as rebuilding goes.
    ///
    /// A parent asks its child whether the subtree keeps its own state across a
    /// rebuild, and publishes whatever the child publishes; a wrapper that
    /// answered for itself would tell the parent that a region carrying a
    /// selection carries nothing.
    #[tokio::test]
    async fn an_attachment_answers_for_the_child_it_wraps() {
        let published = Rc::new(Cell::new(false));
        let element = RawFocusable::new(
            FocusNode::new(),
            Carrier {
                published: Rc::clone(&published),
            }
            .boxed(),
        );
        let ctx = context();

        assert!(element.is_carry_state(), "the child carries state");
        element.with_rebuild_context(&ctx, &mut |_| {});
        assert!(published.get(), "the child's scope must still be published");
    }

    /// A node handed in from outside is the target, so an application can move
    /// focus imperatively.
    #[tokio::test]
    async fn a_supplied_node_can_claim_focus_imperatively() {
        let node = FocusNode::new();
        let ctx = context();
        let root = Focusable::new()
            .node(node.clone())
            .child(child())
            .to_element(&ctx);
        let mut dispatcher = EventDispatcher::new();

        node.request_focus();
        hover(&mut dispatcher, &root, OUTSIDE);

        assert!(node.has_focus());
    }

    /// A text field takes a press through the focus system and consumes nothing,
    /// so whether a press landed on a focus target is how a host tells a press an
    /// element took from one that fell through to the window behind.
    #[tokio::test]
    async fn a_press_reports_whether_it_landed_on_a_focus_target() {
        let node = FocusNode::new();
        let ctx = context();
        let root = Focusable::new()
            .node(node.clone())
            .child(child())
            .to_element(&ctx);
        let mut dispatcher = EventDispatcher::new();
        assert!(!dispatcher.last_press_hit_focus_target());

        press(&mut dispatcher, &root, OUTSIDE);
        assert!(!dispatcher.last_press_hit_focus_target());

        press(&mut dispatcher, &root, INSIDE);
        assert!(dispatcher.last_press_hit_focus_target());

        press(&mut dispatcher, &root, OUTSIDE);
        assert!(
            !dispatcher.last_press_hit_focus_target(),
            "the answer is about the latest press"
        );
    }

    /// A region that asks for focus when it appears is granted it by the frame
    /// that built it. Nothing has to be pressed, moved or typed first: before
    /// this, the grant waited for the next input event, so a field focusing
    /// itself on arrival heard about it — and, say, started its focus
    /// animation — only once the user touched something.
    #[tokio::test]
    async fn an_automatic_region_is_settled_without_an_event() {
        let gained = Rc::new(Cell::new(0));
        let node = FocusNode::new();
        let ctx = context();
        let root = Focusable::new()
            .node(node.clone())
            .behavior(FocusBehavior::Auto)
            .on_focus({
                let gained = Rc::clone(&gained);
                move || gained.set(gained.get() + 1)
            })
            .child(child())
            .to_element(&ctx);
        let mut dispatcher = EventDispatcher::new();
        assert!(
            dispatcher.has_pending_focus_work(),
            "an automatic region owes the frame a focus grant"
        );

        let result = dispatcher.settle_focus(root.as_ref());

        assert!(node.has_focus());
        assert_eq!(gained.get(), 1, "the region is told it gained focus");
        assert!(result.needs_redraw(), "the focused look needs a frame");
        assert!(!dispatcher.has_pending_focus_work());

        let _ = dispatcher.settle_focus(root.as_ref());
        assert_eq!(gained.get(), 1, "an edge is reported once");
    }

    /// Settling costs counters alone while nobody has asked for anything, so the
    /// frame loop can ask on every frame.
    #[tokio::test]
    async fn a_region_that_never_asked_leaves_nothing_to_settle() {
        let ctx = context();
        let node = FocusNode::new();
        let root = Focusable::new()
            .node(node.clone())
            .child(child())
            .to_element(&ctx);
        let mut dispatcher = EventDispatcher::new();

        assert!(!dispatcher.has_pending_focus_work());
        let result = dispatcher.settle_focus(root.as_ref());

        assert!(!node.has_focus());
        assert!(!result.needs_redraw());
    }

    /// A node asked to focus from outside any event — a timer, a finished task
    /// — is granted at the next settle rather than at the next tap.
    #[tokio::test]
    async fn a_focus_request_made_outside_an_event_is_settled() {
        let node = FocusNode::new();
        let ctx = context();
        let root = Focusable::new()
            .node(node.clone())
            .child(child())
            .to_element(&ctx);
        let mut dispatcher = EventDispatcher::new();
        let _ = dispatcher.settle_focus(root.as_ref());
        assert!(!node.has_focus());

        node.request_focus();
        assert!(dispatcher.has_pending_focus_work());
        let _ = dispatcher.settle_focus(root.as_ref());

        assert!(node.has_focus());
    }
}

#[cfg(all(test, feature = "portable-guest"))]
mod portable_tests {
    use super::Focusable;
    use crate::portable::PortableWidgetSchema;
    use crate::RequiredChild;

    #[test]
    fn focusable_schema_keeps_callbacks_and_its_retained_child() {
        let schema = <Focusable<RequiredChild> as PortableWidgetSchema>::SCHEMA;

        assert_eq!(
            schema.widget().canonical_name(),
            "aimer.widget:aimer_widget::Focusable"
        );
        assert_eq!(schema.properties().len(), 0);
        assert_eq!(schema.callbacks().len(), 2);
        assert_eq!(schema.children(), aimer_anteros::ChildCardinality::exactly(1));
    }
}
