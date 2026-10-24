use std::time::Duration;

use aimer_router::{CrossFadeTransition, NavigationAction, NavigationContext, NavigationTransition, Route};
use aimer_widget::base::{BuildContext, ResolvedSize, WindowHandle};

#[derive(Clone)]
struct Page(u32);

impl Route for Page {
    fn parse(_: &str) -> Option<Self> { None }
    fn format(&self) -> String { format!("/{}", self.0) }
    fn view_level(&self) -> u32 { self.0 }
}

fn context() -> BuildContext<'static> {
    use std::sync::OnceLock;
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    let runtime = RUNTIME.get_or_init(|| tokio::runtime::Builder::new_current_thread()
        .enable_all().build().unwrap());
    let canvas = aimer_canvas::FrameCanvas::new(Box::leak(Box::new(aimer_canvas::InnerCanvas::new())));
    BuildContext::new(canvas, ResolvedSize { width: 200.0, height: 100.0 }, 1.0,
        Default::default(), Default::default(), WindowHandle::headless(Default::default(), 1.0),
        runtime.handle().clone())
}

#[test]
fn cross_fade_shows_both_pages_halfway_and_finishes_at_destination() {
    let ctx = context();
    let navigation = NavigationContext {
        from: &Page(0), to: &Page(1), action: NavigationAction::Push, build_context: &ctx,
    };
    let effect = CrossFadeTransition::new().duration(Duration::from_millis(100))
        .resolve(&navigation).unwrap();
    let frame = effect.sample(0.5, ctx.parent_size).unwrap();
    assert_eq!(frame.incoming.opacity, 0.5);
    assert_eq!(frame.outgoing.opacity, 0.5);
    assert_eq!(frame.incoming.transform, aimer_canvas::Mat3::identity());
    let frame = effect.sample(1.0, ctx.parent_size).unwrap();
    assert_eq!(frame.incoming.opacity, 1.0);
    assert_eq!(frame.outgoing.opacity, 0.0);
}

#[test]
fn zoom_is_centered_and_pop_reverses_the_departing_effect() {
    use aimer_router::ZoomTransition;
    let ctx = context();
    let mut navigation = NavigationContext {
        from: &Page(0), to: &Page(1), action: NavigationAction::Push, build_context: &ctx,
    };
    let forward = ZoomTransition::new().resolve(&navigation).unwrap();
    let start = forward.sample(0.0, ctx.parent_size).unwrap();
    assert!((start.incoming.transform.cols[0][0] - 0.92).abs() < 0.00001);
    let center = start.incoming.transform.transform_point(100.0, 50.0);
    assert!((center.0 - 100.0).abs() < 0.00001);
    assert!((center.1 - 50.0).abs() < 0.00001);
    navigation.action = NavigationAction::Pop;
    let backward = ZoomTransition::new().resolve(&navigation).unwrap();
    assert_eq!(backward.sample(0.0, ctx.parent_size).unwrap().outgoing,
        forward.sample(1.0, ctx.parent_size).unwrap().incoming);
    assert_eq!(backward.sample(1.0, ctx.parent_size).unwrap().outgoing,
        forward.sample(0.0, ctx.parent_size).unwrap().incoming);
}

#[test]
fn view_levels_use_direction_and_current_viewport_width() {
    use aimer_router::ViewLevelTransitions;
    let ctx = context();
    for (from, to, offset) in [(0, 2, 200.0), (2, 0, -200.0), (2, 2, 0.0)] {
        let navigation = NavigationContext {
            from: &Page(from), to: &Page(to), action: NavigationAction::Pop, build_context: &ctx,
        };
        let effect = ViewLevelTransitions::new().resolve(&navigation).unwrap();
        let frame = effect.sample(0.0, ctx.parent_size).unwrap();
        assert_eq!(frame.incoming.transform.cols[2][0], offset);
        let resized = effect.sample(0.0, ResolvedSize { width: 400.0, height: 100.0 }).unwrap();
        assert_eq!(resized.incoming.transform.cols[2][0], offset * 2.0);
        if from == to { assert_eq!(frame.incoming.opacity, 0.0); }
    }
}

