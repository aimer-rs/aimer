//! Making the application's Tokio runtime findable while Venus polls a task.
//!
//! Venus owns the UI thread and Tokio owns the sockets: the driver threads run
//! beside the frame, so a TLS handshake or a response decode never eats frame
//! time, and the completion comes home through Venus's waker into the phase the
//! task was spawned into.
//!
//! One thing does not follow from that split. A future from a runtime-backed
//! ecosystem builds its resources on its *first poll* and looks the runtime up
//! in a thread-local — `reqwest`'s connector registering a socket with the
//! reactor, `tokio::time::sleep` registering with the timer wheel. Venus polls
//! on the UI thread, where that thread-local is empty, so such a future would
//! panic with "there is no reactor running" before it ever reached the network.
//!
//! Entering the handle for the duration of a poll is the whole fix, and it is
//! what this adapter is.

use aimer_venus::PollContext;
use tokio::runtime::Handle;

/// Enters the application's Tokio runtime for exactly as long as a task is
/// being polled.
///
/// Installed once by the platform loop, next to `Venus::install`, so *every*
/// spawn path — `spawn`, `spawn_in`, `spawn_frame`, `spawn_idle`, an
/// `AsyncBuilder`, a future launched from a gesture handler — can await a
/// foreign future. Nothing in the widget tree has to know a runtime exists.
///
/// The cost is a thread-local swap per poll, against a poll that is already
/// doing real work.
pub struct TokioPollContext {
    handle: Handle,
}

impl TokioPollContext {
    /// Wraps the handle of the runtime the application drives its I/O with.
    ///
    /// Enable the runtime's time driver to support Venus task deadlines.
    #[inline]
    pub fn new(handle: Handle) -> Self {
        Self { handle }
    }
}

impl PollContext for TokioPollContext {
    /// Runs one poll inside the runtime, and drops the guard before returning.
    ///
    /// Scoped to the poll rather than to the task on purpose: a guard kept
    /// across an `await` would leave the UI thread marked as being inside the
    /// runtime while it builds, lays out and paints.
    #[inline]
    fn enter(&self, poll: &mut dyn FnMut()) {
        let _guard = self.handle.enter();
        poll();
    }

    fn sleep(
        &self,
        duration: std::time::Duration,
    ) -> Option<std::pin::Pin<Box<dyn Future<Output = ()>>>> {
        let _guard = self.handle.enter();
        Some(Box::pin(tokio::time::sleep(duration)))
    }

    /// The window active when the task is spawned owns it.
    ///
    /// With several windows on one scheduler, a task is polled in whichever
    /// window's frame comes first; what it does while it runs — asking for a
    /// frame, recording what to repaint — is still about the window it was
    /// spawned for.
    #[inline]
    fn task_tag(&self) -> u64 {
        aimer_events::window::active_window().map_or(0, |window| window.address() as u64)
    }

    #[inline]
    fn enter_tagged(&self, tag: u64, poll: &mut dyn FnMut()) {
        aimer_events::window::with_window_scope(
            aimer_events::window::WindowKey::from_address(tag as usize),
            poll,
        );
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::Duration;

    use aimer_venus::Venus;
    use tokio::runtime::Runtime;

    use super::*;

    /// The property every runtime-backed crate depends on: while Venus polls,
    /// `Handle::current()` resolves. Everything else — a socket, a timer, a
    /// `reqwest` connector — is downstream of this one fact.
    #[test]
    fn a_task_polled_by_venus_can_find_the_runtime() {
        let runtime = Runtime::new().expect("a runtime for the test");
        let venus = Venus::new();
        venus.set_poll_context(TokioPollContext::new(runtime.handle().clone()));

        let found = Rc::new(Cell::new(false));
        let seen = found.clone();
        venus.spawn(async move { seen.set(Handle::try_current().is_ok()) });
        venus.run_microtasks();

        assert!(found.get());
    }

    /// Without the context the same task is polled bare, which is the state the
    /// adapter exists to leave behind.
    #[test]
    fn a_task_polled_without_the_context_finds_nothing() {
        let venus = Venus::new();

        let found = Rc::new(Cell::new(true));
        let seen = found.clone();
        venus.spawn(async move { seen.set(Handle::try_current().is_ok()) });
        venus.run_microtasks();

        assert!(!found.get());
    }

    /// The end-to-end shape: a Tokio future is spawned into a Venus phase, its
    /// timer runs on Tokio's own thread, and the completion lands back on the
    /// UI thread — while the task keeps a non-`Send` capture the whole way.
    #[test]
    fn a_tokio_timer_completes_inside_a_venus_task() {
        let runtime = Runtime::new().expect("a runtime for the test");
        let venus = Venus::new();
        venus.set_poll_context(TokioPollContext::new(runtime.handle().clone()));

        let slept = Rc::new(Cell::new(false));
        let flag = slept.clone();
        venus.spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            flag.set(true);
        });

        // Stands in for the event loop: the timer fires on a Tokio thread and
        // wakes the task, which the next drain picks up.
        let deadline = aimer_time::Instant::now() + Duration::from_secs(5);
        while venus.task_count() > 0 && aimer_time::Instant::now() < deadline {
            venus.run_microtasks();
            std::thread::sleep(Duration::from_millis(1));
        }

        assert!(slept.get(), "the timer resolved on the UI thread");
    }

    #[test]
    fn a_venus_deadline_can_be_created_outside_the_tokio_context() {
        let runtime = Runtime::new().expect("a runtime for the test");
        let venus = Venus::new();
        venus.set_poll_context(TokioPollContext::new(runtime.handle().clone()));
        assert!(Handle::try_current().is_err());

        let task = venus.timeout(Duration::ZERO, std::future::pending::<()>());
        let result = runtime.block_on(task);

        assert_eq!(result, Err(aimer_venus::TimeoutError::Elapsed));
        assert!(Handle::try_current().is_err());
    }

    #[test]
    fn a_task_is_polled_as_the_window_that_spawned_it() {
        use aimer_events::window::{WindowKey, active_window, register_window_scope, with_window_scope};

        let runtime = Runtime::new().expect("a runtime for the test");
        let venus = Venus::new();
        venus.set_poll_context(TokioPollContext::new(runtime.handle().clone()));
        let (a, b) = (WindowKey::from_address(0xA0), WindowKey::from_address(0xB0));
        let _a = register_window_scope(a, || {}, || {});
        let _b = register_window_scope(b, || {}, || {});

        let seen = Rc::new(Cell::new(None));
        with_window_scope(a, || {
            let seen = seen.clone();
            venus.spawn(async move { seen.set(Some(active_window())) });
        });
        // The task runs while another window is the active one.
        with_window_scope(b, || {
            venus.run_microtasks();
        });

        assert_eq!(seen.get(), Some(Some(a)));
        assert_eq!(active_window(), None);
    }

    #[test]
    fn a_task_spawned_with_no_window_active_runs_in_whichever_polls_it() {
        use aimer_events::window::{WindowKey, active_window, register_window_scope, with_window_scope};

        let runtime = Runtime::new().expect("a runtime for the test");
        let venus = Venus::new();
        venus.set_poll_context(TokioPollContext::new(runtime.handle().clone()));
        let b = WindowKey::from_address(0xB0);
        let _b = register_window_scope(b, || {}, || {});

        let seen = Rc::new(Cell::new(None));
        let observed = seen.clone();
        venus.spawn(async move { observed.set(Some(active_window())) });
        with_window_scope(b, || {
            venus.run_microtasks();
        });

        assert_eq!(seen.get(), Some(Some(b)));
    }
}
