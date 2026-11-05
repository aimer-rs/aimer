//! Running several futures at once and waiting for all of them.
//!
//! These are plain combinators: they are polled by the task that awaits them,
//! so none of the futures needs to be `Send`, and nothing is spawned or boxed
//! per future. Dropping a combinator drops every future still inside it, which
//! is how scope cancellation reaches them.

use std::future::Future;
use std::mem;
use std::pin::Pin;
use std::task::{Context, Poll};

/// One slot of a combinator: a future, then its output, then nothing.
enum MaybeDone<F: Future> {
    Pending(F),
    Done(F::Output),
    Taken,
}

impl<F: Future> MaybeDone<F> {
    #[inline]
    fn new(future: F) -> Self {
        Self::Pending(future)
    }

    /// Polls the future if it is still running, returning whether it is done.
    fn poll_step(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> bool {
        // SAFETY: the future inside `Pending` is pinned structurally. It is only
        // ever polled in place here, and dropped in place by `Pin::set` below;
        // it is never moved out of the slot.
        let output = match unsafe { self.as_mut().get_unchecked_mut() } {
            Self::Pending(future) => match unsafe { Pin::new_unchecked(future) }.poll(cx) {
                Poll::Ready(output) => output,
                Poll::Pending => return false,
            },
            Self::Done(_) => return true,
            Self::Taken => panic!("a joined future was polled after its output was taken"),
        };
        self.set(Self::Done(output));
        true
    }

    /// Moves the output out. Only a finished slot has one.
    fn take_output(&mut self) -> F::Output {
        // Checked first: replacing a `Pending` slot would move a pinned future.
        assert!(
            matches!(self, Self::Done(_)),
            "a joined future's output was taken before it finished"
        );
        match mem::replace(self, Self::Taken) {
            Self::Done(output) => output,
            _ => unreachable!("checked to be Done above"),
        }
    }
}

impl<F, T, E> MaybeDone<F>
where
    F: Future<Output = Result<T, E>>,
{
    /// Moves the error out if the future finished with one.
    fn take_error(&mut self) -> Option<E> {
        if !matches!(self, Self::Done(Err(_))) {
            return None;
        }
        match mem::replace(self, Self::Taken) {
            Self::Done(Err(error)) => Some(error),
            _ => unreachable!("checked to be Done(Err) above"),
        }
    }
}

/// A tuple of futures that can be run to completion together.
///
/// Implemented for tuples of one to eight futures; see [`join`].
pub trait IntoJoin {
    /// The future that drives every member.
    type Join: Future;

    /// Wraps the members so they are polled together.
    fn into_join(self) -> Self::Join;
}

/// A tuple of fallible futures that can be run together, stopping at the first
/// error; see [`try_join`].
pub trait IntoTryJoin {
    /// The future that drives every member.
    type Join: Future;

