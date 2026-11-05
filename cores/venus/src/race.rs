//! Waiting for the first of several futures.
//!
//! The losers are dropped, not finished: a request that lost a race to a
//! timeout is cancelled, not left running. Like the other combinators these are
//! polled by the awaiting task and need nothing to be `Send`.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

/// One of two values, as produced by [`race`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Either<L, R> {
    /// The first future won.
    Left(L),
    /// The second future won.
    Right(R),
}

/// The future returned by [`race`].
#[must_use = "futures do nothing unless awaited"]
pub struct Race<A, B> {
    a: A,
    b: B,
}

impl<A: Future, B: Future> Future for Race<A, B> {
    type Output = Either<A::Output, B::Output>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: both fields are pinned structurally and never moved out of
        // `self`; there is no `Drop` impl.
        let this = unsafe { self.get_unchecked_mut() };
        if let Poll::Ready(value) = unsafe { Pin::new_unchecked(&mut this.a) }.poll(cx) {
            return Poll::Ready(Either::Left(value));
        }
        match unsafe { Pin::new_unchecked(&mut this.b) }.poll(cx) {
            Poll::Ready(value) => Poll::Ready(Either::Right(value)),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Resolves with whichever of `a` and `b` finishes first, dropping the other.
///
/// `a` is polled first, so if both are ready at once `a` wins. A future that
/// finished is never polled again.
///
/// # Panics
///
/// Polling the result again after it returned `Ready` may panic, as with any
/// future.
///
/// # Examples
///
/// ```
/// use std::cell::Cell;
/// use std::rc::Rc;
///
/// use aimer_venus::{Either, Venus, race, yield_now};
///
/// let venus = Venus::new();
/// let result = Rc::new(Cell::new(None));
///
/// let output = result.clone();
/// venus.spawn(async move {
///     let slow = async { yield_now().await; yield_now().await; "slow" };
///     let fast = async { yield_now().await; "fast" };
///     output.set(Some(race(slow, fast).await));
/// });
/// venus.run_microtasks();
///
/// assert_eq!(result.get(), Some(Either::Right("fast")));
/// ```
#[inline]
pub fn race<A: Future, B: Future>(a: A, b: B) -> Race<A, B> {
    Race { a, b }
}

/// The future returned by [`race_all`].
#[must_use = "futures do nothing unless awaited"]
pub struct RaceAll<F> {
    items: Pin<Box<[F]>>,
}

impl<F: Future> Future for RaceAll<F> {
    type Output = (usize, F::Output);

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // `RaceAll` is `Unpin`: the futures live behind the `Pin<Box<_>>`.
        let this = self.get_mut();
        // SAFETY: each future is polled in place and never moved.
        let items = unsafe { this.items.as_mut().get_unchecked_mut() };
        for (index, item) in items.iter_mut().enumerate() {
            if let Poll::Ready(value) = unsafe { Pin::new_unchecked(item) }.poll(cx) {
                return Poll::Ready((index, value));
            }
        }
        Poll::Pending
    }
}

/// Resolves with the output of the first future to finish, and its position in
/// the input, dropping the rest.
///
/// Futures are polled in input order, so among futures ready at the same time
/// the earliest listed wins. **An empty set never resolves.**
///
/// # Examples
///
/// ```
/// use std::cell::Cell;
/// use std::rc::Rc;
///
/// use aimer_venus::{Venus, race_all, yield_now};
///
/// let venus = Venus::new();
/// let result = Rc::new(Cell::new(None));
///
/// let output = result.clone();
/// venus.spawn(async move {
///     let mirrors = (0..3u32).map(|n| async move {
///         for _ in 0..(3 - n) {
///             yield_now().await;
///         }
///         n * 10
///     });
///     output.set(Some(race_all(mirrors).await));
/// });
/// venus.run_microtasks();
///
/// assert_eq!(result.get(), Some((2, 20)));
/// ```
pub fn race_all<I>(futures: I) -> RaceAll<I::Item>
where
    I: IntoIterator,
    I::Item: Future,
{
    let items: Box<[I::Item]> = futures.into_iter().collect();
    RaceAll {
        items: Box::into_pin(items),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use super::*;
    use crate::{Venus, yield_now};

    async fn after<T>(steps: u32, value: T) -> T {
        for _ in 0..steps {
            yield_now().await;
        }
        value
    }

    struct DropFlag(Rc<Cell<bool>>);

    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }

    fn run<T: 'static>(future: impl Future<Output = T> + 'static) -> (Option<T>, usize) {
        let venus = Venus::new();
        let slot = Rc::new(RefCell::new(None));
        let output = slot.clone();
        venus.spawn(async move { *output.borrow_mut() = Some(future.await) });
        let polls = venus.run_microtasks();
        let value = slot.borrow_mut().take();
        (value, polls)
    }

    #[test]
    fn the_faster_future_wins_and_the_loser_is_dropped() {
        let dropped = Rc::new(Cell::new(false));
        let flag = DropFlag(dropped.clone());
        let slow = async move {
            let _flag = flag;
            after(20, "slow").await
        };

        let (outcome, polls) = run(race(slow, after(2, "fast")));

        assert_eq!(outcome, Some(Either::Right("fast")));
        assert_eq!(polls, 3, "the slow future did not hold the race open");
        assert!(dropped.get());
    }

    #[test]
    fn the_first_argument_wins_a_tie() {
        let (outcome, _) = run(race(std::future::ready(1), std::future::ready(2)));

        assert_eq!(outcome, Some(Either::Left(1)));
    }

    #[test]
    fn race_accepts_non_send_futures() {
        let shared = Rc::new(Cell::new(0));
        let touched = shared.clone();
        let (outcome, _) = run(race(
            async move {
                touched.set(9);
                Rc::new(1)
            },
            std::future::pending::<()>(),
        ));

        assert!(matches!(outcome, Some(Either::Left(_))));
        assert_eq!(shared.get(), 9);
    }

    #[test]
    fn race_all_reports_the_winner_and_its_position() {
        let (outcome, _) = run(race_all([after(5, 'a'), after(1, 'b'), after(3, 'c')]));

        assert_eq!(outcome, Some((1, 'b')));
    }

    #[test]
    fn race_all_prefers_the_earliest_of_simultaneous_winners() {
        let (outcome, _) = run(race_all([after(2, 'a'), after(2, 'b')]));

        assert_eq!(outcome, Some((0, 'a')));
    }

    #[test]
    fn race_all_drops_the_losers() {
        let dropped = Rc::new(Cell::new(false));
        let flag = DropFlag(dropped.clone());
        let slow: Pin<Box<dyn Future<Output = u8>>> = Box::pin(async move {
            let _flag = flag;
            after(30, 1).await
        });
        let quick: Pin<Box<dyn Future<Output = u8>>> = Box::pin(after(1, 2));

        let (outcome, _) = run(race_all(vec![slow, quick]));

        assert_eq!(outcome, Some((1, 2)));
        assert!(dropped.get());
    }

    #[test]
    fn racing_nothing_never_resolves() {
        let (outcome, _) = run(race_all(Vec::<std::future::Ready<u8>>::new()));

        assert_eq!(outcome, None);
    }
}
