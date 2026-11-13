//! Std-only stand-in for the framework pieces the designs need.
//!
//! This is *not* the real element/arena machinery: the updater holds an `Rc`
//! (the real one is a `Copy` generational handle), nested stateful children
//! are re-created on every build (no child reconciliation), and everything is
//! single threaded. It exists to compare trait shapes, not to measure.

use std::cell::{Cell, Ref, RefCell};
use std::rc::Rc;

/// Empty stand-in for the framework `BuildContext`.
pub struct BuildContext;

/// What a build produces, flattened so tests can read and "click" it.
#[derive(Clone)]
pub enum Node {
    Text(String),
    Button { label: String, on_press: Rc<dyn Fn()> },
    Column(Vec<Node>),
}

impl Node {
    pub fn render(&self) -> String {
        match self {
            Node::Text(text) => text.clone(),
            Node::Button { label, .. } => format!("[{label}]"),
            Node::Column(children) => children.iter().map(Node::render).collect::<Vec<_>>().join(" "),
        }
    }

    /// Invokes the first button labelled `target`; returns whether one was found.
    pub fn press(&self, target: &str) -> bool {
        match self {
            Node::Button { label, on_press } if label == target => {
                on_press();
                true
            }
            Node::Column(children) => children.iter().any(|child| child.press(target)),
            _ => false,
        }
    }
}

pub trait Widget {
    fn to_element(self) -> Box<dyn Element>;
}

pub trait Element {
    fn render(&mut self, ctx: &BuildContext) -> Node;
    fn is_dirty(&self) -> bool {
        false
    }
}

struct Leaf(Node);

impl Element for Leaf {
    fn render(&mut self, _ctx: &BuildContext) -> Node {
        self.0.clone()
    }
}

pub struct Text(String);

impl Text {
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }
}

impl Widget for Text {
    fn to_element(self) -> Box<dyn Element> {
        Box::new(Leaf(Node::Text(self.0)))
    }
}

pub struct Button {
    label: String,
    on_press: Rc<dyn Fn()>,
}

impl Button {
    pub fn new(label: impl Into<String>, on_press: impl Fn() + 'static) -> Self {
        Self { label: label.into(), on_press: Rc::new(on_press) }
    }
}

impl Widget for Button {
    fn to_element(self) -> Box<dyn Element> {
        Box::new(Leaf(Node::Button { label: self.label, on_press: self.on_press }))
    }
}

#[derive(Default)]
pub struct Column(Vec<Box<dyn Element>>);

impl Column {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn child(mut self, child: impl Widget) -> Self {
        self.0.push(child.to_element());
        self
    }
}

impl Element for Column {
    fn render(&mut self, ctx: &BuildContext) -> Node {
        Node::Column(self.0.iter_mut().map(|child| child.render(ctx)).collect())
    }

    fn is_dirty(&self) -> bool {
        self.0.iter().any(|child| child.is_dirty())
    }
}

impl Widget for Column {
    fn to_element(self) -> Box<dyn Element> {
        Box::new(self)
    }
}

/// Where a live state and its queued mutations live.
pub struct Slot<S> {
    pub(crate) state: RefCell<S>,
    queue: RefCell<Vec<Box<dyn FnOnce(&mut S)>>>,
    pub(crate) dirty: Cell<bool>,
}

impl<S> Slot<S> {
    fn new(state: S) -> Rc<Self> {
        Rc::new(Self { state: RefCell::new(state), queue: RefCell::default(), dirty: Cell::new(false) })
    }

    /// Applies queued mutations, including ones queued by a mutation itself.
    pub(crate) fn apply_pending(&self) {
        loop {
            // The queue borrow ends before any mutation runs, so a mutation may enqueue more.
            let batch = std::mem::take(&mut *self.queue.borrow_mut());
            if batch.is_empty() {
                break;
            }
            let mut state = self.state.borrow_mut();
            for mutation in batch {
                mutation(&mut state);
            }
        }
        self.dirty.set(false);
    }
}

/// Handle that queues mutations of a state from callbacks.
///
/// The state no longer stores this: the framework passes it into `build`.
pub struct StateUpdater<S> {
    slot: Rc<Slot<S>>,
}

