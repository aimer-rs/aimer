//! Moving a task between frame phases with `await`.
//!
//! A task runs in the phase it was spawned into, and `yield_now` comes back in
//! that same phase. These hop instead: the work (or just the continuation)
//! lands in another phase, which is how one handler can wait for the next
//! animation tick and then do slack-time work, without being split across two
//! spawns.

use std::pin::Pin;
use std::task::{Context, Poll};

use crate::handle::{JoinError, JoinHandle};
use crate::task::{Phase, ScopeId};
use crate::venus::Venus;

enum State<T> {
    Running(JoinHandle<T>),
    Unavailable,
}

/// The future returned by [`Venus::run_in_phase`], [`Venus::next_frame`] and
/// their free forms: a task running in another phase, awaited.
///
/// Dropping it before it resolves cancels the hopped work.
#[must_use = "a hop does nothing unless awaited"]
pub struct Hop<T>(State<T>);

impl<T> Future for Hop<T> {
    type Output = Result<T, JoinError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match &mut self.0 {
            State::Running(handle) => Pin::new(handle).poll(cx),
            State::Unavailable => Poll::Ready(Err(JoinError::RuntimeUnavailable)),
        }
    }
}

impl Venus {
    /// Runs `future` as a task in `phase` and resolves to its output.
    ///
    /// The calling task is suspended until the target phase has run the future
    /// to completion; it then resumes in *its own* phase. Awaiting a hop into
    /// [`Phase::Idle`] therefore means "do this when the frame has slack", and
    /// the caller picks the result up on its next turn.
    ///
    /// # Errors
    ///
    /// Resolves to [`JoinError::Cancelled`] if the hopped task is cancelled.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_venus::{Phase, Venus};
    ///
    /// let venus = Venus::new();
    /// let runtime = venus.clone();
    /// let parsed = venus.run_until(async move {
    ///     // Heavy but sliceable: wait for idle room, then do it.
    ///     runtime
    ///         .run_in_phase(Phase::Idle, async { "42".parse::<u32>().unwrap() })
    ///         .await
    /// });
    /// assert_eq!(parsed, Ok(42));
    /// ```
    #[inline]
    pub fn run_in_phase<F>(&self, phase: Phase, future: F) -> Hop<F::Output>
    where
        F: Future + 'static,
        F::Output: 'static,
    {
        Hop(State::Running(self.spawn_with_handle_in_phase(
            phase,
            ScopeId::ROOT,
            future,
        )))
    }

    /// Waits for the next frame-task pass after this call.
    ///
    /// The caller resumes in the same frame's microtask drain, ahead of the
    /// build — so what it does next is visible to *that* frame, the same
    /// guarantee an animation tick has.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::cell::Cell;
    /// use std::rc::Rc;
    ///
    /// use aimer_venus::Venus;
    ///
    /// let venus = Venus::new();
    /// let state = Rc::new(Cell::new(0));
    ///
    /// let runtime = venus.clone();
    /// let mutated = state.clone();
    /// venus.spawn(async move {
    ///     runtime.next_frame().await.unwrap();
    ///     mutated.set(1);
    /// });
    /// venus.run_microtasks(); // the task starts and parks on the next frame
    ///
    /// let read = state.clone();
    /// let built_with = venus.drive_frame(|| read.get());
    /// assert_eq!(built_with, 1, "the continuation landed before the build");
    /// ```
    #[inline]
    pub fn next_frame(&self) -> Hop<()> {
        self.run_in_phase(Phase::Frame, async {})
    }
}

/// [`Venus::run_in_phase`] on the runtime installed for this thread.
///
/// Resolves to [`JoinError::RuntimeUnavailable`] when none is installed.
#[inline]
pub fn run_in_phase<F>(phase: Phase, future: F) -> Hop<F::Output>
where
    F: Future + 'static,
    F::Output: 'static,
{
    match Venus::current() {
        Some(venus) => venus.run_in_phase(phase, future),
        None => Hop(State::Unavailable),
    }
}

/// [`Venus::next_frame`] on the runtime installed for this thread.
#[inline]
pub fn next_frame() -> Hop<()> {
    run_in_phase(Phase::Frame, async {})
}

/// Runs `future` when the frame has idle room; see [`Venus::run_in_phase`].
#[inline]
pub fn on_idle<F>(future: F) -> Hop<F::Output>
where
    F: Future + 'static,
    F::Output: 'static,
{
    run_in_phase(Phase::Idle, future)
}

// `run_until` sleeps the calling thread, which a browser cannot do.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::time::Duration;

    use super::*;
    use crate::FrameBudget;

    #[test]
    fn next_frame_waits_for_the_frame_task_pass() {
        let venus = Venus::new();
        let done = Rc::new(Cell::new(false));
        let (runtime, flag) = (venus.clone(), done.clone());
        venus.spawn(async move {
            runtime.next_frame().await.unwrap();
            flag.set(true);
        });

        venus.run_microtasks();
        assert!(!done.get(), "microtasks alone are not a frame");

        venus.run_frame_tasks();
        venus.run_microtasks();
        assert!(done.get());
    }

    #[test]
    fn the_continuation_after_next_frame_lands_before_the_build() {
        let venus = Venus::new();
        let state = Rc::new(Cell::new(0));
        let (runtime, mutated) = (venus.clone(), state.clone());
        venus.spawn(async move {
            runtime.next_frame().await.unwrap();
            mutated.set(7);
        });
        venus.run_microtasks();

        let read = state.clone();
        let seen = venus.drive_frame(|| read.get());

        assert_eq!(seen, 7);
    }

    #[test]
    fn a_hop_to_idle_runs_only_with_room() {
        let venus = Venus::new();
        let output = Rc::new(RefCell::new(None));
        let (runtime, slot) = (venus.clone(), output.clone());
        venus.spawn(async move {
            *slot.borrow_mut() = Some(runtime.run_in_phase(Phase::Idle, async { 5 }).await);
        });
        venus.run_microtasks();

        assert_eq!(venus.run_idle(&FrameBudget::exhausted()), 0);
        venus.run_microtasks();
        assert!(output.borrow().is_none());

        venus.run_idle(&FrameBudget::from_now(Duration::from_millis(4)));
        venus.run_microtasks();
        assert_eq!(*output.borrow(), Some(Ok(5)));
    }

    #[test]
    fn dropping_a_hop_cancels_the_hopped_work() {
        let venus = Venus::new();
        let ran = Rc::new(Cell::new(false));
        let flag = ran.clone();
        let hop = venus.run_in_phase(Phase::Idle, async move { flag.set(true) });

        drop(hop);
        venus.run_idle(&FrameBudget::from_now(Duration::from_millis(4)));

        assert!(!ran.get());
        assert_eq!(venus.task_count(), 0);
    }

    #[test]
    fn the_free_functions_use_the_installed_runtime() {
        let venus = Venus::new();
        venus.install();
        let hop = on_idle(async { 3 });
        let frame = next_frame();
        Venus::uninstall();

        let (idle, frame) = venus.run_until(async move { (hop.await, frame.await) });

        assert_eq!((idle, frame), (Ok(3), Ok(())));
    }

    #[test]
    fn without_a_runtime_a_hop_reports_it() {
        let venus = Venus::new();
        assert!(Venus::current().is_none());

        let outcome = venus.run_until(next_frame());

        assert_eq!(outcome, Err(JoinError::RuntimeUnavailable));
    }
}
