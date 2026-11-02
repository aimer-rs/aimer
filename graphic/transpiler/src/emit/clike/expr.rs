//! Expressions for the C-like emitters.

use super::*;
use crate::emit::common::{float_text, int_text};
use crate::error::Error;
use crate::sema::{Scalar, is_constructor, is_intrinsic};

impl<'a> Emitter<'a> {
    fn inline_text(&mut self, e: &Expr) -> Result<String> {
        let t = self.emit_expr(e)?;
        Ok(match e {
            Expr::Int(_) | Expr::Float(_) | Expr::Bool(_) | Expr::Path(_) | Expr::Call { .. } | Expr::Field(..) | Expr::Index(..) | Expr::Cast(..) => t,
            _ => format!("({t})"),
        })
    }

    /// A const is replaced by its value. Arrays that are indexed at run time stay as a global table.
    fn inline_const(&mut self, n: &str) -> Result<String> {
        let c = self.g.consts[n].clone();
        if matches!(c.ty, Ty::Array(..)) {
            return Ok(self.id(n));
        }
        // Const values only see other consts, never the locals of the function using them.
        let saved = std::mem::replace(&mut self.env, Env::new());
        let r = self.inline_text(&c.value);
        self.env = saved;
        r
    }

    fn child_text(&self, parent: BinOp, child: &Expr, text: String, right: bool) -> String {
        if let Expr::Binary(cop, ..) = child {
            let p = parent.prec();
            let c = cop.prec();
            let paren = c < p
                || (c == p && right)
                || parent.is_bitwise()
                || cop.is_bitwise()
                || (parent.is_comparison() && cop.is_comparison())
                || matches!((parent, *cop), (BinOp::And, BinOp::Or) | (BinOp::Or, BinOp::And));
            if paren {
                return format!("({text})");
            }
        }
        text
    }

    fn postfix_base(base: &Expr, text: String) -> String {
        if matches!(base, Expr::Binary(..) | Expr::Unary(..)) { format!("({text})") } else { text }
    }

    /// `T(a, b)`. HLSL spells a one-argument conversion as a cast.
    fn construct(&self, ty_text: &str, args: &[String]) -> String {
        if self.d == Dialect::Hlsl && args.len() == 1 {
            format!("(({ty_text})({}))", args[0])
        } else {
            format!("{ty_text}({})", args.join(", "))
        }
    }

