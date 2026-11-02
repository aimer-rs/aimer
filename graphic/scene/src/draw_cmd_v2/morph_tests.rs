//! A node that opens its list with the morph marker plans its two children as
//! the sources of a shader transition.

use std::sync::Arc;

use super::*;

fn marker(params: MorphParams) -> DrawCommand {
    DrawCommand::DrawCustom {
        pipeline_name: Arc::from(MORPH_PIPELINE_NAME),
        data: Arc::from(params.encode().to_vec()),
    }
}

fn fill(rect: Rect) -> DrawCommand {
    DrawCommand::FillRect {
        rect,
        color: Color::rgba8(200, 30, 30, 255),
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

/// morph -> [from, to], each painting its whole box.
fn morph_tree(params: MorphParams) -> (RenderTree, [RenderNodeId; 3]) {
    let tree = RenderTree::new();
    let host = tree.add_root(Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
    let from = tree.add_child(host, Rect::new(0.0, 0.0, 40.0, 40.0)).unwrap();
    let to = tree.add_child(host, Rect::new(50.0, 50.0, 50.0, 50.0)).unwrap();
    record(&tree, host, vec![marker(params)]);
    record(&tree, from, vec![fill(Rect::new(0.0, 0.0, 40.0, 40.0))]);
    record(&tree, to, vec![fill(Rect::new(0.0, 0.0, 50.0, 50.0))]);
    (tree, [host, from, to])
}

#[derive(Debug, PartialEq)]
enum Step {
    Begin(RenderNodeId, Option<MorphGroup>),
    Draw(RenderNodeId),
    End(RenderNodeId),
}

fn steps(operations: Vec<RenderOp>) -> Vec<Step> {
    operations
        .into_iter()
        .map(|operation| match operation {
            RenderOp::BeginOpacityGroup { element, morph, .. } => Step::Begin(element, morph),
            RenderOp::Draw(item) => Step::Draw(item.element),
            RenderOp::EndOpacityGroup { element } => Step::End(element),
        })
        .collect()
}

#[test]
fn params_round_trip_through_the_marker_payload() {
    let mut params = MorphParams::new(MorphStyle::Wipe, 0.4);
    params.direction = [0.0, -1.0];
    params.softness = 0.5;
    params.amplitude = 0.2;
    params.frequency = 9.0;
    params.seed = 3.5;
    params.blur = 12.0;

    assert_eq!(MorphParams::decode(&params.encode()), Some(params));
}

#[test]
fn undrawable_payloads_do_not_decode() {
    let good = MorphParams::new(MorphStyle::Dissolve, 0.5).encode();
    assert!(MorphParams::decode(&good[..35]).is_none());
    assert!(MorphParams::decode(&[good.as_slice(), &[0u8; 4]].concat()).is_none());
    assert!(MorphParams::decode(&[]).is_none());

    let mut unknown_style = good;
    unknown_style[..4].copy_from_slice(&9u32.to_le_bytes());
    assert!(MorphParams::decode(&unknown_style).is_none());

    for progress in [-0.1, 1.1, f32::NAN, f32::INFINITY] {
        let bad = MorphParams::new(MorphStyle::Dissolve, progress).encode();
        assert!(MorphParams::decode(&bad).is_none(), "progress {progress}");
    }
    let mut soft = MorphParams::new(MorphStyle::Dissolve, 0.5);
    soft.softness = 2.0;
    assert!(MorphParams::decode(&soft.encode()).is_none());
    for blur in [-1.0, f32::NAN, f32::INFINITY, 1000.0] {
        let mut bad = MorphParams::new(MorphStyle::Blur, 0.5);
        bad.blur = blur;
        assert!(MorphParams::decode(&bad.encode()).is_none(), "blur {blur}");
    }
}

#[test]
fn a_morph_node_plans_one_source_group_per_child() {
    let params = MorphParams::new(MorphStyle::Dissolve, 0.5);
    let (tree, [host, from, to]) = morph_tree(params);

    assert_eq!(
        steps(tree.render_all()),
        vec![
            Step::Begin(host, Some(MorphGroup::Host(params))),
            Step::Begin(from, Some(MorphGroup::From)),
            Step::Draw(from),
            Step::End(from),
            Step::Begin(to, Some(MorphGroup::To)),
            Step::Draw(to),
            Step::End(to),
            Step::End(host),
        ],
        "the marker draws nothing, so the host has no item of its own"
    );
}

#[test]
fn the_host_group_covers_both_sources() {
    let (tree, _) = morph_tree(MorphParams::new(MorphStyle::Bounds, 0.5));
    let bounds = tree
        .render_all()
        .into_iter()
        .filter_map(|operation| match operation {
            RenderOp::BeginOpacityGroup { bounds, .. } => Some(bounds),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(bounds[0], Rect::new(0.0, 0.0, 100.0, 100.0));
    assert_eq!(bounds[1], Rect::new(0.0, 0.0, 40.0, 40.0));
    assert_eq!(bounds[2], Rect::new(50.0, 50.0, 50.0, 50.0));
}

#[test]
fn a_marked_node_without_exactly_two_children_is_ordinary() {
    let params = MorphParams::new(MorphStyle::Dissolve, 0.5);
    let tree = RenderTree::new();
    let host = tree.add_root(Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
    let only = tree.add_child(host, Rect::new(0.0, 0.0, 40.0, 40.0)).unwrap();
    record(&tree, host, vec![marker(params)]);
    record(&tree, only, vec![fill(Rect::new(0.0, 0.0, 40.0, 40.0))]);

    assert_eq!(
        steps(tree.render_all()),
        vec![Step::Draw(host), Step::Draw(only)]
    );

    let extra = tree.add_child(host, Rect::new(0.0, 0.0, 10.0, 10.0)).unwrap();
    let third = tree.add_child(host, Rect::new(0.0, 0.0, 10.0, 10.0)).unwrap();
    record(&tree, extra, vec![fill(Rect::new(0.0, 0.0, 10.0, 10.0))]);
    record(&tree, third, vec![fill(Rect::new(0.0, 0.0, 10.0, 10.0))]);
    assert!(
        steps(tree.render_all())
            .iter()
            .all(|step| !matches!(step, Step::Begin(_, Some(_)))),
        "three children are not a morph"
    );
}

#[test]
fn an_undecodable_marker_leaves_the_node_ordinary() {
    let tree = RenderTree::new();
    let host = tree.add_root(Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
    let a = tree.add_child(host, Rect::new(0.0, 0.0, 10.0, 10.0)).unwrap();
    let b = tree.add_child(host, Rect::new(20.0, 0.0, 10.0, 10.0)).unwrap();
    record(
        &tree,
        host,
        vec![DrawCommand::DrawCustom {
            pipeline_name: Arc::from(MORPH_PIPELINE_NAME),
            data: Arc::from(vec![1u8, 2, 3]),
        }],
    );
    record(&tree, a, vec![fill(Rect::new(0.0, 0.0, 10.0, 10.0))]);
    record(&tree, b, vec![fill(Rect::new(0.0, 0.0, 10.0, 10.0))]);

    assert_eq!(
        steps(tree.render_all()),
        vec![Step::Draw(host), Step::Draw(a), Step::Draw(b)]
    );
}

#[test]
fn damage_that_misses_a_source_leaves_that_source_out() {
    let params = MorphParams::new(MorphStyle::Wipe, 0.5);
    let (tree, [host, _, to]) = morph_tree(params);

    // Only the incoming box is damaged; the outgoing one paints nothing there.
    let plan = steps(tree.render_order(&[Rect::new(70.0, 70.0, 10.0, 10.0)]));

    assert_eq!(
        plan,
        vec![
            Step::Begin(host, Some(MorphGroup::Host(params))),
            Step::Begin(to, Some(MorphGroup::To)),
            Step::Draw(to),
            Step::End(to),
            Step::End(host),
        ]
    );
}

#[test]
fn a_changed_progress_rerecords_and_damages_the_host() {
    let (tree, [host, _, _]) = morph_tree(MorphParams::new(MorphStyle::Dissolve, 0.2));
    let _ = tree.take_damage();

    record(&tree, host, vec![marker(MorphParams::new(MorphStyle::Dissolve, 0.6))]);

    assert!(!tree.take_damage().is_empty());
    let host_group = tree.render_all().into_iter().find_map(|operation| match operation {
        RenderOp::BeginOpacityGroup { element, morph, .. } if element == host => morph,
        _ => None,
    });
    assert_eq!(
        host_group,
        Some(MorphGroup::Host(MorphParams::new(MorphStyle::Dissolve, 0.6)))
    );
}

/// A host as small as the incoming card, whose outgoing card reaches further.
fn overhanging_morph(params: MorphParams) -> (RenderTree, RenderNodeId) {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 300.0, 300.0)).unwrap();
    let host = tree.add_child(root, Rect::new(10.0, 10.0, 80.0, 40.0)).unwrap();
    let from = tree.add_child(host, Rect::new(0.0, 0.0, 40.0, 200.0)).unwrap();
    let to = tree.add_child(host, Rect::new(0.0, 0.0, 80.0, 40.0)).unwrap();
    record(&tree, host, vec![marker(params)]);
    record(&tree, from, vec![fill(Rect::new(0.0, 0.0, 40.0, 200.0))]);
    record(&tree, to, vec![fill(Rect::new(0.0, 0.0, 80.0, 40.0))]);
    (tree, host)
}

fn covers(damage: &[Rect], target: Rect) -> bool {
    damage.iter().any(|rect| {
        rect.x <= target.x
            && rect.y <= target.y
            && rect.x + rect.width >= target.x + target.width
            && rect.y + rect.height >= target.y + target.height
    })
}

#[test]
fn a_morph_frame_damages_everything_either_picture_covers() {
    let (tree, host) = overhanging_morph(MorphParams::new(MorphStyle::Dissolve, 0.2));
    let _ = tree.take_damage();

    record(&tree, host, vec![marker(MorphParams::new(MorphStyle::Dissolve, 0.6))]);

    // The outgoing card hangs 160 px below the host's own bounds; a frame that
    // only damaged the host would leave its old pixels on screen.
    let damage = tree.take_damage();
    assert!(
        covers(&damage, Rect::new(10.0, 10.0, 80.0, 200.0)),
        "the damage must reach both pictures: {damage:?}"
    );
}

#[test]
fn ending_a_morph_damages_what_the_marker_blended() {
    let (tree, host) = overhanging_morph(MorphParams::new(MorphStyle::Dissolve, 0.9));
    let _ = tree.take_damage();

    record(&tree, host, Vec::new());

    let damage = tree.take_damage();
    assert!(
        covers(&damage, Rect::new(10.0, 10.0, 80.0, 200.0)),
        "dropping the marker repaints the pictures it covered: {damage:?}"
    );
}

#[test]
fn an_ordinary_node_still_damages_only_its_own_bounds() {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 300.0, 300.0)).unwrap();
    let parent = tree.add_child(root, Rect::new(10.0, 10.0, 80.0, 40.0)).unwrap();
    let child = tree.add_child(parent, Rect::new(0.0, 0.0, 40.0, 200.0)).unwrap();
    record(&tree, child, vec![fill(Rect::new(0.0, 0.0, 40.0, 200.0))]);
    let _ = tree.take_damage();

    record(&tree, parent, vec![fill(Rect::new(0.0, 0.0, 80.0, 40.0))]);

    let damage = tree.take_damage();
    assert!(!covers(&damage, Rect::new(10.0, 10.0, 80.0, 200.0)), "{damage:?}");
}
