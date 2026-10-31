//! Saving in the background, coalesced.
//!
//! State that changes often, such as a setting dragged by a slider, must reach
//! storage without a write per change and without the UI waiting on one. A
//! [`WriteBehind`] marks itself dirty on every change and runs at most one save
//! at a time as an idle Venus task; changes that arrive while a save runs, or
//! before the idle task gets its turn, collapse into the next save.

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::rc::{Rc, Weak};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use aimer_venus::{FrameBudget, Lifecycle, LifecycleSubscription, Venus, on_lifecycle};

use crate::error::{PersistError, PersistFuture};

/// Starts one save of the current state, or `None` when there is nothing to
/// save. The state is encoded when this is called, so the save is a consistent
/// snapshot however the state changes while it is written.
pub(crate) type SaveFn = Box<dyn Fn() -> Option<PersistFuture<()>>>;

#[derive(Default)]
struct State {
    dirty: bool,
    /// Whether a save loop is running, in an idle task or inside a `flush`.
    running: bool,
    waiters: Vec<Waker>,
    last_error: Option<PersistError>,
}

struct Shared {
    save: SaveFn,
    state: RefCell<State>,
}

impl Shared {
    /// Saves until nothing is dirty. The caller has already set `running`.
    async fn drive(self: Rc<Self>) {
        loop {
            {
                let mut state = self.state.borrow_mut();
                if !std::mem::take(&mut state.dirty) {
                    state.running = false;
                    for waiter in state.waiters.drain(..) {
                        waiter.wake();
                    }
                    return;
                }
            }
            // Outside the borrow: encoding runs arbitrary user code.
            if let Some(save) = (self.save)() {
                let result = save.await;
                self.state.borrow_mut().last_error = result.err();
            }
        }
    }
}

/// A handle to a coalescing background saver.
#[derive(Clone)]
pub(crate) struct WriteBehind(Rc<Shared>);

impl WriteBehind {
    pub(crate) fn new(save: SaveFn) -> Self {
        let shared = Rc::new(Shared {
            save,
            state: RefCell::new(State::default()),
        });
        register(&shared);
        Self(shared)
    }

    /// Records that the state changed and a save is owed.
    ///
    /// Without a Venus runtime on this thread nothing is scheduled; the change
    /// stays owed until [`flush`](Self::flush).
    pub(crate) fn mark_dirty(&self) {
        let venus = Venus::current();
        let mut state = self.0.state.borrow_mut();
        state.dirty = true;
        if state.running {
            return;
        }
        if let Some(venus) = venus {
            state.running = true;
            drop(state);
            venus.spawn_idle(Rc::clone(&self.0).drive());
        }
    }

    /// Resolves once everything marked dirty so far has been saved, with the
    /// error of the last failed save if it has not been followed by a good one.
    pub(crate) fn flush(&self) -> PersistFuture<()> {
        let shared = Rc::clone(&self.0);
        Box::pin(async move {
            loop {
                enum Next {
                    Wait,
                    Drive,
                    Done,
                }
                let next = {
                    let mut state = shared.state.borrow_mut();
                    if state.running {
                        Next::Wait
                    } else if state.dirty {
                        state.running = true;
                        Next::Drive
                    } else {
                        Next::Done
                    }
                };
                match next {
                    Next::Wait => WaitIdle(Rc::clone(&shared)).await,
                    Next::Drive => Rc::clone(&shared).drive().await,
                    Next::Done => break,
                }
            }
            match shared.state.borrow().last_error.clone() {
                Some(error) => Err(error),
                None => Ok(()),
            }
        })
    }

    /// The error of the most recent background save, if it failed.
    pub(crate) fn last_error(&self) -> Option<PersistError> {
        self.0.state.borrow().last_error.clone()
    }

    /// Whether a change has not yet been handed to storage.
    pub(crate) fn is_dirty(&self) -> bool {
        let state = self.0.state.borrow();
        state.dirty || state.running
    }
}

/// Pending until the running save loop finishes.
struct WaitIdle(Rc<Shared>);

