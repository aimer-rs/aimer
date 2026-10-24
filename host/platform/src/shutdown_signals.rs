//! Turning `SIGTERM`, `SIGINT` and `SIGHUP` into an orderly exit.
//!
//! By default these signals end a process on the spot: no destructor runs, no
//! window closes, and nothing pending is saved. They are what `kill`, Ctrl-C in
//! the terminal that launched the app, and a closing terminal send, so they are
//! how a developer's app is most often "killed". This module redirects them into
//! the event loop's normal exit, which tells the app's services to finish what
//! they hold (see `aimer_venus::notify_lifecycle`).
//!
//! `SIGKILL` and a crash cannot be caught by any process, so nothing here helps
//! with those; they are covered by saving promptly and atomically instead.
//!
//! # How
//!
//! A signal handler may do almost nothing, and the event loop may be asleep, so
//! the signals are blocked in every thread and one dedicated thread waits for
//! them with `sigwait`, which is ordinary thread context where waking the loop is
//! safe. The mask is inherited by threads created afterwards, so [`install`]
//! should run before the app starts threads of its own; a thread that predates
//! it keeps the default behavior, which is no worse than before.
//!
//! The first signal asks for a graceful exit. A second one, or a first one when
//! the loop cannot be reached, restores the default action and re-raises, so a
//! wedged app can always still be stopped with a second Ctrl-C.

use std::mem::MaybeUninit;

const SIGNALS: [libc::c_int; 3] = [libc::SIGTERM, libc::SIGINT, libc::SIGHUP];

/// Starts the thread that waits for the termination signals.
///
/// `request_exit` is called, on that thread, for the first signal, and returns
/// whether it managed to ask the application to exit. When it returns `false`,
/// or a signal arrives after it returned `true`, the process is terminated the
/// way the signal would have terminated it without this module.
///
/// Does nothing if called twice, or if the signals cannot be blocked.
pub fn install(request_exit: impl Fn() -> bool + Send + 'static) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static INSTALLED: AtomicBool = AtomicBool::new(false);
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }

    let set = signal_set();
    // SAFETY: `set` is a fully initialized signal set, and a null old-set
    // pointer is permitted.
    if unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut()) } != 0 {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("aimer-shutdown-signals".to_owned())
        .spawn(move || wait(&set, &request_exit));
    if spawned.is_err() {
        // Without the waiting thread, blocking the signals would swallow them.
        // SAFETY: as above.
        unsafe { libc::pthread_sigmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut()) };
    }
}

fn signal_set() -> libc::sigset_t {
    let mut set = MaybeUninit::<libc::sigset_t>::uninit();
    // SAFETY: `sigemptyset` initializes the whole set, after which adding valid
    // signal numbers is defined.
    unsafe {
        libc::sigemptyset(set.as_mut_ptr());
        for signal in SIGNALS {
            libc::sigaddset(set.as_mut_ptr(), signal);
        }
        set.assume_init()
    }
}

fn wait(set: &libc::sigset_t, request_exit: &dyn Fn() -> bool) {
    let mut graceful_exit_requested = false;
    loop {
        let mut signal: libc::c_int = 0;
        // SAFETY: `set` is initialized and `signal` is a valid out-pointer.
        if unsafe { libc::sigwait(set, &mut signal) } != 0 {
            return;
        }
        if !graceful_exit_requested && request_exit() {
            graceful_exit_requested = true;
            continue;
        }
        terminate_as_by_default(set, signal);
        return;
    }
}

/// Ends the process as `signal` would have if it had never been intercepted.
fn terminate_as_by_default(set: &libc::sigset_t, signal: libc::c_int) {
    // SAFETY: restoring the default disposition, unblocking on this thread, and
    // raising to this thread are all defined for a valid signal number; the
    // default action for these signals terminates the process.
    unsafe {
        libc::signal(signal, libc::SIG_DFL);
        libc::pthread_sigmask(libc::SIG_UNBLOCK, set, std::ptr::null_mut());
        libc::raise(signal);
    }
}
