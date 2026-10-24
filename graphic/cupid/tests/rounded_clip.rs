//! Rounded clips on retained render nodes.
//!
//! The renderer keeps one clip at a time: nested clips intersect their
//! rectangles, but the corner radii come from the innermost clip that was
//! pushed. A node's effective radius therefore follows its innermost
//! clip-owning ancestor (itself included), and a clip owner with square
//! corners resets it.

use aimer_cupid::draw_cmd_v2::{Rect, RenderNodeId, RenderNodeSpec, RenderOp, RenderTree};

struct Fixture {
    tree: RenderTree,
    root: RenderNodeId,
    card: RenderNodeId,
    inherits: RenderNodeId,
    square: RenderNodeId,
}

/// root -> card (rounded clip) -> { inherits (no clip), square (own square clip) }
fn fixture(card_radius: [f32; 4]) -> Fixture {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
    let card = tree
        .add_child(root, Rect::new(10.0, 10.0, 80.0, 80.0))
        .unwrap();
    let inherits = tree
        .add_child(card, Rect::new(5.0, 5.0, 20.0, 20.0))
        .unwrap();
    let square = tree
        .add_child(card, Rect::new(30.0, 5.0, 20.0, 20.0))
        .unwrap();
    let fixture = Fixture {
        tree,
        root,
        card,
        inherits,
        square,
    };
    fixture.sync(card_radius);
    fixture
}

impl Fixture {
    fn sync(&self, card_radius: [f32; 4]) {
        self.tree
            .sync_structure_with_clip_radii(
                &[
                    RenderNodeSpec::new(Some(self.root), None, Rect::new(0.0, 0.0, 100.0, 100.0)),
                    RenderNodeSpec::new(
                        Some(self.card),
                        Some(0),
                        Rect::new(10.0, 10.0, 80.0, 80.0),
                    ),
                    RenderNodeSpec::new(
                        Some(self.inherits),
                        Some(1),
                        Rect::new(5.0, 5.0, 20.0, 20.0),
                    ),
                    RenderNodeSpec::new(
                        Some(self.square),
                        Some(1),
                        Rect::new(30.0, 5.0, 20.0, 20.0),
                    ),
                ],
                &[
                    None,
                    Some(Rect::new(0.0, 0.0, 80.0, 80.0)),
                    None,
                    Some(Rect::new(0.0, 0.0, 20.0, 20.0)),
                ],
                &[[0.0; 4], card_radius, [0.0; 4], [0.0; 4]],
            )
            .unwrap();
    }

    fn radius_of(&self, node: RenderNodeId) -> [f32; 4] {
        self.tree
            .render_all()
            .into_iter()
            .find_map(|operation| match operation {
                RenderOp::Draw(item) if item.element == node => Some(item.clip_radius),
                _ => None,
            })
            .expect("node is visible")
    }
}

#[test]
fn a_node_without_a_clip_has_square_corners() {
    let fixture = fixture([8.0; 4]);
    assert_eq!(fixture.radius_of(fixture.root), [0.0; 4]);
}

#[test]
fn the_clip_owner_and_its_unclipped_descendants_share_its_radius() {
    let fixture = fixture([8.0, 6.0, 4.0, 2.0]);
    assert_eq!(fixture.radius_of(fixture.card), [8.0, 6.0, 4.0, 2.0]);
    assert_eq!(fixture.radius_of(fixture.inherits), [8.0, 6.0, 4.0, 2.0]);
}

#[test]
fn a_nested_square_clip_resets_the_radius_like_the_renderer_does() {
    let fixture = fixture([8.0; 4]);
    assert_eq!(fixture.radius_of(fixture.square), [0.0; 4]);
}

#[test]
fn changing_only_the_radius_damages_the_clipped_subtree() {
    let fixture = fixture([8.0; 4]);
    fixture.tree.take_damage();

    fixture.sync([8.0; 4]);
    assert!(
        fixture.tree.take_damage().is_empty(),
        "an unchanged radius must not damage anything"
    );

    fixture.sync([16.0; 4]);
    assert!(
        !fixture.tree.take_damage().is_empty(),
        "a radius change alters the corner pixels and must be damaged"
    );
}

#[test]
fn radii_must_align_with_the_node_list() {
    let fixture = fixture([8.0; 4]);
    assert!(
        fixture
            .tree
            .sync_structure_with_clip_radii(
                &[RenderNodeSpec::new(
                    Some(fixture.root),
                    None,
                    Rect::new(0.0, 0.0, 100.0, 100.0)
                )],
                &[None],
                &[],
            )
            .is_err()
    );
}

#[test]
fn invalid_radii_are_rejected() {
    let fixture = fixture([8.0; 4]);
    let spec = [RenderNodeSpec::new(
        Some(fixture.root),
        None,
        Rect::new(0.0, 0.0, 100.0, 100.0),
    )];
    for radius in [
        [-1.0, 0.0, 0.0, 0.0],
        [f32::NAN, 0.0, 0.0, 0.0],
        [0.0, f32::INFINITY, 0.0, 0.0],
    ] {
        assert!(
            fixture
                .tree
                .sync_structure_with_clip_radii(&spec, &[None], &[radius])
                .is_err(),
            "{radius:?} must be rejected"
        );
    }
}
