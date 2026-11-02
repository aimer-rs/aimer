//! Assignment targets and swizzle assignment for the WGSL emitter.

use super::*;

impl<'a> Emitter<'a> {
    pub(super) fn emit_lvalue(&mut self, e: &Expr) -> Result<String> {
        match e {
            Expr::Path(n) => match self.env.lookup(n) {
                Some(v) => Ok(v.emitted.clone()),
                None => bail!("cannot assign to `{n}`: only local variables can be assigned"),
            },
            Expr::Field(b, f) => {
                let bt = self.type_of(b)?;
                let base = self.emit_lvalue(b)?;
                match bt {
                    Ty::Vec(..) => {
                        if f.len() != 1 {
                            bail!("internal: multi-component swizzle assignment must go through `emit_assign`");
                        }
                        Ok(format!("{base}.{f}"))
                    }
                    _ => Ok(format!("{base}.{}", ident(f))),
                }
            }
            Expr::Index(b, i) => {
                let base = self.emit_lvalue(b)?;
                let idx = self.emit_expr(i)?;
                Ok(format!("{base}[{idx}]"))
            }
            _ => bail!("invalid assignment target"),
        }
    }

    /// `v.xy = e` is not valid WGSL. It becomes one write per component, with `e` evaluated once.
    pub(super) fn emit_swizzle_assign(&mut self, op: Option<BinOp>, base: &Expr, f: &str, value: &Expr) -> Result<()> {
        let mut seen = Vec::new();
        for c in f.chars() {
            if seen.contains(&c) {
                bail!("cannot assign to the swizzle `.{f}`: component `{c}` appears twice");
            }
            seen.push(c);
        }
        let v = self.emit_expr(value)?;
        let src = if matches!(value, Expr::Path(_)) {
            v
        } else {
            let tmp = self.fresh();
            self.push(format!("let {tmp} = {v};"));
            tmp
        };
        let b = self.emit_lvalue(base)?;
        for (i, c) in f.chars().enumerate() {
            let comp = ["x", "y", "z", "w"][i];
            match op {
                None => self.push(format!("{b}.{c} = {src}.{comp};")),
                Some(op) => self.push(format!("{b}.{c} {}= {src}.{comp};", op.text())),
            }
        }
        Ok(())
    }

    pub(super) fn emit_assign(&mut self, op: Option<BinOp>, target: &Expr, value: &Expr) -> Result<()> {
        if let Expr::Field(b, f) = target {
            if f.len() > 1 && matches!(self.type_of(b)?, Ty::Vec(..)) {
                return self.emit_swizzle_assign(op, b, f, value);
            }
        }
        let v = self.emit_expr(value)?;
        let t = self.emit_lvalue(target)?;
        match op {
            None => self.push(format!("{t} = {v};")),
            Some(op) => self.push(format!("{t} {}= {v};", op.text())),
        }
        Ok(())
    }
}
