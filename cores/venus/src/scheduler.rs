//! The UI-thread task engine: one slab of futures, one ready queue per frame
//! phase.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;
use std::task::Context;

#[cfg(feature = "venus-debug")]
use std::time::Duration;

use crate::budget::{self, FrameBudget};

thread_local! {
    /// How many task polls are in progress on this thread. A poll can start
    /// another (a task that drains the microtask queue), so this counts depth.
    static POLLING: Cell<u32> = const { Cell::new(0) };
}

/// Whether a task is being polled on this thread right now.
///
/// Code that runs from a task, such as the state update a timer or a finished
/// request makes, can ask this to learn that it was reached through the
/// scheduler rather than through input or a direct call.
#[inline]
pub fn is_polling_task() -> bool {
    POLLING.with(|depth| depth.get() > 0)
}

/// Marks a task poll as in progress until dropped.
struct PollingGuard;

impl PollingGuard {
    #[inline]
    fn enter() -> Self {
        POLLING.with(|depth| depth.set(depth.get() + 1));
        Self
    }
}

impl Drop for PollingGuard {
    #[inline]
    fn drop(&mut self) {
        POLLING.with(|depth| depth.set(depth.get() - 1));
    }
}
use crate::poll_context::{PollContext, Tagged};
use crate::task::slab::TaskSlab;
use crate::task::waker::{Notifier, WakeQueue, waker_for};
use crate::task::{Phase, ScopeId, TaskId, TaskScope};

#[cfg(feature = "venus-debug")]
use web_time::Instant;

/// How many microtasks one drain will run before deciding it is looping.
///
/// A microtask that re-wakes itself unconditionally would otherwise hang the UI
/// thread with no diagnostic at all. The limit is far above any legitimate
/// frame — ten thousand `set_state` microtasks is already pathological — so
/// reaching it is a bug, and debug builds say so.
const MICROTASK_DRAIN_LIMIT: usize = 100_000;

#[cfg(feature = "venus-debug")]
fn slow_microtask_warning(task: TaskId, elapsed: Duration) -> Option<String> {
    if elapsed <= crate::MICROTASK_BUDGET_WARNING {
        return None;
    }

    Some(format!(
        "aimer_venus: slow microtask {task:?} took {elapsed:?} (threshold {:?})",
        crate::MICROTASK_BUDGET_WARNING,
    ))
}

struct Inner {
    tasks: TaskSlab,
    ready: [VecDeque<TaskId>; Phase::COUNT],
    next_scope: u64,
    /// Reused across drains so a steady stream of wakes allocates nothing.
    woken: Vec<TaskId>,
}

/// A single-threaded, frame-aligned executor for non-`Send` futures.
///
/// Two things distinguish it from a general-purpose runtime, and both come from
/// what a UI needs rather than from what a server needs:
///
/// - **Tasks are not `Send`.** They are polled on the thread that created the
///   scheduler and nowhere else, so a task may hold an `Rc` from the element
///   tree — a `StateUpdater`, a controller, a widget handle — across an `await`.
/// - **Tasks run in a phase, not "as soon as possible".** A microtask lands
///   before the frame's build; an idle task runs only while the frame has
///   measured room. "Eventually" is not a schedule a UI can be built on.
///
/// The scheduler owns no threads and no timers. Something has to drive it — see
/// [`crate::Venus`] for the frame-shaped wrapper an event loop actually calls,
/// and [`Self::offload`](crate::Venus::offload) for the one place work leaves
/// this thread.
///
/// # Examples
///
/// ```
/// use std::cell::Cell;
/// use std::rc::Rc;
///
/// use aimer_venus::LocalScheduler;
///
/// let scheduler = LocalScheduler::new();
/// let calls = Rc::new(Cell::new(0));
///
/// let counted = calls.clone();
/// scheduler.spawn(async move { counted.set(counted.get() + 1) });
///
/// assert_eq!(scheduler.run_microtasks(), 1);
/// assert_eq!(calls.get(), 1);
/// ```
pub struct LocalScheduler {
    inner: RefCell<Inner>,
    wakes: Arc<WakeQueue>,
    /// The host runtime every poll is wrapped in, if the host installed one.
    ///
    /// Kept apart from [`Inner`] because it is read on the poll path while the
    /// task state is lent out, and because a task is allowed to install one
    /// while it runs.
    poll_context: RefCell<Option<Rc<dyn PollContext>>>,
}

