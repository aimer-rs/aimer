//! Telling the app's services about a panic while they can still act on it.
//!
//! A release build aborts on panic, which ends the process the instant the panic
//! hook returns, with no unwinding, no destructors and no event-loop exit. The
//! hook is therefore the only place left to save anything, and it runs on
//! whichever thread panicked.
//!
//! Services that hold unsaved state, such as a persistence layer, live on the UI
//! thread (their state is `Rc`-based and not `Send`), so a panic on any other
//! thread has to be handed to the UI thread and waited for. A task on the UI
//! thread's runtime sleeps on a thread-safe waker for that: the panicking thread
//! raises a request, wakes the task (which wakes the event loop if it is parked),
//! and blocks until the task has delivered [`Lifecycle::Panic`] to the services.
//! The wait is bounded, because the UI thread may be the very thing that is stuck.

use std::cell::Cell;
use std::future::Future;
use std::panic;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, Waker};
use std::thread::ThreadId;
use std::time::Duration;

use crate::{Lifecycle, Venus, notify_lifecycle};

/// The longest a panicking thread waits for the UI thread to finish saving.
///
/// Matches the persistence layer's lifecycle timeout, which is what the UI thread
/// will spend; a little more would be lost to a watchdog anyway.
const FORWARD_TIMEOUT: Duration = Duration::from_secs(3);

/// The UI thread's side of a request from another thread.
struct Forward {
    home: ThreadId,
    state: Mutex<ForwardState>,
    handled_changed: Condvar,
}

#[derive(Default)]
struct ForwardState {
    requested: u64,
    handled: u64,
    waker: Option<Waker>,
}

impl Forward {
    fn lock(&self) -> MutexGuard<'_, ForwardState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Asks the UI thread to deliver the panic, and waits until it has.
    fn request_and_wait(&self) {
        let mut state = self.lock();
        state.requested += 1;
        let ticket = state.requested;
        if let Some(waker) = state.waker.take() {
            // Outside the lock: waking runs scheduler code.
            drop(state);
            waker.wake();
            state = self.lock();
        }
        let _ = self
            .handled_changed
            .wait_timeout_while(state, FORWARD_TIMEOUT, |state| state.handled < ticket);
    }
}

/// Pending until a request is waiting to be handled.
struct NextRequest(Arc<Forward>);

impl Future for NextRequest {
    type Output = u64;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<u64> {
        let mut state = self.0.lock();
        if state.requested > state.handled {
            Poll::Ready(state.requested)
        } else {
            state.waker = Some(context.waker().clone());
            Poll::Pending
        }
    }
}

static FORWARD: Mutex<Option<Arc<Forward>>> = Mutex::new(None);
static HOOKED: AtomicBool = AtomicBool::new(false);

aimer_tls::thread_local! {
    /// Set while this thread is inside the hook, so a panic raised by a service
    /// reacting to a panic does not recurse.
    static IN_HOOK: Cell<bool> = const { Cell::new(false) };
}

/// Makes every panic, on any thread, deliver [`Lifecycle::Panic`] to the
/// services registered on the UI thread with [`on_lifecycle`](crate::on_lifecycle).
///
/// Call it once on the UI thread, after the runtime is installed
/// ([`Venus::install`](crate::Venus::install)); the runtime is what lets a panic
/// on another thread reach the UI thread. Without one, only panics on the calling
/// thread are delivered.
///
/// The previous panic hook is kept and runs first, so the panic message and
/// backtrace are printed before anything is saved. Calling this again re-targets
/// the forwarding at the current runtime and does not stack hooks.
///
/// The notification runs inside the hook, possibly in the middle of anything,
/// which is why [`Lifecycle::Panic`] callbacks must be minimal and must not run
/// tasks. They must also not panic: Rust aborts on a panic raised while another
/// is being handled, and nothing inside a hook can prevent that.
pub fn install_panic_hook() {
    if let Some(venus) = Venus::current() {
        let forward = Arc::new(Forward {
            home: std::thread::current().id(),
            state: Mutex::new(ForwardState::default()),
            handled_changed: Condvar::new(),
        });
        *FORWARD.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::clone(&forward));
        venus.spawn(async move {
            loop {
                let ticket = NextRequest(Arc::clone(&forward)).await;
                deliver();
                forward.lock().handled = ticket;
                forward.handled_changed.notify_all();
            }
        });
    }

    if !HOOKED.swap(true, Ordering::SeqCst) {
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            previous(info);
            on_panic();
        }));
    }
}

