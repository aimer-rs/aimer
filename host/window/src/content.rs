//! The root of a window's widget tree.
//!
//! A window's content is a closure, not a widget value: a widget is consumed
//! when it is built, so a window whose content could only be handed over once
//! could never be rebuilt from outside. With a closure the window's root can be
//! rebuilt whenever its owner supplies a new one, which is how a
//! [`Window`](crate::Window) stays in step with the widget that declares it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, AnyWidget, PortableWidget, State, StateUpdater, StatefulElement, StatefulWidget,
    Widget,
};

/// Builds the content of a window.
pub(crate) type ContentFn = dyn Fn(&BuildContext) -> AnyWidget;

/// The content of one window, shared by whoever may replace it and the root
/// that shows it.
pub(crate) struct WindowContent {
    builder: RefCell<Rc<ContentFn>>,
    /// How to ask the root to build again, once the root exists.
    refresh: Cell<Option<StateUpdater<LiveRootState>>>,
}

impl WindowContent {
    pub(crate) fn new(builder: Rc<ContentFn>) -> Rc<Self> {
        Rc::new(Self {
            builder: RefCell::new(builder),
            refresh: Cell::new(None),
        })
    }

    /// Shows what `builder` builds from now on.
    ///
    /// The window rebuilds at its next frame. The updater remembers the window
    /// it belongs to, so this may be called from any window.
    pub(crate) fn replace(&self, builder: Rc<ContentFn>) {
        *self.builder.borrow_mut() = builder;
        if let Some(refresh) = self.refresh.get() {
            let _ = refresh.try_set_state(|_| {});
        }
    }

    pub(crate) fn current(&self) -> Rc<ContentFn> {
        self.builder.borrow().clone()
    }

    /// The root widget of a window showing this content.
    pub(crate) fn root(self: &Rc<Self>) -> AnyWidget {
        LiveRoot(self.clone()).boxed()
    }
}

struct LiveRoot(Rc<WindowContent>);

struct LiveRootState {
    content: Rc<WindowContent>,
}

impl StatefulWidget for LiveRoot {
    type State = LiveRootState;

    fn create_state(self) -> LiveRootState {
        LiveRootState { content: self.0 }
    }
}

impl State<LiveRoot> for LiveRootState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.content.refresh.set(Some(updater));
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        // Cloned out first: the content may build something that replaces it.
        let builder = self.content.current();
        builder(ctx)
    }
}

impl Widget for LiveRoot {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "WindowContent", None)
            .0
            .boxed()
    }
}

impl PortableWidget for LiveRoot {}
