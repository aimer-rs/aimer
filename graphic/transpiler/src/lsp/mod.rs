//! Language services for `.aimer` files: diagnostics, outline, go to definition, references,
//! hover, completion and rename checks.
//!
//! The entry point is [`handle`]: one JSON request in, one JSON response out. The messages
//! follow the Language Server Protocol (method names, `Position`, `Range`, `Diagnostic`,
//! `DocumentSymbol` and `CompletionItem` shapes), but there is no transport. A host links the
//! crate (natively or compiled to WebAssembly, see `ffi`) and passes strings.
//!
//! Request: `{"method": "textDocument/hover", "params": {"text": "...", "line": 3, "character": 5}}`
//! Response: `{"result": ...}` or `{"error": "..."}`.
//!
//! Common params: `text` (the document), `name` (shader name, default `shader`),
//! `modules` (`[{"name": "aimer::color", "text": "..."}]`, the sources of imported modules),
//! `externs` (module names the runtime provides). Position methods also take `line` and
//! `character` (0-based, UTF-16 columns).

pub mod builtins;
pub mod json;
pub mod symbols;

#[cfg(target_arch = "wasm32")]
mod ffi;

use crate::error::Error;
use crate::resolve::{ExternalModules, MemoryLoader};
use crate::{Target, parser};
use json::Json;
use symbols::{Decl, Index, Kind, Lines, Loc};

/// Methods this service answers. The host registers a provider for each.
pub const METHODS: &[&str] = &[
    "textDocument/diagnostic",
    "textDocument/documentSymbol",
    "textDocument/definition",
    "textDocument/references",
    "textDocument/hover",
    "textDocument/completion",
    "textDocument/prepareRename",
];

/// Handle one request. Never panics on bad input: errors come back as `{"error": ...}`.
pub fn handle(request: &str) -> String {
    match Json::parse(request).and_then(|req| dispatch(&req)) {
        Ok(result) => Json::obj().with("result", result),
        Err(e) => Json::obj().with("error", e),
    }
    .to_string()
}

fn dispatch(req: &Json) -> Result<Json, String> {
    let method = req.get("method").and_then(Json::as_str).ok_or("missing `method`")?;
    if method == "initialize" {
        return Ok(Json::obj()
            .with("name", "aimer-shader")
            .with("version", env!("CARGO_PKG_VERSION"))
            .with("methods", METHODS.iter().map(|m| Json::from(*m)).collect::<Vec<_>>())
            .with("triggerCharacters", vec![".", "@", "#"]));
    }
    let params = req.get("params").ok_or("missing `params`")?;
    let doc = Doc::from_params(params)?;
    let position = || -> Result<usize, String> {
        let line = params.get("line").and_then(Json::as_usize).ok_or("missing `line`")?;
        let col = params.get("character").and_then(Json::as_usize).ok_or("missing `character`")?;
        Ok(doc.lines().offset(line, col))
    };
    Ok(match method {
        "textDocument/diagnostic" => doc.diagnostics(params),
        "textDocument/documentSymbol" => doc.document_symbols(),
        "textDocument/definition" => doc.definition(position()?),
        "textDocument/references" => doc.references(position()?),
        "textDocument/hover" => doc.hover(position()?),
        "textDocument/completion" => doc.completion(position()?),
        "textDocument/prepareRename" => doc.prepare_rename(position()?),
        _ => return Err(format!("unknown method `{method}`")),
    })
}

struct Module {
    name: String,
    src: String,
    index: Index,
}

struct Doc {
    src: String,
    index: Index,
    modules: Vec<Module>,
}

fn range(lines: &Lines, span: (usize, usize)) -> Json {
    let pos = |off| {
        let (line, col) = lines.position(off);
        Json::obj().with("line", line).with("character", col)
    };
    Json::obj().with("start", pos(span.0)).with("end", pos(span.1))
}

fn markdown(value: String) -> Json {
    Json::obj().with("kind", "markdown").with("value", value)
}

fn symbol_kind(kind: Kind) -> usize {
    match kind {
        Kind::Fn => 12,
        Kind::Struct => 23,
        Kind::Const => 14,
        Kind::Field => 8,
        _ => 13,
    }
}

fn completion_kind(kind: Kind) -> usize {
    match kind {
        Kind::Fn => 3,
        Kind::Struct => 22,
        Kind::Const => 21,
        Kind::Field => 5,
        _ => 6,
    }
}

