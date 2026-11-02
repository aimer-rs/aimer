//! A token-level symbol index: every declaration with its position, scope and doc comment.
//!
//! It works on tokens, not on the AST, so it keeps working while the file does not parse
//! (the usual state while typing). It only needs the source to lex.

use crate::lexer::{Tok, Token, lex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Fn,
    Struct,
    Const,
    Uniform,
    Texture,
    Sampler,
    Field,
    Param,
    Local,
}

impl Kind {
    /// Declared at the top level of a file.
    pub fn is_item(self) -> bool {
        matches!(self, Kind::Fn | Kind::Struct | Kind::Const | Kind::Uniform | Kind::Texture | Kind::Sampler)
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Fn => "function",
            Kind::Struct => "struct",
            Kind::Const => "constant",
            Kind::Uniform => "uniform",
            Kind::Texture => "texture",
            Kind::Sampler => "sampler",
            Kind::Field => "field",
            Kind::Param => "parameter",
            Kind::Local => "variable",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Decl {
    pub name: String,
    pub kind: Kind,
    /// Byte range of the name.
    pub name_span: (usize, usize),
    /// Byte range of the whole declaration (attributes and body included).
    pub span: (usize, usize),
    /// Source text shown on hover: a signature, a struct body, `let mut x: T`.
    pub detail: String,
    /// The `//` comment lines directly above the declaration.
    pub doc: String,
    pub parent: Option<usize>,
    /// Byte range where a parameter or local is visible.
    pub scope: (usize, usize),
    pub public: bool,
}

#[derive(Debug, Default)]
pub struct Index {
    pub toks: Vec<Token>,
    pub decls: Vec<Decl>,
}

/// A declaration, possibly in an imported module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Loc {
    /// `None` for the document itself, else the index of the imported module.
    pub module: Option<usize>,
    pub decl: usize,
}

struct Builder<'a> {
    src: &'a str,
    toks: Vec<Token>,
    decls: Vec<Decl>,
}

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_open(t: &Tok) -> bool {
    matches!(t, Tok::Punct("(" | "[" | "{"))
}

fn is_close(t: &Tok) -> bool {
    matches!(t, Tok::Punct(")" | "]" | "}"))
}

/// The `//` lines directly above byte offset `at`, without the slashes.
fn doc_above(src: &str, at: usize) -> String {
    let line_start = src[..at].rfind('\n').map_or(0, |i| i + 1);
    let mut lines = Vec::new();
    let mut end = line_start;
    while end > 0 {
        let prev_start = src[..end - 1].rfind('\n').map_or(0, |i| i + 1);
        let line = src[prev_start..end - 1].trim();
        let Some(text) = line.strip_prefix("//") else { break };
        let text = text.strip_prefix('/').unwrap_or(text);
        lines.push(text.strip_prefix(' ').unwrap_or(text).to_string());
        end = prev_start;
    }
    lines.reverse();
    lines.join("\n")
}

