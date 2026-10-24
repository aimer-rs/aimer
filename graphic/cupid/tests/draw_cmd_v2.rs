use aimer_cupid::draw_cmd_v2::{
    DrawCommand, LocalVisibleRegion, Rect, RenderNodeSpec, RenderOp, RenderPaintSource, RenderTree,
};
use aimer_cupid::utilities::{Color, Mat3};
use std::sync::Arc;

#[test]
fn writer_commits_commands_to_only_its_element_local_list() {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
    let child = tree
        .add_child(root, Rect::new(10.0, 12.0, 40.0, 30.0))
        .unwrap();

    let context = tree.context(child).unwrap();
    let writer = context.begin_recording().unwrap();
    writer
        .commit(vec![fill_command(Rect::new(0.0, 0.0, 40.0, 30.0))])
        .unwrap();

    let child_list = tree.draw_list_snapshot(child).unwrap();
    assert_eq!(child_list.revision, 1);
    assert!(matches!(
        child_list.commands[0],
        DrawCommand::FillRect { .. }
    ));
    assert_eq!(tree.draw_list_revision(root).unwrap(), 0);
    assert!(tree.draw_list_snapshot(root).unwrap().commands.is_empty());
}

#[test]
fn local_command_snapshots_share_storage_until_the_list_changes() {
    let tree = RenderTree::new();
    let node = tree.add_root(Rect::new(0.0, 0.0, 40.0, 30.0)).unwrap();
    let context = tree.context(node).unwrap();
    context
        .begin_recording()
        .unwrap()
        .commit(vec![fill_command(Rect::new(0.0, 0.0, 20.0, 20.0))])
        .unwrap();

    let first = tree.draw_list_snapshot(node).unwrap();
    let unchanged = tree.draw_list_snapshot(node).unwrap();
    assert!(Arc::ptr_eq(&first.commands, &unchanged.commands));

    context
        .begin_recording()
        .unwrap()
        .commit(vec![fill_command(Rect::new(0.0, 0.0, 24.0, 20.0))])
        .unwrap();
    let changed = tree.draw_list_snapshot(node).unwrap();
    assert_eq!(changed.revision, first.revision + 1);
    assert!(!Arc::ptr_eq(&first.commands, &changed.commands));
}

#[test]
fn writer_rejects_unbalanced_scopes_without_replacing_the_retained_list() {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
    let context = tree.context(root).unwrap();

    context
        .begin_recording()
        .unwrap()
        .commit(vec![fill_command(Rect::new(0.0, 0.0, 20.0, 20.0))])
        .unwrap();
    let before = tree.draw_list_snapshot(root).unwrap();

    assert!(
        context
            .begin_recording()
            .unwrap()
            .commit(vec![DrawCommand::PushClip {
                rect: Rect::new(0.0, 0.0, 12.0, 12.0),
                border_radius: [0.0; 4],
            }])
            .is_err()
    );

    let after = tree.draw_list_snapshot(root).unwrap();
    assert_eq!(after.revision, before.revision);
    assert_eq!(after.commands.len(), before.commands.len());
    assert!(matches!(
        after.commands.first(),
        Some(DrawCommand::FillRect { .. })
    ));
    assert!(tree.dirty_elements().contains(&root));

    drop(context.begin_recording().unwrap());
    assert!(tree.dirty_elements().contains(&root));
    assert!(context.begin_recording().is_ok());
}

#[test]
fn geometry_and_opacity_changes_produce_a_damage_limited_group_plan() {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
    let child = tree
        .add_child(root, Rect::new(10.0, 10.0, 30.0, 30.0))
        .unwrap();
    record_fill(&tree, root);
    record_fill(&tree, child);
    tree.take_damage();

    tree.set_bounds(child, Rect::new(20.0, 10.0, 30.0, 30.0))
        .unwrap();
    tree.set_opacity(root, 0.5).unwrap();
    let frame = tree.render_pending();

    assert!(!frame.damage.is_empty());
    assert!(matches!(
        frame.operations.first(),
        Some(RenderOp::BeginOpacityGroup { element, opacity, .. })
            if *element == root && *opacity == 0.5
    ));
    assert!(
        matches!(frame.operations.last(), Some(RenderOp::EndOpacityGroup { element }) if *element == root)
    );
}

