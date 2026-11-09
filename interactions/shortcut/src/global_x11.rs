//! Linux global shortcuts through X11 key grabs.
//!
//! `XGrabKey` on the root window makes the X server deliver a chord to this
//! application whichever window has focus. The connection is owned by a thread
//! of its own, which waits on the X socket and on a pipe that wakes it for
//! requests, so the UI thread never blocks on the server.
//!
//! `libX11` is loaded at run time with `dlopen`. A machine without it, or
//! without a display to connect to, reports [`GlobalShortcutError::Unsupported`]
//! rather than failing to start. Wayland offers no equivalent API: under
//! XWayland a grab only sees keys pressed while an X11 window has focus.

use std::collections::{HashMap, VecDeque};
use std::ffi::{CString, c_char, c_int, c_long, c_uchar, c_uint, c_ulong, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use super::*;
use crate::global_keys::{x11_keysym, x11_mods};

type Display = *mut c_void;

const KEY_PRESS: c_int = 2;
const GRAB_MODE_ASYNC: c_int = 1;
/// `LockMask` (Caps Lock) and `Mod2Mask` (Num Lock): chords must fire with
/// either on, so each is grabbed in every combination of them.
const LOCK: u32 = 0x2;
const NUM_LOCK: u32 = 0x10;
/// Only the eight modifier bits of an event's state identify a chord.
const MODIFIER_BITS: u32 = 0xFF;

/// The grab combinations that make a chord fire regardless of lock keys.
const LOCK_COMBINATIONS: [u32; 4] = [0, LOCK, NUM_LOCK, LOCK | NUM_LOCK];

/// The state of a key event with the lock keys taken out.
fn chord_modifiers(state: u32) -> u32 {
    state & MODIFIER_BITS & !(LOCK | NUM_LOCK)
}

#[repr(C)]
struct XKeyEvent {
    kind: c_int,
    serial: c_ulong,
    send_event: c_int,
    display: Display,
    window: c_ulong,
    root: c_ulong,
    subwindow: c_ulong,
    time: c_ulong,
    x: c_int,
    y: c_int,
    x_root: c_int,
    y_root: c_int,
    state: c_uint,
    keycode: c_uint,
    same_screen: c_int,
}

/// `XEvent` is a union of 24 `long`s; only the key variant is read here.
#[repr(C)]
union XEvent {
    kind: c_int,
    key: std::mem::ManuallyDrop<XKeyEvent>,
    pad: [c_long; 24],
}

type ErrorHandler = Option<unsafe extern "C" fn(Display, *mut c_void) -> c_int>;

/// The `libX11` entry points the backend uses.
struct Xlib {
    open_display: unsafe extern "C" fn(*const c_char) -> Display,
    close_display: unsafe extern "C" fn(Display) -> c_int,
    default_root_window: unsafe extern "C" fn(Display) -> c_ulong,
    keysym_to_keycode: unsafe extern "C" fn(Display, c_ulong) -> c_uchar,
    grab_key: unsafe extern "C" fn(Display, c_int, c_uint, c_ulong, c_int, c_int, c_int) -> c_int,
    ungrab_key: unsafe extern "C" fn(Display, c_int, c_uint, c_ulong) -> c_int,
    sync: unsafe extern "C" fn(Display, c_int) -> c_int,
    pending: unsafe extern "C" fn(Display) -> c_int,
    next_event: unsafe extern "C" fn(Display, *mut XEvent) -> c_int,
    connection_number: unsafe extern "C" fn(Display) -> c_int,
    set_error_handler: unsafe extern "C" fn(ErrorHandler) -> ErrorHandler,
}

impl Xlib {
    fn load() -> Option<Self> {
        let library = ["libX11.so.6", "libX11.so"].into_iter().find_map(|name| {
            let name = CString::new(name).ok()?;
            // SAFETY: a valid NUL-terminated library name; the handle is never closed.
            let handle = unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
            (!handle.is_null()).then_some(handle)
        })?;
        macro_rules! symbol {
            ($name:literal) => {{
                let name = CString::new($name).ok()?;
                // SAFETY: `library` is a live handle and `name` is NUL-terminated.
                let address = unsafe { libc::dlsym(library, name.as_ptr()) };
                if address.is_null() {
                    return None;
                }
                // SAFETY: every symbol is bound to its documented Xlib signature
                // in the field it initialises.
                unsafe { std::mem::transmute(address) }
            }};
        }
        Some(Self {
            open_display: symbol!("XOpenDisplay"),
            close_display: symbol!("XCloseDisplay"),
            default_root_window: symbol!("XDefaultRootWindow"),
            keysym_to_keycode: symbol!("XKeysymToKeycode"),
            grab_key: symbol!("XGrabKey"),
            ungrab_key: symbol!("XUngrabKey"),
            sync: symbol!("XSync"),
            pending: symbol!("XPending"),
            next_event: symbol!("XNextEvent"),
            connection_number: symbol!("XConnectionNumber"),
            set_error_handler: symbol!("XSetErrorHandler"),
        })
    }
}

/// Set by [`on_x_error`] while a grab is being checked.
static GRAB_FAILED: AtomicBool = AtomicBool::new(false);

unsafe extern "C" fn on_x_error(_: Display, _: *mut c_void) -> c_int {
    GRAB_FAILED.store(true, Ordering::SeqCst);
    0
}

enum Request {
    Register { id: GlobalShortcutId, keysym: u32, modifiers: u32, reply: Sender<Result<(), GlobalShortcutError>> },
    Unregister(GlobalShortcutId),
    Quit,
}

type Queue = Arc<Mutex<VecDeque<Request>>>;

struct X11 {
    queue: Queue,
    wake: c_int,
    thread: Option<JoinHandle<()>>,
}

impl X11 {
    fn start() -> Option<Self> {
        let xlib = Xlib::load()?;
        let mut pipe = [0; 2];
        // SAFETY: a two-element array is what `pipe` fills in.
        if unsafe { libc::pipe(pipe.as_mut_ptr()) } != 0 {
            return None;
        }
        let [read, write] = pipe;
        let queue: Queue = Arc::default();
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread_queue = Arc::clone(&queue);
        let thread = std::thread::Builder::new()
            .name("aimer-global-shortcuts".into())
            // The thread owns the read end and closes it when it ends.
            .spawn(move || {
                run(xlib, thread_queue, read, ready_tx);
                // SAFETY: the read end is used by this thread alone.
                unsafe { libc::close(read) };
            });
        let Ok(thread) = thread else {
            // SAFETY: both descriptors were just opened here and are not used again.
            unsafe {
                libc::close(read);
                libc::close(write);
            }
            return None;
        };
        if !ready_rx.recv().unwrap_or(false) {
            let _ = thread.join();
            // SAFETY: the write end is ours and is not used again.
            unsafe { libc::close(write) };
            return None;
        }
        let thread = Some(thread);
        Some(Self { queue, wake: write, thread })
    }

    fn send(&self, request: Request) {
        self.queue.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).push_back(request);
        // SAFETY: one byte from a live buffer to our own pipe.
        unsafe { libc::write(self.wake, [1u8].as_ptr().cast(), 1) };
    }
}

