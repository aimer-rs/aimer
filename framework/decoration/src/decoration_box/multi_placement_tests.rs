use super::tests::context;
use crate::{Gradient, LightEdge, MultiDecorationBox, OpacityGradient};
use aimer_container::SizedBox;
use aimer_cupid::draw_cmd_v2::{
    COVERAGE_MASK_PIPELINE_NAME, DrawCommand, FADE_MASK_PIPELINE_NAME, Rect, RenderTree,
};
use aimer_cupid::material::{MaterialKind, MaterialRequest};
use aimer_widget::base::BuildContext;
use aimer_widget::{Element, Widget};

fn record_dyn(element: &dyn Element, ctx: &BuildContext) -> Vec<DrawCommand> {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 200.0, 160.0)).unwrap();
    ctx.with_local_v2_paint_context(tree.context(root).unwrap(), |ctx| {
        assert!(element.can_paint_local_v2(ctx));
        element.paint_local_v2(ctx);
    });
    tree.draw_list_snapshot(root).unwrap().commands.to_vec()
}

fn is_marker(command: &DrawCommand) -> bool {
    matches!(command, DrawCommand::DrawCustom { pipeline_name, .. }
        if pipeline_name.as_ref() == COVERAGE_MASK_PIPELINE_NAME)
}

/// The material kinds a list records, in order.
fn kinds(commands: &[DrawCommand]) -> Vec<MaterialKind> {
    commands
        .iter()
        .filter_map(|command| match command {
            DrawCommand::DrawCustom { data, .. } => {
                MaterialRequest::decode(data).ok().map(|request| request.kind)
            }
            _ => None,
        })
        .collect()
}

macro_rules! stack {
    ($placement:ident) => {
        stack!(@build MultiDecorationBox::new()
            .decoration(Gradient::rainbow())
            .decoration(LightEdge::new())
            .$placement())
    };
    () => {
        stack!(@build MultiDecorationBox::new()
            .decoration(Gradient::rainbow())
            .decoration(LightEdge::new()))
    };
    (@build $builder:expr) => {
        $builder.child(SizedBox::new().width(60.0).height(20.0))
    };
}

const LAYERS: [MaterialKind; 2] = [MaterialKind::Gradient, MaterialKind::LightEdge];

#[tokio::test]
async fn a_stack_paints_behind_its_child_by_default() {
    let ctx = context(1.0);
    let element = stack!().to_element(&ctx);
    let commands = record_dyn(element.as_ref(), &ctx);
    assert!(!commands.iter().any(is_marker));
    assert_eq!(kinds(&commands), LAYERS);
    let mut children = 0;
    element.visit_children(&mut |_| children += 1);
    assert_eq!(children, 1, "only the child follows the wrapper");
}

#[tokio::test]
async fn a_masked_stack_opens_the_list_with_one_marker_then_every_layer_in_order() {
    let ctx = context(1.0);
    let element = stack!(masked).to_element(&ctx);
    let commands = record_dyn(element.as_ref(), &ctx);
    assert!(commands.first().is_some_and(is_marker));
    assert_eq!(commands.iter().filter(|command| is_marker(command)).count(), 1);
    assert_eq!(kinds(&commands), LAYERS);
}

#[tokio::test]
async fn a_foreground_stack_paints_every_layer_in_the_node_after_the_child() {
    let ctx = context(1.0);
    let element = stack!(foreground).to_element(&ctx);
    assert!(
        record_dyn(element.as_ref(), &ctx).is_empty(),
        "the wrapper itself paints nothing"
    );
    let mut layer = None;
    element.visit_children(&mut |child| layer = Some(child));
    let layer = layer.expect("a layer follows the child");
    assert_eq!(layer.debug_name(), "ForegroundDecorationLayer");
    let layer_ctx = element
        .retained_v2_child_context_at(&ctx, layer, 1)
        .expect("the wrapper sizes its layer");
    let commands = record_dyn(layer, &layer_ctx);
    assert!(!commands.iter().any(is_marker));
    assert_eq!(kinds(&commands), LAYERS);
}

