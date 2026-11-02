//! Metal and HLSL emitters. There is no MSL or HLSL compiler in the test run, so these tests check the
//! generated text for the constructs that differ between the targets.

use std::path::PathBuf;

use aimer_transpiler::resolve::{FsLoader, NoLoader};

fn msl(src: &str) -> String {
    aimer_transpiler::compile_msl(src, "t", &NoLoader).unwrap_or_else(|e| panic!("msl compile failed: {e}"))
}

fn hlsl(src: &str) -> String {
    aimer_transpiler::compile_hlsl(src, "t", &NoLoader).unwrap_or_else(|e| panic!("hlsl compile failed: {e}"))
}

fn msl_err(src: &str) -> String {
    match aimer_transpiler::compile_msl(src, "t", &NoLoader) {
        Ok(out) => panic!("expected an error, got:\n{out}"),
        Err(e) => e.to_string(),
    }
}

fn hlsl_err(src: &str) -> String {
    match aimer_transpiler::compile_hlsl(src, "t", &NoLoader) {
        Ok(out) => panic!("expected an error, got:\n{out}"),
        Err(e) => e.to_string(),
    }
}

fn has(out: &str, needles: &[&str]) {
    for n in needles {
        assert!(out.contains(n), "missing `{n}` in:\n{out}");
    }
}

fn lacks(out: &str, needles: &[&str]) {
    for n in needles {
        assert!(!out.contains(n), "unexpected `{n}` in:\n{out}");
    }
}

fn compile_file(name: &str, f: fn(&str, &str, &dyn aimer_transpiler::resolve::ModuleLoader) -> aimer_transpiler::error::Result<String>) -> String {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders-src");
    let src = std::fs::read_to_string(dir.join(format!("{name}.aimer"))).unwrap();
    f(&src, name, &FsLoader { root: dir.join("lib") }).unwrap_or_else(|e| panic!("{name}: {e}"))
}

const RESOURCES: &str = "
    uniform struct Viewport { size: vec2, srgb: f32, _pad: f32 }
    #[bind(0, 0)]
    uniform viewport: Viewport;
    #[bind(1, 0)]
    texture t: texture2d;
    #[bind(1, 1)]
    sampler s;
    #[varying]
    struct V { @position position: vec4, uv: vec2 }
";

#[test]
fn ported_shaders_compile_for_both_targets() {
    for name in ["frame_composite", "svg"] {
        compile_file(name, aimer_transpiler::compile_msl);
        compile_file(name, aimer_transpiler::compile_hlsl);
    }
}

#[test]
fn frame_composite_matches_the_hand_written_shape() {
    let m = compile_file("frame_composite", aimer_transpiler::compile_msl);
    has(
        &m,
        &[
            "#include <metal_stdlib>",
            "float4 position [[position]];",
            "constant float2 CORNERS[6] = {",
            "vertex VertexOutput vs_main(uint vertex_index [[vertex_id]])",
            "fragment float4 fs_main(float4 position [[position]], texture2d<float> source_frame [[texture(0)]])",
            "source_frame.read(uint2(int2(position.xy)), uint(0))",
        ],
    );
    let h = compile_file("frame_composite", aimer_transpiler::compile_hlsl);
    has(
        &h,
        &[
            "float4 position : SV_Position;",
            "Texture2D<float4> source_frame : register(t0, space0);",
            "static const float2 CORNERS[6] = {",
            "VertexOutput vs_main(uint vertex_index : SV_VertexID)",
            "float4 fs_main(float4 position : SV_Position) : SV_Target0",
            "source_frame.Load(int3(((int2)(position.xy)), 0))",
        ],
    );
}

#[test]
fn svg_keeps_the_original_interface() {
    let m = compile_file("svg", aimer_transpiler::compile_msl);
    has(
        &m,
        &[
            "float4 position [[position]];",
            "float4 color [[user(loc0)]];",
            "float coverage [[user(loc5)]];",
            "float2 vertex_position [[attribute(0)]];",
        ],
    );
    has(&m, &["float vertex_coverage [[attribute(7)]];", "float4 instance_viewport [[attribute(6)]];"]);
    has(&m, &["vertex VertexOutput vs_main(vs_main_inputs inputs [[stage_in]])", "fragment float4 fs_main(VertexOutput input [[stage_in]])"]);
    let h = compile_file("svg", aimer_transpiler::compile_hlsl);
    has(
        &h,
        &[
            "float2 position : TEXCOORD0;",
            "float coverage : TEXCOORD7;",
            "float4 viewport : TEXCOORD6;",
            "float coverage : TEXCOORD5;",
            "VertexOutput vs_main(SvgVertex vertex, SvgInstance instance)",
            "float4 fs_main(VertexOutput input) : SV_Target0",
        ],
    );
}