/// The X thread: grab keys on request, report them when pressed.
fn run(xlib: Xlib, queue: Queue, wake: c_int, ready: Sender<bool>) {
    // SAFETY: a null name connects to `$DISPLAY`.
    let display = unsafe { (xlib.open_display)(std::ptr::null()) };
    let _ = ready.send(!display.is_null());
    if display.is_null() {
        return;
    }
    // SAFETY: `display` is the connection just opened.
    let root = unsafe { (xlib.default_root_window)(display) };
    let fds = [
        // SAFETY: as above.
        unsafe { (xlib.connection_number)(display) },
        wake,
    ];
    // (keycode, modifiers) -> id, and id -> the same, to release by id.
    let mut by_chord: HashMap<(u32, u32), GlobalShortcutId> = HashMap::new();
    let mut by_id: HashMap<GlobalShortcutId, (u32, u32)> = HashMap::new();

    loop {
        let mut poll = fds.map(|fd| libc::pollfd { fd, events: libc::POLLIN, revents: 0 });
        // SAFETY: two valid `pollfd`s; blocks until the server or the UI thread speaks.
        if unsafe { libc::poll(poll.as_mut_ptr(), 2, -1) } < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            break;
        }
        if poll[1].revents != 0 {
            let mut drain = [0u8; 64];
            // SAFETY: a valid buffer of the length passed.
            unsafe { libc::read(wake, drain.as_mut_ptr().cast(), drain.len()) };
            let requests: Vec<Request> =
                queue.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).drain(..).collect();
            for request in requests {
                match request {
                    Request::Register { id, keysym, modifiers, reply } => {
                        let result = grab(&xlib, display, root, keysym, modifiers);
                        if let Ok(keycode) = result {
                            by_chord.insert((keycode, modifiers), id);
                            by_id.insert(id, (keycode, modifiers));
                        }
                        let _ = reply.send(result.map(|_| ()));
                    }
                    Request::Unregister(id) => {
                        if let Some((keycode, modifiers)) = by_id.remove(&id) {
                            by_chord.remove(&(keycode, modifiers));
                            ungrab(&xlib, display, root, keycode, modifiers);
                        }
                    }
                    Request::Quit => {
                        // SAFETY: the connection is ours and is not used again.
                        unsafe { (xlib.close_display)(display) };
                        return;
                    }
                }
            }
        }
        // SAFETY: `display` is live; the loop drains what the server queued.
        while unsafe { (xlib.pending)(display) } > 0 {
            let mut event = XEvent { pad: [0; 24] };
            // SAFETY: `event` is the full-size buffer `XNextEvent` fills.
            unsafe { (xlib.next_event)(display, &raw mut event) };
            // SAFETY: the type tag is the first field of every variant.
            if unsafe { event.kind } == KEY_PRESS {
                // SAFETY: the tag says this is a key event.
                let key = unsafe { &event.key };
                if let Some(id) = by_chord.get(&(key.keycode, chord_modifiers(key.state))) {
                    notify(*id);
                }
            }
        }
    }
    // SAFETY: the connection is ours and is not used again.
    unsafe { (xlib.close_display)(display) };
}

