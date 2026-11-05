//! Retrying a fallible async operation with a growing delay.

use std::rc::Rc;
use std::time::Duration;

use crate::delay::sleep_on;
use crate::scheduler::LocalScheduler;
use crate::venus::Venus;

/// How long to wait between attempts, and how many attempts to make.
///
/// There is deliberately no random jitter: Venus depends on no source of
/// randomness, and a UI retrying one request does not need to avoid a thundering
/// herd. The delays are therefore deterministic, which also makes them testable.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
///
/// use aimer_venus::Backoff;
///
/// let backoff = Backoff::exponential(Duration::from_millis(100))
///     .max_delay(Duration::from_millis(350))
///     .attempts(5);
///
/// assert_eq!(backoff.delay_after(1), Duration::from_millis(100));
/// assert_eq!(backoff.delay_after(2), Duration::from_millis(200));
/// assert_eq!(backoff.delay_after(3), Duration::from_millis(350), "capped");
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Backoff {
    initial: Duration,
    factor: u32,
    max_delay: Duration,
    attempts: u32,
}

impl Backoff {
    /// The same `delay` between every attempt. Three attempts by default.
    #[inline]
    pub const fn fixed(delay: Duration) -> Self {
        Self {
            initial: delay,
            factor: 1,
            max_delay: delay,
            attempts: 3,
        }
    }

    /// A delay of `initial` that doubles after each failure, capped at 30
    /// seconds. Three attempts by default.
    #[inline]
    pub const fn exponential(initial: Duration) -> Self {
        Self {
            initial,
            factor: 2,
            max_delay: Duration::from_secs(30),
            attempts: 3,
        }
    }

    /// Multiplies the delay by `factor` after each failure.
    #[inline]
    pub const fn factor(mut self, factor: u32) -> Self {
        self.factor = factor;
        self
    }

    /// The longest any single wait may be.
    #[inline]
    pub const fn max_delay(mut self, max_delay: Duration) -> Self {
        self.max_delay = max_delay;
        self
    }

    /// The total number of tries, including the first. At least one is made.
    #[inline]
    pub const fn attempts(mut self, attempts: u32) -> Self {
        self.attempts = if attempts == 0 { 1 } else { attempts };
        self
    }

    /// The wait after the `failures`-th failure (the first failure is `1`).
    pub fn delay_after(&self, failures: u32) -> Duration {
        let growth = self
            .factor
            .checked_pow(failures.saturating_sub(1))
            .unwrap_or(u32::MAX);
        self.initial.saturating_mul(growth).min(self.max_delay)
    }

    /// The total number of tries.
    #[inline]
    pub const fn max_attempts(&self) -> u32 {
        self.attempts
    }
}

impl Venus {
    /// Runs `operation` until it succeeds or the attempts run out, waiting
    /// between tries.
    ///
    /// Resolves to the first success, or to the *last* error. If the host has
    /// no timer, there is nothing to wait with and the last error is returned
    /// at once rather than retrying in a tight loop. Dropping the future stops
    /// the retries.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::cell::Cell;
    /// use std::rc::Rc;
    /// use std::time::Duration;
    ///
    /// use aimer_venus::{Backoff, Venus};
    ///
    /// let venus = Venus::new();
    /// let tries = Rc::new(Cell::new(0));
    ///
    /// let counted = tries.clone();
    /// let runtime = venus.clone();
    /// let outcome = venus.run_until(async move {
    ///     runtime
    ///         .retry(Backoff::fixed(Duration::ZERO), || {
    ///             counted.set(counted.get() + 1);
    ///             async { Err::<(), _>("offline") }
    ///         })
    ///         .await
    /// });
    ///
    /// // This runtime has no host timer, so the first failure is final.
    /// assert_eq!(outcome, Err("offline"));
    /// assert_eq!(tries.get(), 1);
    /// ```
    #[inline]
    pub fn retry<T, E, F, Fut>(
        &self,
        backoff: Backoff,
        operation: F,
    ) -> impl Future<Output = Result<T, E>> + use<T, E, F, Fut>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        self.retry_when(backoff, operation, |_| true)
    }

    /// [`retry`](Self::retry), but only for errors `should_retry` approves; any
    /// other error is returned at once.
    #[inline]
    pub fn retry_when<T, E, F, Fut, P>(
        &self,
        backoff: Backoff,
        operation: F,
        should_retry: P,
    ) -> impl Future<Output = Result<T, E>> + use<T, E, F, Fut, P>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T, E>>,
        P: Fn(&E) -> bool,
    {
        retry_on(Some(self.scheduler().clone()), backoff, operation, should_retry)
    }
}

