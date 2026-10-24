//! The `Window` widget and `NewWindow` against the request queue.
//!
//! No operating-system window is involved: the event loop's half of the
//! conversation is played by the test, which drains the queue the way the host
//! does, and a second headless application stands in for the window the host
//! would create.

use super::*;

use std::any::TypeId;

use aimer_widget::StateUpdater;
use aimer_window::host::{OpenRequest, Request, WindowCommand, drain};
use aimer_window::{NewWindow, Window, WindowId};

/// A provider-like value, to see whether a window inherits it.
struct Marker(u32);

type Updater = Rc<RefCell<Option<StateUpdater<ParentState>>>>;

/// What the window content saw when it built: its own number, and the marker
/// provider in scope for it, if any.
type Built = Rc<RefCell<Vec<(u32, Option<u32>)>>>;

struct Parent {
    updater: Updater,
    closed: Rc<Cell<usize>>,
    built: Built,
    provide: Option<u32>,
    isolated: bool,
}

struct ParentState {
    updater: Updater,
    closed: Rc<Cell<usize>>,
    built: Built,
    provide: Option<u32>,
    isolated: bool,
    mounted: bool,
    open: bool,
    title: &'static str,
    size: (u32, u32),
    /// What the window's content records when it builds, to tell content apart.
    content: u32,
}

impl StatefulWidget for Parent {
    type State = ParentState;

    fn create_state(self) -> ParentState {
        ParentState {
            updater: self.updater,
            closed: self.closed,
            built: self.built,
            provide: self.provide,
            isolated: self.isolated,
            mounted: true,
            open: true,
            title: "First",
            size: (480, 320),
            content: 1,
        }
    }
}

impl State<Parent> for ParentState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        *self.updater.borrow_mut() = Some(updater);
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let closed = self.closed.clone();
        let built = self.built.clone();
        let content = self.content;
        let mut window = Window::new()
            .title(self.title)
            .size(self.size.0, self.size.1)
            .open(self.open)
            .on_close(move || closed.set(closed.get() + 1));
        if self.isolated {
            window = window.isolated();
        }
        let window = window.builder(move |ctx| {
            let marker = ctx.get_state::<Marker>().map(|marker| marker.0);
            built.borrow_mut().push((content, marker));
            SizedBox::new()
        });
        let child = if self.mounted {
            window.to_element(ctx)
        } else {
            SizedBox::new().to_element(ctx)
        };
        let provided = self.provide;
        ElementPassthrough { child, provided }
    }
}

/// Places an already-built child, with a provider around it when asked.
struct ElementPassthrough {
    child: AnyElement,
    provided: Option<u32>,
}

impl Widget for ElementPassthrough {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        self.child
    }
}

impl aimer_widget::PortableWidget for ElementPassthrough {}

impl Widget for Parent {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let provide = self.provide;
        let build = |ctx: &BuildContext| {
            StatefulElement::new_with_name(self, ctx, "WindowApiParent", None)
                .0
                .boxed()
        };
        match provide {
            Some(value) => ctx.with_state(Marker(value), build),
            None => build(ctx),
        }
    }
}

impl aimer_widget::PortableWidget for Parent {}

struct Harness {
    app: HeadlessAimerApp<aimer_modal::ModalHost<Parent>>,
    updater: Updater,
    closed: Rc<Cell<usize>>,
    built: Built,
}

fn harness(provide: Option<u32>, isolated: bool) -> Harness {
    drain();
    let updater = Updater::default();
    let closed = Rc::new(Cell::new(0));
    let built: Built = Rc::default();
    let mut app = AimerApp::start_headless(Parent {
        updater: updater.clone(),
        closed: closed.clone(),
        built: built.clone(),
        provide,
        isolated,
    });
    app.pump_frames(8);
    Harness {
        app,
        updater,
        closed,
        built,
    }
}

impl Harness {
    /// Changes the parent's state and lets it rebuild.
    fn change(&mut self, change: impl FnOnce(&mut ParentState) + 'static) {
        let updater = self.updater.borrow().expect("the parent was built");
        updater.set_state(change);
        self.app.pump_frames(8);
    }
}

fn opens() -> Vec<OpenRequest> {
    drain()
        .into_iter()
        .filter_map(|request| match request {
            Request::Open(open) => Some(open),
            Request::Command(..) => None,
        })
        .collect()
}

fn commands() -> Vec<(WindowId, WindowCommand)> {
    drain()
        .into_iter()
        .filter_map(|request| match request {
            Request::Command(id, command) => Some((id, command)),
            Request::Open(_) => None,
        })
        .collect()
}

#[test]
fn a_window_widget_asks_for_its_window_once() {
    let mut page = harness(None, false);
    let requests = drain();
    let opened: Vec<_> = requests
        .iter()
        .filter_map(|request| match request {
            Request::Open(open) => Some(open),
            Request::Command(..) => None,
        })
        .collect();

    assert_eq!(opened.len(), 1, "one widget, one window");
    assert_eq!(
        opened[0].attr,
        WindowAttr::new().title("First").inner_size(480, 320)
    );

    // Rebuilding the parent for a reason the window does not care about must
    // not ask for a second window.
    page.change(|state| state.content = 1);
    page.app.render_frame();
    assert!(opens().is_empty(), "a rebuild asked for another window");
}

