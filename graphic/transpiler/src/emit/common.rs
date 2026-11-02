//! Pieces shared by every emitter: literal text, where a value goes, and location assignment.

use std::collections::{HashMap, HashSet};

use crate::ast::*;
use crate::error::{Error, Result, bail};
use crate::lexer::Tok;
use crate::resolve::{Program, Stage};
use crate::sema::{Globals, Scalar, Ty};

/// Where the value of an expression goes.
#[derive(Debug, Clone)]
pub enum Dest {
    Discard,
    Return,
    Assign(String),
}

/// Does a const value contain arithmetic? Plain literals and constructors of literals do not.
pub fn has_arithmetic(e: &Expr) -> bool {
    let mut found = false;
    walk_expr(e, &mut |x| {
        if matches!(x, Expr::Binary(..)) {
            found = true;
        }
    });
    found
}

/// Integer literal without the source suffix: `3u32` is `3u`, `3i32` is `3`.
pub fn int_text(s: &str) -> String {
    if let Some(t) = s.strip_suffix("u32") {
        format!("{t}u")
    } else if let Some(t) = s.strip_suffix("i32") {
        t.to_string()
    } else {
        s.to_string()
    }
}

/// Float literal that always has a `.` or an exponent.
pub fn float_text(s: &str) -> String {
    let t = s.strip_suffix("f32").unwrap_or(s);
    let mut out = if t.starts_with('.') { format!("0{t}") } else { t.to_string() };
    if !out.contains(['.', 'e', 'E']) {
        out.push_str(".0");
    }
    out
}

/// Is a user function safe to evaluate when its result may not be used (a `#[select]` arm evaluates both
/// sides)? It must not `discard`, take derivatives, or sample with implicit derivatives, directly or in a callee.
pub fn is_pure_fn(p: &Program, name: &str) -> bool {
    fn visit(p: &Program, name: &str, seen: &mut HashSet<String>) -> bool {
        if !seen.insert(name.to_string()) {
            return true;
        }
        let Some(Item { kind: ItemKind::Fn { body, .. }, .. }) =
            p.items.iter().find(|i| matches!(&i.kind, ItemKind::Fn { name: n, .. } if n == name))
        else {
            return false;
        };
        let mut callees = Vec::new();
        let mut pure = true;
        walk_block(body, &mut |e| match e {
            Expr::Discard => pure = false,
            Expr::Method { name, .. } if name == "sample" => pure = false,
            Expr::Call { name, .. } if matches!(name.as_str(), "dpdx" | "dpdy" | "fwidth") => pure = false,
            Expr::Call { name, .. } if p.globals.fns.contains_key(name) => callees.push(name.clone()),
            _ => {}
        });
        pure && callees.iter().all(|c| visit(p, c, seen))
    }
    visit(p, name, &mut HashSet::new())
}

pub fn mark<'f>(f: &'f Field, name: &str) -> Option<&'f Mark> {
    f.marks.iter().find(|m| m.name == name)
}

pub fn explicit_location(f: &Field) -> Result<Option<u32>> {
    let Some(m) = mark(f, "location") else { return Ok(None) };
    match m.args.as_slice() {
        [Tok::Int(n)] => crate::sema::parse_int_literal(n)
            .and_then(|v| u32::try_from(v).ok())
            .map(Some)
            .ok_or_else(|| Error::new(format!("field `{}`: bad `@location`", f.name))),
        _ => bail!("field `{}`: expected `@location(n)`", f.name),
    }
}

/// Struct name to the location of each of its fields (`None` for builtins and plain fields).
/// Input structs are numbered in the order the vertex entry takes them, varyings per struct.
pub fn assign_locations(p: &Program, g: &Globals) -> Result<HashMap<String, Vec<Option<u32>>>> {
    let mut out = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    if let Some(Item { kind: ItemKind::Fn { params, .. }, .. }) = p.entry(Stage::Vertex) {
        for prm in params {
            if let Ok(Ty::Struct(s)) = g.resolve_type(&prm.ty) {
                order.push(s);
            }
        }
    }
    for it in &p.items {
        if let ItemKind::Struct { name, .. } = &it.kind {
            if !order.contains(name) {
                order.push(name.clone());
            }
        }
    }
    let mut next_input = 0u32;
    let mut used_input: HashSet<u32> = HashSet::new();
    for sname in order {
        let it = p.items.iter().find(|i| matches!(&i.kind, ItemKind::Struct { name, .. } if *name == sname)).unwrap();
        let ItemKind::Struct { fields, .. } = &it.kind else { unreachable!() };
        let is_input = it.attr("input").is_some();
        let is_varying = it.attr("varying").is_some();
        if is_input && is_varying {
            bail!("struct `{sname}` cannot be both `#[input]` and `#[varying]`");
        }
        let mut locs = Vec::new();
        let mut used_local: HashSet<u32> = HashSet::new();
        let mut next_local = 0u32;
        for f in fields {
            let explicit = explicit_location(f)?;
            let is_builtin = mark(f, "position").is_some();
            if is_input && is_builtin {
                bail!("field `{}`: `@position` is only valid in a `#[varying]` struct", f.name);
            }
            if is_varying {
                let t = g.resolve_type(&f.ty)?;
                let is_int = matches!(t.scalar_of(), Some(Scalar::I32 | Scalar::U32));
                if is_int && mark(f, "flat").is_none() {
                    bail!("integer varying `{}.{}` needs `@flat`", sname, f.name);
                }
            }
            let loc = if is_builtin {
                None
            } else if let Some(n) = explicit {
                Some(n)
            } else if is_input {
                while used_input.contains(&next_input) {
                    next_input += 1;
                }
                Some(next_input)
            } else if is_varying {
                while used_local.contains(&next_local) {
                    next_local += 1;
                }
                Some(next_local)
            } else {
                None
            };
            if let Some(l) = loc {
                if is_input {
                    if !used_input.insert(l) {
                        bail!("input location {l} is used twice (struct `{sname}`, field `{}`)", f.name);
                    }
                } else if !used_local.insert(l) {
                    bail!("location {l} is used twice in struct `{sname}` (field `{}`)", f.name);
                }
            }
            locs.push(loc);
        }
        out.insert(sname, locs);
    }
    Ok(out)
}
