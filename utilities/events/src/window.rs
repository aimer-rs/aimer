use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use winit::window::Window;

/// The first window of the application that is still alive.
///
/// Held weakly: a window that closes must be freed by its last owner, not kept
/// alive by this.
static GLOBAL_WINDOW: Mutex<Weak<Window>> = Mutex::new(Weak::new());

/// Whether a frame has been requested since the last display-link tick.
///
/// On iOS the frame loop is driven by a Swift `CADisplayLink` (see
/// `main.swift`). Requesting a frame sets this flag and unpauses the link;
/// each vsync tick consumes it via [`take_frame_requested`]. When a tick finds
/// it cleared, the link is paused again so the app does not render while idle.
static FRAME_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Atomically read-and-clear the pending-frame flag.
///
/// Returns `true` if a frame had been requested since the last call. Used by
/// the iOS display-link tick to decide whether to render this vsync or pause
/// the link.
pub fn take_frame_requested() -> bool {
    FRAME_REQUESTED.swap(false, Ordering::AcqRel)
}

#[cfg(target_os = "ios")]
unsafe extern "C" {
    /// Unpause the Swift `CADisplayLink` so it starts delivering vsync ticks.
    fn aimer_ios_request_frame();
}

type RedrawRequester = Box<dyn Fn() + Send + Sync + 'static>;

