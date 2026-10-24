//! Cooperative yielding: how a task gives the frame back.

use std::pin::Pin;
use std::task::{Context, Poll};

use crate::budget::{IDLE_SLICE_FLOOR, time_remaining_in_frame};

/// Hands the thread back once, and asks to be resumed.
///
/// See [`yield_now`].
#[derive(Debug, Default)]
#[must_use = "a yield does nothing unless it is awaited"]
pub struct YieldNow {
    yielded: bool,
}

impl Future for YieldNow {
    type Output = ();

    #[inline]
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if self.yielded {
            return Poll::Ready(());
        }

        self.yielded = true;
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}

/// Yields to the scheduler, coming back in the same phase.
///
/// A microtask that yields is polled again in the same drain; a frame task that
/// yields is polled again on the next frame; an idle task that yields is polled
/// again while the budget lasts. That difference is the phase doing its job —
/// the task itself says nothing about *when*.
///
/// # Examples
///
/// ```
/// use std::cell::Cell;
/// use std::rc::Rc;
///
/// use aimer_venus::{Venus, yield_now};
///
/// let venus = Venus::new();
/// let steps = Rc::new(Cell::new(0));
///
/// let counted = steps.clone();
/// venus.spawn(async move {
///     counted.set(counted.get() + 1);
///     yield_now().await;
///     counted.set(counted.get() + 1);
/// });
///
/// venus.run_microtasks();
/// assert_eq!(steps.get(), 2, "both halves land in the same drain");
/// ```
#[inline]
pub fn yield_now() -> YieldNow {
    YieldNow::default()
}

/// Yields only if this frame has run out of room, reporting whether it did.
///
/// The ergonomic that makes an eight-millisecond frame workable: a long
/// rasterisation or parse calls this in its inner loop and is spread across
/// frames instead of dropping one. Outside a budgeted phase — a microtask, or a
/// task running under no scheduler at all — there is no deadline to respect and
/// this never yields.
///
/// Cooperation is the only mechanism available: a poll that never returns cannot
/// be interrupted, so work that has no loop to slice belongs on
/// [`Venus::offload`](crate::Venus::offload) instead.
///
/// # Examples
///
/// ```
/// use std::rc::Rc;
/// use std::cell::RefCell;
///
/// use aimer_venus::{Venus, yield_if_over_budget};
///
/// let venus = Venus::new();
/// let decoded = Rc::new(RefCell::new(Vec::new()));
///
/// let tiles = decoded.clone();
/// venus.spawn_idle(async move {
///     for tile in 0..64 {
///         tiles.borrow_mut().push(tile);
///         yield_if_over_budget().await;
///     }
/// });
///
/// venus.run_idle(&venus.idle_budget());
/// ```
#[inline]
pub async fn yield_if_over_budget() -> bool {
    match time_remaining_in_frame() {
        Some(remaining) if remaining <= IDLE_SLICE_FLOOR => {
            yield_now().await;
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::Duration;

    use web_time::Instant;

    use super::*;
    use crate::{FrameBudget, Venus};

    #[test]
    fn a_yield_resumes_within_the_same_microtask_drain() {
        let venus = Venus::new();
        let steps = Rc::new(Cell::new(0));

        let counted = steps.clone();
        venus.spawn(async move {
            for _ in 0..3 {
                counted.set(counted.get() + 1);
                yield_now().await;
            }
        });

        assert_eq!(venus.run_microtasks(), 4, "three yields plus the finish");
        assert_eq!(steps.get(), 3);
    }

    #[test]
    fn nothing_yields_outside_a_budgeted_phase() {
        let venus = Venus::new();
        let yielded = Rc::new(Cell::new(true));

        let observed = yielded.clone();
        venus.spawn(async move {
            observed.set(yield_if_over_budget().await);
        });

        venus.run_microtasks();

        assert!(!yielded.get(), "a microtask has no deadline to respect");
    }

    // The property an eight-millisecond frame depends on: a task that overruns
    // its slice stops, and comes back on the next tick with its state intact.
    #[test]
    fn an_over_budget_idle_task_stops_and_resumes_with_its_state_intact() {
        let venus = Venus::new();
        let iterations = Rc::new(Cell::new(0));

        let counted = iterations.clone();
        venus.spawn_idle(async move {
            for _ in 0..4 {
                let spin_until = Instant::now() + Duration::from_millis(2);
                while Instant::now() < spin_until {}

                counted.set(counted.get() + 1);
                yield_if_over_budget().await;
            }
        });

        venus.run_idle(&FrameBudget::from_now(Duration::from_millis(3)));
        let after_one_frame = iterations.get();

        assert!(
            (1..4).contains(&after_one_frame),
            "a 3ms budget must not swallow four 2ms iterations, got {after_one_frame}"
        );
        assert_eq!(venus.task_count(), 1, "the task survives to the next frame");

        while venus.task_count() > 0 {
            venus.run_idle(&FrameBudget::from_now(Duration::from_millis(3)));
        }

        assert_eq!(iterations.get(), 4, "it resumed where it left off");
    }
}
