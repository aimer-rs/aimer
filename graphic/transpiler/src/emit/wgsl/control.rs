//! `if`, `else` and `match` lowering for the WGSL emitter.

use super::*;

impl<'a> Emitter<'a> {
    pub(super) fn emit_if_rest(&mut self, cond: &str, then: &Block, els: &Option<Box<Expr>>, dest: &Dest) -> Result<()> {
        self.push(format!("if {cond} {{"));
        self.indent += 1;
        self.emit_block(then, dest, true)?;
        self.indent -= 1;
        self.emit_else(els, dest)
    }

    /// Writes the `else` part (if any) and the closing brace of the current `if`.
    pub(super) fn emit_else(&mut self, els: &Option<Box<Expr>>, dest: &Dest) -> Result<()> {
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
                    self.push(format!("}} else if {c} {{"));
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

    pub(super) fn emit_arm_body(&mut self, body: &Expr, dest: &Dest) -> Result<()> {
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

    pub(super) fn emit_match(&mut self, scrutinee: &Expr, arms: &[Arm], dest: &Dest) -> Result<()> {
        let st = self.type_of(scrutinee)?;
        if !matches!(st, Ty::Scalar(Scalar::I32 | Scalar::U32 | Scalar::Bool)) {
            bail!("`match` needs an integer or bool scrutinee, found {}", st.describe());
        }
        let has_wild = arms.iter().any(|a| a.pats.iter().any(|p| matches!(p, Pat::Wild)));
        if !has_wild && st != Ty::bool() {
            bail!("`match` needs a `_` arm");
        }
        // A `break` inside a `switch` leaves the switch, not the loop around it, so those matches
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
            self.push(format!("switch {scr} {{"));
            self.indent += 1;
            let mut default_seen = false;
            for a in arms {
                let mut labels = Vec::new();
                let mut is_default = false;
                for p in &a.pats {
                    match p {
                        Pat::Wild => is_default = true,
                        Pat::Value(v) => {
                            let t = self.emit_expr(v)?;
                            labels.push(t);
                        }
                        Pat::Range { .. } => unreachable!(),
                    }
                }
                if is_default {
                    if default_seen {
                        bail!("`match` has more than one `_` arm");
                    }
                    default_seen = true;
                    labels.push("default".into());
                }
                self.push(format!("case {}: {{", labels.join(", ")));
                self.indent += 1;
                self.emit_arm_body(&a.body, dest)?;
                self.indent -= 1;
                self.push("}");
            }
            self.indent -= 1;
            self.push("}");
            return Ok(());
        }

        // Ranges or a bool scrutinee: an if / else if chain on a copy of the scrutinee.
        let m = if matches!(scrutinee, Expr::Path(_)) && self.is_stable(scrutinee) {
            scr
        } else {
            let t = self.fresh();
            self.push(format!("let {t} = {scr};"));
            t
        };
        let mut first = true;
        let mut closed_with_else = false;
        for a in arms {
            if closed_with_else {
                bail!("`match` arms after `_` are unreachable");
            }
            let mut conds = Vec::new();
            let mut wild = false;
            for p in &a.pats {
                match p {
                    Pat::Wild => wild = true,
                    Pat::Value(v) => {
                        let t = self.emit_expr(v)?;
                        conds.push(format!("{m} == {t}"));
                    }
                    Pat::Range { lo, hi, inclusive } => {
                        let l = self.emit_expr(lo)?;
                        let h = self.emit_expr(hi)?;
                        let op = if *inclusive { "<=" } else { "<" };
                        conds.push(format!("({m} >= {l} && {m} {op} {h})"));
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
                let c = if conds.len() == 1 { conds.remove(0) } else { conds.join(" || ") };
                let c = c.strip_prefix('(').and_then(|x| x.strip_suffix(')')).filter(|x| !x.contains(')') || !x.contains('(')).map(str::to_string).unwrap_or(c);
                if first {
                    self.push(format!("if {c} {{"));
                } else {
                    self.push(format!("}} else if {c} {{"));
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
