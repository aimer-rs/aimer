//! Time-based futures: sleeping, delayed actions and repeating ticks.
//!
//! Venus owns no clock thread. Every timer here is the host's — whatever
//! [`PollContext::sleep`](crate::PollContext::sleep) returns — which is what
//! lets a timer wake a parked event loop and be cancelled by dropping it. A
//! host without timers reports [`TimeoutError::TimerUnavailable`] instead of
//! pretending the time passed.

use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};
use std::time::Duration;

use aimer_time::Instant;

use crate::scheduler::LocalScheduler;
use crate::timeout::TimeoutError;
use crate::venus::Venus;

type HostTimer = Pin<Box<dyn Future<Output = ()>>>;

enum State {
    Waiting(HostTimer),
    Unavailable,
    Elapsed,
}

/// A future that resolves once a duration has passed.
///
/// Created by [`Venus::sleep`], [`Venus::sleep_until`] or the free function
/// [`sleep`]. The clock starts when the value is created, not when it is first
/// polled, so a `Sleep` made up front measures the time spent doing other
/// things as well.
///
/// Dropping a `Sleep` releases the host's timer. Sleeping is cooperative: it
/// yields the thread, it does not block it.
///
/// # Errors
///
/// Resolves to [`TimeoutError::TimerUnavailable`] if the host supplies no
/// timer, or no runtime is installed on the calling thread.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
///
/// use aimer_venus::Venus;
///
/// // With no host timer installed the sleep says so instead of hanging.
/// let venus = Venus::new();
/// let sleep = venus.sleep(Duration::from_millis(250));
/// venus.spawn(async move {
///     assert!(sleep.await.is_err());
/// });
/// venus.run_microtasks();
/// ```
#[must_use = "a sleep does nothing unless it is awaited"]
pub struct Sleep {
    state: State,
}

impl Sleep {
    #[inline]
    pub(crate) fn new(timer: Option<HostTimer>) -> Self {
        Self {
            state: timer.map_or(State::Unavailable, State::Waiting),
        }
    }

    #[inline]
    pub(crate) fn is_available(&self) -> bool {
        !matches!(self.state, State::Unavailable)
    }
}

impl Future for Sleep {
    type Output = Result<(), TimeoutError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match &mut self.state {
            State::Waiting(timer) => match timer.as_mut().poll(cx) {
                Poll::Ready(()) => {
                    // Release the registration now rather than when the
                    // future is eventually dropped.
                    self.state = State::Elapsed;
                    Poll::Ready(Ok(()))
                }
                Poll::Pending => Poll::Pending,
            },
            State::Unavailable => Poll::Ready(Err(TimeoutError::TimerUnavailable)),
            State::Elapsed => Poll::Ready(Ok(())),
        }
    }
}

/// A fixed-period ticker.
///
/// Created by [`Venus::interval`] or the free function [`interval`]. Each
/// [`tick`](Self::tick) resolves one period after the previous deadline, so a
/// slow body does not make the ticker drift. If the body falls more than a
/// whole period behind, the missed ticks are *skipped*, not replayed: the next
/// tick is one period from now. A UI has no use for a burst of catch-up ticks
/// after a stall.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
///
/// use aimer_venus::Venus;
///
/// let venus = Venus::new();
/// let mut ticker = venus.interval(Duration::from_secs(1));
/// venus.spawn(async move {
///     // Without a host timer the first tick reports it.
///     assert!(ticker.tick().await.is_err());
/// });
/// venus.run_microtasks();
/// ```
pub struct Interval {
    scheduler: Option<Rc<LocalScheduler>>,
    period: Duration,
    next: Instant,
}

impl Interval {
    fn new(scheduler: Option<Rc<LocalScheduler>>, period: Duration) -> Self {
        assert!(!period.is_zero(), "an interval needs a non-zero period");
        let now = Instant::now();
        Self {
            scheduler,
            period,
            // A saturated deadline is as good as never for a period this long.
            next: now.checked_add(period).unwrap_or(now),
        }
    }

    /// The time between ticks.
    #[inline]
    pub fn period(&self) -> Duration {
        self.period
    }

    /// Waits for the next tick.
    ///
    /// The first tick comes one period after the interval was created. The
    /// deadline is advanced when this is called, so a tick that is dropped
    /// without being awaited still counts as taken.
    pub fn tick(&mut self) -> Sleep {
        let now = Instant::now();
        let deadline = self.next;
        let following = deadline.checked_add(self.period).unwrap_or(deadline);
        self.next = if following > now {
            following
        } else {
            now.checked_add(self.period).unwrap_or(now)
        };
        match &self.scheduler {
            Some(scheduler) => sleep_on(scheduler, deadline.saturating_duration_since(now)),
            None => Sleep::new(None),
        }
    }
}