#[test]
fn compositor_transform_updates_descendant_geometry_without_rerecording_lists() {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
    let child = tree
        .add_child(root, Rect::new(10.0, 20.0, 40.0, 20.0))
        .unwrap();
    tree.set_paint_source(root, RenderPaintSource::LocalV2)
        .unwrap();
    tree.set_paint_source(child, RenderPaintSource::LocalV2)
        .unwrap();
    record_fill(&tree, root);
    record_fill(&tree, child);
    let root_revision = tree.draw_list_revision(root).unwrap();
    let child_revision = tree.draw_list_revision(child).unwrap();
    tree.take_damage();

    let transform = Mat3::translate(50.0, 50.0)
        .mul(&Mat3::scale(0.5, 0.5))
        .mul(&Mat3::translate(-50.0, -50.0));
    tree.set_transform(root, transform).unwrap();

    let damage = tree.take_damage();
    assert!(!damage.is_empty());
    let operations = tree.render_all();
    let child_item = operations.iter().find_map(|operation| match operation {
        RenderOp::Draw(item) if item.element == child => Some(item),
        _ => None,
    });
    let child_item = child_item.expect("transformed child remains in render order");
    assert_eq!(child_item.bounds, Rect::new(30.0, 35.0, 20.0, 10.0));
    assert_eq!(child_item.transform, transform);
    assert_eq!(tree.draw_list_revision(root).unwrap(), root_revision);
    assert_eq!(tree.draw_list_revision(child).unwrap(), child_revision);
}

#[test]
fn build_context_updates_child_presentation_without_rerecording_lists() {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
    let child = tree
        .add_child(root, Rect::new(10.0, 10.0, 20.0, 20.0))
        .unwrap();
    tree.set_paint_source(root, RenderPaintSource::LocalV2)
        .unwrap();
    tree.set_paint_source(child, RenderPaintSource::LocalV2)
        .unwrap();
    record_fill(&tree, root);
    record_fill(&tree, child);
    let root_revision = tree.draw_list_revision(root).unwrap();
    let child_revision = tree.draw_list_revision(child).unwrap();
    tree.take_damage();

    let context = tree.context(root).unwrap();
    assert!(context.set_child_presentation_at(0, Mat3::scale(1.5, 0.5), 0.4));

    assert!(!tree.take_damage().is_empty());
    assert_eq!(
        tree.element_bounds(child).unwrap(),
        Rect::new(10.0, 10.0, 30.0, 10.0)
    );
    let child_item = tree
        .render_all()
        .into_iter()
        .find_map(|operation| match operation {
            RenderOp::Draw(item) if item.element == child => Some(item),
            _ => None,
        });
    let child_item = child_item.expect("the presented child remains in render order");
    let expected_transform = Mat3::translate(10.0, 10.0)
        .mul(&Mat3::scale(1.5, 0.5))
        .mul(&Mat3::translate(-10.0, -10.0));
    assert_eq!(child_item.transform, expected_transform);
    assert_eq!(child_item.opacity, 0.4);
    assert_eq!(tree.draw_list_revision(root).unwrap(), root_revision);
    assert_eq!(tree.draw_list_revision(child).unwrap(), child_revision);
}

#[test]
fn compositor_transform_rejects_non_finite_values_and_culls_zero_area_content() {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
    tree.set_paint_source(root, RenderPaintSource::LocalV2)
        .unwrap();
    record_fill(&tree, root);
    tree.take_damage();

    assert_eq!(
        tree.set_transform(root, Mat3::scale(f32::NAN, 1.0)),
        Err(aimer_cupid::draw_cmd_v2::RenderTreeError::InvalidTransform)
    );
    assert!(tree.take_damage().is_empty());

    let collapse = Mat3::translate(50.0, 50.0)
        .mul(&Mat3::scale(0.0, 0.0))
        .mul(&Mat3::translate(-50.0, -50.0));
    tree.set_transform(root, collapse).unwrap();
    assert!(!tree.take_damage().is_empty());
    assert!(tree.render_all().is_empty());
}