aimer_tls::thread_local! {
    /// A redraw requester that answers for this thread alone.
    ///
    /// The platform requester and the global window are one per process, which
    /// is exactly right for an application that owns the screen. An application
    /// running without a window is not that: several of them can be alive at
    /// once, each with its own frames to schedule, and each on its own thread.
    /// Installing per thread keeps a frame request with the application that
    /// made it.
    static THREAD_REDRAW_REQUESTER: RefCell<Option<Rc<dyn Fn()>>> =
        const { RefCell::new(None) };
    static THREAD_REDRAW_OBSERVER: RefCell<Option<Rc<dyn Fn()>>> =
        const { RefCell::new(None) };
    /// Set while a frame request that scrolling alone caused is being made.
    static SCROLL_FRAME_REQUEST: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Set while a frame request that a compositor animation alone caused is
    /// being made.
    static SCOPED_FRAME_REQUEST: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Requests the next frame on behalf of work that can be revisited alone.
///
/// Identical to [`request_animation_frame`], except that the request says why
/// it was made. A compositor animation changes only a retained node's
/// transform or opacity, and a state update rebuilds only the subtree of one
/// stateful element. A frame that exists for nothing but such work has no need
/// to visit the rest of the tree, and a frame loop that can tell this request
/// apart from a widget's own may do less work for it. Anything else that wants
/// a frame asks with [`request_animation_frame`] and keeps the frame a full one.
#[doc(hidden)]
pub fn request_scoped_frame() {
    let previous = SCOPED_FRAME_REQUEST.with(|flag| flag.replace(true));
    request_animation_frame();
    SCOPED_FRAME_REQUEST.with(|flag| flag.set(previous));
}

/// Whether the frame request being made right now was made by
/// [`request_scoped_frame`]. Meant for the requester that
/// records the reason.
#[doc(hidden)]
pub fn is_scoped_frame_request() -> bool {
    SCOPED_FRAME_REQUEST.with(std::cell::Cell::get)
}

/// Requests the next frame on behalf of scrolling.
///
/// Identical to [`request_animation_frame`], except that the request says why
/// it was made. A frame that exists only because scroll physics has more
/// distance to cover has nothing to update except what the offset moves, and a
/// frame loop that can tell it apart from a widget's request may do less work
/// for it. Anything else that wants a frame asks with
/// [`request_animation_frame`] and keeps the frame a full one.
#[doc(hidden)]
pub fn request_scroll_frame() {
    let previous = SCROLL_FRAME_REQUEST.with(|flag| flag.replace(true));
    request_animation_frame();
    SCROLL_FRAME_REQUEST.with(|flag| flag.set(previous));
}

/// Whether the frame request being made right now was made by
/// [`request_scroll_frame`]. Meant for the requester that records the reason.
#[doc(hidden)]
pub fn is_scroll_frame_request() -> bool {
    SCROLL_FRAME_REQUEST.with(std::cell::Cell::get)
}

/// Installs an internal observer for direct redraw requests on this thread.
#[doc(hidden)]
pub fn set_thread_redraw_observer<F>(observer: F) -> Option<Rc<dyn Fn()>>
where
    F: Fn() + 'static,
{
    THREAD_REDRAW_OBSERVER.with(|slot| slot.borrow_mut().replace(Rc::new(observer)))
}

/// Restores the observer returned by [`set_thread_redraw_observer`].
#[doc(hidden)]
pub fn restore_thread_redraw_observer(previous: Option<Rc<dyn Fn()>>) {
    THREAD_REDRAW_OBSERVER.with(|slot| *slot.borrow_mut() = previous);
}

/// Notifies the active application's frame scheduler that a window requested a
/// redraw directly, without going through [`request_animation_frame`].
#[doc(hidden)]
pub fn notify_redraw_requested() {
    let observer = THREAD_REDRAW_OBSERVER.with(|slot| slot.borrow().clone());
    if let Some(observer) = observer {
        observer();
    }
}

/// Install a redraw requester for the current thread, replacing any previous
/// one, and hand back the one it replaced.
///
/// Takes precedence over the process-wide requester and the global window, so
/// an application that has no platform window still receives the frame requests
/// its widgets make — a state update, an animation step, an overlay that just
/// opened. Return the previous requester to
/// [`restore_thread_redraw_requester`] once the application is gone, so a
/// dropped application stops receiving them.
///
/// # Examples
///
/// ```
/// use std::cell::Cell;
/// use std::rc::Rc;
///
/// let requests = Rc::new(Cell::new(0));
/// let counted = requests.clone();
/// let previous = aimer_events::window::set_thread_redraw_requester(move || {
///     counted.set(counted.get() + 1);
/// });
///
/// aimer_events::window::request_animation_frame();
/// assert_eq!(requests.get(), 1);
///
/// aimer_events::window::restore_thread_redraw_requester(previous);
/// ```
pub fn set_thread_redraw_requester<F>(requester: F) -> Option<Rc<dyn Fn()>>
where
    F: Fn() + 'static,
{
    THREAD_REDRAW_REQUESTER.with(|slot| slot.borrow_mut().replace(Rc::new(requester)))
}

/// Put back the requester that [`set_thread_redraw_requester`] replaced.
pub fn restore_thread_redraw_requester(previous: Option<Rc<dyn Fn()>>) {
    THREAD_REDRAW_REQUESTER.with(|slot| *slot.borrow_mut() = previous);
}

/// The requester installed for this thread, if any.
#[cfg(not(aimer_portable_guest))]
fn thread_redraw_requester() -> Option<Rc<dyn Fn()>> {
    THREAD_REDRAW_REQUESTER.with(|slot| slot.borrow().clone())
}

/// Names one window to the frame scheduling of the thread that owns it.
///
/// Every window on a UI thread schedules its own frames, but the functions a
/// widget calls to ask for one — [`request_animation_frame`] and its siblings —
/// take no window. They answer for whichever window is *active* on the thread.
/// A key is how a widget that outlives that moment (a state updater, called
/// later by a timer) says which window it belongs to.
///
/// Two windows alive at the same time must not share a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WindowKey(usize);

impl WindowKey {
    /// Builds a key from an address that is unique to the window for as long
    /// as the window lives: the platform window, or the state its handle shares.
    #[inline]
    pub const fn from_address(address: usize) -> Self {
        Self(address)
    }

    /// The address this key was made from, for carrying it through an API that
    /// only holds a number.
    #[inline]
    pub const fn address(self) -> usize {
        self.0
    }
}

/// Thread-local state that belongs to one window and has to follow it.
///
/// A window keeps this state with itself while another window's is on the
/// thread. The state of the window that was active last stays on the thread
/// after its scope ends, so code that runs outside any window — a platform
/// callback, a test — talks to the window that was most recently in use, as it
/// always did when there was only one. It is taken off when a different window
/// becomes active or when the window is unregistered.
pub trait WindowLocals {
    /// Makes the window's state the thread's.
    fn install(&self);

    /// Takes the window's state back off the thread. Always follows an
    /// [`install`](Self::install), and precedes the next window's.
    fn uninstall(&self);
}

/// How a window asks for frames and what it brings onto the thread.
pub struct WindowScopeConfig {
    requester: Rc<dyn Fn()>,
    observer: Rc<dyn Fn()>,
    native: Option<Arc<Window>>,
    locals: Option<Rc<dyn WindowLocals>>,
}

impl WindowScopeConfig {
    /// `requester` schedules the window's next frame; `observer` records that
    /// a redraw was asked for (see [`notify_redraw_requested`]).
    pub fn new(requester: impl Fn() + 'static, observer: impl Fn() + 'static) -> Self {
        Self {
            requester: Rc::new(requester),
            observer: Rc::new(observer),
            native: None,
            locals: None,
        }
    }

    /// The platform window, which [`get_window`] answers with while this
    /// window is the active one.
    pub fn native(mut self, window: Arc<Window>) -> Self {
        self.native = Some(window);
        self
    }

    /// State to put on the thread while this window is active.
    pub fn locals(mut self, locals: Rc<dyn WindowLocals>) -> Self {
        self.locals = Some(locals);
        self
    }
}

/// What a window leaves on the thread while it is active.
struct WindowScope {
    key: WindowKey,
    config: WindowScopeConfig,
}

aimer_tls::thread_local! {
    /// Every window of this thread that can be made active, by key.
    ///
    /// A thread has a handful of windows, so a vector scanned in order is
    /// cheaper than hashing.
    static WINDOW_SCOPES: RefCell<Vec<WindowScope>> = const { RefCell::new(Vec::new()) };
    /// The window whose scope is installed in the thread's requester and
    /// observer slots right now, if it was installed through a scope.
    static ACTIVE_WINDOW: std::cell::Cell<Option<WindowKey>> = const { std::cell::Cell::new(None) };
    /// The platform window of the active scope, if it has one.
    static ACTIVE_NATIVE: RefCell<Option<Arc<Window>>> = const { RefCell::new(None) };
    /// The window whose [`WindowLocals`] are on the thread.
    ///
    /// Not the same as [`ACTIVE_WINDOW`]: this one is still set between scopes.
    static INSTALLED: RefCell<Option<(WindowKey, Rc<dyn WindowLocals>)>> = const { RefCell::new(None) };
}

/// Puts `key`'s state on the thread, taking whichever window's was there off.
///
/// Does nothing when it is already there, which is what makes entering the same
/// window again cheap.
fn install_locals_for(key: WindowKey, locals: Option<Rc<dyn WindowLocals>>) {
    let displaced = INSTALLED.with(|installed| {
        let mut installed = installed.borrow_mut();
        if installed.as_ref().is_some_and(|(held, _)| *held == key) {
            return None;
        }
        Some(installed.take())
    });
    let Some(displaced) = displaced else {
        return;
    };
    // Outside the borrow: both calls run the owner's code.
    if let Some((_, previous)) = displaced {
        previous.uninstall();
    }
    if let Some(locals) = locals {
        locals.install();
        INSTALLED.with(|installed| *installed.borrow_mut() = Some((key, locals)));
    }
}

/// Keeps a window registered with [`register_window_scope`]; unregisters it
/// when dropped.
#[must_use = "the window is unregistered as soon as this is dropped"]
pub struct WindowScopeRegistration {
    key: WindowKey,
}

impl Drop for WindowScopeRegistration {
    fn drop(&mut self) {
        // A window that is gone must not leave its state on the thread.
        let held = INSTALLED.with(|installed| {
            let mut installed = installed.borrow_mut();
            if installed.as_ref().is_some_and(|(held, _)| *held == self.key) {
                installed.take()
            } else {
                None
            }
        });
        if let Some((_, locals)) = held {
            locals.uninstall();
        }
        WINDOW_SCOPES.with(|scopes| scopes.borrow_mut().retain(|scope| scope.key != self.key));
    }
}

/// Registers how the window `key` asks for frames, so [`enter_window_scope`]
/// can make it the active one.
///
/// `requester` schedules the window's next frame, and `observer` records that a
/// redraw was asked for (see [`notify_redraw_requested`]). Registering a key
/// that is already registered replaces it.
pub fn register_window_scope(
    key: WindowKey,
    requester: impl Fn() + 'static,
    observer: impl Fn() + 'static,
) -> WindowScopeRegistration {
    register_window_scope_with(key, WindowScopeConfig::new(requester, observer))
}

/// Like [`register_window_scope`], for a window that also has a platform window
/// or state of its own to bring onto the thread.
pub fn register_window_scope_with(
    key: WindowKey,
    config: WindowScopeConfig,
) -> WindowScopeRegistration {
    let scope = WindowScope { key, config };
    WINDOW_SCOPES.with(|scopes| {
        let mut scopes = scopes.borrow_mut();
        match scopes.iter_mut().find(|existing| existing.key == key) {
            Some(existing) => *existing = scope,
            None => scopes.push(scope),
        }
    });
    WindowScopeRegistration { key }
}

/// What entering a window displaced, to be put back.
struct Displaced {
    requester: Option<Rc<dyn Fn()>>,
    observer: Option<Rc<dyn Fn()>>,
    key: Option<WindowKey>,
    native: Option<Arc<Window>>,
}

/// Puts a window back as the thread's active one when dropped. See
/// [`enter_window_scope`].
#[must_use = "the previous window becomes active again as soon as this is dropped"]
pub struct WindowScopeGuard {
    /// What to put back; `None` when entering changed nothing.
    previous: Option<Displaced>,
}

impl WindowScopeGuard {
    /// Whether entering made a different window the active one, as opposed to
    /// finding the right one already active (or not knowing the window at all).
    #[inline]
    pub fn switched(&self) -> bool {
        self.previous.is_some()
    }
}

impl Drop for WindowScopeGuard {
    fn drop(&mut self) {
        if let Some(displaced) = self.previous.take() {
            THREAD_REDRAW_REQUESTER.with(|slot| *slot.borrow_mut() = displaced.requester);
            THREAD_REDRAW_OBSERVER.with(|slot| *slot.borrow_mut() = displaced.observer);
            ACTIVE_WINDOW.with(|active| active.set(displaced.key));
            ACTIVE_NATIVE.with(|native| *native.borrow_mut() = displaced.native);
            // A window that was running when this one was entered is running
            // again, and expects its own state. When none was, this window's
            // stays on the thread.
            if let Some(resumed) = displaced.key {
                let locals = WINDOW_SCOPES.with(|scopes| {
                    scopes
                        .borrow()
                        .iter()
                        .find(|scope| scope.key == resumed)
                        .and_then(|scope| scope.config.locals.clone())
                });
                install_locals_for(resumed, locals);
            }
        }
    }
}

/// Makes the window `key` the one the thread's frame requests answer for, until
/// the guard is dropped.
///
/// Entering the window that is already active, or one that was never
/// registered, changes nothing and costs a comparison. Guards nest: dropping
/// one restores whatever was active when it was made.
#[inline]
pub fn enter_window_scope(key: WindowKey) -> WindowScopeGuard {
    if ACTIVE_WINDOW.with(std::cell::Cell::get) == Some(key) {
        return WindowScopeGuard { previous: None };
    }
    enter_registered_scope(key)
}

fn enter_registered_scope(key: WindowKey) -> WindowScopeGuard {
    let found = WINDOW_SCOPES.with(|scopes| {
        scopes
            .borrow()
            .iter()
            .find(|scope| scope.key == key)
            .map(|scope| {
                (
                    scope.config.requester.clone(),
                    scope.config.observer.clone(),
                    scope.config.native.clone(),
                    scope.config.locals.clone(),
                )
            })
    });
    let Some((requester, observer, native, locals)) = found else {
        return WindowScopeGuard { previous: None };
    };
    let displaced = Displaced {
        requester: THREAD_REDRAW_REQUESTER.with(|slot| slot.borrow_mut().replace(requester)),
        observer: THREAD_REDRAW_OBSERVER.with(|slot| slot.borrow_mut().replace(observer)),
        key: ACTIVE_WINDOW.with(|active| active.replace(Some(key))),
        native: ACTIVE_NATIVE.with(|slot| slot.replace(native)),
    };
    install_locals_for(key, locals);
    WindowScopeGuard {
        previous: Some(displaced),
    }
}

/// The window that is active on this thread right now, if one was entered
/// through [`enter_window_scope`].
///
/// Code that has no window of its own to ask — a function that reads "the
/// window's" state without being handed a handle — answers for this one.
#[inline]
pub fn active_window() -> Option<WindowKey> {
    ACTIVE_WINDOW.with(std::cell::Cell::get)
}

/// Runs `f` with the window `key` active. See [`enter_window_scope`].
#[inline]
pub fn with_window_scope<R>(key: WindowKey, f: impl FnOnce() -> R) -> R {
    let _active = enter_window_scope(key);
    f()
}

/// Optional, platform-supplied redraw requester. When installed it is used to
/// schedule the next frame through the event loop (e.g. an `EventLoopProxy`)
/// instead of calling `Window::request_redraw()` directly.
static REDRAW_REQUESTER: OnceLock<RedrawRequester> = OnceLock::new();

/// Store the application window so other crates can access it.
///
/// The first window wins for as long as it is alive; once it is gone the next
/// one registered takes its place. Only a weak reference is kept.
pub fn set_window(window: &Arc<Window>) {
    let mut global = GLOBAL_WINDOW.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if global.strong_count() == 0 {
        *global = Arc::downgrade(window);
    }
}

/// Retrieve the application window reference, if it has been set.
///
/// With several windows this answers for the active one (see
/// [`enter_window_scope`]) when it has a platform window, and for the first
/// window registered with [`set_window`] otherwise.
pub fn get_window() -> Option<Arc<Window>> {
    ACTIVE_NATIVE
        .with(|native| native.borrow().clone())
        .or_else(global_window)
}

fn global_window() -> Option<Arc<Window>> {
    GLOBAL_WINDOW
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .upgrade()
}

/// Install a platform redraw requester.
///
/// On some platforms (notably iOS) calling `Window::request_redraw()`
/// synchronously from inside the draw cycle is coalesced and does not schedule
/// the next frame. Routing the request through the event loop (via an
/// `EventLoopProxy`) delivers it after the current frame completes without
/// spawning a thread. The application installs that closure here.
pub fn set_redraw_requester<F>(requester: F)
where
    F: Fn() + Send + Sync + 'static,
{
    let _ = REDRAW_REQUESTER.set(Box::new(requester));
}

/// Request the next frame render.
///
/// Prefers the installed event-loop-driven requester (safe to call from within
/// the draw cycle); falls back to `Window::request_redraw()` when none was
/// installed.
///
/// On iOS, `request_redraw()` issued from inside the draw cycle (or from a
/// `user_event` that arrives immediately after) is silently coalesced by the
/// system — the next `RedrawRequested` is never delivered and animations stop
/// after a single step.
///
/// Instead, the frame loop is driven by a Swift `CADisplayLink` synced to the
/// display (up to 120 Hz on ProMotion). Requesting a frame simply raises the
/// [`FRAME_REQUESTED`] flag and unpauses the link; the next vsync tick then
/// delivers a `FrameReady` (see `aimer_ios_frame_tick` in `aimer_quiver`) that
/// renders directly on iOS, avoiding a second UIKit draw callback. The link pauses
/// itself once a tick observes no pending request, so the app stays idle when
/// nothing is animating.
#[cfg(aimer_portable_guest)]
pub fn request_animation_frame() {
    // Portable guests do not own a browser or native event loop. The reload
    // host polls the guest scheduler at its safe point, so forwarding this
    // request into winit would pull the browser event-loop ABI into the
    // capability-only module.
}

#[cfg(not(aimer_portable_guest))]
pub fn request_animation_frame() {
    // An application without a platform window schedules its own frames, and
    // says so per thread, so its request never reaches the display link or a
    // window belonging to somebody else.
    if let Some(requester) = thread_redraw_requester() {
        requester();
        return;
    }

    #[cfg(target_os = "ios")]
    {
        // Mark a frame as pending and make sure the display link is running so
        // the next vsync delivers it.
        FRAME_REQUESTED.store(true, Ordering::Release);
        unsafe {
            aimer_ios_request_frame();
        }
    }
    #[cfg(not(target_os = "ios"))]
    {
        request_animation_frame_inner();
    }
}

/// Inner implementation — actual redraw request (platform-independent).
#[cfg_attr(target_os = "ios", allow(dead_code))]
#[cfg(not(aimer_portable_guest))]
fn request_animation_frame_inner() {
    if let Some(requester) = REDRAW_REQUESTER.get() {
        requester();
        return;
    }
    if let Some(window) = global_window() {
        window.request_redraw();
    }
}

#[cfg(all(test, not(aimer_portable_guest)))]
mod scroll_request_tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;

    /// Runs `request` with a requester that records whether each request it
    /// received was a scroll request.
    fn observed(request: impl FnOnce()) -> Vec<bool> {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let previous = set_thread_redraw_requester({
            let seen = seen.clone();
            move || seen.borrow_mut().push(is_scroll_frame_request())
        });
        request();
        restore_thread_redraw_requester(previous);
        seen.take()
    }

    /// Like [`observed`], for the compositor-animation marker.
    fn observed_compositor(request: impl FnOnce()) -> Vec<bool> {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let previous = set_thread_redraw_requester({
            let seen = seen.clone();
            move || seen.borrow_mut().push(is_scoped_frame_request())
        });
        request();
        restore_thread_redraw_requester(previous);
        seen.take()
    }

    #[test]
    fn a_compositor_request_says_it_is_one_and_the_mark_does_not_leak() {
        assert_eq!(observed_compositor(request_scoped_frame), [true]);
        assert_eq!(observed_compositor(request_animation_frame), [false]);
        assert!(!is_scoped_frame_request());
    }

    #[test]
    fn a_scroll_request_says_it_is_one_and_an_ordinary_request_does_not() {
        assert_eq!(observed(request_scroll_frame), [true]);
        assert_eq!(observed(request_animation_frame), [false]);
        assert!(!is_scroll_frame_request());
    }

    #[test]
    fn the_mark_does_not_outlive_the_request() {
        let seen = observed(|| {
            request_scroll_frame();
            request_animation_frame();
            request_scroll_frame();
        });
        assert_eq!(seen, [true, false, true]);
        assert!(!is_scroll_frame_request());
    }

    #[test]
    fn nested_scroll_requests_restore_the_outer_mark() {
        let seen = observed(|| {
            let previous = SCROLL_FRAME_REQUEST.with(|flag| flag.replace(true));
            request_scroll_frame();
            assert!(is_scroll_frame_request());
            SCROLL_FRAME_REQUEST.with(|flag| flag.set(previous));
        });
        assert_eq!(seen, [true]);
        assert!(!is_scroll_frame_request());
    }
}