#[inline]
pub(crate) fn sleep_on(scheduler: &LocalScheduler, duration: Duration) -> Sleep {
    Sleep::new(scheduler.sleep(duration))
}

impl Venus {
    /// Waits for `duration` without blocking the thread.
    ///
    /// The clock starts now. Await the result inside a Venus task; the future
    /// and anything the task holds need not be [`Send`].
    ///
    /// # Errors
    ///
    /// Resolves to [`TimeoutError::TimerUnavailable`] if the host supplies no
    /// timer.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use aimer_venus::Venus;
    ///
    /// let venus = Venus::new();
    /// let runtime = venus.clone();
    /// venus.spawn(async move {
    ///     // Resolves `Err` here: this runtime has no host timer.
    ///     let _ = runtime.sleep(Duration::from_millis(16)).await;
    /// });
    /// ```
    #[inline]
    pub fn sleep(&self, duration: Duration) -> Sleep {
        sleep_on(self.scheduler(), duration)
    }

    /// Waits until `deadline`; a deadline already past resolves on the first
    /// poll the host allows.
    #[inline]
    pub fn sleep_until(&self, deadline: Instant) -> Sleep {
        self.sleep(deadline.saturating_duration_since(Instant::now()))
    }

    /// Runs `action` once `duration` has passed, resolving to its result.
    ///
    /// Nothing runs unless the returned future is awaited or spawned, and
    /// dropping it before expiry means `action` never runs. To fire and forget
    /// from a callback, use [`spawn_delayed`](Self::spawn_delayed).
    ///
    /// # Errors
    ///
    /// Resolves to [`TimeoutError::TimerUnavailable`], without running
    /// `action`, if the host supplies no timer.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use aimer_venus::Venus;
    ///
    /// let venus = Venus::new();
    /// let later = venus.delayed(Duration::from_millis(300), || "tooltip shown");
    /// venus.spawn(async move {
    ///     let _ = later.await;
    /// });
    /// ```
    #[inline]
    pub fn delayed<T, F>(
        &self,
        duration: Duration,
        action: F,
    ) -> impl Future<Output = Result<T, TimeoutError>> + use<T, F>
    where
        F: FnOnce() -> T,
    {
        delayed_after(self.sleep(duration), action)
    }

    /// Spawns a microtask that runs `action` once `duration` has passed.
    ///
    /// The returned task can be [aborted](Self::abort) to cancel the action;
    /// use [`spawn_delayed_in`](Self::spawn_delayed_in) to tie it to an
    /// element's scope instead.
    ///
    /// # Errors
    ///
    /// Returns [`TimeoutError::TimerUnavailable`] without spawning anything if
    /// the host supplies no timer, so an action is never silently lost.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// use aimer_venus::{TimeoutError, Venus};
    ///
    /// let venus = Venus::new();
    /// let scheduled = venus.spawn_delayed(Duration::from_secs(2), || {});
    /// assert_eq!(scheduled, Err(TimeoutError::TimerUnavailable));
    /// ```
    #[inline]
    pub fn spawn_delayed(
        &self,
        duration: Duration,
        action: impl FnOnce() + 'static,
    ) -> Result<crate::TaskId, TimeoutError> {
        self.spawn_delayed_in(crate::ScopeId::ROOT, duration, action)
    }

    /// [`spawn_delayed`](Self::spawn_delayed) owned by `scope`: dropping the
    /// scope cancels the action.
    pub fn spawn_delayed_in(
        &self,
        scope: crate::ScopeId,
        duration: Duration,
        action: impl FnOnce() + 'static,
    ) -> Result<crate::TaskId, TimeoutError> {
        spawn_delayed_on(self.scheduler(), scope, duration, action)
    }

    /// Creates a ticker firing every `period`, first one period from now.
    ///
    /// # Panics
    ///
    /// Panics if `period` is zero.
    #[inline]
    pub fn interval(&self, period: Duration) -> Interval {
        Interval::new(Some(self.scheduler().clone()), period)
    }
}

/// Spawns `action` to run after `duration`, refusing if there is no timer.
pub(crate) fn spawn_delayed_on(
    scheduler: &LocalScheduler,
    scope: crate::ScopeId,
    duration: Duration,
    action: impl FnOnce() + 'static,
) -> Result<crate::TaskId, TimeoutError> {
    let sleep = sleep_on(scheduler, duration);
    if !sleep.is_available() {
        return Err(TimeoutError::TimerUnavailable);
    }
    Ok(scheduler.spawn_in(scope, async move {
        let _ = delayed_after(sleep, action).await;
    }))
}

