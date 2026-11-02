//! Types and expression typing. Shared by every emitter.

use std::collections::HashMap;

use crate::ast::*;
use crate::error::{Result, bail};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scalar {
    F32,
    I32,
    U32,
    Bool,
}

impl Scalar {
    pub fn name(self) -> &'static str {
        match self {
            Scalar::F32 => "f32",
            Scalar::I32 => "i32",
            Scalar::U32 => "u32",
            Scalar::Bool => "bool",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    Scalar(Scalar),
    Vec(u8, Scalar),
    Mat(u8),
    Array(Box<Ty>, Box<Expr>),
    Struct(String),
    Unit,
    Never,
    Texture,
    Sampler,
}

impl Ty {
    pub fn f32() -> Ty {
        Ty::Scalar(Scalar::F32)
    }

    pub fn bool() -> Ty {
        Ty::Scalar(Scalar::Bool)
    }

    /// Element type of a scalar or vector.
    pub fn scalar_of(&self) -> Option<Scalar> {
        match self {
            Ty::Scalar(s) | Ty::Vec(_, s) => Some(*s),
            _ => None,
        }
    }

    pub fn components(&self) -> Option<u8> {
        match self {
            Ty::Scalar(_) => Some(1),
            Ty::Vec(n, _) => Some(*n),
            _ => None,
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Ty::Scalar(s) => s.name().into(),
            Ty::Vec(n, Scalar::F32) => format!("vec{n}"),
            Ty::Vec(n, s) => format!("vec{n}<{}>", s.name()),
            Ty::Mat(n) => format!("mat{n}"),
            Ty::Array(t, _) => format!("[{}; _]", t.describe()),
            Ty::Struct(n) => n.clone(),
            Ty::Unit => "()".into(),
            Ty::Never => "!".into(),
            Ty::Texture => "texture2d".into(),
            Ty::Sampler => "sampler".into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FnSig {
    pub params: Vec<Ty>,
    pub ret: Ty,
}

#[derive(Debug, Clone)]
pub struct ConstInfo {
    pub ty: Ty,
    pub value: Expr,
}

#[derive(Debug, Clone, Default)]
pub struct Globals {
    pub structs: HashMap<String, Vec<Field>>,
    pub consts: HashMap<String, ConstInfo>,
    pub fns: HashMap<String, FnSig>,
    pub resources: HashMap<String, Ty>,
}

#[derive(Debug, Clone)]
pub struct Var {
    pub ty: Ty,
    pub mutable: bool,
    /// Name written to the target.
    pub emitted: String,
}

/// What the `break`s of one loop carry, collected while its body is typed.
#[derive(Debug, Clone, Default)]
struct LoopFrame {
    /// Type of every `break value`.
    values: Vec<Ty>,
    /// Is there a `break` without a value?
    bare: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Env {
    scopes: Vec<HashMap<String, Var>>,
    /// One frame per enclosing loop being typed (`loop`, `while` and `for`).
    loops: Vec<LoopFrame>,
}

impl Env {
    pub fn new() -> Env {
        Env { scopes: vec![HashMap::new()], loops: Vec::new() }
    }

    pub fn push(&mut self) {
        self.scopes.push(HashMap::new());
    }

    pub fn pop(&mut self) {
        self.scopes.pop();
    }

    pub fn declare(&mut self, name: &str, var: Var) {
        self.scopes.last_mut().expect("scope").insert(name.to_string(), var);
    }

    pub fn lookup(&self, name: &str) -> Option<&Var> {
        self.scopes.iter().rev().find_map(|s| s.get(name))
    }

    /// Reserve an emitted name without making it visible to source lookups.
    pub fn declare_hidden(&mut self, emitted: &str) {
        let var = Var { ty: Ty::Unit, mutable: false, emitted: emitted.to_string() };
        self.scopes.last_mut().expect("scope").insert(format!("${emitted}"), var);
    }

    /// Is `emitted` already used by a visible local?
    pub fn name_taken(&self, emitted: &str) -> bool {
        self.scopes.iter().any(|s| s.values().any(|v| v.emitted == emitted))
    }
}

const SAME_AS_FIRST: &[&str] = &[
    "abs", "sign", "floor", "ceil", "round", "trunc", "fract", "sqrt", "inverseSqrt", "sin", "cos", "tan", "asin",
    "acos", "atan", "sinh", "cosh", "tanh", "exp", "exp2", "log", "log2", "saturate", "normalize", "min", "max",
    "clamp", "mix", "pow", "fma", "atan2", "reflect", "cross", "dpdx", "dpdy", "fwidth", "degrees", "radians",
];
const SAME_AS_LAST: &[&str] = &["step", "smoothstep"];
const ELEM_OF_FIRST: &[&str] = &["length", "dot", "distance"];
const BOOL_RESULT: &[&str] = &["all", "any"];

pub fn is_intrinsic(name: &str) -> bool {
    SAME_AS_FIRST.contains(&name)
        || SAME_AS_LAST.contains(&name)
        || ELEM_OF_FIRST.contains(&name)
        || BOOL_RESULT.contains(&name)
}

pub fn is_constructor(name: &str) -> bool {
    matches!(
        name,
        "f32" | "i32" | "u32" | "bool" | "vec2" | "vec3" | "vec4" | "mat2" | "mat3" | "mat4" | "bitcast"
    )
}

/// Resolved swizzle: component count, or `None` if `name` is not a swizzle of an `n`-component vector.
pub fn swizzle_len(name: &str, n: u8) -> Option<u8> {
    let set = if name.chars().all(|c| "xyzw".contains(c)) {
        "xyzw"
    } else if name.chars().all(|c| "rgba".contains(c)) {
        "rgba"
    } else {
        return None;
    };
    if name.is_empty() || name.len() > 4 {
        return None;
    }
    if name.chars().all(|c| set.find(c).is_some_and(|i| (i as u8) < n)) {
        Some(name.len() as u8)
    } else {
        None
    }
}

pub fn parse_int_literal(s: &str) -> Option<i64> {
    let t = s.trim_end_matches("u32").trim_end_matches("i32").trim_end_matches('u');
    match t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        Some(h) => i64::from_str_radix(h, 16).ok(),
        None => t.parse().ok(),
    }
}

/// The variable at the root of an assignment target (`a.b[2].x` gives `a`).
pub fn root_var(e: &Expr) -> Option<&str> {
    match e {
        Expr::Path(n) => Some(n),
        Expr::Field(b, _) | Expr::Index(b, _) => root_var(b),
        _ => None,
    }
}

fn has_break(b: &Block) -> bool {
    fn in_expr(e: &Expr) -> bool {
        match e {
            Expr::Break(_) => true,
            Expr::If { then, els, .. } => has_break(then) || els.as_deref().is_some_and(in_expr),
            Expr::Block(b) => has_break(b),
            Expr::Match { arms, .. } => arms.iter().any(|a| in_expr(&a.body)),
            _ => false,
        }
    }
    b.stmts.iter().any(|s| matches!(s, Stmt::Expr(e) if in_expr(e))) || b.tail.as_deref().is_some_and(in_expr)
}

impl Globals {
    pub fn resolve_type(&self, t: &Type) -> Result<Ty> {
        Ok(match t {
            Type::Unit => Ty::Unit,
            Type::Builtin(n) => match n.as_str() {
                "vertex_index" | "instance_index" => Ty::Scalar(Scalar::U32),
                "frag_coord" | "frag_color" => Ty::Vec(4, Scalar::F32),
                _ => bail!("unknown builtin type `@{n}`"),
            },
            Type::Array(elem, n) => {
                let size = match self.const_int(n) {
                    Some(v) => Box::new(Expr::Int(v.to_string())),
                    None => n.clone(),
                };
                Ty::Array(Box::new(self.resolve_type(elem)?), size)
            }
            Type::Named(name, args) => {
                let scalar_arg = |args: &[Type]| -> Result<Scalar> {
                    match args.first() {
                        None => Ok(Scalar::F32),
                        Some(a) => match self.resolve_type(a)? {
                            Ty::Scalar(s) => Ok(s),
                            other => bail!("`{name}` element type must be a scalar, found {}", other.describe()),
                        },
                    }
                };
                match name.as_str() {
                    "f32" => Ty::Scalar(Scalar::F32),
                    "i32" => Ty::Scalar(Scalar::I32),
                    "u32" => Ty::Scalar(Scalar::U32),
                    "bool" => Ty::Scalar(Scalar::Bool),
                    "vec2" => Ty::Vec(2, scalar_arg(args)?),
                    "vec3" => Ty::Vec(3, scalar_arg(args)?),
                    "vec4" => Ty::Vec(4, scalar_arg(args)?),
                    "mat2" => Ty::Mat(2),
                    "mat3" => Ty::Mat(3),
                    "mat4" => Ty::Mat(4),
                    "texture2d" => Ty::Texture,
                    "sampler" => Ty::Sampler,
                    n if self.structs.contains_key(n) => Ty::Struct(n.to_string()),
                    n => bail!("unknown type `{n}`"),
                }
            }
        })
    }

    /// Value of a compile-time integer expression (literals, consts, `+ - * /`).
    pub fn const_int(&self, e: &Expr) -> Option<i64> {
        match e {
            Expr::Int(s) => parse_int_literal(s),
            Expr::Path(n) => self.consts.get(n).and_then(|c| self.const_int(&c.value)),
            Expr::Binary(op, a, b) => {
                let (a, b) = (self.const_int(a)?, self.const_int(b)?);
                match op {
                    BinOp::Add => a.checked_add(b),
                    BinOp::Sub => a.checked_sub(b),
                    BinOp::Mul => a.checked_mul(b),
                    BinOp::Div if b != 0 => Some(a / b),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    pub fn field_type(&self, struct_name: &str, field: &str) -> Result<Ty> {
        let fields = self.structs.get(struct_name).ok_or_else(|| crate::error::Error::new(format!("unknown struct `{struct_name}`")))?;
        match fields.iter().find(|f| f.name == field) {
            Some(f) => self.resolve_type(&f.ty),
            None => bail!("struct `{struct_name}` has no field `{field}`"),
        }
    }

    pub fn type_of_block(&self, env: &mut Env, b: &Block) -> Result<Ty> {
        env.push();
        let r = self.type_of_block_inner(env, b);
        env.pop();
        r
    }

    /// Types a block in the current scope (the caller owns the scope).
    pub fn type_of_block_inner(&self, env: &mut Env, b: &Block) -> Result<Ty> {
        let mut diverges = false;
        for s in &b.stmts {
            match s {
                Stmt::Let { mutable, name, ty, init } => {
                    let declared = ty.as_ref().map(|t| self.resolve_type(t)).transpose()?;
                    let inferred = match init {
                        Some(e) => Some(self.type_of(env, e)?),
                        None => None,
                    };
                    let ty = match (declared, inferred) {
                        (Some(d), Some(i)) if i != d && i != Ty::Never => {
                            bail!("`{name}`: expected {}, found {}", d.describe(), i.describe())
                        }
                        (Some(d), _) => d,
                        (None, Some(i)) => i,
                        (None, None) => bail!("`{name}` needs a type annotation or an initialiser"),
                    };
                    env.declare(name, Var { ty, mutable: *mutable, emitted: name.clone() });
                }
                Stmt::Expr(e) => {
                    if self.type_of(env, e)? == Ty::Never {
                        diverges = true;
                    }
                }
            }
        }
        match &b.tail {
            Some(e) => self.type_of(env, e),
            None => Ok(if diverges { Ty::Never } else { Ty::Unit }),
        }
    }

    pub fn type_of(&self, env: &mut Env, e: &Expr) -> Result<Ty> {
        match e {
            Expr::Int(s) => Ok(Ty::Scalar(if s.ends_with('u') || s.ends_with("u32") { Scalar::U32 } else { Scalar::I32 })),
            Expr::Float(_) => Ok(Ty::f32()),
            Expr::Bool(_) => Ok(Ty::bool()),
            Expr::Path(n) => {
                if let Some(v) = env.lookup(n) {
                    Ok(v.ty.clone())
                } else if let Some(c) = self.consts.get(n) {
                    Ok(c.ty.clone())
                } else if let Some(r) = self.resources.get(n) {
                    Ok(r.clone())
                } else {
                    bail!("unknown name `{n}`")
                }
            }
            Expr::Unary(op, x) => {
                let t = self.type_of(env, x)?;
                match op {
                    UnOp::Not if t.scalar_of() == Some(Scalar::Bool) => Ok(t),
                    UnOp::Neg if matches!(t.scalar_of(), Some(Scalar::F32 | Scalar::I32)) => Ok(t),
                    UnOp::BitNot if matches!(t.scalar_of(), Some(Scalar::I32 | Scalar::U32)) => Ok(t),
                    _ => bail!("cannot apply `{}` to {}", op.text(), t.describe()),
                }
            }
            Expr::Binary(op, l, r) => {
                let lt = self.type_of(env, l)?;
                let rt = self.type_of(env, r)?;
                self.binary_type(*op, &lt, &rt)
            }
            Expr::Assign { op, target, value } => {
                let tt = self.type_of(env, target)?;
                let vt = self.type_of(env, value)?;
                let Some(root) = root_var(target) else { bail!("invalid assignment target") };
                match env.lookup(root) {
                    Some(v) if !v.mutable => bail!("cannot assign to immutable variable `{root}`; declare it with `let mut`"),
                    Some(_) => {}
                    None => bail!("cannot assign to `{root}`: only local variables can be assigned"),
                }
                match op {
                    None => {
                        if vt != tt && vt != Ty::Never {
                            bail!("mismatched types in assignment: expected {}, found {}", tt.describe(), vt.describe());
                        }
                    }
                    Some(op) => {
                        let r = self.binary_type(*op, &tt, &vt)?;
                        if r != tt {
                            bail!("`{}=` would change the type from {} to {}", op.text(), tt.describe(), r.describe());
                        }
                    }
                }
                Ok(Ty::Unit)
            }
            Expr::Call { name, generics, args } => {
                let tys = args.iter().map(|a| self.type_of(env, a)).collect::<Result<Vec<_>>>()?;
                if let Some(sig) = self.fns.get(name) {
                    if sig.params.len() != tys.len() {
                        bail!("`{name}` takes {} argument(s), {} given", sig.params.len(), tys.len());
                    }
                    for (i, (p, a)) in sig.params.iter().zip(&tys).enumerate() {
                        if p != a {
                            bail!("argument {} of `{name}`: expected {}, found {}", i + 1, p.describe(), a.describe());
                        }
                    }
                    return Ok(sig.ret.clone());
                }
                self.builtin_call_type(name, generics, &tys)
            }
            Expr::Method { recv, name, args } => {
                let rt = self.type_of(env, recv)?;
                for a in args {
                    self.type_of(env, a)?;
                }
                if rt != Ty::Texture {
                    bail!("`.{name}()` is only available on textures, found {}", rt.describe());
                }
                match name.as_str() {
                    "sample" | "sample_level" | "load" => Ok(Ty::Vec(4, Scalar::F32)),
                    "size" => Ok(Ty::Vec(2, Scalar::U32)),
                    _ => bail!("unknown texture method `{name}`"),
                }
            }
            Expr::Field(base, f) => match self.type_of(env, base)? {
                Ty::Struct(s) => self.field_type(&s, f),
                Ty::Vec(n, s) => match swizzle_len(f, n) {
                    Some(1) => Ok(Ty::Scalar(s)),
                    Some(k) => Ok(Ty::Vec(k, s)),
                    None => bail!("`{f}` is not a valid swizzle of a {}-component vector", n),
                },
                other => bail!("{} has no field `{f}`", other.describe()),
            },
            Expr::Index(base, idx) => {
                let it = self.type_of(env, idx)?;
                if !matches!(it, Ty::Scalar(Scalar::I32 | Scalar::U32)) {
                    bail!("index must be i32 or u32, found {}", it.describe());
                }
                match self.type_of(env, base)? {
                    Ty::Array(t, _) => Ok(*t),
                    Ty::Vec(_, s) => Ok(Ty::Scalar(s)),
                    Ty::Mat(n) => Ok(Ty::Vec(n, Scalar::F32)),
                    other => bail!("cannot index {}", other.describe()),
                }
            }
            Expr::Cast(x, t) => {
                let from = self.type_of(env, x)?;
                let to = self.resolve_type(t)?;
                if !matches!(from, Ty::Scalar(_)) || !matches!(to, Ty::Scalar(_)) {
                    bail!("`as` converts scalars only, found {} as {}", from.describe(), to.describe());
                }
                Ok(to)
            }
            Expr::StructLit { name, fields, base } => {
                let Some(decl) = self.structs.get(name) else { bail!("unknown struct `{name}`") };
                for (f, v) in fields {
                    let want = self.field_type(name, f)?;
                    let got = self.type_of(env, v)?;
                    if want != got {
                        bail!("field `{f}` of `{name}`: expected {}, found {}", want.describe(), got.describe());
                    }
                }
                match base {
                    Some(b) => {
                        let bt = self.type_of(env, b)?;
                        if bt != Ty::Struct(name.clone()) {
                            bail!("`..` base must be a `{name}`, found {}", bt.describe());
                        }
                    }
                    None => {
                        for f in decl {
                            if !fields.iter().any(|(n, _)| *n == f.name) {
                                bail!("struct literal `{name}` is missing field `{}`", f.name);
                            }
                        }
                    }
                }
                for (i, (f, _)) in fields.iter().enumerate() {
                    if fields[..i].iter().any(|(g, _)| g == f) {
                        bail!("field `{f}` given twice in `{name}`");
                    }
                }
                Ok(Ty::Struct(name.clone()))
            }
            Expr::ArrayLit(items) => {
                let Some(first) = items.first() else { bail!("empty array literal") };
                let t = self.type_of(env, first)?;
                for it in &items[1..] {
                    let u = self.type_of(env, it)?;
                    if u != t {
                        bail!("array elements differ: {} and {}", t.describe(), u.describe());
                    }
                }
                Ok(Ty::Array(Box::new(t), Box::new(Expr::Int(items.len().to_string()))))
            }
            Expr::Block(b) => self.type_of_block(env, b),
            Expr::If { cond, then, els, .. } => {
                let ct = self.type_of(env, cond)?;
                if ct != Ty::bool() {
                    bail!("`if` condition must be bool, found {}", ct.describe());
                }
                let tt = self.type_of_block(env, then)?;
                let Some(els) = els else {
                    if !matches!(tt, Ty::Unit | Ty::Never) {
                        bail!("`if` without `else` must have type (), found {}", tt.describe());
                    }
                    return Ok(Ty::Unit);
                };
                let et = self.type_of(env, els)?;
                match (tt, et) {
                    (Ty::Never, e) => Ok(e),
                    (t, Ty::Never) => Ok(t),
                    (t, e) if t == e => Ok(t),
                    (t, e) => bail!("`if` branches differ: {} and {}", t.describe(), e.describe()),
                }
            }
            Expr::Match { scrutinee, arms } => {
                self.type_of(env, scrutinee)?;
                let mut out: Option<Ty> = None;
                for a in arms {
                    let t = self.type_of(env, &a.body)?;
                    match (&out, t) {
                        (_, Ty::Never) => {}
                        (None, t) => out = Some(t),
                        (Some(o), t) if *o == t => {}
                        (Some(o), t) => bail!("`match` arms differ: {} and {}", o.describe(), t.describe()),
                    }
                }
                Ok(out.unwrap_or(Ty::Never))
            }
            Expr::Loop(b) => {
                env.loops.push(LoopFrame::default());
                let r = self.type_of_block(env, b);
                let frame = env.loops.pop().expect("loop frame");
                r?;
                if frame.values.is_empty() {
                    return Ok(if frame.bare || has_break(b) { Ty::Unit } else { Ty::Never });
                }
                if frame.bare {
                    bail!("a `loop` is used as a value, so every `break` must carry a value");
                }
                let mut out: Option<Ty> = None;
                for t in frame.values {
                    match (&out, t) {
                        (_, Ty::Never) => {}
                        (None, t) => out = Some(t),
                        (Some(o), t) if *o == t => {}
                        (Some(o), t) => bail!("`break` values differ: {} and {}", o.describe(), t.describe()),
                    }
                }
                Ok(out.unwrap_or(Ty::Never))
            }
            Expr::While { cond, body } => {
                let ct = self.type_of(env, cond)?;
                if ct != Ty::bool() {
                    bail!("`while` condition must be bool, found {}", ct.describe());
                }
                env.loops.push(LoopFrame::default());
                let r = self.type_of_block(env, body);
                env.loops.pop();
                r?;
                Ok(Ty::Unit)
            }
            Expr::For { var, lo, hi, body, .. } => {
                let lt = self.type_of(env, lo)?;
                let ht = self.type_of(env, hi)?;
                if lt != ht || !matches!(lt, Ty::Scalar(Scalar::I32 | Scalar::U32)) {
                    bail!("`for` range bounds must be the same integer type, found {} and {}", lt.describe(), ht.describe());
                }
                env.push();
                env.declare(var, Var { ty: lt, mutable: false, emitted: var.clone() });
                env.loops.push(LoopFrame::default());
                let r = self.type_of_block(env, body);
                env.loops.pop();
                env.pop();
                r?;
                Ok(Ty::Unit)
            }
            Expr::Break(v) => {
                let t = match v {
                    Some(v) => Some(self.type_of(env, v)?),
                    None => None,
                };
                // Outside of any loop frame (the emitter types a fragment of a loop body) there is
                // nothing to record. The emitter reports a misplaced `break` itself.
                if let Some(f) = env.loops.last_mut() {
                    match t {
                        Some(t) => f.values.push(t),
                        None => f.bare = true,
                    }
                }
                Ok(Ty::Never)
            }
            Expr::Return(_) | Expr::Continue => Ok(Ty::Never),
            Expr::Discard => Ok(Ty::Unit),
        }
    }

    fn binary_type(&self, op: BinOp, l: &Ty, r: &Ty) -> Result<Ty> {
        let mismatch = || crate::error::Error::new(format!("cannot apply `{}` to {} and {}", op.text(), l.describe(), r.describe()));
        match op {
            BinOp::And | BinOp::Or => {
                if *l == Ty::bool() && *r == Ty::bool() {
                    Ok(Ty::bool())
                } else {
                    Err(mismatch())
                }
            }
            _ if op.is_comparison() => {
                if l == r {
                    match l {
                        Ty::Scalar(_) => Ok(Ty::bool()),
                        Ty::Vec(n, _) => Ok(Ty::Vec(*n, Scalar::Bool)),
                        _ => Err(mismatch()),
                    }
                } else {
                    Err(mismatch())
                }
            }
            BinOp::Shl | BinOp::Shr => Ok(l.clone()),
            _ => {
                if l == r {
                    return match l {
                        Ty::Scalar(Scalar::Bool) | Ty::Vec(_, Scalar::Bool) => Err(mismatch()),
                        Ty::Scalar(_) | Ty::Vec(..) => Ok(l.clone()),
                        Ty::Mat(_) if matches!(op, BinOp::Mul | BinOp::Add | BinOp::Sub) => Ok(l.clone()),
                        _ => Err(mismatch()),
                    };
                }
                match (l, r) {
                    (Ty::Scalar(a), Ty::Vec(_, b)) if a == b && *a != Scalar::Bool => Ok(r.clone()),
                    (Ty::Vec(_, a), Ty::Scalar(b)) if a == b && *a != Scalar::Bool => Ok(l.clone()),
                    (Ty::Scalar(Scalar::F32), Ty::Mat(_)) | (Ty::Mat(_), Ty::Scalar(Scalar::F32)) if op == BinOp::Mul => {
                        Ok(if matches!(l, Ty::Mat(_)) { l.clone() } else { r.clone() })
                    }
                    (Ty::Mat(n), Ty::Vec(m, Scalar::F32)) if n == m && op == BinOp::Mul => Ok(r.clone()),
                    (Ty::Vec(m, Scalar::F32), Ty::Mat(n)) if n == m && op == BinOp::Mul => Ok(l.clone()),
                    _ => Err(mismatch()),
                }
            }
        }
    }

    fn builtin_call_type(&self, name: &str, generics: &[Type], args: &[Ty]) -> Result<Ty> {
        match name {
            "f32" | "i32" | "u32" | "bool" => {
                let to = self.resolve_type(&Type::Named(name.into(), vec![]))?;
                match args {
                    [Ty::Scalar(_)] => Ok(to),
                    _ => bail!("`{name}(..)` takes one scalar argument"),
                }
            }
            "vec2" | "vec3" | "vec4" => {
                let n: u8 = name[3..].parse().unwrap();
                let elem = match generics.first() {
                    Some(g) => match self.resolve_type(g)? {
                        Ty::Scalar(s) => s,
                        other => bail!("`{name}` element type must be a scalar, found {}", other.describe()),
                    },
                    None => args.iter().find_map(Ty::scalar_of).unwrap_or(Scalar::F32),
                };
                let total: u32 = args.iter().map(|a| a.components().map(u32::from).unwrap_or(0)).sum();
                if args.iter().any(|a| a.components().is_none()) {
                    bail!("`{name}(..)` arguments must be scalars or vectors");
                }
                if !args.is_empty() && total != u32::from(n) && total != 1 {
                    bail!("`{name}(..)` needs {n} components or one value to splat, got {total}");
                }
                Ok(Ty::Vec(n, elem))
            }
            "mat2" | "mat3" | "mat4" => Ok(Ty::Mat(name[3..].parse().unwrap())),
            "bitcast" => match generics.first() {
                Some(g) => self.resolve_type(g),
                None => bail!("`bitcast` needs a target type: `bitcast<u32>(x)`"),
            },
            n if SAME_AS_FIRST.contains(&n) => args.first().cloned().ok_or_else(|| crate::error::Error::new(format!("`{n}` needs arguments"))),
            n if SAME_AS_LAST.contains(&n) => args.last().cloned().ok_or_else(|| crate::error::Error::new(format!("`{n}` needs arguments"))),
            n if ELEM_OF_FIRST.contains(&n) => match args.first().and_then(Ty::scalar_of) {
                Some(s) => Ok(Ty::Scalar(s)),
                None => bail!("`{n}` needs a scalar or vector argument"),
            },
            n if BOOL_RESULT.contains(&n) => Ok(Ty::bool()),
            n => bail!("unknown function `{n}`"),
        }
    }
}
