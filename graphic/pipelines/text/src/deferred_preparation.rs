//! Which text a frame owes the screen, and how much of the rest it can afford.
//!
//! A scroll viewport hands its child a rectangle wider than the viewport, so a
//! line is built, laid out, shaped and rasterized a few frames before it
//! scrolls in rather than on the single frame its edge crosses the boundary.
//! That moves the work; it does not remove it. During a sustained fling fresh
//! content arrives every frame, so preparing the whole widened rectangle
//! unconditionally turns one visible stall into a continuous one.
//!
//! What makes the widened rectangle safe is that, by definition, **nothing on
//! screen depends on the part of it that is off screen**. Dropping a visible
//! glyph shows blank text; dropping one that is clipped away shows nothing at
//! all. This module is that asymmetry: [`request_is_on_screen`] says which
//! side of it a request falls on, and [`PreparationBudget`] says when the
//! frame has spent enough on the other side.
//!
//! Everything here is a pure function of geometry and of a clock the caller
//! passes in, so the policy is testable without a GPU and without sleeping.

use std::time::Duration;

use aimer_utils::AnimInstant;

/// How long one frame may spend preparing text before what is left over is
/// postponed.
///
/// Sized against a 120 Hz frame (8.3 ms): the budget has to leave room for the
/// rest of the frame — encoding, atlas uploads, present — while being long
/// enough that a viewport of fresh text still makes progress every frame
/// instead of trickling in a request at a time.
///
/// The budget bounds the work *ahead of the viewport* only. Text that can show
/// a pixel this frame is always prepared, however long it takes: a frame that
/// renders blanks is worse than a frame that is late.
pub(crate) const PREPARATION_BUDGET: Duration = Duration::from_millis(4);

/// How many ahead-of-view requests are prepared between two readings of the
/// clock.
///
/// The batch executor spreads a group across its workers, so asking it for one
/// request at a time would serialize the very work that is meant to overlap;
/// reading the clock only between groups keeps the check off the hot path.
/// Small enough that one group cannot overshoot the budget by much.
pub(crate) const PREPARATION_CHUNK: usize = 8;

/// Whether a request can put a pixel on screen this frame.
///
/// `rect` is the request's bounds — `[x, y, width, height]` — `clip` the
/// rectangle it is drawn under, in the same convention the glyph shaders use
/// (a width of zero or less means *unclipped*), and `surface` the size of the
/// render target.
///
/// A request is on screen when its bounds meet both the clip and the surface.
/// Everything else was drawn only because a viewport asked for it early, and
/// is what the frame is allowed to postpone.
///
/// An extent that is zero or negative — as an unmeasured or unconstrained
/// request's is, and as the renderer's "the rest of the surface" default
/// becomes for content placed past the surface — is read as *unbounded*, not
/// as empty. The text reaches at least its origin and grows away from it, so
/// only the origin can rule it out, and text that turns out to be visible is
/// never postponed.
pub(crate) fn request_is_on_screen(rect: [f32; 4], clip: [f32; 4], surface: (f32, f32)) -> bool {
    let rect = [
        rect[0],
        rect[1],
        known_extent(rect[2]),
        known_extent(rect[3]),
    ];

    if !rects_intersect(rect, [0.0, 0.0, surface.0, surface.1]) {
        return false;
    }

    // Matches the shaders' and `glyph_intersects_clip`'s convention: a
    // non-positive width is the absence of a clip, not an empty one.
    if clip[2] <= 0.0 {
        return true;
    }

    rects_intersect(rect, clip)
}

/// An extent as the geometry means it: an unusable one reaches as far as it
/// might.
#[inline]
fn known_extent(extent: f32) -> f32 {
    if extent > 0.0 { extent } else { f32::INFINITY }
}

/// Whether two `[x, y, width, height]` rectangles overlap on both axes.
///
/// Touching edges do not count: a rectangle ending exactly where another
/// begins covers none of it.
#[inline]
fn rects_intersect(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0] + a[2] > b[0] && b[0] + b[2] > a[0] && a[1] + a[3] > b[1] && b[1] + b[3] > a[1]
}