impl Builder<'_> {
    fn n(&self) -> usize {
        self.toks.len()
    }

    fn is_p(&self, i: usize, p: &str) -> bool {
        matches!(self.toks.get(i).map(|t| &t.tok), Some(Tok::Punct(q)) if *q == p)
    }

    fn ident(&self, i: usize) -> Option<&str> {
        match self.toks.get(i).map(|t| &t.tok) {
            Some(Tok::Ident(s)) => Some(s),
            _ => None,
        }
    }

    fn is_kw(&self, i: usize, k: &str) -> bool {
        self.ident(i) == Some(k)
    }

    fn is_eof(&self, i: usize) -> bool {
        i >= self.n() || matches!(self.toks[i].tok, Tok::Eof)
    }

    /// Source text of tokens `a..=b`.
    fn text(&self, a: usize, b: usize) -> &str {
        let b = b.min(self.n() - 1);
        &self.src[self.toks[a].start..self.toks[b].end.max(self.toks[a].start)]
    }

    /// `toks[i]` opens a group. Returns the index after its closer (or the `Eof` token).
    fn skip_group(&self, i: usize) -> usize {
        let mut depth = 0isize;
        let mut j = i;
        while j < self.n() {
            let t = &self.toks[j].tok;
            if matches!(t, Tok::Eof) {
                return j;
            }
            if is_open(t) {
                depth += 1;
            } else if is_close(t) {
                depth -= 1;
                if depth <= 0 {
                    return j + 1;
                }
            }
            j += 1;
        }
        self.n() - 1
    }

    fn skip_to_semi(&self, mut i: usize) -> usize {
        while !self.is_eof(i) {
            if self.is_p(i, ";") {
                return i + 1;
            }
            i = if is_open(&self.toks[i].tok) { self.skip_group(i) } else { i + 1 };
        }
        i.min(self.n() - 1)
    }

    /// First token at or after `from` (below `limit`) that is one of `stops`, skipping groups.
    fn type_end(&self, from: usize, limit: usize, stops: &[&str]) -> usize {
        let mut m = from;
        while m < limit && !self.is_eof(m) {
            if let Tok::Punct(p) = &self.toks[m].tok {
                if stops.contains(p) {
                    return m;
                }
            }
            m = if is_open(&self.toks[m].tok) { self.skip_group(m) } else { m + 1 };
        }
        m.min(limit)
    }

    fn push(&mut self, d: Decl) -> usize {
        self.decls.push(d);
        self.decls.len() - 1
    }

    fn decl(&self, name_tok: usize, kind: Kind, span: (usize, usize), detail: String, public: bool) -> Decl {
        let t = &self.toks[name_tok];
        Decl {
            name: self.ident(name_tok).unwrap_or_default().to_string(),
            kind,
            name_span: (t.start, t.end),
            span,
            detail,
            doc: if kind == Kind::Param || kind == Kind::Local { String::new() } else { doc_above(self.src, span.0) },
            parent: None,
            scope: (0, usize::MAX),
            public,
        }
    }

    fn run(&mut self) {
        let mut i = 0;
        while !self.is_eof(i) {
            let first = i;
            while self.is_p(i, "#") && self.is_p(i + 1, "[") {
                i = self.skip_group(i + 1);
            }
            let mut public = false;
            if self.is_kw(i, "pub") {
                public = true;
                i += 1;
            }
            let head = i;
            let next = match self.ident(i) {
                Some("uniform") if self.is_kw(i + 1, "struct") => self.struct_item(first, head, i + 1, public),
                Some("struct") => self.struct_item(first, head, i, public),
                Some("fn") => self.fn_item(first, head, public),
                Some("const") => self.simple_item(first, head, Kind::Const, public),
                Some("uniform") => self.simple_item(first, head, Kind::Uniform, public),
                Some("texture") => self.simple_item(first, head, Kind::Texture, public),
                Some("sampler") => self.simple_item(first, head, Kind::Sampler, public),
                Some("import" | "module" | "shader") => self.skip_to_semi(i),
                _ => i + 1,
            };
            i = next.max(first + 1);
        }
    }

    fn simple_item(&mut self, first: usize, head: usize, kind: Kind, public: bool) -> usize {
        let end = self.skip_to_semi(head);
        if self.ident(head + 1).is_none() {
            return end;
        }
        let last = end.saturating_sub(1).max(head);
        let mut detail = self.text(head, last).trim_end_matches(';').trim().to_string();
        if detail.len() > 300 {
            detail = format!("{}...", detail.chars().take(300).collect::<String>());
        }
        let span = (self.toks[first].start, self.toks[last].end);
        let d = self.decl(head + 1, kind, span, detail, public);
        self.push(d);
        end
    }

    fn struct_item(&mut self, first: usize, head: usize, kw: usize, public: bool) -> usize {
        if self.ident(kw + 1).is_none() || !self.is_p(kw + 2, "{") {
            return kw + 1;
        }
        let open = kw + 2;
        let end = self.skip_group(open);
        let last = end.saturating_sub(1).max(open);
        let detail = self.text(head, last).to_string();
        let span = (self.toks[first].start, self.toks[last].end);
        let d = self.decl(kw + 1, Kind::Struct, span, detail, public);
        let parent = self.push(d);
        let mut k = open + 1;
        while k < last {
            if is_open(&self.toks[k].tok) {
                k = self.skip_group(k);
                continue;
            }
            if self.ident(k).is_some() && self.is_p(k + 1, ":") {
                let te = self.type_end(k + 2, last, &[","]);
                let tail = te.saturating_sub(1).max(k + 1);
                let ty = squash(self.text(k + 2, tail));
                let mut f = self.decl(k, Kind::Field, (self.toks[k].start, self.toks[tail].end), format!("{}: {ty}", self.ident(k).unwrap_or_default()), public);
                f.parent = Some(parent);
                self.push(f);
                k = te;
                continue;
            }
            k += 1;
        }
        end
    }

    fn fn_item(&mut self, first: usize, head: usize, public: bool) -> usize {
        if self.ident(head + 1).is_none() {
            return head + 1;
        }
        let mut j = head + 2;
        let mut params = Vec::new();
        let mut params_end = j;
        if self.is_p(j, "(") {
            let close = self.skip_group(j);
            params_end = close.saturating_sub(1);
            let mut k = j + 1;
            while k < params_end {
                if is_open(&self.toks[k].tok) {
                    k = self.skip_group(k);
                    continue;
                }
                if self.ident(k).is_some() && self.is_p(k + 1, ":") {
                    let te = self.type_end(k + 2, params_end, &[","]);
                    params.push((k, te));
                    k = te;
                    continue;
                }
                k += 1;
            }
            j = close;
        }
        while !self.is_eof(j) && !self.is_p(j, "{") && !self.is_p(j, ";") {
            j = if is_open(&self.toks[j].tok) { self.skip_group(j) } else { j + 1 };
        }
        let sig = squash(self.text(head, j.saturating_sub(1).max(head)));
        let has_body = self.is_p(j, "{");
        let end = if has_body { self.skip_group(j) } else { (j + 1).min(self.n() - 1) };
        let last = end.saturating_sub(1).max(head);
        let span = (self.toks[first].start, self.toks[last].end);
        let d = self.decl(head + 1, Kind::Fn, span, sig, public);
        let fn_idx = self.push(d);
        let body_scope = if has_body { (self.toks[j].start, self.toks[last].end) } else { (0, 0) };
        for (k, te) in params {
            let ty = squash(self.text(k + 2, te.saturating_sub(1).max(k + 1)));
            let name = self.ident(k).unwrap_or_default().to_string();
            let mut p = self.decl(k, Kind::Param, (self.toks[k].start, self.toks[te.saturating_sub(1).max(k + 1)].end), format!("{name}: {ty}"), false);
            p.parent = Some(fn_idx);
            p.scope = body_scope;
            self.push(p);
        }
        let _ = params_end;
        if has_body {
            self.locals(j, end, fn_idx);
        }
        end
    }

    /// `let` and `for` variables of a function body (`open` is its `{`, `end` is after its `}`).
    fn locals(&mut self, open: usize, end: usize, fn_idx: usize) {
        let body_end = self.toks[end.saturating_sub(1).max(open)].end;
        let mut stack: Vec<Vec<usize>> = vec![Vec::new()];
        let mut pending_for: Option<usize> = None;
        let mut k = open + 1;
        while k + 1 < end {
            match &self.toks[k].tok {
                Tok::Punct("{") => stack.push(pending_for.take().into_iter().collect()),
                Tok::Punct("}") => {
                    for d in stack.pop().unwrap_or_default() {
                        self.decls[d].scope.1 = self.toks[k].end;
                    }
                    if stack.is_empty() {
                        stack.push(Vec::new());
                    }
                }
                Tok::Ident(w) if w == "let" => {
                    let mut m = k + 1;
                    let mutable = self.is_kw(m, "mut");
                    if mutable {
                        m += 1;
                    }
                    if let Some(name) = self.ident(m).map(str::to_string) {
                        let mut detail = format!("let {}{name}", if mutable { "mut " } else { "" });
                        if self.is_p(m + 1, ":") {
                            let te = self.type_end(m + 2, end, &["=", ";"]);
                            detail.push_str(&format!(": {}", squash(self.text(m + 2, te.saturating_sub(1).max(m + 2)))));
                        }
                        let mut d = self.decl(m, Kind::Local, (self.toks[k].start, self.toks[m].end), detail, false);
                        d.parent = Some(fn_idx);
                        d.scope = (self.toks[m].end, body_end);
                        let idx = self.push(d);
                        stack.last_mut().expect("never empty").push(idx);
                    }
                }
                Tok::Ident(w) if w == "for" => {
                    if let (Some(name), true) = (self.ident(k + 1).map(str::to_string), self.is_kw(k + 2, "in")) {
                        let mut d = self.decl(k + 1, Kind::Local, (self.toks[k].start, self.toks[k + 1].end), format!("for {name} in ..."), false);
                        d.parent = Some(fn_idx);
                        d.scope = (self.toks[k + 1].end, body_end);
                        pending_for = Some(self.push(d));
                    }
                }
                _ => {}
            }
            k += 1;
        }
    }
}