    /// Wraps the members so they are polled together.
    fn into_try_join(self) -> Self::Join;
}

/// Polls every future in `futures` concurrently, resolving to all their outputs.
///
/// The futures are polled in order whenever the task wakes, so a future listed
/// first sees a shared wake first. They run on the awaiting task and are not
/// `Send`-bound. Prefer the [`join!`](crate::join!) macro for the call site.
///
/// # Panics
///
/// Polling the result again after it returned `Ready` panics.
///
/// # Examples
///
/// ```
/// use std::cell::Cell;
/// use std::rc::Rc;
///
/// use aimer_venus::{Venus, join, yield_now};
///
/// let venus = Venus::new();
/// let result = Rc::new(Cell::new(None));
///
/// let output = result.clone();
/// venus.spawn(async move {
///     let slow = async { yield_now().await; 1 };
///     let fast = async { 2 };
///     output.set(Some(join((slow, fast)).await));
/// });
/// venus.run_microtasks();
///
/// assert_eq!(result.get(), Some((1, 2)));
/// ```
#[inline]
pub fn join<T: IntoJoin>(futures: T) -> T::Join {
    futures.into_join()
}

/// Polls every future in `futures` concurrently, resolving to all the `Ok`
/// values, or to the first `Err` as soon as one appears.
///
/// The futures still running when an error arrives are dropped with the result,
/// which cancels them. All the futures must share one error type. Prefer the
/// [`try_join!`](crate::try_join!) macro for the call site.
///
/// # Panics
///
/// Polling the result again after it returned `Ready` panics.
///
/// # Examples
///
/// ```
/// use std::cell::Cell;
/// use std::rc::Rc;
///
/// use aimer_venus::{Venus, try_join};
///
/// let venus = Venus::new();
/// let result = Rc::new(Cell::new(None));
///
/// let output = result.clone();
/// venus.spawn(async move {
///     let a = async { Ok::<_, &str>(1) };
///     let b = async { Err::<u8, _>("no network") };
///     output.set(Some(try_join((a, b)).await));
/// });
/// venus.run_microtasks();
///
/// assert_eq!(result.get(), Some(Err("no network")));
/// ```
#[inline]
pub fn try_join<T: IntoTryJoin>(futures: T) -> T::Join {
    futures.into_try_join()
}

macro_rules! tuple_join {
    ($join:ident, $try_join:ident; $($F:ident $T:ident $f:ident),+) => {
        #[doc = concat!("The future returned by [`join`] for a tuple of ", stringify!($($F),+), ".")]
        pub struct $join<$($F: Future),+> {
            $($f: MaybeDone<$F>,)+
        }

        impl<$($F: Future),+> Future for $join<$($F),+> {
            type Output = ($($F::Output,)+);

            fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                // SAFETY: every field is pinned structurally and none is moved
                // out of `self`; `take_output` only moves finished outputs,
                // which are not pinned. There is no `Drop` impl.
                let this = unsafe { self.get_unchecked_mut() };
                let mut done = true;
                // `&=` rather than `&&`: every future must be polled each time.
                $(done &= unsafe { Pin::new_unchecked(&mut this.$f) }.poll_step(cx);)+
                if done {
                    Poll::Ready(($(this.$f.take_output(),)+))
                } else {
                    Poll::Pending
                }
            }
        }

        impl<$($F: Future),+> IntoJoin for ($($F,)+) {
            type Join = $join<$($F),+>;

            #[inline]
            fn into_join(self) -> Self::Join {
                let ($($f,)+) = self;
                $join { $($f: MaybeDone::new($f),)+ }
            }
        }

        #[doc = concat!("The future returned by [`try_join`] for a tuple of ", stringify!($($F),+), ".")]
        pub struct $try_join<$($F: Future),+> {
            $($f: MaybeDone<$F>,)+
        }

        impl<E, $($F, $T),+> Future for $try_join<$($F),+>
        where
            $($F: Future<Output = Result<$T, E>>,)+
        {
            type Output = Result<($($T,)+), E>;

            fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                // SAFETY: as for the infallible join above.
                let this = unsafe { self.get_unchecked_mut() };
                let mut done = true;
                $(
                    done &= unsafe { Pin::new_unchecked(&mut this.$f) }.poll_step(cx);
                    if let Some(error) = this.$f.take_error() {
                        return Poll::Ready(Err(error));
                    }
                )+
                if !done {
                    return Poll::Pending;
                }
                Poll::Ready(Ok(($(
                    match this.$f.take_output() {
                        Ok(value) => value,
                        Err(error) => return Poll::Ready(Err(error)),
                    },
                )+)))
            }
        }

        impl<E, $($F, $T),+> IntoTryJoin for ($($F,)+)
        where
            $($F: Future<Output = Result<$T, E>>,)+
        {
            type Join = $try_join<$($F),+>;

            #[inline]
            fn into_try_join(self) -> Self::Join {
                let ($($f,)+) = self;
                $try_join { $($f: MaybeDone::new($f),)+ }
            }
        }
    };
}

tuple_join!(Join1, TryJoin1; F1 T1 f1);
tuple_join!(Join2, TryJoin2; F1 T1 f1, F2 T2 f2);
tuple_join!(Join3, TryJoin3; F1 T1 f1, F2 T2 f2, F3 T3 f3);
tuple_join!(Join4, TryJoin4; F1 T1 f1, F2 T2 f2, F3 T3 f3, F4 T4 f4);
tuple_join!(Join5, TryJoin5; F1 T1 f1, F2 T2 f2, F3 T3 f3, F4 T4 f4, F5 T5 f5);
tuple_join!(Join6, TryJoin6; F1 T1 f1, F2 T2 f2, F3 T3 f3, F4 T4 f4, F5 T5 f5, F6 T6 f6);
tuple_join!(Join7, TryJoin7; F1 T1 f1, F2 T2 f2, F3 T3 f3, F4 T4 f4, F5 T5 f5, F6 T6 f6, F7 T7 f7);
tuple_join!(
    Join8, TryJoin8;
    F1 T1 f1, F2 T2 f2, F3 T3 f3, F4 T4 f4, F5 T5 f5, F6 T6 f6, F7 T7 f7, F8 T8 f8
);

