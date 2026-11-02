//! GLSL 3.30 core and GLSL ES 3.00 emitters. The output is compiled and linked with `glslangValidator`
//! (Khronos reference compiler) when it is installed; without it those checks print a skip note, and the
//! text checks still run.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use aimer_transpiler::emit::clike::GlslOutput;
use aimer_transpiler::resolve::{FsLoader, ModuleLoader, NoLoader};

#[derive(Clone, Copy, PartialEq)]
enum Profile {
    Desktop,
    Es,
}

fn compile(src: &str, name: &str, p: Profile, loader: &dyn ModuleLoader) -> Result<GlslOutput, String> {
    let r = match p {
        Profile::Desktop => aimer_transpiler::compile_glsl(src, name, loader),
        Profile::Es => aimer_transpiler::compile_glsl_es(src, name, loader),
    };
    r.map_err(|e| e.to_string())
}

fn glsl(src: &str, p: Profile) -> GlslOutput {
    compile(src, "t", p, &NoLoader).unwrap_or_else(|e| panic!("glsl compile failed: {e}"))
}

fn glsl_err(src: &str) -> String {
    match compile(src, "t", Profile::Desktop, &NoLoader) {
        Ok(o) => panic!("expected an error, got:\n{}", o.vertex.unwrap_or_default()),
        Err(e) => e,
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

fn compile_file(name: &str, p: Profile) -> GlslOutput {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders-src");
    let src = std::fs::read_to_string(dir.join(format!("{name}.aimer"))).unwrap();
    compile(&src, name, p, &FsLoader { root: dir.join("lib") }).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn validator_available() -> bool {
    Command::new("glslangValidator").arg("--version").output().is_ok_and(|o| o.status.success())
}

/// Compile every stage with glslangValidator, then link them together.
fn validate(label: &str, o: &GlslOutput) {
    if !validator_available() {
        eprintln!("skipping glslangValidator check for `{label}`: glslangValidator is not installed");
        return;
    }
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!("aimer_glsl_{}_{}", std::process::id(), COUNTER.fetch_add(1, Ordering::SeqCst)));
    std::fs::create_dir_all(&dir).unwrap();
    let mut files = Vec::new();
    for (ext, text) in [("vert", &o.vertex), ("frag", &o.fragment)] {
        if let Some(text) = text {
            let f = dir.join(format!("s.{ext}"));
            std::fs::write(&f, text).unwrap();
            files.push(f);
        }
    }
    for f in &files {
        let out = Command::new("glslangValidator").arg(f).output().expect("run glslangValidator");
        assert!(
            out.status.success(),
            "{label}: glslangValidator rejected {}:\n{}{}\n---\n{}",
            f.display(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
            std::fs::read_to_string(f).unwrap()
        );
    }
    if files.len() > 1 {
        let out = Command::new("glslangValidator").arg("-l").args(&files).output().expect("run glslangValidator");
        assert!(
            out.status.success(),
            "{label}: link failed:\n{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Compile for both profiles, validate both, and return (desktop, es).
fn both(src: &str) -> (GlslOutput, GlslOutput) {
    let d = glsl(src, Profile::Desktop);
    let e = glsl(src, Profile::Es);
    validate("desktop", &d);
    validate("es", &e);
    (d, e)
}

const WIDE: &str = "
shader t;

uniform struct Viewport { size: vec2, srgb: f32, _pad: f32 }

#[bind(0, 0)]
uniform viewport: Viewport;
#[bind(1, 0)]
texture t: texture2d;
#[bind(1, 1)]
sampler s;

#[input(step = instance)]
struct Inst { offset: vec2, tint: vec4 }

#[varying]
struct V { @position position: vec4, @flat id: u32, uv: vec2, tint: vec4 }

const TABLE: [f32; 3] = [1.0, 2.0, 3.0];

fn rem2(a: vec2, b: vec2) -> vec2 { a % b }

#[vertex]
fn vs_main(i: @vertex_index, inst: Inst) -> V {
    let mut out: V;
    out.position = vec4(inst.offset / viewport.size, 0.0, 1.0);
    out.id = i;
    out.uv = rem2(inst.offset, vec2(2.0, 3.0));
    out.tint = inst.tint;
    out
}

#[fragment]
fn fs_main(v: V, pos: @frag_coord) -> @frag_color {
    let c = t.sample(s, v.uv);
    let e = t.load(vec2<i32>(pos.xy), 0);
    let lt = v.uv < vec2(0.5, 0.5);
    let m = if all(lt) { c } else { v.tint };
    if any(!lt) { discard; }
    let k = bitcast<f32>(v.id) * TABLE[v.id];
    m * k + e * f32(t.size().x)
}
";

#[test]
fn ported_shaders_compile_and_link() {
    for name in ["frame_composite", "svg"] {
        for p in [Profile::Desktop, Profile::Es] {
            let o = compile_file(name, p);
            assert!(o.vertex.is_some() && o.fragment.is_some());
            validate(name, &o);
        }
    }
}

#[test]
fn frame_composite_matches_the_hand_written_shape() {
    let o = compile_file("frame_composite", Profile::Desktop);
    let (v, f) = (o.vertex.unwrap(), o.fragment.unwrap());
    has(
        &v,
        &[
            "#version 330 core",
            "const vec2 CORNERS[6] = vec2[6](",
            "uint(gl_VertexID)",
            "gl_Position = _out.position;",
            "void main() {",
        ],
    );
    has(
        &f,
        &[
            "#version 330 core",
            "uniform sampler2D _smp_source_frame;",
            "layout(location = 0) out vec4 _fs2p_location0;",
            "texelFetch(_smp_source_frame, ivec2(position.xy), 0)",
            "gl_FragCoord",
        ],
    );
    lacks(&f, &["CORNERS", "gl_VertexID"]);
    has(&o.layout, &["\"shader\": \"frame_composite\"", "\"uniform\": \"_smp_source_frame\"", "\"binding\": 0"]);
}

#[test]
fn es_profile_differs_in_version_and_precision_only() {
    let d = compile_file("svg", Profile::Desktop);
    let e = compile_file("svg", Profile::Es);
    let (dv, ev) = (d.vertex.unwrap(), e.vertex.unwrap());
    has(&dv, &["#version 330 core"]);
    lacks(&dv, &["precision"]);
    has(&ev, &["#version 300 es", "precision highp float;", "precision highp int;", "precision highp sampler2D;"]);
    let strip = |s: &str| {
        s.lines().filter(|l| !l.is_empty() && !l.starts_with("#version") && !l.starts_with("precision")).collect::<Vec<_>>().join("\n")
    };
    assert_eq!(strip(&dv), strip(&ev));
    assert_eq!(d.fragment.map(|f| strip(&f)), e.fragment.map(|f| strip(&f)));
}

#[test]
fn svg_keeps_the_original_interface() {
    let o = compile_file("svg", Profile::Desktop);
    let (v, f) = (o.vertex.unwrap(), o.fragment.unwrap());
    has(
        &v,
        &[
            "layout(location = 0) in vec2 _p2vs_location0;",
            "layout(location = 7) in float _p2vs_location7;",
            "layout(location = 6) in vec4 _p2vs_location6;",
            "out vec4 _vs2fs_location0;",
            "out float _vs2fs_location5;",
            "VertexOutput vs_main(SvgVertex vertex, SvgInstance instance)",
        ],
    );
    has(&f, &["in vec4 _vs2fs_location0;", "in float _vs2fs_location5;", "vec4 fs_main(VertexOutput input_)"]);
    lacks(&f, &["_p2vs_location", "SvgInstance", "SvgVertex"]);
    has(&o.layout, &["\"step\": \"instance\"", "\"step\": \"vertex\"", "\"location\": 7"]);
}

#[test]
fn stages_only_get_what_they_use() {
    let (d, _) = both(WIDE);
    let (v, f) = (d.vertex.unwrap(), d.fragment.unwrap());
    has(&v, &["layout(std140) uniform viewport_block_vertex { Viewport viewport_vs; };", "viewport_vs.size", "struct Inst"]);
    lacks(&v, &["sampler2D", "TABLE", "_aimer_rem(float", "struct Inst {\n    vec2 offset;\n    vec4 tint;\n};\n\nstruct Inst"]);
    has(&f, &["uniform sampler2D _smp_t_s;", "uniform sampler2D _smp_t;"]);
    lacks(&f, &["viewport_vs", "uniform viewport_block", "struct Inst", "rem2", "_p2vs_location"]);
}

#[test]
fn uniform_blocks_and_samplers_land_in_the_layout() {
    let (d, _) = both(WIDE);
    has(
        &d.layout,
        &[
            "\"block\": \"viewport_block_vertex\", \"resource\": \"viewport\", \"group\": 0, \"binding\": 0",
            "\"uniform\": \"_smp_t_s\", \"texture\": { \"name\": \"t\", \"group\": 1, \"binding\": 0 }, \"sampler\": { \"name\": \"s\", \"group\": 1, \"binding\": 1 }",
            "\"uniform\": \"_smp_t\", \"texture\": { \"name\": \"t\", \"group\": 1, \"binding\": 0 }, \"sampler\": null",
            "\"type\": \"vec2\", \"step\": \"instance\"",
        ],
    );
}

#[test]
fn lowering_uses_glsl_spellings() {
    let (d, _) = both(WIDE);
    let (v, f) = (d.vertex.unwrap(), d.fragment.unwrap());
    // Varyings are matched by name; an integer varying is `flat` on both sides.
    has(&v, &["flat out uint _vs2fs_location0;", "out vec2 _vs2fs_location1;", "uint(gl_VertexID)"]);
    has(&f, &["flat in uint _vs2fs_location0;", "in vec2 _vs2fs_location1;", "V(gl_FragCoord, _vs2fs_location0"]);
    // Float `%` is a truncated remainder; GLSL has no `%` on floats.
    has(&v, &["vec2 _aimer_rem(vec2 x, vec2 y) {", "return x - y * trunc(x / y);", "_aimer_rem(a, b)"]);
    // Vector comparisons, `!` on vectors, texture calls, casts and bit casts.
    has(
        &f,
        &[
            "lessThan(v.uv, vec2(0.5, 0.5))",
            "not(lt)",
            "texture(_smp_t_s, v.uv)",
            "texelFetch(_smp_t, ivec2(pos.xy), 0)",
            "uvec2(textureSize(_smp_t, 0))",
            "uintBitsToFloat(v.id)",
            "discard;",
            "const float TABLE[3] = float[3](",
        ],
    );
    // A zero initialised struct is spelled out, since GLSL has no `{}` or `(T)0`.
    has(&v, &["V out_ = V(vec4(0), 0u, vec2(0), vec4(0));"]);
}

#[test]
fn zero_initialised_struct_and_array_locals() {
    let src = "
shader t;
#[varying]
struct V { @position position: vec4, uv: vec2 }
#[vertex]
fn vs_main(i: @vertex_index) -> V {
    let mut out: V;
    let mut a: [f32; 2];
    a[0] = 1.0;
    let b = [1.0, 2.0, 3.0];
    out.position = vec4(a[0] + b[i], 0.0, 0.0, 1.0);
    out
}
";
    let (d, _) = both(src);
    let v = d.vertex.unwrap();
    has(&v, &["V out_ = V(vec4(0), vec2(0));", "float a[2] = float[2](0.0, 0.0);", "float b[3] = float[3](1.0, 2.0, 3.0);"]);
    assert!(d.fragment.is_none(), "no fragment entry means no fragment shader");
}

#[test]
fn reserved_glsl_names_are_renamed() {
    let src = "
shader t;
#[varying]
struct V { @position position: vec4, uv: vec2 }
fn filter(input: vec2, output: f32) -> f32 { input.x + output }
#[vertex]
fn vs_main(i: @vertex_index) -> V {
    let sample = filter(vec2(1.0, 2.0), 3.0);
    let gl_thing = sample * 2.0;
    V { position: vec4(gl_thing, 0.0, 0.0, 1.0), uv: vec2(0.0, 0.0) }
}
";
    let (d, _) = both(src);
    let v = d.vertex.unwrap();
    has(&v, &["float filter_(vec2 input_, float output_)", "filter_(vec2(1.0, 2.0), 3.0)", "v_gl_thing"]);
}

#[test]
fn fma_is_rejected() {
    let src = "
shader t;
#[varying]
struct V { @position position: vec4 }
#[vertex]
fn vs_main(i: @vertex_index) -> V {
    V { position: vec4(fma(1.0, 2.0, 3.0), 0.0, 0.0, 1.0) }
}
";
    let e = glsl_err(src);
    assert!(e.contains("fma"), "{e}");
}

#[test]
fn texture_parameters_get_one_copy_per_binding() {
    let src = "
shader t;
#[bind(0, 0)]
texture a: texture2d;
#[bind(0, 1)]
texture b: texture2d;
#[bind(0, 2)]
sampler smp;
#[varying]
struct V { @position position: vec4, uv: vec2 }
#[vertex]
fn vs_main(i: @vertex_index) -> V {
    V { position: vec4(0.0, 0.0, 0.0, 1.0), uv: vec2(0.0, 0.0) }
}
fn grab(tex: texture2d, uv: vec2) -> vec4 { tex.sample_level(smp, uv, 0.0) }
fn both(first: texture2d, second: texture2d, uv: vec2) -> vec4 { grab(first, uv) + grab(second, uv) + grab(first, uv + 1.0) }
#[fragment]
fn fs_main(v: V) -> @frag_color {
    both(a, b, v.uv) + both(b, a, v.uv) + grab(a, v.uv)
}
";
    let (d, e) = both(src);
    for o in [&d, &e] {
        let f = o.fragment.as_ref().unwrap();
        // One copy per texture the helper is called with, prototyped before its first use.
        has(f, &["vec4 grab_a(vec2 uv);", "vec4 grab_b(vec2 uv);", "vec4 both_a_b(vec2 uv)", "vec4 both_b_a(vec2 uv)"]);
        has(f, &["textureLod(_smp_a_smp, uv, 0.0)", "textureLod(_smp_b_smp, uv, 0.0)", "grab_a(v.uv)", "both_a_b(v.uv)"]);
        lacks(f, &["texture2d", "vec4 grab(", "sampler2D tex"]);
        // Texture arguments never reach the call.
        lacks(f, &["grab_a(a", "both_a_b(a"]);
    }
    has(&d.layout, &["\"uniform\": \"_smp_a_smp\"", "\"uniform\": \"_smp_b_smp\""]);
}