impl<S> Clone for StateUpdater<S> {
    fn clone(&self) -> Self {
        Self { slot: self.slot.clone() }
    }
}

impl<S: 'static> StateUpdater<S> {
    pub(crate) fn new(slot: Rc<Slot<S>>) -> Self {
        Self { slot }
    }

    /// Queues `mutation`; it runs before the next build, and several calls cost one rebuild.
    pub fn set_state(&self, mutation: impl FnOnce(&mut S) + 'static) {
        self.slot.queue.borrow_mut().push(Box::new(mutation));
        self.slot.dirty.set(true);
    }

    /// Borrows the state. Panics inside a queued mutation, where it is mutably borrowed.
    pub fn read(&self) -> Ref<'_, S> {
        self.slot.state.borrow()
    }

    /// A press-handler shape: every call queues `mutation`. Replaces the
    /// `let updater = self.updater; move || updater.set_state(..)` dance.
    pub fn callback(&self, mutation: impl Fn(&mut S) + 'static) -> impl Fn() + 'static {
        let this = self.clone();
        let mutation = Rc::new(mutation);
        move || {
            let mutation = mutation.clone();
            this.set_state(move |state| mutation(state));
        }
    }
}

/// What `State::build` receives: a view of the state, plus the updater.
///
/// Derefs to the state, so `build` reads fields directly (`this.count`). The view
/// borrows the state for the one `build` call; that borrow is the whole safety
/// argument: queued mutations only run before a build, so nothing can change the
/// state while the view exists, and a closure that captured the view could not be
/// `'static`. A handler that has to read the state when it runs takes
/// [`StateView::updater`] instead.
pub struct StateView<'a, S: 'static> {
    state: &'a S,
    updater: StateUpdater<S>,
}

impl<'a, S: 'static> StateView<'a, S> {
    pub(crate) fn new(state: &'a S, updater: StateUpdater<S>) -> Self {
        Self { state, updater }
    }

    /// The `'static` handle, for closures that outlive this build.
    pub fn updater(&self) -> StateUpdater<S> {
        self.updater.clone()
    }

    /// Queues `mutation`; see [`StateUpdater::set_state`].
    pub fn set_state(&self, mutation: impl FnOnce(&mut S) + 'static) {
        self.updater.set_state(mutation);
    }

    /// A press-handler shape; see [`StateUpdater::callback`].
    pub fn callback(&self, mutation: impl Fn(&mut S) + 'static) -> impl Fn() + 'static {
        self.updater.callback(mutation)
    }
}

impl<S: 'static> std::ops::Deref for StateView<'_, S> {
    type Target = S;

    fn deref(&self) -> &S {
        self.state
    }
}

/// The widget-side half of an element: keeps the widget until the state exists,
/// then keeps the state. Shared by both designs so they differ only in trait shape.
pub struct Mount<W, S> {
    widget: Option<W>,
    pub(crate) slot: Option<Rc<Slot<S>>>,
}

impl<W, S: 'static> Mount<W, S> {
    pub fn new(widget: W) -> Self {
        Self { widget: Some(widget), slot: None }
    }

    /// Creates the state on first use (running `init`), then returns its slot.
    pub(crate) fn slot_or_create(
        &mut self,
        create: impl FnOnce(W) -> S,
        init: impl FnOnce(&mut S),
    ) -> Rc<Slot<S>> {
        if let Some(slot) = &self.slot {
            return slot.clone();
        }
        let widget = self.widget.take().expect("the widget is kept until the state is created");
        // `init` runs on the bare state, before any slot or updater exists: it is a plain
        // `&mut` mutation, and nothing can have queued or read anything yet.
        let mut state = create(widget);
        init(&mut state);
        let slot = Slot::new(state);
        self.slot = Some(slot.clone());
        slot
    }

    /// A parent re-emitted the widget: keep the live state, hand it the new props.
    pub fn reconcile(&mut self, widget: W, update: impl FnOnce(&mut S, W)) {
        match &self.slot {
            Some(slot) => update(&mut slot.state.borrow_mut(), widget),
            None => self.widget = Some(widget),
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.slot.as_ref().is_some_and(|slot| slot.dirty.get())
    }
}