#[cfg(all(test, not(aimer_portable_guest)))]
mod window_scope_tests {
    use std::cell::Cell;

    use super::*;

    /// A window that counts the frames requested of it and the redraws it
    /// observed.
    struct Counted {
        requests: Rc<Cell<usize>>,
        observed: Rc<Cell<usize>>,
        _registration: WindowScopeRegistration,
    }

    fn window(address: usize) -> (WindowKey, Counted) {
        let key = WindowKey::from_address(address);
        let requests = Rc::new(Cell::new(0));
        let observed = Rc::new(Cell::new(0));
        let registration = register_window_scope(
            key,
            {
                let requests = requests.clone();
                move || requests.set(requests.get() + 1)
            },
            {
                let observed = observed.clone();
                move || observed.set(observed.get() + 1)
            },
        );
        (key, Counted { requests, observed, _registration: registration })
    }

    #[test]
    fn a_request_made_inside_a_window_reaches_that_window_only() {
        let (a, first) = window(0xA);
        let (_b, second) = window(0xB);

        with_window_scope(a, request_animation_frame);

        assert_eq!((first.requests.get(), second.requests.get()), (1, 0));
    }

    #[test]
    fn the_observer_follows_the_window() {
        let (a, first) = window(0xA);
        let (b, second) = window(0xB);

        with_window_scope(a, notify_redraw_requested);
        with_window_scope(b, notify_redraw_requested);
        with_window_scope(b, notify_redraw_requested);

        assert_eq!((first.observed.get(), second.observed.get()), (1, 2));
    }

