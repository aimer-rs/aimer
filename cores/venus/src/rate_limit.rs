//! Rate limiting for bursty events: [`Debouncer`] and [`Throttler`].
//!
//! Both are driven by the host's timers, so an action fires by itself — nothing
//! has to poll them each frame, unlike `aimer_time::Debounce` and
//! `aimer_time::Throttle`, which only answer "would it fire now?". Both own the
//! tasks they schedule: dropping the limiter cancels whatever it was holding,
//! which is what an element that owns one wants when it unmounts.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use aimer_time::Instant;

use crate::delay::spawn_delayed_on;
use crate::scheduler::LocalScheduler;
use crate::task::{ScopeId, TaskId};
use crate::timeout::TimeoutError;
use crate::venus::Venus;

/// Runs an action once its calls have stopped for a quiet period.
///
/// Every [`call`](Self::call) replaces the previous pending action and
/// restarts the wait, so a burst — keystrokes in a search box, a window being
/// dragged to a new size — produces one run, with the *last* action, after the
/// burst ends.
///
/// Dropping the debouncer cancels the pending action.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
///
/// use aimer_venus::Venus;
///
/// let venus = Venus::new();
/// let search = venus.debouncer(Duration::from_millis(250));
///
/// // Without a host timer there is nothing to wait with, and it says so.
/// assert!(search.call(|| { /* run the query */ }).is_err());
/// ```
pub struct Debouncer {
    scheduler: Option<Rc<LocalScheduler>>,
    delay: Duration,
    pending: Cell<Option<TaskId>>,
}

impl Debouncer {
    fn new(scheduler: Option<Rc<LocalScheduler>>, delay: Duration) -> Self {
        Self {
            scheduler,
            delay,
            pending: Cell::new(None),
        }
    }

    /// The quiet period an action waits out.
    #[inline]
    pub fn delay(&self) -> Duration {
        self.delay
    }

    /// Schedules `action` to run after the quiet period, replacing any action
    /// still waiting.
    ///
    /// # Errors
    ///
    /// Returns [`TimeoutError::TimerUnavailable`] if the host supplies no timer
    /// or no runtime is installed. The action is not run and the previously
    /// pending one, if any, is left alone.
    pub fn call(&self, action: impl FnOnce() + 'static) -> Result<(), TimeoutError> {
        let scheduler = self
            .scheduler
            .as_deref()
            .ok_or(TimeoutError::TimerUnavailable)?;
        // The new timer exists before the old one is cancelled, so a failure
        // loses nothing.
        let task = spawn_delayed_on(scheduler, ScopeId::ROOT, self.delay, action)?;
        if let Some(previous) = self.pending.replace(Some(task)) {
            scheduler.abort(previous);
        }
        Ok(())
    }

    /// Drops the pending action, returning whether there was one.
    pub fn cancel(&self) -> bool {
        match (self.pending.take(), self.scheduler.as_deref()) {
            (Some(task), Some(scheduler)) => scheduler.abort(task),
            _ => false,
        }
    }

    /// Whether an action is waiting for its quiet period to end.
    pub fn is_pending(&self) -> bool {
        match (self.pending.get(), self.scheduler.as_deref()) {
            (Some(task), Some(scheduler)) => scheduler.is_running(task),
            _ => false,
        }
    }
}

impl Drop for Debouncer {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// What a [`Throttler`] remembers between calls.
struct Window {
    last_run: Cell<Option<Instant>>,
    /// The newest action that arrived while the window was closed.
    trailing: RefCell<Option<Box<dyn FnOnce()>>>,
    timer: Cell<Option<TaskId>>,
}

/// Lets an action run at most once per interval, without losing the last call.
///
/// The first [`call`](Self::call) runs immediately and opens a window. Calls
/// inside the window are not dropped: the newest one is kept and runs when the
/// window closes, which opens the next. A steady stream of calls therefore
/// runs once per interval, and the final call of a burst always runs — a
/// slider's last value is never lost to the throttle.
///
/// Dropping the throttler cancels the deferred action.
///
/// # Examples
///
/// ```
/// use std::cell::Cell;
/// use std::rc::Rc;
/// use std::time::Duration;
///
/// use aimer_venus::Venus;
///
/// let venus = Venus::new();
/// let scroll = venus.throttler(Duration::from_millis(100));
/// let saved = Rc::new(Cell::new(0));
///
/// let value = saved.clone();
/// scroll.call(move || value.set(1)).unwrap();
/// assert_eq!(saved.get(), 1, "the first call runs at once");
/// ```
pub struct Throttler {
    scheduler: Option<Rc<LocalScheduler>>,
    interval: Duration,
    window: Rc<Window>,
}

impl Throttler {
    fn new(scheduler: Option<Rc<LocalScheduler>>, interval: Duration) -> Self {
        Self {
            scheduler,
            interval,
            window: Rc::new(Window {
                last_run: Cell::new(None),
                trailing: RefCell::new(None),
                timer: Cell::new(None),
            }),
        }
    }

