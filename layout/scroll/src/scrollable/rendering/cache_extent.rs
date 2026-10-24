//! The margin by which a scroll viewport prepares content it cannot yet show.
//!
//! A viewport hands its child the rectangle the child is expected to
//! materialize. Handing down the *exact* viewport makes that rectangle a
//! perfect culling contract — and puts a line's entire cost (build, layout,
//! shaping, syntax highlighting, glyph rasterization) on the single frame its
//! top edge crosses the boundary. That frame is the pause the user feels.
//!
//! A cache extent widens the rectangle so the work happens a few frames before
//! the content is needed. It is the spatial counterpart of the index-based
//! overscan a lazy list already keeps, and it is why the widened rectangle is
//! biased: content is only ever needed on the side the viewport is travelling
//! toward, so a margin large enough for a fling would be wasted on the side
//! left behind.
//!
//! Nothing here allocates or branches on state — it is a pure function of the
//! viewport and how far it moved last frame, so the caller can compute it
//! every frame.

use aimer_attribute::position::Vec2d;

use crate::ScrollAxis;

/// Fraction of the viewport prepared beyond an edge the viewport is not
/// travelling toward.
///
/// Half a viewport is enough to absorb a wheel tick or the reversal of a drag
/// without becoming visible work: the content behind this margin is painted
/// but clipped away on the GPU, which is noise next to shaping and
/// rasterization.
pub const CACHE_EXTENT_VIEWPORT_FRACTION: f32 = 0.5;

/// Upper bound, as a fraction of the viewport, on the margin ahead of the
/// direction of travel.
///
/// The lead grows with speed so a fling is not outrun, but an unbounded lead
/// turns a scroll into "prepare the whole document" and reintroduces the very
/// stall this exists to remove.
pub const MAX_CACHE_EXTENT_VIEWPORT_FRACTION: f32 = 1.5;

/// Absolute ceiling (physical px) on either margin.
///
/// A viewport can be arbitrarily tall — a scrollable nested in an unbounded
/// parent measures its whole content — and a fraction of such a viewport is
/// not a margin, it is the document. This bounds the work regardless.
pub const MAX_CACHE_EXTENT_PX: f32 = 4000.0;

/// How many frames of travel the leading margin looks ahead.
///
/// `travel` is measured per drawn frame, so this is a lead time expressed in
/// frames: at 120 Hz six frames is ~50 ms, long enough to cover the gap
/// between preparing content and showing it, short enough that a fast fling
/// hits the cap rather than dragging an ever-growing amount of work along.
pub const CACHE_EXTENT_LEAD_FRAMES: f32 = 6.0;

/// The span of content a viewport asks its child to materialize.
///
/// Expressed along one axis, in the child's coordinates: the same units as the
/// visible rectangle it widens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CacheWindow {
    /// Where the window begins — at or before the visible start.
    pub(crate) start: f32,
    /// How far it reaches — at or beyond the viewport extent.
    pub(crate) extent: f32,
}

/// The window to prepare for a viewport of `viewport_extent` whose visible
/// content begins at `visible_start` and which moved `travel` along the axis
/// since the previous drawn frame.
///
/// `travel` is signed in the same direction as `visible_start`: positive means
/// the viewport is moving toward larger coordinates (toward the content end),
/// and its magnitude is a distance per frame, so it doubles as the speed the
/// lead is derived from. Zero — an idle viewport — yields a symmetric margin,
/// since the next move is equally likely either way.
///
/// The returned window always contains `[visible_start, visible_start +
/// viewport_extent)`: widening may never cull something that is on screen.
pub(crate) fn cache_window(visible_start: f32, viewport_extent: f32, travel: f32) -> CacheWindow {
    let viewport_extent = viewport_extent.max(0.0);
    let base = (viewport_extent * CACHE_EXTENT_VIEWPORT_FRACTION).min(MAX_CACHE_EXTENT_PX);
    let cap = (viewport_extent * MAX_CACHE_EXTENT_VIEWPORT_FRACTION).min(MAX_CACHE_EXTENT_PX);

    let travel = if travel.is_finite() { travel } else { 0.0 };
    let lead = (base + travel.abs() * CACHE_EXTENT_LEAD_FRAMES).min(cap);

    let (before, after) = if travel < 0.0 {
        (lead, base)
    } else {
        (base, lead)
    };

    CacheWindow {
        start: visible_start - before,
        extent: viewport_extent + before + after,
    }
}

