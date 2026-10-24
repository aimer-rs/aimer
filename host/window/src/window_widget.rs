use std::cell::{Cell, RefCell};
use std::rc::Rc;

use aimer_container::ZeroSizedBox;
use aimer_widget::base::BuildContext;
use aimer_widget::components::context::InheritedStates;
use aimer_widget::{
    AnyElement, PortableWidget, State, StateUpdater, StatefulElement, StatefulWidget, Widget,
};

use crate::content::WindowContent;
use crate::new_window::{ContentBuilder, RequiredBuilder, inherited_from};
use crate::request::{self, OpenRequest, Request};
use aimer_venus::Venus;

use crate::{WindowAttr, WindowId};

/// A window that exists while this widget does.
///
/// Put it anywhere in a tree. It takes no room and paints nothing where it is
/// placed; what it declares is a window of its own, showing what its
/// [`builder`](Window::builder) builds. While the widget is mounted and
/// [`open`](Window::open) is `true` the window is on screen; when either stops
/// being true the window closes. The state that decides whether it is shown
/// stays with the widget that holds it, as with any other widget:
///
/// - Rebuilding the parent with a new title or size updates the window. A size
///   is applied only when the property changes, so it does not undo the user
///   resizing the window in between.
/// - Rebuilding the parent with a new `builder` rebuilds the window's content.
/// - When the user closes the window, [`on_close`](Window::on_close) is called.
///   Set the state that feeds `open` to `false` there. A window the user closed
///   stays closed while `open` stays `true`; it opens again after `open` has
///   been `false`.
///
/// The window sees the providers that were in scope where the widget is, and
/// shares their state with the window it was declared in. Call
/// [`isolated`](Window::isolated) for a window that starts with none.
///
/// Use [`NewWindow`](crate::NewWindow) to open a window from a callback instead.
///
/// # Examples
///
/// ```no_run
/// use aimer_container::SizedBox;
/// use aimer_widget::{State, StateUpdater, StatefulWidget, Widget, base::BuildContext};
/// use aimer_window::Window;
///
/// struct Page;
/// struct PageState {
///     updater: Option<StateUpdater<PageState>>,
///     settings_open: bool,
/// }
///
/// impl StatefulWidget for Page {
///     type State = PageState;
///     fn create_state(self) -> PageState {
///         PageState { updater: None, settings_open: true }
///     }
/// }
///
/// impl State<Page> for PageState {
///     fn init_state(&mut self, updater: StateUpdater<Self>) {
///         self.updater = Some(updater);
///     }
///
///     fn build(&self, _ctx: &BuildContext) -> impl Widget {
///         let updater = self.updater.expect("set before the first build");
///         Window::new()
///             .title("Settings")
///             .size(480, 320)
///             .open(self.settings_open)
///             .on_close(move || updater.set_state(|page| page.settings_open = false))
///             .builder(|_ctx| SizedBox::new())
///     }
/// }
/// ```
pub struct Window<B = RequiredBuilder> {
    attr: WindowAttr,
    open: bool,
    inherit: bool,
    on_close: Option<Rc<dyn Fn()>>,
    builder: B,
}

impl Window {
    /// Creates a window that is open, inherits the providers around it, and has
    /// Aimer's standard appearance.
    ///
    /// Finish it with [`Window::builder`].
    #[inline]
    pub fn new() -> Self {
        Self {
            attr: WindowAttr::new(),
            open: true,
            inherit: true,
            on_close: None,
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

    /// Sets the logical width and height of the window's content.
    #[inline]
    pub fn size(mut self, width: u32, height: u32) -> Self {
        self.attr = self.attr.inner_size(width, height);
        self
    }

    /// Sets whether the window is on screen. The default is `true`.
    #[inline]
    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    /// Makes the window start with no providers, instead of the ones in scope
    /// where this widget is.
    #[inline]
    pub fn isolated(mut self) -> Self {
        self.inherit = false;
        self
    }

    /// Calls `callback` when the window has closed without this widget asking
    /// it to: the user closed it, or something called [`WindowId::close`].
    #[inline]
    pub fn on_close(mut self, callback: impl Fn() + 'static) -> Self {
        self.on_close = Some(Rc::new(callback));
        self
    }

    /// Sets what the window shows and completes this widget.
    ///
    /// `build` is called to build the window's content, and again whenever the
    /// content has to be rebuilt from the top. A new closure from a rebuilt
    /// parent replaces it.
    #[inline]
    pub fn builder<W: Widget + 'static>(
        self,
        build: impl Fn(&BuildContext) -> W + 'static,
    ) -> Window<ContentBuilder> {
        Window {
            attr: self.attr,
            open: self.open,
            inherit: self.inherit,
            on_close: self.on_close,
            builder: ContentBuilder::new(build),
        }
    }
}

impl Default for Window {
    fn default() -> Self {
        Self::new()
    }
}

impl Widget for Window<ContentBuilder> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(WindowHost(self), ctx, "Window", None)
            .0
            .boxed()
    }
}