#[test]
fn dynamic_selector_can_inspect_routes_and_disable_navigation() {
    use aimer_router::DynamicTransitions;
    let ctx = context();
    let transition = DynamicTransitions::new(|navigation: &NavigationContext<'_, '_, Page>| {
        assert_eq!(navigation.from.0, 0);
        if navigation.to.0 == 2 { None } else { CrossFadeTransition::new().resolve(navigation) }
    });
    for (to, animated) in [(1, true), (2, false)] {
        let navigation = NavigationContext {
            from: &Page(0), to: &Page(to), action: NavigationAction::Push, build_context: &ctx,
        };
        assert_eq!(transition.resolve(&navigation).is_some(), animated);
    }
}

#[test]
fn custom_effects_reject_non_finite_values_and_clamp_opacity() {
    use aimer_router::{curves, NavigationTransitionFrame, PaintPresentation, ResolvedNavigationTransition};
    let effect = ResolvedNavigationTransition::new(Duration::ZERO, curves::LINEAR, |p, _| {
        NavigationTransitionFrame {
            incoming: PaintPresentation { transform: aimer_canvas::Mat3::identity(), opacity: p * 4.0, corner_radius: 0.0, reveal: None },
            outgoing: PaintPresentation { transform: aimer_canvas::Mat3::identity(), opacity: -1.0, corner_radius: 0.0, reveal: None },
        }
    }).unwrap();
    let size = ResolvedSize { width: 200.0, height: 100.0 };
    assert_eq!(effect.duration(), Duration::ZERO);
    let frame = effect.sample(0.5, size).unwrap();
    assert_eq!(frame.incoming.opacity, 1.0);
    assert_eq!(frame.outgoing.opacity, 0.0);
    assert!(effect.sample(f32::NAN, size).is_none());
    assert!(effect.sample(0.0, ResolvedSize { width: f32::INFINITY, height: 1.0 }).is_none());
    assert!(ResolvedNavigationTransition::new(Duration::ZERO,
        curves::cubic_bezier(f32::NAN, 0.0, 1.0, 1.0), move |_, _| frame).is_none());
    let invalid = ResolvedNavigationTransition::new(Duration::ZERO, curves::LINEAR, |_, _| {
        NavigationTransitionFrame {
            incoming: PaintPresentation { transform: aimer_canvas::Mat3::translate(f32::NAN, 0.0), opacity: 1.0, corner_radius: 0.0, reveal: None },
            outgoing: PaintPresentation::IDENTITY_OPAQUE,
        }
    }).unwrap();
    assert!(invalid.sample(0.0, size).is_none());
}

#[test]
fn custom_effects_carry_a_corner_radius_and_reject_invalid_ones() {
    use aimer_router::{curves, NavigationTransitionFrame, PaintPresentation, ResolvedNavigationTransition};
    let size = ResolvedSize { width: 200.0, height: 100.0 };
    let with_radius = |radius: f32| ResolvedNavigationTransition::new(Duration::ZERO, curves::LINEAR, move |_, _| {
        NavigationTransitionFrame {
            incoming: PaintPresentation { corner_radius: radius, ..PaintPresentation::IDENTITY_OPAQUE },
            outgoing: PaintPresentation::IDENTITY_OPAQUE,
        }
    }).unwrap();

    assert_eq!(with_radius(12.0).sample(0.5, size).unwrap().incoming.corner_radius, 12.0);
    for invalid in [-1.0, f32::NAN, f32::INFINITY] {
        assert!(with_radius(invalid).sample(0.5, size).is_none(), "{invalid}");
    }
}

