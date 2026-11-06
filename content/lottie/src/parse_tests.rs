use crate::*;

/// A composition around `layers` (already comma-joined JSON objects).
fn wrap(layers: &str) -> String {
    format!(r#"{{"v":"5.7.4","fr":30,"ip":0,"op":60,"w":100,"h":80,"layers":[{layers}]}}"#)
}

fn comp(layers: &str) -> Composition {
    Composition::parse(wrap(layers)).expect("valid document")
}

/// The transform of a one-layer composition whose `ks` is `ks`.
fn transform(ks: &str) -> Transform {
    let c = comp(&format!(r#"{{"ty":3,"nm":"t","ks":{ks}}}"#));
    c.layers.into_iter().next().expect("one layer").transform
}

/// The items of a one-shape-layer composition.
fn shapes(items: &str) -> Vec<Shape> {
    let c = comp(&format!(r#"{{"ty":4,"nm":"s","ks":{{}},"shapes":[{items}]}}"#));
    match c.layers.into_iter().next().expect("one layer").kind {
        LayerKind::Shape(items) => items,
        other => panic!("expected a shape layer, got {other:?}"),
    }
}

fn st<T>(value: T) -> Property<T> {
    Property::Static(value)
}

#[test]
fn header() {
    let c = Composition::parse(wrap("")).expect("valid");
    assert_eq!((c.width, c.height), (100.0, 80.0));
    assert_eq!(c.frame_rate, 30.0);
    assert_eq!((c.in_frame, c.out_frame), (0.0, 60.0));
    assert_eq!(c.duration_secs(), 2.0);
    assert!(c.layers.is_empty());
    assert!(c.diagnostics.is_empty());
}

#[test]
fn in_point_defaults_to_zero_and_shortens_duration() {
    let c = Composition::parse(r#"{"fr":30,"op":90,"ip":30,"w":1,"h":1}"#).expect("valid");
    assert_eq!(c.duration_secs(), 2.0);
    let c = Composition::parse(r#"{"fr":30,"op":90,"w":1,"h":1}"#).expect("valid");
    assert_eq!(c.in_frame, 0.0);
}

#[test]
fn header_errors() {
    assert_eq!(Composition::parse("[]"), Err(LottieError::NotAnObject));
    assert_eq!(
        Composition::parse(r#"{"fr":30,"op":60,"h":1}"#),
        Err(LottieError::MissingField("w"))
    );
    assert_eq!(
        Composition::parse(r#"{"fr":"30","op":60,"w":1,"h":1}"#),
        Err(LottieError::MissingField("fr"))
    );
    assert_eq!(
        Composition::parse(r#"{"fr":0,"op":60,"w":1,"h":1}"#),
        Err(LottieError::InvalidField("fr"))
    );
    assert_eq!(
        Composition::parse(r#"{"fr":30,"op":60,"w":0,"h":1}"#),
        Err(LottieError::InvalidField("w"))
    );
    assert!(matches!(
        Composition::parse(r#"{"fr":30,"#),
        Err(LottieError::Syntax { offset: 9, .. })
    ));
}

#[test]
fn layer_basics_and_defaults() {
    let c = comp(r#"{"ty":3,"nm":"parent","ind":1,"ks":{}},
                    {"ty":3,"nm":"child","ind":2,"parent":1,"ip":10,"op":50,"st":5,"hd":true,"ks":{}}"#);
    let (a, b) = (&c.layers[0], &c.layers[1]);
    assert_eq!((a.index, a.parent, a.name.as_str()), (Some(1), None, "parent"));
    assert_eq!((a.in_frame, a.out_frame, a.start_frame), (0.0, 60.0, 0.0));
    assert!(!a.hidden);
    assert_eq!(a.transform, Transform::default());
    assert_eq!(a.kind, LayerKind::Null);
    assert_eq!((b.index, b.parent), (Some(2), Some(1)));
    assert_eq!((b.in_frame, b.out_frame, b.start_frame), (10.0, 50.0, 5.0));
    assert!(b.hidden);
}

#[test]
fn unsupported_layer_is_kept_and_reported() {
    let c = comp(r#"{"ty":5,"nm":"label","ind":7,"ks":{}}"#);
    assert_eq!(c.layers[0].kind, LayerKind::Unsupported(5));
    assert_eq!(c.layers[0].index, Some(7));
    assert_eq!(
        c.diagnostics,
        vec![Diagnostic::Unsupported { layer: "label".into(), feature: "layer type 5".into() }]
    );
}

#[test]
fn layer_without_a_type_is_dropped_with_a_diagnostic() {
    let c = comp(r#"{"nm":"broken","ks":{}}"#);
    assert!(c.layers.is_empty());
    assert_eq!(
        c.diagnostics,
        vec![Diagnostic::Malformed { layer: "broken".into(), what: "layer type" }]
    );
}

#[test]
fn static_transform() {
    let t = transform(
        r#"{"a":{"a":0,"k":[5,6,0]},"p":{"a":0,"k":[10,20,0]},"s":{"a":0,"k":[50,60,100]},
            "r":{"a":0,"k":45},"o":{"a":0,"k":25}}"#,
    );
    assert_eq!(t.anchor, st([5.0, 6.0]));
    assert_eq!(t.position, Position::Joint(st([10.0, 20.0])));
    assert_eq!(t.scale, st([50.0, 60.0]));
    assert_eq!(t.rotation, st(45.0));
    assert_eq!(t.opacity, st(25.0));
}

#[test]
fn split_position() {
    let t = transform(r#"{"p":{"s":true,"x":{"a":0,"k":3},"y":{"a":0,"k":4}}}"#);
    assert_eq!(t.position, Position::Split { x: st(3.0), y: st(4.0) });
}

#[test]
fn animated_vec2_with_bezier_and_linear_segments() {
    let t = transform(
        r#"{"p":{"a":1,"k":[
            {"t":0,"s":[0,0],"o":{"x":[0.25],"y":[0.5]},"i":{"x":[0.75],"y":[1]}},
            {"t":30,"s":[100,50]}]}}"#,
    );
    assert_eq!(
        t.position,
        Position::Joint(Property::Animated(vec![
            Keyframe {
                time: 0.0,
                value: [0.0, 0.0],
                interpolation: Interpolation::Bezier { ease_out: [0.25, 0.5], ease_in: [0.75, 1.0] },
            },
            Keyframe { time: 30.0, value: [100.0, 50.0], interpolation: Interpolation::Linear },
        ]))
    );
}

#[test]
fn animated_scalar_accepts_scalar_easing_and_hold() {
    let t = transform(
        r#"{"r":{"a":1,"k":[
            {"t":0,"s":[0],"h":1},
            {"t":10,"s":[90],"o":{"x":0.5,"y":0},"i":{"x":0.5,"y":1}},
            {"t":20,"s":[180]}]}}"#,
    );
    let Property::Animated(keys) = t.rotation else { panic!("expected keyframes") };
    assert_eq!(keys[0].interpolation, Interpolation::Hold);
    assert_eq!(
        keys[1].interpolation,
        Interpolation::Bezier { ease_out: [0.5, 0.0], ease_in: [0.5, 1.0] }
    );
    assert_eq!(keys[2].value, 180.0);
}

#[test]
fn old_style_end_value_fills_a_last_keyframe_without_start() {
    let t = transform(r#"{"r":{"a":1,"k":[{"t":0,"s":[0],"e":[10]},{"t":10}]}}"#);
    let Property::Animated(keys) = t.rotation else { panic!("expected keyframes") };
    assert_eq!(keys.len(), 2);
    assert_eq!(keys[1].value, 10.0);
}

#[test]
fn malformed_property_falls_back_to_the_default() {
    let c = comp(r#"{"ty":3,"nm":"bad","ks":{"r":{"a":0,"k":"nope"},"o":{"a":1,"k":[]}}}"#);
    let t = &c.layers[0].transform;
    assert_eq!(t.rotation, st(0.0));
    assert_eq!(t.opacity, st(100.0));
    assert_eq!(c.diagnostics.len(), 2);
    assert!(
        c.diagnostics
            .iter()
            .all(|d| matches!(d, Diagnostic::Malformed { layer, .. } if layer == "bad"))
    );
}

#[test]
fn expression_is_reported_and_the_plain_value_is_kept() {
    let c = comp(r#"{"ty":3,"nm":"spin","ks":{"r":{"a":0,"k":15,"x":"time * 90"}}}"#);
    assert_eq!(c.layers[0].transform.rotation, st(15.0));
    assert_eq!(c.diagnostics, vec![Diagnostic::Expression { layer: "spin".into() }]);
}

#[test]
fn spatial_tangents_and_skew_are_reported() {
    let c = comp(
        r#"{"ty":3,"nm":"curvy","ks":{
            "p":{"a":1,"k":[{"t":0,"s":[0,0],"to":[10,0],"ti":[0,0]},{"t":10,"s":[50,50]}]},
            "sk":{"a":0,"k":20}}}"#,
    );
    let features: Vec<_> = c
        .diagnostics
        .iter()
        .map(|d| match d {
            Diagnostic::Unsupported { feature, .. } => feature.as_str(),
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    assert_eq!(features, ["spatial tangents", "skew"]);
}

#[test]
fn zero_spatial_tangents_and_zero_skew_are_silent() {
    let c = comp(
        r#"{"ty":3,"nm":"flat","ks":{
            "p":{"a":1,"k":[{"t":0,"s":[0,0],"to":[0,0],"ti":[0,0]},{"t":10,"s":[50,50]}]},
            "sk":{"a":0,"k":0}}}"#,
    );
    assert!(c.diagnostics.is_empty(), "{:?}", c.diagnostics);
}

#[test]
fn group_with_rect_ellipse_fill_stroke() {
    let items = shapes(
        r#"{"ty":"gr","nm":"g","it":[
            {"ty":"rc","p":{"a":0,"k":[1,2]},"s":{"a":0,"k":[30,40]},"r":{"a":0,"k":4}},
            {"ty":"el","p":{"a":0,"k":[0,0]},"s":{"a":0,"k":[8,8]}},
            {"ty":"fl","c":{"a":0,"k":[1,0.5,0,1]},"o":{"a":0,"k":50},"r":2},
            {"ty":"st","c":{"a":0,"k":[0,0,0]},"o":{"a":0,"k":100},"w":{"a":0,"k":2},
             "lc":2,"lj":3,"ml":7},
            {"ty":"tr","p":{"a":0,"k":[9,9]},"o":{"a":0,"k":80}}]}"#,
    );
    let [Shape::Group(g)] = items.as_slice() else { panic!("one group expected") };
    assert_eq!(g.name, "g");
    assert_eq!(g.transform.position, Position::Joint(st([9.0, 9.0])));
    assert_eq!(g.transform.opacity, st(80.0));
    assert_eq!(g.items.len(), 4, "the transform is lifted out of the items");
    assert_eq!(
        g.items[0],
        Shape::Rect(Rect { position: st([1.0, 2.0]), size: st([30.0, 40.0]), roundness: st(4.0) })
    );
    assert_eq!(g.items[1], Shape::Ellipse(Ellipse { position: st([0.0, 0.0]), size: st([8.0, 8.0]) }));
    assert_eq!(
        g.items[2],
        Shape::Fill(Fill { color: st([1.0, 0.5, 0.0, 1.0]), opacity: st(50.0), rule: FillRule::EvenOdd })
    );
    assert_eq!(
        g.items[3],
        Shape::Stroke(Stroke {
            color: st([0.0, 0.0, 0.0, 1.0]),
            opacity: st(100.0),
            width: st(2.0),
            cap: LineCap::Round,
            join: LineJoin::Bevel,
            miter_limit: 7.0,
        })
    );
}

#[test]
fn fill_and_stroke_defaults() {
    let items = shapes(r#"{"ty":"fl"},{"ty":"st"}"#);
    assert_eq!(
        items[0],
        Shape::Fill(Fill { color: st([1.0; 4]), opacity: st(100.0), rule: FillRule::NonZero })
    );
    let Shape::Stroke(s) = &items[1] else { panic!("stroke expected") };
    assert_eq!((s.width.clone(), s.cap, s.join, s.miter_limit), (st(1.0), LineCap::Butt, LineJoin::Miter, 4.0));
}

#[test]
fn path_shape() {
    let items = shapes(
        r#"{"ty":"sh","ks":{"a":0,"k":{"c":true,
            "v":[[0,0],[10,0],[10,10]],"i":[[0,0],[0,0],[0,0]],"o":[[1,1],[0,0],[0,0]]}}}"#,
    );
    let [Shape::Path(p)] = items.as_slice() else { panic!("one path expected") };
    let Property::Static(b) = &p.path else { panic!("static path expected") };
    assert!(b.closed);
    assert_eq!(b.vertices, [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]]);
    assert_eq!(b.out_tangents[0], [1.0, 1.0]);
}

#[test]
fn animated_path_reads_each_keyframe_shape() {
    let items = shapes(
        r#"{"ty":"sh","ks":{"a":1,"k":[
            {"t":0,"s":[{"c":false,"v":[[0,0],[1,1]],"i":[[0,0],[0,0]],"o":[[0,0],[0,0]]}]},
            {"t":9,"s":[{"c":false,"v":[[0,0],[5,5]],"i":[[0,0],[0,0]],"o":[[0,0],[0,0]]}]}]}}"#,
    );
    let [Shape::Path(p)] = items.as_slice() else { panic!("one path expected") };
    let Property::Animated(keys) = &p.path else { panic!("keyframes expected") };
    assert_eq!(keys[1].value.vertices[1], [5.0, 5.0]);
}

#[test]
fn path_with_mismatched_tangents_is_malformed() {
    let c = comp(
        r#"{"ty":4,"nm":"p","ks":{},"shapes":[{"ty":"sh","ks":{"a":0,"k":{"c":false,
            "v":[[0,0],[1,1]],"i":[[0,0]],"o":[[0,0],[0,0]]}}}]}"#,
    );
    assert_eq!(
        c.diagnostics,
        vec![Diagnostic::Malformed { layer: "p".into(), what: "path" }]
    );
}