#[test]
fn invalid_geometry_and_opacity_are_rejected_without_adding_damage() {
    let tree = RenderTree::new();
    assert_eq!(
        tree.add_root(Rect::new(0.0, 0.0, -1.0, 10.0)),
        Err(aimer_cupid::draw_cmd_v2::RenderTreeError::InvalidBounds)
    );

    let root = tree.add_root(Rect::new(0.0, 0.0, 10.0, 10.0)).unwrap();
    tree.take_damage();
    assert_eq!(
        tree.set_opacity(root, f32::NAN),
        Err(aimer_cupid::draw_cmd_v2::RenderTreeError::InvalidOpacity)
    );
    assert_eq!(
        tree.set_opacity(root, 1.1),
        Err(aimer_cupid::draw_cmd_v2::RenderTreeError::InvalidOpacity)
    );
    assert!(tree.take_damage().is_empty());
}

#[test]
fn disjoint_nested_clips_hide_descendants_without_creating_damage() {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(10.0, 10.0, 40.0, 40.0)).unwrap();
    let child = tree
        .add_child(root, Rect::new(40.0, 40.0, 20.0, 20.0))
        .unwrap();
    tree.set_clip(root, Some(Rect::new(0.0, 0.0, 20.0, 20.0)))
        .unwrap();
    tree.set_clip(child, Some(Rect::new(0.0, 0.0, 20.0, 20.0)))
        .unwrap();
    tree.set_paint_source(root, RenderPaintSource::LocalV2)
        .unwrap();
    tree.set_paint_source(child, RenderPaintSource::LocalV2)
        .unwrap();
    tree.take_damage();

    let operations = tree.render_all();
    assert!(matches!(
        operations.as_slice(),
        [RenderOp::Draw(item)] if item.element == root
    ));

    tree.invalidate_paint(child).unwrap();
    assert!(tree.take_damage().is_empty());
}

#[test]
fn structure_sync_moves_a_clipped_viewport_child_atomically() {
    let tree = RenderTree::new();
    let viewport = tree.add_root(Rect::new(0.0, 0.0, 100.0, 80.0)).unwrap();
    let content = tree
        .add_child(viewport, Rect::new(0.0, 0.0, 100.0, 1_000.0))
        .unwrap();
    tree.set_clip(content, Some(Rect::new(0.0, 0.0, 100.0, 80.0)))
        .unwrap();
    tree.take_damage();

    tree.sync_structure_with_clips(
        &[
            RenderNodeSpec::new(Some(viewport), None, Rect::new(0.0, 0.0, 100.0, 80.0)),
            RenderNodeSpec::new(
                Some(content),
                Some(0),
                Rect::new(0.0, -400.0, 100.0, 1_000.0),
            ),
        ],
        &[None, Some(Rect::new(0.0, 400.0, 100.0, 80.0))],
    )
    .unwrap();

    let damage = tree.take_damage();
    assert!(!damage.is_empty());
    assert!(
        damage.iter().all(|rect| {
            rect.x >= 0.0
                && rect.y >= 0.0
                && rect.x + rect.width <= 100.0
                && rect.y + rect.height <= 80.0
        }),
        "viewport sync damaged intermediate offscreen geometry: {damage:?}"
    );
}

#[test]
fn subtree_local_v2_check_accepts_fully_retained_descendants() {
    let tree = RenderTree::new();
    let viewport = tree.add_root(Rect::new(0.0, 0.0, 100.0, 80.0)).unwrap();
    let content = tree
        .add_child(viewport, Rect::new(0.0, 0.0, 100.0, 1_000.0))
        .unwrap();
    tree.set_paint_source(viewport, RenderPaintSource::LocalV2)
        .unwrap();
    tree.set_paint_source(content, RenderPaintSource::LocalV2)
        .unwrap();

    assert!(tree.subtree_uses_only_local_v2(viewport).unwrap());
}

