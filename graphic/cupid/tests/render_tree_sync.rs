use aimer_cupid::draw_cmd_v2::{
    DrawCommand, Rect, RenderNodeSpec, RenderOp, RenderTree, RenderTreeError,
};
use aimer_cupid::utilities::Color;

#[test]
fn sync_keeps_ids_and_local_paint_while_reordering_and_resizing_nodes() {
    let tree = RenderTree::new();
    let initial = [
        RenderNodeSpec::new(None, None, Rect::new(0.0, 0.0, 300.0, 200.0)),
        RenderNodeSpec::new(None, Some(0), Rect::new(10.0, 10.0, 40.0, 30.0)),
        RenderNodeSpec::new(None, Some(0), Rect::new(100.0, 10.0, 50.0, 30.0)),
    ];
    let ids = tree.sync_structure(&initial).unwrap();
    let root = ids[0];
    let left = ids[1];
    let right = ids[2];

    tree.context(left)
        .unwrap()
        .begin_recording()
        .unwrap()
        .commit(vec![DrawCommand::FillRect {
            rect: Rect::new(0.0, 0.0, 40.0, 30.0),
            color: Color::rgba8(20, 80, 180, 255),
            border_radius: [0.0; 4],
            border_width: [0.0; 4],
            border_color: Color::transparent(),
            outline_width: [0.0; 4],
            outline_color: Color::transparent(),
        }])
        .unwrap();
    assert_eq!(tree.draw_list_revision(left).unwrap(), 1);
    let _ = tree.take_damage();

    let reordered = [
        RenderNodeSpec::new(Some(root), None, Rect::new(0.0, 0.0, 300.0, 200.0)),
        RenderNodeSpec::new(Some(right), Some(0), Rect::new(100.0, 10.0, 50.0, 30.0)),
        RenderNodeSpec::new(Some(left), Some(0), Rect::new(20.0, 10.0, 60.0, 30.0)),
    ];
    let reordered_ids = tree.sync_structure(&reordered).unwrap();

    assert_eq!(reordered_ids, [root, right, left]);
    assert_eq!(tree.parent_of(right).unwrap(), Some(root));
    assert_eq!(tree.parent_of(left).unwrap(), Some(root));
    assert_eq!(
        tree.element_bounds(left).unwrap(),
        Rect::new(20.0, 10.0, 60.0, 30.0)
    );
    assert_eq!(tree.draw_list_revision(left).unwrap(), 1);
    let draw_order = tree
        .render_all()
        .into_iter()
        .filter_map(|operation| match operation {
            RenderOp::Draw(item) => Some(item.element),
            RenderOp::BeginOpacityGroup { .. } | RenderOp::EndOpacityGroup { .. } => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(draw_order, [root, right, left]);
    assert!(!tree.take_damage().is_empty());
}

#[test]
fn sync_rejects_duplicate_ids_and_removes_nodes_omitted_from_the_next_snapshot() {
    let tree = RenderTree::new();
    let initial = [
        RenderNodeSpec::new(None, None, Rect::new(0.0, 0.0, 100.0, 100.0)),
        RenderNodeSpec::new(None, Some(0), Rect::new(0.0, 0.0, 10.0, 10.0)),
    ];
    let ids = tree.sync_structure(&initial).unwrap();
    let duplicate = [
        RenderNodeSpec::new(Some(ids[0]), None, Rect::new(0.0, 0.0, 100.0, 100.0)),
        RenderNodeSpec::new(Some(ids[1]), Some(0), Rect::new(0.0, 0.0, 10.0, 10.0)),
        RenderNodeSpec::new(Some(ids[1]), Some(0), Rect::new(10.0, 0.0, 10.0, 10.0)),
    ];

    assert_eq!(
        tree.sync_structure(&duplicate),
        Err(RenderTreeError::DuplicateNode(ids[1]))
    );

    let invalid_parent = [
        RenderNodeSpec::new(Some(ids[0]), None, Rect::new(0.0, 0.0, 100.0, 100.0)),
        RenderNodeSpec::new(Some(ids[1]), Some(1), Rect::new(0.0, 0.0, 10.0, 10.0)),
    ];
    assert!(matches!(
        tree.sync_structure(&invalid_parent),
        Err(RenderTreeError::InvalidParentIndex { node: 1, parent: 1 })
    ));

    let without_child = [RenderNodeSpec::new(
        Some(ids[0]),
        None,
        Rect::new(0.0, 0.0, 100.0, 100.0),
    )];
    tree.sync_structure(&without_child).unwrap();
    assert!(matches!(
        tree.context(ids[1]),
        Err(RenderTreeError::UnknownNode(id)) if id == ids[1]
    ));
}

/// Builds the same three-node tree twice, moves one node, and returns the
/// bounds and damage that each way of applying the move leaves behind.
fn moved_leaf_outcome(
    apply: impl FnOnce(&RenderTree, &[aimer_cupid::draw_cmd_v2::RenderNodeId]),
) -> (Vec<Rect>, Vec<Rect>) {
    let tree = RenderTree::new();
    let ids = tree
        .sync_structure(&[
            RenderNodeSpec::new(None, None, Rect::new(0.0, 0.0, 300.0, 200.0)),
            RenderNodeSpec::new(None, Some(0), Rect::new(10.0, 10.0, 40.0, 30.0)),
            RenderNodeSpec::new(None, Some(0), Rect::new(100.0, 10.0, 50.0, 30.0)),
        ])
        .unwrap();
    let _ = tree.take_damage();
    apply(&tree, &ids);
    let bounds = ids
        .iter()
        .map(|id| tree.element_bounds(*id).unwrap())
        .collect();
    (bounds, tree.take_damage())
}

#[test]
fn geometry_sync_matches_the_structure_sync_it_replaces() {
    let moved = Rect::new(20.0, 14.0, 60.0, 30.0);
    let clip = Some(Rect::new(0.0, 0.0, 30.0, 30.0));
    let radius = [4.0, 4.0, 0.0, 0.0];

    let full = moved_leaf_outcome(|tree, ids| {
        tree.sync_structure_with_clip_radii(
            &[
                RenderNodeSpec::new(Some(ids[0]), None, Rect::new(0.0, 0.0, 300.0, 200.0)),
                RenderNodeSpec::new(Some(ids[1]), Some(0), moved),
                RenderNodeSpec::new(Some(ids[2]), Some(0), Rect::new(100.0, 10.0, 50.0, 30.0)),
            ],
            &[None, clip, None],
            &[[0.0; 4], radius, [0.0; 4]],
        )
        .unwrap();
    });
    let incremental = moved_leaf_outcome(|tree, ids| {
        tree.sync_geometry(&[aimer_cupid::draw_cmd_v2::NodeGeometry {
            node: ids[1],
            bounds: moved,
            clip,
            clip_radius: radius,
        }])
        .unwrap();
    });

    assert_eq!(incremental.0, full.0, "the same bounds");
    assert_eq!(incremental.1, full.1, "the same damage");
}

#[test]
fn geometry_sync_leaves_an_unchanged_node_without_damage() {
    let (_, damage) = moved_leaf_outcome(|tree, ids| {
        tree.sync_geometry(&[aimer_cupid::draw_cmd_v2::NodeGeometry {
            node: ids[2],
            bounds: Rect::new(100.0, 10.0, 50.0, 30.0),
            clip: None,
            clip_radius: [0.0; 4],
        }])
        .unwrap();
    });

    assert!(damage.is_empty());
}

#[test]
fn geometry_sync_rejects_what_the_structure_sync_rejects() {
    let tree = RenderTree::new();
    let ids = tree
        .sync_structure(&[RenderNodeSpec::new(
            None,
            None,
            Rect::new(0.0, 0.0, 10.0, 10.0),
        )])
        .unwrap();
    let geometry = |bounds, clip, clip_radius| aimer_cupid::draw_cmd_v2::NodeGeometry {
        node: ids[0],
        bounds,
        clip,
        clip_radius,
    };

    assert_eq!(
        tree.sync_geometry(&[geometry(
            Rect::new(0.0, 0.0, f32::NAN, 10.0),
            None,
            [0.0; 4]
        )]),
        Err(RenderTreeError::InvalidBounds)
    );
    assert_eq!(
        tree.sync_geometry(&[geometry(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            None,
            [-1.0, 0.0, 0.0, 0.0]
        )]),
        Err(RenderTreeError::InvalidBounds)
    );
    let missing = aimer_cupid::draw_cmd_v2::NodeGeometry {
        node: ids[0],
        bounds: Rect::new(0.0, 0.0, 10.0, 10.0),
        clip: None,
        clip_radius: [0.0; 4],
    };
    let other = RenderTree::new();
    assert!(matches!(
        other.sync_geometry(&[missing]),
        Err(RenderTreeError::UnknownNode(_))
    ));
}