#[test]
fn msl_threads_resources_through_helpers() {
    let src = format!(
        "{RESOURCES}
        fn scale(p: vec2) -> vec2 {{ p / viewport.size }}
        fn sample_at(uv: vec2) -> vec4 {{ t.sample(s, scale(uv)) }}
        #[fragment]
        fn fs_main(v: V) -> @frag_color {{ sample_at(v.uv) }}
        #[vertex]
        fn vs_main() -> V {{ V {{ position: vec4(scale(vec2(1.0)), 0.0, 1.0), uv: vec2(0.0) }} }}"
    );
    let m = msl(&src);
    has(
        &m,
        &[
            "static float2 scale(float2 p, constant Viewport& viewport)",
            "static float4 sample_at(float2 uv, constant Viewport& viewport, texture2d<float> t, sampler s)",
            "return t.sample(s, scale(uv, viewport));",
            "return sample_at(v.uv, viewport, t, s);",
            // group * 4 + binding
            "fragment float4 fs_main(V v [[stage_in]], constant Viewport& viewport [[buffer(0)]], texture2d<float> t [[texture(4)]], sampler s [[sampler(5)]])",
            "vertex V vs_main(constant Viewport& viewport [[buffer(0)]])",
        ],
    );
    // The vertex entry does not read the texture, so it does not take it.
    let vs = m.split("vertex V vs_main").nth(1).unwrap().lines().next().unwrap();
    assert!(!vs.contains("texture2d"), "{vs}");
}

#[test]
fn hlsl_uses_globals_with_register_spaces() {
    let src = format!(
        "{RESOURCES}
        fn scale(p: vec2) -> vec2 {{ p / viewport.size }}
        #[fragment]
        fn fs_main(v: V) -> @frag_color {{ t.sample(s, scale(v.uv)) }}"
    );
    let h = hlsl(&src);
    has(
        &h,
        &[
            "ConstantBuffer<Viewport> viewport : register(b0, space0);",
            "Texture2D<float4> t : register(t0, space1);",
            "SamplerState s : register(s1, space1);",
            "float2 scale(float2 p) {",
            "return t.Sample(s, scale(v.uv));",
        ],
    );
    lacks(&h, &["constant Viewport&"]);
}

#[test]
fn texture_methods_per_target() {
    let src = format!(
        "{RESOURCES}
        #[fragment]
        fn fs_main(v: V) -> @frag_color {{
            let a = t.sample_level(s, v.uv, 0.0);
            let b = t.load(vec2<i32>(1, 2), 0);
            let c = t.size();
            a + b + vec4(f32(c.x))
        }}"
    );
    let m = msl(&src);
    has(
        &m,
        &[
            "t.sample(s, v.uv, level(0.0))",
            "t.read(uint2(int2(1, 2)), uint(0))",
            "uint2(t.get_width(), t.get_height())",
        ],
    );
    let h = hlsl(&src);
    has(
        &h,
        &[
            "t.SampleLevel(s, v.uv, 0.0)",
            "t.Load(int3(int2(1, 2), 0))",
            "_aimer_texture_size(t)",
            "uint2 _aimer_texture_size(Texture2D<float4> t) {",
            "t.GetDimensions(w, h);",
        ],
    );
    // The helper only exists when `size()` is used.
    let h2 = hlsl(&format!("{RESOURCES} #[fragment] fn fs_main(v: V) -> @frag_color {{ t.sample(s, v.uv) }}"));
    lacks(&h2, &["_aimer_texture_size"]);
}

#[test]
fn single_input_struct_is_the_stage_in_struct() {
    let src = "
        #[input(step = vertex)]
        struct I { @location(0) pos: vec2, @location(3) color: vec4 }
        #[varying]
        struct V { @position position: vec4, color: vec4 }
        #[vertex]
        fn vs_main(i: I) -> V { V { position: vec4(i.pos, 0.0, 1.0), color: i.color } }";
    let m = msl(src);
    has(
        &m,
        &[
            "float2 pos [[attribute(0)]];",
            "float4 color [[attribute(3)]];",
            "vertex V vs_main(I i [[stage_in]])",
        ],
    );
    lacks(&m, &["vs_main_inputs"]);
    let h = hlsl(src);
    has(&h, &["float2 pos : TEXCOORD0;", "float4 color : TEXCOORD3;", "V vs_main(I i)"]);
}

