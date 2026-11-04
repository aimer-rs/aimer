use super::*;

fn scene() -> (RenderTree, RenderNodeId, RenderNodeId, RenderNodeId, RenderNodeId) {
    let tree = RenderTree::new();
    let viewport = tree.add_root(Rect::new(0.0, 0.0, 120.0, 60.0)).unwrap();
    tree.set_clip(viewport, Some(Rect::new(0.0, 0.0, 120.0, 60.0))).unwrap();
    let container = tree.add_child(viewport, Rect::new(0.0, 0.0, 100.0, 40.0)).unwrap();
    tree.set_clip(container, Some(Rect::new(0.0, 0.0, 100.0, 40.0))).unwrap();
    let scalable = tree.add_child(container, Rect::new(0.0, 0.0, 100.0, 40.0)).unwrap();
    tree.set_clip_escape(scalable, true).unwrap();
    let child = tree.add_child(scalable, Rect::new(0.0, 0.0, 100.0, 40.0)).unwrap();
    tree.set_presentation(child, Mat3::scale(2.0, 2.0), 1.0).unwrap();
    (tree, viewport, container, scalable, child)
}

fn draw(tree: &RenderTree, id: RenderNodeId) -> RenderItem {
    tree.render_all().into_iter().find_map(|op| match op {
        RenderOp::Draw(item) if item.element == id => Some(item),
        _ => None,
    }).expect("the node must draw")
}

#[test]
fn escaping_ancestor_clips_preserves_descendant_clips_and_sibling_clipping() {
    let (tree, _, container, _, child) = scene();
    assert_eq!(draw(&tree, child).clip, None);
    let sibling = tree.add_child(container, Rect::new(0.0, 0.0, 200.0, 80.0)).unwrap();
    assert_eq!(draw(&tree, sibling).clip, Some(Rect::new(0.0, 0.0, 100.0, 40.0)));

    // An explicit ancestor clip is escaped, but a clip inside the scaled
    // subtree remains local to that descendant.
    tree.set_clip(container, Some(Rect::new(0.0, 0.0, 110.0, 50.0))).unwrap();
    assert_eq!(draw(&tree, child).clip, None);
    tree.set_clip(child, Some(Rect::new(0.0, 0.0, 20.0, 10.0))).unwrap();
    assert_eq!(draw(&tree, child).clip, Some(Rect::new(0.0, 0.0, 40.0, 20.0)));
}

#[test]
fn an_empty_container_clip_does_not_hide_overflowing_descendants() {
    let (tree, _, container, _, child) = scene();
    tree.set_clip(container, Some(Rect::new(0.0, 0.0, 0.0, 0.0))).unwrap();
    assert_eq!(draw(&tree, child).clip, None);
}

#[test]
fn overflow_damage_covers_pixels_beyond_the_container() {
    let (tree, _, _, scalable, child) = scene();
    tree.take_damage();
    tree.set_presentation(child, Mat3::identity(), 1.0).unwrap();
    let damage = tree.take_damage().into_iter().reduce(Rect::union).unwrap();
    assert_eq!(damage, Rect::new(0.0, 0.0, 200.0, 80.0));
    tree.set_presentation(child, Mat3::scale(2.0, 2.0), 1.0).unwrap();
    tree.take_damage();
    tree.set_clip_escape(scalable, false).unwrap();
    let damage = tree.take_damage().into_iter().reduce(Rect::union).unwrap();
    assert_eq!(damage, Rect::new(0.0, 0.0, 200.0, 80.0));
}

#[test]
fn opacity_groups_do_not_reapply_escaped_container_clips() {
    let (tree, _, container, _, _) = scene();
    tree.set_opacity(container, 0.5).unwrap();
    let group_clip = tree.render_all().into_iter().find_map(|op| match op {
        RenderOp::BeginOpacityGroup { element, clip, .. } if element == container => Some(clip),
        _ => None,
    });
    assert_eq!(group_clip, Some(None));
}

#[test]
fn repeated_policies_preserve_geometry_revision_and_damage() {
    let (tree, _, container, scalable, _) = scene();
    tree.take_damage();
    let revision = tree.geometry_revision();
    for _ in 0..32 {
        tree.set_clip_escape(scalable, true).unwrap();
    }
    assert_eq!(tree.geometry_revision(), revision);
    assert!(tree.take_damage().is_empty());
    assert_eq!(tree.set_clip(container, Some(Rect::new(f32::NAN, 0.0, 1.0, 1.0))),
        Err(RenderTreeError::InvalidBounds));
}
