//! A page growing out of the item that opened it, and shrinking back into it.

use std::time::Duration;

use aimer_cupid::draw_cmd_v2::{Rect, RenderNodeId, RenderTree};
use aimer_router::{
    Curve, curves, ContainerTransition, ExpandTransition, NavigationAction, NavigationContext, NavigationTransition, Route,
    NavigationTransitionFrame, ResolvedNavigationTransition,
};
use aimer_widget::base::{BuildContext, ResolvedSize, WindowHandle};
use aimer_widget::{AnyElement, ErrorWidget, Key, Widget};

#[derive(Clone)]
struct Page(u32);

impl Route for Page {
    fn parse(_: &str) -> Option<Self> { None }
    fn format(&self) -> String { format!("/{}", self.0) }
}

const VIEWPORT: ResolvedSize = ResolvedSize { width: 400.0, height: 800.0 };

fn context() -> BuildContext<'static> {
    use std::sync::OnceLock;
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    let runtime = RUNTIME.get_or_init(|| tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap());
    let canvas = aimer_canvas::FrameCanvas::new(Box::leak(Box::new(aimer_canvas::InnerCanvas::new())));
    BuildContext::new(canvas, VIEWPORT, 1.0,
        Default::default(), Default::default(), WindowHandle::headless(Default::default(), 1.0),
        runtime.handle().clone())
}

/// Page `n` is opened from the item tagged `album-n`; page 0 has no item.
fn transition() -> ContainerTransition<Page> {
    ContainerTransition::new().tag(|page: &Page| (page.0 != 0).then(|| Key::from(format!("album-{}", page.0))))
}

/// An item on screen: its tag is registered against a node at `bounds`.
struct Item {
    element: AnyElement,
    tree: RenderTree,
    node: RenderNodeId,
}

fn item(tag: &str, bounds: Rect, ctx: &BuildContext) -> Item {
    let element = aimer_animation::SharedElement::new().tag(tag).child(ErrorWidget::new("cover")).to_element(ctx);
    registered(element, bounds, ctx)
}

/// Like [`item`], but the tile can be drawn again, so the transition can lead
/// with a copy of it.
fn repeatable_item(tag: &str, bounds: Rect, ctx: &BuildContext) -> Item {
    let element = aimer_animation::SharedElement::new().tag(tag)
        .child_with(|| ErrorWidget::new("cover")).to_element(ctx);
    registered(element, bounds, ctx)
}

fn registered(element: AnyElement, bounds: Rect, ctx: &BuildContext) -> Item {
    let tree = RenderTree::new();
    let node = tree.add_root(bounds).unwrap();
    let retained = tree.context(node).unwrap();
    assert!(ctx.with_local_v2_paint_context(retained, |ctx| element.sync_local_v2_state(ctx)));
    Item { element, tree, node }
}

fn resolve(
    transition: &ContainerTransition<Page>,
    from: u32,
    to: u32,
    action: NavigationAction,
    ctx: &BuildContext,
) -> Option<ResolvedNavigationTransition> {
    transition.resolve(&NavigationContext { from: &Page(from), to: &Page(to), action, build_context: ctx })
}

fn corner(frame: &NavigationTransitionFrame, x: f32, y: f32) -> (f32, f32) {
    frame.incoming.transform.transform_point(x, y)
}

fn near(actual: (f32, f32), expected: (f32, f32)) {
    assert!((actual.0 - expected.0).abs() < 1e-3 && (actual.1 - expected.1).abs() < 1e-3,
        "{actual:?} != {expected:?}");
}

#[test]
fn a_push_grows_the_destination_out_of_the_tagged_item() {
    let ctx = context();
    let _item = item("album-7", Rect::new(20.0, 120.0, 160.0, 160.0), &ctx);

    let effect = resolve(&transition(), 0, 7, NavigationAction::Push, &ctx).unwrap();

    let start = effect.sample(0.0, VIEWPORT).unwrap();
    near(corner(&start, 0.0, 0.0), (20.0, 120.0));
    near(corner(&start, VIEWPORT.width, 0.0), (180.0, 120.0));
    assert_eq!(start.incoming.corner_radius, 12.0);
    let end = effect.sample(1.0, VIEWPORT).unwrap();
    assert_eq!(end.incoming.transform, aimer_canvas::Mat3::identity());
    assert_eq!(end.incoming.corner_radius, 0.0);
    // The page behind stays underneath and dims.
    assert!(!effect.is_outgoing_on_top());
    assert_eq!(start.outgoing.opacity, 1.0);
    assert!(end.outgoing.opacity < 1.0);
}

#[test]
fn the_item_is_measured_from_the_hosts_corner_not_the_windows() {
    let ctx = context();
    let _item = item("album-7", Rect::new(20.0, 120.0, 160.0, 160.0), &ctx);
    let effect = resolve(&transition(), 0, 7, NavigationAction::Push, &ctx).unwrap();

    // A router beside a 10x50 sidebar: the item is that much closer to its corner.
    let start = effect.sample_at(0.0, VIEWPORT, (10.0, 50.0)).unwrap();

    near(corner(&start, 0.0, 0.0), (10.0, 70.0));
}

#[test]
fn a_pop_shrinks_the_departing_page_into_the_item_and_keeps_it_visible_on_top() {
    let ctx = context();
    let _item = item("album-7", Rect::new(20.0, 120.0, 160.0, 160.0), &ctx);

    let effect = resolve(&transition(), 7, 0, NavigationAction::Pop, &ctx).unwrap();

    // Playback runs 0 -> 1; a pop begins with the page filling the viewport...
    let start = effect.sample(0.0, VIEWPORT).unwrap();
    assert_eq!(start.outgoing.transform, aimer_canvas::Mat3::identity());
    assert_eq!(start.outgoing.corner_radius, 0.0);
    // ...and ends with it back on the item, painted above the returning page.
    let end = effect.sample(1.0, VIEWPORT).unwrap();
    near(end.outgoing.transform.transform_point(0.0, 0.0), (20.0, 120.0));
    assert_eq!(end.outgoing.corner_radius, 12.0);
    assert!(effect.is_outgoing_on_top());
    assert_eq!(end.incoming.opacity, 1.0);
}