async fn delayed_after<T>(
    sleep: Sleep,
    action: impl FnOnce() -> T,
) -> Result<T, TimeoutError> {
    sleep.await?;
    Ok(action())
}

/// [`Venus::sleep`] on the runtime installed for this thread.
///
/// Resolves to [`TimeoutError::TimerUnavailable`] when none is installed, which
/// is how a widget under test with no event loop finds out.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
///
/// use aimer_venus::{Venus, sleep};
///
/// let venus = Venus::new();
/// venus.install();
/// venus.spawn(async {
///     let _ = sleep(Duration::from_millis(100)).await;
/// });
/// venus.run_microtasks();
/// Venus::uninstall();
/// ```
#[inline]
pub fn sleep(duration: Duration) -> Sleep {
    Venus::current().map_or_else(|| Sleep::new(None), |venus| venus.sleep(duration))
}

/// [`Venus::sleep_until`] on the runtime installed for this thread.
#[inline]
pub fn sleep_until(deadline: Instant) -> Sleep {
    sleep(deadline.saturating_duration_since(Instant::now()))
}

/// [`Venus::delayed`] on the runtime installed for this thread.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
///
/// use aimer_venus::{Venus, delayed};
///
/// let venus = Venus::new();
/// venus.install();
/// let pending = delayed(Duration::from_millis(500), || 1 + 1);
/// venus.spawn(async move {
///     let _ = pending.await;
/// });
/// Venus::uninstall();
/// ```
#[inline]
pub fn delayed<T, F>(
    duration: Duration,
    action: F,
) -> impl Future<Output = Result<T, TimeoutError>> + use<T, F>
where
    F: FnOnce() -> T,
{
    delayed_after(sleep(duration), action)
}

/// [`Venus::timeout`] on the runtime installed for this thread.
///
/// Reports [`TimeoutError::TimerUnavailable`], without polling `future`, when
/// none is installed.
#[inline]
pub fn timeout<F: Future>(
    duration: Duration,
    future: F,
) -> impl Future<Output = Result<F::Output, TimeoutError>> + use<F> {
    let timer = Venus::current().and_then(|venus| venus.scheduler().sleep(duration));
    crate::timeout::run(future, timer)
}

/// [`Venus::spawn_delayed`] on the runtime installed for this thread.
///
/// Reports [`TimeoutError::TimerUnavailable`] when none is installed.
#[inline]
pub fn spawn_delayed(
    duration: Duration,
    action: impl FnOnce() + 'static,
) -> Result<crate::TaskId, TimeoutError> {
    Venus::current()
        .ok_or(TimeoutError::TimerUnavailable)?
        .spawn_delayed(duration, action)
}

