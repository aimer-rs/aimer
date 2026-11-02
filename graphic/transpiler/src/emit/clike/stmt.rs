//! Blocks, statements and control flow for the C-like emitters.

use super::*;
use crate::sema::{Scalar, Var};

impl<'a> Emitter<'a> {
    // ---- helpers ----

    pub(super) fn push(&mut self, s: impl AsRef<str>) {
        let line = format!("{}{}", "    ".repeat(self.indent), s.as_ref());
        self.lines.push(line);
    }

    pub(super) fn capture<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T>) -> Result<(T, Vec<String>)> {
        let saved = std::mem::take(&mut self.lines);
        let r = f(self);
        let got = std::mem::replace(&mut self.lines, saved);
        Ok((r?, got))
    }

    /// A name that no visible local uses. It is reserved until the scope ends.
    pub(super) fn alloc_name(&mut self, source: &str) -> String {
        let base = self.id(source);
        let mut e = base.clone();
        let mut i = 1;
        while self.env.name_taken(&e) {
            e = format!("{base}_{i}");
            i += 1;
        }
        self.env.declare_hidden(&e);
        e
    }

    pub(super) fn declare_local(&mut self, source: &str, ty: Ty, mutable: bool) -> String {
        let emitted = self.alloc_name(source);
        self.env.declare(source, Var { ty, mutable, emitted: emitted.clone() });
        emitted
    }

    pub(super) fn fresh(&mut self) -> String {
        loop {
            let name = format!("_t{}", self.tmp);
            self.tmp += 1;
            if !self.env.name_taken(&name) {
                self.env.declare_hidden(&name);
                return name;
            }
        }
    }

    pub(super) fn type_of(&mut self, e: &Expr) -> Result<Ty> {
        let g = self.g;
        g.type_of(&mut self.env, e)
    }

    /// Does evaluating `e` later give the same value as evaluating it now?
    pub(super) fn is_stable(&self, e: &Expr) -> bool {
        match e {
            Expr::Int(_) | Expr::Float(_) | Expr::Bool(_) => true,
            Expr::Path(n) => match self.env.lookup(n) {
                Some(v) => !v.mutable,
                None => true,
            },
            _ => false,
        }
    }

    /// Emit operands left to right. If a later operand needs statements, earlier unstable operands
    /// are copied to temporaries first, so source evaluation order is kept.
    pub(super) fn emit_operands(&mut self, es: &[&Expr]) -> Result<Vec<String>> {
        let mut parts = Vec::new();
        for e in es {
            let (t, lines) = self.capture(|s| s.emit_expr(e))?;
            if t.is_empty() {
                bail!("`return`, `break` and `continue` cannot be used inside an operand");
            }
            parts.push((t, lines));
        }
        let last = parts.iter().rposition(|(_, l)| !l.is_empty());
        let mut out = Vec::new();
        for (i, ((t, lines), e)) in parts.into_iter().zip(es).enumerate() {
            self.lines.extend(lines);
            if matches!(last, Some(l) if i < l) && !self.is_stable(e) {
                let tmp = self.fresh();
                let ty = self.type_of(e)?;
                let decl = self.decl(&ty, &tmp)?;
                self.push(format!("{decl} = {t};"));
                out.push(tmp);
            } else {
                out.push(t);
            }
        }
        Ok(out)
    }

    // ---- blocks and statements ----

    pub(super) fn emit_block(&mut self, b: &Block, dest: &Dest, new_scope: bool) -> Result<()> {
        if new_scope {
            self.env.push();
        }
        for s in &b.stmts {
            self.emit_stmt(s)?;
        }
        if let Some(t) = &b.tail {
            self.emit_into(t, dest)?;
        }
        if new_scope {
            self.env.pop();
        }
        Ok(())
    }

    fn needs_statements(e: &Expr) -> bool {
        matches!(e, Expr::If { select: false, .. } | Expr::Match { .. } | Expr::Block(_) | Expr::Loop(_))
    }

    fn emit_stmt(&mut self, s: &Stmt) -> Result<()> {
        match s {
            Stmt::Expr(e) => self.emit_into(e, &Dest::Discard),
            Stmt::Let { mutable, name, ty, init } => {
                let declared = ty.as_ref().map(|t| self.g.resolve_type(t)).transpose()?;
                let Some(init) = init else {
                    if !*mutable {
                        bail!("`let {name};` without a value must be `let mut {name}: T;`");
                    }
                    let Some(t) = declared else { bail!("`let mut {name};` needs a type") };
                    let emitted = self.declare_local(name, t.clone(), true);
                    let decl = self.decl(&t, &emitted)?;
                    let zero = self.zero_init(&t)?;
                    self.push(format!("{decl} = {zero};"));
                    return Ok(());
                };
                let var_ty = {
                    let it = self.type_of(init)?;
                    match &declared {
                        Some(d) if *d != it && it != Ty::Never => {
                            bail!("`{name}`: declared {}, initialiser is {}", d.describe(), it.describe())
                        }
                        Some(d) => d.clone(),
                        None => it,
                    }
                };
                if matches!(var_ty, Ty::Unit | Ty::Never) {
                    bail!("`{name}` has type {}; it cannot be bound", var_ty.describe());
                }
                if let Expr::ArrayLit(items) = init {
                    let refs: Vec<&Expr> = items.iter().collect();
                    let texts = self.emit_operands(&refs)?;
                    let emitted = self.declare_local(name, var_ty.clone(), *mutable);
                    let decl = self.decl(&var_ty, &emitted)?;
                    if self.d == Dialect::Glsl {
                        let Ty::Array(e, n) = &var_ty else { bail!("`{name}` is not an array") };
                        let (et, n) = (self.ty_text(e)?, self.array_size(n)?);
                        self.push(format!("{decl} = {et}[{n}]({});", texts.join(", ")));
                    } else {
                        self.push(format!("{decl} = {{ {} }};", texts.join(", ")));
                    }
                    return Ok(());
                }
                if Self::needs_statements(init) {
                    let emitted = self.alloc_name(name);
                    let decl = self.decl(&var_ty, &emitted)?;
                    self.push(format!("{decl};"));
                    self.emit_into(init, &Dest::Assign(emitted.clone()))?;
                    self.env.declare(name, Var { ty: var_ty, mutable: *mutable, emitted });
                } else {
                    let text = self.emit_expr(init)?;
                    if text.is_empty() {
                        bail!("`{name}` is initialised with an expression that has no value");
                    }
                    let emitted = self.declare_local(name, var_ty.clone(), *mutable);
                    let decl = self.decl(&var_ty, &emitted)?;
                    self.push(format!("{decl} = {text};"));
                }
                Ok(())
            }
        }
    }

    pub(super) fn emit_into(&mut self, e: &Expr, dest: &Dest) -> Result<()> {
        match e {
            Expr::Block(b) => {
                self.push("{");
                self.indent += 1;
                self.emit_block(b, dest, true)?;
                self.indent -= 1;
                self.push("}");
                Ok(())
            }
            Expr::If { select: false, cond, then, els } => {
                let c = self.emit_expr(cond)?;
                self.emit_if_rest(&c, then, els, dest)
            }
            Expr::Match { scrutinee, arms } => self.emit_match(scrutinee, arms, dest),
            Expr::Loop(b) => {
                self.push("while (true) {");
                self.loops.push(match dest {
                    Dest::Discard => None,
                    d => Some(d.clone()),
                });
                self.indent += 1;
                self.emit_block(b, &Dest::Discard, true)?;
                self.indent -= 1;
                self.loops.pop();
                self.push("}");
                Ok(())
            }
            Expr::While { cond, body } => {
                self.indent += 1;
                let (c, lines) = self.capture(|s| s.emit_expr(cond))?;
                self.indent -= 1;
                self.loops.push(None);
                if lines.is_empty() {
                    self.push(format!("while ({c}) {{"));
                    self.indent += 1;
                } else {
                    self.push("while (true) {");
                    self.lines.extend(lines);
                    self.indent += 1;
                    self.push(format!("if (!({c})) {{"));
                    self.push("    break;");
                    self.push("}");
                }
                self.emit_block(body, &Dest::Discard, true)?;
                self.indent -= 1;
                self.loops.pop();
                self.push("}");
                Ok(())
            }
            Expr::For { var, lo, hi, inclusive, body } => {
                let ty = self.type_of(lo)?;
                let texts = self.emit_operands(&[lo, hi])?;
                let hi_text = if self.is_stable(hi) {
                    texts[1].clone()
                } else {
                    let t = self.fresh();
                    let decl = self.decl(&ty, &t)?;
                    self.push(format!("{decl} = {};", texts[1]));
                    t
                };
                self.env.push();
                let i = self.declare_local(var, ty.clone(), false);
                let cmp = if *inclusive { "<=" } else { "<" };
                let tt = self.ty_text(&ty)?;
                self.push(format!("for ({tt} {i} = {}; {i} {cmp} {hi_text}; {i}++) {{", texts[0]));
                self.loops.push(None);
                self.indent += 1;
                self.emit_block(body, &Dest::Discard, true)?;
                self.indent -= 1;
                self.loops.pop();
                self.env.pop();
                self.push("}");
                Ok(())
            }
            Expr::Return(v) => {
                match v {
                    Some(x) => {
                        let vt = self.type_of(x)?;
                        if vt != self.ret && vt != Ty::Never {
                            bail!("`return` of {}, but the function returns {}", vt.describe(), self.ret.describe());
                        }
                        if self.ret == Ty::Unit {
                            bail!("`return` with a value in a function that returns ()");
                        }
                        let t = self.emit_expr(x)?;
                        self.push(format!("return {t};"));
                    }
                    None => {
                        if self.ret != Ty::Unit {
                            bail!("`return` without a value in a function that returns {}", self.ret.describe());
                        }
                        self.push("return;");
                    }
                }
                Ok(())
            }
            Expr::Break(v) => {
                let Some(ctx) = self.loops.last().cloned() else { bail!("`break` outside of a loop") };
                match (v, ctx) {
                    (Some(x), Some(d)) => {
                        self.emit_into(x, &d)?;
                        if !matches!(d, Dest::Return) {
                            self.push("break;");
                        }
                    }
                    (Some(_), None) => bail!("`break` with a value is only allowed in a `loop` whose value is used"),
                    (None, Some(_)) => bail!("`break` in a `loop` whose value is used must carry a value"),
                    (None, None) => self.push("break;"),
                }
                Ok(())
            }
            Expr::Continue => {
                self.push("continue;");
                Ok(())
            }
            Expr::Discard => {
                self.push(match self.d {
                    Dialect::Msl => "discard_fragment();",
                    Dialect::Hlsl | Dialect::Glsl => "discard;",
                });
                Ok(())
            }
            _ => {
                let ty = self.type_of(e)?;
                let text = self.emit_expr(e)?;
                match dest {
                    Dest::Discard => {
                        if !text.is_empty() {
                            if ty == Ty::Unit || self.d != Dialect::Msl {
                                self.push(format!("{text};"));
                            } else {
                                self.push(format!("(void){text};"));
                            }
                        }
                    }
                    Dest::Return => {
                        if self.ret == Ty::Unit {
                            if !text.is_empty() {
                                self.push(format!("{text};"));
                            }
                        } else {
                            self.push(format!("return {text};"));
                        }
                    }
                    Dest::Assign(n) => {
                        if text.is_empty() {
                            bail!("this expression has no value");
                        }
                        self.push(format!("{n} = {text};"));
                    }
                }
                Ok(())
            }
        }
    }

    fn emit_if_rest(&mut self, cond: &str, then: &Block, els: &Option<Box<Expr>>, dest: &Dest) -> Result<()> {
        self.push(format!("if ({cond}) {{"));
        self.indent += 1;
        self.emit_block(then, dest, true)?;
        self.indent -= 1;
        self.emit_else(els, dest)
    }

    /// Writes the `else` part (if any) and the closing brace of the current `if`.
    fn emit_else(&mut self, els: &Option<Box<Expr>>, dest: &Dest) -> Result<()> {
        let Some(e) = els else {
            self.push("}");
            return Ok(());
        };
        match &**e {
            Expr::If { select: false, cond, then, els: els2 } => {
                self.indent += 1;
                let (c, lines) = self.capture(|s| s.emit_expr(cond))?;
                self.indent -= 1;
                if lines.is_empty() {
                    self.push(format!("}} else if ({c}) {{"));
                    self.indent += 1;
                    self.emit_block(then, dest, true)?;
                    self.indent -= 1;
                    self.emit_else(els2, dest)
                } else {
                    self.push("} else {");
                    self.lines.extend(lines);
                    self.indent += 1;
                    self.emit_if_rest(&c, then, els2, dest)?;
                    self.indent -= 1;
                    self.push("}");
                    Ok(())
                }
            }
            Expr::Block(b) => {
                self.push("} else {");
                self.indent += 1;
                self.emit_block(b, dest, true)?;
                self.indent -= 1;
                self.push("}");
                Ok(())
            }
            other => bail!("unexpected `else` branch {other:?}"),
        }
    }

    fn emit_arm_body(&mut self, body: &Expr, dest: &Dest) -> Result<()> {
        match body {
            Expr::Block(b) => self.emit_block(b, dest, true),
            other => {
                self.env.push();
                let r = self.emit_into(other, dest);
                self.env.pop();
                r
            }
        }
    }

    fn emit_match(&mut self, scrutinee: &Expr, arms: &[Arm], dest: &Dest) -> Result<()> {
        let st = self.type_of(scrutinee)?;
        if !matches!(st, Ty::Scalar(Scalar::I32 | Scalar::U32 | Scalar::Bool)) {
            bail!("`match` needs an integer or bool scrutinee, found {}", st.describe());
        }
        let has_wild = arms.iter().any(|a| a.pats.iter().any(|p| matches!(p, Pat::Wild)));
        if !has_wild && st != Ty::bool() {
            bail!("`match` needs a `_` arm");
        }
        // A `break` in a `switch` leaves the switch, not the loop around it, so those matches
        // become an `if` chain.
        let arm_breaks = arms.iter().any(|a| {
            let mut found = false;
            walk_expr(&a.body, &mut |x| found |= matches!(x, Expr::Break(_)));
            found
        });
        let switchable = st != Ty::bool()
            && !arm_breaks
            && arms.iter().all(|a| a.pats.iter().all(|p| matches!(p, Pat::Wild | Pat::Value(_))));
        let scr = self.emit_expr(scrutinee)?;

        if switchable {
            self.push(format!("switch ({scr}) {{"));
            self.indent += 1;
            let mut default_seen = false;
            for a in arms {
                let mut labels = Vec::new();
                for p in &a.pats {
                    match p {
                        Pat::Wild => {
                            if default_seen {
                                bail!("`match` has more than one `_` arm");
                            }
                            default_seen = true;
                            labels.push("default:".to_string());
                        }
                        Pat::Value(v) => {
                            let t = self.emit_expr(v)?;
                            labels.push(format!("case {t}:"));
                        }
                        Pat::Range { .. } => unreachable!(),
                    }
                }
                self.push(format!("{} {{", labels.join(" ")));
                self.indent += 1;
                self.emit_arm_body(&a.body, dest)?;
                self.push("break;");
                self.indent -= 1;
                self.push("}");
            }
            self.indent -= 1;
            self.push("}");
            return Ok(());
        }

        // Ranges, a bool scrutinee or a `break` in an arm: an if / else if chain on a copy of the scrutinee.
        let m = if matches!(scrutinee, Expr::Path(_)) && self.is_stable(scrutinee) {
            scr
        } else {
            let t = self.fresh();
            let decl = self.decl(&st, &t)?;
            self.push(format!("{decl} = {scr};"));
            t
        };
        let mut first = true;
        let mut closed_with_else = false;
        for a in arms {
            if closed_with_else {
                bail!("`match` arms after `_` are unreachable");
            }
            let mut conds: Vec<(String, bool)> = Vec::new();
            let mut wild = false;
            for p in &a.pats {
                match p {
                    Pat::Wild => wild = true,
                    Pat::Value(v) => {
                        let t = self.emit_expr(v)?;
                        conds.push((format!("{m} == {t}"), false));
                    }
                    Pat::Range { lo, hi, inclusive } => {
                        let l = self.emit_expr(lo)?;
                        let h = self.emit_expr(hi)?;
                        let op = if *inclusive { "<=" } else { "<" };
                        conds.push((format!("{m} >= {l} && {m} {op} {h}"), true));
                    }
                }
            }
            if wild {
                closed_with_else = true;
                if first {
                    self.push("{");
                } else {
                    self.push("} else {");
                }
            } else {
                let c = if conds.len() == 1 {
                    conds.remove(0).0
                } else {
                    conds.into_iter().map(|(t, range)| if range { format!("({t})") } else { t }).collect::<Vec<_>>().join(" || ")
                };
                if first {
                    self.push(format!("if ({c}) {{"));
                } else {
                    self.push(format!("}} else if ({c}) {{"));
                }
            }
            first = false;
            self.indent += 1;
            self.emit_arm_body(&a.body, dest)?;
            self.indent -= 1;
        }
        if first {
            bail!("`match` has no arms");
        }
        self.push("}");
        Ok(())
    }
}