/// Grabs the chord in every lock-key combination. Returns the keycode.
fn grab(xlib: &Xlib, display: Display, root: c_ulong, keysym: u32, modifiers: u32) -> Result<u32, GlobalShortcutError> {
    // SAFETY: `display` is live.
    let keycode = u32::from(unsafe { (xlib.keysym_to_keycode)(display, c_ulong::from(keysym)) });
    if keycode == 0 {
        return Err(GlobalShortcutError::KeyNotSupported);
    }
    // A grab another client already holds is reported as an asynchronous X
    // error. Xlib's handler is process-wide and winit installs one too, so ours
    // is in place only for the length of this synchronised call.
    GRAB_FAILED.store(false, Ordering::SeqCst);
    // SAFETY: the handler is a plain function; the previous one is restored below.
    let previous = unsafe { (xlib.set_error_handler)(Some(on_x_error)) };
    for combination in LOCK_COMBINATIONS {
        // SAFETY: `display` and `root` are live, and the arguments are in range.
        unsafe {
            (xlib.grab_key)(display, keycode as c_int, modifiers | combination, root, 0, GRAB_MODE_ASYNC, GRAB_MODE_ASYNC);
        }
    }
    // SAFETY: flushes the requests so any error has been delivered.
    unsafe { (xlib.sync)(display, 0) };
    // SAFETY: restores what was there.
    unsafe { (xlib.set_error_handler)(previous) };
    if GRAB_FAILED.load(Ordering::SeqCst) {
        ungrab(xlib, display, root, keycode, modifiers);
        return Err(GlobalShortcutError::Taken);
    }
    Ok(keycode)
}

fn ungrab(xlib: &Xlib, display: Display, root: c_ulong, keycode: u32, modifiers: u32) {
    for combination in LOCK_COMBINATIONS {
        // SAFETY: `display` and `root` are live; ungrabbing what is not held is harmless.
        unsafe { (xlib.ungrab_key)(display, keycode as c_int, modifiers | combination, root) };
    }
}

impl Backend for X11 {
    fn register(&mut self, id: GlobalShortcutId, chord: &KeyChord) -> Result<(), GlobalShortcutError> {
        let keysym = x11_keysym(chord.key()).ok_or(GlobalShortcutError::KeyNotSupported)?;
        let (reply, answer) = mpsc::channel();
        self.send(Request::Register { id, keysym, modifiers: x11_mods(chord.mods()), reply });
        answer
            .recv()
            .unwrap_or_else(|_| Err(GlobalShortcutError::Platform("the shortcut thread ended".into())))
    }

    fn unregister(&mut self, id: GlobalShortcutId) {
        self.send(Request::Unregister(id));
    }
}

impl Drop for X11 {
    fn drop(&mut self) {
        self.send(Request::Quit);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        // SAFETY: the pipe's write end is ours, and the thread that read it has ended.
        unsafe { libc::close(self.wake) };
    }
}

pub(super) fn backend() -> Box<dyn Backend> {
    match X11::start() {
        Some(x11) => Box::new(x11),
        None => Box::new(Unavailable),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_keys_do_not_change_which_chord_a_press_is() {
        assert_eq!(chord_modifiers(0x4), 0x4);
        assert_eq!(chord_modifiers(0x4 | LOCK), 0x4);
        assert_eq!(chord_modifiers(0x4 | NUM_LOCK | LOCK), 0x4);
        assert_eq!(chord_modifiers(0x4 | 0xFF00), 0x4, "button state is not a modifier");
    }

    #[test]
    fn a_chord_is_grabbed_with_and_without_each_lock_key() {
        assert!(LOCK_COMBINATIONS.contains(&0));
        assert!(LOCK_COMBINATIONS.contains(&(LOCK | NUM_LOCK)));
        assert_eq!(LOCK_COMBINATIONS.len(), 4);
    }
}
