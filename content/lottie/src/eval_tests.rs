use crate::*;

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

fn near2(a: Vec2, b: Vec2) -> bool {
    near(a[0], b[0]) && near(a[1], b[1])
}

fn key<T>(time: f32, value: T, interpolation: Interpolation) -> Keyframe<T> {
    Keyframe { time, value, interpolation }
}

fn lin<T>(time: f32, value: T) -> Keyframe<T> {
    key(time, value, Interpolation::Linear)
}

fn wrap(layers: &str) -> String {
    format!(r#"{{"fr":30,"ip":0,"op":60,"w":100,"h":80,"layers":[{layers}]}}"#)
}

fn comp(layers: &str) -> Composition {
    Composition::parse(wrap(layers)).expect("valid document")
}

fn states(c: &Composition, frame: f32) -> Vec<LayerState> {
    let mut out = Vec::new();
    c.layer_states(frame, &mut out);
    out
}

// ---- Property::value_at ----------------------------------------------------

#[test]
fn static_property_ignores_the_frame() {
    assert_eq!(Property::Static(4.0f32).value_at(-10.0), 4.0);
    assert_eq!(Property::Static(4.0f32).value_at(1e9), 4.0);
}

#[test]
fn linear_keyframes_interpolate_and_hold_at_the_ends() {
    let p = Property::Animated(vec![lin(0.0, 0.0f32), lin(10.0, 100.0)]);
    assert_eq!(p.value_at(5.0), 50.0);
    assert_eq!(p.value_at(0.0), 0.0);
    assert_eq!(p.value_at(10.0), 100.0);
    assert_eq!(p.value_at(-3.0), 0.0);
    assert_eq!(p.value_at(99.0), 100.0);
}

#[test]
fn the_segment_is_chosen_by_time() {
    let p = Property::Animated(vec![lin(0.0, 0.0f32), lin(10.0, 100.0), lin(20.0, 0.0)]);
    assert_eq!(p.value_at(5.0), 50.0);
    assert_eq!(p.value_at(10.0), 100.0);
    assert_eq!(p.value_at(15.0), 50.0);
}

#[test]
fn hold_keeps_the_value_until_the_next_keyframe() {
    let p = Property::Animated(vec![key(0.0, 1.0f32, Interpolation::Hold), lin(10.0, 9.0)]);
    assert_eq!(p.value_at(9.9), 1.0);
    assert_eq!(p.value_at(10.0), 9.0);
}

#[test]
fn bezier_easing_shapes_the_progress() {
    let ease = Interpolation::Bezier { ease_out: [0.42, 0.0], ease_in: [0.58, 1.0] };
    let p = Property::Animated(vec![key(0.0, 0.0f32, ease), lin(10.0, 100.0)]);
    assert!(near(p.value_at(5.0), 50.0), "symmetric easing crosses the middle");
    assert!(p.value_at(2.0) < 20.0, "ease-in starts slower than linear");
    assert!(p.value_at(8.0) > 80.0, "ease-out ends slower than linear");
}

#[test]
fn vec2_and_color_blend_per_channel() {
    let p = Property::Animated(vec![lin(0.0, [0.0f32, 10.0]), lin(10.0, [10.0, 20.0])]);
    assert!(near2(p.value_at(5.0), [5.0, 15.0]));
    let c = Property::Animated(vec![lin(0.0, [0.0f32, 0.0, 0.0, 1.0]), lin(2.0, [1.0, 0.5, 0.0, 0.0])]);
    assert_eq!(c.value_at(1.0), [0.5, 0.25, 0.0, 0.5]);
}

#[test]
fn bezier_paths_blend_when_shapes_match_and_hold_otherwise() {
    let path = |v: Vec<Vec2>| Bezier {
        closed: true,
        in_tangents: vec![[0.0; 2]; v.len()],
        out_tangents: vec![[0.0; 2]; v.len()],
        vertices: v,
    };
    let two = Property::Animated(vec![
        lin(0.0, path(vec![[0.0, 0.0], [10.0, 0.0]])),
        lin(10.0, path(vec![[0.0, 10.0], [20.0, 0.0]])),
    ]);
    assert_eq!(two.value_at(5.0).vertices, [[0.0, 5.0], [15.0, 0.0]]);

    let mismatched = Property::Animated(vec![
        lin(0.0, path(vec![[0.0, 0.0], [1.0, 1.0]])),
        lin(10.0, path(vec![[0.0, 0.0], [1.0, 1.0], [2.0, 2.0]])),
    ]);
    assert_eq!(mismatched.value_at(5.0).vertices.len(), 2, "different shapes hold the first");
    assert_eq!(mismatched.value_at(10.0).vertices.len(), 3);
}

#[test]
fn degenerate_inputs_do_not_panic_or_produce_nan() {
    let p = Property::Animated(vec![lin(5.0, 1.0f32), lin(5.0, 2.0), lin(9.0, 3.0)]);
    for frame in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 5.0, 4.0, 6.0] {
        let v = p.value_at(frame);
        assert!(v.is_finite(), "frame {frame} gave {v}");
    }
    let one = Property::Animated(vec![lin(3.0, 7.0f32)]);
    assert_eq!(one.value_at(0.0), 7.0);
    assert_eq!(one.value_at(100.0), 7.0);
}