#[test]
fn several_input_structs_are_merged_in_msl_only() {
    let src = "
        #[input(step = vertex)]
        struct A { pos: vec2 }
        #[input(step = instance)]
        struct B { offset: vec2 }
        #[varying]
        struct V { @position position: vec4 }
        #[vertex]
        fn vs_main(a: A, b: B) -> V { V { position: vec4(a.pos + b.offset, 0.0, 1.0) } }";
    let m = msl(src);
    has(
        &m,
        &[
            "struct vs_main_inputs {",
            "float2 a_pos [[attribute(0)]];",
            "float2 b_offset [[attribute(1)]];",
            "vertex V vs_main(vs_main_inputs inputs [[stage_in]])",
            "A a;",
            "a.pos = inputs.a_pos;",
            "b.offset = inputs.b_offset;",
        ],
    );
    // The members of the merged struct carry the attributes, the plain structs do not.
    let a = m.split("struct A {").nth(1).unwrap().split("};").next().unwrap();
    assert!(!a.contains("attribute"), "{a}");
    let h = hlsl(src);
    has(&h, &["float2 pos : TEXCOORD0;", "float2 offset : TEXCOORD1;", "V vs_main(A a, B b)"]);
}

#[test]
fn flat_varyings() {
    let src = "
        #[varying]
        struct V { @position position: vec4, @flat kind: u32, @flat @location(4) other: i32 }
        #[vertex]
        fn vs_main() -> V { V { position: vec4(0.0), kind: 1u, other: 2 } }";
    has(&msl(src), &["uint kind [[user(loc0), flat]];", "int other [[user(loc4), flat]];"]);
    has(&hlsl(src), &["nointerpolation uint kind : TEXCOORD0;", "nointerpolation int other : TEXCOORD4;"]);
}

#[test]
fn intrinsics_are_renamed_per_target() {
    let src = "fn f(a: f32, b: f32, v: vec2) -> f32 {
        sign(a) + round(a) + mix(a, b, 0.5) + fract(a) + inverseSqrt(a) + fma(a, b, a) + dpdx(a) + dpdy(b) + degrees(a) + radians(b) + dot(v, v)
    }";
    let m = msl(src);
    has(&m, &["sign(a)", "rint(a)", "mix(a, b, 0.5)", "fract(a)", "rsqrt(a)", "fma(a, b, a)", "dfdx(a)", "dfdy(b)", "(a * 57.29578)", "(b * 0.017453292)", "dot(v, v)"]);
    let h = hlsl(src);
    has(&h, &["((float)sign(a))", "round(a)", "lerp(a, b, 0.5)", "frac(a)", "rsqrt(a)", "mad(a, b, a)", "ddx(a)", "ddy(b)", "degrees(a)", "radians(b)", "dot(v, v)"]);
}

#[test]
fn remainder_of_floats() {
    let src = "fn f(a: f32, b: f32, i: i32) -> f32 { (a % b) + f32(i % 3) }";
    has(&msl(src), &["fmod(a, b)", "i % 3"]);
    has(&hlsl(src), &["a % b", "i % 3"]);
}

#[test]
fn constructors_and_casts() {
    let src = "fn f(a: f32, v: vec2, u: u32) -> vec4 { vec4(vec3(a), f32(u)) + vec4(v, v) + vec4(f32(vec2<i32>(v).x)) }";
    let m = msl(src);
    has(&m, &["float4(float3(a), float(u))", "float4(v, v)", "float(int2(v).x)"]);
    let h = hlsl(src);
    has(&h, &["float4(((float3)(a)), ((float)(u)))", "float4(v, v)", "((float)(((int2)(v)).x))"]);
}

#[test]
fn bitcast() {
    let src = "fn f(u: u32, f: f32) -> f32 { bitcast<f32>(u) + f32(bitcast<u32>(f)) + f32(bitcast<i32>(f)) }";
    has(&msl(src), &["as_type<float>(u)", "as_type<uint>(f)", "as_type<int>(f)"]);
    has(&hlsl(src), &["asfloat(u)", "asuint(f)", "asint(f)"]);
}

