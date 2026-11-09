//! Windows global shortcuts through `RegisterHotKey`.
//!
//! A hot key is posted to the message queue of the thread that registered it,
//! and winit's event loop does not forward thread messages, so the keys live on
//! a thread of their own that does nothing but pump messages. Registering and
//! unregistering must happen on that same thread, so requests reach it through
//! a queue and a thread message that wakes it.

use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use super::*;
use crate::global_keys::{windows_key, windows_mods};

#[repr(C)]
struct Point {
    x: i32,
    y: i32,
}

#[repr(C)]
struct Msg {
    hwnd: *mut c_void,
    message: u32,
    w_param: usize,
    l_param: isize,
    time: u32,
    pt: Point,
}

const WM_HOTKEY: u32 = 0x0312;
/// A message that only means "look at the request queue".
const WM_APP: u32 = 0x8000;
const MOD_NOREPEAT: u32 = 0x4000;
const PM_NOREMOVE: u32 = 0;
const ERROR_HOTKEY_ALREADY_REGISTERED: u32 = 1409;

#[link(name = "user32")]
unsafe extern "system" {
    fn RegisterHotKey(hwnd: *mut c_void, id: i32, modifiers: u32, vk: u32) -> i32;
    fn UnregisterHotKey(hwnd: *mut c_void, id: i32) -> i32;
    fn GetMessageW(msg: *mut Msg, hwnd: *mut c_void, min: u32, max: u32) -> i32;
    fn PeekMessageW(msg: *mut Msg, hwnd: *mut c_void, min: u32, max: u32, remove: u32) -> i32;
    fn PostThreadMessageW(thread: u32, message: u32, w_param: usize, l_param: isize) -> i32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentThreadId() -> u32;
    fn GetLastError() -> u32;
}

enum Request {
    Register { id: GlobalShortcutId, modifiers: u32, vk: u32, reply: Sender<Result<(), GlobalShortcutError>> },
    Unregister(GlobalShortcutId),
    Quit,
}

type Queue = Arc<Mutex<VecDeque<Request>>>;

/// Owns the message thread; dropping it ends the thread and so every hot key.
struct Windows {
    queue: Queue,
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

impl Windows {
    fn start() -> Option<Self> {
        let queue: Queue = Arc::default();
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread_queue = Arc::clone(&queue);
        let thread = std::thread::Builder::new()
            .name("aimer-global-shortcuts".into())
            .spawn(move || pump(thread_queue, ready_tx))
            .ok()?;
        let thread_id = ready_rx.recv().ok()?;
        Some(Self { queue, thread_id, thread: Some(thread) })
    }

    fn send(&self, request: Request) {
        self.queue.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).push_back(request);
        // SAFETY: plain thread message to a thread this struct started.
        unsafe { PostThreadMessageW(self.thread_id, WM_APP, 0, 0) };
    }
}

/// The message thread: wait for hot keys and for requests, forever.
fn pump(queue: Queue, ready: Sender<u32>) {
    let mut msg = Msg { hwnd: std::ptr::null_mut(), message: 0, w_param: 0, l_param: 0, time: 0, pt: Point { x: 0, y: 0 } };
    // SAFETY: peeking creates this thread's message queue, which must exist
    // before anything can post to the thread.
    unsafe { PeekMessageW(&raw mut msg, std::ptr::null_mut(), WM_APP, WM_APP, PM_NOREMOVE) };
    // SAFETY: no arguments.
    let _ = ready.send(unsafe { GetCurrentThreadId() });

    // SAFETY: `msg` is a valid out buffer; a negative result is an error and
    // ends the loop like a quit does.
    while unsafe { GetMessageW(&raw mut msg, std::ptr::null_mut(), 0, 0) } > 0 {
        match msg.message {
            WM_HOTKEY => notify(GlobalShortcutId::from_raw(msg.w_param as u32)),
            WM_APP => {
                let requests: Vec<Request> =
                    queue.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).drain(..).collect();
                for request in requests {
                    match request {
                        Request::Register { id, modifiers, vk, reply } => {
                            // SAFETY: a null window registers for this thread.
                            let ok = unsafe {
                                RegisterHotKey(std::ptr::null_mut(), id.as_raw() as i32, modifiers | MOD_NOREPEAT, vk)
                            };
                            let _ = reply.send(if ok != 0 {
                                Ok(())
                            } else {
                                // SAFETY: no arguments; read straight after the failing call.
                                Err(match unsafe { GetLastError() } {
                                    ERROR_HOTKEY_ALREADY_REGISTERED => GlobalShortcutError::Taken,
                                    other => GlobalShortcutError::Platform(format!("RegisterHotKey failed ({other})")),
                                })
                            });
                        }
                        Request::Unregister(id) => {
                            // SAFETY: as above; an id that is not ours just fails.
                            unsafe { UnregisterHotKey(std::ptr::null_mut(), id.as_raw() as i32) };
                        }
                        // Hot keys registered to a thread end with the thread.
                        Request::Quit => return,
                    }
                }
            }
            _ => {}
        }
    }
}

impl Backend for Windows {
    fn register(&mut self, id: GlobalShortcutId, chord: &KeyChord) -> Result<(), GlobalShortcutError> {
        let vk = windows_key(chord.key()).ok_or(GlobalShortcutError::KeyNotSupported)?;
        let (reply, answer) = mpsc::channel();
        self.send(Request::Register { id, modifiers: windows_mods(chord.mods()), vk, reply });
        answer
            .recv()
            .unwrap_or_else(|_| Err(GlobalShortcutError::Platform("the shortcut thread ended".into())))
    }

    fn unregister(&mut self, id: GlobalShortcutId) {
        self.send(Request::Unregister(id));
    }
}

impl Drop for Windows {
    fn drop(&mut self) {
        self.send(Request::Quit);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub(super) fn backend() -> Box<dyn Backend> {
    match Windows::start() {
        Some(windows) => Box::new(windows),
        None => Box::new(Unavailable),
    }
}
