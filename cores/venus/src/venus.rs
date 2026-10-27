//! The frame-shaped runtime an event loop actually talks to.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use crate::budget::{FrameBudget, FrameGovernor};
#[cfg(not(target_arch = "wasm32"))]
use crate::offload::{OffloadPool, Offloaded};
use crate::poll_context::PollContext;
use crate::scheduler::LocalScheduler;
use crate::task::{Phase, ScopeId, TaskId, TaskScope};

aimer_tls::thread_local! {
    /// The runtime installed on this thread, if any.
    ///
    /// This is what deletes the spawner parameter the framework used to thread
    /// through the entire element tree: a callback deep in a paragraph can reach
    /// the runtime without anyone having handed it down. Thread-local rather
    /// than global because a runtime belongs to one UI thread's frames, and a
    /// second window on a second thread is a second runtime.
    static CURRENT: RefCell<Option<Rc<Venus>>> = const { RefCell::new(None) };
}

/// Aimer's UI-thread runtime.
///
/// Venus is a scheduler, not a replacement for Tokio. It owns the *ordering* of
/// asynchronous work relative to a frame, and one small pool for work that must
/// leave the thread; real I/O runtimes remain welcome alongside it.
///
/// What it gives a GUI that a general-purpose runtime cannot:
///
/// - **Non-`Send` tasks.** A handler may `await` while holding a `StateUpdater`,
///   a controller, or any other `Rc` from the element tree.
/// - **Frame phases.** A resolved future's `set_state` lands before *this*
///   frame's build instead of a frame later.
/// - **A budget.** Background work runs only in the slack of a frame, and stops
///   when the slack runs out.
/// - **Structural cancellation.** An unmounting element drops a [`TaskScope`]
///   and its tasks are gone.
///
/// # Examples
///
/// Driving one frame, the way an event loop does:
///
/// ```
/// use std::cell::Cell;
/// use std::rc::Rc;
///
/// use aimer_venus::Venus;
///
/// let venus = Venus::new();
/// let state = Rc::new(Cell::new(0));
/// let built_with = Rc::new(Cell::new(0));
///
/// let mutated = state.clone();
/// venus.spawn(async move { mutated.set(9) });
///
/// let read = state.clone();
/// let observed = built_with.clone();
/// venus.drive_frame(|| observed.set(read.get()));
///
/// assert_eq!(built_with.get(), 9, "the effect landed before the build");
/// ```
pub struct Venus {
    scheduler: Rc<LocalScheduler>,
    governor: RefCell<FrameGovernor>,
    #[cfg(not(target_arch = "wasm32"))]
    pool: OffloadPool,
}

impl Venus {
    /// Creates a runtime for a 60 Hz display, owned by the calling thread.
    ///
    /// The worker pool is created eagerly on native targets: spawning threads
    /// lazily would put the cost inside whichever frame first needed them, which
    /// is precisely the frame that is already busy.
    pub fn new() -> Rc<Self> {
        Self::with_governor(FrameGovernor::default())
    }

    /// Creates a runtime for a display refreshing `hz` times per second.
    #[inline]
    pub fn for_refresh_rate(hz: f32) -> Rc<Self> {
        Self::with_governor(FrameGovernor::for_refresh_rate(hz))
    }

    /// Creates a runtime driven by `governor`.
    pub fn with_governor(governor: FrameGovernor) -> Rc<Self> {
        Rc::new(Self {
            scheduler: LocalScheduler::new(),
            governor: RefCell::new(governor),
            #[cfg(not(target_arch = "wasm32"))]
            pool: OffloadPool::with_default_threads(),
        })
    }

    /// The task engine underneath, for code that schedules by phase explicitly.
    #[inline]
    pub fn scheduler(&self) -> &Rc<LocalScheduler> {
        &self.scheduler
    }

    /// Spawns `future` as a microtask: it runs before the next build phase.
    #[inline]
    pub fn spawn(&self, future: impl Future<Output = ()> + 'static) -> TaskId {
        self.scheduler.spawn(future)
    }