#[test]
fn a_pop_finds_the_item_after_its_page_was_dropped() {
    let ctx = context();
    drop(item("album-7", Rect::new(20.0, 120.0, 160.0, 160.0), &ctx));

    // The page holding the item is rebuilt only after the pop has resolved.
    let effect = resolve(&transition(), 7, 0, NavigationAction::Pop, &ctx).unwrap();

    near(effect.sample(1.0, VIEWPORT).unwrap().outgoing.transform.transform_point(0.0, 0.0), (20.0, 120.0));
}

#[test]
fn the_item_is_looked_up_again_every_frame() {
    let ctx = context();
    let item = item("album-7", Rect::new(20.0, 120.0, 160.0, 160.0), &ctx);
    let effect = resolve(&transition(), 0, 7, NavigationAction::Push, &ctx).unwrap();

    // The grid scrolls while the page is growing.
    item.tree.set_bounds(item.node, Rect::new(20.0, 60.0, 160.0, 160.0)).unwrap();

    near(corner(&effect.sample(0.0, VIEWPORT).unwrap(), 0.0, 0.0), (20.0, 60.0));
}

#[test]
fn an_item_that_disappears_mid_flight_leaves_the_last_known_position() {
    let ctx = context();
    let live = item("album-7", Rect::new(20.0, 120.0, 160.0, 160.0), &ctx);
    let effect = resolve(&transition(), 0, 7, NavigationAction::Push, &ctx).unwrap();
    let before = effect.sample(0.0, VIEWPORT).unwrap();

    drop(live);

    assert_eq!(effect.sample(0.0, VIEWPORT).unwrap(), before);
}

#[test]
fn without_a_tagged_item_it_falls_back_to_a_plain_zoom() {
    let ctx = context();
    for (to, case) in [(8, "tag registered nowhere"), (0, "the route has no tag")] {
        let effect = resolve(&transition(), 1, to, NavigationAction::Push, &ctx)
            .unwrap_or_else(|| panic!("{case}: should still animate"));

        let start = effect.sample(0.0, VIEWPORT).unwrap();
        assert_eq!(start.incoming.corner_radius, 0.0, "{case}");
        assert!(!effect.is_outgoing_on_top(), "{case}");
        // Zoom scales about the viewport centre, so the centre does not move.
        near(corner(&start, 200.0, 400.0), (200.0, 400.0));
    }
}

#[test]
fn the_effect_is_configurable() {
    let ctx = context();
    let _item = item("album-7", Rect::new(20.0, 120.0, 160.0, 160.0), &ctx);
    let configured = transition()
        .duration(Duration::from_millis(250))
        .curve(curves::LINEAR)
        .source_radius(24.0)
        .page_radius(6.0)
        .scrim(0.5);

    let effect = resolve(&configured, 0, 7, NavigationAction::Push, &ctx).unwrap();

    assert_eq!(effect.duration(), Duration::from_millis(250));
    assert_eq!(effect.curve(), curves::LINEAR.into_any());
    assert_eq!(effect.sample(0.0, VIEWPORT).unwrap().incoming.corner_radius, 24.0);
    assert_eq!(effect.sample(1.0, VIEWPORT).unwrap().incoming.corner_radius, 6.0);
    assert_eq!(effect.sample(1.0, VIEWPORT).unwrap().outgoing.opacity, 0.5);
}

#[test]
fn an_untagged_transition_always_falls_back() {
    let ctx = context();
    let _item = item("album-7", Rect::new(20.0, 120.0, 160.0, 160.0), &ctx);

    let effect = resolve(&ContainerTransition::new(), 0, 7, NavigationAction::Push, &ctx).unwrap();

    assert_eq!(effect.sample(0.0, VIEWPORT).unwrap().incoming.corner_radius, 0.0);
}

#[test]
fn a_zero_duration_swaps_immediately() {
    let ctx = context();
    let _item = item("album-7", Rect::new(20.0, 120.0, 160.0, 160.0), &ctx);

    let effect = resolve(&transition().duration(Duration::ZERO), 0, 7, NavigationAction::Push, &ctx).unwrap();

    assert!(effect.duration().is_zero());
}

#[test]
fn a_repeatable_item_leads_the_motion_with_a_copy_of_itself() {
    let ctx = context();
    let _item = repeatable_item("album-7", Rect::new(20.0, 120.0, 160.0, 160.0), &ctx);

    let effect = resolve(&transition(), 0, 7, NavigationAction::Push, &ctx).unwrap();

    let anchor = effect.anchor().expect("the item can be drawn again");
    assert_eq!((anchor.size().width, anchor.size().height), (160.0, 160.0));
    let start = effect.sample_anchored_at(0.0, VIEWPORT, (0.0, 0.0)).unwrap();
    // The copy lies exactly on the item and the page is not there yet.
    near(start.anchor.transform.transform_point(0.0, 0.0), (20.0, 120.0));
    near(start.anchor.transform.transform_point(160.0, 160.0), (180.0, 280.0));
    assert_eq!(start.anchor.opacity, 1.0);
    assert_eq!(start.frame.incoming.opacity, 0.0);
    // By the end the copy is gone and the page fills the host.
    let end = effect.sample_anchored_at(1.0, VIEWPORT, (0.0, 0.0)).unwrap();
    assert_eq!(end.anchor.opacity, 0.0);
    assert_eq!(end.frame.incoming.opacity, 1.0);
}

#[test]
fn the_page_takes_over_from_the_item_a_quarter_of_the_way_in() {
    let ctx = context();
    let _item = repeatable_item("album-7", Rect::new(20.0, 120.0, 160.0, 160.0), &ctx);
    let effect = resolve(&transition(), 0, 7, NavigationAction::Push, &ctx).unwrap();

    let before = effect.sample_anchored_at(0.15, VIEWPORT, (0.0, 0.0)).unwrap();
    let after = effect.sample_anchored_at(0.40, VIEWPORT, (0.0, 0.0)).unwrap();

    assert_eq!((before.anchor.opacity, before.frame.incoming.opacity), (1.0, 0.0));
    assert!(after.anchor.opacity.abs() < 1e-3 && (after.frame.incoming.opacity - 1.0).abs() < 1e-3);
}