/// Waits for several futures at once, resolving to a tuple of their outputs.
///
/// Accepts one to eight futures of different types. Equivalent to
/// [`join`]`((a, b, ..))`.
///
/// # Examples
///
/// ```
/// use std::cell::Cell;
/// use std::rc::Rc;
///
/// use aimer_venus::{Venus, join};
///
/// let venus = Venus::new();
/// let result = Rc::new(Cell::new(None));
///
/// let output = result.clone();
/// venus.spawn(async move {
///     let (width, name) = join!(async { 320 }, async { "avatar" }).await;
///     output.set(Some((width, name)));
/// });
/// venus.run_microtasks();
///
/// assert_eq!(result.get(), Some((320, "avatar")));
/// ```
#[macro_export]
macro_rules! join {
    ($($future:expr),+ $(,)?) => {
        $crate::join(($($future,)+))
    };
}

/// Waits for several fallible futures at once, resolving to a tuple of their
/// `Ok` values or to the first `Err`.
///
/// Equivalent to [`try_join`]`((a, b, ..))`; see it for the cancellation rules.
///
/// # Examples
///
/// ```
/// use std::cell::Cell;
/// use std::rc::Rc;
///
/// use aimer_venus::{Venus, try_join};
///
/// let venus = Venus::new();
/// let result = Rc::new(Cell::new(None));
///
/// let output = result.clone();
/// venus.spawn(async move {
///     let both = try_join!(async { Ok::<_, ()>(1) }, async { Ok("two") }).await;
///     output.set(Some(both));
/// });
/// venus.run_microtasks();
///
/// assert_eq!(result.get(), Some(Ok((1, "two"))));
/// ```
#[macro_export]
macro_rules! try_join {
    ($($future:expr),+ $(,)?) => {
        $crate::try_join(($($future,)+))
    };
}

/// The future returned by [`join_all`].
#[must_use = "futures do nothing unless awaited"]
pub struct JoinAll<F: Future> {
    // One allocation for the whole set. The slice is never resized after
    // construction, so the futures stay where they were first polled.
    items: Pin<Box<[MaybeDone<F>]>>,
}

impl<F: Future> Future for JoinAll<F> {
    type Output = Vec<F::Output>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // `JoinAll` is `Unpin`: the futures live behind the `Pin<Box<_>>`.
        let this = self.get_mut();
        // SAFETY: the slice is only used to poll in place or to move out
        // finished outputs; no `Pending` slot is ever moved.
        let items = unsafe { this.items.as_mut().get_unchecked_mut() };
        let mut done = true;
        for item in items.iter_mut() {
            done &= unsafe { Pin::new_unchecked(item) }.poll_step(cx);
        }
        if done {
            Poll::Ready(items.iter_mut().map(MaybeDone::take_output).collect())
        } else {
            Poll::Pending
        }
    }
}

/// Polls every future in `futures` concurrently, resolving to their outputs in
/// the order the futures were given.
///
/// An empty set resolves immediately to an empty `Vec`.
///
/// Every wake polls each future that is still running, so the cost of a wake
/// grows with the number of futures *not yet finished*. That is the right
/// trade for the handful of futures a screen waits on; thousands of long-lived
/// futures belong in separate tasks.
///
/// # Panics
///
/// Polling the result again after it returned `Ready` panics.
///
/// # Examples
///
/// ```
/// use std::cell::RefCell;
/// use std::rc::Rc;
///
/// use aimer_venus::{Venus, join_all, yield_now};
///
/// let venus = Venus::new();
/// let result = Rc::new(RefCell::new(Vec::new()));
///
/// let output = result.clone();
/// venus.spawn(async move {
///     let loads = (0..3u32).map(|n| async move {
///         for _ in 0..(3 - n) {
///             yield_now().await;
///         }
///         n
///     });
///     *output.borrow_mut() = join_all(loads).await;
/// });
/// venus.run_microtasks();
///
/// // Results follow the input order, not the finishing order.
/// assert_eq!(*result.borrow(), [0, 1, 2]);
/// ```
pub fn join_all<I>(futures: I) -> JoinAll<I::Item>
where
    I: IntoIterator,
    I::Item: Future,
{
    let items: Box<[MaybeDone<I::Item>]> = futures.into_iter().map(MaybeDone::new).collect();
    JoinAll {
        items: Box::into_pin(items),
    }
}