#[test]
fn subtree_local_v2_check_rejects_unresolved_descendants() {
    let tree = RenderTree::new();
    let viewport = tree.add_root(Rect::new(0.0, 0.0, 100.0, 80.0)).unwrap();
    let content = tree
        .add_child(viewport, Rect::new(0.0, 0.0, 100.0, 1_000.0))
        .unwrap();
    let leaf = tree
        .add_child(content, Rect::new(0.0, 0.0, 10.0, 10.0))
        .unwrap();
    tree.set_paint_source(viewport, RenderPaintSource::LocalV2)
        .unwrap();
    tree.set_paint_source(content, RenderPaintSource::LocalV2)
        .unwrap();

    assert!(!tree.subtree_uses_only_local_v2(viewport).unwrap());

    tree.set_paint_source(leaf, RenderPaintSource::LocalV2)
        .unwrap();
    assert!(tree.subtree_uses_only_local_v2(viewport).unwrap());

    tree.set_paint_source(leaf, RenderPaintSource::Unresolved)
        .unwrap();
    assert!(!tree.subtree_uses_only_local_v2(viewport).unwrap());
}

#[test]
fn structure_sync_rejects_clips_that_do_not_match_the_node_list() {
    let tree = RenderTree::new();
    let node = tree.add_root(Rect::new(0.0, 0.0, 20.0, 20.0)).unwrap();
    tree.take_damage();

    assert_eq!(
        tree.sync_structure_with_clips(
            &[RenderNodeSpec::new(
                Some(node),
                None,
                Rect::new(10.0, 10.0, 20.0, 20.0)
            )],
            &[],
        ),
        Err(aimer_cupid::draw_cmd_v2::RenderTreeError::InvalidBounds)
    );
    assert_eq!(
        tree.element_bounds(node).unwrap(),
        Rect::new(0.0, 0.0, 20.0, 20.0)
    );
    assert!(tree.take_damage().is_empty());
}

fn fill_command(rect: Rect) -> DrawCommand {
    DrawCommand::FillRect {
        rect,
        color: Color::rgba8(1, 2, 3, 255),
        border_radius: [0.0; 4],
        border_width: [0.0; 4],
        border_color: Color::transparent(),
        outline_width: [0.0; 4],
        outline_color: Color::transparent(),
    }
}

/// A local list is authored against the node's size (a background fills it), so
/// a node that grows must be recorded again or the new area stays unpainted.
#[test]
fn resizing_a_node_marks_its_recorded_list_stale_but_moving_it_does_not() {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
    record_fill(&tree, root);
    assert!(tree.dirty_elements().is_empty());

    tree.set_bounds(root, Rect::new(10.0, 5.0, 100.0, 100.0))
        .unwrap();
    assert!(
        tree.dirty_elements().is_empty(),
        "a move keeps the recorded list"
    );

    tree.set_bounds(root, Rect::new(10.0, 5.0, 140.0, 100.0))
        .unwrap();
    assert_eq!(tree.dirty_elements(), vec![root]);
}

#[test]
fn structure_sync_that_resizes_a_node_marks_its_recorded_list_stale() {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
    let child = tree
        .add_child(root, Rect::new(0.0, 0.0, 40.0, 40.0))
        .unwrap();
    record_fill(&tree, root);
    record_fill(&tree, child);
    assert!(tree.dirty_elements().is_empty());

    // Same structure: only the root grows.
    tree.sync_structure_with_clips(
        &[
            RenderNodeSpec::new(Some(root), None, Rect::new(0.0, 0.0, 160.0, 100.0)),
            RenderNodeSpec::new(Some(child), Some(0), Rect::new(0.0, 0.0, 40.0, 40.0)),
        ],
        &[None, None],
    )
    .unwrap();
    assert_eq!(tree.dirty_elements(), vec![root]);
    record_fill(&tree, root);

    // A new sibling changes the structure while the child grows.
    tree.sync_structure_with_clips(
        &[
            RenderNodeSpec::new(Some(root), None, Rect::new(0.0, 0.0, 160.0, 100.0)),
            RenderNodeSpec::new(Some(child), Some(0), Rect::new(0.0, 0.0, 60.0, 40.0)),
            RenderNodeSpec::new(None, Some(0), Rect::new(70.0, 0.0, 20.0, 20.0)),
        ],
        &[None, None, None],
    )
    .unwrap();
    let dirty = tree.dirty_elements();
    assert!(
        dirty.contains(&child),
        "the grown child must be recorded again"
    );
    assert!(!dirty.contains(&root), "the unchanged root keeps its list");
}

