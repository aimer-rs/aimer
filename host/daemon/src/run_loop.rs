//! Blocking the calling thread until the daemon is asked to quit.
//!
//! macOS cannot simply park when there is a tray: the status bar item is
//! serviced by the application event loop, which must run on the main thread.
//! Everywhere else the tray has a thread of its own, so waiting is a condition
//! variable, and so it is on macOS for a daemon off the main thread (it can have
//! no tray).

use crate::DaemonError;
use crate::daemon::Shared;

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, Ordering};

    use objc2::MainThreadMarker;
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSEvent, NSEventModifierFlags, NSEventType,
    };
    use objc2_foundation::NSPoint;

    use super::{DaemonError, Shared};

    unsafe extern "C" {
        /// The main dispatch queue (`dispatch_get_main_queue()` is a macro over
        /// this symbol).
        static _dispatch_main_q: c_void;
        fn dispatch_async_f(
            queue: *const c_void,
            context: *mut c_void,
            work: extern "C" fn(*mut c_void),
        );
    }

    /// Whether the system event loop is (about to be) running under
    /// [`block_until_quit`], and so needs [`wake`] to leave it.
    static LOOP_RUNNING: AtomicBool = AtomicBool::new(false);

    pub(crate) fn prepare(show_in_dock: bool, has_tray: bool) -> Result<(), DaemonError> {
        let Some(mtm) = MainThreadMarker::new() else {
            // The status bar may only be touched from the main thread.
            return if has_tray { Err(DaemonError::NotMainThread) } else { Ok(()) };
        };
        let policy = if show_in_dock {
            NSApplicationActivationPolicy::Regular
        } else {
            // No Dock icon and no menu bar of its own: the tray is the interface.
            NSApplicationActivationPolicy::Accessory
        };
        NSApplication::sharedApplication(mtm).setActivationPolicy(policy);
        Ok(())
    }

    pub(crate) fn block_until_quit(shared: &Shared) {
        let Some(mtm) = MainThreadMarker::new() else {
            shared.wait_for_quit();
            return;
        };
        // Publish "running" before checking for a quit, and `wake` publishes the
        // quit before checking "running": one of the two always sees the other,
        // so a quit cannot slip between the check and the loop starting.
        LOOP_RUNNING.store(true, Ordering::SeqCst);
        if !shared.is_quitting() {
            NSApplication::sharedApplication(mtm).run();
        }
        LOOP_RUNNING.store(false, Ordering::SeqCst);
    }

    /// Asks the main thread to leave [`block_until_quit`]; callable from any
    /// thread, after the quit flag is set.
    pub(crate) fn wake() {
        if !LOOP_RUNNING.load(Ordering::SeqCst) {
            return;
        }
        // SAFETY: `_dispatch_main_q` is the process-wide main queue, valid for
        // the life of the process, and `stop_loop` ignores its context.
        unsafe {
            dispatch_async_f(&raw const _dispatch_main_q, std::ptr::null_mut(), stop_loop);
        }
    }

    extern "C" fn stop_loop(_context: *mut c_void) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        let app = NSApplication::sharedApplication(mtm);
        app.stop(None);
        // `stop:` only takes effect once the loop handles another event, and a
        // loop with nothing to do would sleep through it.
        let wake = NSEvent::otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2(
            NSEventType::ApplicationDefined,
            NSPoint::new(0.0, 0.0),
            NSEventModifierFlags::empty(),
            0.0,
            0,
            None,
            0,
            0,
            0,
        );
        if let Some(wake) = wake {
            app.postEvent_atStart(&wake, true);
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::{DaemonError, Shared};

    pub(crate) fn prepare(_show_in_dock: bool, _has_tray: bool) -> Result<(), DaemonError> {
        Ok(())
    }

    pub(crate) fn block_until_quit(shared: &Shared) {
        shared.wait_for_quit();
    }

    /// Nothing to do: waiters are woken by the condition variable.
    pub(crate) fn wake() {}
}

pub(crate) use imp::{block_until_quit, prepare, wake};
