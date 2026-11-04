//! A node's own clip can carry corner radii that move with its presentation.
//!
//! Container-style transitions scale a whole page into a rounded card, so the
//! radius must be settable per frame without re-recording and must follow the
//! node's transform the way the clip rectangle does.

use super::*;

fn scene() -> (RenderTree, RenderNodeId) {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
    let child = tree.add_child(root, Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
    (tree, child)
}

fn draw(tree: &RenderTree, id: RenderNodeId) -> RenderItem {
    tree.render_all().into_iter().find_map(|op| match op {
        RenderOp::Draw(item) if item.element == id => Some(item),
        _ => None,
    }).expect("the node must draw")
}

const FULL: Rect = Rect { x: 0.0, y: 0.0, width: 100.0, height: 100.0 };

#[test]
fn a_clip_is_set_together_with_its_radii() {
    let (tree, child) = scene();

    tree.set_clip_with_radius(child, Some(FULL), [8.0, 6.0, 4.0, 2.0]).unwrap();

    let item = draw(&tree, child);
    assert_eq!(item.clip, Some(FULL));
    assert_eq!(item.clip_radius, [8.0, 6.0, 4.0, 2.0]);
}

#[test]
fn radii_scale_with_the_presentation_transform() {
    let (tree, child) = scene();
    tree.set_clip_with_radius(child, Some(FULL), [8.0; 4]).unwrap();

    tree.set_presentation(child, Mat3::scale(0.5, 0.5), 1.0).unwrap();

    let item = draw(&tree, child);
    assert_eq!(item.clip, Some(Rect::new(0.0, 0.0, 50.0, 50.0)));
    assert_eq!(item.clip_radius, [4.0; 4]);
}

#[test]
fn an_unchanged_clip_and_radius_record_no_damage() {
    let (tree, child) = scene();
    tree.set_clip_with_radius(child, Some(FULL), [8.0; 4]).unwrap();
    tree.take_damage();

    tree.set_clip_with_radius(child, Some(FULL), [8.0; 4]).unwrap();

    assert!(tree.take_damage().is_empty());
}

#[test]
fn changing_only_the_radius_damages_the_node() {
    let (tree, child) = scene();
    tree.set_clip_with_radius(child, Some(FULL), [8.0; 4]).unwrap();
    tree.take_damage();

    tree.set_clip_with_radius(child, Some(FULL), [16.0; 4]).unwrap();

    assert!(!tree.take_damage().is_empty());
    assert_eq!(draw(&tree, child).clip_radius, [16.0; 4]);
}

#[test]
fn radii_without_a_clip_are_dropped() {
    let (tree, child) = scene();

    tree.set_clip_with_radius(child, None, [8.0; 4]).unwrap();

    assert_eq!(draw(&tree, child).clip_radius, [0.0; 4]);
}

#[test]
fn invalid_input_is_rejected_and_changes_nothing() {
    let (tree, child) = scene();
    tree.set_clip_with_radius(child, Some(FULL), [8.0; 4]).unwrap();

    for radius in [[-1.0; 4], [f32::NAN; 4], [f32::INFINITY, 0.0, 0.0, 0.0]] {
        assert_eq!(
            tree.set_clip_with_radius(child, Some(FULL), radius),
            Err(RenderTreeError::InvalidBounds)
        );
    }
    assert_eq!(
        tree.set_clip_with_radius(child, Some(Rect::new(f32::NAN, 0.0, 1.0, 1.0)), [0.0; 4]),
        Err(RenderTreeError::InvalidBounds)
    );
    assert_eq!(draw(&tree, child).clip_radius, [8.0; 4]);
}

#[test]
fn the_square_clip_setter_still_clears_radii() {
    let (tree, child) = scene();
    tree.set_clip_with_radius(child, Some(FULL), [8.0; 4]).unwrap();

    tree.set_clip(child, Some(FULL)).unwrap();

    assert_eq!(draw(&tree, child).clip_radius, [0.0; 4]);
}
