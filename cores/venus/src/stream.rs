//! A minimal stream: an async sequence of values.
//!
//! The standard library has no stable stream trait, and a UI runtime that
//! avoids third-party dependencies cannot borrow one. This is the small core
//! that channels, watchers and event feeds need — a poll method, an awaitable
//! `next`, and the handful of adapters a handler reaches for.

use std::pin::Pin;
use std::task::{Context, Poll};

/// A sequence of values produced asynchronously.
///
/// `poll_next` resolves to `Some(item)` for each value and to `None` once, when
/// the stream has ended. Polling an ended stream again is allowed to panic.
pub trait Stream {
    /// What the stream yields.
    type Item;

    /// Attempts to pull the next value, registering the task's waker if there
    /// is none yet.
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>>;
}

impl<S: Stream + Unpin + ?Sized> Stream for &mut S {
    type Item = S::Item;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut **self).poll_next(cx)
    }
}

impl<S: Stream + ?Sized> Stream for Pin<Box<S>> {
    type Item = S::Item;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.as_mut().get_mut().as_mut().poll_next(cx)
    }
}

/// Adapters and awaiting for any [`Stream`].
pub trait StreamExt: Stream {
    /// Waits for the next value; `None` once the stream has ended.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_venus::{StreamExt, Venus, stream};
    ///
    /// let venus = Venus::new();
    /// let first = venus.run_until(async {
    ///     let mut numbers = stream::iter([1, 2, 3]);
    ///     numbers.next().await
    /// });
    /// assert_eq!(first, Some(1));
    /// ```
    #[inline]
    fn next(&mut self) -> Next<'_, Self>
    where
        Self: Unpin,
    {
        Next { stream: self }
    }

    /// Transforms each value.
    #[inline]
    fn map<B, F: FnMut(Self::Item) -> B>(self, f: F) -> Map<Self, F>
    where
        Self: Sized,
    {
        Map { stream: self, f }
    }

    /// Keeps only the values for which `predicate` is true.
    #[inline]
    fn filter<P: FnMut(&Self::Item) -> bool>(self, predicate: P) -> Filter<Self, P>
    where
        Self: Sized,
    {
        Filter { stream: self, predicate }
    }

    /// Ends the stream after `count` values, without pulling a further one.
    #[inline]
    fn take(self, count: usize) -> Take<Self>
    where
        Self: Sized,
    {
        Take { stream: self, remaining: count }
    }

    /// Runs `f` on every value until the stream ends.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::cell::Cell;
    /// use std::rc::Rc;
    ///
    /// use aimer_venus::{StreamExt, Venus, stream};
    ///
    /// let venus = Venus::new();
    /// let total = Rc::new(Cell::new(0));
    ///
    /// let sum = total.clone();
    /// venus.run_until(
    ///     stream::iter([1, 2, 3]).for_each(move |n| sum.set(sum.get() + n)),
    /// );
    /// assert_eq!(total.get(), 6);
    /// ```
    #[inline]
    fn for_each<F: FnMut(Self::Item)>(self, f: F) -> ForEach<Self, F>
    where
        Self: Sized,
    {
        ForEach { stream: self, f }
    }

    /// Gathers every value into a `Vec`, resolving when the stream ends.
    #[inline]
    fn collect_vec(self) -> CollectVec<Self>
    where
        Self: Sized,
    {
        CollectVec { stream: self, items: Vec::new() }
    }
}

impl<S: Stream + ?Sized> StreamExt for S {}

/// The future returned by [`StreamExt::next`].
#[must_use = "futures do nothing unless awaited"]
pub struct Next<'a, S: ?Sized> {
    stream: &'a mut S,
}

impl<S: Stream + Unpin + ?Sized> Future for Next<'_, S> {
    type Output = Option<S::Item>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut *self.stream).poll_next(cx)
    }
}

/// The stream returned by [`StreamExt::map`].
#[must_use = "streams do nothing unless polled"]
pub struct Map<S, F> {
    stream: S,
    f: F,
}