/// [`Venus::retry`] on the runtime installed for this thread.
///
/// With no runtime there is no timer, so the operation gets a single attempt.
#[inline]
pub fn retry<T, E, F, Fut>(
    backoff: Backoff,
    operation: F,
) -> impl Future<Output = Result<T, E>> + use<T, E, F, Fut>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    retry_when(backoff, operation, |_| true)
}

/// [`Venus::retry_when`] on the runtime installed for this thread.
#[inline]
pub fn retry_when<T, E, F, Fut, P>(
    backoff: Backoff,
    operation: F,
    should_retry: P,
) -> impl Future<Output = Result<T, E>> + use<T, E, F, Fut, P>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    P: Fn(&E) -> bool,
{
    // Read now, not at first poll, like every other free constructor here.
    let scheduler = Venus::current().map(|venus| venus.scheduler().clone());
    retry_on(scheduler, backoff, operation, should_retry)
}

/// The retry loop. `scheduler` is where timers come from; without one the first
/// failure is final.
async fn retry_on<T, E, F, Fut, P>(
    scheduler: Option<Rc<LocalScheduler>>,
    backoff: Backoff,
    mut operation: F,
    should_retry: P,
) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    P: Fn(&E) -> bool,
{
    let mut attempt = 1;
    loop {
        let error = match operation().await {
            Ok(value) => return Ok(value),
            Err(error) => error,
        };
        if attempt >= backoff.max_attempts() || !should_retry(&error) {
            return Err(error);
        }
        let Some(scheduler) = &scheduler else {
            return Err(error);
        };
        if sleep_on(scheduler, backoff.delay_after(attempt)).await.is_err() {
            return Err(error);
        }
        attempt += 1;
    }
}

