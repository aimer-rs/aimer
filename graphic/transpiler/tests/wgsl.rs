use std::path::PathBuf;

use aimer_transpiler::resolve::{FsLoader, NoLoader};

fn validate(wgsl: &str) {
    let module = naga::front::wgsl::parse_str(wgsl).unwrap_or_else(|e| panic!("naga parse failed:\n{}\n---\n{wgsl}", e.emit_to_string(wgsl)));
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(&module)
        .unwrap_or_else(|e| panic!("naga validation failed: {e:?}\n---\n{wgsl}"));
}

/// Compile a self-contained snippet and validate the result with naga.
fn ok(src: &str) -> String {
    let out = aimer_transpiler::compile_wgsl(src, "t", &NoLoader).unwrap_or_else(|e| panic!("compile failed: {e}"));
    validate(&out);
    out
}

fn err(src: &str) -> String {
    match aimer_transpiler::compile_wgsl(src, "t", &NoLoader) {
        Ok(out) => panic!("expected an error, got:\n{out}"),
        Err(e) => e.to_string(),
    }
}

fn shaders_src() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders-src")
}

fn compile_file(name: &str) -> String {
    let dir = shaders_src();
    let src = std::fs::read_to_string(dir.join(format!("{name}.aimer"))).unwrap();
    aimer_transpiler::compile_wgsl(&src, name, &FsLoader { root: dir.join("lib") }).unwrap_or_else(|e| panic!("{name}: {e}"))
}

#[test]
fn ported_shaders_are_valid_wgsl() {
    for name in ["frame_composite", "svg"] {
        validate(&compile_file(name));
    }
}

#[test]
fn svg_keeps_the_original_interface() {
    let out = compile_file("svg");
    for needle in [
        "@location(0) position: vec2<f32>",
        "@location(7) coverage: f32",
        "@location(1) transform_x: vec4<f32>",
        "@location(6) viewport: vec4<f32>",
        "@builtin(position) position: vec4<f32>",
        "@location(5) coverage: f32",
        "fn vs_main(vertex: SvgVertex, instance: SvgInstance) -> VertexOutput",
        "fn fs_main(input: VertexOutput) -> @location(0) vec4<f32>",
        "radius = select(radii.w, radii.x, input_point.y < 0.0);",
    ] {
        assert!(out.contains(needle), "missing `{needle}` in:\n{out}");
    }
}

#[test]
fn frame_composite_matches_the_original_structure() {
    let out = compile_file("frame_composite");
    assert!(out.contains("@group(0) @binding(0)\nvar source_frame: texture_2d<f32>;"));
    assert!(out.contains("var corners = array<vec2<f32>, 6>("));
    assert!(out.contains("return textureLoad(source_frame, vec2<i32>(position.xy), 0);"));
}