#[test]
fn a_pop_plays_the_same_thing_in_reverse_and_ends_on_the_item() {
    let ctx = context();
    let _item = repeatable_item("album-7", Rect::new(20.0, 120.0, 160.0, 160.0), &ctx);

    let effect = resolve(&transition(), 7, 0, NavigationAction::Pop, &ctx).unwrap();

    // The pop starts with the page alone, filling the host...
    let start = effect.sample_anchored_at(0.0, VIEWPORT, (0.0, 0.0)).unwrap();
    assert_eq!((start.anchor.opacity, start.frame.outgoing.opacity), (0.0, 1.0));
    // ...hands over to the item's copy three quarters of the way through...
    let late = effect.sample_anchored_at(0.90, VIEWPORT, (0.0, 0.0)).unwrap();
    assert_eq!((late.anchor.opacity, late.frame.outgoing.opacity), (1.0, 0.0));
    // ...which finishes on the item itself.
    let end = effect.sample_anchored_at(1.0, VIEWPORT, (0.0, 0.0)).unwrap();
    near(end.anchor.transform.transform_point(0.0, 0.0), (20.0, 120.0));
    near(end.anchor.transform.transform_point(160.0, 160.0), (180.0, 280.0));
    assert!(effect.is_outgoing_on_top());
}

#[test]
fn a_pop_can_lead_with_the_copy_after_the_items_page_was_dropped() {
    let ctx = context();
    drop(repeatable_item("album-7", Rect::new(20.0, 120.0, 160.0, 160.0), &ctx));

    let effect = resolve(&transition(), 7, 0, NavigationAction::Pop, &ctx).unwrap();

    assert!(effect.anchor().is_some(), "the remembered item can still be drawn again");
}

#[test]
fn an_item_that_cannot_be_repeated_keeps_the_page_only_motion() {
    let ctx = context();
    let _item = item("album-7", Rect::new(20.0, 120.0, 160.0, 160.0), &ctx);

    let effect = resolve(&transition(), 0, 7, NavigationAction::Push, &ctx).unwrap();

    assert!(effect.anchor().is_none());
    // The page fades itself in over the first quarter.
    assert_eq!(effect.sample(0.125, VIEWPORT).unwrap().incoming.opacity, 0.5);
}

#[test]
fn the_copy_is_measured_from_the_hosts_corner_too() {
    let ctx = context();
    let _item = repeatable_item("album-7", Rect::new(20.0, 120.0, 160.0, 160.0), &ctx);
    let effect = resolve(&transition(), 0, 7, NavigationAction::Push, &ctx).unwrap();

    let start = effect.sample_anchored_at(0.0, VIEWPORT, (10.0, 50.0)).unwrap();

    near(start.anchor.transform.transform_point(0.0, 0.0), (10.0, 70.0));
}

#[test]
fn the_page_curve_reshapes_the_pages_growth_in_both_directions() {
    let ctx = context();
    let _item = item("album-7", Rect::new(20.0, 120.0, 160.0, 160.0), &ctx);
    let plain = resolve(&transition(), 0, 7, NavigationAction::Push, &ctx).unwrap();
    let eased = resolve(&transition().page_curve(curves::EASE_IN), 0, 7, NavigationAction::Push, &ctx).unwrap();

    // Halfway through, the eased page is where the plain one is at EaseIn(0.5).
    let got = eased.sample(0.5, VIEWPORT).unwrap().incoming.transform;
    let expected = plain.sample(curves::EASE_IN.transform(0.5), VIEWPORT).unwrap().incoming.transform;
    assert_eq!(got, expected);
    assert_ne!(got, plain.sample(0.5, VIEWPORT).unwrap().incoming.transform);

    // Going back runs the same curve over reversed time.
    let pop = resolve(&transition().page_curve(curves::EASE_IN), 7, 0, NavigationAction::Pop, &ctx).unwrap();
    assert_eq!(pop.sample(0.25, VIEWPORT).unwrap().outgoing.transform,
        eased.sample(0.75, VIEWPORT).unwrap().incoming.transform);
}

#[test]
fn the_anchor_curve_reshapes_the_copys_travel_and_leaves_its_fade_alone() {
    let ctx = context();
    let _item = repeatable_item("album-7", Rect::new(20.0, 120.0, 160.0, 160.0), &ctx);
    let plain = resolve(&transition(), 0, 7, NavigationAction::Push, &ctx).unwrap();
    let eased = resolve(&transition().anchor_curve(curves::EASE_IN), 0, 7, NavigationAction::Push, &ctx).unwrap();
    let origin = (0.0, 0.0);

    let plain_frame = plain.sample_anchored_at(0.125, VIEWPORT, origin).unwrap();
    let eased_frame = eased.sample_anchored_at(0.125, VIEWPORT, origin).unwrap();

    assert_ne!(plain_frame.anchor.transform, eased_frame.anchor.transform);
    assert_eq!(plain_frame.anchor.opacity, eased_frame.anchor.opacity);
    // It still arrives on the card at the handoff.
    let landed = eased.sample_anchored_at(0.25, VIEWPORT, origin).unwrap();
    near(landed.anchor.transform.transform_point(0.0, 0.0), landed.frame.incoming.transform.transform_point(0.0, 0.0));
}