    /// The minimum time between runs.
    #[inline]
    pub fn interval(&self) -> Duration {
        self.interval
    }

    /// Runs `action` now if the window is open, otherwise defers it to the end
    /// of the window, replacing any action already deferred.
    ///
    /// # Errors
    ///
    /// Returns [`TimeoutError::TimerUnavailable`] if the call had to be
    /// deferred and the host supplies no timer or no runtime is installed. The
    /// action is not run and the previously deferred one is left alone.
    pub fn call(&self, action: impl FnOnce() + 'static) -> Result<(), TimeoutError> {
        let now = Instant::now();
        let window = &self.window;
        let since_last = window
            .last_run
            .get()
            .map(|last| now.saturating_duration_since(last));
        let open = since_last.is_none_or(|elapsed| elapsed >= self.interval);
        let deferred_timer = window.timer.get();

        // A late timer can leave the window open with a deferred action still
        // waiting; the newest call must not jump ahead of it, so it joins it.
        if open && deferred_timer.is_none() {
            window.last_run.set(Some(now));
            action();
            return Ok(());
        }

        let scheduler = self
            .scheduler
            .as_deref()
            .ok_or(TimeoutError::TimerUnavailable)?;
        if !deferred_timer.is_some_and(|task| scheduler.is_running(task)) {
            let remaining = since_last.map_or(Duration::ZERO, |elapsed| {
                self.interval.saturating_sub(elapsed)
            });
            let closing = Rc::clone(window);
            let task = spawn_delayed_on(scheduler, ScopeId::ROOT, remaining, move || {
                closing.timer.set(None);
                // Taken before running so the action may call back in.
                let action = closing.trailing.borrow_mut().take();
                if let Some(action) = action {
                    closing.last_run.set(Some(Instant::now()));
                    action();
                }
            })?;
            window.timer.set(Some(task));
        }
        *window.trailing.borrow_mut() = Some(Box::new(action));
        Ok(())
    }

    /// Drops the deferred action, returning whether there was one. The current
    /// window stays closed until it runs out.
    pub fn cancel(&self) -> bool {
        if let (Some(task), Some(scheduler)) = (self.window.timer.take(), self.scheduler.as_deref())
        {
            scheduler.abort(task);
        }
        self.window.trailing.borrow_mut().take().is_some()
    }

    /// Whether an action is deferred, waiting for the window to close.
    pub fn is_pending(&self) -> bool {
        self.window.trailing.borrow().is_some()
    }
}

impl Drop for Throttler {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl Venus {
    /// Creates a [`Debouncer`] that waits `delay` after the last call.
    #[inline]
    pub fn debouncer(&self, delay: Duration) -> Debouncer {
        Debouncer::new(Some(self.scheduler().clone()), delay)
    }