#[test]
fn trim_path() {
    let items = shapes(
        r#"{"ty":"tm","s":{"a":0,"k":10},"e":{"a":1,"k":[{"t":0,"s":[0]},{"t":9,"s":[100]}]},
            "o":{"a":0,"k":90},"m":2}"#,
    );
    let [Shape::Trim(t)] = items.as_slice() else { panic!("one trim expected") };
    assert_eq!(t.start, st(10.0));
    assert!(matches!(t.end, Property::Animated(_)));
    assert_eq!(t.offset, st(90.0));
    assert!(!t.simultaneous);
}

#[test]
fn trim_defaults_cover_the_whole_path() {
    let items = shapes(r#"{"ty":"tm"}"#);
    let [Shape::Trim(t)] = items.as_slice() else { panic!("one trim expected") };
    assert_eq!((t.start.clone(), t.end.clone(), t.offset.clone()), (st(0.0), st(100.0), st(0.0)));
    assert!(t.simultaneous);
}

#[test]
fn unsupported_and_hidden_shapes_are_skipped() {
    let c = comp(
        r#"{"ty":4,"nm":"s","ks":{},"shapes":[
            {"ty":"rp","c":{"a":0,"k":3}},
            {"ty":"fl","hd":true},
            {"ty":"el","p":{"a":0,"k":[0,0]},"s":{"a":0,"k":[2,2]}}]}"#,
    );
    let LayerKind::Shape(items) = &c.layers[0].kind else { panic!("shape layer expected") };
    assert_eq!(items.len(), 1);
    assert!(matches!(items[0], Shape::Ellipse(_)));
    assert_eq!(
        c.diagnostics,
        vec![Diagnostic::Unsupported { layer: "s".into(), feature: "shape type `rp`".into() }]
    );
}