/// Prepares as much of `ahead_of_view` as `budget` allows, in chunks, and
/// returns how many items were prepared.
///
/// Items are taken in order — a viewport's requests arrive in document order,
/// so the ones nearest the visible edge, which the user reaches first, are the
/// ones that get the budget. The clock is read once per chunk rather than once
/// per item: the executor spreads a chunk across its workers, so a chunk is
/// the smallest unit that keeps them busy, and it bounds the overshoot to the
/// cost of the chunk that was admitted last.
///
/// `prepare` returning `false` — a stage that could not complete — stops the
/// run as an exhausted budget does. Its items are not prepared, and reporting
/// them as such would leave the caller drawing text that has no layout.
///
/// `now` is a parameter so the policy can be exercised against a clock that
/// does not tick on its own.
pub(crate) fn prepare_ahead_of_view<T>(
    ahead_of_view: &[T],
    budget: PreparationBudget,
    mut now: impl FnMut() -> AnimInstant,
    mut prepare: impl FnMut(&[T]) -> bool,
) -> usize {
    let mut prepared = 0;
    for chunk in ahead_of_view.chunks(PREPARATION_CHUNK) {
        if !budget.allows(now()) || !prepare(chunk) {
            break;
        }
        prepared += chunk.len();
    }

    prepared
}

/// The share of a frame still available for work the frame does not owe the
/// screen.
///
/// Holds a deadline rather than an elapsed total so that asking costs one
/// comparison, and so the budget spans the *whole* preparation — the visible
/// text is charged to it too, which is what stops a frame already spent on
/// what the user can see from adding more on top.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PreparationBudget {
    deadline: AnimInstant,
}

impl PreparationBudget {
    /// A budget of `budget` starting at `start`.
    #[inline]
    pub(crate) fn starting_at(start: AnimInstant, budget: Duration) -> Self {
        Self {
            deadline: start + budget,
        }
    }

