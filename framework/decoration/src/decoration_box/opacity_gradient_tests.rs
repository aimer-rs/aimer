use super::test_support::stacked;
use crate::{BorderRadius, BoxDecoration, Decoration, OpacityGradient, OpacityMaskBox};
use aimer_attribute::ResolvedSize;
use aimer_color::prelude::Color;
use aimer_container::{SizedBox, ZeroSizedBox};
use aimer_cupid::draw_cmd_v2::{DrawCommand, FADE_MASK_PIPELINE_NAME};
use aimer_cupid::material::{GradientShape, MaterialKind, MaterialRequest};
use aimer_widget::{Drawable, EventTreeRole, FocusNode, Focusable, Widget};

use super::tests::{context, record};

/// Straight RGBA8 of a black stop with `alpha`, packed as the gradient does.
fn black(alpha: u8) -> u32 {
    u32::from_le_bytes([0, 0, 0, alpha])
}

fn request(gradient: OpacityGradient) -> MaterialRequest {
    stacked(&[&gradient])
}

fn is_marker(command: &DrawCommand) -> bool {
    matches!(command, DrawCommand::DrawCustom { pipeline_name, data }
        if pipeline_name.as_ref() == FADE_MASK_PIPELINE_NAME && data.is_empty())
}

#[test]
fn an_opacity_gradient_records_a_ramp_of_black_alpha_stops() {
    let request = request(OpacityGradient::linear().stop(0.0, 1.0).stop(1.0, 0.0));
    assert_eq!(request.kind, MaterialKind::Gradient);
    assert_eq!(request.bounds, [3.0, 4.0, 80.0, 40.0]);
    assert_eq!(request.gradient.shape, GradientShape::Linear);
    assert_eq!(request.gradient.stop_count, 2);
    assert_eq!(request.gradient.offsets[..2], [0.0, 1.0]);
    assert_eq!(request.gradient.colors[..2], [black(255), black(0)]);
}

#[test]
fn opacities_round_to_the_nearest_alpha_and_clamp_to_the_unit_range() {
    let request = request(
        OpacityGradient::linear()
            .stop(0.0, 0.5)
            .stop(0.5, 2.0)
            .stop(1.0, -1.0),
    );
    assert_eq!(request.gradient.colors[..3], [black(128), black(255), black(0)]);
}

#[test]
fn nonfinite_stops_and_a_ninth_stop_are_ignored() {
    let ignored = OpacityGradient::linear()
        .stop(f32::NAN, 1.0)
        .stop(0.0, f32::NAN)
        .stop(0.0, f32::INFINITY);
    assert!(ignored.is_empty(), "no stop survived");

    let mut full = OpacityGradient::linear();
    for index in 0..9 {
        full = full.stop(index as f32 / 8.0, 1.0);
    }
    assert_eq!(request(full).gradient.stop_count, 8);
}

#[test]
fn opacities_spread_evenly_and_replace_the_stops() {
    let request = request(
        OpacityGradient::linear()
            .stop(0.3, 1.0)
            .opacities([0.0, 1.0, 0.0]),
    );
    assert_eq!(request.gradient.stop_count, 3);
    assert_eq!(request.gradient.offsets[..3], [0.0, 0.5, 1.0]);
    assert_eq!(request.gradient.colors[..3], [black(0), black(255), black(0)]);
}

#[test]
fn shapes_and_geometry_match_the_color_gradient() {
    use crate::Gradient;
    let faded = |gradient: OpacityGradient| gradient.opacities([1.0, 0.0]);
    let colored = |gradient: Gradient| gradient.colors([Color::RED, Color::BLUE]);
    let geometry = |request: MaterialRequest| (request.gradient.shape, request.gradient.geometry);

    assert_eq!(
        geometry(request(faded(OpacityGradient::linear().angle(45.0)))),
        geometry(stacked(&[&colored(Gradient::linear().angle(45.0))])),
    );
    assert_eq!(
        geometry(request(faded(OpacityGradient::radial().center(0.2, 0.8).radius(0.5)))),
        geometry(stacked(&[&colored(Gradient::radial().center(0.2, 0.8).radius(0.5))])),
    );
    assert_eq!(
        geometry(request(faded(OpacityGradient::conic().from_angle(90.0).center(0.0, 1.0)))),
        geometry(stacked(&[&colored(Gradient::conic().from_angle(90.0).center(0.0, 1.0))])),
    );
}