#[test]
fn if_as_value_with_a_call_uses_a_temporary() {
    let out = ok("
        fn g(x: f32) -> f32 { x * 2.0 }
        fn f(c: f32, a: f32, b: f32) -> f32 {
            let w = if c > 0.0 { a * 2.0 } else { g(b) };
            w + 1.0
        }");
    assert!(out.contains("var w: f32;"), "{out}");
    assert!(out.contains("if c > 0.0 {"), "{out}");
    assert!(!out.contains("select("), "{out}");
}

#[test]
fn tail_if_returns_from_each_branch() {
    let out = ok("fn f(c: f32) -> f32 { if c <= 0.5 { c / 2.0 } else if c <= 1.0 { c } else { 1.0 } }");
    assert!(out.contains("return c / 2.0;"), "{out}");
    assert!(out.contains("} else if c <= 1.0 {"), "{out}");
}

#[test]
fn select_attribute_emits_select() {
    let out = ok("fn f(c: f32, a: f32, b: f32) -> f32 { #[select] if c < 0.5 { a } else { b } }");
    assert!(out.contains("return select(b, a, c < 0.5);"), "{out}");
}

#[test]
fn select_allows_pure_calls_only() {
    let out = ok("fn g(x: f32) -> f32 { x }\nfn f(c: f32) -> f32 { #[select] if c < 0.5 { g(c) } else { 1.0 } }");
    assert!(out.contains("select(1.0, g(c), c < 0.5)"), "{out}");
    let e = err("fn g(x: f32) -> f32 { fwidth(x) }\nfn f(c: f32) -> f32 { #[select] if c < 0.5 { g(c) } else { 1.0 } }");
    assert!(e.contains("pure functions"), "{e}");
    let e = err("fn g(x: f32) -> f32 { if x < 0.0 { discard; } x }\nfn f(c: f32) -> f32 { #[select] if c < 0.5 { g(c) } else { 1.0 } }");
    assert!(e.contains("pure functions"), "{e}");
}

#[test]
fn match_becomes_switch_or_if_chain() {
    let out = ok("fn f(n: u32) -> f32 { match n { 0u => 1.0, 1u | 2u => 2.0, _ => 3.0 } }");
    assert!(out.contains("switch n {"), "{out}");
    assert!(out.contains("case 1u, 2u: {"), "{out}");
    assert!(out.contains("default: {"), "{out}");

    let out = ok("fn f(n: u32) -> f32 { match n { 0u => 1.0, 3u..=7u => 2.0, _ => 3.0 } }");
    assert!(!out.contains("switch"), "{out}");
    assert!(out.contains("n >= 3u && n <= 7u"), "{out}");
}

#[test]
fn match_without_wildcard_is_an_error() {
    let e = err("fn f(n: u32) -> f32 { match n { 0u => 1.0, 1u => 2.0 } }");
    assert!(e.contains("`_`"), "{e}");
}

#[test]
fn shadowing_is_renamed() {
    let out = ok("fn f(a: f32) -> f32 { let x = a; let x = x + 1.0; let a = x * 2.0; a }");
    assert!(out.contains("let x = a;"), "{out}");
    assert!(out.contains("let x_1 = x + 1.0;"), "{out}");
    assert!(out.contains("let a_1 = x_1 * 2.0;"), "{out}");
}

#[test]
fn reserved_words_are_renamed() {
    let out = ok("fn f(target: f32) -> f32 { let filter = target + 1.0; filter }");
    assert!(out.contains("fn f(target_: f32)"), "{out}");
    assert!(out.contains("let filter_ = target_ + 1.0;"), "{out}");
}

#[test]
fn constants_are_inlined_without_folding() {
    let out = ok("
        const A: f32 = 1.5;
        const H: f32 = A * 0.5;
        const N: u32 = 6u;
        fn f(x: f32) -> f32 { if x >= A { x * H } else { 2.0 * H } }
        fn g() -> u32 { N + 1u }");
    assert!(!out.contains("const "), "{out}");
    assert!(out.contains("x >= 1.5"), "{out}");
    // Const arithmetic is typed as f32 so WGSL does not evaluate it in abstract floats.
    assert!(out.contains("x * (f32(1.5) * f32(0.5))"), "{out}");
    assert!(out.contains("2.0 * (f32(1.5) * f32(0.5))"), "{out}");
    assert!(out.contains("return 6u + 1u;"), "{out}");
}

#[test]
fn swizzle_assignment_is_split_per_component() {
    let out = ok("
        fn f(a: vec4, b: vec2) -> vec4 {
            let mut v = a;
            v.xy = b * 2.0;
            v.zw += b;
            v.wz = b;
            v
        }");
    assert!(out.contains("let _t0 = b * 2.0;"), "{out}");
    assert!(out.contains("v.x = _t0.x;") && out.contains("v.y = _t0.y;"), "{out}");
    assert!(out.contains("v.z += b.x;") && out.contains("v.w += b.y;"), "{out}");
    assert!(out.contains("v.w = b.x;") && out.contains("v.z = b.y;"), "{out}");
}

#[test]
fn error_swizzle_assignment_with_repeated_component() {
    let e = err("fn f(b: vec2) -> vec4 { let mut v = vec4(0.0); v.xx = b; v }");
    assert!(e.contains("appears twice"), "{e}");
}

#[test]
fn const_array_constant_index_is_inlined_runtime_index_is_a_table() {
    let src = "
        const C: [vec2; 2] = [vec2(0.0, 1.0), vec2(2.0, 3.0)];
        fn a() -> vec2 { C[1] }
        fn b(i: u32) -> vec2 { C[i] }";
    let out = ok(src);
    assert!(out.contains("return vec2<f32>(2.0, 3.0);"), "{out}");
    assert!(out.contains("var c = array<vec2<f32>, 2>("), "{out}");
    assert!(out.contains("return c[i];"), "{out}");
}

#[test]
fn const_array_size_from_a_const() {
    let out = ok("
        const N: u32 = 2u;
        const C: [f32; N] = [1.0, 2.0];
        fn f(i: u32) -> f32 { C[i] }");
    assert!(out.contains("array<f32, 2>"), "{out}");
}

#[test]
fn loop_with_break_value() {
    let out = ok("
        fn f(n: u32) -> f32 {
            let mut i = 0u;
            let mut acc = 0.0;
            let r: f32 = loop {
                if i >= n { break acc; }
                acc += 1.0;
                i += 1u;
            };
            r
        }");
    assert!(out.contains("var r: f32;"), "{out}");
    assert!(out.contains("r = acc;"), "{out}");
}

#[test]
fn loop_break_value_type_is_inferred_without_annotation() {
    let out = ok("
        fn f(n: u32) -> f32 {
            let mut i = 0u;
            let mut acc = 0.0;
            let r = loop {
                if i >= n { break acc * 2.0; }
                acc += 1.0;
                i += 1u;
            };
            r + 1.0
        }");
    assert!(out.contains("var r: f32;"), "{out}");
    assert!(out.contains("r = acc * 2.0;"), "{out}");
}

#[test]
fn loop_as_tail_value_and_vector_break() {
    let out = ok("
        fn f(n: u32) -> vec2 {
            let mut i = 0u;
            loop {
                if i >= n { break vec2(1.0, 2.0); }
                i += 1u;
            }
        }");
    // A function must not end in a `loop`, so the value goes through a temporary.
    assert!(out.contains("var _t0: vec2<f32>;"), "{out}");
    assert!(out.contains("_t0 = vec2<f32>(1.0, 2.0);"), "{out}");
    assert!(out.contains("return _t0;"), "{out}");
}

#[test]
fn loop_value_with_several_breaks_and_nested_loops() {
    let out = ok("
        fn f(n: u32) -> u32 {
            let mut i = 0u;
            let r = loop {
                let mut j = 0u;
                while j < n {
                    if j == 3u { break; }
                    j += 1u;
                }
                if i > 5u { break i; }
                if j == 0u { break 7u; }
                i += 1u;
            };
            r
        }");
    assert!(out.contains("var r: u32;"), "{out}");
}

#[test]
fn loop_infers_type_even_when_the_annotation_is_given() {
    let out = ok("fn f() -> f32 { let r: f32 = loop { break 1.0; }; r }");
    assert!(out.contains("var r: f32;"), "{out}");
    let e = err("fn f() -> f32 { let r: u32 = loop { break 1.0; }; 0.0 }");
    assert!(e.contains("expected u32, found f32"), "{e}");
}

#[test]
fn error_loop_break_values_differ() {
    let e = err("fn f(c: bool) -> f32 { let r = loop { if c { break 1.0; } break 2u; }; 0.0 }");
    assert!(e.contains("`break` values differ"), "{e}");
}

#[test]
fn error_loop_value_with_a_bare_break() {
    let e = err("fn f(c: bool) -> f32 { let r = loop { if c { break; } break 2.0; }; r }");
    assert!(e.contains("every `break` must carry a value"), "{e}");
}

#[test]
fn match_arm_with_break_is_an_if_chain() {
    // A `break` in a `switch` only leaves the switch, so the match must not become one.
    let out = ok("
        fn f(k: u32) -> f32 {
            var_acc();
            let mut acc = 0.0;
            loop {
                match k {
                    0u => { break; }
                    _ => { acc += 1.0; }
                }
            }
            acc
        }
        fn var_acc() {}");
    assert!(!out.contains("switch"), "{out}");
    assert!(out.contains("if k == 0u {"), "{out}");
}

#[test]
fn loops_for_and_while() {
    let out = ok("
        fn f(n: u32) -> f32 {
            let mut acc = 0.0;
            for i in 0u..n { acc += f32(i); }
            let mut x = 4.0;
            while x > 1.0 { x -= 1.0; }
            acc + x
        }");
    assert!(out.contains("for (var i = 0u; i < n; i++) {"), "{out}");
    assert!(out.contains("while x > 1.0 {"), "{out}");
}

#[test]
fn short_circuit_with_statements_on_the_right() {
    let out = ok("
        fn g(x: f32) -> f32 { x }
        fn f(a: bool, x: f32) -> bool {
            a && (if x > 0.0 { g(x) } else { g(0.0 - x) }) > 1.0
        }");
    // The right side only runs when the left side is true.
    assert!(out.contains("var _t1 = a;"), "{out}");
    assert!(out.contains("if _t1 {"), "{out}");
    assert!(out.contains("_t1 = _t0 > 1.0;"), "{out}");
}

#[test]
fn evaluation_order_is_kept_when_a_later_operand_hoists() {
    let out = ok("
        fn g(x: f32) -> f32 { x }
        fn f(c: bool, a: f32) -> f32 {
            let mut m = a;
            m + (if c { g(1.0) } else { g(2.0) })
        }");
    // `m` is mutable, so it must be copied before the hoisted statements run.
    assert!(out.contains("let _t"), "{out}");
}

#[test]
fn block_expression_and_struct_update() {
    ok("
        struct P { a: f32, b: f32 }
        fn f(x: f32) -> P {
            let base = P { a: x, b: 1.0 };
            let len = {
                let d = base.a - base.b;
                sqrt(d * d)
            };
            P { a: len, ..base }
        }");
}

#[test]
fn precedence_adds_parentheses_where_needed() {
    let out = ok("fn f(a: f32, b: f32, c: f32) -> f32 { (a + b) * c - (a - b) }");
    assert!(out.contains("return (a + b) * c - (a - b);"), "{out}");
    let out = ok("fn f(a: bool, b: bool, c: bool) -> bool { (a || b) && c }");
    assert!(out.contains("(a || b) && c"), "{out}");
    let out = ok("fn f(a: bool, b: bool, c: bool) -> bool { a || (b && c) }");
    assert!(out.contains("a || (b && c)"), "{out}");
}

#[test]
fn texture_methods() {
    let out = ok("
        #[bind(0, 0)]
        texture t: texture2d;
        #[bind(0, 1)]
        sampler s;
        fn f(uv: vec2) -> vec4 { t.sample(s, uv) + t.sample_level(s, uv, 0.0) }");
    assert!(out.contains("textureSample(t, s, uv)"), "{out}");
    assert!(out.contains("textureSampleLevel(t, s, uv, 0.0)"), "{out}");
}

#[test]
fn discard_and_unit_functions() {
    ok("
        #[varying]
        struct V { @position p: vec4, a: f32 }
        #[fragment]
        fn fs(v: V) -> @frag_color {
            if v.a < 0.5 { discard; }
            vec4(v.a)
        }
        #[vertex]
        fn vs() -> V { V { p: vec4(0.0), a: 1.0 } }");
}

#[test]
fn uniform_struct_and_builtin_params() {
    let out = ok("
        uniform struct Viewport { size: vec2, srgb: f32, _pad: f32 }
        #[bind(0, 0)]
        uniform viewport: Viewport;
        #[varying]
        struct V { @position p: vec4, @flat kind: u32, c: vec4 }
        #[vertex]
        fn vs(vi: @vertex_index) -> V {
            V { p: vec4(viewport.size / 2.0, 0.0, f32(vi)), kind: vi, c: vec4(viewport.srgb) }
        }");
    assert!(out.contains("@group(0) @binding(0)\nvar<uniform> viewport: Viewport;"), "{out}");
    assert!(out.contains("@location(0) @interpolate(flat) kind: u32"), "{out}");
    assert!(out.contains("@location(1) c: vec4<f32>"), "{out}");
}

#[test]
fn unused_functions_are_dropped() {
    let out = ok("
        #[varying]
        struct V { @position p: vec4 }
        fn unused() -> f32 { 1.0 }
        fn used() -> f32 { 2.0 }
        #[vertex]
        fn vs() -> V { V { p: vec4(used()) } }");
    assert!(out.contains("fn used()"), "{out}");
    assert!(!out.contains("fn unused()"), "{out}");
}

// ---- errors ----

#[test]
fn error_assign_to_immutable() {
    let e = err("fn f() -> f32 { let x = 1.0; x = 2.0; x }");
    assert!(e.contains("immutable") && e.contains("`x`"), "{e}");
}

#[test]
fn error_trailing_semicolon_in_value_function() {
    let e = err("fn f() -> f32 { 1.0; }");
    assert!(e.contains("last expression"), "{e}");
}

#[test]
fn error_mixed_int_and_float() {
    let e = err("fn f(a: f32) -> f32 { a + 1 }");
    assert!(e.contains("cannot apply `+`"), "{e}");
}

#[test]
fn error_const_must_be_compile_time() {
    let e = err("fn g() -> f32 { 1.0 }\nconst A: f32 = g();\nfn f() -> f32 { A }");
    assert!(e.contains("compile-time"), "{e}");
}

#[test]
fn error_const_cycle() {
    let e = err("const A: f32 = B;\nconst B: f32 = A;\nfn f() -> f32 { A }");
    assert!(e.contains("depends on itself"), "{e}");
}

#[test]
fn error_resource_without_bind() {
    let e = err("texture t: texture2d;");
    assert!(e.contains("#[bind"), "{e}");
}

#[test]
fn error_integer_varying_needs_flat() {
    let e = err("
        #[varying]
        struct V { @position p: vec4, k: u32 }
        #[vertex]
        fn vs() -> V { V { p: vec4(0.0), k: 1u } }");
    assert!(e.contains("@flat"), "{e}");
}

#[test]
fn error_unknown_name_and_function() {
    assert!(err("fn f() -> f32 { y }").contains("unknown name `y`"));
    assert!(err("fn f() -> f32 { nope(1.0) }").contains("unknown function `nope`"));
}

#[test]
fn error_wrong_argument_type() {
    let e = err("fn g(x: f32) -> f32 { x }\nfn f() -> f32 { g(1) }");
    assert!(e.contains("argument 1 of `g`"), "{e}");
}

#[test]
fn error_missing_struct_field() {
    let e = err("struct P { a: f32, b: f32 }\nfn f() -> P { P { a: 1.0 } }");
    assert!(e.contains("missing field `b`"), "{e}");
}

#[test]
fn error_parse_error_has_a_line() {
    let e = err("fn f() -> f32 {\n    let x = ;\n}");
    assert!(e.starts_with("line 2:"), "{e}");
}

#[test]
fn error_import_without_loader() {
    let e = err("import aimer::color::*;\nfn f() -> f32 { 1.0 }");
    assert!(e.contains("cannot import"), "{e}");
}
