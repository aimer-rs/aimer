//! The hook a host runtime installs so futures from other ecosystems can be
//! polled on the UI thread.

use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};
use std::time::Duration;

/// Something a host runtime has to be *inside of* while a task is polled.
///
/// Venus polls futures on the thread that owns the frame and has no runtime
/// context of its own. That is enough for a future that only awaits other
/// futures, and it is not enough for the ecosystems built on a runtime:
/// `reqwest`, `tokio::fs`, `tokio::time::sleep` and their relatives construct
/// their resources lazily on the *first poll* and look their runtime up in a
/// thread-local. Polled bare, they panic with "there is no reactor running" —
/// which is a fact about where the poll happened, not about the future.
///
/// A host that wants those futures to work installs an adapter with
/// [`Venus::set_poll_context`](crate::Venus::set_poll_context). Venus keeps no
/// knowledge of any runtime: it only knows how to run a closure inside whatever
/// the host handed it.
///
/// The completion side needs nothing here — a foreign reactor wakes Venus's
/// waker from its own thread, and the task resumes in the frame phase it was
/// spawned into, exactly as an [`offload`](crate::Venus::offload) result does.
///
/// # Implementing
///
/// The context must be entered for the duration of the call and no longer.
/// Holding a runtime guard across an `await` would leave the whole thread
/// marked as being inside that runtime *between* polls, which is the state such
/// guards exist to scope.
///
/// ```
/// use std::cell::Cell;
/// use std::rc::Rc;
///
/// use aimer_venus::{PollContext, Venus};
///
/// /// Stands in for a real runtime's enter-guard.
/// struct MarkThread(Rc<Cell<bool>>);
///
/// impl PollContext for MarkThread {
///     fn enter(&self, poll: &mut dyn FnMut()) {
///         self.0.set(true);
///         poll();
///         self.0.set(false);
///     }
/// }
///
/// let inside = Rc::new(Cell::new(false));
/// let venus = Venus::new();
/// venus.set_poll_context(MarkThread(inside.clone()));
///
/// let observed = Rc::new(Cell::new(false));
/// let seen = observed.clone();
/// let marked = inside.clone();
/// venus.spawn(async move { seen.set(marked.get()) });
/// venus.run_microtasks();
///
/// assert!(observed.get(), "the task was polled inside the context");
/// assert!(!inside.get(), "and the context was left again");
/// ```
pub trait PollContext {
    /// Runs `poll` with the host's runtime entered, and no longer.
    fn enter(&self, poll: &mut dyn FnMut());

    /// Creates a cancellable host timer starting now.
    ///
    /// The timer must wake its waiter when it expires, including while the
    /// event loop is parked. Dropping it must release its registration. Hosts
    /// without timers may keep the default `None`; [`Venus::timeout`](crate::Venus::timeout)
    /// then reports [`TimeoutError::TimerUnavailable`](crate::TimeoutError::TimerUnavailable).
    /// This method may be called outside [`Self::enter`], so implementations
    /// must enter their runtime when constructing a runtime-backed timer.
    fn sleep(&self, _duration: Duration) -> Option<Pin<Box<dyn Future<Output = ()>>>> {
        None
    }

    /// Names who a task being spawned right now belongs to, or `0` for nobody.
    ///
    /// A host with several owners of tasks — windows sharing one scheduler —
    /// answers with the owner that is active at the call. Venus remembers the
    /// answer with the task and hands it back to [`Self::enter_tagged`] on every
    /// poll, wherever and whenever that poll happens. A task nobody owns costs
    /// nothing extra.
    fn task_tag(&self) -> u64 {
        0
    }

    /// Runs one poll of a task whose [`Self::task_tag`] was `tag`.
    ///
    /// Called from inside [`Self::enter`], never for tag `0`. The default does
    /// nothing but poll.
    fn enter_tagged(&self, _tag: u64, poll: &mut dyn FnMut()) {
        poll();
    }
}