    /// Creates a [`Throttler`] that lets one action through per `interval`.
    #[inline]
    pub fn throttler(&self, interval: Duration) -> Throttler {
        Throttler::new(Some(self.scheduler().clone()), interval)
    }
}

/// [`Venus::debouncer`] on the runtime installed for this thread.
///
/// Every call reports [`TimeoutError::TimerUnavailable`] when none is installed.
#[inline]
pub fn debouncer(delay: Duration) -> Debouncer {
    Debouncer::new(Venus::current().map(|venus| venus.scheduler().clone()), delay)
}

/// [`Venus::throttler`] on the runtime installed for this thread.
///
/// Without a runtime the first call of each window still runs; a call that
/// would have to be deferred reports [`TimeoutError::TimerUnavailable`].
#[inline]
pub fn throttler(interval: Duration) -> Throttler {
    Throttler::new(
        Venus::current().map(|venus| venus.scheduler().clone()),
        interval,
    )
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::time::Duration;

    use super::*;
    use crate::test_host::with_timers;

    fn recorder() -> (Rc<RefCell<Vec<u32>>>, impl Fn(u32) -> Box<dyn FnOnce()>) {
        let log = Rc::new(RefCell::new(Vec::new()));
        let sink = log.clone();
        (log, move |value| {
            let sink = sink.clone();
            Box::new(move || sink.borrow_mut().push(value))
        })
    }

    #[test]
    fn a_debounced_burst_runs_once_with_the_last_action() {
        let (venus, timers) = with_timers();
        let search = venus.debouncer(Duration::from_millis(250));
        let (log, action) = recorder();

        for value in 1..=3 {
            search.call(action(value)).expect("a timer is installed");
            venus.run_microtasks();
        }
        assert!(search.is_pending());
        assert!(log.borrow().is_empty(), "nothing runs during the burst");
        assert_eq!(timers.len(), 3);
        assert_eq!(timers.duration(2), Duration::from_millis(250));
        assert!(timers.state(0).dropped.get(), "superseded timers are released");

        for index in 0..3 {
            timers.fire(index);
            venus.run_microtasks();
        }

        assert_eq!(*log.borrow(), [3]);
        assert!(!search.is_pending());
        assert_eq!(venus.task_count(), 0);
    }

    #[test]
    fn a_debouncer_can_be_used_again_after_it_fires() {
        let (venus, timers) = with_timers();
        let search = venus.debouncer(Duration::from_millis(10));
        let (log, action) = recorder();

        search.call(action(1)).unwrap();
        venus.run_microtasks();
        timers.fire(0);
        venus.run_microtasks();
        search.call(action(2)).unwrap();
        venus.run_microtasks();
        timers.fire(1);
        venus.run_microtasks();

        assert_eq!(*log.borrow(), [1, 2]);
    }

    #[test]
    fn cancelling_a_debounce_drops_the_action() {
        let (venus, timers) = with_timers();
        let search = venus.debouncer(Duration::from_millis(10));
        let (log, action) = recorder();
        search.call(action(1)).unwrap();
        venus.run_microtasks();

        assert!(search.cancel());
        assert!(!search.cancel(), "nothing left to cancel");
        timers.fire(0);
        venus.run_microtasks();

        assert!(log.borrow().is_empty());
        assert!(!search.is_pending());
    }

    #[test]
    fn dropping_a_debouncer_cancels_its_pending_action() {
        let (venus, timers) = with_timers();
        let (log, action) = recorder();
        {
            let search = venus.debouncer(Duration::from_millis(10));
            search.call(action(1)).unwrap();
            venus.run_microtasks();
        }

        timers.fire(0);
        venus.run_microtasks();

        assert!(log.borrow().is_empty());
        assert!(timers.state(0).dropped.get());
        assert_eq!(venus.task_count(), 0);
    }

    #[test]
    fn a_debounce_without_a_timer_refuses_and_keeps_nothing() {
        let venus = Venus::new();
        let search = venus.debouncer(Duration::from_millis(10));
        let (log, action) = recorder();

        assert_eq!(search.call(action(1)), Err(TimeoutError::TimerUnavailable));

        venus.run_microtasks();
        assert!(log.borrow().is_empty());
        assert!(!search.is_pending());
        assert_eq!(venus.task_count(), 0);
    }

    #[test]
    fn the_first_throttled_call_runs_immediately() {
        let (venus, timers) = with_timers();
        let scroll = venus.throttler(Duration::from_secs(10));
        let (log, action) = recorder();

        scroll.call(action(1)).unwrap();

        assert_eq!(*log.borrow(), [1], "no microtask needed");
        assert_eq!(timers.len(), 0, "and no timer");
        assert!(!scroll.is_pending());
        drop(venus);
    }

    #[test]
    fn calls_inside_the_window_keep_only_the_newest_and_run_it_at_the_end() {
        let (venus, timers) = with_timers();
        let scroll = venus.throttler(Duration::from_secs(10));
        let (log, action) = recorder();

        scroll.call(action(1)).unwrap();
        scroll.call(action(2)).unwrap();
        scroll.call(action(3)).unwrap();
        venus.run_microtasks();

        assert_eq!(*log.borrow(), [1]);
        assert!(scroll.is_pending());
        assert_eq!(timers.len(), 1, "one timer however many calls were deferred");
        let asked = timers.duration(0);
        assert!(asked <= Duration::from_secs(10));
        assert!(asked > Duration::from_secs(9), "got {asked:?}");

        timers.fire(0);
        venus.run_microtasks();

        assert_eq!(*log.borrow(), [1, 3], "the newest deferred call is the one that runs");
        assert!(!scroll.is_pending());
        assert_eq!(venus.task_count(), 0);
    }

    #[test]
    fn running_the_deferred_action_opens_the_next_window() {
        let (venus, timers) = with_timers();
        let scroll = venus.throttler(Duration::from_secs(10));
        let (log, action) = recorder();
        scroll.call(action(1)).unwrap();
        scroll.call(action(2)).unwrap();
        venus.run_microtasks();
        timers.fire(0);
        venus.run_microtasks();

        // The window the trailing run opened is still closed.
        scroll.call(action(3)).unwrap();
        venus.run_microtasks();

        assert_eq!(*log.borrow(), [1, 2]);
        assert_eq!(timers.len(), 2);
        assert!(scroll.is_pending());
    }

    #[test]
    fn a_call_after_the_interval_has_passed_runs_at_once() {
        let (venus, timers) = with_timers();
        let scroll = venus.throttler(Duration::from_millis(15));
        let (log, action) = recorder();

        scroll.call(action(1)).unwrap();
        std::thread::sleep(Duration::from_millis(40));
        scroll.call(action(2)).unwrap();

        assert_eq!(*log.borrow(), [1, 2]);
        assert_eq!(timers.len(), 0);
        drop(venus);
    }

    #[test]
    fn a_deferred_action_may_call_the_throttler_again() {
        let (venus, timers) = with_timers();
        let scroll = Rc::new(venus.throttler(Duration::from_secs(10)));
        let (log, action) = recorder();
        scroll.call(action(1)).unwrap();

        let again = scroll.clone();
        let sink = log.clone();
        scroll
            .call(move || {
                sink.borrow_mut().push(2);
                // Inside the window the trailing run just opened.
                again.call(|| {}).expect("deferral must not deadlock");
            })
            .unwrap();
        venus.run_microtasks();
        timers.fire(0);
        venus.run_microtasks();

        assert_eq!(*log.borrow(), [1, 2]);
        assert!(scroll.is_pending(), "the nested call was deferred, not lost");
    }

    #[test]
    fn cancelling_a_throttle_drops_the_deferred_action() {
        let (venus, timers) = with_timers();
        let scroll = venus.throttler(Duration::from_secs(10));
        let (log, action) = recorder();
        scroll.call(action(1)).unwrap();
        scroll.call(action(2)).unwrap();
        venus.run_microtasks();

        assert!(scroll.cancel());
        assert!(!scroll.cancel());
        timers.fire(0);
        venus.run_microtasks();

        assert_eq!(*log.borrow(), [1]);
        assert!(timers.state(0).dropped.get());
    }

    #[test]
    fn dropping_a_throttler_cancels_its_deferred_action() {
        let (venus, timers) = with_timers();
        let (log, action) = recorder();
        {
            let scroll = venus.throttler(Duration::from_secs(10));
            scroll.call(action(1)).unwrap();
            scroll.call(action(2)).unwrap();
            venus.run_microtasks();
        }

        timers.fire(0);
        venus.run_microtasks();

        assert_eq!(*log.borrow(), [1]);
        assert_eq!(venus.task_count(), 0);
    }

    #[test]
    fn a_deferral_without_a_timer_refuses_but_the_leading_call_still_runs() {
        let venus = Venus::new();
        let scroll = venus.throttler(Duration::from_secs(10));
        let (log, action) = recorder();

        scroll.call(action(1)).unwrap();
        assert_eq!(scroll.call(action(2)), Err(TimeoutError::TimerUnavailable));

        assert_eq!(*log.borrow(), [1]);
        assert!(!scroll.is_pending());
    }

    #[test]
    fn the_free_functions_use_the_installed_runtime() {
        let (venus, timers) = with_timers();
        venus.install();
        let search = debouncer(Duration::from_millis(5));
        let scroll = throttler(Duration::from_secs(10));
        Venus::uninstall();
        let (log, action) = recorder();

        search.call(action(1)).unwrap();
        scroll.call(action(2)).unwrap();
        scroll.call(action(3)).unwrap();
        venus.run_microtasks();
        for index in 0..timers.len() {
            timers.fire(index);
            venus.run_microtasks();
        }

        let mut ran = log.borrow().clone();
        ran.sort();
        assert_eq!(ran, [1, 2, 3]);
    }

    #[test]
    fn the_free_functions_report_a_missing_runtime() {
        assert!(Venus::current().is_none());
        let ran = Rc::new(Cell::new(false));

        let flag = ran.clone();
        assert_eq!(
            debouncer(Duration::ZERO).call(move || flag.set(true)),
            Err(TimeoutError::TimerUnavailable)
        );
        assert!(!ran.get());
    }
}