#[test]
fn an_opacity_gradient_follows_a_preceding_border_radius() {
    let radius = BorderRadius::from(8);
    let ramp = OpacityGradient::linear().opacities([1.0, 0.0]);
    let request = stacked(&[&radius, &ramp]);
    assert_eq!(request.corner_radii, [8.0; 4]);
}

#[test]
fn a_stopless_opacity_gradient_is_empty_and_paints_nothing() {
    assert!(OpacityGradient::linear().is_empty());
    assert!(!OpacityGradient::linear().stop(0.0, 1.0).is_empty());
}

#[tokio::test]
async fn an_opacity_mask_box_heads_its_list_with_the_fade_marker_then_the_ramp() {
    let ctx = context(1.0);
    let element = OpacityMaskBox::new()
        .decoration(OpacityGradient::linear().opacities([1.0, 0.0]))
        .child(SizedBox::new().width(60.0).height(20.0))
        .to_element(&ctx);
    let commands = record(&element, &ctx);
    assert!(
        matches!(commands.as_slice(), [marker, DrawCommand::DrawCustom { pipeline_name, .. }]
            if is_marker(marker) && pipeline_name.as_ref() == "aimer.material"),
        "marker, then one gradient packet"
    );
}

#[tokio::test]
async fn an_opacity_gradient_wraps_a_child_directly() {
    let ctx = context(1.0);
    let element = OpacityGradient::linear()
        .opacities([1.0, 0.0])
        .child(SizedBox::new().width(60.0).height(20.0))
        .to_element(&ctx);
    assert!(record(&element, &ctx).first().is_some_and(is_marker));
}

#[tokio::test]
async fn an_empty_decoration_leaves_the_child_unfaded() {
    let ctx = context(1.0);
    for element in [
        OpacityMaskBox::new()
            .child(SizedBox::new().width(24.0).height(12.0))
            .to_element(&ctx),
        OpacityMaskBox::new()
            .decoration(OpacityGradient::linear())
            .child(SizedBox::new().width(24.0).height(12.0))
            .to_element(&ctx),
        OpacityMaskBox::new()
            .decoration(BoxDecoration::new())
            .child(SizedBox::new().width(24.0).height(12.0))
            .to_element(&ctx),
    ] {
        assert!(
            record(&element, &ctx).is_empty(),
            "no marker and no mask when there is nothing to fade by"
        );
    }
}

#[tokio::test]
async fn a_plain_masked_box_still_records_the_coverage_marker_not_the_fade_one() {
    use crate::{Gradient, MaskedDecorationBox};
    let ctx = context(1.0);
    let element = MaskedDecorationBox::new()
        .decoration(Gradient::rainbow())
        .child(SizedBox::new().width(60.0).height(20.0))
        .to_element(&ctx);
    assert!(!record(&element, &ctx).iter().any(is_marker));
}

#[tokio::test]
async fn an_opacity_mask_box_keeps_its_childs_layout_and_events() {
    let ctx = context(1.0);
    let focus = FocusNode::new();
    let element = OpacityMaskBox::new()
        .decoration(OpacityGradient::linear().opacities([1.0, 0.0]))
        .child(Focusable::new().node(focus.clone()).child(ZeroSizedBox))
        .to_element(&ctx);
    assert_eq!(element.event_tree_role(), EventTreeRole::Transparent);
    assert!(element.focus_node().is_none());
    let mut structural = 0;
    element.structural_children(&mut |_| structural += 1);
    assert_eq!(structural, 1);

    let sized = OpacityMaskBox::new()
        .decoration(OpacityGradient::linear().opacities([1.0, 0.0]))
        .child(SizedBox::new().width(24.0).height(12.0))
        .to_element(&ctx);
    let expected = ResolvedSize { width: 24.0, height: 12.0 };
    assert_eq!(sized.computed_size(&ctx), expected);
    sized.visit_children(&mut |child| assert_eq!(child.computed_size(&ctx), expected));
}

#[tokio::test]
async fn an_opacity_mask_box_repaints_on_scale_changes_like_any_decoration_box() {
    let mut ctx = context(1.0);
    let element = OpacityMaskBox::new()
        .decoration(OpacityGradient::linear().opacities([1.0, 0.0]))
        .child(SizedBox::new().width(24.0).height(12.0))
        .to_element(&ctx);
    record(&element, &ctx);
    assert!(!element.local_v2_paint_needs_recording(&ctx));
    ctx.scale = 2.0;
    assert!(element.local_v2_paint_needs_recording(&ctx));
    assert!(record(&element, &ctx).first().is_some_and(is_marker));
}
