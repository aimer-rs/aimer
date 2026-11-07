//! A reverse scrollable keeps its distance from the content end, not from the
//! start, so growth *before* the viewport never moves what is on screen.

use super::*;
use crate::ScrollAxis;

fn size(width: f32, height: f32) -> ResolvedSize {
    ResolvedSize { width, height }
}

/// An engine parked at `offset`, anchored to the end along `axis`.
fn reverse_ctrl(axis: ScrollAxis, offset: Vec2d) -> Rc<ScrollState> {
    let mut state = ScrollState::for_test_at(offset);
    state.axis = axis;
    state.reverse = true;
    Rc::new(state)
}

/// Paints one frame of a 100x100 viewport over content that measures `content`.
fn paint(ctrl: &Rc<ScrollState>, content: ResolvedSize) {
    let ctx = drawing_context(Some((0.0, 0.0, 100.0, 100.0)));
    refining_scrollable(ctrl.clone(), Rc::new(Cell::new(content)), content).update(&ctx);
}

#[tokio::test]
async fn the_first_layout_shows_the_content_end() {
    let ctrl = reverse_ctrl(ScrollAxis::Vertical, Vec2d::ZERO);

    paint(&ctrl, size(100.0, 350.0));

    assert_eq!(ctrl.cached_max_scroll.get().y, 250.0);
    assert_eq!(ctrl.scroll_offset.get().y, -250.0);
}

#[tokio::test]
async fn growth_before_the_viewport_keeps_the_distance_from_the_end() {
    let ctrl = reverse_ctrl(ScrollAxis::Vertical, Vec2d::ZERO);
    paint(&ctrl, size(100.0, 350.0));
    // The user reads 100 px back from the newest content.
    ctrl.set_scroll_offset(Vec2d { x: 0.0, y: -150.0 });

    // Older history loads above: the content is 100 px taller.
    paint(&ctrl, size(100.0, 450.0));

    assert_eq!(ctrl.cached_max_scroll.get().y, 350.0);
    assert_eq!(
        ctrl.scroll_offset.get().y,
        -250.0,
        "still 100 px from the end, so the same rows are on screen"
    );
}

#[tokio::test]
async fn a_viewport_at_the_end_follows_appended_content() {
    let ctrl = reverse_ctrl(ScrollAxis::Vertical, Vec2d::ZERO);
    paint(&ctrl, size(100.0, 350.0));

    paint(&ctrl, size(100.0, 500.0));

    assert_eq!(ctrl.scroll_offset.get().y, -400.0);
}

#[tokio::test]
async fn an_unchanged_extent_leaves_the_offset_untouched() {
    let ctrl = reverse_ctrl(ScrollAxis::Vertical, Vec2d::ZERO);
    paint(&ctrl, size(100.0, 350.0));
    ctrl.set_scroll_offset(Vec2d { x: 0.0, y: -123.4 });

    paint(&ctrl, size(100.0, 350.0));
    paint(&ctrl, size(100.0, 350.0));

    assert_eq!(ctrl.scroll_offset.get().y, -123.4);
}

#[tokio::test]
async fn a_forward_scrollable_still_keeps_its_distance_from_the_start() {
    let mut state = ScrollState::for_test_at(Vec2d::ZERO);
    state.reverse = false;
    let ctrl = Rc::new(state);
    paint(&ctrl, size(100.0, 350.0));
    ctrl.set_scroll_offset(Vec2d { x: 0.0, y: -150.0 });

    paint(&ctrl, size(100.0, 450.0));

    assert_eq!(ctrl.scroll_offset.get().y, -150.0);
}

#[tokio::test]
async fn a_remembered_distance_from_the_end_is_applied_on_the_first_layout() {
    // A positive seed is a distance from the end: the engine has no range yet,
    // so it is written as if the range were empty.
    let ctrl = reverse_ctrl(ScrollAxis::Vertical, Vec2d { x: 0.0, y: 100.0 });

    paint(&ctrl, size(100.0, 350.0));

    assert_eq!(ctrl.scroll_offset.get().y, -150.0);
}

#[tokio::test]
async fn content_shorter_than_the_viewport_hugs_the_end() {
    let ctrl = reverse_ctrl(ScrollAxis::Vertical, Vec2d::ZERO);

    paint(&ctrl, size(100.0, 40.0));

    let shown = ctrl.visual_offset(ctrl.scroll_offset.get());
    assert_eq!(shown.y, 60.0, "a 40 px transcript sits at the bottom of 100 px");
}

#[tokio::test]
async fn content_that_fills_the_viewport_is_not_shifted() {
    let ctrl = reverse_ctrl(ScrollAxis::Vertical, Vec2d::ZERO);

    paint(&ctrl, size(100.0, 100.0));

    assert_eq!(ctrl.visual_offset(ctrl.scroll_offset.get()).y, 0.0);
}

#[tokio::test]
async fn a_forward_scrollable_keeps_short_content_at_the_start() {
    let ctrl = Rc::new(ScrollState::for_test_at(Vec2d::ZERO));

    paint(&ctrl, size(100.0, 40.0));

    assert_eq!(ctrl.visual_offset(ctrl.scroll_offset.get()).y, 0.0);
}

#[tokio::test]
async fn a_horizontal_reverse_scrollable_anchors_to_the_right_end() {
    let ctrl = reverse_ctrl(ScrollAxis::Horizontal, Vec2d::ZERO);
    paint(&ctrl, size(350.0, 100.0));
    assert_eq!(ctrl.scroll_offset.get().x, -250.0);
    ctrl.set_scroll_offset(Vec2d { x: -150.0, y: 0.0 });

    paint(&ctrl, size(450.0, 100.0));

    assert_eq!(ctrl.scroll_offset.get().x, -250.0);
    assert_eq!(ctrl.scroll_offset.get().y, 0.0);
}

#[tokio::test]
async fn short_horizontal_content_hugs_the_right_end() {
    let ctrl = reverse_ctrl(ScrollAxis::Horizontal, Vec2d::ZERO);

    paint(&ctrl, size(30.0, 100.0));

    let shown = ctrl.visual_offset(ctrl.scroll_offset.get());
    assert_eq!(shown.x, 70.0);
    assert_eq!(shown.y, 0.0);
}
