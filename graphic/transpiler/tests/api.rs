use aimer_transpiler::resolve::{ExternalModules, MemoryLoader, NoLoader};
use aimer_transpiler::{Target, compile, emit, load_program};

const SRC: &str = "fn twice(x: f32) -> f32 { x * 2.0 }";

#[test]
fn target_names_round_trip() {
    for t in Target::ALL {
        assert_eq!(t.name().parse::<Target>().unwrap(), t);
        assert_eq!(t.to_string(), t.name());
    }
    assert!("spirv".parse::<Target>().is_err());
}

#[test]
fn compile_gives_the_expected_file_per_target() {
    let ext = |t| compile(SRC, "demo", &NoLoader, t).map(|o| o.files.iter().map(|f| f.ext).collect::<Vec<_>>());
    assert_eq!(ext(Target::Wgsl).unwrap(), ["wgsl"]);
    assert_eq!(ext(Target::Msl).unwrap(), ["metal"]);
    assert_eq!(ext(Target::Hlsl).unwrap(), ["hlsl"]);
    assert_eq!(ext(Target::Layout).unwrap(), ["layout.json"]);
}

#[test]
fn one_loaded_program_can_be_emitted_several_times() {
    let program = load_program(SRC, "demo", &NoLoader).unwrap();
    let wgsl = emit(&program, Target::Wgsl).unwrap();
    assert_eq!(wgsl.target, Target::Wgsl);
    assert!(wgsl.get("wgsl").unwrap().contains("fn twice"));
    assert!(wgsl.get("metal").is_none());
    assert!(emit(&program, Target::Msl).unwrap().get("metal").unwrap().contains("twice"));
}

#[test]
fn memory_loader_serves_imports() {
    let loader = MemoryLoader::new().with("aimer::m", "module aimer::m;\npub fn helper(x: f32) -> f32 { x * 3.0 }\n");
    let src = "import aimer::m::helper;\nfn f(x: f32) -> f32 { helper(x) }";
    let out = compile(src, "t", &loader, Target::Wgsl).unwrap();
    assert!(out.get("wgsl").unwrap().contains("fn helper("));

    let ext = ExternalModules { inner: &loader, modules: vec!["aimer::m".to_string()] };
    let out = compile(src, "t", &ext, Target::Wgsl).unwrap();
    assert!(!out.get("wgsl").unwrap().contains("fn helper("));

    let e = compile(src, "t", &MemoryLoader::new(), Target::Wgsl).unwrap_err();
    assert!(e.to_string().contains("aimer::m"), "{e}");
}

#[test]
fn errors_are_std_errors() {
    fn takes_error(_: &dyn std::error::Error) {}
    let e = compile("fn f( {", "t", &NoLoader, Target::Wgsl).unwrap_err();
    takes_error(&e);
}