/// The rectangle a viewport hands its child: the visible rectangle widened
/// along the axis it scrolls.
///
/// `visible` is the top-left of the visible content and `viewport` its size,
/// both in the child's coordinates; `travel` is how far that top-left moved
/// since the previous drawn frame. Only the scrolled axis is widened — the
/// cross axis cannot bring new content into view, so a margin there would be
/// work that is never needed.
pub(crate) fn cache_rect(
    axis: ScrollAxis,
    visible: Vec2d,
    viewport: (f32, f32),
    travel: Vec2d,
) -> (f32, f32, f32, f32) {
    let (viewport_w, viewport_h) = viewport;
    match axis {
        ScrollAxis::Vertical => {
            let window = cache_window(visible.y, viewport_h, travel.y);
            (visible.x, window.start, viewport_w, window.extent)
        }
        ScrollAxis::Horizontal => {
            let window = cache_window(visible.x, viewport_w, travel.x);
            (window.start, visible.y, window.extent, viewport_h)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEWPORT: f32 = 800.0;

    #[test]
    fn an_idle_viewport_prepares_the_same_margin_on_both_sides() {
        let window = cache_window(100.0, VIEWPORT, 0.0);

        let margin = VIEWPORT * CACHE_EXTENT_VIEWPORT_FRACTION;
        assert_eq!(window.start, 100.0 - margin);
        assert_eq!(window.extent, VIEWPORT + 2.0 * margin);
    }

    #[test]
    fn the_window_always_contains_what_is_on_screen() {
        for travel in [-500.0, -1.0, 0.0, 1.0, 500.0] {
            let window = cache_window(100.0, VIEWPORT, travel);

            assert!(
                window.start <= 100.0,
                "culling visible content shows blanks"
            );
            assert!(window.start + window.extent >= 100.0 + VIEWPORT);
        }
    }

    #[test]
    fn travel_toward_the_content_end_leads_ahead_of_the_viewport() {
        let idle = cache_window(0.0, VIEWPORT, 0.0);
        let moving = cache_window(0.0, VIEWPORT, 20.0);

        assert_eq!(
            moving.start, idle.start,
            "the side left behind keeps its bounce-back margin"
        );
        assert_eq!(
            moving.extent,
            idle.extent + 20.0 * CACHE_EXTENT_LEAD_FRAMES,
            "the lead grows with speed"
        );
    }

    #[test]
    fn travel_toward_the_content_start_leads_the_other_way() {
        let idle = cache_window(0.0, VIEWPORT, 0.0);
        let moving = cache_window(0.0, VIEWPORT, -20.0);

        assert_eq!(moving.start, idle.start - 20.0 * CACHE_EXTENT_LEAD_FRAMES);
        assert_eq!(
            moving.start + moving.extent,
            idle.start + idle.extent,
            "the side left behind keeps its bounce-back margin"
        );
    }

    #[test]
    fn a_fling_cannot_grow_into_preparing_the_document() {
        let window = cache_window(0.0, VIEWPORT, 10_000.0);

        let base = VIEWPORT * CACHE_EXTENT_VIEWPORT_FRACTION;
        let cap = VIEWPORT * MAX_CACHE_EXTENT_VIEWPORT_FRACTION;
        assert_eq!(window.extent, VIEWPORT + base + cap);
    }

    #[test]
    fn a_viewport_measuring_its_whole_content_still_prepares_a_margin() {
        let window = cache_window(0.0, 200_000.0, 10_000.0);

        assert_eq!(window.start, -MAX_CACHE_EXTENT_PX);
        assert_eq!(window.extent, 200_000.0 + 2.0 * MAX_CACHE_EXTENT_PX);
    }

    #[test]
    fn a_vertical_viewport_only_widens_the_axis_it_scrolls() {
        let rect = cache_rect(
            ScrollAxis::Vertical,
            Vec2d { x: 5.0, y: 100.0 },
            (300.0, VIEWPORT),
            Vec2d { x: 40.0, y: 0.0 },
        );

        let window = cache_window(100.0, VIEWPORT, 0.0);
        assert_eq!(rect, (5.0, window.start, 300.0, window.extent));
    }

    #[test]
    fn a_horizontal_viewport_widens_the_other_axis() {
        let rect = cache_rect(
            ScrollAxis::Horizontal,
            Vec2d { x: 100.0, y: 5.0 },
            (VIEWPORT, 300.0),
            Vec2d { x: 20.0, y: 0.0 },
        );

        let window = cache_window(100.0, VIEWPORT, 20.0);
        assert_eq!(rect, (window.start, 5.0, window.extent, 300.0));
    }

    #[test]
    fn a_degenerate_viewport_is_not_a_negative_window() {
        let window = cache_window(0.0, -10.0, f32::NAN);

        assert_eq!(
            window,
            CacheWindow {
                start: 0.0,
                extent: 0.0
            }
        );
    }
}