impl LocalScheduler {
    /// Creates a scheduler owned by the calling thread.
    ///
    /// Returned in an [`Rc`] because a [`TaskScope`] holds a weak reference back
    /// to cancel through, and because handing the same scheduler to every
    /// element is the entire point.
    pub fn new() -> Rc<Self> {
        Rc::new(Self {
            inner: RefCell::new(Inner {
                tasks: TaskSlab::new(),
                ready: [VecDeque::new(), VecDeque::new(), VecDeque::new()],
                next_scope: 1,
                woken: Vec::new(),
            }),
            wakes: WakeQueue::new(),
            poll_context: RefCell::new(None),
        })
    }

    /// Spawns `future` as a microtask in the root scope.
    ///
    /// The default for a handler that resolved something and wants to mutate
    /// state: it will run before the next build phase.
    #[inline]
    pub fn spawn(&self, future: impl Future<Output = ()> + 'static) -> TaskId {
        self.spawn_in_phase(Phase::Microtask, ScopeId::ROOT, future)
    }

    /// Spawns `future` as a microtask owned by `scope`.
    #[inline]
    pub fn spawn_in(&self, scope: ScopeId, future: impl Future<Output = ()> + 'static) -> TaskId {
        self.spawn_in_phase(Phase::Microtask, scope, future)
    }

    /// Spawns `future` into `phase`, owned by `scope`.
    ///
    /// The task is ready immediately: it is polled the first time its phase
    /// runs, not on some later frame.
    pub fn spawn_in_phase(
        &self,
        phase: Phase,
        scope: ScopeId,
        future: impl Future<Output = ()> + 'static,
    ) -> TaskId {
        // Asked before `inner` is borrowed: the host reads its own state.
        let owner = self
            .poll_context
            .borrow()
            .clone()
            .map(|host| (host.task_tag(), host))
            .filter(|(tag, _)| *tag != 0);
        let future: crate::task::LocalFuture = match owner {
            Some((tag, host)) => Box::pin(Tagged::new(tag, host, future)),
            None => Box::pin(future),
        };
        let mut inner = self.inner.borrow_mut();
        let id = inner.tasks.reserve();
        let waker = waker_for(id, &self.wakes);
        inner.tasks.occupy(id, future, waker, phase, scope);
        inner.ready[phase.index()].push_back(id);
        id
    }

    /// Creates a scope whose tasks are cancelled when it is dropped.
    pub fn scope(self: &Rc<Self>) -> TaskScope {
        let id = {
            let mut inner = self.inner.borrow_mut();
            let id = inner.next_scope;
            inner.next_scope = id.wrapping_add(1);
            ScopeId::new(id)
        };
        TaskScope::new(id, self)
    }

    /// Cancels one task, dropping its future.
    ///
    /// Cancelling a task that is currently being polled takes effect when it
    /// next yields: its future has nowhere to return to, so it is dropped then.
    #[inline]
    pub fn abort(&self, task: TaskId) -> bool {
        self.inner.borrow_mut().tasks.remove(task)
    }

    /// Cancels every task in `scope`, returning how many there were.
    #[inline]
    pub fn cancel_scope(&self, scope: ScopeId) -> usize {
        self.inner.borrow_mut().tasks.remove_scope(scope)
    }

    /// Whether `task` is still alive.
    #[inline]
    pub fn is_running(&self, task: TaskId) -> bool {
        self.inner.borrow().tasks.contains(task)
    }

    /// How many tasks are alive, across every phase.
    #[inline]
    pub fn task_count(&self) -> usize {
        self.inner.borrow().tasks.len()
    }

    /// Whether any phase has work waiting.
    ///
    /// An event loop reads this to decide whether it may wait for OS events or
    /// has to come round again immediately.
    pub fn has_ready_work(&self) -> bool {
        if self.wakes.has_pending() {
            return true;
        }
        let inner = self.inner.borrow();
        inner.ready.iter().any(|queue| !queue.is_empty())
    }

    /// Installs the callback that wakes a parked event loop.
    ///
    /// On native platforms, only wakes raised off the UI thread call it — a
    /// worker finishing while the loop sleeps. Wakes raised on the UI thread
    /// never do: the loop is demonstrably awake, and pinging it per `await`
    /// would be a cost on the hottest path there is. WebAssembly is the
    /// exception: a JavaScript promise may resume a same-thread task while
    /// the browser event loop is parked, so those wakes notify too.
    #[inline]
    pub fn set_notifier(&self, notifier: impl Fn() + Send + Sync + 'static) {
        self.wakes.set_notifier(Box::new(notifier) as Notifier);
    }

