use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use aimer_attribute::size::ResolvedSize;
use aimer_cupid::draw_cmd::Rect;
use aimer_decoration::Decoration;
use aimer_widget::{AnyElement, Drawable, Element, LayoutElement, Rebuildable, Widget};

use crate::matte::{Clock, Cut, PlaneWidget, RawMattes};
use crate::widget::{LottieFit, RawLottie};
use crate::widget_tests::{context, paint, svg_draws};
use crate::{Block, Composition, Lottie, LottieScene, Playback};

const RECT: &str = r#"{"ty":"rc","p":{"a":0,"k":[0,0]},"s":{"a":0,"k":[10,10]},"r":{"a":0,"k":0}}"#;
const RED: &str = r#"{"ty":"fl","c":{"a":0,"k":[1,0,0,1]},"o":{"a":0,"k":100}}"#;

/// Listed top first: a plain layer, a layer that slides from x=0 to x=100 and is
/// cut by the matte, and the matte. Drawn bottom first that is the cut layer,
/// then the plain one.
fn matted_source() -> String {
    format!(
        r#"{{"fr":30,"ip":0,"op":60,"w":100,"h":80,"layers":[
        {{"ty":4,"nm":"above","ind":1,"ks":{{}},"shapes":[{RECT},{RED}]}},
        {{"ty":4,"nm":"cut","ind":2,"tt":1,"tp":3,
          "ks":{{"p":{{"a":1,"k":[{{"t":0,"s":[0,0]}},{{"t":60,"s":[100,0]}}]}}}},
          "shapes":[{RECT},{RED}]}},
        {{"ty":4,"nm":"matte","ind":3,"td":1,"ks":{{}},"shapes":[{RECT},{RED}]}}]}}"#
    )
}

fn matted() -> Arc<LottieScene> {
    Arc::new(LottieScene::new(Composition::parse(matted_source()).expect("valid document")))
}

fn mattes(element: &AnyElement) -> &RawMattes {
    element.option_any().and_then(|any| any.downcast_ref::<RawMattes>()).expect("a matte wrapper")
}

fn names(element: &dyn Element) -> Vec<&'static str> {
    let mut out = Vec::new();
    element.visit_children(&mut |child| out.push(child.debug_name()));
    out
}

fn children(element: &dyn Element) -> Vec<&dyn Element> {
    let mut out = Vec::new();
    element.visit_children(&mut |child| out.push(child));
    out
}

fn clock() -> Rc<Clock> {
    Rc::new(Clock::new(Playback::new(0.0, 60.0, 30.0).playing(true)))
}

#[test]
fn the_fixture_has_the_blocks_the_tests_assume() {
    assert_eq!(matted().blocks(), [Block::Matted { content: 0, matte: 1 }, Block::Plain(2)]);
}

#[tokio::test]
async fn a_scene_with_a_matte_is_a_wrapper_over_a_stack_of_its_blocks() {
    let ctx = context(1.0);
    let element = Lottie::new().animation(matted()).to_element(&ctx);
    assert!(element.option_any().is_some_and(|any| any.is::<RawMattes>()));
    let layers = children(&*element);
    assert_eq!(layers.len(), 1, "the wrapper holds the stack");
    // Bottom first: the cut layer under its matte, then the plain layer.
    assert_eq!(names(layers[0]), ["MaskedDecorationBox", "LottiePlane"]);
    let masked = children(layers[0])[0];
    assert_eq!(names(masked), ["LottiePlane"], "the matte is the masking child");
}

#[tokio::test]
async fn a_scene_without_a_matte_stays_one_element() {
    let ctx = context(1.0);
    let flat = Arc::new(LottieScene::new(
        Composition::parse(format!(
            r#"{{"fr":30,"ip":0,"op":60,"w":100,"h":80,"layers":[
            {{"ty":4,"nm":"a","ind":1,"ks":{{}},"shapes":[{RECT},{RED}]}}]}}"#
        ))
        .expect("valid document"),
    ));
    let element = Lottie::new().animation(flat).to_element(&ctx);
    assert!(element.option_any().is_some_and(|any| any.is::<RawLottie>()));
    assert!(children(&*element).is_empty());
}

