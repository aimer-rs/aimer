use std::rc::Rc;
use std::sync::Arc;

use aimer_attribute::dimension::Dimension;
use aimer_attribute::position::Vec2d;
use aimer_attribute::size::{ResolvedSize, Size};
use aimer_cupid::draw_cmd::{DrawCommand, Rect, RenderTree};
use aimer_widget::base::{BuildContext, WindowHandle};
use aimer_widget::{AnyElement, Drawable, Element, EventElement, EventTreeRole, LayoutElement, Rebuildable, Widget};

use crate::widget::{LottieFit, RawLottie, fit_destination};
use crate::{Composition, Lottie, LottieScene};

/// 100 x 80, 2 s at 30 fps, with a rectangle that slides from x=0 to x=100.
const SLIDER: &str = r#"{"fr":30,"ip":0,"op":60,"w":100,"h":80,"layers":[
    {"ty":4,"nm":"L","ks":{"p":{"a":1,"k":[{"t":0,"s":[0,0]},{"t":60,"s":[100,0]}]}},
     "shapes":[{"ty":"gr","it":[
        {"ty":"rc","p":{"a":0,"k":[0,0]},"s":{"a":0,"k":[10,10]},"r":{"a":0,"k":0}},
        {"ty":"fl","c":{"a":0,"k":[1,0,0,1]},"o":{"a":0,"k":100}}]}]}]}"#;

fn animation() -> Arc<LottieScene> {
    Arc::new(LottieScene::new(Composition::parse(SLIDER).expect("valid document")))
}

pub(crate) fn context(scale: f32) -> BuildContext<'static> {
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

/// Runs one retained paint frame and returns the draw commands it recorded.
pub(crate) fn paint(ctx: &BuildContext, element: &AnyElement, width: f32, height: f32) -> Vec<DrawCommand> {
    let tree = RenderTree::new();
    let node = tree.add_root(Rect::new(0.0, 0.0, width, height)).unwrap();
    let nodes = Rc::new(aimer_widget::ElementNodeMap::from([(element.id(), node)]));
    ctx.canvas.begin_frame();
    aimer_widget::begin_paint_frame(200, 200);
    aimer_widget::with_v2_render_tree_context(tree.clone(), nodes, Some(element.id()), || {
        element.update(ctx)
    });
    tree.draw_list_snapshot(node).unwrap().commands.to_vec()
}

pub(crate) fn svg_draws(commands: &[DrawCommand]) -> Vec<(&Arc<aimer_cupid::svg::SvgScene>, Rect, &[aimer_cupid::svg::SvgNodeStyleOverride])> {
    commands
        .iter()
        .filter_map(|c| match c {
            DrawCommand::Svg { scene, destination, overrides } => Some((scene, *destination, &overrides[..])),
            _ => None,
        })
        .collect()
}

// ---- fit ------------------------------------------------------------------

#[test]
fn contain_centers_with_letterbox() {
    assert_eq!(fit_destination((100.0, 80.0), (200.0, 80.0), LottieFit::Contain), Rect::new(50.0, 0.0, 100.0, 80.0));
    assert_eq!(fit_destination((100.0, 80.0), (100.0, 160.0), LottieFit::Contain), Rect::new(0.0, 40.0, 100.0, 80.0));
    assert_eq!(fit_destination((100.0, 80.0), (200.0, 160.0), LottieFit::Contain), Rect::new(0.0, 0.0, 200.0, 160.0));
}

#[test]
fn cover_overflows_the_short_axis() {
    assert_eq!(fit_destination((100.0, 80.0), (200.0, 80.0), LottieFit::Cover), Rect::new(0.0, -40.0, 200.0, 160.0));
}

#[test]
fn fill_stretches() {
    assert_eq!(fit_destination((100.0, 80.0), (200.0, 30.0), LottieFit::Fill), Rect::new(0.0, 0.0, 200.0, 30.0));
}

#[test]
fn a_zero_sized_box_fits_to_nothing_without_nan() {
    for fit in [LottieFit::Contain, LottieFit::Cover, LottieFit::Fill] {
        let r = fit_destination((100.0, 80.0), (0.0, 0.0), fit);
        assert!(r.width == 0.0 && r.height == 0.0 && r.x.is_finite() && r.y.is_finite(), "{fit:?} {r:?}");
    }
}

// ---- layout ---------------------------------------------------------------

#[tokio::test]
async fn natural_size_is_the_canvas_scaled() {
    let ctx = context(2.0);
    let element = Lottie::new().animation(animation()).to_element(&ctx);
    assert_eq!(element.computed_size(&ctx), ResolvedSize { width: 200.0, height: 160.0 });
}

#[tokio::test]
async fn one_given_axis_derives_the_other_from_the_aspect_ratio() {
    let ctx = context(1.0);
    let by_width = Lottie::new().width(50.0).animation(animation()).to_element(&ctx);
    assert_eq!(by_width.computed_size(&ctx), ResolvedSize { width: 50.0, height: 40.0 });
    let by_height = Lottie::new().height(40.0).animation(animation()).to_element(&ctx);
    assert_eq!(by_height.computed_size(&ctx), ResolvedSize { width: 50.0, height: 40.0 });
}

#[tokio::test]
async fn both_axes_given_are_used_as_they_are() {
    let ctx = context(2.0);
    let element = Lottie::new().width(30.0).height(10.0).animation(animation()).to_element(&ctx);
    assert_eq!(element.computed_size(&ctx), ResolvedSize { width: 60.0, height: 20.0 });
    assert_eq!(element.size(), Some(Size { width: Dimension::Px(30.0), height: Dimension::Px(10.0) }));
    assert!(element.is_layout_stable());
}