    /// Installs the runtime every task is polled inside from now on.
    ///
    /// See [`PollContext`] for what this buys: futures from `reqwest`, `tokio`
    /// and every other ecosystem that builds its resources on the first poll
    /// and looks its runtime up in a thread-local. Replaces whatever was
    /// installed before, because a thread has one host runtime rather than a
    /// stack of them.
    #[inline]
    pub fn set_poll_context(&self, context: impl PollContext + 'static) {
        *self.poll_context.borrow_mut() = Some(Rc::new(context) as Rc<dyn PollContext>);
    }

    /// Removes the installed runtime, returning polls to bare.
    ///
    /// A host shutting its runtime down says so here: a context that outlives
    /// the runtime it enters is a handle to something that no longer exists.
    #[inline]
    pub fn clear_poll_context(&self) {
        *self.poll_context.borrow_mut() = None;
    }

    pub(crate) fn sleep(
        &self,
        duration: std::time::Duration,
    ) -> Option<std::pin::Pin<Box<dyn Future<Output = ()>>>> {
        let host = self.poll_context.borrow().clone()?;
        host.sleep(duration)
    }

    /// Runs microtasks until none are left, returning how many ran.
    ///
    /// Drained to exhaustion rather than budgeted, because budgeting this phase
    /// is the one-frame-latency bug: a microtask exists precisely because its
    /// effect has to be visible to *this* frame's build. The contract that keeps
    /// that affordable is on the task, not on the scheduler — a microtask may
    /// mutate state, it may not do work.
    pub fn run_microtasks(&self) -> usize {
        let mut polled = 0;

        loop {
            self.dispatch_wakes();
            let Some(task) = self.take_ready(Phase::Microtask) else {
                break;
            };

            self.poll_task(task, Phase::Microtask);
            polled += 1;

            if polled >= MICROTASK_DRAIN_LIMIT {
                debug_assert!(
                    false,
                    "a microtask is re-waking itself: {polled} polls in one drain"
                );
                break;
            }
        }

        polled
    }

    /// Runs each frame task once, returning how many ran.
    ///
    /// The queue is sampled before the pass, so a task that re-arms itself while
    /// running waits for the next frame instead of spinning inside this one.
    pub fn run_frame_tasks(&self) -> usize {
        self.dispatch_wakes();

        let mut remaining = self.inner.borrow().ready[Phase::Frame.index()].len();
        let mut polled = 0;

        while remaining > 0 {
            let Some(task) = self.take_ready(Phase::Frame) else {
                break;
            };
            self.poll_task(task, Phase::Frame);
            polled += 1;
            remaining -= 1;
        }

        polled
    }

    /// Runs idle tasks while `budget` has room, returning how many polls it got
    /// through.
    ///
    /// Nothing runs without measured room, and the budget is published for the
    /// duration so a task can slice itself with
    /// [`crate::yield_if_over_budget`]. A task still holding the thread when the
    /// budget runs out is *not* interrupted — cooperative scheduling cannot —
    /// which is why unsliceable work belongs on
    /// [`Venus::offload`](crate::Venus::offload).
    pub fn run_idle(&self, budget: &FrameBudget) -> usize {
        budget::with_active(budget, || {
            let mut polled = 0;

            while budget.has_room() {
                self.dispatch_wakes();
                let Some(task) = self.take_ready(Phase::Idle) else {
                    break;
                };

                self.poll_task(task, Phase::Idle);
                polled += 1;
            }

            polled
        })
    }

    /// Moves published wakes into the ready queue of each woken task's phase.
    ///
    /// Gated on the wake flags before anything is borrowed: this runs before
    /// every poll of a microtask drain, and in the overwhelmingly common "no
    /// new wakes" case it must cost a flag read, not a borrow and a buffer
    /// swap.
    fn dispatch_wakes(&self) {
        if !self.wakes.has_pending() {
            return;
        }

        let mut borrow = self.inner.borrow_mut();
        let Inner {
            tasks,
            ready,
            woken,
            ..
        } = &mut *borrow;

        let mut buffer = std::mem::take(woken);
        self.wakes.drain_into(&mut buffer);

        for id in buffer.drain(..) {
            let Some(task) = tasks.get_mut(id) else {
                continue;
            };
            if task.queued {
                continue;
            }
            task.queued = true;
            let phase = task.phase;
            ready[phase.index()].push_back(id);
        }

        *woken = buffer;
    }