/// A scrollable's content node: it moves under a fixed viewport clip, and its
/// only descendant is a small leaf, so the tight footprint is far smaller than
/// the viewport.
fn viewport_content() -> (
    RenderTree,
    aimer_cupid::draw_cmd_v2::RenderNodeId,
    aimer_cupid::draw_cmd_v2::RenderNodeId,
) {
    let tree = RenderTree::new();
    let viewport = tree.add_root(Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
    let content = tree
        .add_child(viewport, Rect::new(0.0, 0.0, 100.0, 400.0))
        .unwrap();
    let leaf = tree
        .add_child(content, Rect::new(0.0, 0.0, 10.0, 10.0))
        .unwrap();
    record_fill(&tree, leaf);
    // The clip is local to the content's origin, so it compensates the offset
    // and keeps the viewport fixed in the window.
    tree.set_geometry(
        content,
        Rect::new(0.0, 0.0, 100.0, 400.0),
        Some(Rect::new(0.0, 0.0, 100.0, 100.0)),
    )
    .unwrap();
    tree.take_damage();
    (tree, content, leaf)
}

fn union_of(rects: &[Rect]) -> Rect {
    assert!(!rects.is_empty(), "damage was expected");
    let left = rects.iter().map(|rect| rect.x).fold(f32::MAX, f32::min);
    let top = rects.iter().map(|rect| rect.y).fold(f32::MAX, f32::min);
    let right = rects
        .iter()
        .map(|rect| rect.x + rect.width)
        .fold(f32::MIN, f32::max);
    let bottom = rects
        .iter()
        .map(|rect| rect.y + rect.height)
        .fold(f32::MIN, f32::max);
    Rect::new(left, top, right - left, bottom - top)
}

#[test]
fn moving_a_clipped_viewport_child_damages_the_whole_viewport_clip() {
    let (tree, content, _leaf) = viewport_content();

    tree.set_geometry(
        content,
        Rect::new(0.0, -50.0, 100.0, 400.0),
        Some(Rect::new(0.0, 50.0, 100.0, 100.0)),
    )
    .unwrap();

    // The clip bounds every descendant, so the damage is the clip footprint
    // and does not have to be recovered by measuring the content subtree.
    assert_eq!(
        union_of(&tree.take_damage()),
        Rect::new(0.0, 0.0, 100.0, 100.0)
    );
}

#[test]
fn moving_a_clipped_child_to_the_same_geometry_adds_no_damage() {
    let (tree, content, _leaf) = viewport_content();

    tree.set_geometry(
        content,
        Rect::new(0.0, 0.0, 100.0, 400.0),
        Some(Rect::new(0.0, 0.0, 100.0, 100.0)),
    )
    .unwrap();

    assert!(tree.take_damage().is_empty());
}

#[test]
fn moving_an_unclipped_child_keeps_the_tight_subtree_damage() {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
    let child = tree
        .add_child(root, Rect::new(10.0, 10.0, 30.0, 30.0))
        .unwrap();
    tree.take_damage();

    tree.set_geometry(child, Rect::new(50.0, 10.0, 30.0, 30.0), None)
        .unwrap();

    assert_eq!(
        union_of(&tree.take_damage()),
        Rect::new(10.0, 10.0, 70.0, 30.0)
    );
}

#[test]
fn the_local_visible_region_is_the_inherited_clip_in_the_nodes_own_coordinates() {
    let tree = RenderTree::new();
    let viewport = tree.add_root(Rect::new(10.0, 20.0, 100.0, 100.0)).unwrap();
    let content = tree
        .add_child(viewport, Rect::new(0.0, 0.0, 100.0, 400.0))
        .unwrap();
    tree.set_clip(viewport, Some(Rect::new(0.0, 0.0, 100.0, 100.0)))
        .unwrap();
    let inside = tree
        .add_child(content, Rect::new(0.0, 10.0, 100.0, 30.0))
        .unwrap();
    let below = tree
        .add_child(content, Rect::new(0.0, 200.0, 100.0, 30.0))
        .unwrap();

    // The clip covers world (10, 20) to (110, 120); `inside` sits at (10, 30).
    assert_eq!(
        tree.local_visible_region(inside).unwrap(),
        LocalVisibleRegion::Within(Rect::new(0.0, -10.0, 100.0, 100.0))
    );
    // The region is where anything below the node could be seen, so it does not
    // depend on whether the node itself is in it: `below` is far outside.
    assert_eq!(
        tree.local_visible_region(below).unwrap(),
        LocalVisibleRegion::Within(Rect::new(0.0, -200.0, 100.0, 100.0))
    );
}

#[test]
fn a_nodes_own_clip_bounds_its_region_and_an_unclipped_tree_has_none() {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 200.0, 200.0)).unwrap();
    let child = tree
        .add_child(root, Rect::new(30.0, 40.0, 100.0, 100.0))
        .unwrap();
    assert_eq!(
        tree.local_visible_region(child).unwrap(),
        LocalVisibleRegion::Unbounded
    );

    tree.set_clip(child, Some(Rect::new(5.0, 6.0, 20.0, 30.0)))
        .unwrap();
    assert_eq!(
        tree.local_visible_region(child).unwrap(),
        LocalVisibleRegion::Within(Rect::new(5.0, 6.0, 20.0, 30.0))
    );
}

