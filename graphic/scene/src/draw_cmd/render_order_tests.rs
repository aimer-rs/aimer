//! The paint-order plan caches how element identities map to node positions.
//!
//! The cache is keyed on the tree's topology, so the properties that matter
//! are that an unchanged topology never has to be resolved again, and that
//! every edit which can change parents, children or sibling order is seen on
//! the next plan.

use super::*;

fn drawn(tree: &RenderTree) -> Vec<RenderNodeId> {
    tree.render_all()
        .into_iter()
        .filter_map(|op| match op {
            RenderOp::Draw(item) => Some(item.element),
            _ => None,
        })
        .collect()
}

fn rect(x: f32, y: f32) -> Rect {
    Rect::new(x, y, 20.0, 20.0)
}

/// root -> [a -> [a1, a2], b]
fn sample() -> (RenderTree, [RenderNodeId; 5]) {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 200.0, 200.0)).unwrap();
    let a = tree.add_child(root, rect(0.0, 0.0)).unwrap();
    let a1 = tree.add_child(a, rect(1.0, 1.0)).unwrap();
    let a2 = tree.add_child(a, rect(2.0, 2.0)).unwrap();
    let b = tree.add_child(root, rect(30.0, 0.0)).unwrap();
    (tree, [root, a, a1, a2, b])
}

#[test]
fn nodes_are_planned_parent_first_in_sibling_order() {
    let (tree, [root, a, a1, a2, b]) = sample();
    assert_eq!(drawn(&tree), vec![root, a, a1, a2, b]);
}

#[test]
fn an_unchanged_topology_is_not_resolved_again() {
    let (tree, [root, a, a1, a2, b]) = sample();
    assert_eq!(drawn(&tree), vec![root, a, a1, a2, b]);

    // Dropping the identity index proves the second plan resolves parents and
    // children from the cached positions instead of looking each one up.
    tree.draw_cmd.borrow_mut().indices.clear();

    assert_eq!(drawn(&tree), vec![root, a, a1, a2, b]);
}

#[test]
fn moving_a_node_replans_its_geometry_without_a_topology_change() {
    let (tree, [_, a, a1, _, _]) = sample();
    let before = tree.render_all();
    tree.set_bounds(a, Rect::new(50.0, 40.0, 20.0, 20.0))
        .unwrap();

    let origin_of = |ops: &[RenderOp], id| {
        ops.iter().find_map(|op| match op {
            RenderOp::Draw(item) if item.element == id => Some(item.origin),
            _ => None,
        })
    };
    let after = tree.render_all();
    assert_eq!(origin_of(&before, a1), Some((1.0, 1.0)));
    assert_eq!(origin_of(&after, a1), Some((51.0, 41.0)));
}

#[test]
fn a_node_added_after_a_plan_is_part_of_the_next_one() {
    let (tree, [root, a, a1, a2, b]) = sample();
    assert_eq!(drawn(&tree), vec![root, a, a1, a2, b]);

    let a3 = tree.add_child(a, rect(3.0, 3.0)).unwrap();
    let late_root = tree.add_root(Rect::new(0.0, 0.0, 10.0, 10.0)).unwrap();

    assert_eq!(drawn(&tree), vec![root, a, a1, a2, a3, b, late_root]);
}

#[test]
fn reordering_siblings_through_a_structure_sync_changes_paint_order() {
    let (tree, [root, a, a1, a2, b]) = sample();
    assert_eq!(drawn(&tree), vec![root, a, a1, a2, b]);

    tree.sync_structure(&[
        RenderNodeSpec::new(Some(root), None, Rect::new(0.0, 0.0, 200.0, 200.0)),
        RenderNodeSpec::new(Some(b), Some(0), rect(30.0, 0.0)),
        RenderNodeSpec::new(Some(a), Some(0), rect(0.0, 0.0)),
        RenderNodeSpec::new(Some(a2), Some(2), rect(2.0, 2.0)),
        RenderNodeSpec::new(Some(a1), Some(2), rect(1.0, 1.0)),
    ])
    .unwrap();

    assert_eq!(drawn(&tree), vec![root, b, a, a2, a1]);
}