// ---- Affine ----------------------------------------------------------------

#[test]
fn affine_primitives() {
    assert!(near2(Affine::translate(3.0, 4.0).apply([1.0, 1.0]), [4.0, 5.0]));
    assert!(near2(Affine::scale(2.0, 3.0).apply([1.0, 1.0]), [2.0, 3.0]));
    // Clockwise on a y-down canvas: the x axis turns into the y axis.
    assert!(near2(Affine::rotate_degrees(90.0).apply([1.0, 0.0]), [0.0, 1.0]));
    assert_eq!(Affine::IDENTITY.apply([7.0, 8.0]), [7.0, 8.0]);
}

#[test]
fn affine_product_applies_the_right_operand_first() {
    let m = Affine::translate(10.0, 0.0) * Affine::scale(2.0, 2.0);
    assert!(near2(m.apply([1.0, 1.0]), [12.0, 2.0]));
    let n = Affine::scale(2.0, 2.0) * Affine::translate(10.0, 0.0);
    assert!(near2(n.apply([1.0, 1.0]), [22.0, 2.0]));
}

// ---- Transform::local_at ---------------------------------------------------

fn transform_of(ks: &str) -> Transform {
    comp(&format!(r#"{{"ty":3,"ks":{ks}}}"#)).layers.remove(0).transform
}

#[test]
fn default_transform_is_the_identity() {
    let t = Transform::default().local_at(12.0);
    assert!(near2(t.matrix.apply([3.0, 4.0]), [3.0, 4.0]));
    assert_eq!(t.opacity, 1.0);
}

#[test]
fn transform_pivots_around_the_anchor() {
    let t = transform_of(
        r#"{"a":{"a":0,"k":[5,5]},"p":{"a":0,"k":[10,20]},
            "s":{"a":0,"k":[200,200]},"r":{"a":0,"k":90}}"#,
    )
    .local_at(0.0);
    // The anchor lands on the position; one unit right of it, doubled and
    // turned a quarter clockwise, lands two units below.
    assert!(near2(t.matrix.apply([5.0, 5.0]), [10.0, 20.0]));
    assert!(near2(t.matrix.apply([6.0, 5.0]), [10.0, 22.0]));
}

#[test]
fn split_position_and_animation_are_evaluated() {
    let t = transform_of(
        r#"{"p":{"s":true,"x":{"a":1,"k":[{"t":0,"s":[0]},{"t":10,"s":[100]}]},
            "y":{"a":0,"k":7}}}"#,
    );
    assert!(near2(t.local_at(5.0).matrix.apply([0.0, 0.0]), [50.0, 7.0]));
}