    #[test]
    fn leaving_a_window_puts_back_the_one_that_was_active() {
        let (a, first) = window(0xA);
        let (b, second) = window(0xB);

        let outer = enter_window_scope(a);
        with_window_scope(b, request_animation_frame);
        request_animation_frame();
        drop(outer);

        assert_eq!((first.requests.get(), second.requests.get()), (1, 1));
        assert_eq!(ACTIVE_WINDOW.with(Cell::get), None);
    }

    #[test]
    fn entering_the_active_window_again_changes_nothing() {
        let (a, first) = window(0xA);

        let _outer = enter_window_scope(a);
        let inner = enter_window_scope(a);
        assert!(inner.previous.is_none());
        drop(inner);
        request_animation_frame();

        assert_eq!(first.requests.get(), 1, "the inner guard deactivated the window");
    }

    #[test]
    fn an_unregistered_window_leaves_the_active_one_alone() {
        let (a, first) = window(0xA);

        let _outer = enter_window_scope(a);
        with_window_scope(WindowKey::from_address(0xDEAD), request_animation_frame);

        assert_eq!(first.requests.get(), 1);
    }

    #[test]
    fn a_dropped_window_can_no_longer_be_entered() {
        let (a, first) = window(0xA);
        let requests = first.requests.clone();
        drop(first);

        with_window_scope(a, request_animation_frame);

        assert_eq!(requests.get(), 0);
    }

