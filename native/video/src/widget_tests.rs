use aimer_attribute::position::Vec2d;
use aimer_cupid::draw_cmd::{DrawCommand, Rect as RenderRect, RenderNodeId, RenderTree};
use aimer_widget::base::WindowHandle;

use super::*;
use crate::test_support::{last, ready_player};

fn assert_rect(actual: Rect, x: f32, y: f32, width: f32, height: f32) {
    let close = |a: f32, b: f32| (a - b).abs() < 1e-4;
    assert!(
        close(actual.x, x) && close(actual.y, y) && close(actual.width, width) && close(actual.height, height),
        "{actual:?} != ({x}, {y}, {width}, {height})"
    );
}

#[test]
fn contain_letterboxes_a_wide_video_in_a_tall_box() {
    assert_rect(VideoFit::Contain.place((1920.0, 1080.0), (100.0, 200.0)), 0.0, 71.875, 100.0, 56.25);
}

#[test]
fn contain_pillarboxes_a_tall_video_in_a_wide_box() {
    assert_rect(VideoFit::Contain.place((1080.0, 1920.0), (400.0, 160.0)), 155.0, 0.0, 90.0, 160.0);
}

#[test]
fn cover_fills_the_box_and_overflows_on_one_axis() {
    assert_rect(VideoFit::Cover.place((1920.0, 1080.0), (100.0, 100.0)), -38.888_89, 0.0, 177.777_78, 100.0);
}

#[test]
fn fill_stretches_and_an_unknown_size_fills_the_box() {
    assert_rect(VideoFit::Fill.place((1920.0, 1080.0), (100.0, 100.0)), 0.0, 0.0, 100.0, 100.0);
    assert_rect(VideoFit::Contain.place((0.0, 0.0), (80.0, 60.0)), 0.0, 0.0, 80.0, 60.0);
}

fn context(scale: f32) -> BuildContext<'static> {
    let canvas = {
        let inner = Box::leak(Box::new(aimer_canvas::InnerCanvas::new()));
        aimer_canvas::FrameCanvas::new(inner)
    };
    BuildContext::new(
        canvas,
        ResolvedSize::default(),
        scale,
        Vec2d::default(),
        Vec2d::default(),
        WindowHandle::headless(Default::default(), f64::from(scale)),
        tokio::runtime::Handle::current(),
    )
}

/// Runs one retained paint frame for `element`.
fn paint_frame(ctx: &BuildContext, element: &AnyElement, tree: &RenderTree, node: RenderNodeId) {
    let nodes = std::rc::Rc::new(aimer_widget::ElementNodeMap::from([(element.id(), node)]));
    ctx.canvas.begin_frame();
    aimer_widget::begin_paint_frame(160, 90);
    aimer_widget::with_v2_render_tree_context(tree.clone(), nodes, Some(element.id()), || {
        element.update(ctx)
    });
}

fn video_commands(tree: &RenderTree, node: RenderNodeId) -> Vec<(RenderRect, u64)> {
    tree.draw_list_snapshot(node)
        .unwrap()
        .commands
        .iter()
        .filter_map(|command| match command {
            DrawCommand::DrawExternalTexture { rect, texture } => Some((*rect, texture.generation())),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn nothing_is_drawn_until_the_first_frame_arrives() {
    let ctx = context(1.0);
    let player = ready_player();
    let element = Video::new().player(player).width(160.0).height(90.0).to_element(&ctx);
    let tree = RenderTree::new();
    let node = tree.add_root(RenderRect::new(0.0, 0.0, 160.0, 90.0)).unwrap();

    paint_frame(&ctx, &element, &tree, node);
    assert!(video_commands(&tree, node).is_empty());
}

#[tokio::test]
async fn a_frame_is_drawn_fitted_and_a_new_one_is_recorded_again() {
    let ctx = context(1.0);
    let player = ready_player();
    let element = Video::new()
        .player(player.clone())
        .width(160.0)
        .height(160.0)
        .to_element(&ctx);
    let tree = RenderTree::new();
    let node = tree.add_root(RenderRect::new(0.0, 0.0, 160.0, 160.0)).unwrap();

    last().lock().unwrap().pending_frames.push(1);
    paint_frame(&ctx, &element, &tree, node);
    let first = video_commands(&tree, node);
    assert_eq!(first.len(), 1, "one video quad");
    // 1920 x 1080 contained in a 160 x 160 box.
    assert_eq!(first[0].0, RenderRect::new(0.0, 35.0, 160.0, 90.0));
    assert!(!element.paint_needs_recording(&ctx), "nothing changed since");

    last().lock().unwrap().pending_frames.push(2);
    player.poll();
    assert!(element.paint_needs_recording(&ctx), "a new frame is a new picture");
    paint_frame(&ctx, &element, &tree, node);
    let second = video_commands(&tree, node);
    assert_ne!(second[0].1, first[0].1, "recorded with the new generation");
}

#[tokio::test]
async fn only_cover_clips_to_the_box() {
    let ctx = context(2.0);
    let contain = Video::new().player(ready_player()).width(40.0).height(30.0).to_element(&ctx);
    let cover = Video::new()
        .player(ready_player())
        .fit(VideoFit::Cover)
        .width(40.0)
        .height(30.0)
        .to_element(&ctx);
    assert_eq!(contain.retained_clip(&ctx), None);
    assert_eq!(cover.retained_clip(&ctx), Some(RenderRect::new(0.0, 0.0, 40.0, 30.0)));
}

#[tokio::test]
async fn without_a_player_it_lays_out_and_draws_nothing() {
    let ctx = context(1.0);
    let element = Video::new().width(64.0).height(36.0).to_element(&ctx);
    let tree = RenderTree::new();
    let node = tree.add_root(RenderRect::new(0.0, 0.0, 64.0, 36.0)).unwrap();
    paint_frame(&ctx, &element, &tree, node);
    assert!(video_commands(&tree, node).is_empty());
    assert_eq!(element.event_tree_role(), EventTreeRole::Transparent);
}