#[test]
fn an_invalid_element_curve_is_ignored() {
    let ctx = context();
    let _item = repeatable_item("album-7", Rect::new(20.0, 120.0, 160.0, 160.0), &ctx);
    let bad = curves::cubic_bezier(f32::NAN, 0.0, 1.0, 1.0);
    let plain = resolve(&transition(), 0, 7, NavigationAction::Push, &ctx).unwrap();
    let ignored = resolve(&transition().anchor_curve(bad).page_curve(bad), 0, 7, NavigationAction::Push, &ctx).unwrap();

    for step in 0..=10 {
        let p = step as f32 / 10.0;
        assert_eq!(plain.sample_anchored_at(p, VIEWPORT, (0.0, 0.0)), ignored.sample_anchored_at(p, VIEWPORT, (0.0, 0.0)));
    }
}

#[test]
fn a_users_own_curve_can_shape_the_page() {
    /// Jumps straight to the end: the page is full size from the first frame.
    struct Immediately;
    impl Curve for Immediately {
        fn transform(&self, _: f32) -> f32 { 1.0 }
    }
    let ctx = context();
    let _item = item("album-7", Rect::new(20.0, 120.0, 160.0, 160.0), &ctx);

    let effect = resolve(&transition().page_curve(Immediately), 0, 7, NavigationAction::Push, &ctx).unwrap();

    let at_start = effect.sample(0.0, VIEWPORT).unwrap();
    assert_eq!(at_start.incoming.transform, aimer_canvas::Mat3::identity());
    assert_eq!(at_start.incoming.corner_radius, 0.0);
}

#[test]
fn a_users_own_curve_can_ease_the_whole_navigation() {
    struct Halfway;
    impl Curve for Halfway {
        fn transform(&self, t: f32) -> f32 { t * 0.5 }
    }
    let ctx = context();
    let _item = item("album-7", Rect::new(20.0, 120.0, 160.0, 160.0), &ctx);

    let effect = resolve(&transition().curve(Halfway), 0, 7, NavigationAction::Push, &ctx).unwrap();

    assert_eq!(effect.curve().transform(1.0), 0.5);
    assert!(effect.curve().descriptor().is_none(), "a custom curve has no portable description");
}

/// A row as wide as the host, which a scaled page could only slide out of.
fn revealing() -> ContainerTransition<Page> {
    transition().reveal(true)
}

#[test]
fn a_revealed_push_opens_a_window_on_the_item_and_never_scales_the_page() {
    let ctx = context();
    let _item = item("album-7", Rect::new(0.0, 260.0, 400.0, 160.0), &ctx);

    let effect = resolve(&revealing(), 0, 7, NavigationAction::Push, &ctx).unwrap();

    let start = effect.sample(0.0, VIEWPORT).unwrap();
    near(corner(&start, 0.0, 0.0), (0.0, 260.0));
    near(corner(&start, 1.0, 1.0), (1.0, 261.0));
    let window = start.incoming.reveal.expect("the page shows through a window");
    assert_eq!((window.width, window.height), (400.0, 160.0));
    assert_eq!(start.incoming.corner_radius, 12.0);
    let end = effect.sample(1.0, VIEWPORT).unwrap();
    assert_eq!(end.incoming.transform, aimer_canvas::Mat3::identity());
    // The window has grown to the whole page, which needs no clip.
    assert_eq!(end.incoming.reveal, None);
    // The page behind still dims, and a plain transition has no window at all.
    assert!(end.outgoing.opacity < 1.0);
    let plain = resolve(&transition(), 0, 7, NavigationAction::Push, &ctx).unwrap();
    assert_eq!(plain.sample(0.5, VIEWPORT).unwrap().incoming.reveal, None);
}

#[test]
fn a_revealed_pop_closes_the_window_back_onto_the_item_above_the_returning_page() {
    let ctx = context();
    let _item = item("album-7", Rect::new(0.0, 260.0, 400.0, 160.0), &ctx);

    let effect = resolve(&revealing(), 7, 0, NavigationAction::Pop, &ctx).unwrap();

    let start = effect.sample(0.0, VIEWPORT).unwrap();
    assert_eq!(start.outgoing.transform, aimer_canvas::Mat3::identity());
    let end = effect.sample(1.0, VIEWPORT).unwrap();
    near(end.outgoing.transform.transform_point(0.0, 0.0), (0.0, 260.0));
    let window = end.outgoing.reveal.unwrap();
    assert_eq!((window.width, window.height), (400.0, 160.0));
    assert!(effect.is_outgoing_on_top());
}

#[test]
fn a_revealed_push_keeps_the_copy_of_the_item_in_its_slot() {
    let ctx = context();
    let _item = repeatable_item("album-7", Rect::new(0.0, 260.0, 400.0, 160.0), &ctx);
    let effect = resolve(&revealing(), 0, 7, NavigationAction::Push, &ctx).unwrap();

    for progress in [0.0, 0.1, 0.2] {
        let frame = effect.sample_anchored_at(progress, VIEWPORT, (0.0, 0.0)).unwrap();
        near(frame.anchor.transform.transform_point(0.0, 0.0), (0.0, 260.0));
        near(frame.anchor.transform.transform_point(1.0, 1.0), (1.0, 261.0));
    }
    let end = effect.sample_anchored_at(1.0, VIEWPORT, (0.0, 0.0)).unwrap();
    assert_eq!(end.anchor.opacity, 0.0);
    assert_eq!(end.frame.incoming.opacity, 1.0);
}

#[test]
fn the_page_behind_is_only_blurred_when_asked() {
    let ctx = context();
    let _item = item("album-7", Rect::new(0.0, 260.0, 400.0, 160.0), &ctx);

    let plain = resolve(&revealing(), 0, 7, NavigationAction::Push, &ctx).unwrap();
    let blurred = resolve(&revealing().blur(12.0), 0, 7, NavigationAction::Push, &ctx).unwrap();
    let pop = resolve(&revealing().blur(12.0), 7, 0, NavigationAction::Pop, &ctx).unwrap();

    assert!(!plain.has_veil());
    assert!(blurred.has_veil() && pop.has_veil());
    // A blur of nothing is no blur.
    assert!(!resolve(&revealing().blur(0.0), 0, 7, NavigationAction::Push, &ctx).unwrap().has_veil());
    assert!(!resolve(&revealing().blur(-4.0), 0, 7, NavigationAction::Push, &ctx).unwrap().has_veil());
}