    pub(super) fn emit_expr(&mut self, e: &Expr) -> Result<String> {
        match e {
            Expr::Int(s) => Ok(int_text(s)),
            Expr::Float(s) => Ok(float_text(s)),
            Expr::Bool(b) => Ok(b.to_string()),
            Expr::Path(n) => {
                if let Some(v) = self.env.lookup(n) {
                    Ok(v.emitted.clone())
                } else if self.g.consts.contains_key(n) {
                    self.inline_const(n)
                } else if self.g.resources.contains_key(n) {
                    if self.d == Dialect::Glsl && matches!(self.g.resources[n], Ty::Struct(_)) {
                        return Ok(self.glsl_uniform_name(n));
                    }
                    Ok(self.id(n))
                } else {
                    bail!("unknown name `{n}`")
                }
            }
            Expr::Unary(op, x) => {
                let t = self.emit_expr(x)?;
                if self.d == Dialect::Glsl && *op == UnOp::Not && matches!(self.type_of(x)?, Ty::Vec(..)) {
                    return Ok(format!("not({t})"));
                }
                let t = if matches!(**x, Expr::Binary(..) | Expr::Unary(..)) { format!("({t})") } else { t };
                Ok(format!("{}{t}", op.text()))
            }
            Expr::Binary(op @ (BinOp::And | BinOp::Or), l, r) => self.emit_logical(*op, l, r),
            Expr::Binary(op, l, r) => {
                let lty = self.type_of(l)?;
                let rty = self.type_of(r)?;
                if *op == BinOp::Mul && (matches!(lty, Ty::Mat(_)) || matches!(rty, Ty::Mat(_))) && self.d == Dialect::Hlsl {
                    bail!("matrices are not supported by the HLSL emitter yet");
                }
                let t = self.emit_operands(&[l, r])?;
                // MSL has no `%` on floats. WGSL `%` on floats is a truncated remainder, like `fmod`.
                if *op == BinOp::Rem && self.d == Dialect::Msl && lty.scalar_of() == Some(Scalar::F32) {
                    return Ok(format!("fmod({}, {})", t[0], t[1]));
                }
                if self.d == Dialect::Glsl {
                    if *op == BinOp::Rem && lty.scalar_of() == Some(Scalar::F32) {
                        let ty = if matches!(lty, Ty::Vec(..)) { lty.clone() } else { rty.clone() };
                        let tt = self.ty_text(&ty)?;
                        if !self.glsl_rem.contains(&tt) {
                            self.glsl_rem.push(tt);
                        }
                        return Ok(format!("_aimer_rem({}, {})", t[0], t[1]));
                    }
                    let func = match op {
                        BinOp::Lt => Some("lessThan"),
                        BinOp::Le => Some("lessThanEqual"),
                        BinOp::Gt => Some("greaterThan"),
                        BinOp::Ge => Some("greaterThanEqual"),
                        BinOp::Eq => Some("equal"),
                        BinOp::Ne => Some("notEqual"),
                        _ => None,
                    };
                    if let (Some(func), true) = (func, matches!(lty, Ty::Vec(..)) || matches!(rty, Ty::Vec(..))) {
                        let vt = if matches!(lty, Ty::Vec(..)) { lty.clone() } else { rty.clone() };
                        let tt = self.ty_text(&vt)?;
                        let a = if matches!(lty, Ty::Vec(..)) { t[0].clone() } else { format!("{tt}({})", t[0]) };
                        let b = if matches!(rty, Ty::Vec(..)) { t[1].clone() } else { format!("{tt}({})", t[1]) };
                        return Ok(format!("{func}({a}, {b})"));
                    }
                }
                let lt = self.child_text(*op, l, t[0].clone(), false);
                let rt = self.child_text(*op, r, t[1].clone(), true);
                Ok(format!("{lt} {} {rt}", op.text()))
            }
            Expr::Assign { op, target, value } => {
                self.emit_assign(*op, target, value)?;
                Ok(String::new())
            }
            Expr::Call { name, generics, args } => self.emit_call(e, name, generics, args),
            Expr::Method { recv, name, args } => self.emit_method(recv, name, args),
            Expr::Field(base, f) => {
                let bt = self.type_of(base)?;
                let t = self.emit_expr(base)?;
                let t = Self::postfix_base(base, t);
                match bt {
                    Ty::Vec(..) => Ok(format!("{t}.{f}")),
                    _ => Ok(format!("{t}.{}", self.id(f))),
                }
            }
            Expr::Index(base, idx) => {
                if let Expr::Path(n) = &**base {
                    if self.env.lookup(n).is_none() && self.g.consts.get(n).is_some_and(|c| matches!(c.ty, Ty::Array(..))) {
                        let c = self.g.consts[n].clone();
                        if let (Some(i), Expr::ArrayLit(items)) = (self.g.const_int(idx), &c.value) {
                            let Some(item) = usize::try_from(i).ok().and_then(|i| items.get(i)) else {
                                bail!("index {i} is out of range for const `{n}` ({} elements)", items.len());
                            };
                            let saved = std::mem::replace(&mut self.env, Env::new());
                            let r = self.inline_text(item);
                            self.env = saved;
                            return r;
                        }
                        let table = self.id(n);
                        let t = self.emit_operands(&[idx])?;
                        return Ok(format!("{table}[{}]", t[0]));
                    }
                }
                let t = self.emit_operands(&[base, idx])?;
                Ok(format!("{}[{}]", Self::postfix_base(base, t[0].clone()), t[1]))
            }
            Expr::Cast(x, to) => {
                let to = self.g.resolve_type(to)?;
                let t = self.emit_expr(x)?;
                let tt = self.ty_text(&to)?;
                Ok(match self.d {
                    Dialect::Msl | Dialect::Glsl => format!("{tt}({t})"),
                    Dialect::Hlsl => format!("(({tt})({t}))"),
                })
            }
            Expr::StructLit { name, fields, base } => self.emit_struct_lit(name, fields, base),
            Expr::ArrayLit(_) => bail!("an array literal can only initialise a `let` or a const"),
            Expr::If { select: true, cond, then, els } => self.emit_select(cond, then, els),
            Expr::Block(_) | Expr::If { .. } | Expr::Match { .. } | Expr::Loop(_) => {
                let ty = self.type_of(e)?;
                if matches!(ty, Ty::Unit | Ty::Never) {
                    self.emit_into(e, &Dest::Discard)?;
                    return Ok(String::new());
                }
                let t = self.fresh();
                let decl = self.decl(&ty, &t)?;
                self.push(format!("{decl};"));
                self.emit_into(e, &Dest::Assign(t.clone()))?;
                Ok(t)
            }
            Expr::While { .. } | Expr::For { .. } | Expr::Return(_) | Expr::Break(_) | Expr::Continue | Expr::Discard => {
                self.emit_into(e, &Dest::Discard)?;
                Ok(String::new())
            }
        }
    }

