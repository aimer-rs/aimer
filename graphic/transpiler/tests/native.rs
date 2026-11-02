//! Compile the generated Metal and HLSL of every shader in `shaders-src/` with the real compilers
//! (`xcrun metal` and `dxc`). A missing compiler prints a skip note instead of failing.

use std::path::{Path, PathBuf};
use std::process::Command;

use aimer_transpiler::ast::ItemKind;
use aimer_transpiler::resolve::{FsLoader, Stage};

fn shader_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("shaders-src")
}

fn shaders() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(shader_dir()).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x == "aimer") {
            let name = p.file_stem().unwrap().to_string_lossy().to_string();
            out.push((name, std::fs::read_to_string(&p).unwrap()));
        }
    }
    out.sort();
    out
}

fn tool_ok(cmd: &mut Command) -> bool {
    cmd.output().is_ok_and(|o| o.status.success())
}

fn scratch(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("aimer_native_{label}_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn entry_name(p: &aimer_transpiler::resolve::Program, s: Stage) -> Option<String> {
    p.entry(s).map(|i| match &i.kind {
        ItemKind::Fn { name, .. } => name.clone(),
        _ => unreachable!(),
    })
}

fn run(cmd: &mut Command, what: &str, file: &Path, failures: &mut Vec<String>) {
    let out = cmd.output().expect("run compiler");
    if !out.status.success() {
        failures.push(format!(
            "{what}: {}\n{}{}",
            file.display(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
}

#[test]
fn metal_compiles_every_shader() {
    if !tool_ok(Command::new("xcrun").args(["--find", "metal"])) {
        eprintln!("skipping Metal compile: `xcrun metal` is not available");
        return;
    }
    let dir = scratch("metal");
    let mut failures = Vec::new();
    for (name, src) in shaders() {
        let text = aimer_transpiler::compile_msl(&src, &name, &FsLoader { root: shader_dir().join("lib") }).unwrap_or_else(|e| panic!("{name}: {e}"));
        let f = dir.join(format!("{name}.metal"));
        std::fs::write(&f, text).unwrap();
        let air = dir.join(format!("{name}.air"));
        run(
            Command::new("xcrun").args(["metal", "-std=metal3.0", "-Werror", "-c"]).arg(&f).arg("-o").arg(&air),
            "metal",
            &f,
            &mut failures,
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(failures.is_empty(), "{}", failures.join("\n---\n"));
}

#[test]
fn dxc_compiles_every_shader() {
    if !tool_ok(Command::new("dxc").arg("--version")) {
        eprintln!("skipping HLSL compile: `dxc` is not installed");
        return;
    }
    let dir = scratch("dxc");
    let mut failures = Vec::new();
    for (name, src) in shaders() {
        let loader = FsLoader { root: shader_dir().join("lib") };
        let text = aimer_transpiler::compile_hlsl(&src, &name, &loader).unwrap_or_else(|e| panic!("{name}: {e}"));
        let program = aimer_transpiler::load_program(&src, &name, &loader).unwrap();
        let f = dir.join(format!("{name}.hlsl"));
        std::fs::write(&f, text).unwrap();
        for (stage, profile) in [(Stage::Vertex, "vs_6_0"), (Stage::Fragment, "ps_6_0")] {
            let Some(entry) = entry_name(&program, stage) else { continue };
            run(
                Command::new("dxc").args(["-T", profile, "-E", &entry, "-WX"]).arg(&f).arg("-Fo").arg(dir.join(format!("{name}.{profile}.dxil"))),
                &format!("dxc {profile}"),
                &f,
                &mut failures,
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(failures.is_empty(), "{}", failures.join("\n---\n"));
}