#[test]
fn reparenting_and_removing_nodes_through_a_structure_sync_is_planned() {
    let (tree, [root, a, a1, a2, b]) = sample();
    assert_eq!(drawn(&tree), vec![root, a, a1, a2, b]);

    // `a2` is dropped, `a1` moves under `b`, and a new node joins `a`.
    let ids = tree
        .sync_structure(&[
            RenderNodeSpec::new(Some(root), None, Rect::new(0.0, 0.0, 200.0, 200.0)),
            RenderNodeSpec::new(Some(a), Some(0), rect(0.0, 0.0)),
            RenderNodeSpec::new(None, Some(1), rect(5.0, 5.0)),
            RenderNodeSpec::new(Some(b), Some(0), rect(30.0, 0.0)),
            RenderNodeSpec::new(Some(a1), Some(3), rect(1.0, 1.0)),
        ])
        .unwrap();

    assert_eq!(drawn(&tree), vec![root, a, ids[2], b, a1]);
}

/// The plan derives each node's world state in one top-down pass; the
/// per-query definitions in `world.rs` derive it independently. They describe
/// the same geometry, so a plan item must land where the query says it does.
#[test]
fn planned_geometry_agrees_with_the_world_queries() {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(3.0, 5.0, 400.0, 300.0)).unwrap();
    let plain = tree
        .add_child(root, Rect::new(7.0, 11.0, 90.0, 60.0))
        .unwrap();
    let scaled = tree
        .add_child(root, Rect::new(120.0, 40.0, 80.0, 50.0))
        .unwrap();
    let below_scaled = tree
        .add_child(scaled, Rect::new(4.0, 6.0, 30.0, 20.0))
        .unwrap();
    let presented = tree
        .add_child(plain, Rect::new(2.0, 3.0, 40.0, 40.0))
        .unwrap();
    let animated = tree
        .add_child(presented, Rect::new(1.0, 1.0, 20.0, 20.0))
        .unwrap();
    {
        let mut inner = tree.draw_cmd.borrow_mut();
        inner.node_mut(scaled).unwrap().transform = Mat3::scale(1.5, 0.75);
        inner.node_mut(presented).unwrap().presentation_transform = Mat3::translate(9.0, -4.0);
        inner.node_mut(animated).unwrap().animation_clip = Some(Rect::new(0.0, 0.0, 10.0, 10.0));
        inner.node_mut(plain).unwrap().clip = Some(Rect::new(0.0, 0.0, 60.0, 40.0));
    }

    let inner = tree.draw_cmd.borrow();
    for id in [root, plain, scaled, below_scaled, presented, animated] {
        let item = tree
            .render_all()
            .into_iter()
            .find_map(|op| match op {
                RenderOp::Draw(item) if item.element == id => Some(item),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{id:?} was not planned"));
        let expected = inner.world_bounds(id).unwrap();
        let tolerance = 1e-3;
        for (planned, queried, what) in [
            (item.bounds.x, expected.x, "x"),
            (item.bounds.y, expected.y, "y"),
            (item.bounds.width, expected.width, "width"),
            (item.bounds.height, expected.height, "height"),
        ] {
            assert!(
                (planned - queried).abs() <= tolerance,
                "{id:?} {what}: planned {planned}, queried {queried}"
            );
        }
    }
}

/// The original mapping of a rectangle through a transform, kept as the
/// specification the identity shortcut has to reproduce exactly.
fn specified_transform_rect(transform: Mat3, rect: Rect) -> Option<Rect> {
    let corners = [
        transform.transform_point(rect.x, rect.y),
        transform.transform_point(rect.x + rect.width, rect.y),
        transform.transform_point(rect.x, rect.y + rect.height),
        transform.transform_point(rect.x + rect.width, rect.y + rect.height),
    ];
    let (mut min_x, mut max_x) = (f32::INFINITY, f32::NEG_INFINITY);
    let (mut min_y, mut max_y) = (f32::INFINITY, f32::NEG_INFINITY);
    for (x, y) in corners {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }
    let bounds = Rect::new(min_x, min_y, max_x - min_x, max_y - min_y);
    bounds.is_valid().then_some(bounds)
}

fn same_rect(left: Option<Rect>, right: Option<Rect>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            left.x == right.x
                && left.y == right.y
                && left.width == right.width
                && left.height == right.height
        }
        _ => false,
    }
}