#[test]
fn a_zoom_with_nothing_to_grow_from_blurs_the_page_behind_too() {
    let ctx = context();

    // No item is registered under the tag, so the effect is a plain zoom.
    let zoom = resolve(&revealing().blur(12.0), 0, 8, NavigationAction::Push, &ctx).unwrap();

    assert!(zoom.has_veil());
}

/// A row as wide as the host, opened in three stages.
fn staged() -> ContainerTransition<Page> {
    transition().staged(true).curve(curves::LINEAR)
}

#[test]
fn a_staged_push_scales_the_item_then_turns_it_into_the_page_then_grows_the_page() {
    let ctx = context();
    let _item = repeatable_item("album-7", Rect::new(0.0, 260.0, 400.0, 160.0), &ctx);
    let effect = resolve(&staged(), 0, 7, NavigationAction::Push, &ctx).unwrap();
    let at = |p: f32| effect.sample_anchored_at(p, VIEWPORT, (0.0, 0.0)).unwrap();

    // First third: the copy of the item grows, and the page is not there.
    let lifted = at(1.0 / 3.0);
    assert!(lifted.anchor.transform.transform_point(400.0, 0.0).0 > 400.0);
    assert_eq!((lifted.anchor.opacity, lifted.frame.incoming.opacity), (1.0, 0.0));
    // Middle third: the copy fades out as the page fades in, both on the item.
    let turning = at(0.5);
    assert!((turning.anchor.opacity - 0.5).abs() < 1e-4 && turning.frame.incoming.opacity > 0.99);
    let window = turning.frame.incoming.reveal.unwrap();
    assert_eq!((window.width, window.height), (400.0, 160.0));
    // Last third: the window grows to the host.
    let growing = at(5.0 / 6.0);
    assert!(growing.frame.incoming.reveal.unwrap().height > 400.0);
    let end = at(1.0);
    assert_eq!(end.frame.incoming.transform, aimer_canvas::Mat3::identity());
    assert_eq!(end.frame.incoming.reveal, None);
}

#[test]
fn a_staged_pop_plays_the_stages_in_the_opposite_order() {
    let ctx = context();
    let _item = repeatable_item("album-7", Rect::new(0.0, 260.0, 400.0, 160.0), &ctx);
    let effect = resolve(&staged(), 7, 0, NavigationAction::Pop, &ctx).unwrap();
    let at = |p: f32| effect.sample_anchored_at(p, VIEWPORT, (0.0, 0.0)).unwrap();

    // A pop starts with the page filling the host, and its first third shrinks it.
    assert_eq!(at(0.0).frame.outgoing.reveal, None);
    assert!(at(1.0 / 6.0).frame.outgoing.reveal.unwrap().height > 160.0);
    // Then the page turns back into the item, and the item scales back down.
    let turning = at(0.5);
    assert!((turning.anchor.opacity - 0.5).abs() < 1e-4);
    let end = at(1.0);
    assert_eq!(end.anchor.transform.transform_point(0.0, 0.0), (0.0, 260.0));
    assert_eq!(end.anchor.opacity, 1.0);
    assert!(effect.is_outgoing_on_top());
}

#[test]
fn a_staged_blur_is_a_layer_over_everything_and_a_plain_one_is_not() {
    let ctx = context();
    let _item = repeatable_item("album-7", Rect::new(0.0, 260.0, 400.0, 160.0), &ctx);

    let blurred = resolve(&staged().blur(12.0), 0, 7, NavigationAction::Push, &ctx).unwrap();
    let plain = resolve(&staged(), 0, 7, NavigationAction::Push, &ctx).unwrap();

    assert!(blurred.has_veil() && !plain.has_veil());
}

#[test]
fn overlapping_stages_start_the_window_before_the_item_has_finished_turning() {
    let ctx = context();
    let _item = repeatable_item("album-7", Rect::new(0.0, 260.0, 400.0, 160.0), &ctx);
    let overlapped = staged().stage_overlap(0.5);
    let push = resolve(&overlapped, 0, 7, NavigationAction::Push, &ctx).unwrap();
    let pop = resolve(&overlapped, 7, 0, NavigationAction::Pop, &ctx).unwrap();

    // Half overlap: the copy is still fading at 0.625, and the window is
    // already past the item's size. Without overlap it would be neither.
    let mid = push.sample_anchored_at(0.625, VIEWPORT, (0.0, 0.0)).unwrap();
    assert!(mid.anchor.opacity > 0.0 && mid.anchor.opacity < 1.0);
    assert!(mid.frame.incoming.reveal.unwrap().height > 160.0);
    let apart = resolve(&staged(), 0, 7, NavigationAction::Push, &ctx).unwrap();
    let window = apart.sample_anchored_at(0.625, VIEWPORT, (0.0, 0.0)).unwrap().frame.incoming.reveal.unwrap();
    assert_eq!(window.height, 160.0);
    // A pop plays the same motion backwards: the window is still shrinking
    // when the copy starts to come back.
    let back = pop.sample_anchored_at(0.375, VIEWPORT, (0.0, 0.0)).unwrap();
    assert!(back.anchor.opacity > 0.0 && back.anchor.opacity < 1.0);
    assert!(back.frame.outgoing.reveal.unwrap().height > 160.0);
    // It still ends where a plain staged motion does.
    let end = push.sample_anchored_at(1.0, VIEWPORT, (0.0, 0.0)).unwrap();
    assert_eq!((end.frame.incoming.reveal, end.anchor.opacity), (None, 0.0));
}

#[test]
fn overlapping_stages_keep_the_blur() {
    let ctx = context();
    let _item = repeatable_item("album-7", Rect::new(0.0, 260.0, 400.0, 160.0), &ctx);

    let blurred = resolve(&staged().stage_overlap(0.5).blur(12.0), 0, 7, NavigationAction::Push, &ctx).unwrap();

    assert!(blurred.has_veil());
}