#[test]
fn disjoint_clips_leave_nothing_visible() {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(10.0, 10.0, 40.0, 40.0)).unwrap();
    let child = tree
        .add_child(root, Rect::new(40.0, 40.0, 20.0, 20.0))
        .unwrap();
    tree.set_clip(root, Some(Rect::new(0.0, 0.0, 20.0, 20.0)))
        .unwrap();
    tree.set_clip(child, Some(Rect::new(0.0, 0.0, 20.0, 20.0)))
        .unwrap();

    assert_eq!(
        tree.local_visible_region(child).unwrap(),
        LocalVisibleRegion::Nothing
    );
}

#[test]
fn the_local_visible_region_follows_a_moved_clipped_viewport_child() {
    let (tree, content, leaf) = viewport_content();
    assert_eq!(
        tree.local_visible_region(leaf).unwrap(),
        LocalVisibleRegion::Within(Rect::new(0.0, 0.0, 100.0, 100.0))
    );

    // Scrolling the content by 150: the viewport now shows local y 150..250.
    tree.set_geometry(
        content,
        Rect::new(0.0, -150.0, 100.0, 400.0),
        Some(Rect::new(0.0, 150.0, 100.0, 100.0)),
    )
    .unwrap();

    assert_eq!(
        tree.local_visible_region(leaf).unwrap(),
        LocalVisibleRegion::Within(Rect::new(0.0, 150.0, 100.0, 100.0))
    );
}

#[test]
fn a_scaled_or_rotated_placement_has_no_trustworthy_local_region() {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
    let child = tree
        .add_child(root, Rect::new(10.0, 10.0, 50.0, 50.0))
        .unwrap();
    tree.set_clip(root, Some(Rect::new(0.0, 0.0, 100.0, 100.0)))
        .unwrap();
    assert!(matches!(
        tree.local_visible_region(child).unwrap(),
        LocalVisibleRegion::Within(_)
    ));

    tree.set_transform(root, Mat3::scale(2.0, 2.0)).unwrap();
    assert_eq!(
        tree.local_visible_region(child).unwrap(),
        LocalVisibleRegion::Unbounded
    );

    tree.set_transform(root, Mat3::rotate(0.5)).unwrap();
    assert_eq!(
        tree.local_visible_region(child).unwrap(),
        LocalVisibleRegion::Unbounded
    );

    // A plain translation of the root carries its clip and the child along
    // together, so the clip is unchanged relative to the child.
    tree.set_transform(root, Mat3::translate(7.0, 3.0)).unwrap();
    assert_eq!(
        tree.local_visible_region(child).unwrap(),
        LocalVisibleRegion::Within(Rect::new(-10.0, -10.0, 100.0, 100.0))
    );
}

#[test]
fn the_local_visible_region_rejects_an_unknown_node() {
    let tree = RenderTree::new();
    let other = RenderTree::new();
    let foreign = other.add_root(Rect::new(0.0, 0.0, 10.0, 10.0)).unwrap();
    assert!(tree.local_visible_region(foreign).is_err());
}

