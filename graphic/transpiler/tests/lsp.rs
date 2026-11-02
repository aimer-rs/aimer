use aimer_transpiler::lsp::handle;
use aimer_transpiler::lsp::json::Json;

fn request(method: &str, text: &str, extra: Json) -> Json {
    let mut params = Json::obj().with("text", text);
    if let (Json::Obj(fields), Json::Obj(more)) = (&mut params, extra) {
        fields.extend(more);
    }
    let req = Json::obj().with("method", method).with("params", params);
    let res = Json::parse(&handle(&req.to_string())).unwrap();
    if let Some(e) = res.get("error") {
        panic!("{method} failed: {e}");
    }
    res.get("result").unwrap().clone()
}

fn at(line: usize, character: usize) -> Json {
    Json::obj().with("line", line).with("character", character)
}

/// Position of the first `needle` in `text`, `delta` characters in.
fn find(text: &str, needle: &str, delta: usize) -> Json {
    let off = text.find(needle).unwrap_or_else(|| panic!("`{needle}` not in text")) + delta;
    let line = text[..off].matches('\n').count();
    let col = off - text[..off].rfind('\n').map_or(0, |i| i + 1);
    at(line, col)
}

fn start_of(range: &Json) -> (usize, usize) {
    let s = range.get("start").unwrap();
    (s.get("line").unwrap().as_usize().unwrap(), s.get("character").unwrap().as_usize().unwrap())
}

fn end_of(range: &Json) -> (usize, usize) {
    let s = range.get("end").unwrap();
    (s.get("line").unwrap().as_usize().unwrap(), s.get("character").unwrap().as_usize().unwrap())
}

fn arr(j: &Json) -> &[Json] {
    j.as_array().unwrap()
}

fn text_of(j: &Json) -> String {
    j.get("contents").unwrap().get("value").unwrap().as_str().unwrap().to_string()
}

const SRC: &str = "\
// Doubles a value.
fn twice(x: f32) -> f32 {
    let y = x * 2.0;
    y
}

struct Point {
    pos: vec2,
    w: f32,
}

fn len(p: Point) -> f32 {
    let x = p.pos.x;
    twice(x) + p.w
}
";

#[test]
fn initialize_lists_methods() {
    let r = request("initialize", "", Json::obj());
    assert!(arr(r.get("methods").unwrap()).len() >= 6);
}

#[test]
fn diagnostics_are_empty_for_valid_source() {
    assert!(arr(&request("textDocument/diagnostic", SRC, Json::obj())).is_empty());
}

#[test]
fn syntax_error_points_at_the_token() {
    let src = "fn f(x: f32) -> f32 {\n    x +* 2.0\n}\n";
    let d = request("textDocument/diagnostic", src, Json::obj());
    let d = &arr(&d)[0];
    let r = d.get("range").unwrap();
    assert_eq!(start_of(r), (1, 7));
    assert_eq!(end_of(r), (1, 8));
    assert_eq!(d.get("severity").unwrap().as_usize(), Some(1));
    assert!(d.get("message").unwrap().as_str().unwrap().contains("expected"));
}

#[test]
fn lexer_error_points_at_the_character() {
    let d = request("textDocument/diagnostic", "fn f() {\n    let a = $;\n}\n", Json::obj());
    let r = arr(&d)[0].get("range").unwrap().clone();
    assert_eq!((start_of(&r), end_of(&r)), ((1, 12), (1, 13)));
}

#[test]
fn type_error_is_reported_on_its_line() {
    let src = "fn f(x: f32) -> f32 {\n    x + nope(x)\n}\n";
    let d = request("textDocument/diagnostic", src, Json::obj());
    let r = arr(&d)[0].get("range").unwrap().clone();
    // The name the message quotes, inside the function it names.
    assert_eq!((start_of(&r), end_of(&r)), ((1, 8), (1, 12)), "{d}");
    assert!(arr(&d)[0].get("message").unwrap().as_str().unwrap().contains("nope"));
}

#[test]
fn missing_module_is_reported_on_its_import() {
    let src = "shader t;\n\nimport aimer::color::*;\nfn f(x: f32) -> f32 { x }\n";
    let d = request("textDocument/diagnostic", src, Json::obj());
    let r = arr(&d)[0].get("range").unwrap().clone();
    assert_eq!(start_of(&r).0, 2, "{d}");
    // Providing the module (as an external one) clears it.
    let modules = Json::obj().with("modules", vec![Json::obj().with("name", "aimer::color").with("text", "module aimer::color;\npub fn id(x: f32) -> f32 { x }\n")]);
    assert!(arr(&request("textDocument/diagnostic", src, modules)).is_empty());
}