    /// Can `e` be evaluated eagerly? HLSL `&&`, `||` and `?:` may evaluate both sides, so a right side that
    /// indexes or calls a user function is lowered to an `if` instead.
    fn is_eager_safe(&self, e: &Expr) -> bool {
        let mut safe = true;
        walk_expr(e, &mut |x| match x {
            Expr::Index(..) => safe = false,
            Expr::Call { name, .. } if self.g.fns.contains_key(name) => safe = false,
            _ => {}
        });
        safe
    }

    fn emit_logical(&mut self, op: BinOp, l: &Expr, r: &Expr) -> Result<String> {
        let lt = self.emit_expr(l)?;
        self.indent += 1;
        let (rt, rlines) = self.capture(|s| s.emit_expr(r))?;
        self.indent -= 1;
        if rlines.is_empty() && (self.d != Dialect::Hlsl || self.is_eager_safe(r)) {
            let lt = self.child_text(op, l, lt, false);
            let rt = self.child_text(op, r, rt, true);
            return Ok(format!("{lt} {} {rt}", op.text()));
        }
        // The right side needs statements or must not run eagerly: it only runs when the left side allows it.
        let tmp = self.fresh();
        self.push(format!("bool {tmp} = {lt};"));
        self.push(if op == BinOp::And { format!("if ({tmp}) {{") } else { format!("if (!{tmp}) {{") });
        self.lines.extend(rlines);
        self.indent += 1;
        self.push(format!("{tmp} = {rt};"));
        self.indent -= 1;
        self.push("}");
        Ok(tmp)
    }

    fn is_trivial(&self, e: &Expr) -> bool {
        match e {
            Expr::Int(_) | Expr::Float(_) | Expr::Bool(_) | Expr::Path(_) => true,
            Expr::Unary(_, x) | Expr::Field(x, _) | Expr::Cast(x, _) => self.is_trivial(x),
            Expr::Binary(_, a, b) | Expr::Index(a, b) => self.is_trivial(a) && self.is_trivial(b),
            Expr::Call { name, args, .. } => {
                let callable = if self.g.fns.contains_key(name) {
                    crate::emit::common::is_pure_fn(self.p, name)
                } else {
                    is_intrinsic(name) || is_constructor(name)
                };
                callable && args.iter().all(|a| self.is_trivial(a))
            }
            _ => false,
        }
    }

    fn emit_select(&mut self, cond: &Expr, then: &Block, els: &Option<Box<Expr>>) -> Result<String> {
        let arm = |b: &Block| -> Option<Expr> {
            if b.stmts.is_empty() { b.tail.as_deref().cloned() } else { None }
        };
        let (Some(t), Some(Expr::Block(eb))) = (arm(then), els.as_deref()) else {
            bail!("`#[select]` needs `if c {{ a }} else {{ b }}` with plain value arms");
        };
        let Some(f) = arm(eb) else { bail!("`#[select]` needs `if c {{ a }} else {{ b }}` with plain value arms") };
        if !self.is_trivial(&t) || !self.is_trivial(&f) {
            bail!("`#[select]` arms must be trivial expressions (no blocks or control flow, and only calls to pure functions); `select` evaluates both arms");
        }
        // The condition is a single bool, so a ternary does the job in both targets.
        let texts = self.emit_operands(&[cond, &t, &f])?;
        let c = if matches!(cond, Expr::Path(_) | Expr::Call { .. } | Expr::Field(..)) { texts[0].clone() } else { format!("({})", texts[0]) };
        Ok(format!("({c} ? {} : {})", texts[1], texts[2]))
    }