#[test]
fn a_node_is_settled_only_while_it_holds_a_clean_recorded_list() {
    let tree = RenderTree::new();
    let node = tree.add_root(Rect::new(0.0, 0.0, 40.0, 40.0)).unwrap();
    assert!(
        !tree.is_settled(node).unwrap(),
        "a new node has no paint source"
    );

    tree.set_paint_source(node, RenderPaintSource::LocalV2)
        .unwrap();
    assert!(
        !tree.is_settled(node).unwrap(),
        "its list was never recorded"
    );

    record_fill(&tree, node);
    assert!(tree.is_settled(node).unwrap());

    tree.invalidate_paint(node).unwrap();
    assert!(
        !tree.is_settled(node).unwrap(),
        "an invalidated list must be recorded again"
    );

    record_fill(&tree, node);
    assert!(tree.is_settled(node).unwrap());

    // Growing the node makes its list stale; a pure move does not.
    tree.set_bounds(node, Rect::new(5.0, 0.0, 40.0, 40.0))
        .unwrap();
    assert!(tree.is_settled(node).unwrap());
    tree.set_bounds(node, Rect::new(5.0, 0.0, 60.0, 40.0))
        .unwrap();
    assert!(!tree.is_settled(node).unwrap());
}

#[test]
fn a_node_whose_element_painted_nothing_is_settled_once_the_attempt_is_marked() {
    // A layout container records no commands, so its list is never committed
    // and `needs_recording` stays true. It is nonetheless up to date.
    let tree = RenderTree::new();
    let node = tree.add_root(Rect::new(0.0, 0.0, 40.0, 40.0)).unwrap();
    tree.set_paint_source(node, RenderPaintSource::LocalV2)
        .unwrap();
    assert!(
        !tree.is_settled(node).unwrap(),
        "no recording was attempted yet"
    );

    tree.mark_recording_attempted(node).unwrap();
    assert!(tree.is_settled(node).unwrap());
    assert!(
        tree.needs_recording(node).unwrap(),
        "marking an attempt must not change what is asked to be recorded"
    );
    assert_eq!(tree.dirty_elements(), vec![node]);

    // Anything that makes the list stale unsettles it again.
    tree.invalidate_paint(node).unwrap();
    assert!(!tree.is_settled(node).unwrap());
    tree.mark_recording_attempted(node).unwrap();
    assert!(tree.is_settled(node).unwrap());

    tree.set_bounds(node, Rect::new(0.0, 0.0, 50.0, 40.0))
        .unwrap();
    assert!(
        !tree.is_settled(node).unwrap(),
        "a resized node must be recorded again"
    );
}

#[test]
fn marking_an_attempt_does_not_settle_a_node_that_is_being_recorded() {
    let tree = RenderTree::new();
    let node = tree.add_root(Rect::new(0.0, 0.0, 40.0, 40.0)).unwrap();
    tree.set_paint_source(node, RenderPaintSource::LocalV2)
        .unwrap();
    let _writer = tree.context(node).unwrap().begin_recording().unwrap();

    tree.mark_recording_attempted(node).unwrap();

    assert!(!tree.is_settled(node).unwrap());
}

#[test]
fn a_recorder_dropped_without_committing_leaves_the_node_unsettled() {
    let tree = RenderTree::new();
    let node = tree.add_root(Rect::new(0.0, 0.0, 40.0, 40.0)).unwrap();
    tree.set_paint_source(node, RenderPaintSource::LocalV2)
        .unwrap();
    record_fill(&tree, node);
    assert!(tree.is_settled(node).unwrap());
    tree.invalidate_paint(node).unwrap();

    drop(tree.context(node).unwrap().begin_recording().unwrap());

    assert!(
        !tree.is_settled(node).unwrap(),
        "the abandoned recording must be retried"
    );
}

#[test]
fn a_node_with_an_open_recorder_is_not_settled() {
    let tree = RenderTree::new();
    let node = tree.add_root(Rect::new(0.0, 0.0, 40.0, 40.0)).unwrap();
    tree.set_paint_source(node, RenderPaintSource::LocalV2)
        .unwrap();
    record_fill(&tree, node);
    let _writer = tree.context(node).unwrap().begin_recording().unwrap();
    assert!(!tree.is_settled(node).unwrap());
}

fn image_command(texture_id: u32) -> DrawCommand {
    DrawCommand::DrawImage {
        rect: Rect::new(0.0, 0.0, 8.0, 8.0),
        texture_id,
    }
}

