//! Task identity, frame phases, and the scopes that own tasks.

pub(crate) mod slab;
pub(crate) mod waker;

use std::pin::Pin;
use std::rc::{Rc, Weak};

use crate::scheduler::LocalScheduler;

pub use crate::task::waker::Notifier;

/// A spawned future, as Venus stores it.
///
/// Deliberately *not* `Send`: a Venus task is polled on the UI thread and
/// nowhere else, which is exactly what lets it hold a `StateUpdater`, a
/// controller, or any other `Rc` from the element tree.
pub(crate) type LocalFuture = Pin<Box<dyn Future<Output = ()>>>;

/// A handle naming one spawned task.
///
/// Copyable and eight bytes wide, so a widget can keep one in a field without
/// thinking about it. It stays valid until the task finishes or is cancelled,
/// after which it names nothing forever — see [`crate::LocalScheduler::abort`].
///
/// # Examples
///
/// ```
/// use aimer_venus::Venus;
///
/// let venus = Venus::new();
/// let task = venus.spawn(async {});
///
/// assert!(venus.is_running(task));
/// venus.run_microtasks();
/// assert!(!venus.is_running(task));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TaskId {
    index: u32,
    generation: u32,
}

impl TaskId {
    #[inline]
    pub(crate) const fn new(index: u32, generation: u32) -> Self {
        Self { index, generation }
    }

    #[inline]
    pub(crate) const fn index(self) -> u32 {
        self.index
    }

    #[inline]
    pub(crate) const fn generation(self) -> u32 {
        self.generation
    }
}

/// When during a frame a task is allowed to run.
///
/// The phase is the whole reason Venus exists rather than a `spawn_local` on
/// somebody else's runtime: "as soon as possible" is not a schedule a UI can be
/// built on, "before this frame's build" is.
///
/// # Examples
///
/// ```
/// use aimer_venus::Phase;
///
/// // Phases run in declaration order within a frame.
/// assert!(Phase::Microtask < Phase::Frame);
/// assert!(Phase::Frame < Phase::Idle);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum Phase {
    /// Runs before this frame's build phase, and is drained to exhaustion.
    ///
    /// This is where a resolved future's `set_state` belongs, and it is why the
    /// effect is visible in the *same* frame instead of the next one. A
    /// microtask may mutate state; it must not do work. Builds with the
    /// `venus-debug` feature complain when one takes longer than
    /// [`crate::MICROTASK_BUDGET_WARNING`].
    #[default]
    Microtask,
    /// Runs once per frame, after the microtask drain.
    ///
    /// Animation tickers and "after layout" callbacks: work that is inherently
    /// per-frame and gains nothing from running twice in one.
    Frame,
    /// Runs only while the frame has measured room left.
    ///
    /// Image decode, glyph rasterisation, prefetch. A task here must be
    /// resumable — see [`crate::yield_if_over_budget`] — because it will be
    /// stopped mid-way. Work that cannot be sliced belongs on
    /// [`crate::Venus::offload`] instead.
    Idle,
}

impl Phase {
    /// How many phases there are, i.e. how many ready queues the scheduler
    /// keeps.
    pub(crate) const COUNT: usize = 3;

    #[inline]
    pub(crate) const fn index(self) -> usize {
        self as usize
    }
}

/// The owner of a group of tasks.
///
/// # Examples
///
/// ```
/// use aimer_venus::{ScopeId, Venus};
///
/// let venus = Venus::new();
/// let scope = venus.scope();
///
/// assert_ne!(scope.id(), ScopeId::ROOT);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct ScopeId(u64);

impl ScopeId {
    /// The scope of tasks nobody claimed, which live until they finish.
    pub const ROOT: Self = Self(0);

    #[inline]
    pub(crate) const fn new(id: u64) -> Self {
        Self(id)
    }
}

/// A scope handle that cancels its tasks when dropped.
///
/// This is Venus's answer to per-widget cancellation bookkeeping: an element
/// keeps a `TaskScope` in its state, spawns everything into it, and unmounting
/// drops the scope. No abort handle per request, no generation counter per
/// widget, no stale reply to recognise and discard — the tasks are simply gone.
///
/// Dropping a scope does not touch a task that is *currently running*; that
/// task's future is dropped as soon as it next yields, because there is no
/// longer anywhere to put it back.
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
/// let ran = Rc::new(Cell::new(false));
///
/// let scope = venus.scope();
/// let flag = ran.clone();
/// venus.spawn_in(scope.id(), async move { flag.set(true) });
///
/// drop(scope);
/// venus.run_microtasks();
///
/// assert!(!ran.get(), "an unmounted element's task never runs");
/// ```
pub struct TaskScope {
    id: ScopeId,
    scheduler: Weak<LocalScheduler>,
}

impl TaskScope {
    #[inline]
    pub(crate) fn new(id: ScopeId, scheduler: &Rc<LocalScheduler>) -> Self {
        Self {
            id,
            scheduler: Rc::downgrade(scheduler),
        }
    }

    /// The id to spawn into.
    #[inline]
    pub const fn id(&self) -> ScopeId {
        self.id
    }

    /// Cancels every task in this scope without giving up ownership.
    ///
    /// A widget whose request key changed wants exactly this: drop the
    /// in-flight work, keep the scope for the next request.
    pub fn cancel(&self) -> usize {
        match self.scheduler.upgrade() {
            Some(scheduler) => scheduler.cancel_scope(self.id),
            None => 0,
        }
    }
}

impl Drop for TaskScope {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl std::fmt::Debug for TaskScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TaskScope({})", self.id.0)
    }
}
