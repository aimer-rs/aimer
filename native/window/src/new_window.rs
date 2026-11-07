use std::cell::RefCell;
use std::rc::Rc;

use aimer_widget::base::BuildContext;
use aimer_widget::components::context::InheritedStates;
use aimer_widget::Widget;

use crate::content::{ContentFn, WindowContent};
use crate::request::{self, OpenRequest, Request};
use crate::{WindowAttr, WindowId};

/// The state of a window builder that has no content yet.
pub struct RequiredBuilder;

/// The state of a window builder that has content, and can be opened.
pub struct ContentBuilder(pub(crate) Rc<ContentFn>);

impl ContentBuilder {
    /// Erases the widget a closure builds, so the window can hold any content.
    pub(crate) fn new<W: Widget + 'static>(build: impl Fn(&BuildContext) -> W + 'static) -> Self {
        Self(Rc::new(move |ctx: &BuildContext| build(ctx).boxed()))
    }
}

/// Opens a window from anywhere on the UI thread.
///
/// The window appears the next time the event loop runs, so [`open`](Self::open)
/// returns at once with the [`WindowId`] that names it. Its content is built by
/// a closure, which is how the window can be built again later; the content is
/// the root of a tree of its own, and nothing from the window that opened it is
/// inside it unless it is [inherited](Self::inherit).
///
/// Use the [`Window`](crate::Window) widget instead when the window should come
/// and go with a widget, or with a piece of state.
///
/// # Examples
///
/// ```no_run
/// use aimer_container::SizedBox;
/// use aimer_window::NewWindow;
///
/// let id = NewWindow::new()
///     .title("About")
///     .size(320, 240)
///     .on_close(|| println!("About was closed"))
///     .builder(|_ctx| SizedBox::new())
///     .open();
///
/// assert!(!id.is_open(), "it exists once the event loop has created it");
/// ```
pub struct NewWindow<B = RequiredBuilder> {
    attr: WindowAttr,
    inherited: Option<Rc<RefCell<InheritedStates>>>,
    on_closed: Option<Box<dyn FnOnce()>>,
    builder: B,
}

impl NewWindow {
    /// Creates a builder for a window with Aimer's standard appearance.
    ///
    /// Finish it with [`NewWindow::builder`].
    #[inline]
    pub fn new() -> Self {
        Self {
            attr: WindowAttr::new(),
            inherited: None,
            on_closed: None,
            builder: RequiredBuilder,
        }
    }

    /// Sets every property of the window at once.
    #[inline]
    pub fn attr(mut self, attr: WindowAttr) -> Self {
        self.attr = attr;
        self
    }

    /// Sets the text shown in the window's title bar.
    #[inline]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.attr = self.attr.title(title);
        self
    }

    /// Sets the initial logical width and height of the window's content.
    #[inline]
    pub fn size(mut self, width: u32, height: u32) -> Self {
        self.attr = self.attr.inner_size(width, height);
        self
    }

    /// Lets the window see what `ctx` sees: the providers above the widget
    /// that is building.
    ///
    /// A provider is shared, not copied, so the window reads and updates the
    /// same state as the window that opened it. Without this the window starts
    /// with no providers at all.
    #[inline]
    pub fn inherit(mut self, ctx: &BuildContext) -> Self {
        self.inherited = Some(inherited_from(ctx));
        self
    }

    /// Calls `callback` once the window has closed, however it closed: by the
    /// user, by [`WindowId::close`], or because the application is ending.
    ///
    /// It runs on the UI thread, after the window is gone.
    #[inline]
    pub fn on_close(mut self, callback: impl FnOnce() + 'static) -> Self {
        self.on_closed = Some(Box::new(callback));
        self
    }

    /// Sets what the window shows and completes this builder.
    ///
    /// `build` is called to build the window's content, and again whenever the
    /// content has to be rebuilt from the top.
    #[inline]
    pub fn builder<W: Widget + 'static>(
        self,
        build: impl Fn(&BuildContext) -> W + 'static,
    ) -> NewWindow<ContentBuilder> {
        NewWindow {
            attr: self.attr,
            inherited: self.inherited,
            on_closed: self.on_closed,
            builder: ContentBuilder::new(build),
        }
    }
}

impl Default for NewWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl NewWindow<ContentBuilder> {
    /// Asks the event loop for the window and returns the id that names it.
    pub fn open(self) -> WindowId {
        let id = request::next_id();
        let content = WindowContent::new(self.builder.0);
        request::push(Request::Open(OpenRequest {
            id,
            attr: self.attr,
            root: content.root(),
            inherited: self.inherited,
            on_closed: self.on_closed,
        }));
        id
    }
}

/// The providers in scope where `ctx` is building, as a map a window can start
/// from.
pub(crate) fn inherited_from(ctx: &BuildContext) -> Rc<RefCell<InheritedStates>> {
    Rc::new(RefCell::new(ctx.inherited_states.borrow().clone()))
}

#[cfg(test)]
mod tests {
    use aimer_container::SizedBox;

    use super::*;

    fn opened() -> Vec<OpenRequest> {
        request::drain()
            .into_iter()
            .map(|request| match request {
                Request::Open(open) => open,
                Request::Command(..) => panic!("unexpected command"),
            })
            .collect()
    }

    #[test]
    fn opening_queues_one_request_with_the_properties_given() {
        request::drain();

        let id = NewWindow::new()
            .title("Settings")
            .size(480, 320)
            .builder(|_ctx| SizedBox::new())
            .open();

        let requests = opened();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].id, id);
        assert_eq!(requests[0].attr, WindowAttr::new().title("Settings").inner_size(480, 320));
        assert!(requests[0].inherited.is_none(), "nothing is inherited unless asked for");
        assert!(requests[0].on_closed.is_none());
    }

    #[test]
    fn every_window_asked_for_gets_an_id_of_its_own() {
        request::drain();

        let first = NewWindow::new().builder(|_ctx| SizedBox::new()).open();
        let second = NewWindow::new().builder(|_ctx| SizedBox::new()).open();

        assert_ne!(first, second);
        let requests = opened();
        assert_eq!(
            requests.iter().map(|open| open.id).collect::<Vec<_>>(),
            [first, second],
            "requests keep the order they were made in"
        );
    }

    #[test]
    fn a_close_callback_travels_with_the_request() {
        request::drain();
        let called = Rc::new(std::cell::Cell::new(false));
        let flag = called.clone();

        NewWindow::new()
            .on_close(move || flag.set(true))
            .builder(|_ctx| SizedBox::new())
            .open();

        let mut requests = opened();
        let callback = requests.remove(0).on_closed.expect("the callback was kept");
        assert!(!called.get(), "it runs when the window closes, not before");
        callback();
        assert!(called.get());
    }
}
