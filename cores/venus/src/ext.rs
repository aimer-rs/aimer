//! Method-call forms of the combinators, for any future.

use std::time::Duration;

use crate::handle::JoinHandle;
use crate::join::IntoJoin;
use crate::race::Race;
use crate::task::ScopeId;
use crate::timeout::TimeoutError;
use crate::venus::Venus;

/// Chainable helpers available on every [`Future`].
///
/// Import the trait to use them:
///
/// ```
/// use std::time::Duration;
///
/// use aimer_venus::{FutureExt, Venus};
///
/// let venus = Venus::new();
/// venus.install();
///
/// // Both bound the wait and give the work a home, in one expression.
/// let load = async { 1 }.timeout(Duration::from_secs(5));
/// let handle = async { 2 }.into_handle().expect("a runtime is installed");
/// # drop((load, handle));
/// Venus::uninstall();
/// ```
pub trait FutureExt: Future + Sized {
    /// Limits how long this future may remain pending; see
    /// [`Venus::timeout`](crate::Venus::timeout), whose runtime this uses —
    /// the one installed for the calling thread.
    ///
    /// The deadline starts now, not at first poll.
    #[inline]
    fn timeout(
        self,
        duration: Duration,
    ) -> impl Future<Output = Result<Self::Output, TimeoutError>> {
        crate::delay::timeout(duration, self)
    }

    /// Resolves with whichever of `self` and `other` finishes first, dropping
    /// the other; see [`race`](crate::race).
    #[inline]
    fn race<B: Future>(self, other: B) -> Race<Self, B> {
        crate::race(self, other)
    }

    /// Runs `self` and `other` together and resolves to both outputs; see
    /// [`join`](crate::join).
    #[inline]
    fn join<B: Future>(self, other: B) -> <(Self, B) as IntoJoin>::Join {
        crate::join((self, other))
    }

    /// Spawns this future on the runtime installed for the calling thread and
    /// returns the handle that owns it; `None` when there is none.
    ///
    /// Dropping the handle cancels the future; see [`JoinHandle`].
    #[inline]
    fn into_handle(self) -> Option<JoinHandle<Self::Output>>
    where
        Self: 'static,
        Self::Output: 'static,
    {
        crate::spawn_with_handle(self)
    }

    /// [`into_handle`](Self::into_handle) owned by `scope`: ending the scope
    /// cancels the future.
    #[inline]
    fn into_handle_in(self, scope: ScopeId) -> Option<JoinHandle<Self::Output>>
    where
        Self: 'static,
        Self::Output: 'static,
    {
        Venus::current().map(|venus| venus.spawn_with_handle_in(scope, self))
    }
}

impl<F: Future> FutureExt for F {}

// `run_until` sleeps the calling thread, which a browser cannot do.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::Duration;

    use super::*;
    use crate::race::Either;
    use crate::test_host::with_timers;
    use crate::yield_now;

    #[test]
    fn timeout_uses_the_installed_runtimes_timer() {
        let (venus, timers) = with_timers();
        venus.install();
        let limited = std::future::pending::<()>().timeout(Duration::from_secs(2));
        Venus::uninstall();

        let handle = venus.spawn_with_handle(limited);
        venus.run_until_stalled();
        assert_eq!(timers.duration(0), Duration::from_secs(2));
        timers.fire(0);

        assert_eq!(
            venus.run_until(async move { handle.await.unwrap() }),
            Err(TimeoutError::Elapsed)
        );
    }

    #[test]
    fn race_and_join_chain() {
        let venus = Venus::new();
        let fast = async {
            yield_now().await;
            "fast"
        };
        let slow = async {
            yield_now().await;
            yield_now().await;
            "slow"
        };

        assert_eq!(venus.run_until(fast.race(slow)), Either::Left("fast"));
        assert_eq!(venus.run_until(async { 1 }.join(async { "two" })), (1, "two"));
    }

    #[test]
    fn into_handle_owns_the_future() {
        let venus = Venus::new();
        venus.install();
        let ran = Rc::new(Cell::new(false));
        let flag = ran.clone();
        let handle = async move { flag.set(true) }.into_handle().expect("a runtime is installed");
        Venus::uninstall();

        drop(handle);
        venus.run_until_stalled();

        assert!(!ran.get(), "dropping the handle cancelled it");
    }

    #[test]
    fn into_handle_in_ends_with_the_scope() {
        let venus = Venus::new();
        venus.install();
        let scope = venus.scope();
        let handle = std::future::pending::<u8>()
            .into_handle_in(scope.id())
            .expect("a runtime is installed");
        Venus::uninstall();

        drop(scope);

        assert!(handle.is_finished());
    }

    #[test]
    fn into_handle_needs_a_runtime() {
        assert!(async {}.into_handle().is_none());
    }
}