/// UTF-16 aware line table, since editors count columns in UTF-16 units.
pub struct Lines<'a> {
    src: &'a str,
    starts: Vec<usize>,
}

impl<'a> Lines<'a> {
    pub fn new(src: &'a str) -> Self {
        let mut starts = vec![0];
        starts.extend(src.match_indices('\n').map(|(i, _)| i + 1));
        Lines { src, starts }
    }

    pub fn count(&self) -> usize {
        self.starts.len()
    }

    pub fn position(&self, offset: usize) -> (usize, usize) {
        let offset = offset.min(self.src.len());
        let line = self.starts.partition_point(|s| *s <= offset) - 1;
        let mut off = offset;
        while !self.src.is_char_boundary(off) {
            off -= 1;
        }
        (line, self.src[self.starts[line]..off].encode_utf16().count())
    }

    pub fn offset(&self, line: usize, col: usize) -> usize {
        let Some(&start) = self.starts.get(line) else { return self.src.len() };
        let end = self.starts.get(line + 1).map_or(self.src.len(), |e| e - 1);
        let mut units = 0;
        for (i, c) in self.src[start..end].char_indices() {
            if units >= col {
                return start + i;
            }
            units += c.len_utf16();
        }
        end
    }

    /// Byte range of the non-blank text of a 1-based line.
    pub fn line_span(&self, line: usize) -> (usize, usize) {
        let l = line.clamp(1, self.count()) - 1;
        let start = self.starts[l];
        let end = self.starts.get(l + 1).map_or(self.src.len(), |e| e - 1);
        let text = &self.src[start..end];
        let lead = text.len() - text.trim_start().len();
        let trimmed = text.trim();
        if trimmed.is_empty() { (start, start) } else { (start + lead, start + lead + trimmed.len()) }
    }
}

