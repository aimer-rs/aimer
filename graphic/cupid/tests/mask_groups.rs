//! A node whose list opens with the coverage-mask marker is planned as a group
//! whose descendants provide the coverage its own commands show through.

use aimer_cupid::draw_cmd_v2::{
    COVERAGE_MASK_PIPELINE_NAME, DrawCommand, Rect, RenderNodeId, RenderOp, RenderTree,
};
use aimer_cupid::utilities::Color;
use std::sync::Arc;

fn marker() -> DrawCommand {
    DrawCommand::DrawCustom {
        pipeline_name: Arc::from(COVERAGE_MASK_PIPELINE_NAME),
        data: Arc::from(Vec::new()),
    }
}

fn fill(width: f32, height: f32) -> DrawCommand {
    DrawCommand::FillRect {
        rect: Rect::new(0.0, 0.0, width, height),
        color: Color::rgba8(255, 0, 0, 255),
        border_radius: [0.0; 4],
        border_width: [0.0; 4],
        border_color: Color::transparent(),
        outline_width: [0.0; 4],
        outline_color: Color::transparent(),
    }
}

fn record(tree: &RenderTree, node: RenderNodeId, commands: Vec<DrawCommand>) {
    tree.context(node)
        .unwrap()
        .begin_recording()
        .unwrap()
        .commit(commands)
        .unwrap();
}

/// A compact description of the plan: `[N` opens a group, `[N*` a masked one,
/// `dN` draws a node and `]N` closes a group.
fn plan(tree: &RenderTree) -> Vec<String> {
    tree.render_all()
        .into_iter()
        .map(|operation| match operation {
            RenderOp::BeginOpacityGroup {
                element, masked, ..
            } => format!("[{}{}", element.get(), if masked { "*" } else { "" }),
            RenderOp::Draw(item) => format!("d{}", item.element.get()),
            RenderOp::EndOpacityGroup { element } => format!("]{}", element.get()),
        })
        .collect()
}

/// root -> [masked -> [coverage]]
fn sample() -> (RenderTree, RenderNodeId, RenderNodeId, RenderNodeId) {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
    let masked = tree.add_child(root, Rect::new(10.0, 10.0, 60.0, 30.0)).unwrap();
    let coverage = tree.add_child(masked, Rect::new(0.0, 0.0, 60.0, 30.0)).unwrap();
    record(&tree, root, vec![fill(100.0, 100.0)]);
    record(&tree, coverage, vec![fill(60.0, 30.0)]);
    (tree, root, masked, coverage)
}

#[test]
fn a_marked_node_with_children_is_planned_as_a_masked_group() {
    let (tree, root, masked, coverage) = sample();
    record(&tree, masked, vec![marker(), fill(60.0, 30.0)]);
    let (root, masked, coverage) = (root.get(), masked.get(), coverage.get());
    assert_eq!(
        plan(&tree),
        [
            format!("d{root}"),
            format!("[{masked}*"),
            format!("d{masked}"),
            format!("d{coverage}"),
            format!("]{masked}"),
        ]
    );
}

#[test]
fn an_unmarked_node_plans_exactly_as_before() {
    let (tree, root, masked, coverage) = sample();
    record(&tree, masked, vec![fill(60.0, 30.0)]);
    assert_eq!(
        plan(&tree),
        [root, masked, coverage].map(|id| format!("d{}", id.get()))
    );
}

#[test]
fn the_group_follows_the_marker_when_a_list_is_recorded_again() {
    let (tree, _, masked, _) = sample();
    record(&tree, masked, vec![fill(60.0, 30.0)]);
    assert!(!plan(&tree).iter().any(|op| op.starts_with('[')));
    record(&tree, masked, vec![marker(), fill(60.0, 30.0)]);
    assert!(plan(&tree).contains(&format!("[{}*", masked.get())));
    record(&tree, masked, vec![fill(60.0, 30.0)]);
    assert!(!plan(&tree).iter().any(|op| op.starts_with('[')));
}

#[test]
fn a_marked_node_without_coverage_draws_nothing() {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
    let lonely = tree.add_child(root, Rect::new(10.0, 10.0, 60.0, 30.0)).unwrap();
    record(&tree, root, vec![fill(100.0, 100.0)]);
    record(&tree, lonely, vec![marker(), fill(60.0, 30.0)]);
    assert_eq!(
        plan(&tree),
        [format!("d{}", root.get())],
        "masking by nothing shows nothing"
    );
}

#[test]
fn opacity_and_a_mask_share_one_group() {
    let (tree, _, masked, coverage) = sample();
    record(&tree, masked, vec![marker(), fill(60.0, 30.0)]);
    tree.set_opacity(masked, 0.5).unwrap();
    let operations = tree.render_all();
    let groups: Vec<_> = operations
        .iter()
        .filter_map(|operation| match operation {
            RenderOp::BeginOpacityGroup {
                element,
                opacity,
                masked,
                ..
            } => Some((element.get(), *opacity, *masked)),
            _ => None,
        })
        .collect();
    assert_eq!(groups, [(masked.get(), 0.5, true)]);
    let draws: Vec<_> = operations
        .iter()
        .filter_map(|operation| match operation {
            RenderOp::Draw(item) => Some((item.element.get(), item.opacity)),
            _ => None,
        })
        .collect();
    assert!(
        draws.contains(&(masked.get(), 1.0)) && draws.contains(&(coverage.get(), 1.0)),
        "the group carries the opacity, so its items draw at full strength: {draws:?}"
    );
}