    /// Whether work may still begin at `now`.
    ///
    /// The answer is about *starting* a chunk, not finishing one: the frame
    /// overshoots by at most whatever the chunk it admitted costs, which is
    /// what [`PREPARATION_CHUNK`] keeps small.
    #[inline]
    pub(crate) fn allows(&self, now: AnimInstant) -> bool {
        now < self.deadline
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SURFACE: (f32, f32) = (400.0, 800.0);
    const UNCLIPPED: [f32; 4] = [0.0, 0.0, -1.0, 0.0];

    #[test]
    fn text_inside_the_viewport_is_owed_to_the_screen() {
        assert!(request_is_on_screen(
            [10.0, 10.0, 100.0, 20.0],
            [0.0, 0.0, 400.0, 800.0],
            SURFACE
        ));
    }

    #[test]
    fn text_straddling_the_clip_edge_is_owed_to_the_screen() {
        // One row of pixels inside the clip is still a row the user sees.
        assert!(request_is_on_screen(
            [10.0, 99.0, 100.0, 20.0],
            [0.0, 100.0, 400.0, 600.0],
            SURFACE
        ));
    }

    #[test]
    fn text_a_viewport_prepared_ahead_of_itself_is_not() {
        // Below the scroll viewport's clip: drawn only because the cache
        // extent asked for it, and discarded by the clip on the GPU.
        assert!(!request_is_on_screen(
            [10.0, 900.0, 100.0, 20.0],
            [0.0, 100.0, 400.0, 600.0],
            SURFACE
        ));
    }

    #[test]
    fn text_off_the_render_target_is_not_owed_to_the_screen() {
        assert!(!request_is_on_screen(
            [10.0, 1200.0, 100.0, 20.0],
            UNCLIPPED,
            SURFACE
        ));
    }

    #[test]
    fn unclipped_text_on_the_target_is_owed_to_the_screen() {
        assert!(request_is_on_screen(
            [10.0, 10.0, 100.0, 20.0],
            UNCLIPPED,
            SURFACE
        ));
    }

    #[test]
    fn text_of_unknown_extent_reaching_the_clip_is_owed_to_the_screen() {
        // A request that was never measured says nothing about where it ends,
        // and guessing short here is a blank on screen.
        for rect in [
            [10.0, 50.0, 0.0, 0.0],
            [10.0, 50.0, 100.0, 0.0],
            [10.0, 50.0, -1.0, 20.0],
        ] {
            assert!(request_is_on_screen(
                rect,
                [0.0, 0.0, 400.0, 600.0],
                SURFACE
            ));
        }
    }

    #[test]
    fn text_of_unknown_extent_starting_past_the_clip_is_not() {
        // The renderer's default height is "the rest of the surface", which
        // goes negative exactly for the content a viewport prepared below
        // itself — the case this whole split exists for.
        for rect in [
            [10.0, 900.0, 100.0, -100.0],
            [10.0, 900.0, 0.0, 0.0],
            [10.0, 900.0, -1.0, 20.0],
        ] {
            assert!(!request_is_on_screen(
                rect,
                [0.0, 100.0, 400.0, 600.0],
                SURFACE
            ));
        }
    }

    /// A clock that only moves when the test says so.
    fn clock(steps: Vec<Duration>) -> (AnimInstant, impl FnMut() -> AnimInstant) {
        let start = AnimInstant::now();
        let mut steps = steps.into_iter();
        let tick = move || start + steps.next().unwrap_or(Duration::ZERO);

        (start, tick)
    }

    #[test]
    fn a_frame_with_time_to_spare_prepares_everything_ahead_of_it() {
        let ahead: Vec<usize> = (0..PREPARATION_CHUNK * 2 + 3).collect();
        let (start, now) = clock(vec![Duration::ZERO; 8]);
        let mut prepared_items = Vec::new();

        let prepared = prepare_ahead_of_view(
            &ahead,
            PreparationBudget::starting_at(start, PREPARATION_BUDGET),
            now,
            |chunk| {
                prepared_items.extend_from_slice(chunk);
                true
            },
        );

        assert_eq!(prepared, ahead.len());
        assert_eq!(prepared_items, ahead);
    }

    #[test]
    fn an_exhausted_budget_stops_at_a_chunk_boundary() {
        let ahead: Vec<usize> = (0..PREPARATION_CHUNK * 3).collect();
        // Two chunks fit; the third starts after the deadline.
        let (start, now) = clock(vec![
            Duration::ZERO,
            Duration::from_millis(1),
            PREPARATION_BUDGET,
        ]);
        let mut chunks = 0;

        let prepared = prepare_ahead_of_view(
            &ahead,
            PreparationBudget::starting_at(start, PREPARATION_BUDGET),
            now,
            |_| {
                chunks += 1;
                true
            },
        );

        assert_eq!(chunks, 2, "no chunk may begin past the deadline");
        assert_eq!(prepared, PREPARATION_CHUNK * 2);
    }

    #[test]
    fn a_frame_out_of_budget_before_it_starts_prepares_nothing() {
        let ahead: Vec<usize> = (0..PREPARATION_CHUNK).collect();
        let (start, now) = clock(vec![PREPARATION_BUDGET]);

        let prepared = prepare_ahead_of_view(
            &ahead,
            PreparationBudget::starting_at(start, PREPARATION_BUDGET),
            now,
            |_| panic!("a frame with no budget left must not prepare anything"),
        );

        assert_eq!(prepared, 0);
    }

    #[test]
    fn a_chunk_that_could_not_be_prepared_is_not_reported_as_prepared() {
        let ahead: Vec<usize> = (0..PREPARATION_CHUNK * 3).collect();
        let (start, now) = clock(vec![Duration::ZERO; 8]);
        let mut chunks = 0;

        let prepared = prepare_ahead_of_view(
            &ahead,
            PreparationBudget::starting_at(start, PREPARATION_BUDGET),
            now,
            |_| {
                chunks += 1;
                chunks < 2
            },
        );

        assert_eq!(prepared, PREPARATION_CHUNK);
        assert_eq!(chunks, 2, "the run stops at the chunk that failed");
    }

    #[test]
    fn a_budget_admits_work_until_its_deadline() {
        let start = AnimInstant::now();
        let budget = PreparationBudget::starting_at(start, Duration::from_millis(4));

        assert!(budget.allows(start));
        assert!(budget.allows(start + Duration::from_millis(3)));
        assert!(!budget.allows(start + Duration::from_millis(4)));
        assert!(!budget.allows(start + Duration::from_millis(40)));
    }

    #[test]
    fn a_frame_already_spent_on_visible_text_admits_nothing_more() {
        // The budget covers the whole preparation, so work the frame does not
        // owe the screen is what gives way when the visible text was heavy.
        let start = AnimInstant::now();
        let budget = PreparationBudget::starting_at(start, PREPARATION_BUDGET);

        assert!(!budget.allows(start + PREPARATION_BUDGET + Duration::from_millis(1)));
    }
}
