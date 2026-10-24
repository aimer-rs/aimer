//! Which presses an SVG takes for itself and which it leaves to what is around it.
//!
//! An icon inside a button or a tab bar is drawn over the thing that handles the
//! press, and children are offered an event before their parent. An SVG that took
//! every press landing on a drawn pixel would make the control behind it dead
//! exactly where its icon is.

use std::cell::Cell;
use std::rc::Rc;

use aimer_attribute::Vec2d;
use aimer_events::element::ElementEvent;
use aimer_events::pointer::{PointerButton, PointerInfo};
use aimer_widget::{CaptureRequest, Element, EventResult};

use crate::{StaticSvg, Svg, SvgHit, SvgStyle};

/// A 48px icon whose single path crosses the middle of the box.
static LINE: StaticSvg = StaticSvg::new(
    "pointer-test-line",
    r#"<svg viewBox="0 0 24 24" fill="none" stroke="currentColor"><path id="line" d="M2 12h20"/></svg>"#,
);

/// Builds `svg` as a laid-out element and hands it to `test`.
fn with_element(svg: Svg, test: impl FnOnce(&dyn Element)) {
    let runtime = tokio::runtime::Builder::new_current_thread().build().unwrap();
    let _guard = runtime.enter();
    let inner = aimer_canvas::InnerCanvas::new();
    let mut context = aimer_widget::base::BuildContext::new(
        aimer_canvas::FrameCanvas::new(&inner),
        aimer_attribute::ResolvedSize { width: 48.0, height: 48.0 },
        1.0,
        Default::default(),
        Default::default(),
        aimer_widget::base::WindowHandle::headless(Default::default(), 1.0),
        runtime.handle().clone(),
    );
    context.box_constraint = aimer_attribute::BoxConstraint {
        min_width: 0.0,
        min_height: 0.0,
        max_width: 48.0,
        max_height: 48.0,
    };
    use aimer_widget::Widget;
    let element = svg.width(48).height(48).to_element(&context);
    element.sync_paint_geometry(&context);
    test(element.as_ref());
}

/// A press on the path: the line runs through the middle of the 48px box.
fn on_the_path() -> PointerInfo {
    PointerInfo::mouse(Vec2d { x: 24.0, y: 24.0 }, PointerButton::Primary)
}

/// A press on the box, well clear of the path.
fn off_the_path() -> PointerInfo {
    PointerInfo::mouse(Vec2d { x: 24.0, y: 4.0 }, PointerButton::Primary)
}

fn press(element: &dyn Element, pointer: PointerInfo) -> (EventResult, EventResult) {
    let down = element.on_event(&ElementEvent::PointerDown(pointer));
    let up = element.on_event(&ElementEvent::PointerUp(pointer));
    (down, up)
}

#[test]
fn an_svg_nobody_listens_on_lets_a_press_on_its_pixels_through() {
    with_element(Svg::new(&LINE), |element| {
        let (down, up) = press(element, on_the_path());

        assert!(!down.is_consumed(), "the control behind the icon must see the press");
        assert_eq!(down.capture_request(), CaptureRequest::None);
        assert!(!up.is_consumed());
    });
}

#[test]
fn a_press_callback_on_the_hit_path_takes_the_press_and_runs_on_release() {
    let presses = Rc::new(Cell::new(0));
    let observed = presses.clone();
    let svg = Svg::new(&LINE).on_path_press("path", move |_: SvgHit| observed.set(observed.get() + 1));

    with_element(svg, |element| {
        let down = element.on_event(&ElementEvent::PointerDown(on_the_path()));
        assert!(down.is_consumed());
        assert!(matches!(down.capture_request(), CaptureRequest::Capture(_)));
        assert_eq!(presses.get(), 0, "a press completes on release");

        let up = element.on_event(&ElementEvent::PointerUp(on_the_path()));
        assert!(up.is_consumed());
        assert_eq!(presses.get(), 1);
    });
}

#[test]
fn a_press_callback_for_another_selector_leaves_the_press_alone() {
    let presses = Rc::new(Cell::new(0));
    let observed = presses.clone();
    let svg = Svg::new(&LINE).on_path_press("#somewhere-else", move |_: SvgHit| {
        observed.set(observed.get() + 1)
    });

    with_element(svg, |element| {
        let (down, up) = press(element, on_the_path());

        assert!(!down.is_consumed());
        assert!(!up.is_consumed());
        assert_eq!(presses.get(), 0);
    });
}

#[test]
fn a_press_beside_the_path_is_never_taken() {
    let svg = Svg::new(&LINE).on_path_press("path", |_: SvgHit| {});

    with_element(svg, |element| {
        let (down, up) = press(element, off_the_path());

        assert!(!down.is_consumed());
        assert!(!up.is_consumed());
    });
}

#[test]
fn a_pressed_style_on_the_hit_path_takes_the_press() {
    let svg = Svg::new(&LINE)
        .try_pressed_style("path", SvgStyle::new().opacity(0.3))
        .unwrap();

    with_element(svg, |element| {
        let down = element.on_event(&ElementEvent::PointerDown(on_the_path()));

        assert!(down.is_consumed(), "the pressed look follows a press it owns");
        assert!(matches!(down.capture_request(), CaptureRequest::Capture(_)));
    });
}

#[test]
fn a_pressed_style_for_another_selector_leaves_the_press_alone() {
    let svg = Svg::new(&LINE)
        .try_pressed_style("#somewhere-else", SvgStyle::new().opacity(0.3))
        .unwrap();

    with_element(svg, |element| {
        let down = element.on_event(&ElementEvent::PointerDown(on_the_path()));

        assert!(!down.is_consumed());
        assert_eq!(down.capture_request(), CaptureRequest::None);
    });
}

#[test]
fn a_hover_style_alone_never_takes_a_press() {
    let svg = Svg::new(&LINE)
        .try_hover_style("path", SvgStyle::new().opacity(0.8))
        .unwrap();

    with_element(svg, |element| {
        let (down, up) = press(element, on_the_path());

        assert!(!down.is_consumed(), "hovering needs no press");
        assert!(!up.is_consumed());
    });
}