#[test]
fn diagnostics_display_names_the_layer() {
    let d = Diagnostic::Expression { layer: "spin".into() };
    assert!(d.to_string().contains("spin"));
    assert!(LottieError::MissingField("w").to_string().contains('w'));
}

#[test]
fn track_matte_fields_are_read() {
    let c = comp(
        r#"{"ty":4,"nm":"content","ind":1,"tt":1,"tp":2,"ks":{},"shapes":[]},
           {"ty":4,"nm":"matte","ind":2,"td":1,"ks":{},"shapes":[]}"#,
    );
    assert_eq!(c.layers[0].matte, Some(MatteMode::Alpha));
    assert_eq!(c.layers[0].matte_slot, Some(1), "tp names the matte by its ind");
    assert!(!c.layers[0].is_matte);
    assert!(c.layers[1].is_matte);
    assert_eq!(c.layers[1].matte, None);
    assert!(c.diagnostics.is_empty(), "{:?}", c.diagnostics);
}

#[test]
fn a_matte_without_tp_is_the_layer_above() {
    // Older files omit `tp`: the matte is the layer listed just before.
    let c = comp(
        r#"{"ty":4,"nm":"matte","ind":1,"td":1,"ks":{},"shapes":[]},
           {"ty":4,"nm":"content","ind":2,"tt":2,"ks":{},"shapes":[]}"#,
    );
    assert_eq!(c.layers[1].matte, Some(MatteMode::AlphaInverted));
    assert_eq!(c.layers[1].matte_slot, Some(0));
}