    fn emit_method(&mut self, recv: &Expr, name: &str, args: &[Expr]) -> Result<String> {
        if self.d == Dialect::Glsl {
            return self.glsl_method(recv, name, args);
        }
        let mut es: Vec<&Expr> = vec![recv];
        es.extend(args.iter());
        let arg_tys: Vec<Ty> = args.iter().map(|a| self.type_of(a)).collect::<Result<_>>()?;
        let t = self.emit_operands(&es)?;
        let tex = Self::postfix_base(recv, t[0].clone());
        let bad = || Error::new(format!("`.{name}(..)` called with the wrong number of arguments"));
        Ok(match (name, t.len(), self.d) {
            ("sample", 3, Dialect::Msl) => format!("{tex}.sample({}, {})", t[1], t[2]),
            ("sample", 3, Dialect::Hlsl) => format!("{tex}.Sample({}, {})", t[1], t[2]),
            ("sample_level", 4, Dialect::Msl) => format!("{tex}.sample({}, {}, level({}))", t[1], t[2], t[3]),
            ("sample_level", 4, Dialect::Hlsl) => format!("{tex}.SampleLevel({}, {}, {})", t[1], t[2], t[3]),
            ("load", 3, Dialect::Msl) => format!("{tex}.read(uint2({}), uint({}))", t[1], t[2]),
            ("load", 3, Dialect::Hlsl) => {
                let coord = if arg_tys[0] == Ty::Vec(2, Scalar::I32) { t[1].clone() } else { format!("((int2)({}))", t[1]) };
                let lod = if arg_tys[1] == Ty::Scalar(Scalar::I32) { t[2].clone() } else { format!("((int)({}))", t[2]) };
                format!("{tex}.Load(int3({coord}, {lod}))")
            }
            ("size", 1, Dialect::Msl) => format!("uint2({tex}.get_width(), {tex}.get_height())"),
            (_, _, Dialect::Glsl) => unreachable!(),
            ("size", 1, Dialect::Hlsl) => {
                self.needs_texture_size = true;
                format!("_aimer_texture_size({tex})")
            }
            ("sample" | "sample_level" | "load" | "size", ..) => return Err(bad()),
            _ => bail!("unknown texture method `{name}`"),
        })
    }