#[test]
fn match_becomes_switch_unless_an_arm_breaks() {
    let plain = "fn f(k: u32) -> f32 { match k { 0u => 1.0, 1u | 2u => 2.0, _ => 3.0 } }";
    for out in [msl(plain), hlsl(plain)] {
        has(&out, &["switch (k) {", "case 0u: {", "case 1u: case 2u: {", "default: {", "break;"]);
    }
    // A `break` inside a `switch` would leave the switch, so this one is an `if` chain.
    let with_break = "
        fn f(k: u32) -> f32 {
            let mut acc = 0.0;
            loop {
                match k {
                    0u => { break; }
                    1u => { acc += 1.0; }
                    _ => { acc += 2.0; }
                }
                acc += 0.5;
            }
            acc
        }";
    for out in [msl(with_break), hlsl(with_break)] {
        has(&out, &["if (k == 0u) {", "} else if (k == 1u) {", "} else {"]);
        lacks(&out, &["switch"]);
    }
}

#[test]
fn match_with_ranges_is_an_if_chain() {
    let src = "fn f(k: i32) -> f32 { match k { 0..=3 => 1.0, 4 | 7 => 2.0, 8..10 => 4.0, _ => 3.0 } }";
    for out in [msl(src), hlsl(src)] {
        has(&out, &["if (k >= 0 && k <= 3) {", "} else if (k == 4 || k == 7) {", "} else if (k >= 8 && k < 10) {"]);
    }
}

#[test]
fn loops_and_values() {
    let src = "
        fn f(n: u32) -> f32 {
            let mut acc = 0.0;
            for i in 0u..n { acc += 1.0; }
            let mut j = 0u;
            while j < n { j += 1u; }
            let r = loop { if j > 9u { break acc * 2.0; } j += 1u; };
            r
        }";
    for out in [msl(src), hlsl(src)] {
        has(&out, &["for (uint i = 0u; i < n; i++) {", "while (j < n) {", "float r;", "while (true) {", "r = acc * 2.0;", "break;"]);
    }
}

#[test]
fn loop_as_tail_value_returns_from_inside() {
    let src = "fn f(n: u32) -> u32 { let mut i = 0u; loop { if i >= n { break i; } i += 1u; } }";
    for out in [msl(src), hlsl(src)] {
        has(&out, &["while (true) {", "return i;"]);
        lacks(&out, &["_t0"]);
    }
}

#[test]
fn if_and_select() {
    let src = "fn f(c: bool, a: f32, b: f32) -> f32 {
        let x = if c { a * 2.0 } else { b };
        let y = #[select] if c { a } else { b };
        x + y
    }";
    for out in [msl(src), hlsl(src)] {
        has(&out, &["float x;", "if (c) {", "x = a * 2.0;", "float y = (c ? a : b);"]);
    }
    // `#[select]` evaluates both arms, like the WGSL `select` it stands for: a call is fine when the
    // function is pure, and an error when it could discard or take derivatives.
    has(&msl("fn g(x: f32) -> f32 { x } fn f(c: bool, a: f32) -> f32 { #[select] if c { g(a) } else { a } }"), &["(c ? g(a) : a)"]);
    for impure in [
        "fn g(x: f32) -> f32 { if x < 0.0 { discard; } x }",
        "fn g(x: f32) -> f32 { fwidth(x) }",
        "fn g(x: f32) -> f32 { h(x) } fn h(x: f32) -> f32 { dpdx(x) }",
    ] {
        let e = msl_err(&format!("{impure} fn f(c: bool, a: f32) -> f32 {{ #[select] if c {{ g(a) }} else {{ a }} }}"));
        assert!(e.contains("pure functions"), "{e}");
    }
}

#[test]
fn logical_operators_and_short_circuit() {
    let src = "
        const T: [f32; 3] = [1.0, 2.0, 3.0];
        fn f(i: u32, a: bool) -> bool { a && i < 3u && T[i] > 1.0 }";
    // MSL short-circuits like WGSL.
    has(&msl(src), &["a && i < 3u && T[i] > 1.0"]);
    // HLSL may evaluate both sides, so an index on the right is guarded by an `if`.
    let h = hlsl(src);
    has(&h, &["bool _t0 = a && i < 3u;", "if (_t0) {", "_t0 = T[i] > 1.0;"]);
    let plain = "fn f(a: bool, b: bool, c: bool) -> bool { a && (b || c) }";
    has(&hlsl(plain), &["a && (b || c)"]);
}