    /// Pops the next ready task of `phase`, clearing its queued flag.
    ///
    /// Stale ids — a task cancelled after it was queued — are skipped here, so
    /// cancellation never has to walk the ready queues.
    fn take_ready(&self, phase: Phase) -> Option<TaskId> {
        let mut borrow = self.inner.borrow_mut();
        let Inner { tasks, ready, .. } = &mut *borrow;

        while let Some(id) = ready[phase.index()].pop_front() {
            if let Some(task) = tasks.get_mut(id) {
                task.queued = false;
                return Some(id);
            }
        }

        None
    }

    /// Polls one task with the scheduler's interior state un-borrowed.
    ///
    /// The future is lent out for the duration, which is what lets a running
    /// task spawn another task, cancel a scope, or drop its own scope without
    /// panicking on a re-entrant borrow.
    fn poll_task(&self, task: TaskId, phase: Phase) {
        let Some((mut future, waker)) = self.inner.borrow_mut().tasks.lend(task) else {
            return;
        };

        #[cfg(not(feature = "venus-debug"))]
        let _ = phase;

        #[cfg(feature = "venus-debug")]
        let started = (phase == Phase::Microtask).then(Instant::now);

        let mut context = Context::from_waker(&waker);

        // Cloned rather than borrowed across the poll: the task about to run may
        // install a context of its own, and a live borrow is the one thing that
        // would turn that into a panic. `None` — every test, and the whole
        // browser — clones to nothing at all.
        let host = self.poll_context.borrow().clone();
        let mut finished = false;
        let _polling = PollingGuard::enter();
        {
            let mut poll_once = || finished = future.as_mut().poll(&mut context).is_ready();
            match host {
                Some(host) => host.enter(&mut poll_once),
                None => poll_once(),
            }
        }

        #[cfg(feature = "venus-debug")]
        if let Some(started) = started
            && let Some(warning) = slow_microtask_warning(task, started.elapsed())
        {
            eprintln!("{warning}");
        }

        let mut inner = self.inner.borrow_mut();
        if finished {
            inner.tasks.remove(task);
            return;
        }

        // A `false` here means the task was cancelled while it was running, so
        // the future has nowhere to go back to and is dropped with this scope.
        let _ = inner.tasks.restore(task, future, waker);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::task::Poll;
    use std::time::Duration;

    use super::*;
    use crate::yield_now;

    #[test]
    fn a_task_only_runs_in_the_phase_it_was_spawned_into() {
        let scheduler = LocalScheduler::new();
        let ran = Rc::new(Cell::new(0));

        let counted = ran.clone();
        scheduler.spawn_in_phase(Phase::Frame, ScopeId::ROOT, async move {
            counted.set(counted.get() + 1);
        });

        assert_eq!(scheduler.run_microtasks(), 0);
        assert_eq!(
            scheduler.run_idle(&FrameBudget::from_now(Duration::from_millis(4))),
            0
        );
        assert_eq!(ran.get(), 0);

        assert_eq!(scheduler.run_frame_tasks(), 1);
        assert_eq!(ran.get(), 1);
    }

    // A frame task that re-arms itself must not monopolise the frame it is
    // already running in — that is the difference between "once per frame" and
    // "as often as it asks".
    #[test]
    fn a_frame_task_that_rearms_itself_waits_for_the_next_frame() {
        let scheduler = LocalScheduler::new();
        let ticks = Rc::new(Cell::new(0));

        let counted = ticks.clone();
        scheduler.spawn_in_phase(Phase::Frame, ScopeId::ROOT, async move {
            loop {
                counted.set(counted.get() + 1);
                yield_now().await;
            }
        });

        assert_eq!(scheduler.run_frame_tasks(), 1);
        assert_eq!(ticks.get(), 1);

        assert_eq!(scheduler.run_frame_tasks(), 1);
        assert_eq!(ticks.get(), 2);
    }

    // Spawning from inside a task is the ordinary case — a handler resolving a
    // request and queueing the state mutation — and it must not re-enter the
    // scheduler's borrow.
    #[test]
    fn a_task_may_spawn_another_task_while_it_runs() {
        let scheduler = LocalScheduler::new();
        let ran = Rc::new(Cell::new(0));

        let inner_scheduler = scheduler.clone();
        let counted = ran.clone();
        scheduler.spawn(async move {
            let nested = counted.clone();
            inner_scheduler.spawn(async move { nested.set(nested.get() + 1) });
            counted.set(counted.get() + 1);
        });

        scheduler.run_microtasks();

        assert_eq!(ran.get(), 2);
        assert_eq!(scheduler.task_count(), 0);
    }

    #[test]
    fn a_task_may_drop_its_own_scope_while_it_runs() {
        let scheduler = LocalScheduler::new();
        let reached_the_end = Rc::new(Cell::new(false));

        let scope = scheduler.scope();
        let scope_id = scope.id();
        let holder = Rc::new(RefCell::new(Some(scope)));

        let owned = holder.clone();
        let flag = reached_the_end.clone();
        scheduler.spawn_in(scope_id, async move {
            owned.borrow_mut().take();
            yield_now().await;
            flag.set(true);
        });

        scheduler.run_microtasks();

        assert!(!reached_the_end.get(), "the cancelled task must not resume");
        assert_eq!(scheduler.task_count(), 0);
    }

    #[test]
    fn aborting_a_queued_task_leaves_no_stale_poll_behind() {
        let scheduler = LocalScheduler::new();
        let ran = Rc::new(Cell::new(false));

        let flag = ran.clone();
        let task = scheduler.spawn(async move { flag.set(true) });

        assert!(scheduler.is_running(task));
        assert!(scheduler.abort(task));

        assert_eq!(scheduler.run_microtasks(), 0);
        assert!(!ran.get());
        assert!(!scheduler.is_running(task));
    }

    // Several wakes arriving before the task next runs are one wake: without the
    // queued flag a hover handler woken per pointer event would be polled once
    // per event.
    #[test]
    fn repeated_wakes_before_a_poll_collapse_into_one() {
        let scheduler = LocalScheduler::new();
        let polls = Rc::new(Cell::new(0));

        // Parks once, handing its waker out so the test can wake it by hand —
        // `yield_now` would re-queue itself before the extra wakes arrived.
        struct Capture {
            slot: Rc<RefCell<Option<std::task::Waker>>>,
            parked: bool,
        }

        impl Future for Capture {
            type Output = ();

            fn poll(self: std::pin::Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                let this = self.get_mut();
                if this.parked {
                    return Poll::Ready(());
                }

                this.parked = true;
                *this.slot.borrow_mut() = Some(cx.waker().clone());
                Poll::Pending
            }
        }

        let counted = polls.clone();
        let waker_slot: Rc<RefCell<Option<std::task::Waker>>> = Rc::new(RefCell::new(None));
        let stored = waker_slot.clone();

        scheduler.spawn(async move {
            counted.set(counted.get() + 1);
            Capture {
                slot: stored,
                parked: false,
            }
            .await;
            counted.set(counted.get() + 1);
        });

        assert_eq!(scheduler.run_microtasks(), 1);
        assert_eq!(polls.get(), 1);

        let waker = waker_slot.borrow().clone().expect("a captured waker");
        waker.wake_by_ref();
        waker.wake_by_ref();
        waker.wake_by_ref();

        assert_eq!(scheduler.run_microtasks(), 1, "three wakes, one poll");
        assert_eq!(polls.get(), 2);
    }

    #[test]
    fn an_idle_task_is_not_started_without_room() {
        let scheduler = LocalScheduler::new();
        let ran = Rc::new(Cell::new(false));

        let flag = ran.clone();
        scheduler.spawn_in_phase(Phase::Idle, ScopeId::ROOT, async move { flag.set(true) });

        assert_eq!(scheduler.run_idle(&FrameBudget::exhausted()), 0);
        assert!(!ran.get());
        assert_eq!(scheduler.task_count(), 1);

        scheduler.run_idle(&FrameBudget::from_now(Duration::from_millis(4)));
        assert!(ran.get());
    }

    #[test]
    fn the_scheduler_reports_whether_the_loop_may_sleep() {
        let scheduler = LocalScheduler::new();
        assert!(!scheduler.has_ready_work());

        scheduler.spawn(async {});
        assert!(scheduler.has_ready_work());

        scheduler.run_microtasks();
        assert!(!scheduler.has_ready_work());
    }

    #[cfg(feature = "venus-debug")]
    #[test]
    fn a_microtask_warning_starts_strictly_above_the_threshold() {
        let task = TaskId::new(7, 2);

        assert!(super::slow_microtask_warning(task, crate::MICROTASK_BUDGET_WARNING).is_none());

        let warning = super::slow_microtask_warning(
            task,
            crate::MICROTASK_BUDGET_WARNING + Duration::from_nanos(1),
        )
        .expect("a poll above the warning threshold");
        assert!(warning.contains("slow microtask"));
        assert!(warning.contains("TaskId"));
    }
}