impl Doc {
    fn from_params(params: &Json) -> Result<Doc, String> {
        let src = params.get("text").and_then(Json::as_str).ok_or("missing `text`")?.to_string();
        let mut modules = Vec::new();
        for m in params.get("modules").and_then(Json::as_array).unwrap_or(&[]) {
            let name = m.get("name").and_then(Json::as_str).ok_or("module without `name`")?;
            let text = m.get("text").and_then(Json::as_str).ok_or("module without `text`")?;
            modules.push(Module { name: name.to_string(), src: text.to_string(), index: Index::build(text) });
        }
        let index = Index::build(&src);
        Ok(Doc { src, index, modules })
    }

    fn lines(&self) -> Lines<'_> {
        Lines::new(&self.src)
    }

    // ---- diagnostics ----

    fn diagnostics(&self, params: &Json) -> Json {
        let name = params.get("name").and_then(Json::as_str).unwrap_or("shader");
        let externs: Vec<String> = params
            .get("externs")
            .and_then(Json::as_array)
            .unwrap_or(&[])
            .iter()
            .filter_map(|e| e.as_str().map(str::to_string))
            .collect();
        let mut mem = MemoryLoader::new();
        for m in &self.modules {
            mem.insert(m.name.clone(), m.src.clone());
        }
        let loader = ExternalModules { inner: &mem, modules: externs };
        let lines = self.lines();
        let mut out = Vec::new();
        match parser::parse(&self.src) {
            Err(e) => out.push(self.error_diagnostic(&lines, &e, None)),
            Ok(_) => {
                // A module that does not parse is reported on its import, not at a line of this file.
                let mut broken = false;
                for m in &self.modules {
                    if let Err(e) = parser::parse(&m.src) {
                        let msg = Error::new(format!("module `{}` has an error: {e}", m.name));
                        out.push(self.error_diagnostic(&lines, &msg, Some(&m.name)));
                        broken = true;
                    }
                }
                if !broken {
                    let result = crate::load_program(&self.src, name, &loader).and_then(|p| crate::emit(&p, Target::Wgsl).map(|_| ()));
                    if let Err(e) = result {
                        out.push(self.error_diagnostic(&lines, &e, None));
                    }
                }
            }
        }
        Json::from(out)
    }

    fn error_diagnostic(&self, lines: &Lines, e: &Error, module: Option<&str>) -> Json {
        let span = match (e.span, e.line) {
            (Some(s), _) => s,
            (None, 0) => self.locate(lines, module, &e.msg),
            (None, l) => lines.line_span(l),
        };
        Json::obj().with("range", range(lines, span)).with("severity", 1usize).with("source", "aimer").with("message", e.msg.clone())
    }

    /// Where an error without a position belongs. The resolver and type checker only know names:
    /// a module path (`import` line) or a function (`in fn `f`: ...`) and the item the message quotes.
    fn locate(&self, lines: &Lines, module: Option<&str>, msg: &str) -> (usize, usize) {
        let quoted: Vec<&str> = msg.split('`').skip(1).step_by(2).collect();
        if let Some(m) = module {
            if let Some(s) = self.import_span(lines, m) {
                return s;
            }
        }
        for q in &quoted {
            if q.contains("::") {
                if let Some(s) = self.import_span(lines, q) {
                    return s;
                }
            }
        }
        let func = msg.strip_prefix("in fn `").and_then(|r| r.split('`').next()).and_then(|n| self.index.decls.iter().find(|d| d.kind == Kind::Fn && d.name == n));
        let scope = func.map_or((0, usize::MAX), |d| d.span);
        // The last quoted name is the most specific one: struct `P` has no field `zz`.
        for q in quoted.iter().skip(usize::from(func.is_some())).rev() {
            let hit = self.index.toks.iter().enumerate().find(|(i, t)| self.index.is_ident(*i) && self.index.name(*i) == *q && scope.0 <= t.start && t.end <= scope.1);
            if let Some((_, t)) = hit {
                return (t.start, t.end);
            }
        }
        match func {
            Some(d) => d.name_span,
            None => self.import_span_named(lines, &quoted).unwrap_or_else(|| lines.line_span(1)),
        }
    }

    /// The first `import` line that mentions `path`.
    fn import_span(&self, lines: &Lines, path: &str) -> Option<(usize, usize)> {
        self.src.lines().position(|line| {
            let t = line.trim_start();
            (t.starts_with("import ") || t.starts_with("pub import ")) && t.contains(path)
        })
        .map(|i| lines.line_span(i + 1))
    }

    /// An `import` line ending in one of the quoted names, e.g. `import a::secret;`.
    fn import_span_named(&self, lines: &Lines, quoted: &[&str]) -> Option<(usize, usize)> {
        quoted.iter().find_map(|q| self.import_span(lines, &format!("::{q}")))
    }

    // ---- symbols ----

    fn document_symbols(&self) -> Json {
        let lines = self.lines();
        let mut out = Vec::new();
        for (i, d) in self.index.decls.iter().enumerate() {
            if !d.kind.is_item() {
                continue;
            }
            let children: Vec<Json> = self
                .index
                .decls
                .iter()
                .filter(|f| f.kind == Kind::Field && f.parent == Some(i))
                .map(|f| {
                    Json::obj()
                        .with("name", f.name.clone())
                        .with("detail", f.detail.split_once(": ").map_or(String::new(), |(_, t)| t.to_string()))
                        .with("kind", symbol_kind(Kind::Field))
                        .with("range", range(&lines, f.span))
                        .with("selectionRange", range(&lines, f.name_span))
                })
                .collect();
            let detail = if d.kind == Kind::Struct { String::new() } else { d.detail.clone() };
            out.push(
                Json::obj()
                    .with("name", d.name.clone())
                    .with("detail", detail)
                    .with("kind", symbol_kind(d.kind))
                    .with("range", range(&lines, d.span))
                    .with("selectionRange", range(&lines, d.name_span))
                    .with("children", children),
            );
        }
        Json::from(out)
    }

    // ---- navigation ----

    fn decl(&self, loc: Loc) -> &Decl {
        match loc.module {
            None => &self.index.decls[loc.decl],
            Some(m) => &self.modules[m].index.decls[loc.decl],
        }
    }

    /// Declarations an identifier may mean: the document first, then public items of imported modules.
    fn resolve(&self, ti: usize) -> Vec<Loc> {
        let local: Vec<Loc> = self.index.resolve(ti).into_iter().map(|decl| Loc { module: None, decl }).collect();
        if !local.is_empty() {
            return local;
        }
        let name = self.index.name(ti);
        let special = ti > 0 && (self.index.is_punct(ti - 1, ".") || self.index.is_punct(ti - 1, "@"));
        if special {
            return Vec::new();
        }
        let mut found = Vec::new();
        for (mi, m) in self.modules.iter().enumerate() {
            for (di, d) in m.index.public_items() {
                if d.name == name {
                    found.push(Loc { module: Some(mi), decl: di });
                }
            }
        }
        found
    }

    fn definition(&self, offset: usize) -> Json {
        let Some(ti) = self.index.ident_at(offset) else { return Json::Null };
        let mut out = Vec::new();
        for loc in self.resolve(ti) {
            let d = self.decl(loc);
            let (lines_src, module) = match loc.module {
                None => (&self.src, Json::Null),
                Some(m) => (&self.modules[m].src, Json::from(self.modules[m].name.clone())),
            };
            out.push(Json::obj().with("module", module).with("range", range(&Lines::new(lines_src), d.name_span)));
        }
        Json::from(out)
    }

    fn references(&self, offset: usize) -> Json {
        let Some(ti) = self.index.ident_at(offset) else { return Json::from(Vec::<Json>::new()) };
        let targets: Vec<usize> = self.resolve(ti).into_iter().filter(|l| l.module.is_none()).map(|l| l.decl).collect();
        let lines = self.lines();
        let spans = self.index.references(&targets).into_iter().map(|t| range(&lines, (self.index.toks[t].start, self.index.toks[t].end)));
        Json::from(spans.collect::<Vec<_>>())
    }

    fn prepare_rename(&self, offset: usize) -> Json {
        let Some(ti) = self.index.ident_at(offset) else { return Json::Null };
        let targets = self.resolve(ti);
        if targets.len() != 1 || targets[0].module.is_some() {
            return Json::Null;
        }
        let tok = &self.index.toks[ti];
        Json::obj().with("range", range(&self.lines(), (tok.start, tok.end))).with("placeholder", self.index.name(ti))
    }

    // ---- hover ----

    fn describe(&self, loc: Loc) -> String {
        let d = self.decl(loc);
        let index = match loc.module {
            None => &self.index,
            Some(m) => &self.modules[m].index,
        };
        let head = match d.kind {
            Kind::Field => {
                let parent = d.parent.map(|p| index.decls[p].name.as_str()).unwrap_or("?");
                format!("(field) {parent}.{}", d.detail)
            }
            Kind::Param => format!("(parameter) {}", d.detail),
            _ => d.detail.clone(),
        };
        let mut text = format!("```aimer\n{head}\n```");
        if !d.doc.is_empty() {
            text.push_str("\n\n");
            text.push_str(&d.doc);
        }
        if let Some(m) = loc.module {
            text.push_str(&format!("\n\n*from `{}`*", self.modules[m].name));
        }
        text
    }

    fn hover(&self, offset: usize) -> Json {
        let Some(ti) = self.index.ident_at(offset) else { return Json::Null };
        let name = self.index.name(ti);
        let mut parts: Vec<String> = self.resolve(ti).into_iter().take(3).map(|l| self.describe(l)).collect();
        if parts.is_empty() {
            let after = |p: &str| ti > 0 && self.index.is_punct(ti - 1, p);
            let doc = if after("@") {
                builtins::builtin_var_doc(name)
            } else if after("[") && ti > 1 && self.index.is_punct(ti - 2, "#") {
                builtins::attr_doc(name)
            } else {
                builtins::function_doc(name).or_else(|| builtins::type_doc(name))
            };
            parts.extend(doc);
        }
        if parts.is_empty() {
            return Json::Null;
        }
        let tok = &self.index.toks[ti];
        Json::obj().with("contents", markdown(parts.join("\n\n---\n\n"))).with("range", range(&self.lines(), (tok.start, tok.end)))
    }

    // ---- completion ----

    fn completion(&self, offset: usize) -> Json {
        let word_start = self.index.ident_at(offset).map_or(offset, |t| self.index.toks[t].start);
        let before = self.index.toks.partition_point(|t| t.end <= word_start);
        let prev = (0..before).rev().find(|&i| !matches!(self.index.toks[i].tok, crate::lexer::Tok::Eof));
        let punct = |i: Option<usize>, p: &str| i.is_some_and(|i| self.index.is_punct(i, p));
        let mut items: Vec<Json> = Vec::new();
        let item = |label: &str, kind: usize, detail: &str, doc: &str| {
            let mut j = Json::obj().with("label", label).with("kind", kind).with("detail", detail);
            if !doc.is_empty() {
                j = j.with("documentation", markdown(doc.to_string()));
            }
            j
        };
        let snippet = |j: Json, label: &str| j.with("insertText", format!("{label}($0)")).with("insertTextFormat", 2usize);

        if punct(prev, ".") {
            let mut seen = Vec::new();
            for d in self.index.decls.iter().filter(|d| d.kind == Kind::Field) {
                if !seen.contains(&d.name) {
                    seen.push(d.name.clone());
                    items.push(item(&d.name, 5, &d.detail, &d.doc));
                }
            }
            for s in builtins::SWIZZLES {
                items.push(item(s, 10, "swizzle", ""));
            }
            for (n, sig, doc) in builtins::TEXTURE_METHODS {
                items.push(snippet(item(n, 2, sig, doc), n));
            }
        } else if punct(prev, "@") {
            for (n, doc) in builtins::BUILTIN_VARS {
                items.push(item(n, 20, "builtin", doc));
            }
        } else if punct(prev, "[") && prev.is_some_and(|p| p > 0 && self.index.is_punct(p - 1, "#")) {
            for (n, doc) in builtins::ATTRS {
                items.push(item(n, 10, "attribute", doc));
            }
        } else {
            for k in builtins::KEYWORDS {
                items.push(item(k, 14, "keyword", ""));
            }
            for (n, doc) in builtins::TYPES {
                items.push(item(n, 7, "type", doc));
            }
            for n in builtins::function_names() {
                items.push(snippet(item(n, 3, "builtin function", ""), n));
            }
            for i in self.index.visible_locals(offset) {
                let d = &self.index.decls[i];
                items.push(item(&d.name, 6, &d.detail, ""));
            }
            for d in self.index.decls.iter().filter(|d| d.kind.is_item()) {
                let j = item(&d.name, completion_kind(d.kind), &d.detail.lines().next().unwrap_or_default().to_string(), &d.doc);
                items.push(if d.kind == Kind::Fn { snippet(j, &d.name) } else { j });
            }
            for m in &self.modules {
                for (_, d) in m.index.public_items() {
                    if self.index.decls.iter().any(|own| own.kind.is_item() && own.name == d.name) {
                        continue;
                    }
                    let j = item(&d.name, completion_kind(d.kind), &d.detail.lines().next().unwrap_or_default().to_string(), &d.doc).with("labelDetails", Json::obj().with("description", m.name.clone()));
                    items.push(if d.kind == Kind::Fn { snippet(j, &d.name) } else { j });
                }
            }
        }
        Json::from(items)
    }
}