// `run_until` sleeps the calling thread, which a browser cannot do.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use super::*;
    use crate::test_host::{Timers, with_timers};

    /// Fires every timer as soon as it is requested, recording the delays.
    fn drive_with_timers<T: 'static>(
        venus: &Rc<Venus>,
        timers: &Rc<Timers>,
        future: impl Future<Output = T> + 'static,
    ) -> T {
        let handle = venus.spawn_with_handle(future);
        let mut fired = 0;
        while !handle.is_finished() {
            venus.run_until_stalled();
            while fired < timers.len() {
                timers.fire(fired);
                fired += 1;
            }
            venus.run_until_stalled();
        }
        venus.run_until(async move { handle.await.unwrap() })
    }

    #[test]
    fn delays_double_up_to_the_cap() {
        let backoff = Backoff::exponential(Duration::from_millis(100)).max_delay(Duration::from_millis(500));

        let delays: Vec<_> = (1..=5).map(|n| backoff.delay_after(n)).collect();

        assert_eq!(
            delays,
            [100, 200, 400, 500, 500].map(Duration::from_millis)
        );
    }

    #[test]
    fn a_huge_failure_count_saturates_instead_of_overflowing() {
        let backoff = Backoff::exponential(Duration::from_secs(1));

        assert_eq!(backoff.delay_after(10_000), Duration::from_secs(30));
    }

    #[test]
    fn fixed_backoff_never_grows() {
        let backoff = Backoff::fixed(Duration::from_millis(40));

        assert_eq!(backoff.delay_after(1), backoff.delay_after(9));
    }

    #[test]
    fn zero_attempts_still_means_one_try() {
        assert_eq!(Backoff::fixed(Duration::ZERO).attempts(0).max_attempts(), 1);
    }

    #[test]
    fn retries_until_success_waiting_the_backoff_between_tries() {
        let (venus, timers) = with_timers();
        let tries = Rc::new(Cell::new(0));
        let counted = tries.clone();
        let backoff = Backoff::exponential(Duration::from_millis(10)).attempts(5);

        let outcome = drive_with_timers(&venus, &timers, {
            let retried = venus.retry(backoff, move || {
                counted.set(counted.get() + 1);
                let attempt = counted.get();
                async move { if attempt < 3 { Err("flaky") } else { Ok(attempt) } }
            });
            retried
        });

        assert_eq!(outcome, Ok(3));
        assert_eq!(tries.get(), 3);
        assert_eq!(timers.duration(0), Duration::from_millis(10));
        assert_eq!(timers.duration(1), Duration::from_millis(20));
    }

    #[test]
    fn gives_up_with_the_last_error_when_attempts_run_out() {
        let (venus, timers) = with_timers();
        let tries = Rc::new(Cell::new(0));
        let counted = tries.clone();

        let outcome = drive_with_timers(
            &venus,
            &timers,
            venus.retry(Backoff::fixed(Duration::ZERO).attempts(3), move || {
                counted.set(counted.get() + 1);
                let n = counted.get();
                async move { Err::<(), _>(n) }
            }),
        );

        assert_eq!(outcome, Err(3));
        assert_eq!(tries.get(), 3);
    }

    #[test]
    fn retry_when_stops_at_an_error_it_does_not_approve() {
        let (venus, timers) = with_timers();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let log = seen.clone();

        let outcome = drive_with_timers(
            &venus,
            &timers,
            venus.retry_when(
                Backoff::fixed(Duration::ZERO).attempts(10),
                move || {
                    let n = log.borrow().len();
                    log.borrow_mut().push(n);
                    async move { Err::<(), _>(if n < 2 { "transient" } else { "fatal" }) }
                },
                |error| *error == "transient",
            ),
        );

        assert_eq!(outcome, Err("fatal"));
        assert_eq!(seen.borrow().len(), 3, "two retries, then the fatal one");
    }

    #[test]
    fn without_a_timer_the_first_failure_is_final() {
        let venus = Venus::new();
        let tries = Rc::new(Cell::new(0));
        let counted = tries.clone();

        let outcome = venus.run_until(venus.retry(Backoff::fixed(Duration::ZERO).attempts(9), move || {
            counted.set(counted.get() + 1);
            async { Err::<(), _>("down") }
        }));

        assert_eq!(outcome, Err("down"));
        assert_eq!(tries.get(), 1, "no timer means no waiting, and no spinning");
    }

    #[test]
    fn dropping_the_retry_stops_further_attempts() {
        let (venus, timers) = with_timers();
        let tries = Rc::new(Cell::new(0));
        let counted = tries.clone();
        let handle = venus.spawn_with_handle(venus.retry(
            Backoff::fixed(Duration::from_secs(60)).attempts(5),
            move || {
                counted.set(counted.get() + 1);
                async { Err::<(), _>("down") }
            },
        ));
        venus.run_until_stalled();
        assert_eq!(tries.get(), 1);

        drop(handle);
        timers.fire(0);
        venus.run_until_stalled();

        assert_eq!(tries.get(), 1);
        assert_eq!(venus.task_count(), 0);
    }

    #[test]
    fn the_free_function_gets_one_attempt_without_a_runtime() {
        let venus = Venus::new();
        let tries = Rc::new(Cell::new(0));
        let counted = tries.clone();

        let outcome = venus.run_until(retry(Backoff::fixed(Duration::ZERO), move || {
            counted.set(counted.get() + 1);
            async { Err::<(), _>("x") }
        }));

        assert_eq!(outcome, Err("x"));
        assert_eq!(tries.get(), 1);
    }
}
