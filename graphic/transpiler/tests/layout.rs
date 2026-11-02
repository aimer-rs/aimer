//! std140 uniform checks and vertex format metadata.

use aimer_transpiler::resolve::NoLoader;

fn program(uniform_fields: &str, input: &str) -> String {
    format!(
        "
shader t;

uniform struct U {{ {uniform_fields} }}
#[bind(0, 0)]
uniform u: U;

#[input(step = instance)]
struct I {{ {input} }}

#[varying]
struct V {{ @position position: vec4 }}

#[vertex]
fn vs_main(i: I) -> V {{
    V {{ position: vec4(0.0, 0.0, 0.0, 1.0) }}
}}
"
    )
}

fn err(uniform_fields: &str, input: &str) -> String {
    match aimer_transpiler::compile_wgsl(&program(uniform_fields, input), "t", &NoLoader) {
        Ok(_) => panic!("expected an error"),
        Err(e) => e.to_string(),
    }
}

fn ok(uniform_fields: &str, input: &str) {
    aimer_transpiler::compile_wgsl(&program(uniform_fields, input), "t", &NoLoader).unwrap_or_else(|e| panic!("{e}"));
}

fn layout_json(src: &str) -> String {
    let p = aimer_transpiler::load_program(src, "t", &NoLoader).unwrap_or_else(|e| panic!("{e}"));
    aimer_transpiler::layout::json(&p).unwrap()
}

const GOOD_INPUT: &str = "p: vec2";

#[test]
fn std140_accepts_padded_blocks() {
    ok("a: vec2, b: f32, _pad: f32", GOOD_INPUT);
    ok("a: vec4, m: mat4, rows: [vec4; 3]", GOOD_INPUT);
    ok("a: vec3, b: f32", GOOD_INPUT);
    ok("a: vec2, b: vec2, c: vec4", GOOD_INPUT);
}

#[test]
fn std140_rejects_a_size_that_is_not_a_multiple_of_16() {
    let e = err("a: vec2, b: f32", GOOD_INPUT);
    assert!(e.contains("rounds its size up to 16"), "{e}");
}

#[test]
fn std140_rejects_a_hole_between_fields() {
    let e = err("a: f32, b: vec2, c: vec4, d: f32, e: f32, f: f32", GOOD_INPUT);
    assert!(e.contains("field `b`") && e.contains("4 bytes of padding"), "{e}");
}

#[test]
fn std140_rejects_a_vec3_followed_by_a_vec2() {
    let e = err("a: vec3, b: vec2, c: f32, d: f32", GOOD_INPUT);
    assert!(e.contains("field `b`"), "{e}");
}

#[test]
fn std140_rejects_scalar_arrays_bool_and_small_matrices() {
    let e = err("a: [f32; 4]", GOOD_INPUT);
    assert!(e.contains("pads it to 16"), "{e}");
    let e = err("a: vec4, flag: bool, p: vec2, q: f32", GOOD_INPUT);
    assert!(e.contains("`bool` cannot be stored"), "{e}");
    let e = err("m: mat3", GOOD_INPUT);
    assert!(e.contains("mat3"), "{e}");
}

#[test]
fn vertex_formats_set_offsets_and_native_names() {
    let src = program(
        "a: vec4",
        "pos: vec2, color: vec4 as unorm8x4, flags: vec4<u32> as uint8x4, uv: vec2 as float16x2, id: u32, extra: vec4 as snorm16x4",
    );
    let j = layout_json(&src);
    for needle in [
        "\"stride\": 32",
        "\"step\": \"instance\"",
        "\"field\": \"pos\", \"offset\": 0, \"format\": \"float32x2\"",
        "\"field\": \"color\", \"offset\": 8, \"format\": \"unorm8x4\", \"metal\": \"MTLVertexFormatUChar4Normalized\", \"dxgi\": \"DXGI_FORMAT_R8G8B8A8_UNORM\"",
        "\"field\": \"flags\", \"offset\": 12",
        "\"field\": \"uv\", \"offset\": 16, \"format\": \"float16x2\", \"metal\": \"MTLVertexFormatHalf2\"",
        "\"field\": \"id\", \"offset\": 20, \"format\": \"uint32\"",
        "\"field\": \"extra\", \"offset\": 24, \"format\": \"snorm16x4\"",
        "\"size\": 16",
    ] {
        assert!(j.contains(needle), "missing `{needle}` in:\n{j}");
    }
}

#[test]
fn vertex_format_errors() {
    let e = err("a: vec4", "c: vec4 as unorm8x3");
    assert!(e.contains("unknown vertex format"), "{e}");
    let e = err("a: vec4", "c: vec3 as unorm8x4");
    assert!(e.contains("read as vec4") && e.contains("vec3"), "{e}");
    let e = err("a: vec4", "c: vec4 as uint8x4");
    assert!(e.contains("u32"), "{e}");
    let e = err("a: vec4", "m: mat4");
    assert!(e.contains("cannot be a vertex attribute"), "{e}");
}

#[test]
fn format_is_only_for_inputs() {
    let src = "
shader t;
#[varying]
struct V { @position position: vec4, c: vec4 as unorm8x4 }
#[vertex]
fn vs_main(vi: @vertex_index) -> V { V { position: vec4(0.0), c: vec4(0.0) } }
";
    let e = aimer_transpiler::compile_wgsl(src, "t", &NoLoader).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(e.contains("only valid in an `#[input]` struct"), "{e}");
}

#[test]
fn glsl_and_msl_keep_the_shader_type_for_normalised_formats() {
    let src = program("a: vec4", "color: vec4 as unorm8x4");
    let msl = aimer_transpiler::compile_msl(&src, "t", &NoLoader).unwrap();
    assert!(msl.contains("float4 color"), "{msl}");
    let glsl = aimer_transpiler::compile_glsl(&src, "t", &NoLoader).unwrap();
    assert!(glsl.vertex.unwrap().contains("in vec4 _p2vs_location0"));
}