#[tokio::test]
async fn the_wrapper_is_sized_like_a_plain_lottie() {
    let ctx = context(2.0);
    let natural = Lottie::new().animation(matted()).to_element(&ctx);
    assert_eq!(natural.computed_size(&ctx), ResolvedSize { width: 200.0, height: 160.0 });
    let by_width = Lottie::new().width(50.0).animation(matted()).to_element(&ctx);
    assert_eq!(by_width.computed_size(&ctx), ResolvedSize { width: 100.0, height: 80.0 });
    let fixed = Lottie::new().width(10.0).height(10.0).animation(matted()).to_element(&ctx);
    assert!(fixed.is_layout_stable());
    let relative = Lottie::new().width(aimer_attribute::dimension::Dimension::Percent(50.0)).animation(matted()).to_element(&ctx);
    assert!(!relative.is_layout_stable(), "a percentage depends on the parent");
}

#[tokio::test]
async fn a_plane_fills_the_space_it_is_given_and_draws_its_own_nodes() {
    let mut ctx = context(1.0);
    ctx.parent_size = ResolvedSize { width: 100.0, height: 80.0 };
    let element = PlaneWidget::new(matted(), 0, LottieFit::Contain, clock()).to_element(&ctx);
    assert_eq!(element.computed_size(&ctx), ResolvedSize { width: 100.0, height: 80.0 });
    let commands = paint(&ctx, &element, 100.0, 80.0);
    let draws = svg_draws(&commands);
    assert_eq!(draws.len(), 1);
    assert_eq!(draws[0].1, Rect::new(0.0, 0.0, 100.0, 80.0));
    assert_eq!(draws[0].0.nodes.len(), 1, "one node: the cut layer's rect");
}

#[tokio::test]
async fn a_plane_follows_the_shared_clock_and_is_recorded_once_per_frame() {
    let mut ctx = context(1.0);
    ctx.parent_size = ResolvedSize { width: 100.0, height: 80.0 };
    let clock = clock();
    let element = PlaneWidget::new(matted(), 0, LottieFit::Contain, clock.clone()).to_element(&ctx);
    let _ = paint(&ctx, &element, 100.0, 80.0);
    assert!(!element.paint_needs_recording(&ctx), "nothing moved since it was recorded");

    let mut playback = clock.playback.get();
    playback.advance(Duration::from_secs(1));
    clock.playback.set(playback);
    assert!(element.paint_needs_recording(&ctx), "the shared clock moved");
    let commands = paint(&ctx, &element, 100.0, 80.0);
    let draws = svg_draws(&commands);
    let moved = draws[0].2.iter().find(|o| o.node_id.0 == 0).and_then(|o| o.transform).expect("moved");
    assert_eq!(moved.tx, 50.0, "frame 30 of a slide from 0 to 100 over 60 frames");
}

#[test]
fn the_cut_layer_is_re_recorded_when_the_clock_moves() {
    let clock = clock();
    let cut = Cut::new(matted(), 0, LottieFit::Contain, clock.clone());
    let before = cut.revision();
    assert_eq!(cut.revision(), before, "the same frame is the same revision");
    let mut playback = clock.playback.get();
    playback.advance(Duration::from_secs(1));
    clock.playback.set(playback);
    assert_ne!(cut.revision(), before);
    assert!(!cut.is_empty());
}

#[tokio::test]
async fn the_wrapper_advances_the_clock_the_planes_share() {
    let ctx = context(1.0);
    let playing = Lottie::new().animation(matted()).to_element(&ctx);
    assert!(playing.sync_paint_state(&ctx));
    assert!(mattes(&playing).clock.last_tick.get().is_some(), "playing needs the next tick's start");

    let paused = Lottie::new().playing(false).animation(matted()).to_element(&ctx);
    assert!(paused.sync_paint_state(&ctx));
    assert!(mattes(&paused).clock.last_tick.get().is_none());
}

#[tokio::test]
async fn a_rebuild_keeps_the_playhead_of_the_same_animation_only() {
    let ctx = context(1.0);
    let scene = matted();
    let old = Lottie::new().animation(scene.clone()).to_element(&ctx);
    let mut playback = mattes(&old).clock.playback.get();
    playback.advance(Duration::from_secs(1));
    mattes(&old).clock.playback.set(playback);

    let same = Lottie::new().animation(scene.clone()).to_element(&ctx);
    same.adopt_runtime_state_from(&*old);
    assert_eq!(mattes(&same).clock.playback.get().frame(), 30.0);

    let other = Lottie::new().animation(matted()).to_element(&ctx);
    other.adopt_runtime_state_from(&*old);
    assert_eq!(mattes(&other).clock.playback.get().frame(), 0.0);

    let fixed = Lottie::new().progress(0.25).animation(scene).to_element(&ctx);
    fixed.adopt_runtime_state_from(&*old);
    assert_eq!(mattes(&fixed).clock.playback.get().frame(), 15.0);
}