#[tokio::test]
async fn the_last_placement_wins() {
    let ctx = context(1.0);
    let foreground_last = MultiDecorationBox::new()
        .decoration(Gradient::rainbow())
        .masked()
        .foreground()
        .child(SizedBox::new().width(60.0).height(20.0))
        .to_element(&ctx);
    assert!(!record_dyn(foreground_last.as_ref(), &ctx).iter().any(is_marker));
    let mut children = 0;
    foreground_last.visit_children(&mut |_| children += 1);
    assert_eq!(children, 2, "foreground adds a layer node");

    let masked_last = MultiDecorationBox::new()
        .decoration(Gradient::rainbow())
        .foreground()
        .masked()
        .child(SizedBox::new().width(60.0).height(20.0))
        .to_element(&ctx);
    assert!(record_dyn(masked_last.as_ref(), &ctx).first().is_some_and(is_marker));
}

fn is_fade_marker(command: &DrawCommand) -> bool {
    matches!(command, DrawCommand::DrawCustom { pipeline_name, .. }
        if pipeline_name.as_ref() == FADE_MASK_PIPELINE_NAME)
}

#[tokio::test]
async fn a_faded_stack_opens_the_list_with_one_fade_marker_then_every_layer_in_order() {
    let ctx = context(1.0);
    let element = stack!(faded).to_element(&ctx);
    let commands = record_dyn(element.as_ref(), &ctx);
    assert!(commands.first().is_some_and(is_fade_marker));
    assert_eq!(commands.iter().filter(|command| is_fade_marker(command)).count(), 1);
    assert!(!commands.iter().any(is_marker), "not the coverage marker");
    assert_eq!(kinds(&commands), LAYERS);
    let mut children = 0;
    element.visit_children(&mut |_| children += 1);
    assert_eq!(children, 1, "the child is the only node below the wrapper");
}

#[tokio::test]
async fn a_faded_stack_takes_any_mix_of_layers() {
    let ctx = context(1.0);
    let element = MultiDecorationBox::new()
        .decoration(OpacityGradient::linear().opacities([1.0, 0.0]))
        .decoration(LightEdge::new())
        .faded()
        .child(SizedBox::new().width(60.0).height(20.0))
        .to_element(&ctx);
    let commands = record_dyn(element.as_ref(), &ctx);
    assert!(commands.first().is_some_and(is_fade_marker));
    assert_eq!(kinds(&commands), LAYERS, "an opacity ramp records a gradient material");
}

#[tokio::test]
async fn an_empty_faded_stack_leaves_the_child_unfaded() {
    let ctx = context(1.0);
    let element = MultiDecorationBox::new()
        .faded()
        .child(SizedBox::new().width(60.0).height(20.0))
        .to_element(&ctx);
    assert!(!record_dyn(element.as_ref(), &ctx).iter().any(is_fade_marker));
}

#[tokio::test]
async fn the_last_placement_wins_over_faded_too() {
    let ctx = context(1.0);
    let foreground_last = MultiDecorationBox::new()
        .decoration(Gradient::rainbow())
        .faded()
        .foreground()
        .child(SizedBox::new().width(60.0).height(20.0))
        .to_element(&ctx);
    assert!(!record_dyn(foreground_last.as_ref(), &ctx).iter().any(is_fade_marker));

    let faded_last = MultiDecorationBox::new()
        .decoration(Gradient::rainbow())
        .masked()
        .faded()
        .child(SizedBox::new().width(60.0).height(20.0))
        .to_element(&ctx);
    let commands = record_dyn(faded_last.as_ref(), &ctx);
    assert!(commands.first().is_some_and(is_fade_marker));
    assert!(!commands.iter().any(is_marker));

    let masked_last = MultiDecorationBox::new()
        .decoration(Gradient::rainbow())
        .faded()
        .masked()
        .child(SizedBox::new().width(60.0).height(20.0))
        .to_element(&ctx);
    let commands = record_dyn(masked_last.as_ref(), &ctx);
    assert!(commands.first().is_some_and(is_marker));
    assert!(!commands.iter().any(is_fade_marker));
}