/// Runs the registered callbacks on the calling thread.
///
/// A callback must not panic. Rust aborts the process on a panic raised while
/// another is being handled, and that holds inside a panic hook whatever
/// `catch_unwind` is wrapped around it, so there is no way to contain one.
fn deliver() {
    notify_lifecycle(Lifecycle::Panic);
}

fn on_panic() {
    if IN_HOOK.with(|flag| flag.replace(true)) {
        return;
    }
    let forward = FORWARD
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    match forward {
        Some(forward) if forward.home != std::thread::current().id() => forward.request_and_wait(),
        _ => deliver(),
    }
    IN_HOOK.with(|flag| flag.set(false));
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::thread;
    use std::time::Instant;

    use super::*;
    use crate::on_lifecycle;

    fn silence_panic_output() {
        // Keep the test log readable; the hook chain still runs.
        let previous = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            if std::env::var_os("AIMER_TEST_SHOW_PANICS").is_some() {
                previous(info);
            }
        }));
    }

    #[test]
    fn a_panic_on_the_ui_thread_is_delivered_before_it_unwinds() {
        silence_panic_output();
        let venus = Venus::new();
        venus.install();
        install_panic_hook();

        let seen = Rc::new(RefCell::new(Vec::new()));
        let log = seen.clone();
        let _subscription = on_lifecycle(move |event| log.borrow_mut().push(event));

        let result = panic::catch_unwind(|| panic!("boom"));

        assert!(result.is_err());
        assert_eq!(*seen.borrow(), [Lifecycle::Panic]);
        Venus::uninstall();
    }

    /// The case that matters in a release build: a worker panics, the process
    /// aborts when the hook returns, and the UI thread's state must be saved
    /// before the worker is let go.
    #[test]
    fn a_panic_on_another_thread_is_delivered_on_the_ui_thread_before_it_returns() {
        silence_panic_output();
        let venus = Venus::new();
        venus.install();
        install_panic_hook();

        let ui_thread = thread::current().id();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let log = seen.clone();
        let _subscription = on_lifecycle(move |event| {
            log.borrow_mut().push((event, thread::current().id()));
        });

        let worker = thread::spawn(|| {
            let _ = panic::catch_unwind(|| panic!("worker boom"));
            // Only reached once the hook has returned, i.e. after the UI thread
            // finished its part.
            Instant::now()
        });

        let deadline = Instant::now() + Duration::from_secs(10);
        while !worker.is_finished() {
            assert!(Instant::now() < deadline, "the worker was never released");
            venus.run_microtasks();
            thread::sleep(Duration::from_millis(1));
        }
        let released_at = worker.join().expect("the worker finishes");

        assert_eq!(*seen.borrow(), [(Lifecycle::Panic, ui_thread)]);
        assert!(released_at.elapsed() < Duration::from_secs(5));
        Venus::uninstall();
    }

    /// The UI thread may be what is stuck; the worker must not hang with it.
    #[test]
    fn a_ui_thread_that_never_answers_cannot_hold_the_panicking_thread_forever() {
        silence_panic_output();
        let venus = Venus::new();
        venus.install();
        install_panic_hook();

        let started = Instant::now();
        // The UI thread never pumps the runtime.
        let worker = thread::spawn(|| {
            let _ = panic::catch_unwind(|| panic!("worker boom"));
        });
        worker.join().expect("the worker finishes");

        let waited = started.elapsed();
        assert!(waited >= FORWARD_TIMEOUT, "gave up after {waited:?}");
        assert!(waited < FORWARD_TIMEOUT + Duration::from_secs(5), "{waited:?}");
        Venus::uninstall();
    }

    #[test]
    fn installing_twice_does_not_stack_hooks() {
        silence_panic_output();
        let venus = Venus::new();
        venus.install();
        install_panic_hook();
        install_panic_hook();

        let count = Rc::new(std::cell::Cell::new(0));
        let counted = count.clone();
        let _subscription = on_lifecycle(move |_| counted.set(counted.get() + 1));

        let _ = panic::catch_unwind(|| panic!("once"));

        assert_eq!(count.get(), 1);
        Venus::uninstall();
    }
}