/// A long lift, with the turn at the end of it, then the window.
fn long_lift() -> aimer_animation::StageSpans {
    aimer_animation::StageSpans::new(0.0..0.55, 0.4..0.55, 0.55..1.0)
}

#[test]
fn stage_spans_place_a_push_and_a_pop_on_the_timeline() {
    let ctx = context();
    let _item = repeatable_item("album-7", Rect::new(0.0, 260.0, 400.0, 160.0), &ctx);
    let placed = staged().stage_spans(long_lift());
    let push = resolve(&placed, 0, 7, NavigationAction::Push, &ctx).unwrap();
    let pop = resolve(&placed, 7, 0, NavigationAction::Pop, &ctx).unwrap();
    let pushed = |p: f32| push.sample_anchored_at(p, VIEWPORT, (0.0, 0.0)).unwrap();
    let popped = |p: f32| pop.sample_anchored_at(p, VIEWPORT, (0.0, 0.0)).unwrap();

    // Still lifting, and no page, well before the turn at 0.4..0.55.
    assert_eq!((pushed(0.3).anchor.opacity, pushed(0.3).frame.incoming.opacity), (1.0, 0.0));
    assert!(pushed(0.3).anchor.transform.transform_point(400.0, 0.0).0 > 400.0);
    // Halfway through the turn the copy is half gone; the window has not moved.
    assert!((pushed(0.475).anchor.opacity - 0.5).abs() < 1e-4);
    assert_eq!(pushed(0.475).frame.incoming.reveal.unwrap().height, 160.0);
    // The window then takes the rest.
    assert!(pushed(0.7).frame.incoming.reveal.unwrap().height > 160.0);
    assert_eq!(pushed(1.0).frame.incoming.reveal, None);
    // A pop is the same motion backwards: the turn is at 0.45..0.6 of it.
    assert!((popped(0.525).anchor.opacity - 0.5).abs() < 1e-4);
    assert!(popped(0.3).frame.outgoing.reveal.unwrap().height > 160.0);
    assert_eq!(popped(1.0).anchor.opacity, 1.0);
}

#[test]
fn stage_spans_keep_the_blur() {
    let ctx = context();
    let _item = repeatable_item("album-7", Rect::new(0.0, 260.0, 400.0, 160.0), &ctx);

    let blurred = resolve(&staged().stage_spans(long_lift()).blur(12.0), 0, 7, NavigationAction::Push, &ctx).unwrap();

    assert!(blurred.has_veil());
}

#[test]
fn the_page_behind_dims_only_over_the_dim_span_of_the_stage_spans() {
    let ctx = context();
    let _item = repeatable_item("album-7", Rect::new(0.0, 260.0, 400.0, 160.0), &ctx);
    let placed = staged().scrim(0.5).stage_spans(long_lift().dimmed_over(0.7..1.0));
    let push = resolve(&placed, 0, 7, NavigationAction::Push, &ctx).unwrap();
    let pop = resolve(&placed, 7, 0, NavigationAction::Pop, &ctx).unwrap();

    // A push leaves the page behind alone until the page starts to grow, and
    // has it dimmed by the scrim when the page fills the host.
    let behind = |p: f32| push.sample_anchored_at(p, VIEWPORT, (0.0, 0.0)).unwrap().frame.outgoing.opacity;
    assert_eq!((behind(0.0), behind(0.5), behind(0.7)), (1.0, 1.0, 1.0));
    assert!(behind(0.85) < 1.0);
    assert!((behind(1.0) - 0.5).abs() < 1e-4);
    // A pop plays it backwards: the page returning to view lights up first.
    let returning = |p: f32| pop.sample_anchored_at(p, VIEWPORT, (0.0, 0.0)).unwrap().frame.incoming.opacity;
    assert!((returning(0.0) - 0.5).abs() < 1e-4);
    assert!(returning(0.15) > returning(0.0));
    assert_eq!(returning(0.5), 1.0);
}

/// A row as wide as the host, opened by its window growing while it dissolves.
fn expanding() -> ExpandTransition<Page> {
    ExpandTransition::new().tag(|page: &Page| (page.0 != 0).then(|| Key::from(format!("album-{}", page.0))))
}

fn row() -> Rect {
    Rect::new(0.0, 260.0, 400.0, 160.0)
}

#[test]
fn an_expanding_push_grows_the_window_while_the_item_dissolves_into_the_page() {
    let ctx = context();
    let _item = repeatable_item("album-7", row(), &ctx);
    let effect = resolve_expand(&expanding(), 0, 7, NavigationAction::Push, &ctx).unwrap();
    let at = |p: f32| effect.sample_anchored_at(p, VIEWPORT, (0.0, 0.0)).unwrap();

    // The window is the item to begin with, with the item's corners.
    let start = at(0.0);
    let window = start.frame.incoming.reveal.unwrap();
    assert_eq!((window.width, window.height), (400.0, 160.0));
    near(corner(&start.frame, 0.0, 0.0), (0.0, 260.0));
    assert_eq!((start.frame.incoming.corner_radius, start.anchor.opacity), (12.0, 1.0));
    // It is opening before the item has started to go.
    assert!(at(0.1).frame.incoming.reveal.unwrap().height > 160.0);
    assert_eq!(at(0.1).anchor.opacity, 1.0);
    // Half way through the dissolve the item is half gone, the window still opening.
    let middle = at(0.45);
    assert!((middle.anchor.opacity - 0.5).abs() < 1e-4);
    assert!(middle.frame.incoming.reveal.unwrap().height < 800.0);
    // The page is opaque all the way, so the page behind never shows through.
    for p in [0.0, 0.1, 0.3, 0.6, 0.9] {
        assert_eq!(at(p).frame.incoming.opacity, 1.0, "{p}");
    }
    // The item itself is what grows: its copy is stretched to the window, so its
    // far corner is the window's.
    let middle_window = middle.frame.incoming.reveal.unwrap();
    let origin = corner(&middle.frame, 0.0, 0.0);
    near(middle.anchor.transform.transform_point(0.0, 0.0), origin);
    near(middle.anchor.transform.transform_point(400.0, 160.0),
        (origin.0 + middle_window.width, origin.1 + middle_window.height));
    // It ends as the whole host.
    let end = at(1.0);
    assert_eq!((end.frame.incoming.reveal, end.anchor.opacity), (None, 0.0));
    assert_eq!(end.frame.incoming.transform, aimer_canvas::Mat3::identity());
    assert_eq!(end.frame.incoming.corner_radius, 0.0);
    assert!(!effect.is_outgoing_on_top());
}