/// [`Venus::interval`] on the runtime installed for this thread.
///
/// Every tick reports [`TimeoutError::TimerUnavailable`] when none is installed.
///
/// # Panics
///
/// Panics if `period` is zero.
#[inline]
pub fn interval(period: Duration) -> Interval {
    Interval::new(
        Venus::current().map(|venus| venus.scheduler().clone()),
        period,
    )
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::time::Duration;

    use super::*;
    use crate::test_host::with_timers;

    #[test]
    fn a_sleep_parks_until_the_host_timer_fires() {
        let (venus, timers) = with_timers();
        let result = Rc::new(Cell::new(None));
        let output = result.clone();
        let sleep = venus.sleep(Duration::from_millis(40));
        venus.spawn(async move { output.set(Some(sleep.await)) });

        venus.run_microtasks();
        assert_eq!(result.get(), None);
        assert!(!venus.has_ready_work(), "a pending sleep must not busy poll");
        assert_eq!(timers.duration(0), Duration::from_millis(40));

        timers.fire(0);
        assert!(venus.has_ready_work());
        venus.run_microtasks();

        assert_eq!(result.get(), Some(Ok(())));
        assert!(timers.state(0).dropped.get(), "the timer is released on expiry");
        assert_eq!(venus.task_count(), 0);
    }

    #[test]
    fn a_sleep_without_a_host_timer_reports_it() {
        let venus = Venus::new();
        let result = Rc::new(Cell::new(None));
        let output = result.clone();
        let sleep = venus.sleep(Duration::from_secs(1));
        venus.spawn(async move { output.set(Some(sleep.await)) });

        venus.run_microtasks();

        assert_eq!(result.get(), Some(Err(TimeoutError::TimerUnavailable)));
    }

    #[test]
    fn dropping_a_sleep_releases_the_timer() {
        let (venus, timers) = with_timers();
        let scope = venus.scope();
        let sleep = venus.sleep(Duration::from_secs(60));
        venus.spawn_in(scope.id(), async move {
            let _ = sleep.await;
        });
        venus.run_microtasks();
        assert!(!timers.state(0).dropped.get());

        drop(scope);

        assert!(timers.state(0).dropped.get());
        assert_eq!(venus.task_count(), 0);
    }

    #[test]
    fn sleeping_until_a_past_deadline_asks_for_no_time() {
        let (venus, timers) = with_timers();
        let past = Instant::now();
        let _sleep = venus.sleep_until(past);

        assert_eq!(timers.duration(0), Duration::ZERO);
    }

    #[test]
    fn sleeping_until_a_future_deadline_asks_for_the_remainder() {
        let (venus, timers) = with_timers();
        let deadline = Instant::now() + Duration::from_secs(30);
        let _sleep = venus.sleep_until(deadline);

        let asked = timers.duration(0);
        assert!(asked <= Duration::from_secs(30));
        assert!(asked > Duration::from_secs(29), "got {asked:?}");
    }

    #[test]
    fn a_delayed_action_runs_after_the_timer_and_not_before() {
        let (venus, timers) = with_timers();
        let ran = Rc::new(Cell::new(false));
        let result = Rc::new(Cell::new(None));
        let flag = ran.clone();
        let output = result.clone();
        let later = venus.delayed(Duration::from_millis(300), move || {
            flag.set(true);
            Rc::new(5)
        });
        venus.spawn(async move { output.set(Some(*later.await.expect("timer").as_ref())) });

        venus.run_microtasks();
        assert!(!ran.get(), "the action waits for the timer");

        timers.fire(0);
        venus.run_microtasks();

        assert!(ran.get());
        assert_eq!(result.get(), Some(5));
    }

    #[test]
    fn a_dropped_delayed_action_never_runs() {
        let (venus, timers) = with_timers();
        let ran = Rc::new(Cell::new(false));
        let flag = ran.clone();
        let scope = venus.scope();
        let later = venus.delayed(Duration::from_secs(1), move || flag.set(true));
        venus.spawn_in(scope.id(), async move {
            let _ = later.await;
        });
        venus.run_microtasks();

        drop(scope);
        timers.fire(0);
        venus.run_microtasks();

        assert!(!ran.get());
    }

    #[test]
    fn a_delayed_action_is_not_run_without_a_timer() {
        let venus = Venus::new();
        let ran = Rc::new(Cell::new(false));
        let flag = ran.clone();
        let result = Rc::new(Cell::new(None));
        let output = result.clone();
        let later = venus.delayed(Duration::ZERO, move || flag.set(true));
        venus.spawn(async move { output.set(Some(later.await)) });

        venus.run_microtasks();

        assert_eq!(result.get(), Some(Err(TimeoutError::TimerUnavailable)));
        assert!(!ran.get());
    }

    #[test]
    fn spawn_delayed_runs_the_action_when_the_timer_fires() {
        let (venus, timers) = with_timers();
        let ran = Rc::new(Cell::new(0));
        let counted = ran.clone();
        let task = venus
            .spawn_delayed(Duration::from_millis(10), move || counted.set(counted.get() + 1))
            .expect("a timer is installed");

        venus.run_microtasks();
        assert_eq!(ran.get(), 0);
        assert!(venus.is_running(task));

        timers.fire(0);
        venus.run_microtasks();

        assert_eq!(ran.get(), 1);
        assert!(!venus.is_running(task));
    }

    #[test]
    fn aborting_a_spawned_delay_cancels_the_action() {
        let (venus, timers) = with_timers();
        let ran = Rc::new(Cell::new(false));
        let flag = ran.clone();
        let task = venus
            .spawn_delayed(Duration::from_millis(10), move || flag.set(true))
            .expect("a timer is installed");
        venus.run_microtasks();

        assert!(venus.abort(task));
        timers.fire(0);
        venus.run_microtasks();

        assert!(!ran.get());
        assert!(timers.state(0).dropped.get());
    }

    #[test]
    fn a_scoped_delay_dies_with_its_scope() {
        let (venus, timers) = with_timers();
        let ran = Rc::new(Cell::new(false));
        let flag = ran.clone();
        let scope = venus.scope();
        venus
            .spawn_delayed_in(scope.id(), Duration::from_millis(10), move || flag.set(true))
            .expect("a timer is installed");
        venus.run_microtasks();

        drop(scope);
        timers.fire(0);
        venus.run_microtasks();

        assert!(!ran.get());
        assert_eq!(venus.task_count(), 0);
    }

    #[test]
    fn spawn_delayed_without_a_timer_refuses_instead_of_losing_the_action() {
        let venus = Venus::new();

        let scheduled = venus.spawn_delayed(Duration::from_secs(1), || {});

        assert_eq!(scheduled, Err(TimeoutError::TimerUnavailable));
        assert_eq!(venus.task_count(), 0);
    }

    #[test]
    fn an_interval_ticks_once_per_period_from_the_previous_deadline() {
        let (venus, timers) = with_timers();
        let period = Duration::from_secs(10);
        let mut ticker = venus.interval(period);
        assert_eq!(ticker.period(), period);
        let ticks = Rc::new(Cell::new(0));
        let counted = ticks.clone();
        venus.spawn(async move {
            while ticker.tick().await.is_ok() {
                counted.set(counted.get() + 1);
            }
        });

        venus.run_microtasks();
        for index in 0..3 {
            // The fake clock does not advance, so each deadline sits one more
            // whole period out: ticks are anchored to deadlines, not to when
            // the previous tick happened to be handled.
            let expected = period * (index as u32 + 1);
            let asked = timers.duration(index);
            assert!(asked <= expected, "tick {index} asked for {asked:?}");
            assert!(
                asked > expected - Duration::from_secs(1),
                "tick {index} asked for {asked:?}"
            );
            timers.fire(index);
            venus.run_microtasks();
            assert_eq!(ticks.get(), index + 1);
        }
    }

    #[test]
    fn an_interval_skips_missed_ticks_instead_of_bursting() {
        let (venus, timers) = with_timers();
        let mut ticker = venus.interval(Duration::from_millis(20));

        // Fall more than two whole periods behind.
        std::thread::sleep(Duration::from_millis(70));
        let _late = ticker.tick();
        let _next = ticker.tick();

        assert_eq!(timers.duration(0), Duration::ZERO, "the overdue tick is due now");
        let after = timers.duration(1);
        assert!(
            after >= Duration::from_millis(5),
            "the next tick must be a period away, not another overdue one: {after:?}"
        );
    }

    #[test]
    #[should_panic(expected = "non-zero period")]
    fn a_zero_interval_is_rejected() {
        let venus = Venus::new();
        let _ = venus.interval(Duration::ZERO);
    }

    #[test]
    fn free_functions_use_the_installed_runtime() {
        let (venus, timers) = with_timers();
        venus.install();
        let ran = Rc::new(Cell::new(false));
        let flag = ran.clone();
        let later = delayed(Duration::from_millis(5), move || flag.set(true));
        let slept = sleep(Duration::from_millis(7));
        venus.spawn(async move {
            slept.await.expect("timer");
            let _ = later.await;
        });
        let timeout_result = Rc::new(Cell::new(None));
        let output = timeout_result.clone();
        let limited = timeout(Duration::from_millis(9), std::future::pending::<()>());
        venus.spawn(async move { output.set(Some(limited.await)) });
        Venus::uninstall();

        venus.run_microtasks();
        let mut seen: Vec<_> = (0..3).map(|index| timers.duration(index)).collect();
        seen.sort();
        assert_eq!(
            seen,
            [5, 7, 9].map(Duration::from_millis),
            "every free function reached the installed host"
        );
        assert!(!ran.get());

        // Fire in request order: delayed (5ms), sleep (7ms), timeout (9ms).
        for index in 0..3 {
            timers.fire(index);
            venus.run_microtasks();
        }
        assert!(ran.get());
        assert_eq!(timeout_result.get(), Some(Err(TimeoutError::Elapsed)));
    }

    #[test]
    fn free_functions_report_a_missing_runtime() {
        assert!(Venus::current().is_none());
        let venus = Venus::new();
        let results = Rc::new(RefCell::new(Vec::new()));
        let log = results.clone();
        let sleeping = sleep(Duration::from_millis(1));
        let mut ticker = interval(Duration::from_millis(1));
        let first_tick = ticker.tick();
        venus.spawn(async move {
            log.borrow_mut().push(sleeping.await);
            log.borrow_mut().push(first_tick.await);
        });
        venus.run_microtasks();

        assert_eq!(
            *results.borrow(),
            [Err(TimeoutError::TimerUnavailable); 2]
        );
        assert_eq!(
            spawn_delayed(Duration::ZERO, || {}),
            Err(TimeoutError::TimerUnavailable)
        );
    }
}
