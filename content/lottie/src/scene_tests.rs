use std::sync::Arc;

use aimer_cupid::svg::{SvgColor, SvgFillRule, SvgLineCap, SvgNodeStyleOverride, SvgPathCommand};

use crate::{Block, Composition, LottieScene};

fn wrap(layers: &str) -> String {
    format!(r#"{{"fr":30,"ip":0,"op":60,"w":100,"h":80,"layers":[{layers}]}}"#)
}

fn scene(layers: &str) -> LottieScene {
    LottieScene::new(Composition::parse(wrap(layers)).expect("valid document"))
}

const RECT: &str = r#"{"ty":"rc","p":{"a":0,"k":[0,0]},"s":{"a":0,"k":[20,10]},"r":{"a":0,"k":0}}"#;
const ELLIPSE: &str = r#"{"ty":"el","p":{"a":0,"k":[30,0]},"s":{"a":0,"k":[8,8]}}"#;
const RED: &str = r#"{"ty":"fl","c":{"a":0,"k":[1,0,0,1]},"o":{"a":0,"k":100}}"#;
const STROKE: &str = r#"{"ty":"st","c":{"a":0,"k":[0,0,1,1]},"o":{"a":0,"k":100},"w":{"a":0,"k":2},"lc":2}"#;

/// A shape layer with `ks` whose single group holds `items`.
fn layer(ks: &str, group_tr: &str, items: &str) -> String {
    format!(
        r#"{{"ty":4,"nm":"L","ks":{ks},"shapes":[{{"ty":"gr","it":[{items},{{"ty":"tr"{group_tr}}}]}}]}}"#
    )
}

fn pos(x: f32, y: f32) -> String {
    format!(r#"{{"p":{{"a":0,"k":[{x},{y}]}}}}"#)
}

fn find(overrides: &[SvgNodeStyleOverride], node: u32) -> Option<&SvgNodeStyleOverride> {
    overrides.iter().find(|o| o.node_id.0 == node)
}

#[test]
fn rect_fill_becomes_one_path_node() {
    let s = scene(&layer("{}", "", &format!("{RECT},{RED}")));
    let f = s.frame(0.0);
    assert_eq!((f.scene.viewport.width, f.scene.viewport.height), (100.0, 80.0));
    assert_eq!(f.scene.nodes.len(), 1);
    assert_eq!(f.scene.geometries.len(), 1);
    let node = &f.scene.nodes[0];
    assert_eq!(node.geometry, Some(0));
    assert!(node.stroke.is_none());
    let fill = node.fill.as_ref().expect("a fill");
    assert_eq!(fill.color, SvgColor { r: 1.0, g: 0.0, b: 0.0, a: 1.0 });
    assert_eq!(fill.rule, SvgFillRule::NonZero);
    assert_eq!(
        &*f.scene.geometries[0].commands,
        [
            SvgPathCommand::MoveTo { x: -10.0, y: -5.0 },
            SvgPathCommand::LineTo { x: 10.0, y: -5.0 },
            SvgPathCommand::LineTo { x: 10.0, y: 5.0 },
            SvgPathCommand::LineTo { x: -10.0, y: 5.0 },
            SvgPathCommand::Close,
        ]
    );
    assert!(f.overrides.is_empty(), "nothing differs from the base node: {:?}", f.overrides);
}

#[test]
fn stroke_node_carries_width_cap_and_color() {
    let s = scene(&layer("{}", "", &format!("{RECT},{STROKE}")));
    let node = &s.frame(0.0).scene.nodes[0];
    assert!(node.fill.is_none());
    let stroke = node.stroke.as_ref().expect("a stroke");
    assert_eq!(stroke.width, 2.0);
    assert_eq!(stroke.line_cap, SvgLineCap::Round);
    assert_eq!(stroke.color, SvgColor { r: 0.0, g: 0.0, b: 1.0, a: 1.0 });
}

#[test]
fn a_painter_lower_in_the_list_is_drawn_first() {
    // [rect, fill, stroke]: the fill sits above the stroke, so the stroke is
    // drawn first. The renderer draws nodes in index order.
    let f = scene(&layer("{}", "", &format!("{RECT},{RED},{STROKE}"))).frame(0.0);
    assert_eq!(f.scene.nodes.len(), 2);
    assert!(f.scene.nodes[0].stroke.is_some() && f.scene.nodes[0].fill.is_none());
    assert!(f.scene.nodes[1].fill.is_some() && f.scene.nodes[1].stroke.is_none());
}

#[test]
fn shapes_above_one_painter_merge_into_one_path() {
    let f = scene(&layer("{}", "", &format!("{RECT},{ELLIPSE},{RED}"))).frame(0.0);
    assert_eq!(f.scene.nodes.len(), 1, "one painter is one node");
    let moves = f.scene.geometries[0]
        .commands
        .iter()
        .filter(|c| matches!(c, SvgPathCommand::MoveTo { .. }))
        .count();
    assert_eq!(moves, 2, "two subpaths in one geometry");
}

#[test]
fn a_painter_only_covers_the_shapes_above_it() {
    // [rect, fill, ellipse]: the fill does not reach the ellipse below it.
    let f = scene(&layer("{}", "", &format!("{RECT},{RED},{ELLIPSE}"))).frame(0.0);
    assert_eq!(f.scene.nodes.len(), 1);
    assert_eq!(f.scene.geometries[0].commands.len(), 5, "rect only");
}

#[test]
fn painters_without_shapes_draw_nothing() {
    let f = scene(&layer("{}", "", RED)).frame(0.0);
    assert!(f.scene.nodes.is_empty());
}

#[test]
fn layers_are_drawn_bottom_first() {
    // File order puts the top layer first, so it must get the highest node index.
    let top = layer("{}", "", &format!("{RECT},{RED}"));
    let bottom = layer("{}", "", &format!("{RECT},{STROKE}"));
    let f = scene(&format!("{top},{bottom}")).frame(0.0);
    assert!(f.scene.nodes[0].stroke.is_some(), "bottom layer first");
    assert!(f.scene.nodes[1].fill.is_some(), "top layer last");
}

#[test]
fn layer_and_group_transforms_compose_into_each_leaf() {
    let s = scene(&layer(&pos(10.0, 0.0), r#","p":{"a":0,"k":[5,0]}"#, &format!("{RECT},{RED}")));
    let f = s.frame(0.0);
    let o = find(&f.overrides, 0).expect("the leaf moved");
    let t = o.transform.expect("transform override");
    assert_eq!((t.sx, t.sy, t.tx, t.ty), (1.0, 1.0, 15.0, 0.0));
}

#[test]
fn sibling_groups_keep_their_own_transforms() {
    let g = |x: f32| {
        format!(
            r#"{{"ty":"gr","it":[{RECT},{RED},{{"ty":"tr","p":{{"a":0,"k":[{x},0]}}}}]}}"#
        )
    };
    let layer = format!(r#"{{"ty":4,"nm":"L","ks":{{}},"shapes":[{},{}]}}"#, g(1.0), g(2.0));
    let f = scene(&layer).frame(0.0);
    // The later group is drawn first.
    let tx = |node| find(&f.overrides, node).and_then(|o| o.transform).map(|t| t.tx);
    assert_eq!(tx(0), Some(2.0));
    assert_eq!(tx(1), Some(1.0));
}

#[test]
fn nested_groups_multiply() {
    let inner = format!(r#"{{"ty":"gr","it":[{RECT},{RED},{{"ty":"tr","p":{{"a":0,"k":[3,0]}}}}]}}"#);
    let layer = format!(
        r#"{{"ty":4,"nm":"L","ks":{},"shapes":[{{"ty":"gr","it":[{inner},{{"ty":"tr","p":{{"a":0,"k":[4,0]}}}}]}}]}}"#,
        pos(1.0, 0.0)
    );
    let f = scene(&layer).frame(0.0);
    assert_eq!(f.overrides[0].transform.map(|t| t.tx), Some(8.0));
}

#[test]
fn opacity_multiplies_down_the_tree_and_fill_opacity_goes_into_the_color() {
    let ks = r#"{"o":{"a":0,"k":50}}"#;
    let tr = r#","o":{"a":0,"k":50}"#;
    let fill = r#"{"ty":"fl","c":{"a":0,"k":[1,0,0,1]},"o":{"a":0,"k":40}}"#;
    let f = scene(&layer(ks, tr, &format!("{RECT},{fill}"))).frame(0.0);
    // Fill opacity is part of the node's base color; layer and group opacity
    // multiply into the node opacity.
    let base = f.scene.nodes[0].fill.as_ref().expect("fill").color;
    assert!((base.a - 0.4).abs() < 1e-6);
    let o = find(&f.overrides, 0).expect("override");
    assert!((o.opacity.expect("opacity") - 0.25).abs() < 1e-6);
    assert!(o.fill.is_none(), "the color already matches the node");
}

#[test]
fn animated_transform_changes_overrides_but_reuses_the_scene() {
    let ks = r#"{"p":{"a":1,"k":[{"t":0,"s":[0,0]},{"t":10,"s":[100,0]}]}}"#;
    let s = scene(&layer(ks, "", &format!("{RECT},{RED}")));
    let (a, b) = (s.frame(0.0), s.frame(5.0));
    assert!(Arc::ptr_eq(&a.scene, &b.scene), "static geometry must not rebuild the scene");
    assert!(a.overrides.is_empty(), "frame 0 matches the base node");
    assert_eq!(b.overrides[0].transform.map(|t| t.tx), Some(50.0));
}

#[test]
fn layers_outside_their_frame_range_are_skipped_by_zero_opacity() {
    let l = r#"{"ty":4,"nm":"L","ip":10,"op":20,"ks":{},"shapes":[{"ty":"gr","it":[RECT,RED]}]}"#
        .replace("RECT", RECT)
        .replace("RED", RED);
    let s = scene(&l);
    assert_eq!(s.frame(5.0).overrides[0].opacity, Some(0.0));
    assert!(s.frame(15.0).overrides.is_empty());
    assert_eq!(s.frame(25.0).overrides[0].opacity, Some(0.0));
}

#[test]
fn animated_fill_color_is_an_override() {
    let fill = r#"{"ty":"fl","c":{"a":1,"k":[{"t":0,"s":[0,0,0,1]},{"t":10,"s":[1,1,1,1]}]},"o":{"a":0,"k":100}}"#;
    let s = scene(&layer("{}", "", &format!("{RECT},{fill}")));
    assert!(s.frame(0.0).overrides.is_empty());
    let color = s.frame(5.0).overrides[0].fill.flatten().expect("color");
    assert!((color.r - 0.5).abs() < 1e-6 && color.a == 1.0);
}

#[test]
fn animated_geometry_rebuilds_only_what_changed() {
    let rect = r#"{"ty":"rc","p":{"a":0,"k":[0,0]},
        "s":{"a":1,"k":[{"t":0,"s":[10,10]},{"t":10,"s":[30,10]}]},"r":{"a":0,"k":0}}"#;
    let s = scene(&layer("{}", "", &format!("{rect},{RED}")));
    let (a, b) = (s.frame(0.0), s.frame(10.0));
    assert!(!Arc::ptr_eq(&a.scene, &b.scene));
    assert!(Arc::ptr_eq(&a.scene.nodes, &b.scene.nodes), "nodes are shared when only geometry moves");
    let right = |f: &crate::LottieFrame| match f.scene.geometries[0].commands[1] {
        SvgPathCommand::LineTo { x, .. } => x,
        other => panic!("{other:?}"),
    };
    assert_eq!((right(&a), right(&b)), (5.0, 15.0));
}

#[test]
fn animated_stroke_width_rebuilds_the_nodes() {
    let st = r#"{"ty":"st","c":{"a":0,"k":[0,0,1,1]},"o":{"a":0,"k":100},
        "w":{"a":1,"k":[{"t":0,"s":[2]},{"t":10,"s":[10]}]}}"#;
    let s = scene(&layer("{}", "", &format!("{RECT},{st}")));
    let width = |f: f32| s.frame(f).scene.nodes[0].stroke.as_ref().map(|s| s.width);
    assert_eq!(width(0.0), Some(2.0));
    assert_eq!(width(5.0), Some(6.0));
}

fn trim_json(end: &str) -> String {
    format!(r#"{{"ty":"tm","s":{{"a":0,"k":0}},"e":{end},"o":{{"a":0,"k":0}},"m":1}}"#)
}

fn static_trim(percent: u32) -> String {
    trim_json(&format!(r#"{{"a":0,"k":{percent}}}"#))
}

/// Commands of the first geometry as `(x, y)` points, with `None` for a close.
fn points(f: &crate::LottieFrame, i: usize) -> Vec<Option<(f32, f32)>> {
    f.scene.geometries[i]
        .commands
        .iter()
        .map(|c| match *c {
            SvgPathCommand::MoveTo { x, y } | SvgPathCommand::LineTo { x, y } => Some((x, y)),
            SvgPathCommand::Close => None,
            other => panic!("unexpected {other:?}"),
        })
        .collect()
}

#[test]
fn a_trim_cuts_the_shapes_above_it() {
    // The 20 x 10 rectangle has a perimeter of 60; half of it is 30 units:
    // the top edge and the right edge.
    let f = scene(&layer("{}", "", &format!("{RECT},{},{STROKE}", static_trim(50)))).frame(0.0);
    assert_eq!(
        points(&f, 0),
        [Some((-10.0, -5.0)), Some((10.0, -5.0)), Some((10.0, 5.0))],
        "an open path, not closed"
    );
}

#[test]
fn trimming_everything_keeps_the_path_closed() {
    let f = scene(&layer("{}", "", &format!("{RECT},{},{STROKE}", static_trim(100)))).frame(0.0);
    assert_eq!(points(&f, 0).last(), Some(&None), "a whole path keeps its close");
}

#[test]
fn a_trim_that_shows_nothing_leaves_an_empty_geometry() {
    let f = scene(&layer("{}", "", &format!("{RECT},{},{STROKE}", static_trim(0)))).frame(0.0);
    assert!(f.scene.geometries[0].commands.is_empty());
}

#[test]
fn a_trim_below_the_painter_does_not_apply() {
    let f = scene(&layer("{}", "", &format!("{RECT},{STROKE},{}", static_trim(50)))).frame(0.0);
    assert_eq!(f.scene.geometries[0].commands.len(), 5, "the full rectangle");
}

#[test]
fn only_the_shapes_above_a_trim_are_trimmed() {
    // [rect, trim, ellipse, stroke]: the stroke covers both, the trim only the rect.
    let items = format!("{RECT},{},{ELLIPSE},{STROKE}", static_trim(50));
    let f = scene(&layer("{}", "", &items)).frame(0.0);
    let commands = &f.scene.geometries[0].commands;
    assert_eq!(f.scene.nodes.len(), 1);
    let moves = commands.iter().filter(|c| matches!(c, SvgPathCommand::MoveTo { .. })).count();
    assert_eq!(moves, 2, "the cut rectangle and the whole ellipse");
    assert_eq!(commands.last(), Some(&SvgPathCommand::Close), "the ellipse is whole and closed");
    // The cut rectangle is three commands (move, two lines), then the ellipse's
    // five (move, four arcs ... close) begin; an uncut rectangle would be five.
    assert!(matches!(commands[3], SvgPathCommand::MoveTo { .. }), "{commands:?}");
    assert!(matches!(commands[2], SvgPathCommand::LineTo { x, y } if (x, y) == (10.0, 5.0)));
}

#[test]
fn an_animated_trim_rebuilds_only_the_geometry() {
    let end = r#"{"a":1,"k":[{"t":0,"s":[0]},{"t":10,"s":[100]}]}"#;
    let s = scene(&layer("{}", "", &format!("{RECT},{},{STROKE}", trim_json(end))));
    let (early, late, done) = (s.frame(2.0), s.frame(7.0), s.frame(10.0));
    assert!(!Arc::ptr_eq(&early.scene, &late.scene));
    assert!(Arc::ptr_eq(&early.scene.nodes, &late.scene.nodes), "nodes are shared");
    let (a, b) = (points(&early, 0).len(), points(&late, 0).len());
    assert!(a < b, "more of the path shows later: {a} vs {b}");
    assert_eq!(points(&done, 0).last(), Some(&None), "fully drawn is closed again");
}

#[test]
fn the_trim_offset_moves_the_window_around_the_path() {
    // A quarter of the perimeter (15 units), shifted a quarter turn (15 units).
    let tm = r#"{"ty":"tm","s":{"a":0,"k":0},"e":{"a":0,"k":25},"o":{"a":0,"k":90},"m":1}"#;
    let f = scene(&layer("{}", "", &format!("{RECT},{tm},{STROKE}"))).frame(0.0);
    // 15..30 of the perimeter: the last 5 units of the top edge, then the right edge.
    assert_eq!(points(&f, 0), [Some((5.0, -5.0)), Some((10.0, -5.0)), Some((10.0, 5.0))]);
}

#[test]
fn non_shape_layers_and_empty_compositions_make_empty_scenes() {
    let s = scene(r#"{"ty":3,"ks":{}},{"ty":5,"ks":{}}"#);
    assert!(s.frame(0.0).scene.nodes.is_empty());
    assert!(scene("").frame(0.0).overrides.is_empty());
}

#[test]
fn node_ids_match_their_position() {
    let f = scene(&layer("{}", "", &format!("{RECT},{ELLIPSE},{RED},{STROKE}"))).frame(0.0);
    for (i, node) in f.scene.nodes.iter().enumerate() {
        assert_eq!(node.node_id.0 as usize, i);
        assert_eq!(node.geometry, Some(i));
    }
}

/// A shape layer `ind` with the extra top-level fields `extra` (such as
/// `"tt":1,"tp":3,`), holding one rect with a fill at x = `x`.
fn numbered(ind: i32, extra: &str, x: f32) -> String {
    let rect = RECT;
    format!(
        r#"{{"ty":4,"nm":"L{ind}","ind":{ind},{extra}"ks":{{"p":{{"a":0,"k":[{x},0]}}}},"shapes":[{rect},{RED}]}}"#
    )
}

fn plane_x(s: &LottieScene, plane: usize) -> Vec<f32> {
    let f = s.plane_frame(plane, 0.0);
    f.scene
        .nodes
        .iter()
        .map(|node| {
            find(&f.overrides, node.node_id.0)
                .and_then(|o| o.transform)
                .map_or(node.transform.tx, |t| t.tx)
        })
        .collect()
}

#[test]
fn a_scene_without_mattes_is_one_plain_plane() {
    let s = scene(&format!("{},{}", numbered(1, "", 10.0), numbered(2, "", 20.0)));
    assert_eq!(s.blocks(), [Block::Plain(0)]);
    assert_eq!(s.frame(0.0).scene.nodes.len(), 2);
}

#[test]
fn a_matte_layer_is_never_drawn_by_itself() {
    let s = scene(&format!("{},{}", numbered(1, "", 10.0), numbered(2, r#""td":1,"#, 99.0)));
    assert_eq!(s.blocks(), [Block::Plain(0)]);
    assert_eq!(plane_x(&s, 0), [10.0], "only the plain layer is left");
}

#[test]
fn a_matted_layer_pairs_with_its_matte_between_its_neighbours() {
    // Listed top first: 1 above, 2 matted by 3, 3 the matte, 4 below.
    let layers = [
        numbered(1, "", 1.0),
        numbered(2, r#""tt":1,"tp":3,"#, 2.0),
        numbered(3, r#""td":1,"#, 3.0),
        numbered(4, "", 4.0),
    ]
    .join(",");
    let s = scene(&layers);
    // Drawn bottom first: 4, then 2 cut by 3, then 1.
    let blocks = s.blocks().to_vec();
    let [Block::Plain(below), Block::Matted { content, matte }, Block::Plain(above)] = blocks[..] else {
        panic!("unexpected blocks {blocks:?}");
    };
    assert_eq!(plane_x(&s, below), [4.0]);
    assert_eq!(plane_x(&s, content), [2.0]);
    assert_eq!(plane_x(&s, matte), [3.0], "the matte plane holds the matte layer");
    assert_eq!(plane_x(&s, above), [1.0]);
}

#[test]
fn layers_next_to_each_other_share_a_plane() {
    let layers = [
        numbered(1, "", 1.0),
        numbered(2, "", 2.0),
        numbered(3, r#""tt":1,"tp":4,"#, 3.0),
        numbered(4, r#""td":1,"#, 4.0),
        numbered(5, "", 5.0),
        numbered(6, "", 6.0),
    ]
    .join(",");
    let s = scene(&layers);
    let blocks = s.blocks().to_vec();
    let [Block::Plain(below), Block::Matted { .. }, Block::Plain(above)] = blocks[..] else {
        panic!("unexpected blocks {blocks:?}");
    };
    assert_eq!(plane_x(&s, below), [6.0, 5.0]);
    assert_eq!(plane_x(&s, above), [2.0, 1.0]);
}

#[test]
fn the_matte_follows_its_own_animation() {
    let moving = format!(
        r#"{{"ty":4,"nm":"M","ind":2,"td":1,"ks":{{"p":{{"a":1,"k":[{{"t":0,"s":[0,0]}},{{"t":60,"s":[60,0]}}]}}}},"shapes":[{RECT},{RED}]}}"#
    );
    let s = scene(&format!("{},{moving}", numbered(1, r#""tt":1,"tp":2,"#, 5.0)));
    let [Block::Matted { content, matte }] = s.blocks()[..] else { panic!("{:?}", s.blocks()) };
    let at = |plane, frame| {
        let f = s.plane_frame(plane, frame);
        find(&f.overrides, 0).and_then(|o| o.transform).map_or(f.scene.nodes[0].transform.tx, |t| t.tx)
    };
    assert_eq!(at(matte, 30.0), 30.0);
    assert_eq!(at(content, 30.0), 5.0, "the content does not move with the matte");
}

#[test]
fn a_matte_mode_the_renderer_cannot_apply_hides_the_layer() {
    for mode in [2, 3, 4] {
        let layers = [numbered(1, &format!(r#""tt":{mode},"tp":2,"#), 1.0), numbered(2, r#""td":1,"#, 2.0)].join(",");
        let s = scene(&layers);
        assert_eq!(s.blocks(), [Block::Plain(0)], "tt {mode}");
        assert!(s.frame(0.0).scene.nodes.is_empty(), "tt {mode}");
    }
}

#[test]
fn a_matte_with_nothing_to_draw_hides_what_it_cuts() {
    let empty = r#"{"ty":3,"nm":"M","ind":2,"td":1,"ks":{}}"#;
    let s = scene(&format!("{},{empty}", numbered(1, r#""tt":1,"tp":2,"#, 1.0)));
    assert_eq!(s.blocks(), [Block::Plain(0)]);
    assert!(s.frame(0.0).scene.nodes.is_empty());
}
