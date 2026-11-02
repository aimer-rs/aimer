//! Rewrites item names inside one source file (a module or the shader) to the final, globally
//! unique names chosen by the resolver. Local variables shadow items and are left alone.

use std::collections::{HashMap, HashSet};

use crate::ast::*;
use crate::error::{Error, Result, bail};

/// What a name used in one file refers to.
#[derive(Debug, Default)]
pub struct Scope {
    /// source name -> final name
    pub names: HashMap<String, String>,
    /// source name -> modules that glob-import it, for names that are used but ambiguous
    pub ambiguous: HashMap<String, Vec<String>>,
    /// Names of items that exist somewhere in the program but are not visible in this file.
    pub hidden: HashSet<String>,
}

struct Renamer<'a> {
    scope: &'a Scope,
    locals: Vec<HashSet<String>>,
}

impl Renamer<'_> {
    fn is_local(&self, n: &str) -> bool {
        self.locals.iter().any(|s| s.contains(n))
    }

    fn item(&self, n: &str) -> Result<String> {
        if let Some(mods) = self.scope.ambiguous.get(n) {
            bail!("`{n}` is ambiguous: it is imported from {}; import it explicitly", mods.join(" and "));
        }
        if let Some(f) = self.scope.names.get(n) {
            return Ok(f.clone());
        }
        if self.scope.hidden.contains(n) {
            bail!("`{n}` is not in scope here: it is private to a module or not imported");
        }
        Ok(n.to_string())
    }

    fn declare(&mut self, n: &str) {
        self.locals.last_mut().expect("scope").insert(n.to_string());
    }

    fn ty(&self, t: &mut Type) -> Result<()> {
        match t {
            Type::Named(n, args) => {
                *n = self.item(n)?;
                args.iter_mut().try_for_each(|a| self.ty(a))
            }
            Type::Array(inner, len) => {
                self.ty(inner)?;
                // An array length only sees consts, never locals.
                self.expr_in(len, &mut Vec::new())
            }
            Type::Builtin(_) | Type::Unit => Ok(()),
        }
    }

    fn expr_in(&self, e: &mut Expr, locals: &mut Vec<HashSet<String>>) -> Result<()> {
        let mut r = Renamer { scope: self.scope, locals: std::mem::take(locals) };
        let res = r.expr(e);
        *locals = r.locals;
        res
    }

    fn block(&mut self, b: &mut Block) -> Result<()> {
        self.locals.push(HashSet::new());
        let r = (|| {
            for s in &mut b.stmts {
                match s {
                    Stmt::Let { name, ty, init, .. } => {
                        if let Some(i) = init {
                            self.expr(i)?;
                        }
                        if let Some(t) = ty {
                            self.ty(t)?;
                        }
                        self.declare(name);
                    }
                    Stmt::Expr(e) => self.expr(e)?,
                }
            }
            if let Some(t) = &mut b.tail {
                self.expr(t)?;
            }
            Ok(())
        })();
        self.locals.pop();
        r
    }

    fn expr(&mut self, e: &mut Expr) -> Result<()> {
        match e {
            Expr::Int(_) | Expr::Float(_) | Expr::Bool(_) | Expr::Continue | Expr::Discard => Ok(()),
            Expr::Path(n) => {
                if !self.is_local(n) {
                    *n = self.item(n)?;
                }
                Ok(())
            }
            Expr::Unary(_, x) | Expr::Field(x, _) => self.expr(x),
            Expr::Cast(x, t) => {
                self.expr(x)?;
                self.ty(t)
            }
            Expr::Binary(_, a, b) | Expr::Index(a, b) => {
                self.expr(a)?;
                self.expr(b)
            }
            Expr::Assign { target, value, .. } => {
                self.expr(target)?;
                self.expr(value)
            }
            Expr::Call { name, generics, args } => {
                *name = self.item(name)?;
                generics.iter_mut().try_for_each(|g| self.ty(g))?;
                args.iter_mut().try_for_each(|a| self.expr(a))
            }
            Expr::Method { recv, args, .. } => {
                self.expr(recv)?;
                args.iter_mut().try_for_each(|a| self.expr(a))
            }
            Expr::StructLit { name, fields, base } => {
                *name = self.item(name)?;
                fields.iter_mut().try_for_each(|(_, v)| self.expr(v))?;
                if let Some(b) = base {
                    self.expr(b)?;
                }
                Ok(())
            }
            Expr::ArrayLit(items) => items.iter_mut().try_for_each(|a| self.expr(a)),
            Expr::Block(b) | Expr::Loop(b) => self.block(b),
            Expr::If { cond, then, els, .. } => {
                self.expr(cond)?;
                self.block(then)?;
                match els {
                    Some(e) => self.expr(e),
                    None => Ok(()),
                }
            }
            Expr::Match { scrutinee, arms } => {
                self.expr(scrutinee)?;
                for a in arms {
                    for p in &mut a.pats {
                        match p {
                            Pat::Wild => {}
                            Pat::Value(v) => self.expr(v)?,
                            Pat::Range { lo, hi, .. } => {
                                self.expr(lo)?;
                                self.expr(hi)?;
                            }
                        }
                    }
                    self.expr(&mut a.body)?;
                }
                Ok(())
            }
            Expr::While { cond, body } => {
                self.expr(cond)?;
                self.block(body)
            }
            Expr::For { var, lo, hi, body, .. } => {
                self.expr(lo)?;
                self.expr(hi)?;
                self.locals.push(HashSet::from([var.clone()]));
                let r = self.block(body);
                self.locals.pop();
                r
            }
            Expr::Return(v) | Expr::Break(v) => match v {
                Some(v) => self.expr(v),
                None => Ok(()),
            },
        }
    }
}

/// Rename the declared name of `item` to `final_name` and every item reference inside it.
pub fn rename_item(item: &mut Item, scope: &Scope, final_name: &str) -> Result<()> {
    let mut r = Renamer { scope, locals: vec![HashSet::new()] };
    let ctx = |e: Error, n: &str| e.context(format!("`{n}`"));
    match &mut item.kind {
        ItemKind::Import { .. } => Ok(()),
        ItemKind::Const { name, ty, value } => {
            let old = name.clone();
            *name = final_name.to_string();
            r.ty(ty).and_then(|_| r.expr(value)).map_err(|e| ctx(e, &old))
        }
        ItemKind::Struct { name, fields, .. } => {
            let old = name.clone();
            *name = final_name.to_string();
            fields.iter_mut().try_for_each(|f| r.ty(&mut f.ty)).map_err(|e| ctx(e, &old))
        }
        ItemKind::Resource { name, ty, .. } => {
            let old = name.clone();
            *name = final_name.to_string();
            match ty {
                Some(t) => r.ty(t).map_err(|e| ctx(e, &old)),
                None => Ok(()),
            }
        }
        ItemKind::Fn { name, params, ret, body } => {
            let old = name.clone();
            *name = final_name.to_string();
            (|| {
                for p in params.iter_mut() {
                    r.ty(&mut p.ty)?;
                    r.declare(&p.name);
                }
                if let Some(t) = ret {
                    r.ty(t)?;
                }
                r.block(body)
            })()
            .map_err(|e| ctx(e, &old))
        }
    }
}