#[test]
fn a_stretched_expansion_scales_the_page_into_the_window_in_both_directions() {
    let ctx = context();
    let _item = repeatable_item("album-7", row(), &ctx);
    let push = resolve_expand(&expanding().stretch(true), 0, 7, NavigationAction::Push, &ctx).unwrap();
    let pop = resolve_expand(&expanding().stretch(true), 7, 0, NavigationAction::Pop, &ctx).unwrap();
    let far = (VIEWPORT.width, VIEWPORT.height);

    // Opening: the whole page is on the item, at the item's size, not clipped to it.
    let start = push.sample_anchored_at(0.0, VIEWPORT, (0.0, 0.0)).unwrap().frame.incoming;
    assert_eq!(start.reveal, None);
    near(start.transform.transform_point(0.0, 0.0), (0.0, 260.0));
    near(start.transform.transform_point(far.0, far.1), (400.0, 420.0));
    assert_eq!(start.corner_radius, 12.0);
    // It fills the host at the end, unscaled.
    let end = push.sample_anchored_at(1.0, VIEWPORT, (0.0, 0.0)).unwrap().frame.incoming;
    assert_eq!((end.reveal, end.transform), (None, aimer_canvas::Mat3::identity()));

    // Closing plays it backwards on the departing page: it shrinks onto the item.
    let begin = pop.sample_anchored_at(0.0, VIEWPORT, (0.0, 0.0)).unwrap().frame.outgoing;
    assert_eq!((begin.reveal, begin.transform), (None, aimer_canvas::Mat3::identity()));
    let last = pop.sample_anchored_at(1.0, VIEWPORT, (0.0, 0.0)).unwrap().frame.outgoing;
    near(last.transform.transform_point(0.0, 0.0), (0.0, 260.0));
    near(last.transform.transform_point(far.0, far.1), (400.0, 420.0));

    // Without it the page is revealed, as before.
    let revealed = resolve_expand(&expanding(), 0, 7, NavigationAction::Push, &ctx).unwrap();
    assert!(revealed.sample_anchored_at(0.0, VIEWPORT, (0.0, 0.0)).unwrap().frame.incoming.reveal.is_some());
}

#[test]
fn an_expanding_crossfade_has_the_page_rise_as_the_item_falls() {
    let ctx = context();
    let _item = repeatable_item("album-7", row(), &ctx);
    let push = resolve_expand(&expanding().crossfade(true).dissolve_curve(curves::EASE_IN_OUT), 0, 7, NavigationAction::Push, &ctx).unwrap();
    let pop = resolve_expand(&expanding().crossfade(true), 7, 0, NavigationAction::Pop, &ctx).unwrap();
    let pushed = |p: f32| push.sample_anchored_at(p, VIEWPORT, (0.0, 0.0)).unwrap();

    // The page is not there when the window starts to open, and is whole once the
    // item has gone.
    assert_eq!(pushed(0.1).frame.incoming.opacity, 0.0);
    assert_eq!(pushed(0.9).frame.incoming.opacity, 1.0);
    for step in 0..=20 {
        let frame = pushed(step as f32 / 20.0);
        assert!((frame.frame.incoming.opacity + frame.anchor.opacity - 1.0).abs() < 1e-4, "{step}");
    }
    // A pop plays it backwards: the page being left (outgoing) falls as the item
    // comes back (at 0.55 of the way, the mirror of 0.45).
    let back = pop.sample_anchored_at(0.55, VIEWPORT, (0.0, 0.0)).unwrap();
    assert!((back.frame.outgoing.opacity - 0.5).abs() < 1e-4 && (back.anchor.opacity - 0.5).abs() < 1e-4);
    // Without it the page stays opaque, as it did.
    let plain = resolve_expand(&expanding(), 0, 7, NavigationAction::Push, &ctx).unwrap();
    assert_eq!(plain.sample_anchored_at(0.1, VIEWPORT, (0.0, 0.0)).unwrap().frame.incoming.opacity, 1.0);
}

#[test]
fn an_expanding_pop_plays_it_backwards_into_the_item() {
    let ctx = context();
    let _item = repeatable_item("album-7", row(), &ctx);
    let effect = resolve_expand(&expanding(), 7, 0, NavigationAction::Pop, &ctx).unwrap();
    let at = |p: f32| effect.sample_anchored_at(p, VIEWPORT, (0.0, 0.0)).unwrap();

    // It begins with the page filling the host and the item gone...
    let start = at(0.0);
    assert_eq!((start.frame.outgoing.reveal, start.anchor.opacity), (None, 0.0));
    // ...brings the item back at the mirror of where it went (0.45 of the way
    // there is 0.55 of the way back)...
    assert!((at(0.55).anchor.opacity - 0.5).abs() < 1e-4);
    // ...and ends with the window back on the item, painted above the page that
    // returns, which is fully lit again.
    let end = at(1.0);
    let window = end.frame.outgoing.reveal.unwrap();
    assert_eq!((window.width, window.height, end.anchor.opacity), (400.0, 160.0, 1.0));
    near(corner(&NavigationTransitionFrame { incoming: end.frame.outgoing, outgoing: end.frame.incoming }, 0.0, 0.0), (0.0, 260.0));
    assert_eq!(end.frame.incoming.opacity, 1.0);
    assert!(effect.is_outgoing_on_top());
}