    /// Limits how long `future` may remain pending, cancelling it on expiry.
    ///
    /// The deadline starts when this method is called, using the timer supplied
    /// by [`PollContext::sleep`]. Await the result inside a Venus task, or spawn
    /// it to run in the microtask phase. This wrapper alone does not spawn work.
    /// The future and its output need not be [`Send`].
    ///
    /// A ready future wins when both it and the timer are ready. A timeout is
    /// cooperative: it cannot interrupt blocking work inside a single poll.
    /// The losing future or timer is dropped before this resolves. Dropping
    /// the wrapper also cancels both.
    ///
    /// # Errors
    ///
    /// Returns [`TimeoutError::Elapsed`](crate::TimeoutError::Elapsed) on expiry,
    /// or [`TimeoutError::TimerUnavailable`](crate::TimeoutError::TimerUnavailable)
    /// without polling `future` if the host does not supply timers.
    #[inline]
    pub fn timeout<F: Future>(
        &self,
        duration: Duration,
        future: F,
    ) -> impl Future<Output = Result<F::Output, crate::TimeoutError>> + use<F> {
        crate::timeout::run(future, self.scheduler.sleep(duration))
    }

    /// Spawns `future` as a microtask owned by `scope`.
    #[inline]
    pub fn spawn_in(&self, scope: ScopeId, future: impl Future<Output = ()> + 'static) -> TaskId {
        self.scheduler.spawn_in(scope, future)
    }

    /// Spawns `future` as a frame task: it runs once per frame.
    #[inline]
    pub fn spawn_frame(&self, future: impl Future<Output = ()> + 'static) -> TaskId {
        self.scheduler
            .spawn_in_phase(Phase::Frame, ScopeId::ROOT, future)
    }

    /// Spawns `future` as an idle task: it runs in the slack of a frame.
    #[inline]
    pub fn spawn_idle(&self, future: impl Future<Output = ()> + 'static) -> TaskId {
        self.scheduler
            .spawn_in_phase(Phase::Idle, ScopeId::ROOT, future)
    }

    /// Spawns `future` into `phase`, owned by `scope`.
    #[inline]
    pub fn spawn_in_phase(
        &self,
        phase: Phase,
        scope: ScopeId,
        future: impl Future<Output = ()> + 'static,
    ) -> TaskId {
        self.scheduler.spawn_in_phase(phase, scope, future)
    }

    /// Creates a scope whose tasks are cancelled when it is dropped.
    #[inline]
    pub fn scope(&self) -> TaskScope {
        self.scheduler.scope()
    }

    /// Cancels one task.
    #[inline]
    pub fn abort(&self, task: TaskId) -> bool {
        self.scheduler.abort(task)
    }

    /// Whether `task` is still alive.
    #[inline]
    pub fn is_running(&self, task: TaskId) -> bool {
        self.scheduler.is_running(task)
    }

    /// How many tasks are alive, across every phase.
    #[inline]
    pub fn task_count(&self) -> usize {
        self.scheduler.task_count()
    }

    /// Whether any phase has work waiting, i.e. whether the loop may sleep.
    #[inline]
    pub fn has_ready_work(&self) -> bool {
        self.scheduler.has_ready_work()
    }

    /// Installs the callback that wakes a parked event loop.
    #[inline]
    pub fn set_notifier(&self, notifier: impl Fn() + Send + Sync + 'static) {
        self.scheduler.set_notifier(notifier);
    }

    /// Installs the host runtime every task is polled inside.
    ///
    /// This is what lets a future from another ecosystem — `reqwest`,
    /// `tokio::fs`, a `sleep` — be spawned anywhere in the UI: see
    /// [`PollContext`]. Venus itself names no runtime; the host installs the
    /// adapter once, next to [`install`](Self::install).
    #[inline]
    pub fn set_poll_context(&self, context: impl PollContext + 'static) {
        self.scheduler.set_poll_context(context);
    }

    /// Removes the installed host runtime, returning polls to bare.
    #[inline]
    pub fn clear_poll_context(&self) {
        self.scheduler.clear_poll_context();
    }

    /// Runs microtasks until none are left, returning how many ran.
    #[inline]
    pub fn run_microtasks(&self) -> usize {
        self.scheduler.run_microtasks()
    }