#[test]
fn broken_module_is_reported_on_the_import() {
    let src = "import m::*;\nfn f(x: f32) -> f32 { x }\n";
    let modules = Json::obj().with("modules", vec![Json::obj().with("name", "m").with("text", "module m;\npub fn g( {")]);
    let d = request("textDocument/diagnostic", src, modules);
    assert_eq!(start_of(arr(&d)[0].get("range").unwrap()).0, 0, "{d}");
}

#[test]
fn every_sample_shader_is_clean() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/shaders-src");
    let mut modules = Vec::new();
    for name in ["clip", "color", "sdf"] {
        let text = std::fs::read_to_string(format!("{root}/lib/{name}.aimer")).unwrap();
        modules.push(Json::obj().with("name", format!("aimer::{name}")).with("text", text));
    }
    let mut checked = 0;
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "aimer") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let params = Json::obj().with("modules", modules.clone()).with("name", path.file_stem().unwrap().to_str().unwrap());
        let d = request("textDocument/diagnostic", &text, params);
        assert!(arr(&d).is_empty(), "{}: {d}", path.display());
        checked += 1;
    }
    assert!(checked >= 5);
}

#[test]
fn outline_has_items_and_fields() {
    let s = request("textDocument/documentSymbol", SRC, Json::obj());
    let names: Vec<&str> = arr(&s).iter().map(|d| d.get("name").unwrap().as_str().unwrap()).collect();
    assert_eq!(names, ["twice", "Point", "len"]);
    let point = &arr(&s)[1];
    let fields: Vec<&str> = arr(point.get("children").unwrap()).iter().map(|d| d.get("name").unwrap().as_str().unwrap()).collect();
    assert_eq!(fields, ["pos", "w"]);
    assert_eq!(start_of(arr(&s)[0].get("selectionRange").unwrap()), (1, 3));
}

#[test]
fn definition_of_a_function_a_local_and_a_field() {
    let p = find(SRC, "twice(x) +", 1);
    let d = request("textDocument/definition", SRC, p);
    assert_eq!(start_of(arr(&d)[0].get("range").unwrap()), (1, 3));
    assert_eq!(arr(&d)[0].get("module"), Some(&Json::Null));

    // `x` in `twice(x)` is the local `x` of `len`, not the parameter of `twice`.
    let p = find(SRC, "twice(x)", 6);
    let d = request("textDocument/definition", SRC, p);
    assert_eq!(start_of(arr(&d)[0].get("range").unwrap()), (12, 8));

    let p = find(SRC, "p.pos", 3);
    let d = request("textDocument/definition", SRC, p);
    assert_eq!(start_of(arr(&d)[0].get("range").unwrap()), (7, 4));
}

#[test]
fn definition_into_an_imported_module() {
    let src = "import m::helper;\nfn f(x: f32) -> f32 { helper(x) }\n";
    let modules = Json::obj().with("modules", vec![Json::obj().with("name", "m").with("text", "module m;\n\n// Triples.\npub fn helper(x: f32) -> f32 { x * 3.0 }\n")]);
    let p = find(src, "helper(x)", 2);
    let d = request("textDocument/definition", src, p.clone().with("modules", modules.get("modules").unwrap().clone()));
    let d = &arr(&d)[0];
    assert_eq!(d.get("module").unwrap().as_str(), Some("m"));
    assert_eq!(start_of(d.get("range").unwrap()), (3, 7));
    let h = request("textDocument/hover", src, p.with("modules", modules.get("modules").unwrap().clone()));
    let text = text_of(&h);
    assert!(text.contains("fn helper(x: f32) -> f32") && text.contains("Triples.") && text.contains("from `m`"), "{text}");
}

#[test]
fn references_follow_shadowing() {
    // Two different `x`: the parameter of `twice` and the local of `len`.
    let p = find(SRC, "fn twice(x", 9);
    let r = request("textDocument/references", SRC, p);
    let lines: Vec<usize> = arr(&r).iter().map(|r| start_of(r).0).collect();
    assert_eq!(lines, [1, 2], "{r}");

    let p = find(SRC, "let x", 4);
    let r = request("textDocument/references", SRC, p);
    let lines: Vec<usize> = arr(&r).iter().map(|r| start_of(r).0).collect();
    assert_eq!(lines, [12, 13], "{r}");

    let p = find(SRC, "fn twice", 3);
    let r = request("textDocument/references", SRC, p);
    assert_eq!(arr(&r).len(), 2);
}