#[tokio::test]
async fn a_percentage_depends_on_the_parent_so_it_is_not_stable() {
    let ctx = context(1.0);
    let element = Lottie::new().width(Dimension::Percent(50.0)).animation(animation()).to_element(&ctx);
    assert!(!element.is_layout_stable());
    assert_eq!(element.size(), None);
}

// ---- painting ---------------------------------------------------------------

#[tokio::test]
async fn a_fixed_progress_draws_that_frame_of_the_scene() {
    let ctx = context(1.0);
    let element = Lottie::new().progress(0.5).animation(animation()).to_element(&ctx);
    let commands = paint(&ctx, &element, 100.0, 80.0);
    let draws = svg_draws(&commands);
    assert_eq!(draws.len(), 1, "{commands:?}");
    let (scene, destination, overrides) = draws[0];
    assert_eq!(destination, Rect::new(0.0, 0.0, 100.0, 80.0));
    assert_eq!(scene.nodes.len(), 1);
    // Halfway through 0..60 frames the rectangle has slid 50 units.
    assert_eq!(overrides[0].transform.map(|t| t.tx), Some(50.0));
}

#[tokio::test]
async fn the_destination_follows_the_fit() {
    let ctx = context(1.0);
    let element = Lottie::new().width(200.0).height(80.0).progress(0.0).animation(animation()).to_element(&ctx);
    let commands = paint(&ctx, &element, 200.0, 80.0);
    assert_eq!(svg_draws(&commands)[0].1, Rect::new(50.0, 0.0, 100.0, 80.0));
}

#[tokio::test]
async fn a_still_animation_is_recorded_once_until_the_box_changes() {
    let ctx = context(1.0);
    let element = Lottie::new().progress(0.25).animation(animation()).to_element(&ctx);
    assert!(element.paint_needs_recording(&ctx));
    paint(&ctx, &element, 100.0, 80.0);
    assert!(!element.paint_needs_recording(&ctx));
    assert!(element.paint_needs_recording(&context(2.0)), "a new scale needs a new recording");
}

#[tokio::test]
async fn an_unusable_scale_is_not_painted() {
    for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        let ctx = context(scale);
        let element = Lottie::new().animation(animation()).to_element(&ctx);
        assert!(!element.can_paint(&ctx), "scale {scale}");
    }
}

#[tokio::test]
async fn it_does_not_take_pointer_events() {
    let ctx = context(1.0);
    let element = Lottie::new().animation(animation()).to_element(&ctx);
    assert_eq!(element.event_tree_role(), EventTreeRole::Transparent);
}

// ---- rebuilds ---------------------------------------------------------------

fn raw(element: &AnyElement) -> &RawLottie {
    element.option_any().and_then(|any| any.downcast_ref::<RawLottie>()).expect("a Lottie element")
}

#[tokio::test]
async fn a_rebuild_with_the_same_animation_keeps_the_playhead() {
    let ctx = context(1.0);
    let scene = animation();
    let old = Lottie::new().animation(scene.clone()).to_element(&ctx);
    let mut playback = raw(&old).playback.get();
    playback.advance(std::time::Duration::from_secs(1));
    raw(&old).playback.set(playback);

    let new = Lottie::new().animation(scene).to_element(&ctx);
    assert_eq!(raw(&new).playback.get().frame(), 0.0);
    new.adopt_runtime_state_from(&*old);
    assert_eq!(raw(&new).playback.get().frame(), 30.0);
}

#[tokio::test]
async fn a_different_animation_starts_over() {
    let ctx = context(1.0);
    let old = Lottie::new().animation(animation()).to_element(&ctx);
    let mut playback = raw(&old).playback.get();
    playback.advance(std::time::Duration::from_secs(1));
    raw(&old).playback.set(playback);

    let new = Lottie::new().animation(animation()).to_element(&ctx);
    new.adopt_runtime_state_from(&*old);
    assert_eq!(raw(&new).playback.get().frame(), 0.0);
}

#[tokio::test]
async fn a_fixed_progress_is_not_overridden_by_the_old_playhead() {
    let ctx = context(1.0);
    let scene = animation();
    let old = Lottie::new().animation(scene.clone()).to_element(&ctx);
    let mut playback = raw(&old).playback.get();
    playback.advance(std::time::Duration::from_secs(1));
    raw(&old).playback.set(playback);

    let new = Lottie::new().progress(0.25).animation(scene).to_element(&ctx);
    new.adopt_runtime_state_from(&*old);
    assert_eq!(raw(&new).playback.get().frame(), 15.0);
}

#[tokio::test]
async fn a_playing_element_keeps_a_clock_and_a_still_one_does_not() {
    let ctx = context(1.0);
    let playing = Lottie::new().animation(animation()).to_element(&ctx);
    assert!(playing.sync_paint_state(&ctx));
    assert!(raw(&playing).last_tick.get().is_some(), "playing needs the next tick's start");

    let still = Lottie::new().progress(0.5).animation(animation()).to_element(&ctx);
    assert!(still.sync_paint_state(&ctx));
    assert!(raw(&still).last_tick.get().is_none(), "a still animation asks for no frames");

    let paused = Lottie::new().playing(false).animation(animation()).to_element(&ctx);
    assert!(paused.sync_paint_state(&ctx));
    assert!(raw(&paused).last_tick.get().is_none());
}