    /// Runs each frame task once, returning how many ran.
    #[inline]
    pub fn run_frame_tasks(&self) -> usize {
        self.scheduler.run_frame_tasks()
    }

    /// Runs idle tasks while `budget` has room, returning how many polls it got
    /// through.
    #[inline]
    pub fn run_idle(&self, budget: &FrameBudget) -> usize {
        self.scheduler.run_idle(budget)
    }

    /// Marks the start of a frame, for the budget's benefit.
    #[inline]
    pub fn begin_frame(&self) {
        self.governor.borrow_mut().begin_frame();
    }

    /// What is left of this frame for idle work.
    ///
    /// Empty when the previous frame overran — recovery comes before background
    /// work.
    #[inline]
    pub fn idle_budget(&self) -> FrameBudget {
        self.governor.borrow().idle_budget()
    }

    /// Marks the end of a frame, recording whether it overran.
    #[inline]
    pub fn end_frame(&self) {
        self.governor.borrow_mut().end_frame();
    }

    /// The display's frame interval.
    #[inline]
    pub fn frame_time(&self) -> Duration {
        self.governor.borrow().frame_time()
    }

    /// Retunes the runtime for the display it turned out to be drawing on.
    ///
    /// A runtime is created before a window exists, so it starts on an assumed
    /// 60 Hz. An event loop that learns the real rate is expected to say so:
    /// until it does, a 120 Hz display is budgeted as though its frames were
    /// twice as long, which is how background work ends up spending time the
    /// frame never had.
    ///
    /// A rate the platform could not report is ignored rather than believed.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use aimer_venus::Venus;
    ///
    /// let venus = Venus::new();
    /// assert!(venus.frame_time() > Duration::from_millis(16));
    ///
    /// venus.set_refresh_rate(120.0);
    /// assert!(venus.frame_time() < Duration::from_micros(8_400));
    /// ```
    #[inline]
    pub fn set_refresh_rate(&self, hz: f32) {
        self.governor.borrow_mut().set_refresh_rate(hz);
    }

    /// Runs one whole frame around `build`.
    ///
    /// The order is the contract, and it is the same one browsers settled on:
    ///
    /// 1. **frame tasks** — animation ticks, so the values `build` reads are for
    ///    this frame;
    /// 2. **microtasks** — drained to exhaustion, so every resolved effect is
    ///    visible to `build`;
    /// 3. **`build`** — the caller's build, layout and paint;
    /// 4. **idle tasks** — whatever slack is left.
    ///
    /// An event loop that needs to interleave its own work between the phases
    /// calls them individually instead.
    pub fn drive_frame<R>(&self, build: impl FnOnce() -> R) -> R {
        self.begin_frame();
        self.run_frame_tasks();
        self.run_microtasks();

        let built = build();

        let budget = self.idle_budget();
        self.run_idle(&budget);
        self.end_frame();

        built
    }

    /// Runs potentially blocking `work` on a worker thread, resolving on the UI thread.
    ///
    /// The escape hatch a frame budget makes necessary: work with no loop to
    /// slice — a decode, a parse, a blocking read — cannot cooperate with the
    /// frame scheduler, so it leaves the UI thread entirely. The awaiting task
    /// keeps its non-`Send` captures, because only the closure and the result
    /// cross the boundary.
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
    ///     let parsed = runtime
    ///         .spawn_blocking(|| "41".parse::<i32>().unwrap_or_default())
    ///         .await;
    ///     // Still on the UI thread, still holding the `Rc`.
    ///     mutated.set(parsed + 1);
    /// });
    ///
    /// while venus.task_count() > 0 {
    ///     venus.run_microtasks();
    /// }
    /// assert_eq!(state.get(), 42);
    /// ```
    #[cfg(not(target_arch = "wasm32"))]
    #[inline]
    pub fn spawn_blocking<T, F>(&self, work: F) -> Offloaded<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        self.pool.spawn_blocking(work)
    }

    /// Compatibility alias for [`Self::spawn_blocking`].
    #[cfg(not(target_arch = "wasm32"))]
    #[inline]
    pub fn offload<T, F>(&self, work: F) -> Offloaded<T>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        self.spawn_blocking(work)
    }

    /// The worker pool, for code that wants to size or inspect it.
    #[cfg(not(target_arch = "wasm32"))]
    #[inline]
    pub fn offload_pool(&self) -> &OffloadPool {
        &self.pool
    }

    /// Installs this runtime as the one for the calling thread.
    ///
    /// Callbacks reach it through [`Venus::current`] instead of being handed a
    /// spawner from above, which is what lets a handler eleven elements deep
    /// spawn a task at all.
    pub fn install(self: &Rc<Self>) {
        CURRENT.with(|current| {
            *current.borrow_mut() = Some(Rc::clone(self));
        });
    }

    /// Removes the runtime installed on the calling thread, if any.
    pub fn uninstall() -> Option<Rc<Self>> {
        CURRENT.with(|current| current.borrow_mut().take())
    }

    /// The runtime installed on the calling thread, if any.
    pub fn current() -> Option<Rc<Self>> {
        CURRENT.with(|current| current.borrow().clone())
    }
}