impl Future for WaitIdle {
    type Output = ();

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<()> {
        let mut state = self.0.state.borrow_mut();
        if state.running {
            state.waiters.push(context.waker().clone());
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    }
}

// ───────────────────────── protecting unsaved changes ─────────────────────────
//
// A background save protects against nothing if the process dies before it runs.
// Every saver on the thread is therefore registered here, and the event loop's
// lifecycle notifications (see `aimer_venus::notify_lifecycle`) finish them at the
// moments a platform warns the app: when it loses focus, goes to the background,
// or quits. A mobile OS may kill a backgrounded app without another word, so the
// background and exit saves block until the data has reached storage.

/// How long a lifecycle notification may block the UI thread finishing saves.
///
/// Mobile watchdogs kill an app that stalls its main thread for several
/// seconds, so this stays well below that. Saves are small and almost always
/// finish in milliseconds.
const DEFAULT_LIFECYCLE_TIMEOUT: Duration = Duration::from_secs(2);

aimer_tls::thread_local! {
    static FLUSH_ON_PANIC: Cell<bool> = const { Cell::new(true) };
    static SAVERS: RefCell<Vec<Weak<Shared>>> = const { RefCell::new(Vec::new()) };
    static LIFECYCLE: RefCell<Option<LifecycleSubscription>> = const { RefCell::new(None) };
    static TIMEOUT: Cell<Duration> = const { Cell::new(DEFAULT_LIFECYCLE_TIMEOUT) };
}

fn register(shared: &Rc<Shared>) {
    SAVERS.with(|savers| {
        let mut savers = savers.borrow_mut();
        savers.retain(|saver| saver.strong_count() > 0);
        savers.push(Rc::downgrade(shared));
    });
    LIFECYCLE.with(|lifecycle| {
        let mut lifecycle = lifecycle.borrow_mut();
        if lifecycle.is_none() {
            *lifecycle = Some(on_lifecycle(on_lifecycle_event));
        }
    });
}

fn live_savers() -> Vec<WriteBehind> {
    SAVERS.with(|savers| {
        let mut savers = savers.borrow_mut();
        savers.retain(|saver| saver.strong_count() > 0);
        savers.iter().filter_map(Weak::upgrade).map(WriteBehind).collect()
    })
}

/// Whether any saver on this thread has a change not yet handed to storage.
pub(crate) fn any_unsaved() -> bool {
    live_savers().iter().any(WriteBehind::is_dirty)
}

pub(crate) fn lifecycle_timeout() -> Duration {
    TIMEOUT.with(Cell::get)
}

pub(crate) fn set_lifecycle_timeout(timeout: Duration) {
    TIMEOUT.with(|current| current.set(timeout));
}

fn on_lifecycle_event(event: Lifecycle) {
    match event {
        Lifecycle::Inactive => start_pending_saves(),
        Lifecycle::Panic => emergency_flush(),
        Lifecycle::Background | Lifecycle::Exit => {
            // Nothing to do about a timeout or an error here: the app is on its
            // way out and the outcome is available to it through `last_save_error`.
            let _ = flush_all_blocking(lifecycle_timeout());
        }
    }
}

/// Gives the pending saves their turn right now instead of at idle, without
/// waiting for them: the writes start on the storage workers and finish even if
/// the UI thread moves on.
pub(crate) fn start_pending_saves() {
    if aimer_venus::is_polling_task() {
        return;
    }
    if let Some(venus) = Venus::current() {
        venus.run_microtasks();
        venus.run_idle(&FrameBudget::from_now(Duration::from_millis(8)));
    }
}

/// How a [`flush_all_blocking`] ended.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FlushOutcome {
    /// Every pending change has reached storage.
    Complete,
    /// The time ran out first.
    TimedOut {
        /// How many savers still had unsaved changes.
        unsaved: usize,
    },
    /// Everything finished, but a save failed. Carries the first failure.
    Failed(PersistError),
}

/// Blocks the calling (UI) thread until every pending change of every
/// [`Preferences`](crate::Preferences) and [`Persisted`](crate::Persisted) on it
/// has been saved, or `timeout` passes.
///
/// While waiting it runs the Venus runtime installed on the thread, which is
/// what completes the saves, so tasks other than the saves also make progress.
/// Do not call it from inside a Venus task: await the individual `flush()`
/// futures there instead. In that case, and on the web, where nothing may block,
/// it returns immediately having only started the saves.
///
/// The event loop already calls this when the app goes to the background and
/// when it exits; call it yourself only at other points where a crash must not
/// lose state.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use aimer_persist::{FlushOutcome, flush_all_blocking};
///
/// // Nothing is pending, so there is nothing to wait for.
/// assert_eq!(flush_all_blocking(Duration::from_secs(1)), FlushOutcome::Complete);
/// ```
pub fn flush_all_blocking(timeout: Duration) -> FlushOutcome {
    let mut pending: Vec<(WriteBehind, PersistFuture<()>)> = live_savers()
        .into_iter()
        .filter(WriteBehind::is_dirty)
        .map(|saver| {
            let flush = saver.flush();
            (saver, flush)
        })
        .collect();
    if pending.is_empty() {
        return FlushOutcome::Complete;
    }
    if !can_block() {
        start_pending_saves();
        return FlushOutcome::TimedOut {
            unsaved: pending.len(),
        };
    }

    let deadline = aimer_time::Instant::now() + timeout;
    let mut first_error = None;
    let mut context = Context::from_waker(Waker::noop());
    loop {
        // Every future is polled each round rather than waited on: the savers
        // finish through Venus, which is pumped below, and a saver with no
        // runtime finishes by being polled.
        pending.retain_mut(|(_, flush)| match flush.as_mut().poll(&mut context) {
            Poll::Ready(result) => {
                if let Err(error) = result {
                    first_error.get_or_insert(error);
                }
                false
            }
            Poll::Pending => true,
        });
        if pending.is_empty() {
            return match first_error {
                Some(error) => FlushOutcome::Failed(error),
                None => FlushOutcome::Complete,
            };
        }
        if aimer_time::Instant::now() >= deadline {
            return FlushOutcome::TimedOut {
                unsaved: pending.len(),
            };
        }
        if let Some(venus) = Venus::current() {
            venus.run_microtasks();
            venus.run_idle(&FrameBudget::from_now(Duration::from_millis(8)));
        }
        // Storage work is in flight on another thread; do not spin on it.
        std::thread::sleep(Duration::from_micros(500));
    }
}