#[test]
fn positioned_effects_receive_the_hosts_window_origin() {
    use aimer_router::{curves, NavigationTransitionFrame, PaintPresentation, ResolvedNavigationTransition};
    let size = ResolvedSize { width: 200.0, height: 100.0 };
    let effect = ResolvedNavigationTransition::positioned(Duration::from_millis(100), curves::LINEAR, |_, _, origin| {
        NavigationTransitionFrame {
            incoming: PaintPresentation {
                transform: aimer_canvas::Mat3::translate(origin.0, origin.1),
                ..PaintPresentation::IDENTITY_OPAQUE
            },
            outgoing: PaintPresentation::IDENTITY_OPAQUE,
        }
    }).unwrap();

    let at = effect.sample_at(0.5, size, (10.0, 20.0)).unwrap();
    assert_eq!(at.incoming.transform.transform_point(0.0, 0.0), (10.0, 20.0));
    // Plain sampling has no window position to offer.
    let plain = effect.sample(0.5, size).unwrap();
    assert_eq!(plain.incoming.transform.transform_point(0.0, 0.0), (0.0, 0.0));
    assert!(effect.sample_at(0.5, size, (f32::NAN, 0.0)).is_none());
}

#[test]
fn painting_the_departing_page_on_top_is_opt_in() {
    use aimer_router::{curves, NavigationTransitionFrame, ResolvedNavigationTransition};
    let frame = NavigationTransitionFrame {
        incoming: aimer_router::PaintPresentation::IDENTITY_OPAQUE,
        outgoing: aimer_router::PaintPresentation::IDENTITY_OPAQUE,
    };
    let effect = ResolvedNavigationTransition::new(Duration::from_millis(100), curves::LINEAR, move |_, _| frame).unwrap();

    assert!(!effect.is_outgoing_on_top());
    assert!(effect.outgoing_on_top(true).is_outgoing_on_top());
}

#[test]
fn slide_pushes_from_the_right_and_parallaxes_the_page_beneath() {
    use aimer_router::SlideTransition;
    let ctx = context();
    let navigation = NavigationContext {
        from: &Page(0), to: &Page(1), action: NavigationAction::Push, build_context: &ctx,
    };
    let effect = SlideTransition::new().resolve(&navigation).unwrap();
    assert!(!effect.is_outgoing_on_top(), "the arriving page covers the one it leaves");
    let x = |presentation: aimer_router::PaintPresentation| presentation.transform.cols[2][0];

    let start = effect.sample(0.0, ctx.parent_size).unwrap();
    assert_eq!(x(start.incoming), 200.0);
    assert_eq!(x(start.outgoing), 0.0);

    let end = effect.sample(1.0, ctx.parent_size).unwrap();
    assert_eq!(x(end.incoming), 0.0);
    assert!((x(end.outgoing) + 60.0).abs() < 0.001, "the page beneath moves 30% as far");
}

#[test]
fn slide_pop_carries_the_departing_page_off_to_the_right_above_the_page_returning() {
    use aimer_router::SlideTransition;
    let ctx = context();
    let navigation = NavigationContext {
        from: &Page(1), to: &Page(0), action: NavigationAction::Pop, build_context: &ctx,
    };
    let effect = SlideTransition::new().resolve(&navigation).unwrap();
    assert!(effect.is_outgoing_on_top(), "the page being dragged away stays in front");
    let x = |presentation: aimer_router::PaintPresentation| presentation.transform.cols[2][0];

    let start = effect.sample(0.0, ctx.parent_size).unwrap();
    assert_eq!(x(start.outgoing), 0.0);
    assert!((x(start.incoming) + 60.0).abs() < 0.001);

    let end = effect.sample(1.0, ctx.parent_size).unwrap();
    assert_eq!(x(end.outgoing), 200.0);
    assert_eq!(x(end.incoming), 0.0);

    let half = effect.sample(0.5, ctx.parent_size).unwrap();
    assert_eq!(x(half.outgoing), 100.0);
    assert!((x(half.incoming) + 30.0).abs() < 0.001);
}

#[test]
fn slide_is_a_350ms_effect_that_can_be_retimed_or_disabled() {
    use aimer_router::SlideTransition;
    let ctx = context();
    let navigation = NavigationContext {
        from: &Page(0), to: &Page(1), action: NavigationAction::Push, build_context: &ctx,
    };
    assert_eq!(SlideTransition::new().resolve(&navigation).unwrap().duration(), Duration::from_millis(350));
    let retimed = SlideTransition::new().duration(Duration::from_millis(120)).resolve(&navigation).unwrap();
    assert_eq!(retimed.duration(), Duration::from_millis(120));
}