fn resource_image_command(texture_id: u32) -> DrawCommand {
    let resource = aimer_cupid::draw_cmd_v2::ImageResource::rgba8(
        texture_id,
        1,
        1,
        1,
        Arc::from(vec![255u8, 0, 0, 255]),
    )
    .expect("a 1x1 RGBA image");
    DrawCommand::DrawImageWithResource {
        rect: Rect::new(0.0, 0.0, 8.0, 8.0),
        resource: Arc::new(resource),
    }
}

fn record(
    tree: &RenderTree,
    node: aimer_cupid::draw_cmd_v2::RenderNodeId,
    commands: Vec<DrawCommand>,
) {
    tree.context(node)
        .unwrap()
        .begin_recording()
        .unwrap()
        .commit(commands)
        .unwrap();
}

#[test]
fn a_snapshot_lists_the_textures_its_commands_draw_in_command_order() {
    let tree = RenderTree::new();
    let node = tree.add_root(Rect::new(0.0, 0.0, 40.0, 40.0)).unwrap();
    assert!(
        tree.draw_list_snapshot(node)
            .unwrap()
            .image_textures
            .is_empty()
    );

    record(
        &tree,
        node,
        vec![
            fill_command(Rect::new(0.0, 0.0, 4.0, 4.0)),
            image_command(7),
            resource_image_command(3),
            fill_command(Rect::new(1.0, 1.0, 4.0, 4.0)),
            image_command(7),
            resource_image_command(11),
        ],
    );

    // Order and repeats are kept: they are the sequence the commands draw in.
    assert_eq!(
        &*tree.draw_list_snapshot(node).unwrap().image_textures,
        &[7, 3, 7, 11]
    );
}

#[test]
fn a_list_without_images_has_no_textures_and_re_recording_replaces_them() {
    let tree = RenderTree::new();
    let node = tree.add_root(Rect::new(0.0, 0.0, 40.0, 40.0)).unwrap();

    record(&tree, node, vec![image_command(5)]);
    assert_eq!(
        &*tree.draw_list_snapshot(node).unwrap().image_textures,
        &[5]
    );

    record(
        &tree,
        node,
        vec![fill_command(Rect::new(0.0, 0.0, 4.0, 4.0))],
    );
    assert!(
        tree.draw_list_snapshot(node)
            .unwrap()
            .image_textures
            .is_empty()
    );

    record(&tree, node, vec![resource_image_command(9)]);
    assert_eq!(
        &*tree.draw_list_snapshot(node).unwrap().image_textures,
        &[9]
    );
}

#[test]
fn a_rejected_recording_keeps_the_previous_textures() {
    let tree = RenderTree::new();
    let node = tree.add_root(Rect::new(0.0, 0.0, 40.0, 40.0)).unwrap();
    record(&tree, node, vec![image_command(5)]);

    // An unbalanced clip scope is refused, so the old list stays.
    assert!(
        tree.context(node)
            .unwrap()
            .begin_recording()
            .unwrap()
            .commit(vec![
                image_command(6),
                DrawCommand::PushClip {
                    rect: Rect::new(0.0, 0.0, 4.0, 4.0),
                    border_radius: [0.0; 4],
                },
            ])
            .is_err()
    );

    assert_eq!(
        &*tree.draw_list_snapshot(node).unwrap().image_textures,
        &[5]
    );
}

#[test]
fn a_planned_item_carries_its_listed_textures() {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 40.0, 40.0)).unwrap();
    let child = tree
        .add_child(root, Rect::new(0.0, 0.0, 20.0, 20.0))
        .unwrap();
    record(&tree, root, vec![image_command(1)]);
    record(
        &tree,
        child,
        vec![image_command(2), resource_image_command(4)],
    );

    let textures = tree
        .render_all()
        .into_iter()
        .filter_map(|operation| match operation {
            RenderOp::Draw(item) => Some((item.element, item.snapshot().image_textures.to_vec())),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(textures, vec![(root, vec![1]), (child, vec![2, 4])]);
}

fn record_fill(tree: &RenderTree, node: aimer_cupid::draw_cmd_v2::RenderNodeId) {
    tree.context(node)
        .unwrap()
        .begin_recording()
        .unwrap()
        .commit(vec![fill_command(Rect::new(0.0, 0.0, 20.0, 20.0))])
        .unwrap();
}