    /// Records the order its window's state was put on and taken off the thread.
    struct Recorded {
        log: Rc<RefCell<Vec<&'static str>>>,
        name: &'static str,
    }

    impl WindowLocals for Recorded {
        fn install(&self) {
            self.log.borrow_mut().push(self.name);
            self.log.borrow_mut().push("install");
        }

        fn uninstall(&self) {
            self.log.borrow_mut().push(self.name);
            self.log.borrow_mut().push("uninstall");
        }
    }

    fn window_with_locals(
        address: usize,
        name: &'static str,
        log: &Rc<RefCell<Vec<&'static str>>>,
    ) -> (WindowKey, WindowScopeRegistration) {
        let key = WindowKey::from_address(address);
        let registration = register_window_scope_with(
            key,
            WindowScopeConfig::new(|| {}, || {}).locals(Rc::new(Recorded {
                log: log.clone(),
                name,
            })),
        );
        (key, registration)
    }

    #[test]
    fn a_windows_state_stays_on_the_thread_after_its_scope_ends() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let (a, _a) = window_with_locals(0xA, "a", &log);

        with_window_scope(a, || log.borrow_mut().push("inside"));

        // Code outside any window still talks to the window used last.
        assert_eq!(*log.borrow(), ["a", "install", "inside"]);
    }

    #[test]
    fn entering_another_window_takes_the_last_ones_state_off_first() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let (a, _a) = window_with_locals(0xA, "a", &log);
        let (b, _b) = window_with_locals(0xB, "b", &log);

        with_window_scope(a, || {});
        with_window_scope(b, || {});

        assert_eq!(
            *log.borrow(),
            ["a", "install", "a", "uninstall", "b", "install"]
        );
    }

    #[test]
    fn a_window_that_was_running_gets_its_state_back_when_a_nested_one_ends() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let (a, _a) = window_with_locals(0xA, "a", &log);
        let (b, _b) = window_with_locals(0xB, "b", &log);

        with_window_scope(a, || with_window_scope(b, || {}));

        assert_eq!(
            *log.borrow(),
            [
                "a", "install", "a", "uninstall", "b", "install", "b", "uninstall", "a",
                "install"
            ]
        );
    }

    #[test]
    fn entering_the_window_whose_state_is_on_the_thread_installs_nothing() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let (a, _a) = window_with_locals(0xA, "a", &log);

        with_window_scope(a, || {});
        with_window_scope(a, || {});
        with_window_scope(a, || with_window_scope(a, || {}));

        assert_eq!(*log.borrow(), ["a", "install"]);
    }

    #[test]
    fn a_window_that_is_unregistered_takes_its_state_off_the_thread() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let (a, registration) = window_with_locals(0xA, "a", &log);

        with_window_scope(a, || {});
        drop(registration);

        assert_eq!(*log.borrow(), ["a", "install", "a", "uninstall"]);
    }

    #[test]
    fn with_no_window_active_there_is_no_active_window() {
        assert_eq!(active_window(), None);
        let (a, _first) = window(0xA);

        with_window_scope(a, || assert_eq!(active_window(), Some(a)));

        assert_eq!(active_window(), None);
    }

    /// Cost of entering a window scope, for `cargo nextest run --release
    /// --run-ignored only window_scope_cost`. Not an assertion: it prints what
    /// a state update pays for naming its window.
    #[test]
    #[ignore = "measurement, not a check"]
    fn window_scope_cost() {
        use std::hint::black_box;
        use aimer_time::Instant;

        const ROUNDS: u32 = 20_000_000;
        let time = |label: &str, mut body: Box<dyn FnMut()>| {
            for _ in 0..ROUNDS / 10 {
                body();
            }
            let start = Instant::now();
            for _ in 0..ROUNDS {
                body();
            }
            let per = start.elapsed().as_secs_f64() * 1e9 / f64::from(ROUNDS);
            eprintln!("{label:<58} {per:6.2} ns");
        };
        let count = Rc::new(Cell::new(0u64));
        let sink = count.clone();
        let previous = set_thread_redraw_requester(move || sink.set(sink.get() + 1));

        time("request_animation_frame alone", Box::new(|| request_animation_frame()));
        let nowhere = WindowKey::from_address(0xDEAD);
        time(
            "with_window_scope, no window registered (native, one window)",
            Box::new(move || with_window_scope(black_box(nowhere), request_animation_frame)),
        );
        let (a, _first) = window(0xA);
        let (b, _second) = window(0xB);
        let _entered = enter_window_scope(a);
        time(
            "with_window_scope, window already active (inside an event)",
            Box::new(move || with_window_scope(black_box(a), request_animation_frame)),
        );
        time(
            "with_window_scope, switching to the other window",
            Box::new(move || with_window_scope(black_box(b), request_animation_frame)),
        );
        restore_thread_redraw_requester(previous);
        assert!(count.get() > 0);
    }
}
