//! Every shader in `shaders-src/` must compile to all five targets. WGSL is validated with naga and
//! GLSL and GLSL ES with `glslangValidator` (compile and link). Metal and HLSL are compiled by the real
//! compilers in `tests/native.rs`.

use std::path::PathBuf;
use std::process::Command;

use aimer_transpiler::resolve::FsLoader;

fn shader_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders-src")
}

fn shaders() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(shader_dir()).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x == "aimer") {
            out.push((p.file_stem().unwrap().to_string_lossy().to_string(), std::fs::read_to_string(&p).unwrap()));
        }
    }
    out.sort();
    out
}

fn loader() -> FsLoader {
    FsLoader { root: shader_dir().join("lib") }
}

#[test]
fn wgsl_validates_with_naga() {
    for (name, src) in shaders() {
        let wgsl = aimer_transpiler::compile_wgsl(&src, &name, &loader()).unwrap_or_else(|e| panic!("{name}: {e}"));
        let module = naga::front::wgsl::parse_str(&wgsl).unwrap_or_else(|e| panic!("{name}: naga parse failed:\n{}\n---\n{wgsl}", e.emit_to_string(&wgsl)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{name}: naga validation failed: {e:?}\n---\n{wgsl}"));
    }
}

#[test]
fn glsl_compiles_and_links() {
    if !Command::new("glslangValidator").arg("--version").output().is_ok_and(|o| o.status.success()) {
        eprintln!("skipping GLSL checks: glslangValidator is not installed");
        return;
    }
    let dir = std::env::temp_dir().join(format!("aimer_ported_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut failures = Vec::new();
    for (name, src) in shaders() {
        for es in [false, true] {
            let out = if es { aimer_transpiler::compile_glsl_es(&src, &name, &loader()) } else { aimer_transpiler::compile_glsl(&src, &name, &loader()) }
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            let label = format!("{name} ({})", if es { "es" } else { "desktop" });
            let mut files = Vec::new();
            for (ext, text) in [("vert", &out.vertex), ("frag", &out.fragment)] {
                if let Some(text) = text {
                    let f = dir.join(format!("{name}_{es}.{ext}"));
                    std::fs::write(&f, text).unwrap();
                    files.push(f);
                }
            }
            for f in &files {
                let r = Command::new("glslangValidator").arg(f).output().unwrap();
                if !r.status.success() {
                    failures.push(format!("{label}: {}\n{}", f.display(), String::from_utf8_lossy(&r.stdout)));
                }
            }
            if files.len() > 1 {
                let r = Command::new("glslangValidator").arg("-l").args(&files).output().unwrap();
                if !r.status.success() {
                    failures.push(format!("{label}: link failed\n{}", String::from_utf8_lossy(&r.stdout)));
                }
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(failures.is_empty(), "{}", failures.join("\n---\n"));
}

#[test]
fn layout_json_lists_every_vertex_attribute() {
    for (name, src) in shaders() {
        let p = aimer_transpiler::load_program(&src, &name, &loader()).unwrap_or_else(|e| panic!("{name}: {e}"));
        let json = aimer_transpiler::layout::json(&p).unwrap_or_else(|e| panic!("{name}: {e}"));
        let expected: usize = aimer_transpiler::layout::vertex_layout(&p).unwrap().iter().map(|b| b.attributes.len()).sum();
        assert_eq!(json.matches("\"field\":").count(), expected, "{name}:\n{json}");
    }
}

#[test]
fn rect_instance_is_the_144_byte_record_of_the_hand_written_shader() {
    let src = std::fs::read_to_string(shader_dir().join("rect.aimer")).unwrap();
    let p = aimer_transpiler::load_program(&src, "rect", &loader()).unwrap();
    let buffers = aimer_transpiler::layout::vertex_layout(&p).unwrap();
    assert_eq!(buffers.len(), 1);
    let b = &buffers[0];
    assert_eq!(b.stride, 144);
    assert!(b.per_instance);
    let offsets: Vec<(String, u32)> = b.attributes.iter().map(|a| (a.field.clone(), a.offset)).collect();
    assert_eq!(offsets[2], ("color".to_string(), 16));
    assert_eq!(offsets[3], ("border_radius".to_string(), 20));
    assert_eq!(offsets[5], ("border_color".to_string(), 52));
    assert_eq!(offsets[12], ("shadow_flags".to_string(), 128));
    assert_eq!(b.attributes.iter().filter(|a| a.format.name == "unorm8x4").count(), 4);
}