#[test]
fn opacity_is_a_clamped_fraction() {
    for (percent, want) in [(50, 0.5), (100, 1.0), (150, 1.0), (-5, 0.0), (0, 0.0)] {
        let t = transform_of(&format!(r#"{{"o":{{"a":0,"k":{percent}}}}}"#)).local_at(0.0);
        assert!(near(t.opacity, want), "{percent}% gave {}", t.opacity);
    }
}

// ---- Composition ------------------------------------------------------------

#[test]
fn parent_transforms_apply_even_when_the_parent_comes_later() {
    // File order is child first, as exporters write it.
    let c = comp(
        r#"{"ty":3,"nm":"child","ind":2,"parent":1,"ks":{"p":{"a":0,"k":[10,0]},"o":{"a":0,"k":100}}},
           {"ty":3,"nm":"parent","ind":1,"ks":{"p":{"a":0,"k":[100,0]},"o":{"a":0,"k":50}}}"#,
    );
    assert!(c.diagnostics.is_empty(), "{:?}", c.diagnostics);
    let s = states(&c, 0.0);
    assert!(near2(s[0].world.apply([0.0, 0.0]), [110.0, 0.0]));
    assert!(near2(s[0].local.apply([0.0, 0.0]), [10.0, 0.0]));
    assert!(near2(s[1].world.apply([0.0, 0.0]), [100.0, 0.0]));
    assert!(near(s[0].opacity, 1.0), "opacity is not inherited");
    assert!(near(s[1].opacity, 0.5));
}

#[test]
fn a_grandparent_chain_multiplies_in_order() {
    let c = comp(
        r#"{"ty":3,"ind":3,"parent":2,"ks":{"p":{"a":0,"k":[1,0]}}},
           {"ty":3,"ind":2,"parent":1,"ks":{"s":{"a":0,"k":[200,200]},"p":{"a":0,"k":[10,0]}}},
           {"ty":3,"ind":1,"ks":{"p":{"a":0,"k":[100,0]}}}"#,
    );
    // child (1,0) -> scaled by 2 -> +10 -> +100
    assert!(near2(states(&c, 0.0)[0].world.apply([0.0, 0.0]), [112.0, 0.0]));
}

#[test]
fn visibility_follows_the_frame_range_and_hidden_flag() {
    let c = comp(
        r#"{"ty":3,"ip":10,"op":20,"ks":{}},
           {"ty":3,"hd":true,"ks":{}}"#,
    );
    assert!(!states(&c, 9.0)[0].visible);
    assert!(states(&c, 10.0)[0].visible);
    assert!(states(&c, 19.9)[0].visible);
    assert!(!states(&c, 20.0)[0].visible, "the out point is exclusive");
    assert!(!states(&c, 15.0)[1].visible, "hidden layers never show");
}

#[test]
fn animation_is_evaluated_per_frame() {
    let c = comp(
        r#"{"ty":3,"ks":{"p":{"a":1,"k":[{"t":0,"s":[0,0]},{"t":10,"s":[100,0]}]}}}"#,
    );
    assert!(near2(states(&c, 5.0)[0].world.apply([0.0, 0.0]), [50.0, 0.0]));
    assert!(near2(states(&c, 10.0)[0].world.apply([0.0, 0.0]), [100.0, 0.0]));
}

#[test]
fn the_output_buffer_is_reused() {
    let c = comp(r#"{"ty":3,"ks":{}},{"ty":3,"ks":{}}"#);
    let mut out = Vec::new();
    c.layer_states(0.0, &mut out);
    c.layer_states(1.0, &mut out);
    assert_eq!(out.len(), 2, "each call replaces the previous contents");
}

#[test]
fn parent_cycles_and_dangling_parents_are_dropped_with_diagnostics() {
    let c = comp(
        r#"{"ty":3,"nm":"a","ind":1,"parent":2,"ks":{}},
           {"ty":3,"nm":"b","ind":2,"parent":1,"ks":{}},
           {"ty":3,"nm":"me","ind":3,"parent":3,"ks":{}},
           {"ty":3,"nm":"orphan","ind":4,"parent":99,"ks":{}}"#,
    );
    // Evaluation must terminate and every layer must still get a state.
    assert_eq!(states(&c, 0.0).len(), 4);
    let reported: Vec<_> = c
        .diagnostics
        .iter()
        .map(|d| match d {
            Diagnostic::Malformed { layer, what } => (layer.as_str(), *what),
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(reported.len(), 3, "{reported:?}");
    assert!(reported.contains(&("me", "parent reference")));
    assert!(reported.contains(&("orphan", "parent reference")));
    assert!(reported.iter().any(|(l, w)| (*l == "a" || *l == "b") && *w == "parent reference"));
}

#[test]
fn frame_at_maps_seconds_and_holds_the_ends() {
    let c = comp("");
    assert_eq!(c.frame_at(0.0), 0.0);
    assert_eq!(c.frame_at(1.0), 30.0);
    assert_eq!(c.frame_at(100.0), 60.0);
    assert_eq!(c.frame_at(-1.0), 0.0);
}

// ---- Trim -------------------------------------------------------------------

fn trim(start: f32, end: f32, offset: f32) -> Trim {
    Trim {
        start: Property::Static(start),
        end: Property::Static(end),
        offset: Property::Static(offset),
        simultaneous: true,
    }
}

fn segs(t: Trim) -> Vec<(f32, f32)> {
    t.segments_at(0.0).into_iter().flatten().collect()
}

fn same(got: Vec<(f32, f32)>, want: &[(f32, f32)]) {
    assert_eq!(got.len(), want.len(), "{got:?} vs {want:?}");
    for (g, w) in got.iter().zip(want) {
        assert!(near(g.0, w.0) && near(g.1, w.1), "{got:?} vs {want:?}");
    }
}

#[test]
fn trim_segments() {
    same(segs(trim(0.0, 100.0, 0.0)), &[(0.0, 1.0)]);
    same(segs(trim(25.0, 75.0, 0.0)), &[(0.25, 0.75)]);
    same(segs(trim(75.0, 25.0, 0.0)), &[(0.25, 0.75)]); // swapped ends
    same(segs(trim(10.0, 20.0, 90.0)), &[(0.35, 0.45)]); // 90 deg = a quarter turn
    same(segs(trim(0.0, 50.0, 180.0)), &[(0.5, 1.0)]);
    same(segs(trim(50.0, 100.0, 90.0)), &[(0.75, 1.0), (0.0, 0.25)]); // wraps
    same(segs(trim(0.0, 25.0, -90.0)), &[(0.75, 1.0)]); // negative offset
    same(segs(trim(20.0, 20.0, 0.0)), &[]); // nothing visible
    same(segs(trim(-40.0, 140.0, 0.0)), &[(0.0, 1.0)]); // clamped
}

#[test]
fn a_full_length_trim_is_whole_whatever_the_offset() {
    same(segs(trim(0.0, 100.0, 123.0)), &[(0.0, 1.0)]);
}

#[test]
fn trim_is_animated() {
    let t = Trim {
        start: Property::Static(0.0),
        end: Property::Animated(vec![lin(0.0, 0.0), lin(10.0, 100.0)]),
        offset: Property::Static(0.0),
        simultaneous: true,
    };
    let at = |f| t.segments_at(f).into_iter().flatten().collect::<Vec<_>>();
    same(at(5.0), &[(0.0, 0.5)]);
    same(at(10.0), &[(0.0, 1.0)]);
}

#[test]
fn a_layer_leading_into_a_cycle_keeps_its_own_parent() {
    // tail -> u <-> v: only the edge that closes the loop may be cut.
    let c = comp(
        r#"{"ty":3,"nm":"tail","ind":5,"parent":6,"ks":{"p":{"a":0,"k":[1,0]}}},
           {"ty":3,"nm":"u","ind":6,"parent":7,"ks":{"p":{"a":0,"k":[10,0]}}},
           {"ty":3,"nm":"v","ind":7,"parent":6,"ks":{"p":{"a":0,"k":[100,0]}}}"#,
    );
    assert_eq!(c.diagnostics.len(), 1, "{:?}", c.diagnostics);
    assert!(matches!(&c.diagnostics[0], Diagnostic::Malformed { layer, .. } if layer == "v"));
    // tail is still carried by u, which is still carried by v.
    assert!(near2(states(&c, 0.0)[0].world.apply([0.0, 0.0]), [111.0, 0.0]));
}
