use super::tests::{context, record};
use crate::{BoxDecoration, DecorationBox, Gradient, MaskedDecorationBox};
use aimer_attribute::ResolvedSize;
use aimer_color::prelude::Color;
use aimer_container::{SizedBox, ZeroSizedBox};
use aimer_cupid::draw_cmd_v2::{COVERAGE_MASK_PIPELINE_NAME, DrawCommand};
use aimer_widget::{Drawable, EventTreeRole, FocusNode, Focusable, Widget};

fn is_marker(command: &DrawCommand) -> bool {
    matches!(command, DrawCommand::DrawCustom { pipeline_name, data }
        if pipeline_name.as_ref() == COVERAGE_MASK_PIPELINE_NAME && data.is_empty())
}

#[tokio::test]
async fn a_masked_box_heads_its_list_with_the_marker_before_the_decoration() {
    let ctx = context(2.0);
    let element = MaskedDecorationBox::new()
        .decoration(BoxDecoration::new().background_color(Color::RED))
        .child(SizedBox::new().width(24.0).height(12.0).color(Color::BLUE))
        .to_element(&ctx);
    let commands = record(&element, &ctx);
    assert!(
        matches!(commands.as_slice(), [marker, DrawCommand::FillRect { .. }] if is_marker(marker)),
        "marker first, then the decoration: {}",
        commands.len()
    );
}

#[tokio::test]
async fn a_plain_decoration_box_records_no_marker() {
    let ctx = context(2.0);
    let element = DecorationBox::new()
        .decoration(BoxDecoration::new().background_color(Color::RED))
        .child(SizedBox::new().width(24.0).height(12.0))
        .to_element(&ctx);
    assert!(!record(&element, &ctx).iter().any(is_marker));
}

#[tokio::test]
async fn a_masked_gradient_records_the_marker_then_its_ramp() {
    let ctx = context(1.0);
    let element = MaskedDecorationBox::new()
        .decoration(Gradient::rainbow())
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
async fn a_masked_box_keeps_its_childs_layout_and_events() {
    let ctx = context(1.0);
    let focus = FocusNode::new();
    let element = MaskedDecorationBox::new()
        .decoration(BoxDecoration::new().background_color(Color::RED))
        .child(Focusable::new().node(focus.clone()).child(ZeroSizedBox))
        .to_element(&ctx);
    assert_eq!(element.event_tree_role(), EventTreeRole::Transparent);
    assert!(element.focus_node().is_none());
    let mut structural = 0;
    element.structural_children(&mut |_| structural += 1);
    assert_eq!(structural, 1);

    let sized = MaskedDecorationBox::new()
        .decoration(Gradient::rainbow())
        .child(SizedBox::new().width(24.0).height(12.0))
        .to_element(&ctx);
    let expected = ResolvedSize { width: 24.0, height: 12.0 };
    assert_eq!(sized.computed_size(&ctx), expected);
    sized.visit_children(&mut |child| assert_eq!(child.computed_size(&ctx), expected));
}

#[tokio::test]
async fn a_masked_box_repaints_on_scale_changes_like_any_decoration_box() {
    let mut ctx = context(1.0);
    let element = MaskedDecorationBox::new()
        .decoration(Gradient::rainbow())
        .child(SizedBox::new().width(24.0).height(12.0))
        .to_element(&ctx);
    record(&element, &ctx);
    assert!(!element.local_v2_paint_needs_recording(&ctx));
    ctx.scale = 2.0;
    assert!(element.local_v2_paint_needs_recording(&ctx));
    assert!(record(&element, &ctx).first().is_some_and(is_marker));
}