#[test]
fn mapping_a_rect_through_the_identity_changes_nothing_in_any_case() {
    let values = [
        0.0,
        -0.0,
        1.0,
        -1.0,
        0.1,
        3.75,
        -250.5,
        1234.567,
        1e7,
        -1e7,
        f32::MAX,
        f32::MIN,
        f32::MIN_POSITIVE,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NAN,
    ];
    for x in values {
        for y in values {
            for width in values {
                for height in values {
                    let rect = Rect::new(x, y, width, height);
                    assert!(
                        same_rect(
                            transform_rect(Mat3::identity(), rect),
                            specified_transform_rect(Mat3::identity(), rect)
                        ),
                        "rect {rect:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn mapping_through_other_transforms_is_untouched() {
    let rect = Rect::new(3.0, 4.0, 50.0, 20.0);
    for transform in [
        Mat3::translate(5.0, -7.0),
        Mat3::scale(2.0, 0.5),
        Mat3::rotate(0.7),
        Mat3::translate(1.0, 1.0).mul(&Mat3::scale(3.0, 3.0)),
    ] {
        assert!(same_rect(
            transform_rect(transform, rect),
            specified_transform_rect(transform, rect)
        ));
    }
}

#[test]
fn a_tree_without_transforms_is_planned_without_general_rect_mapping() {
    // `untransformed_tree` sprinkles a few transforms on purpose; clear them so
    // the plan sees the common case: not one node moves itself.
    let (tree, ids) = {
        let (tree, ids) = super::world_tests::untransformed_tree(9, 150);
        {
            let mut inner = tree.draw_cmd.borrow_mut();
            for id in &ids {
                let node = inner.node_mut(*id).unwrap();
                node.transform = Mat3::identity();
                node.presentation_transform = Mat3::identity();
            }
        }
        (tree, ids)
    };
    assert!(ids.len() > 100);

    reset_node_lookups();
    let plan = tree.render_all();

    assert!(!plan.is_empty());
    assert_eq!(
        general_rect_transforms(),
        0,
        "the identity case went through the general mapping"
    );
}

/// The plan derives each node's geometry in one top-down pass and the world
/// queries derive it separately; on whole trees, with every kind of transform
/// and clip, they must describe the same geometry.
#[test]
fn planned_items_land_where_the_world_queries_put_them_on_whole_trees() {
    for (seed, untransformed) in [
        (1, false),
        (8, false),
        (64, true),
        (512, true),
        (4096, true),
    ] {
        let (tree, ids) = if untransformed {
            super::world_tests::untransformed_tree(seed, 120)
        } else {
            super::world_tests::varied_tree(seed, 80)
        };
        let inner = tree.draw_cmd.borrow();
        let mut drawn = 0;
        for operation in tree.render_all() {
            let RenderOp::Draw(item) = operation else {
                continue;
            };
            drawn += 1;
            let queried = inner.world_bounds(item.element).unwrap_or_else(|| {
                panic!("{:?} was planned but has no world bounds", item.element)
            });
            for (planned, expected, what) in [
                (item.bounds.x, queried.x, "x"),
                (item.bounds.y, queried.y, "y"),
                (item.bounds.width, queried.width, "width"),
                (item.bounds.height, queried.height, "height"),
            ] {
                assert!(
                    (planned - expected).abs() <= 1e-2 * (1.0 + expected.abs()),
                    "seed {seed} {:?} {what}: planned {planned}, queried {expected}",
                    item.element
                );
            }
        }
        assert!(
            drawn > 0,
            "seed {seed} drew nothing out of {} nodes",
            ids.len()
        );
    }
}

fn describe(operation: &RenderOp) -> String {
    match operation {
        RenderOp::Draw(item) => format!(
            "{:?} {:?} {:?} {:?} {:?} {:?} {} {:?} rev{}",
            item.element,
            item.bounds,
            item.origin,
            item.transform,
            item.clip,
            item.clip_radius,
            item.opacity,
            item.paint_source,
            item.snapshot().revision,
        ),
        RenderOp::BeginOpacityGroup {
            element,
            bounds,
            opacity,
            clip,
            masked,
            ..
        } => format!("begin {element:?} {bounds:?} {opacity} {clip:?} {masked}"),
        RenderOp::EndOpacityGroup { element } => format!("end {element:?}"),
    }
}

fn described(plan: Vec<RenderOp>) -> Vec<String> {
    plan.iter().map(describe).collect()
}

#[test]
fn paint_only_edits_leave_the_geometry_revision_alone() {
    let (tree, [_, a, a1, _, _]) = sample();
    let before = tree.geometry_revision();

    tree.invalidate_paint(a1).unwrap();
    tree.set_paint_source(a, RenderPaintSource::LocalV2)
        .unwrap();
    tree.set_opacity(a, 0.5).unwrap();

    assert_eq!(
        tree.geometry_revision(),
        before,
        "paint cannot move anything"
    );
}

#[test]
fn geometry_is_derived_again_exactly_when_it_can_have_changed() {
    let (tree, [root, a, a1, _, b]) = sample();
    let mut total = 0;
    let mut expect_run = |what: &str, tree: &RenderTree, runs: bool| {
        reset_node_lookups();
        tree.render_all();
        let ran = plan_geometry_runs();
        total += ran;
        assert_eq!(ran, usize::from(runs), "{what}");
    };

    expect_run("the first plan derives it", &tree, true);
    expect_run("an unchanged tree reuses it", &tree, false);
    expect_run("planning again still reuses it", &tree, false);

    tree.invalidate_paint(a1).unwrap();
    expect_run("a repaint does not move anything", &tree, false);
    tree.set_opacity(a, 0.5).unwrap();
    expect_run("an opacity change does not move anything", &tree, false);
    tree.set_paint_source(b, RenderPaintSource::LocalV2)
        .unwrap();
    expect_run("a paint source does not move anything", &tree, false);

    tree.set_bounds(a, Rect::new(5.0, 5.0, 25.0, 25.0)).unwrap();
    expect_run("moving a node derives it again", &tree, true);
    expect_run("and then it is reused again", &tree, false);
    tree.set_clip(a, Some(Rect::new(0.0, 0.0, 10.0, 10.0)))
        .unwrap();
    expect_run("a clip derives it again", &tree, true);
    tree.set_transform(a, Mat3::translate(3.0, 0.0)).unwrap();
    expect_run("a transform derives it again", &tree, true);
    tree.set_compositor_animation(a, Mat3::translate(1.0, 1.0), 1.0, None)
        .unwrap();
    expect_run("an animation derives it again", &tree, true);
    tree.set_geometry(a1, Rect::new(2.0, 2.0, 20.0, 20.0), None)
        .unwrap();
    expect_run("set_geometry derives it again", &tree, true);

    tree.add_child(root, rect(60.0, 60.0)).unwrap();
    expect_run("a new node derives it again", &tree, true);
    tree.sync_structure(&[
        RenderNodeSpec::new(Some(root), None, Rect::new(0.0, 0.0, 200.0, 200.0)),
        RenderNodeSpec::new(Some(b), Some(0), rect(30.0, 0.0)),
    ])
    .unwrap();
    expect_run("a structure sync derives it again", &tree, true);

    assert!(total >= 8);
}

/// After any sequence of edits, the plan that reuses geometry must be the plan
/// that derives it afresh.
#[test]
fn a_reused_plan_always_equals_a_plan_derived_from_scratch() {
    for seed in [3, 19, 144, 2_718] {
        let (tree, ids) = super::world_tests::untransformed_tree(seed, 50);
        let mut rng = super::world_tests::Lcg(seed ^ 0x5eed);
        assert_eq!(
            described(tree.render_all()),
            described(tree.render_all_from_scratch())
        );

        for step in 0..60 {
            let id = ids[(rng.next() as usize) % ids.len()];
            let edit = match rng.next() % 9 {
                0 => {
                    let _ = tree.set_bounds(
                        id,
                        Rect::new(
                            rng.range(-30.0, 60.0),
                            rng.range(-30.0, 60.0),
                            rng.range(1.0, 150.0),
                            rng.range(1.0, 150.0),
                        ),
                    );
                    "set_bounds"
                }
                1 => {
                    let _ = tree.set_transform(id, super::world_tests::random_transform(&mut rng));
                    "set_transform"
                }
                2 => {
                    let clip = rng
                        .chance(60)
                        .then(|| super::world_tests::random_clip(&mut rng));
                    let _ = tree.set_clip(id, clip);
                    "set_clip"
                }
                3 => {
                    let _ = tree.set_opacity(id, rng.range(0.0, 1.0));
                    "set_opacity"
                }
                4 => {
                    let _ = tree.invalidate_paint(id);
                    "invalidate_paint"
                }
                5 => {
                    let _ = tree.set_paint_source(id, RenderPaintSource::LocalV2);
                    "set_paint_source"
                }
                6 => {
                    let _ = tree.set_compositor_animation(
                        id,
                        super::world_tests::random_transform(&mut rng),
                        rng.range(0.0, 1.0),
                        rng.chance(40)
                            .then(|| super::world_tests::random_clip(&mut rng)),
                    );
                    "set_compositor_animation"
                }
                7 => {
                    let _ = tree.add_child(id, Rect::new(1.0, 2.0, 30.0, 30.0));
                    "add_child"
                }
                _ => {
                    let _ = tree.set_geometry(
                        id,
                        Rect::new(rng.range(0.0, 50.0), rng.range(0.0, 50.0), 40.0, 40.0),
                        rng.chance(50)
                            .then(|| super::world_tests::random_clip(&mut rng)),
                    );
                    "set_geometry"
                }
            };
            let reused = described(tree.render_all());
            let fresh = described(tree.render_all_from_scratch());
            assert_eq!(reused, fresh, "seed {seed} step {step} after {edit}");
        }
    }
}