/// A task that is polled on behalf of the owner it was spawned for.
pub(crate) struct Tagged<F> {
    tag: u64,
    host: Rc<dyn PollContext>,
    future: F,
}

impl<F> Tagged<F> {
    pub(crate) fn new(tag: u64, host: Rc<dyn PollContext>, future: F) -> Self {
        Self { tag, host, future }
    }
}

impl<F: Future<Output = ()>> Future for Tagged<F> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        // SAFETY: `future` is the only field projected as pinned, it is never
        // moved out of `self`, and `Tagged` has no `Drop` impl and no other way
        // to reach it, so the pin guarantee of `self` carries over to it.
        let this = unsafe { self.get_unchecked_mut() };
        let mut future = unsafe { Pin::new_unchecked(&mut this.future) };
        let mut result = Poll::Pending;
        this.host
            .enter_tagged(this.tag, &mut || result = future.as_mut().poll(cx));
        result
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;
    use crate::{FrameBudget, LocalScheduler, Phase, ScopeId, Venus, yield_now};
    use std::time::Duration;

    /// Counts the polls it wrapped and publishes whether one is in progress.
    #[derive(Default)]
    struct Counting {
        entries: Rc<Cell<usize>>,
        depth: Rc<Cell<usize>>,
    }

    impl PollContext for Counting {
        fn enter(&self, poll: &mut dyn FnMut()) {
            self.entries.set(self.entries.get() + 1);
            self.depth.set(self.depth.get() + 1);
            poll();
            self.depth.set(self.depth.get() - 1);
        }
    }

    /// The whole point: a future polled by Venus sees the host's runtime, so a
    /// resource it builds on its first poll finds something to register with.
    #[test]
    fn every_poll_happens_inside_the_installed_context() {
        let scheduler = LocalScheduler::new();
        let context = Counting::default();
        let entries = context.entries.clone();
        let depth = context.depth.clone();
        scheduler.set_poll_context(context);

        let observed = Rc::new(Cell::new(0));
        let seen = observed.clone();
        let inside = depth.clone();
        scheduler.spawn(async move {
            if inside.get() > 0 {
                seen.set(seen.get() + 1);
            }
            yield_now().await;
            if inside.get() > 0 {
                seen.set(seen.get() + 1);
            }
        });

        scheduler.run_microtasks();

        assert_eq!(entries.get(), 2, "one entry per poll, not one per task");
        assert_eq!(observed.get(), 2, "both polls ran inside the context");
    }

    /// A runtime guard held across an `await` would mark the thread as being
    /// inside the runtime while the UI is drawing — the exact state the guard
    /// is scoped to prevent.
    #[test]
    fn the_context_is_left_between_polls() {
        let scheduler = LocalScheduler::new();
        let context = Counting::default();
        let depth = context.depth.clone();
        scheduler.set_poll_context(context);

        scheduler.spawn(async {
            yield_now().await;
        });

        scheduler.run_microtasks();

        assert_eq!(depth.get(), 0);
    }

    /// The hook belongs to the scheduler, not to one phase: a frame tick and an
    /// idle task are polled through it too.
    #[test]
    fn the_context_wraps_every_phase() {
        let scheduler = LocalScheduler::new();
        let context = Counting::default();
        let entries = context.entries.clone();
        scheduler.set_poll_context(context);

        scheduler.spawn_in_phase(Phase::Frame, ScopeId::ROOT, async {});
        scheduler.spawn_in_phase(Phase::Idle, ScopeId::ROOT, async {});

        scheduler.run_frame_tasks();
        scheduler.run_idle(&FrameBudget::from_now(Duration::from_millis(4)));

        assert_eq!(entries.get(), 2);
    }

    /// Nothing installed is the common case — a widget under test, a browser
    /// with no runtime to be inside of — and it must still poll.
    #[test]
    fn a_scheduler_without_a_context_polls_directly() {
        let scheduler = LocalScheduler::new();
        let ran = Rc::new(Cell::new(false));

        let flag = ran.clone();
        scheduler.spawn(async move { flag.set(true) });
        scheduler.run_microtasks();

        assert!(ran.get());
    }

    /// A host tearing its runtime down takes the context with it, or the next
    /// poll would enter a handle to a runtime that no longer exists.
    #[test]
    fn a_cleared_context_stops_wrapping_polls() {
        let scheduler = LocalScheduler::new();
        let context = Counting::default();
        let entries = context.entries.clone();
        scheduler.set_poll_context(context);

        scheduler.spawn(async {});
        scheduler.run_microtasks();
        assert_eq!(entries.get(), 1);

        scheduler.clear_poll_context();
        scheduler.spawn(async {});
        scheduler.run_microtasks();

        assert_eq!(entries.get(), 1, "the cleared context was not entered");
    }

    /// The host installs it through the runtime it already holds, not through
    /// the scheduler it never touches.
    #[test]
    fn a_runtime_forwards_the_context_to_its_scheduler() {
        let venus = Venus::new();
        let context = Counting::default();
        let entries = context.entries.clone();
        venus.set_poll_context(context);

        venus.spawn(async {});
        venus.run_microtasks();

        assert_eq!(entries.get(), 1);
    }

    /// Owns tasks by a number the test sets, and records which owner each poll
    /// ran for.
    struct Owners {
        current: Rc<Cell<u64>>,
        polled_for: Rc<std::cell::RefCell<Vec<u64>>>,
        inside: Rc<Cell<u64>>,
    }

    impl PollContext for Owners {
        fn enter(&self, poll: &mut dyn FnMut()) {
            poll();
        }

        fn task_tag(&self) -> u64 {
            self.current.get()
        }

        fn enter_tagged(&self, tag: u64, poll: &mut dyn FnMut()) {
            self.polled_for.borrow_mut().push(tag);
            self.inside.set(tag);
            poll();
            self.inside.set(0);
        }
    }

    fn owned_venus() -> (Rc<Venus>, Rc<Cell<u64>>, Rc<std::cell::RefCell<Vec<u64>>>, Rc<Cell<u64>>) {
        let current = Rc::new(Cell::new(0));
        let polled_for = Rc::new(std::cell::RefCell::new(Vec::new()));
        let inside = Rc::new(Cell::new(0));
        let venus = Venus::new();
        venus.set_poll_context(Owners {
            current: current.clone(),
            polled_for: polled_for.clone(),
            inside: inside.clone(),
        });
        (venus, current, polled_for, inside)
    }

    #[test]
    fn a_task_is_polled_for_the_owner_that_was_active_when_it_was_spawned() {
        let (venus, current, polled_for, inside) = owned_venus();
        let seen = Rc::new(Cell::new(0));

        current.set(7);
        let observed = seen.clone();
        let during = inside.clone();
        venus.spawn(async move { observed.set(during.get()) });
        // Another owner is active by the time the task runs.
        current.set(9);
        venus.run_microtasks();

        assert_eq!(seen.get(), 7, "the task ran for the wrong owner");
        assert_eq!(*polled_for.borrow(), [7]);
    }

    #[test]
    fn every_poll_of_a_task_is_for_its_owner() {
        let (venus, current, polled_for, _inside) = owned_venus();

        current.set(3);
        venus.spawn(async {
            crate::yield_now().await;
            crate::yield_now().await;
        });
        current.set(0);
        for _ in 0..5 {
            venus.run_microtasks();
        }

        assert!(polled_for.borrow().len() >= 2);
        assert!(polled_for.borrow().iter().all(|tag| *tag == 3));
    }

    #[test]
    fn a_task_nobody_owns_is_not_tagged() {
        let (venus, current, polled_for, _inside) = owned_venus();

        current.set(0);
        venus.spawn(async {});
        venus.run_microtasks();

        assert!(polled_for.borrow().is_empty());
    }
}