/// The future returned by [`try_join_all`].
#[must_use = "futures do nothing unless awaited"]
pub struct TryJoinAll<F: Future> {
    items: Pin<Box<[MaybeDone<F>]>>,
}

impl<F, T, E> Future for TryJoinAll<F>
where
    F: Future<Output = Result<T, E>>,
{
    type Output = Result<Vec<T>, E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        // SAFETY: as for `JoinAll`.
        let items = unsafe { this.items.as_mut().get_unchecked_mut() };
        let mut done = true;
        for item in items.iter_mut() {
            done &= unsafe { Pin::new_unchecked(&mut *item) }.poll_step(cx);
            if let Some(error) = item.take_error() {
                return Poll::Ready(Err(error));
            }
        }
        if !done {
            return Poll::Pending;
        }
        let mut values = Vec::with_capacity(items.len());
        for item in items.iter_mut() {
            match item.take_output() {
                Ok(value) => values.push(value),
                Err(error) => return Poll::Ready(Err(error)),
            }
        }
        Poll::Ready(Ok(values))
    }
}

/// Polls every future in `futures` concurrently, resolving to all the `Ok`
/// values in input order, or to the first `Err` as soon as one appears.
///
/// The futures still running when an error arrives are dropped with the
/// result. See [`join_all`] for the cost model.
///
/// # Panics
///
/// Polling the result again after it returned `Ready` panics.
///
/// # Examples
///
/// ```
/// use std::cell::Cell;
/// use std::rc::Rc;
///
/// use aimer_venus::{Venus, try_join_all};
///
/// let venus = Venus::new();
/// let result = Rc::new(Cell::new(None));
///
/// let output = result.clone();
/// venus.spawn(async move {
///     let parse = |text: &'static str| async move { text.parse::<u32>() };
///     output.set(Some(try_join_all([parse("1"), parse("2")]).await.is_ok()));
/// });
/// venus.run_microtasks();
///
/// assert_eq!(result.get(), Some(true));
/// ```
pub fn try_join_all<I, T, E>(futures: I) -> TryJoinAll<I::Item>
where
    I: IntoIterator,
    I::Item: Future<Output = Result<T, E>>,
{
    let items: Box<[MaybeDone<I::Item>]> = futures.into_iter().map(MaybeDone::new).collect();
    TryJoinAll {
        items: Box::into_pin(items),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use crate::{Venus, join_all, try_join_all, yield_now};

    /// Finishes after being polled `steps` more times, yielding each time.
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

    /// Runs `future` as a task to completion, returning its output and how many
    /// times the task was polled. Concurrency shows up as a smaller count.
    fn run<T: 'static>(future: impl Future<Output = T> + 'static) -> (T, usize) {
        let venus = Venus::new();
        let slot = Rc::new(RefCell::new(None));
        let output = slot.clone();
        venus.spawn(async move { *output.borrow_mut() = Some(future.await) });
        let polls = venus.run_microtasks();
        let value = slot.borrow_mut().take().expect("the future finished");
        (value, polls)
    }

    #[test]
    fn join_returns_every_output_in_argument_order() {
        let ((a, b, c), _) = run(async { join!(after(2, "a"), after(0, 7_u8), after(1, 'c')).await });

        assert_eq!((a, b, c), ("a", 7, 'c'));
    }

    #[test]
    fn join_runs_its_futures_concurrently() {
        let (_, polls) = run(async { join!(after(3, ()), after(3, ()), after(3, ())).await });

        // Three yields plus the finishing poll. Run one after another it is ten.
        assert_eq!(polls, 4);
    }

    #[test]
    fn join_accepts_non_send_futures_and_outputs() {
        let (value, _) = run(async {
            let shared = Rc::new(Cell::new(1));
            let (a, b) = join!(
                async {
                    yield_now().await;
                    shared.set(shared.get() + 1);
                    Rc::clone(&shared)
                },
                async { shared.get() },
            )
            .await;
            (a.get(), b)
        });

        assert_eq!(value, (2, 1));
    }

    #[test]
    fn join_takes_a_single_future() {
        let ((only,), _) = run(async { join!(after(1, 5)).await });

        assert_eq!(only, 5);
    }

    #[test]
    fn dropping_a_join_drops_the_futures_inside_it() {
        let dropped = Rc::new(Cell::new(false));
        let flag = DropFlag(dropped.clone());
        let pending = async move {
            let _flag = flag;
            std::future::pending::<()>().await;
        };
        let venus = Venus::new();
        let scope = venus.scope();
        venus.spawn_in(scope.id(), async move {
            join!(pending, after(1, ())).await;
        });
        venus.run_microtasks();
        assert!(!dropped.get());

        drop(scope);

        assert!(dropped.get());
    }

    #[test]
    fn try_join_collects_every_success() {
        let (result, _) = run(async {
            try_join!(async { Ok::<_, String>(1) }, async { Ok("two") }, after(1, Ok(3.0)))
                .await
        });

        assert_eq!(result, Ok((1, "two", 3.0)));
    }

    #[test]
    fn try_join_stops_at_the_first_error_and_drops_the_rest() {
        let dropped = Rc::new(Cell::new(false));
        let flag = DropFlag(dropped.clone());
        let slow = async move {
            let _flag = flag;
            after(50, ()).await;
            Ok::<_, &str>(())
        };

        let (result, polls) = run(async move {
            let outcome = try_join!(slow, async { Err::<(), _>("boom") }).await;
            outcome
        });

        assert_eq!(result, Err("boom"));
        assert_eq!(polls, 1, "the error did not wait for the slow future");
        assert!(dropped.get(), "the future that lost was cancelled");
    }

    #[test]
    fn try_join_reports_the_error_that_arrived_first_not_the_first_listed() {
        let (result, _) = run(async {
            try_join!(
                async {
                    after(3, ()).await;
                    Err::<(), _>("late")
                },
                async {
                    after(1, ()).await;
                    Err::<(), _>("early")
                },
            )
            .await
        });

        assert_eq!(result, Err("early"));
    }

    #[test]
    fn join_all_keeps_input_order_whatever_finishes_first() {
        let (values, polls) = run(async {
            join_all((0..5_u32).map(|n| after(4 - n, n))).await
        });

        assert_eq!(values, [0, 1, 2, 3, 4]);
        assert_eq!(polls, 5, "concurrent: the slowest sets the pace");
    }

    #[test]
    fn join_all_of_nothing_is_ready_at_once() {
        let (values, polls) = run(async { join_all(Vec::<std::future::Ready<u8>>::new()).await });

        assert!(values.is_empty());
        assert_eq!(polls, 1);
    }

    #[test]
    fn join_all_accepts_non_send_outputs() {
        let (values, _) = run(async {
            let cells = [Rc::new(1), Rc::new(2)];
            join_all(cells.into_iter().map(|cell| after(1, cell))).await
        });

        assert_eq!(values.iter().map(|cell| **cell).sum::<i32>(), 3);
    }

    #[test]
    fn try_join_all_returns_values_in_order() {
        let (result, _) = run(async {
            try_join_all((0..4_u32).map(|n| after(3 - n, Ok::<_, ()>(n)))).await
        });

        assert_eq!(result, Ok(vec![0, 1, 2, 3]));
    }

    #[test]
    fn try_join_all_short_circuits_and_cancels_the_remainder() {
        let dropped = Rc::new(Cell::new(false));
        let flag = DropFlag(dropped.clone());
        let slow = Box::pin(async move {
            let _flag = flag;
            after(50, ()).await;
            Ok::<u8, &str>(1)
        });
        let failing = Box::pin(async {
            after(1, ()).await;
            Err("failed")
        });

        let (result, polls) = run(async move {
            let futures: Vec<std::pin::Pin<Box<dyn Future<Output = Result<u8, &str>>>>> =
                vec![slow, failing];
            try_join_all(futures).await
        });

        assert_eq!(result, Err("failed"));
        assert_eq!(polls, 2);
        assert!(dropped.get());
    }

    #[test]
    fn try_join_all_of_nothing_succeeds() {
        let (result, _) = run(async {
            try_join_all(Vec::<std::future::Ready<Result<u8, ()>>>::new()).await
        });

        assert_eq!(result, Ok(Vec::new()));
    }

    #[test]
    fn join_of_eight_futures_compiles_and_orders() {
        let (all, _) = run(async {
            join!(
                async { 1 },
                async { 2 },
                async { 3 },
                async { 4 },
                async { 5 },
                async { 6 },
                async { 7 },
                async { 8 },
            )
            .await
        });

        assert_eq!(all, (1, 2, 3, 4, 5, 6, 7, 8));
    }
}