#[test]
fn the_page_behind_darkens_as_the_expanding_window_covers_it() {
    let ctx = context();
    let _item = repeatable_item("album-7", row(), &ctx);
    let effect = resolve_expand(&expanding().scrim(0.5), 0, 7, NavigationAction::Push, &ctx).unwrap();
    let behind = |p: f32| effect.sample_anchored_at(p, VIEWPORT, (0.0, 0.0)).unwrap().frame.outgoing.opacity;

    assert_eq!(behind(0.0), 1.0);
    assert!(behind(0.5) < 1.0 && behind(0.5) > behind(1.0));
    assert!((behind(1.0) - 0.5).abs() < 1e-4);
}

#[test]
fn expanding_spans_curves_and_corners_are_configurable() {
    let ctx = context();
    let _item = repeatable_item("album-7", row(), &ctx);
    let configured = expanding().grow(0.4..0.8).dissolve(0.0..0.4).source_radius(30.0).page_radius(4.0);
    let effect = resolve_expand(&configured, 0, 7, NavigationAction::Push, &ctx).unwrap();
    let at = |p: f32| effect.sample_anchored_at(p, VIEWPORT, (0.0, 0.0)).unwrap();

    assert_eq!(at(0.0).frame.incoming.corner_radius, 30.0);
    // Nothing grows before 0.4, though the item is half gone at 0.2.
    assert_eq!(at(0.3).frame.incoming.reveal.unwrap().height, 160.0);
    assert!((at(0.2).anchor.opacity - 0.5).abs() < 1e-4);
    // It is the whole host from 0.8, the corners the page's.
    assert_eq!(at(0.8).frame.incoming.reveal, None);
    assert_eq!(at(1.0).frame.incoming.corner_radius, 4.0);
    // A curve eases only the window.
    let eased = resolve_expand(&expanding().grow_curve(curves::EASE_OUT), 0, 7, NavigationAction::Push, &ctx).unwrap();
    let height = |p: f32| eased.sample_anchored_at(p, VIEWPORT, (0.0, 0.0)).unwrap().frame.incoming.reveal.unwrap().height;
    assert!(height(0.3) > effect_height(&resolve_expand(&expanding(), 0, 7, NavigationAction::Push, &ctx).unwrap(), 0.3));
}

fn effect_height(effect: &ResolvedNavigationTransition, p: f32) -> f32 {
    effect.sample_anchored_at(p, VIEWPORT, (0.0, 0.0)).unwrap().frame.incoming.reveal.unwrap().height
}

#[test]
fn an_expanding_item_without_a_copy_lets_the_page_fade_in_over_the_dissolve() {
    let ctx = context();
    // Not repeatable, so there is nothing to draw above the page.
    let _item = item("album-7", row(), &ctx);
    let effect = resolve_expand(&expanding(), 0, 7, NavigationAction::Push, &ctx).unwrap();
    let at = |p: f32| effect.sample_at(p, VIEWPORT, (0.0, 0.0)).unwrap();

    assert_eq!(at(0.1).incoming.opacity, 0.0);
    assert!((at(0.45).incoming.opacity - 0.5).abs() < 1e-4);
    assert_eq!(at(0.9).incoming.opacity, 1.0);
    assert!(at(0.1).incoming.reveal.is_some());
}

#[test]
fn an_expanding_route_with_nothing_to_grow_from_zooms() {
    let ctx = context();

    // No item is registered under the tag, and page 0 has no tag at all.
    for (from, to) in [(0, 8), (0, 0)] {
        let effect = resolve_expand(&expanding(), from, to, NavigationAction::Push, &ctx).unwrap();
        let frame = effect.sample_anchored_at(0.5, VIEWPORT, (0.0, 0.0)).unwrap();

        assert_eq!(frame.frame.incoming.reveal, None, "{from} -> {to}");
        assert_eq!(frame.anchor.opacity, 0.0, "{from} -> {to}");
    }
}

#[test]
fn an_expanding_blur_is_a_layer_over_everything_only_when_asked() {
    let ctx = context();
    let _item = repeatable_item("album-7", row(), &ctx);
    let has_veil = |transition: ExpandTransition<Page>, from, to, action| {
        resolve_expand(&transition, from, to, action, &ctx).unwrap().has_veil()
    };

    assert!(!has_veil(expanding(), 0, 7, NavigationAction::Push));
    assert!(has_veil(expanding().blur(12.0), 0, 7, NavigationAction::Push));
    assert!(has_veil(expanding().blur(12.0).blur_held(0.5), 7, 0, NavigationAction::Pop));
    assert!(!has_veil(expanding().blur(0.0), 0, 7, NavigationAction::Push));
    assert!(!has_veil(expanding().blur(-4.0), 0, 7, NavigationAction::Push));
    // A zoom with nothing to grow from blurs too.
    assert!(has_veil(expanding().blur(12.0), 0, 8, NavigationAction::Push));
}

#[test]
fn an_expanding_blur_covers_the_growing_window_not_the_whole_host() {
    use aimer_router::TransitionPage;
    let ctx = context();
    let _item = repeatable_item("album-7", row(), &ctx);
    let follows = |from, to, action| {
        resolve_expand(&expanding().blur(12.0), from, to, action, &ctx).unwrap().veil_page()
    };

    // The window is the arriving page when it opens and the departing one when it
    // closes back into the item.
    assert_eq!(follows(0, 7, NavigationAction::Push), Some(TransitionPage::Incoming));
    assert_eq!(follows(7, 0, NavigationAction::Pop), Some(TransitionPage::Outgoing));
    // With nothing to grow from there is no window: the zoom's blur fills the host.
    assert_eq!(follows(0, 8, NavigationAction::Push), None);
    // And with no blur there is no layer to place.
    let plain = resolve_expand(&expanding(), 0, 7, NavigationAction::Push, &ctx).unwrap();
    assert_eq!(plain.veil_page(), None);
}

fn resolve_expand(
    transition: &ExpandTransition<Page>,
    from: u32,
    to: u32,
    action: NavigationAction,
    ctx: &BuildContext,
) -> Option<ResolvedNavigationTransition> {
    transition.resolve(&NavigationContext { from: &Page(from), to: &Page(to), action, build_context: ctx })
}