/// Spawns a microtask on the runtime installed for this thread.
///
/// `None` when no runtime is installed, which is a fact worth reporting rather
/// than a panic: a widget tested in isolation has no event loop, and losing a
/// task there should not take the test process with it.
///
/// # Examples
///
/// ```
/// use aimer_venus::{Venus, spawn_local};
///
/// // Nothing installed: the work is declined rather than dropped silently.
/// assert!(spawn_local(async {}).is_none());
///
/// let venus = Venus::new();
/// venus.install();
/// assert!(spawn_local(async {}).is_some());
/// Venus::uninstall();
/// ```
#[inline]
pub fn spawn_local(future: impl Future<Output = ()> + 'static) -> Option<TaskId> {
    Venus::current().map(|venus| venus.spawn(future))
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;

    #[test]
    fn a_frame_runs_its_phases_in_order() {
        let venus = Venus::new();
        let order = Rc::new(RefCell::new(Vec::new()));

        let ticked = order.clone();
        venus.spawn_frame(async move { ticked.borrow_mut().push("frame") });

        let mutated = order.clone();
        venus.spawn(async move { mutated.borrow_mut().push("microtask") });

        let idled = order.clone();
        venus.spawn_idle(async move { idled.borrow_mut().push("idle") });

        let built = order.clone();
        venus.drive_frame(|| built.borrow_mut().push("build"));

        assert_eq!(*order.borrow(), vec!["frame", "microtask", "build", "idle"]);
    }

    #[test]
    fn an_installed_runtime_is_reachable_without_being_handed_down() {
        let venus = Venus::new();
        venus.install();

        let ran = Rc::new(Cell::new(false));
        let flag = ran.clone();
        let task = spawn_local(async move { flag.set(true) });

        assert!(task.is_some());
        Venus::current()
            .expect("an installed runtime")
            .run_microtasks();
        assert!(ran.get());

        Venus::uninstall();
        assert!(Venus::current().is_none());
        assert!(spawn_local(async {}).is_none());
    }

    // A frame that overran must not spend the next frame's slack on background
    // work, or one late frame becomes a run of them.
    #[test]
    fn idle_work_is_withheld_from_the_frame_after_an_overrun() {
        let venus = Venus::with_governor(
            FrameGovernor::new(Duration::from_millis(10)).safety_margin(Duration::ZERO),
        );

        let during_a_healthy_frame = Rc::new(Cell::new(false));
        let flag = during_a_healthy_frame.clone();
        venus.spawn_idle(async move { flag.set(true) });

        venus.begin_frame();
        venus.run_idle(&venus.idle_budget());
        assert!(during_a_healthy_frame.get(), "a frame with slack spends it");

        // Whatever the build did, the frame missed its deadline.
        std::thread::sleep(Duration::from_millis(12));
        venus.end_frame();

        let during_the_recovering_frame = Rc::new(Cell::new(false));
        let flag = during_the_recovering_frame.clone();
        venus.spawn_idle(async move { flag.set(true) });

        venus.begin_frame();
        venus.run_idle(&venus.idle_budget());
        venus.end_frame();

        assert!(
            !during_the_recovering_frame.get(),
            "the frame after an overrun spends nothing on idle work"
        );
    }
}
