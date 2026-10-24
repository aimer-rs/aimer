//! A scrollable asks for another frame after re-measuring its content only when
//! that frame would show something different.
//!
//! The re-measure after painting corrects whatever the frame was drawn with. A
//! correction that changes nothing along the axis the container scrolls on has
//! nothing to show: the extra frame would paint exactly what was just painted.

use super::*;
use crate::ScrollAxis;

const VIEWPORT: f32 = 100.0;

fn size(width: f32, height: f32) -> ResolvedSize {
    ResolvedSize { width, height }
}

/// Seeds the range the previous frame left behind.
fn seed(scrollable: &RawScrollableContainer<AnyElement>, max: Vec2d, content: ResolvedSize) {
    scrollable.ctrl.cached_max_scroll.set(max);
    scrollable.ctrl.cached_content_size.set(content);
    scrollable.ctrl.cached_content_size_valid.set(true);
}

/// Runs the refresh and reports whether it asked for another frame.
fn refresh(
    scrollable: &RawScrollableContainer<AnyElement>,
    initial: ResolvedSize,
    provisional: bool,
    preserve_content_end: bool,
) -> bool {
    let ctx = drawing_context(None);
    let _ = ctx.window.take_redraw_request();
    scrollable.refresh_content_size_after_draw(
        &ctx,
        VIEWPORT,
        VIEWPORT,
        initial,
        provisional,
        preserve_content_end,
    );
    ctx.window.take_redraw_request()
}

/// The window was resized by a few pixels: the cross-axis extent follows the
/// viewport, so the range left over from the last frame is off by that much on
/// the axis the container does not scroll along.
#[tokio::test]
async fn a_provisional_refresh_that_changes_only_the_cross_axis_asks_for_no_frame() {
    let scrollable = sized_scrollable(ScrollAxis::Vertical, size(100.0, 400.0));
    seed(&scrollable, Vec2d { x: 6.0, y: 300.0 }, size(106.0, 400.0));

    assert!(!refresh(&scrollable, size(100.0, 400.0), true, false));
    assert_eq!(
        scrollable.ctrl.cached_max_scroll.get(),
        Vec2d { x: 0.0, y: 300.0 },
        "the stale cross-axis range is still corrected"
    );
}

#[tokio::test]
async fn a_provisional_refresh_that_changes_only_the_cross_axis_asks_for_no_frame_horizontally() {
    let scrollable = sized_scrollable(ScrollAxis::Horizontal, size(400.0, 100.0));
    seed(&scrollable, Vec2d { x: 300.0, y: 6.0 }, size(400.0, 106.0));

    assert!(!refresh(&scrollable, size(400.0, 100.0), true, false));
}

#[tokio::test]
async fn a_refresh_that_moves_the_scroll_range_asks_for_a_frame() {
    let scrollable = sized_scrollable(ScrollAxis::Vertical, size(100.0, 400.0));
    seed(&scrollable, Vec2d { x: 0.0, y: 200.0 }, size(100.0, 300.0));

    assert!(refresh(&scrollable, size(100.0, 400.0), true, false));
}

#[tokio::test]
async fn a_refresh_that_grows_the_content_along_the_axis_asks_for_a_frame() {
    let scrollable = sized_scrollable(ScrollAxis::Vertical, size(100.0, 400.0));
    seed(&scrollable, Vec2d { x: 0.0, y: 200.0 }, size(100.0, 300.0));

    // Not provisional: the content changed while it was painted.
    assert!(refresh(&scrollable, size(100.0, 300.0), false, false));
}

#[tokio::test]
async fn preserving_the_content_end_always_asks_for_a_frame() {
    let scrollable = sized_scrollable(ScrollAxis::Vertical, size(100.0, 400.0));
    seed(&scrollable, Vec2d { x: 6.0, y: 300.0 }, size(106.0, 400.0));

    assert!(refresh(&scrollable, size(100.0, 400.0), true, true));
}

#[tokio::test]
async fn a_refresh_with_no_previous_range_asks_for_a_frame() {
    let scrollable = sized_scrollable(ScrollAxis::Vertical, size(100.0, 400.0));
    scrollable.ctrl.cached_content_size_valid.set(false);

    assert!(refresh(&scrollable, size(100.0, 400.0), true, false));
}

/// Unchanged behavior: with nothing provisional and nothing changed there is
/// nothing to correct.
#[tokio::test]
async fn a_settled_refresh_that_measures_the_same_size_does_nothing() {
    let scrollable = sized_scrollable(ScrollAxis::Vertical, size(100.0, 400.0));
    seed(&scrollable, Vec2d { x: 0.0, y: 300.0 }, size(100.0, 400.0));

    assert!(!refresh(&scrollable, size(100.0, 400.0), false, false));
}
