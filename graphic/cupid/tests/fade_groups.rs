//! A node whose list opens with the fade-mask marker is planned as a group
//! whose own commands provide the alpha its descendants are shown through.

use aimer_cupid::draw_cmd_v2::{
    COVERAGE_MASK_PIPELINE_NAME, DrawCommand, FADE_MASK_PIPELINE_NAME, Rect, RenderNodeId,
    RenderOp, RenderTree,
};
use aimer_cupid::utilities::Color;
use std::sync::Arc;

fn marker() -> DrawCommand {
    DrawCommand::DrawCustom {
        pipeline_name: Arc::from(FADE_MASK_PIPELINE_NAME),
        data: Arc::from(Vec::new()),
    }
}

fn coverage_marker() -> DrawCommand {
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

/// A compact description of the plan: `[N` opens a group, `[N~` a fade group,
/// `[N*` a coverage-masked one, `dN` draws a node and `]N` closes a group.
fn plan(tree: &RenderTree) -> Vec<String> {
    tree.render_all()
        .into_iter()
        .map(|operation| match operation {
            RenderOp::BeginOpacityGroup {
                element,
                masked,
                fade,
                ..
            } => format!(
                "[{}{}{}",
                element.get(),
                if masked { "*" } else { "" },
                if fade { "~" } else { "" }
            ),
            RenderOp::Draw(item) => format!("d{}", item.element.get()),
            RenderOp::EndOpacityGroup { element } => format!("]{}", element.get()),
        })
        .collect()
}

/// root -> [faded -> [content]]
fn sample() -> (RenderTree, RenderNodeId, RenderNodeId, RenderNodeId) {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
    let faded = tree.add_child(root, Rect::new(10.0, 10.0, 60.0, 30.0)).unwrap();
    let content = tree.add_child(faded, Rect::new(0.0, 0.0, 60.0, 30.0)).unwrap();
    record(&tree, root, vec![fill(100.0, 100.0)]);
    record(&tree, content, vec![fill(60.0, 30.0)]);
    (tree, root, faded, content)
}

#[test]
fn a_marked_node_with_children_is_planned_as_a_fade_group() {
    let (tree, root, faded, content) = sample();
    record(&tree, faded, vec![marker(), fill(60.0, 30.0)]);
    let (root, faded, content) = (root.get(), faded.get(), content.get());
    assert_eq!(
        plan(&tree),
        [
            format!("d{root}"),
            format!("[{faded}~"),
            format!("d{faded}"),
            format!("d{content}"),
            format!("]{faded}"),
        ]
    );
}

#[test]
fn an_unmarked_node_plans_exactly_as_before() {
    let (tree, root, faded, content) = sample();
    record(&tree, faded, vec![fill(60.0, 30.0)]);
    assert_eq!(
        plan(&tree),
        [root, faded, content].map(|id| format!("d{}", id.get()))
    );
}

#[test]
fn the_group_follows_the_marker_when_a_list_is_recorded_again() {
    let (tree, _, faded, _) = sample();
    record(&tree, faded, vec![fill(60.0, 30.0)]);
    assert!(!plan(&tree).iter().any(|op| op.starts_with('[')));
    record(&tree, faded, vec![marker(), fill(60.0, 30.0)]);
    assert!(plan(&tree).contains(&format!("[{}~", faded.get())));
    record(&tree, faded, vec![fill(60.0, 30.0)]);
    assert!(!plan(&tree).iter().any(|op| op.starts_with('[')));
}

#[test]
fn a_marked_node_without_children_draws_nothing() {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
    let lonely = tree.add_child(root, Rect::new(10.0, 10.0, 60.0, 30.0)).unwrap();
    record(&tree, root, vec![fill(100.0, 100.0)]);
    record(&tree, lonely, vec![marker(), fill(60.0, 30.0)]);
    assert_eq!(
        plan(&tree),
        [format!("d{}", root.get())],
        "fading nothing shows nothing, and the mask is never drawn by itself"
    );
}

#[test]
fn a_fade_group_is_not_a_coverage_group() {
    let (tree, _, faded, _) = sample();
    record(&tree, faded, vec![marker(), fill(60.0, 30.0)]);
    assert!(!plan(&tree).iter().any(|op| op.ends_with('*')));
    record(&tree, faded, vec![coverage_marker(), fill(60.0, 30.0)]);
    let plan = plan(&tree);
    assert!(plan.contains(&format!("[{}*", faded.get())), "{plan:?}");
    assert!(!plan.iter().any(|op| op.ends_with('~')), "{plan:?}");
}

#[test]
fn opacity_and_a_fade_share_one_group() {
    let (tree, _, faded, content) = sample();
    record(&tree, faded, vec![marker(), fill(60.0, 30.0)]);
    tree.set_opacity(faded, 0.5).unwrap();
    let operations = tree.render_all();
    let groups: Vec<_> = operations
        .iter()
        .filter_map(|operation| match operation {
            RenderOp::BeginOpacityGroup {
                element,
                opacity,
                fade,
                ..
            } => Some((element.get(), *opacity, *fade)),
            _ => None,
        })
        .collect();
    assert_eq!(groups, [(faded.get(), 0.5, true)]);
    let draws: Vec<_> = operations
        .iter()
        .filter_map(|operation| match operation {
            RenderOp::Draw(item) => Some((item.element.get(), item.opacity)),
            _ => None,
        })
        .collect();
    assert!(
        draws.contains(&(faded.get(), 1.0)) && draws.contains(&(content.get(), 1.0)),
        "the group carries the opacity, so its items draw at full strength: {draws:?}"
    );
}