impl Index {
    pub fn build(src: &str) -> Index {
        let Ok(toks) = lex(src) else { return Index::default() };
        let mut b = Builder { src, toks, decls: Vec::new() };
        b.run();
        Index { toks: b.toks, decls: b.decls }
    }

    pub fn name(&self, ti: usize) -> &str {
        match &self.toks[ti].tok {
            Tok::Ident(s) => s,
            _ => "",
        }
    }

    pub fn is_ident(&self, ti: usize) -> bool {
        matches!(self.toks.get(ti).map(|t| &t.tok), Some(Tok::Ident(_)))
    }

    pub fn is_punct(&self, ti: usize, p: &str) -> bool {
        matches!(self.toks.get(ti).map(|t| &t.tok), Some(Tok::Punct(q)) if *q == p)
    }

    /// The identifier token that contains `offset` (its end counts, for a caret after a word).
    pub fn ident_at(&self, offset: usize) -> Option<usize> {
        let i = self.toks.partition_point(|t| t.end < offset);
        (i..self.toks.len()).take(2).find(|&j| self.is_ident(j) && self.toks[j].start <= offset && offset <= self.toks[j].end)
    }

    /// Declarations an identifier token may refer to. Empty for built-ins and unknown names.
    pub fn resolve(&self, ti: usize) -> Vec<usize> {
        let tok = &self.toks[ti];
        let name = self.name(ti);
        if let Some(d) = self.decls.iter().position(|d| d.name_span == (tok.start, tok.end)) {
            return vec![d];
        }
        let after_dot = ti > 0 && self.is_punct(ti - 1, ".");
        let field_key = self.is_punct(ti + 1, ":") && ti > 0 && (self.is_punct(ti - 1, "{") || self.is_punct(ti - 1, ","));
        if after_dot || field_key {
            return self.fields_named(name);
        }
        if ti > 0 && self.is_punct(ti - 1, "@") {
            return Vec::new();
        }
        let local = self
            .decls
            .iter()
            .enumerate()
            .filter(|(_, d)| matches!(d.kind, Kind::Param | Kind::Local) && d.name == name && d.scope.0 <= tok.start && tok.start <= d.scope.1)
            .max_by_key(|(_, d)| d.name_span.0);
        if let Some((i, _)) = local {
            return vec![i];
        }
        self.decls.iter().position(|d| d.kind.is_item() && d.name == name).into_iter().collect()
    }

    pub fn fields_named(&self, name: &str) -> Vec<usize> {
        self.decls.iter().enumerate().filter(|(_, d)| d.kind == Kind::Field && d.name == name).map(|(i, _)| i).collect()
    }

    /// Parameters and locals visible at `offset`.
    pub fn visible_locals(&self, offset: usize) -> Vec<usize> {
        self.decls
            .iter()
            .enumerate()
            .filter(|(_, d)| matches!(d.kind, Kind::Param | Kind::Local) && d.scope.0 <= offset && offset <= d.scope.1)
            .map(|(i, _)| i)
            .collect()
    }

    /// Identifier tokens that refer to any of `targets`.
    pub fn references(&self, targets: &[usize]) -> Vec<usize> {
        let Some(&first) = targets.first() else { return Vec::new() };
        let name = &self.decls[first].name;
        (0..self.toks.len())
            .filter(|&ti| self.is_ident(ti) && self.name(ti) == name)
            .filter(|&ti| self.resolve(ti).iter().any(|d| targets.contains(d)))
            .collect()
    }

    /// Top-level public declarations, for completion and cross-module lookup.
    pub fn public_items(&self) -> impl Iterator<Item = (usize, &Decl)> {
        self.decls.iter().enumerate().filter(|(_, d)| d.kind.is_item() && d.public)
    }
}