    fn emit_call(&mut self, call: &Expr, name: &str, generics: &[Type], args: &[Expr]) -> Result<String> {
        let refs: Vec<&Expr> = args.iter().collect();
        if self.g.fns.contains_key(name) {
            if self.d == Dialect::Glsl {
                if let Some(call) = self.glsl_call_spec(name, args)? {
                    return Ok(call);
                }
            }
            let mut t = self.emit_operands(&refs)?;
            // MSL helpers cannot read global resources: pass the ones the callee needs.
            if self.d == Dialect::Msl {
                for r in self.uses.get(name).cloned().unwrap_or_default() {
                    t.push(self.id(&r));
                }
            }
            return Ok(format!("{}({})", self.id(name), t.join(", ")));
        }
        let arg0 = match args.first() {
            Some(a) => Some(self.type_of(a)?),
            None => None,
        };
        let t = self.emit_operands(&refs)?;
        match name {
            "f32" | "i32" | "u32" | "bool" => {
                let ty = self.type_of(call)?;
                Ok(self.construct(&self.ty_text(&ty)?, &t))
            }
            "vec2" | "vec3" | "vec4" | "mat2" | "mat3" | "mat4" => {
                let ty = self.type_of(call)?;
                Ok(self.construct(&self.ty_text(&ty)?, &t))
            }
            "bitcast" => {
                let Some(g) = generics.first() else { bail!("`bitcast` needs a target type") };
                let to = self.g.resolve_type(g)?;
                Ok(match self.d {
                    Dialect::Glsl => {
                        let from = arg0.as_ref().and_then(|a| a.scalar_of());
                        let Some(from) = from else { bail!("`bitcast` needs a numeric argument") };
                        let tt = self.ty_text(&to)?;
                        match (from, to.scalar_of()) {
                            (a, Some(b)) if a == b => t.join(", "),
                            (Scalar::F32, Some(Scalar::I32)) => format!("floatBitsToInt({})", t.join(", ")),
                            (Scalar::F32, Some(Scalar::U32)) => format!("floatBitsToUint({})", t.join(", ")),
                            (Scalar::I32, Some(Scalar::F32)) => format!("intBitsToFloat({})", t.join(", ")),
                            (Scalar::U32, Some(Scalar::F32)) => format!("uintBitsToFloat({})", t.join(", ")),
                            (Scalar::I32, Some(Scalar::U32)) | (Scalar::U32, Some(Scalar::I32)) => format!("{tt}({})", t.join(", ")),
                            _ => bail!("`bitcast` to {} is not supported in GLSL", to.describe()),
                        }
                    }
                    Dialect::Msl => format!("as_type<{}>({})", self.ty_text(&to)?, t.join(", ")),
                    Dialect::Hlsl => {
                        let f = match to.scalar_of() {
                            Some(Scalar::F32) => "asfloat",
                            Some(Scalar::I32) => "asint",
                            Some(Scalar::U32) => "asuint",
                            _ => bail!("`bitcast` to {} is not supported in HLSL", to.describe()),
                        };
                        format!("{f}({})", t.join(", "))
                    }
                })
            }
            "fma" if self.d == Dialect::Glsl => bail!("`fma` needs GLSL 4.00 or GLSL ES 3.20; it is not available in GLSL 3.30 and GLSL ES 3.00"),
            n if is_intrinsic(n) => Ok(dialect::intrinsic_call(self.d, n, &t, arg0.as_ref())),
            n => bail!("unknown function `{n}`"),
        }
    }

    /// A struct value is built in a local: declare it, then assign the fields in declaration order.
    fn emit_struct_lit(&mut self, name: &str, fields: &[(String, Expr)], base: &Option<Box<Expr>>) -> Result<String> {
        let decl = self.g.structs[name].clone();
        let refs: Vec<&Expr> = fields.iter().map(|(_, v)| v).collect();
        let texts = self.emit_operands(&refs)?;
        let tmp = self.fresh();
        let sty = self.id(name);
        match base {
            Some(b) => {
                let t = self.emit_expr(b)?;
                self.push(format!("{sty} {tmp} = {t};"));
            }
            None => {
                for f in &decl {
                    if !fields.iter().any(|(n, _)| *n == f.name) {
                        bail!("struct literal `{name}` is missing field `{}`", f.name);
                    }
                }
                self.push(format!("{sty} {tmp};"));
            }
        }
        for f in &decl {
            if let Some(i) = fields.iter().position(|(n, _)| *n == f.name) {
                self.push(format!("{tmp}.{} = {};", self.id(&f.name), texts[i]));
            }
        }
        Ok(tmp)
    }

    fn emit_lvalue(&mut self, e: &Expr) -> Result<String> {
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
                        let mut seen = Vec::new();
                        for c in f.chars() {
                            if seen.contains(&c) {
                                bail!("cannot assign to the swizzle `.{f}`: component `{c}` appears twice");
                            }
                            seen.push(c);
                        }
                        Ok(format!("{base}.{f}"))
                    }
                    _ => Ok(format!("{base}.{}", self.id(f))),
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

    fn emit_assign(&mut self, op: Option<BinOp>, target: &Expr, value: &Expr) -> Result<()> {
        let v = self.emit_expr(value)?;
        let t = self.emit_lvalue(target)?;
        match op {
            None => self.push(format!("{t} = {v};")),
            Some(op) => self.push(format!("{t} {}= {v};", op.text())),
        }
        Ok(())
    }
}