#[test]
fn every_matte_mode_is_read() {
    for (code, mode) in [
        (1, MatteMode::Alpha),
        (2, MatteMode::AlphaInverted),
        (3, MatteMode::Luma),
        (4, MatteMode::LumaInverted),
    ] {
        let c = comp(&format!(
            r#"{{"ty":4,"ind":1,"td":1,"ks":{{}},"shapes":[]}},{{"ty":4,"ind":2,"tt":{code},"tp":1,"ks":{{}},"shapes":[]}}"#
        ));
        assert_eq!(c.layers[1].matte, Some(mode), "tt {code}");
    }
}

#[test]
fn a_matte_that_does_not_exist_is_dropped_with_a_diagnostic() {
    let c = comp(r#"{"ty":4,"nm":"orphan","ind":1,"tt":1,"tp":9,"ks":{},"shapes":[]}"#);
    assert_eq!(c.layers[0].matte, None, "no matte to cut by, so the layer is drawn as it is");
    assert_eq!(
        c.diagnostics,
        [Diagnostic::Malformed { layer: "orphan".into(), what: "matte reference" }]
    );
}

#[test]
fn the_first_layer_has_no_layer_above_to_be_its_matte() {
    let c = comp(r#"{"ty":4,"nm":"first","ind":1,"tt":1,"ks":{},"shapes":[]}"#);
    assert_eq!(c.layers[0].matte, None);
    assert_eq!(c.diagnostics.len(), 1);
}

#[test]
fn only_the_alpha_matte_is_free_of_diagnostics() {
    for (code, feature) in [
        (1, None),
        (2, Some("inverted alpha track matte, layer not drawn")),
        (3, Some("luma track matte, layer not drawn")),
        (4, Some("inverted luma track matte, layer not drawn")),
    ] {
        let c = comp(&format!(
            r#"{{"ty":4,"ind":1,"td":1,"ks":{{}},"shapes":[]}},{{"ty":4,"nm":"cut","ind":2,"tt":{code},"tp":1,"ks":{{}},"shapes":[]}}"#
        ));
        let expected: Vec<Diagnostic> = feature
            .map(|feature| Diagnostic::Unsupported { layer: "cut".into(), feature: feature.into() })
            .into_iter()
            .collect();
        assert_eq!(c.diagnostics, expected, "tt {code}");
    }
}