#[test]
fn hover_shows_signature_doc_and_builtins() {
    let h = request("textDocument/hover", SRC, find(SRC, "twice(x) +", 1));
    let t = text_of(&h);
    assert!(t.contains("fn twice(x: f32) -> f32") && t.contains("Doubles a value."), "{t}");

    let h = request("textDocument/hover", SRC, find(SRC, "p.w", 2));
    assert!(text_of(&h).contains("(field) Point.w: f32"));

    let h = request("textDocument/hover", SRC, find(SRC, "p: Point", 0));
    assert!(text_of(&h).contains("(parameter) p: Point"));

    let src = "fn f(a: f32) -> f32 { sin(a) }";
    let h = request("textDocument/hover", src, find(src, "sin", 1));
    assert!(text_of(&h).contains("sin(x: T) -> T"));

    assert_eq!(request("textDocument/hover", SRC, at(0, 0)), Json::Null);
}

#[test]
fn completion_by_context() {
    let labels = |r: &Json| arr(r).iter().map(|c| c.get("label").unwrap().as_str().unwrap().to_string()).collect::<Vec<_>>();

    let src = "fn f(a: f32) -> f32 {\n    let q = a;\n    \n}\n";
    let c = labels(&request("textDocument/completion", src, at(2, 4)));
    for want in ["fn", "vec4", "sin", "a", "q", "f"] {
        assert!(c.contains(&want.to_string()), "{want} missing");
    }

    let c = labels(&request("textDocument/completion", SRC.replace("p.pos.x", "p.").as_str(), find(&SRC.replace("p.pos.x", "p."), "p.;", 2)));
    assert!(c.contains(&"pos".to_string()) && c.contains(&"xyz".to_string()), "{c:?}");
    assert!(!c.contains(&"fn".to_string()));

    let src = "@";
    let c = labels(&request("textDocument/completion", src, at(0, 1)));
    assert!(c.contains(&"position".to_string()) && c.contains(&"frag_color".to_string()));

    let src = "#[";
    let c = labels(&request("textDocument/completion", src, at(0, 2)));
    assert!(c.contains(&"vertex".to_string()) && c.contains(&"bind".to_string()));
}

#[test]
fn rename_is_only_allowed_for_local_symbols() {
    let ok = request("textDocument/prepareRename", SRC, find(SRC, "fn twice", 4));
    assert_eq!(ok.get("placeholder").unwrap().as_str(), Some("twice"));
    assert_eq!(request("textDocument/prepareRename", SRC, find(SRC, "f32 {", 1)), Json::Null);
}

#[test]
fn utf16_columns_are_used() {
    // `é` is one UTF-16 unit but two bytes, `😀` is two units and four bytes.
    let src = "// é😀\nfn f(x: f32) -> f32 { x +* 1.0 }\n";
    let d = request("textDocument/diagnostic", src, Json::obj());
    assert_eq!(start_of(arr(&d)[0].get("range").unwrap()), (1, 25));
    let src = "fn f(x: f32) -> f32 { /* é😀 */ x +* 1.0 }\n";
    let d = request("textDocument/diagnostic", src, Json::obj());
    assert_eq!(start_of(arr(&d)[0].get("range").unwrap()), (0, 35));
}

#[test]
fn bad_requests_give_errors_not_panics() {
    for req in ["", "{", "[]", "{\"method\":\"nope\",\"params\":{}}", "{\"method\":\"textDocument/hover\",\"params\":{}}", "{\"method\":\"textDocument/hover\",\"params\":{\"text\":\"x\"}}"] {
        let res = Json::parse(&handle(req)).unwrap();
        assert!(res.get("error").is_some(), "{req}");
    }
}

#[test]
fn untidy_sources_do_not_panic() {
    for src in ["", "fn", "fn f(", "struct S {", "struct S { a: ", "uniform struct", "const", "fn f() { let", "fn f() { for", "#[", "@", "pub", "fn f(x: f32) -> { }", "/* open", "\u{1F600}"] {
        for method in ["textDocument/diagnostic", "textDocument/documentSymbol"] {
            request(method, src, Json::obj());
        }
        for method in ["textDocument/hover", "textDocument/definition", "textDocument/references", "textDocument/completion", "textDocument/prepareRename"] {
            for col in 0..=src.chars().count() + 1 {
                request(method, src, at(0, col));
            }
            request(method, src, at(7, 3));
        }
    }
}

#[test]
fn errors_point_at_the_most_specific_quoted_name() {
    let src = "struct P { a: f32 }\nfn run(p: P) -> f32 {\n    p.zz\n}\n";
    let d = request("textDocument/diagnostic", src, Json::obj());
    let r = arr(&d)[0].get("range").unwrap().clone();
    assert_eq!((start_of(&r), end_of(&r)), ((2, 6), (2, 8)), "{d}");
}