#[test]
fn struct_literals_arrays_and_swizzles() {
    let src = "
        struct S { a: f32, b: vec2 }
        fn f(x: f32) -> f32 {
            let s = S { a: x, b: vec2(1.0, 2.0) };
            let t = S { a: 2.0, ..s };
            let arr = [1.0, 2.0, 3.0];
            let mut v = vec4(0.0);
            v.xy = vec2(x, x);
            v.zw += vec2(1.0);
            let mut z: S;
            z.a = arr[1];
            t.a + s.b.x + v.x + z.a
        }";
    let m = msl(src);
    has(
        &m,
        &[
            "S _t0;",
            "_t0.a = x;",
            "_t0.b = float2(1.0, 2.0);",
            "S _t1 = s;",
            "_t1.a = 2.0;",
            "float arr[3] = { 1.0, 2.0, 3.0 };",
            "v.xy = float2(x, x);",
            "v.zw += float2(1.0);",
            "S z = S{};",
        ],
    );
    let h = hlsl(src);
    has(&h, &["float arr[3] = { 1.0, 2.0, 3.0 };", "v.zw += ((float2)(1.0));", "S z = (S)0;"]);
}

#[test]
fn repeated_swizzle_component_is_an_error() {
    let src = "fn f() -> vec2 { let mut v = vec2(0.0); v.xx = vec2(1.0); v }";
    assert!(msl_err(src).contains("appears twice"));
    assert!(hlsl_err(src).contains("appears twice"));
}

#[test]
fn const_inlining_and_runtime_tables() {
    let src = "
        const K: f32 = 1.5;
        const HALF: f32 = K * 0.5;
        const TABLE: [vec2; 2] = [vec2(1.0, 2.0), vec2(3.0, 4.0)];
        fn f(i: u32) -> f32 { HALF + TABLE[0].x + TABLE[i].y }";
    for out in [msl(src), hlsl(src)] {
        has(&out, &["(1.5 * 0.5)", "float2(1.0, 2.0)", "TABLE[i].y"]);
        lacks(&out, &["K "]);
    }
    has(&msl(src), &["constant float2 TABLE[2] = {"]);
    has(&hlsl(src), &["static const float2 TABLE[2] = {"]);
}

#[test]
fn reserved_names_are_renamed() {
    let src = "fn f(in: f32, out: f32, half: f32) -> f32 { let distance = in + out; let float2 = half; distance + float2 }";
    for out in [msl(src), hlsl(src)] {
        has(&out, &["float in_, float out_, float half_", "float distance_ = in_ + out_;", "float float2_ = half_;"]);
    }
}

#[test]
fn callees_are_written_before_their_callers() {
    let src = "fn a() -> f32 { b() + 1.0 } fn b() -> f32 { c() } fn c() -> f32 { 2.0 }";
    for out in [msl(src), hlsl(src)] {
        let (a, b, c) = (out.find("float a()").unwrap(), out.find("float b()").unwrap(), out.find("float c()").unwrap());
        assert!(c < b && b < a, "{out}");
    }
}

#[test]
fn structs_are_written_before_the_structs_that_contain_them() {
    let src = "struct Outer { inner: Inner } struct Inner { x: f32 } fn f(o: Outer) -> f32 { o.inner.x }";
    for out in [msl(src), hlsl(src)] {
        assert!(out.find("struct Inner").unwrap() < out.find("struct Outer").unwrap(), "{out}");
    }
}

#[test]
fn discard_per_target() {
    let src = "#[fragment] fn fs_main(c: f32) -> @frag_color { if c < 0.5 { discard; } vec4(c) }";
    has(&msl(src), &["discard_fragment();"]);
    has(&hlsl(src), &["discard;"]);
}

#[test]
fn matrices_work_in_msl_only() {
    let src = "fn f(m: mat2, v: vec2) -> vec2 { m * v }";
    has(&msl(src), &["float2x2 m", "m * v"]);
    assert!(hlsl_err(src).contains("matrices are not supported by the HLSL"));
}

#[test]
fn errors_name_the_function() {
    let e = msl_err("fn f() -> f32 { 1.0; }");
    assert!(e.contains("in fn `f`") && e.contains("remove it to return the value"), "{e}");
}

#[test]
fn hlsl_resources_are_declared_after_the_structs_they_use() {
    let h = hlsl(&format!("{RESOURCES} #[fragment] fn fs_main(v: V) -> @frag_color {{ t.sample(s, v.uv) * viewport.srgb }}"));
    assert!(h.find("struct Viewport").unwrap() < h.find("ConstantBuffer<Viewport>").unwrap(), "{h}");
}