impl PortableWidget for Window<ContentBuilder> {}

/// The widget behind [`Window`]: what is mounted in the tree.
struct WindowHost(Window<ContentBuilder>);

/// Everything the window widget changes after it is built, shared with the
/// work that is deferred to after the build.
struct Inner {
    attr: RefCell<WindowAttr>,
    /// Whether the window should be on screen.
    open: Cell<bool>,
    /// Read when the window closes, so a callback replaced since the window
    /// opened is the one that runs.
    on_close: RefCell<Option<Rc<dyn Fn()>>>,
    content: Rc<WindowContent>,
    /// The window this widget asked for, while it asked for one.
    id: Cell<Option<WindowId>>,
    /// Whether the window was asked for during the current stretch of `open`.
    ///
    /// Stays set after the user closes the window, which is what keeps it
    /// closed until `open` has been `false` once.
    opened: Cell<bool>,
    /// Set once a newer state has taken this one's place.
    ///
    /// The framework builds the state of a widget that was emitted again before
    /// it reconciles it with the one already in the tree, and then drops the new
    /// one. That state must not open a window, so nothing acts on its behalf
    /// before it is known to be the live one.
    superseded: Cell<bool>,
}

struct WindowHostState {
    inner: Rc<Inner>,
    inherit: bool,
}

impl StatefulWidget for WindowHost {
    type State = WindowHostState;

    fn create_state(self) -> WindowHostState {
        let Window {
            attr,
            open,
            inherit,
            on_close,
            builder,
        } = self.0;
        WindowHostState {
            inner: Rc::new(Inner {
                attr: RefCell::new(attr),
                open: Cell::new(open),
                on_close: RefCell::new(on_close),
                content: WindowContent::new(builder.0),
                id: Cell::new(None),
                opened: Cell::new(false),
                superseded: Cell::new(false),
            }),
            inherit,
        }
    }
}

impl State<WindowHost> for WindowHostState {
    fn init_state(&mut self, _updater: StateUpdater<Self>) {}

    fn adopt_config_from(&mut self, new: Self) {
        new.inner.superseded.set(true);
        let inner = &self.inner;
        if let Some(id) = inner.id.get() {
            let next = new.inner.attr.borrow();
            let current = inner.attr.borrow();
            if next.title != current.title {
                id.set_title(next.title.clone());
            }
            if next.inner_size != current.inner_size {
                let (width, height) = next.inner_size;
                id.resize(width, height);
            }
        }
        *inner.attr.borrow_mut() = new.inner.attr.borrow().clone();
        inner.content.replace(new.inner.content.current());
        *inner.on_close.borrow_mut() = new.inner.on_close.borrow_mut().take();
        inner.open.set(new.inner.open.get());
        self.inherit = new.inherit;
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        if self.inner.needs_sync() {
            // Only what has to be acted on is captured, and it is acted on after
            // this build: see `Inner::superseded`.
            let inherited = (self.inherit && self.inner.open.get()).then(|| inherited_from(ctx));
            let inner = self.inner.clone();
            match Venus::current() {
                Some(venus) => {
                    venus.spawn(async move { inner.sync(inherited) });
                }
                None => inner.sync(inherited),
            }
        }
        ZeroSizedBox
    }
}

impl Inner {
    /// Whether the window is not what `open` says it should be.
    fn needs_sync(&self) -> bool {
        if self.open.get() {
            !self.opened.get()
        } else {
            self.id.get().is_some() || self.opened.get()
        }
    }

    /// Makes the window match `open`: asks for it once when it becomes true,
    /// and closes it when it stops.
    fn sync(self: &Rc<Self>, inherited: Option<Rc<RefCell<InheritedStates>>>) {
        if self.superseded.get() {
            return;
        }
        if self.open.get() {
            if !self.opened.replace(true) {
                self.id.set(Some(self.request_window(inherited)));
            }
        } else {
            self.opened.set(false);
            if let Some(id) = self.id.take() {
                id.close();
            }
        }
    }

    fn request_window(
        self: &Rc<Self>,
        inherited: Option<Rc<RefCell<InheritedStates>>>,
    ) -> WindowId {
        let id = request::next_id();
        let widget = Rc::downgrade(self);
        request::push(Request::Open(OpenRequest {
            id,
            attr: self.attr.borrow().clone(),
            root: self.content.root(),
            inherited,
            on_closed: Some(Box::new(move || {
                // Told only when the widget did not ask: one that asked has
                // let go of the id, and one that is gone has nobody to tell.
                let Some(widget) = widget.upgrade() else {
                    return;
                };
                if widget.id.get() != Some(id) {
                    return;
                }
                // Cloned out first: the callback may replace itself.
                let callback = widget.on_close.borrow().clone();
                if let Some(callback) = callback {
                    callback();
                }
            })),
        }));
        id
    }
}

impl Drop for WindowHostState {
    /// The widget left the tree: its window goes with it.
    fn drop(&mut self) {
        if let Some(id) = self.inner.id.take() {
            id.close();
        }
    }
}
