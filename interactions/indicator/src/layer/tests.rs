use std::time::Duration;

use aimer_widget::base::ResolvedSize;

use super::*;
use crate::style::{LayerPresentation, indicator_styles};
use crate::test_support::{Retained, SLOT, Slot, context};
use crate::track::tracks;

/// A style whose presentation a test chooses.
struct Fixed(LayerPresentation);

impl IndicatorStyle for Fixed {
    fn layer(&self, _layer: IndicatorLayer, _size: ResolvedSize) -> Option<aimer_widget::AnyWidget> {
        None
    }

    fn present(&self, _layer: IndicatorLayer, _lift: f32) -> LayerPresentation {
        self.0
    }
}

fn layer(
    controller: &IndicatorController,
    style: impl IndicatorStyle,
    layer: IndicatorLayer,
) -> AnyElement {
    IndicatorLayerWidget {
        controller: controller.clone(),
        track: tracks::row(3, SLOT, SLOT).gap(90.0).into_any(),
        style: style.into_any(),
        layer,
        child: Slot,
    }
    .to_element(&context())
}

/// A controller held down and given a second to finish lifting.
fn lifted(position: f32) -> IndicatorController {
    let controller = IndicatorController::new(position);
    controller.set_pressed(true);
    let start = AnimInstant::now();
    controller.tick(start);
    controller.tick(start + Duration::from_secs(1));
    assert_eq!(controller.lift(), 1.0);
    controller
}

fn assert_close(actual: (f32, f32), expected: (f32, f32)) {
    assert!(
        (actual.0 - expected.0).abs() < 1.0e-3 && (actual.1 - expected.1).abs() < 1.0e-3,
        "{actual:?} is not {expected:?}"
    );
}

#[test]
fn a_resting_layer_sits_on_the_slot_the_controller_is_on() {
    let context = context();
    let controller = IndicatorController::new(2.0);
    let element = layer(&controller, indicator_styles::LIQUID_GLASS, IndicatorLayer::Rest);
    let retained = Retained::new();

    assert!(retained.sync(element.as_ref(), &context));

    // The third slot starts 2 * (10 + 90) pixels along.
    let item = retained.drawn_child().expect("the pill is visible at rest");
    assert_eq!(item.opacity, 1.0);
    assert_close(item.transform.transform_point(0.0, 0.0), (200.0, 0.0));
    assert_close(item.transform.transform_point(SLOT, SLOT), (200.0 + SLOT, SLOT));
}

#[test]
fn a_lifted_lens_grows_about_its_own_center() {
    let context = context();
    let controller = lifted(1.0);
    let element = layer(&controller, indicator_styles::LIQUID_GLASS, IndicatorLayer::Lift);
    let retained = Retained::new();

    assert!(retained.sync(element.as_ref(), &context));

    let item = retained.drawn_child().expect("the lens is visible when lifted");
    assert_eq!(item.opacity, 1.0);
    // The center of the slot does not move...
    assert_close(item.transform.transform_point(SLOT / 2.0, SLOT / 2.0), (100.0 + SLOT / 2.0, SLOT / 2.0));
    // ...while the corners move outward by the extra size.
    let grown = SLOT * 0.18 / 2.0;
    assert_close(item.transform.transform_point(0.0, 0.0), (100.0 - grown, -grown));
}

#[test]
fn a_transparent_layer_is_not_drawn_at_all() {
    let context = context();
    let controller = IndicatorController::new(0.0);
    // At rest the lens is fully transparent. Its backdrop pass is the expensive
    // part of the style, so it must not reach the renderer.
    let element = layer(&controller, indicator_styles::LIQUID_GLASS, IndicatorLayer::Lift);
    let retained = Retained::new();

    assert!(retained.sync(element.as_ref(), &context));

    assert!(retained.drawn_child().is_none(), "a transparent layer was still drawn");
}

#[test]
fn a_layer_that_reappears_is_drawn_again_and_unclipped() {
    let context = context();
    let controller = IndicatorController::new(0.0);
    let element = layer(&controller, indicator_styles::LIQUID_GLASS, IndicatorLayer::Lift);
    let retained = Retained::new();
    assert!(retained.sync(element.as_ref(), &context));
    assert!(retained.drawn_child().is_none());

    // Pressing lifts the controller, which brings the lens back on the same
    // element that hid it.
    controller.set_pressed(true);
    let start = AnimInstant::now();
    controller.tick(start);
    controller.tick(start + Duration::from_secs(1));
    assert!(retained.sync(element.as_ref(), &context));
    let item = retained.drawn_child().expect("the lens returned with the lift");
    assert_eq!(item.clip, None, "the clip that hid the layer was left behind");

    // And letting go hides it again.
    controller.set_pressed(false);
    let start = AnimInstant::now();
    controller.tick(start);
    controller.tick(start + Duration::from_secs(1));
    assert!(retained.sync(element.as_ref(), &context));
    assert!(retained.drawn_child().is_none());
}

#[test]
fn a_non_finite_presentation_hides_the_layer_instead_of_breaking_the_frame() {
    let context = context();
    let controller = IndicatorController::new(0.0);
    for presentation in [
        LayerPresentation { scale: f32::NAN, opacity: 1.0 },
        LayerPresentation { scale: 0.0, opacity: 1.0 },
        LayerPresentation { scale: 1.0, opacity: f32::NAN },
    ] {
        let element = layer(&controller, Fixed(presentation), IndicatorLayer::Rest);
        let retained = Retained::new();
        assert!(retained.sync(element.as_ref(), &context), "{presentation:?}");
    }
}