impl<S: Stream, B, F: FnMut(S::Item) -> B> Stream for Map<S, F> {
    type Item = B;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<B>> {
        // SAFETY: `stream` is pinned structurally and never moved; `f` is not
        // pinned and is only called through a unique reference. No `Drop` impl.
        let this = unsafe { self.get_unchecked_mut() };
        match unsafe { Pin::new_unchecked(&mut this.stream) }.poll_next(cx) {
            Poll::Ready(item) => Poll::Ready(item.map(&mut this.f)),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// The stream returned by [`StreamExt::filter`].
#[must_use = "streams do nothing unless polled"]
pub struct Filter<S, P> {
    stream: S,
    predicate: P,
}

impl<S: Stream, P: FnMut(&S::Item) -> bool> Stream for Filter<S, P> {
    type Item = S::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<S::Item>> {
        // SAFETY: as for `Map`.
        let this = unsafe { self.get_unchecked_mut() };
        loop {
            match unsafe { Pin::new_unchecked(&mut this.stream) }.poll_next(cx) {
                Poll::Ready(Some(item)) if !(this.predicate)(&item) => continue,
                other => return other,
            }
        }
    }
}

/// The stream returned by [`StreamExt::take`].
#[must_use = "streams do nothing unless polled"]
pub struct Take<S> {
    stream: S,
    remaining: usize,
}

impl<S: Stream> Stream for Take<S> {
    type Item = S::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<S::Item>> {
        // SAFETY: as for `Map`.
        let this = unsafe { self.get_unchecked_mut() };
        if this.remaining == 0 {
            return Poll::Ready(None);
        }
        let polled = unsafe { Pin::new_unchecked(&mut this.stream) }.poll_next(cx);
        if let Poll::Ready(Some(_)) = polled {
            this.remaining -= 1;
        }
        polled
    }
}

/// The future returned by [`StreamExt::for_each`].
#[must_use = "futures do nothing unless awaited"]
pub struct ForEach<S, F> {
    stream: S,
    f: F,
}

impl<S: Stream, F: FnMut(S::Item)> Future for ForEach<S, F> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        // SAFETY: as for `Map`.
        let this = unsafe { self.get_unchecked_mut() };
        loop {
            match unsafe { Pin::new_unchecked(&mut this.stream) }.poll_next(cx) {
                Poll::Ready(Some(item)) => (this.f)(item),
                Poll::Ready(None) => return Poll::Ready(()),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

/// The future returned by [`StreamExt::collect_vec`].
#[must_use = "futures do nothing unless awaited"]
pub struct CollectVec<S: Stream> {
    stream: S,
    items: Vec<S::Item>,
}

impl<S: Stream> Future for CollectVec<S> {
    type Output = Vec<S::Item>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Vec<S::Item>> {
        // SAFETY: `stream` is pinned structurally; `items` is not.
        let this = unsafe { self.get_unchecked_mut() };
        loop {
            match unsafe { Pin::new_unchecked(&mut this.stream) }.poll_next(cx) {
                Poll::Ready(Some(item)) => this.items.push(item),
                Poll::Ready(None) => return Poll::Ready(std::mem::take(&mut this.items)),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

/// The stream returned by [`iter`].
#[must_use = "streams do nothing unless polled"]
pub struct Iter<I>(I);

impl<I> Unpin for Iter<I> {}

impl<I: Iterator> Stream for Iter<I> {
    type Item = I::Item;

    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<I::Item>> {
        Poll::Ready(self.0.next())
    }
}

/// A stream that yields the items of `iter`, all immediately ready.
#[inline]
pub fn iter<I: IntoIterator>(iter: I) -> Iter<I::IntoIter> {
    Iter(iter.into_iter())
}

// `run_until` sleeps the calling thread, which a browser cannot do.
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;
    use crate::Venus;

    #[test]
    fn next_yields_each_value_then_none() {
        let venus = Venus::new();
        let seen = venus.run_until(async {
            let mut numbers = iter([1, 2]);
            (numbers.next().await, numbers.next().await, numbers.next().await)
        });

        assert_eq!(seen, (Some(1), Some(2), None));
    }

    #[test]
    fn adapters_compose() {
        let venus = Venus::new();
        let out = venus.run_until(
            iter(1..=10)
                .filter(|n| n % 2 == 0)
                .map(|n| n * 10)
                .take(3)
                .collect_vec(),
        );

        assert_eq!(out, [20, 40, 60]);
    }

    #[test]
    fn take_does_not_pull_past_its_count() {
        let venus = Venus::new();
        let pulled = Rc::new(Cell::new(0));

        let counter = pulled.clone();
        let source = iter((1..=4).inspect(move |_| counter.set(counter.get() + 1)));
        let taken = venus.run_until(source.take(2).collect_vec());

        assert_eq!(taken, [1, 2]);
        assert_eq!(pulled.get(), 2, "the third value was never pulled");
    }

    #[test]
    fn a_boxed_stream_is_a_stream() {
        let venus = Venus::new();
        let boxed: Pin<Box<dyn Stream<Item = u8>>> = Box::pin(iter([5, 6]));

        assert_eq!(venus.run_until(boxed.collect_vec()), [5, 6]);
    }
}