/// Whether the calling thread may block and pump the runtime itself.
fn can_block() -> bool {
    !cfg!(target_arch = "wasm32") && !aimer_venus::is_polling_task()
}

// ─────────────────────────────── when the app panics ───────────────────────────────
//
// In a release build a panic ends the process the moment the panic hook returns,
// so the hook is the last chance to save (see `aimer_venus::install_panic_hook`,
// which delivers `Lifecycle::Panic` here, on the UI thread, whichever thread
// panicked).
//
// This path is deliberately not `flush_all_blocking`:
//
// * It must not run the Venus runtime. The panic may have happened in the middle
//   of a widget build or a task, and running other tasks from inside the hook
//   would re-enter the very code that just failed.
// * It must not panic. Rust aborts on a panic raised while one is being handled,
//   and `catch_unwind` cannot prevent that inside a hook. Everything here uses
//   `try_borrow` and `try_with`, and skips what it cannot reach.
//
// Instead it starts a fresh save of each saver's latest state and polls those
// futures itself. Storage does the I/O on its own threads, and the saves queue
// behind any save already in flight, so when they finish, so has everything
// before them.

/// Enables or disables saving pending changes when the app panics.
/// The default is enabled.
///
/// Saving at a panic persists state that may be part of what went wrong, and an
/// app that reloads that state and panics again would crash on every launch. An
/// app that cannot rule that out can turn this off and rely on the other
/// moments pending changes are saved.
pub(crate) fn set_flush_on_panic(enabled: bool) {
    let _ = FLUSH_ON_PANIC.try_with(|flag| flag.set(enabled));
}

impl WriteBehind {
    /// Whether this saver may have changes not yet in storage, answering `true`
    /// when it cannot tell because its state is borrowed.
    fn may_be_pending(&self) -> bool {
        match self.0.state.try_borrow() {
            Ok(state) => state.dirty || state.running,
            Err(_) => true,
        }
    }

    /// Starts a save of the current state without touching the saver's own
    /// bookkeeping, which a panic may have left borrowed.
    fn start_emergency_save(&self) -> Option<PersistFuture<()>> {
        if self.may_be_pending() {
            (self.0.save)()
        } else {
            None
        }
    }
}

/// The savers on this thread, without panicking if their list is borrowed.
fn savers_if_reachable() -> Vec<WriteBehind> {
    SAVERS
        .try_with(|savers| match savers.try_borrow() {
            Ok(savers) => savers.iter().filter_map(Weak::upgrade).map(WriteBehind).collect(),
            Err(_) => Vec::new(),
        })
        .unwrap_or_default()
}

fn emergency_flush() {
    if !FLUSH_ON_PANIC.try_with(Cell::get).unwrap_or(false) || cfg!(target_arch = "wasm32") {
        return;
    }
    let mut saves: Vec<PersistFuture<()>> = savers_if_reachable()
        .iter()
        .filter_map(WriteBehind::start_emergency_save)
        .collect();

    let deadline = aimer_time::Instant::now() + lifecycle_timeout();
    let mut context = Context::from_waker(Waker::noop());
    while !saves.is_empty() && aimer_time::Instant::now() < deadline {
        // Errors are not reported: the process is dying, and nothing can act.
        saves.retain_mut(|save| save.as_mut().poll(&mut context).is_pending());
        if !saves.is_empty() {
            std::thread::sleep(Duration::from_micros(500));
        }
    }
}
