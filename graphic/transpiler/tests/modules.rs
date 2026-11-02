use std::collections::HashMap;

use aimer_transpiler::error::{Error, Result};
use aimer_transpiler::resolve::ModuleLoader;

struct MapLoader(HashMap<&'static str, &'static str>);

impl ModuleLoader for MapLoader {
    fn load(&self, module: &[String]) -> Result<String> {
        let key = module.join("::");
        self.0.get(key.as_str()).map(|s| s.to_string()).ok_or_else(|| Error::new(format!("no module `{key}`")))
    }
}

fn loader(mods: &[(&'static str, &'static str)]) -> MapLoader {
    MapLoader(mods.iter().copied().collect())
}

fn validate(wgsl: &str) {
    let module = naga::front::wgsl::parse_str(wgsl).unwrap_or_else(|e| panic!("naga parse failed:\n{}\n---\n{wgsl}", e.emit_to_string(wgsl)));
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(&module)
        .unwrap_or_else(|e| panic!("naga validation failed: {e:?}\n---\n{wgsl}"));
}

fn ok(src: &str, mods: &[(&'static str, &'static str)]) -> String {
    let out = aimer_transpiler::compile_wgsl(src, "t", &loader(mods)).unwrap_or_else(|e| panic!("compile failed: {e}"));
    validate(&out);
    out
}

fn err(src: &str, mods: &[(&'static str, &'static str)]) -> String {
    match aimer_transpiler::compile_wgsl(src, "t", &loader(mods)) {
        Ok(out) => panic!("expected an error, got:\n{out}"),
        Err(e) => e.to_string(),
    }
}

const A: &str = "module a;\npub fn helper(x: f32) -> f32 { x * 2.0 }\nfn secret(x: f32) -> f32 { x + 1.0 }\npub fn uses_secret(x: f32) -> f32 { secret(x) }\n";
const B: &str = "module b;\npub fn helper(x: f32) -> f32 { x * 3.0 }\n";

#[test]
fn private_items_cannot_be_imported_by_name() {
    let e = err("import a::secret;\nfn f(x: f32) -> f32 { secret(x) }", &[("a", A)]);
    assert!(e.contains("private"), "{e}");
}

#[test]
fn glob_import_does_not_expose_private_items() {
    // `secret` is not visible, so the call is an unknown function.
    let e = err("import a::*;\nfn f(x: f32) -> f32 { secret(x) }", &[("a", A)]);
    assert!(e.contains("secret"), "{e}");
}

#[test]
fn public_function_may_use_a_private_one() {
    let out = ok("import a::uses_secret;\nfn f(x: f32) -> f32 { uses_secret(x) }", &[("a", A)]);
    assert!(out.contains("fn secret("), "{out}");
}

#[test]
fn module_private_helper_may_share_a_name_with_the_shader() {
    let out = ok("import a::uses_secret;\nfn secret(x: f32) -> f32 { x - 1.0 }\nfn f(x: f32) -> f32 { uses_secret(x) + secret(x) }", &[("a", A)]);
    // The shader keeps its name, the module's helper is prefixed.
    assert!(out.contains("fn secret(x: f32)"), "{out}");
    assert!(out.contains("fn a_secret(x: f32)"), "{out}");
    assert!(out.contains("return a_secret(x);"), "{out}");
}

#[test]
fn same_name_in_two_modules_is_mangled() {
    let m = &[("a", A), ("b", B)];
    let out = ok("import a::helper;\nimport b::*;\nfn f(x: f32) -> f32 { a_user(x) }\nfn a_user(x: f32) -> f32 { helper(x) }", m);
    // Explicit import wins over the glob import: `helper` is a's.
    assert!(out.contains("x * 2.0"), "{out}");
    assert!(!out.contains("x * 3.0"), "b::helper is unused and must be dropped:\n{out}");
}

#[test]
fn explicit_import_clashing_with_an_own_item_is_an_error() {
    let e = err("import a::helper;\nfn helper(x: f32) -> f32 { x }\nfn f(x: f32) -> f32 { helper(x) }", &[("a", A)]);
    assert!(e.contains("clashes"), "{e}");
}

#[test]
fn own_item_wins_over_a_glob_import() {
    let out = ok("import a::*;\nfn helper(x: f32) -> f32 { x - 5.0 }\nfn f(x: f32) -> f32 { helper(x) }", &[("a", A)]);
    assert!(out.contains("x - 5.0"), "{out}");
    assert!(!out.contains("x * 2.0"), "{out}");
}

#[test]
fn two_glob_imports_of_the_same_name_are_ambiguous_when_used() {
    let m = &[("a", A), ("b", B)];
    let e = err("import a::*;\nimport b::*;\nfn f(x: f32) -> f32 { helper(x) }", m);
    assert!(e.contains("ambiguous"), "{e}");
    // Not using the name is fine.
    ok("import a::*;\nimport b::*;\nfn f(x: f32) -> f32 { x }", m);
}

#[test]
fn locals_shadow_imported_names() {
    let out = ok("import a::*;\nfn f(helper: f32) -> f32 { let secret = helper; secret }", &[("a", A)]);
    assert!(out.contains("fn f(helper: f32)"), "{out}");
}

#[test]
fn struct_and_const_clashes_are_renamed_everywhere() {
    let m = &[(
        "a",
        "module a;\nconst K: f32 = 2.0;\npub struct P { v: f32 }\npub fn make() -> P { P { v: K } }\npub fn get(p: P) -> f32 { p.v }\n",
    )];
    let out = ok(
        "import a::*;\nstruct P { w: f32 }\nconst K: f32 = 5.0;\nfn f() -> f32 { let q = P { w: K }; get(make()) + q.w }",
        m,
    );
    assert!(out.contains("struct a_P"), "{out}");
    assert!(out.contains("struct P {"), "{out}");
    assert!(out.contains("a_P(2.0)"), "{out}");
}

#[test]
fn external_modules_are_checked_but_not_emitted() {
    let base = loader(&[("a", A)]);
    let ext = aimer_transpiler::resolve::ExternalModules { inner: &base, modules: vec!["a".to_string()] };
    let src = "import a::helper;\nfn f(x: f32) -> f32 { helper(x) }";
    let out = aimer_transpiler::compile_wgsl(src, "t", &ext).unwrap();
    assert!(out.contains("fn f("), "{out}");
    assert!(!out.contains("fn helper("), "{out}");
    // Without the flag the helper is part of the output.
    let out = aimer_transpiler::compile_wgsl(src, "t", &base).unwrap();
    assert!(out.contains("fn helper("), "{out}");
}

#[test]
fn pub_import_re_exports_public_functions_of_a_function_only_shader() {
    let m = &[("a", A)];
    // A plain import of an entry-less shader drops what is not used.
    let plain = ok("import a::*;\nfn f(x: f32) -> f32 { x }", m);
    assert!(!plain.contains("x * 2.0"), "{plain}");
    // `pub import m::*;` writes every public function of `m` (and what they need).
    let glob = ok("pub import a::*;\nfn f(x: f32) -> f32 { x }", m);
    assert!(glob.contains("fn helper(") && glob.contains("fn uses_secret(") && glob.contains("fn secret("), "{glob}");
    // `pub import m::name;` writes just that function.
    let named = ok("pub import a::helper;\nfn f(x: f32) -> f32 { x }", m);
    assert!(named.contains("fn helper("), "{named}");
    assert!(!named.contains("fn uses_secret("), "{named}");
}