#[test]
fn closing_it_from_the_state_closes_the_window_once_and_does_not_tell_it_back() {
    let mut page = harness(None, false);
    let open = opens().remove(0);
    let id = open.id;

    page.change(|state| state.open = false);
    page.app.render_frame();
    assert_eq!(commands(), [(id, WindowCommand::Close)]);

    // The host reports the close it was asked for.
    (open.on_closed.expect("the host is told when it closes"))();
    assert_eq!(
        page.closed.get(),
        0,
        "the widget was told about a close it asked for"
    );
}

#[test]
fn removing_the_widget_closes_its_window() {
    let mut page = harness(None, false);
    let id = opens().remove(0).id;

    page.change(|state| state.mounted = false);

    assert_eq!(commands(), [(id, WindowCommand::Close)]);
}

#[test]
fn a_window_the_user_closed_stays_closed_until_the_state_has_said_close() {
    let mut page = harness(None, false);
    let open = opens().remove(0);

    // The host reports the user closing the window.
    (open.on_closed.expect("the host is told when it closes"))();
    assert_eq!(page.closed.get(), 1, "on_close was not called");

    page.change(|state| state.title = "Still open in the state");
    assert!(opens().is_empty(), "a closed window was opened again");

    page.change(|state| state.open = false);
    let after_false = commands();
    page.change(|state| state.open = true);
    assert!(
        after_false.iter().all(|(_, command)| *command == WindowCommand::Close),
        "closing an already closed window asks for nothing else"
    );
    assert_eq!(opens().len(), 1, "open → false → true opens the window again");
}

#[test]
fn a_new_title_updates_the_window_and_an_unchanged_size_does_not() {
    let mut page = harness(None, false);
    let id = opens().remove(0).id;

    page.change(|state| state.title = "Second");

    assert_eq!(commands(), [(id, WindowCommand::SetTitle("Second".to_owned()))]);
}

#[test]
fn a_new_size_resizes_the_window_once() {
    let mut page = harness(None, false);
    let id = opens().remove(0).id;

    page.change(|state| state.size = (600, 400));
    page.change(|state| state.content = 2);

    assert_eq!(commands(), [(id, WindowCommand::Resize(600, 400))]);
}

/// The marker the provider put in a map of inherited states, if it is there.
fn marker_in(
    states: &Rc<RefCell<aimer_widget::components::context::InheritedStates>>,
) -> Option<u32> {
    states
        .borrow()
        .get(&TypeId::of::<Marker>())
        .and_then(|state| state.clone().downcast::<Marker>().ok())
        .map(|marker| marker.0)
}

#[test]
fn a_window_inherits_the_providers_where_it_was_declared() {
    let _page = harness(Some(7), false);
    let open = opens().remove(0);

    let inherited = open.inherited.expect("the widget inherits by default");
    assert_eq!(marker_in(&inherited), Some(7));
}

#[test]
fn an_isolated_window_starts_with_no_providers() {
    let _page = harness(Some(7), true);
    let open = opens().remove(0);

    assert!(open.inherited.is_none());
}

/// Stands in for the event loop: a headless application showing the root a
/// request carries, started with the providers the request carries.
fn open_like_the_host(open: OpenRequest) -> HeadlessAimerApp<aimer_modal::ModalHost<aimer_widget::AnyWidget>> {
    let mut window = AimerApp::start_headless(open.root);
    window.app.inherited_states = open.inherited;
    window.pump_frames(8);
    window
}

#[test]
fn the_opened_window_builds_its_content_with_the_inherited_providers() {
    let page = harness(Some(7), false);
    let _window = open_like_the_host(opens().remove(0));

    assert_eq!(*page.built.borrow(), [(1, Some(7))]);
}

#[test]
fn an_isolated_window_builds_without_the_providers() {
    let page = harness(Some(7), true);
    let _window = open_like_the_host(opens().remove(0));

    assert_eq!(*page.built.borrow(), [(1, None)]);
}

#[test]
fn rebuilding_the_declaring_widget_rebuilds_the_windows_content() {
    let mut page = harness(Some(7), false);
    let mut window = open_like_the_host(opens().remove(0));

    // The parent says something new; the window, a tree of its own on its own
    // application, must show it without anyone touching the window.
    page.change(|state| state.content = 2);
    window.pump_frames(8);

    assert_eq!(page.built.borrow().last(), Some(&(2, Some(7))));
}

/// Opens a window from inside a build, as a callback would.
struct Opener {
    inherit: bool,
}

impl Widget for Opener {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let window = NewWindow::new();
        let window = if self.inherit { window.inherit(ctx) } else { window };
        window.builder(|_ctx| SizedBox::new()).open();
        SizedBox::new().to_element(ctx)
    }
}

impl aimer_widget::PortableWidget for Opener {}

/// Puts a marker provider in scope around `Opener`.
struct Scoped(Opener);

impl Widget for Scoped {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        ctx.with_state(Marker(3), |ctx| self.0.to_element(ctx))
    }
}

impl aimer_widget::PortableWidget for Scoped {}

#[test]
fn new_window_inherits_only_when_asked() {
    for (inherit, expected) in [(true, Some(3)), (false, None)] {
        drain();
        let mut app = AimerApp::start_headless(Scoped(Opener { inherit }));
        app.pump_frames(4);

        let open = opens().remove(0);

        assert_eq!(open.inherited.as_ref().and_then(marker_in), expected);
    }
}