#[test]
fn a_layer_takes_no_pointer_input_and_no_focus() {
    let controller = IndicatorController::new(0.0);
    let element = layer(&controller, indicator_styles::LIQUID_GLASS, IndicatorLayer::Rest);

    let mut event_children = 0;
    element.event_children(&mut |_| event_children += 1);
    let mut focus_children = 0;
    element.focus_children(&mut |_| focus_children += 1);
    let mut structural_children = 0;
    element.structural_children(&mut |_| structural_children += 1);

    assert_eq!(event_children, 0);
    assert_eq!(focus_children, 0);
    // Reconciliation still has to see the child.
    assert_eq!(structural_children, 1);
}

/// A slot that paints beyond its layout box, as a lens with a shadow does.
struct Shadowed;

impl Widget for Shadowed {
    fn to_element(self, _context: &BuildContext) -> AnyElement {
        ShadowedElement.boxed()
    }
}

impl PortableWidget for Shadowed {}

struct ShadowedElement;

impl Drawable for ShadowedElement {
    fn update(&self, _context: &BuildContext) {}

    fn retained_v2_paint_outsets(&self, _context: &BuildContext) -> Option<[f32; 4]> {
        Some([7.0, 5.0, 11.0, 13.0])
    }
}

impl EventElement for ShadowedElement {}

impl LayoutElement for ShadowedElement {
    fn computed_size(&self, _context: &BuildContext) -> ResolvedSize {
        ResolvedSize { width: SLOT, height: SLOT }
    }
}

impl Rebuildable for ShadowedElement {}

impl VisitorElement for ShadowedElement {
    fn debug_name(&self) -> &'static str {
        "ShadowedElement"
    }
}

#[test]
fn a_child_with_paint_outsets_is_moved_to_its_slot_without_a_shift() {
    // A child that paints a shadow has a render node that starts at its outsets,
    // but a move is the same distance wherever the node starts. Adding the
    // outsets to it put the layer that far from its slot.
    let context = context();
    let controller = IndicatorController::new(1.0);
    let element = IndicatorLayerWidget {
        controller: controller.clone(),
        track: tracks::row(3, SLOT, SLOT).gap(90.0).into_any(),
        style: indicator_styles::FLAT_PILL.into_any(),
        layer: IndicatorLayer::Rest,
        child: Shadowed,
    }
    .to_element(&context);
    let retained = Retained::new();

    assert!(retained.sync(element.as_ref(), &context));

    let item = retained.drawn_child().expect("the pill is visible at rest");
    assert_close(item.transform.transform_point(0.0, 0.0), (100.0, 0.0));
}

#[test]
fn growing_a_child_with_paint_outsets_keeps_its_layout_center_still() {
    // The render node starts 7 and 5 pixels out from the layout box, so the
    // layout center is at (7 + 5, 5 + 5) in the node's coordinates. Growing about
    // the node's own center would drift it by the outsets.
    let context = context();
    let controller = lifted(1.0);
    let element = IndicatorLayerWidget {
        controller: controller.clone(),
        track: tracks::row(3, SLOT, SLOT).gap(90.0).into_any(),
        style: indicator_styles::LIQUID_GLASS.into_any(),
        layer: IndicatorLayer::Lift,
        child: Shadowed,
    }
    .to_element(&context);
    let retained = Retained::new();

    assert!(retained.sync(element.as_ref(), &context));

    let item = retained.drawn_child().expect("the lens is visible when lifted");
    let (centre_x, centre_y) = (7.0 + SLOT / 2.0, 5.0 + SLOT / 2.0);
    assert_close(
        item.transform.transform_point(centre_x, centre_y),
        (100.0 + centre_x, centre_y),
    );
}

#[test]
fn a_hidden_indicator_draws_neither_layer() {
    let context = context();
    let controller = lifted(1.0);
    controller.set_hidden(true);
    let start = AnimInstant::now();
    controller.tick(start);
    controller.tick(start + Duration::from_secs(1));
    assert_eq!(controller.presence(), 0.0);

    for layer_kind in [IndicatorLayer::Rest, IndicatorLayer::Lift] {
        let element = layer(&controller, indicator_styles::LIQUID_GLASS, layer_kind);
        let retained = Retained::new();
        assert!(retained.sync(element.as_ref(), &context));
        assert!(retained.drawn_child().is_none(), "{layer_kind:?} was drawn while hidden");
    }
}

#[test]
fn a_half_visible_indicator_is_half_as_opaque() {
    let context = context();
    let controller = IndicatorController::new(1.0).fade_duration(Duration::from_millis(100));
    controller.set_hidden(true);
    let start = AnimInstant::now();
    controller.tick(start);
    controller.tick(start + Duration::from_millis(50));
    let element = layer(&controller, indicator_styles::FLAT_PILL, IndicatorLayer::Rest);
    let retained = Retained::new();
    assert!(retained.sync(element.as_ref(), &context));
    let item = retained.drawn_child().expect("half visible is still drawn");
    assert!((item.opacity - 0.5).abs() < 0.05, "opacity {}", item.opacity);
}
