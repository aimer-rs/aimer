use crate::lexer::Tok;

#[derive(Debug, Clone, PartialEq)]
pub struct Attr {
    pub name: String,
    /// Raw tokens between the parentheses, if any.
    pub args: Vec<Tok>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Type {
    /// `f32`, `vec2`, `vec2<u32>`, `Viewport`
    Named(String, Vec<Type>),
    /// `@vertex_index`, `@frag_coord`, `@frag_color`
    Builtin(String),
    /// `[T; N]`
    Array(Box<Type>, Box<Expr>),
    /// `()`
    Unit,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mark {
    pub name: String,
    pub args: Vec<Tok>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub marks: Vec<Mark>,
    pub name: String,
    pub ty: Type,
    /// `as unorm8x4`
    pub format: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub ty: Type,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResourceKind {
    Uniform,
    Texture,
    Sampler,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ItemKind {
    Import { path: Vec<String>, glob: bool },
    Const { name: String, ty: Type, value: Expr },
    Struct { name: String, uniform: bool, fields: Vec<Field> },
    Resource { kind: ResourceKind, name: String, ty: Option<Type> },
    Fn { name: String, params: Vec<Param>, ret: Option<Type>, body: Block },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub attrs: Vec<Attr>,
    pub public: bool,
    pub kind: ItemKind,
}

impl Item {
    pub fn attr(&self, name: &str) -> Option<&Attr> {
        self.attrs.iter().find(|a| a.name == name)
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct File {
    /// `shader name;`
    pub shader: Option<String>,
    /// `module a::b;`
    pub module: Option<Vec<String>>,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    /// Last expression without `;`, the value of the block.
    pub tail: Option<Box<Expr>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Let { mutable: bool, name: String, ty: Option<Type>, init: Option<Expr> },
    Expr(Expr),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Mul,
    Div,
    Rem,
    Add,
    Sub,
    Shl,
    Shr,
    BitAnd,
    BitXor,
    BitOr,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
}

impl BinOp {
    pub fn text(self) -> &'static str {
        match self {
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Shl => "<<",
            BinOp::Shr => ">>",
            BinOp::BitAnd => "&",
            BinOp::BitXor => "^",
            BinOp::BitOr => "|",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Gt => ">",
            BinOp::Le => "<=",
            BinOp::Ge => ">=",
            BinOp::And => "&&",
            BinOp::Or => "||",
        }
    }

    /// Higher binds tighter. Matches the table in the syntax draft.
    pub fn prec(self) -> u8 {
        match self {
            BinOp::Mul | BinOp::Div | BinOp::Rem => 10,
            BinOp::Add | BinOp::Sub => 9,
            BinOp::Shl | BinOp::Shr => 8,
            BinOp::BitAnd => 7,
            BinOp::BitXor => 6,
            BinOp::BitOr => 5,
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => 4,
            BinOp::And => 3,
            BinOp::Or => 2,
        }
    }

    pub fn is_comparison(self) -> bool {
        matches!(self, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge)
    }

    pub fn is_bitwise(self) -> bool {
        matches!(self, BinOp::BitAnd | BinOp::BitXor | BinOp::BitOr | BinOp::Shl | BinOp::Shr)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
    BitNot,
}

impl UnOp {
    pub fn text(self) -> &'static str {
        match self {
            UnOp::Neg => "-",
            UnOp::Not => "!",
            UnOp::BitNot => "~",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pat {
    Wild,
    Value(Expr),
    Range { lo: Expr, hi: Expr, inclusive: bool },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Arm {
    pub pats: Vec<Pat>,
    pub body: Expr,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// Integer literal text, suffix included.
    Int(String),
    /// Float literal text.
    Float(String),
    Bool(bool),
    Path(String),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// `a = b`, `a += b` (op is the binary operator of the compound form).
    Assign { op: Option<BinOp>, target: Box<Expr>, value: Box<Expr> },
    Call { name: String, generics: Vec<Type>, args: Vec<Expr> },
    Method { recv: Box<Expr>, name: String, args: Vec<Expr> },
    Field(Box<Expr>, String),
    Index(Box<Expr>, Box<Expr>),
    Cast(Box<Expr>, Type),
    StructLit { name: String, fields: Vec<(String, Expr)>, base: Option<Box<Expr>> },
    ArrayLit(Vec<Expr>),
    Block(Block),
    If { select: bool, cond: Box<Expr>, then: Block, els: Option<Box<Expr>> },
    Match { scrutinee: Box<Expr>, arms: Vec<Arm> },
    Loop(Block),
    While { cond: Box<Expr>, body: Block },
    For { var: String, lo: Box<Expr>, hi: Box<Expr>, inclusive: bool, body: Block },
    Return(Option<Box<Expr>>),
    Break(Option<Box<Expr>>),
    Continue,
    Discard,
}

impl Expr {
    /// `if`, `match`, `loop`, `while`, `for` and plain blocks. These end a statement without `;`.
    pub fn is_block_like(&self) -> bool {
        matches!(
            self,
            Expr::Block(_) | Expr::If { .. } | Expr::Match { .. } | Expr::Loop(_) | Expr::While { .. } | Expr::For { .. }
        )
    }
}

/// Pre-order walk over an expression and everything nested in it (blocks, statements, arms).
pub fn walk_expr(e: &Expr, f: &mut dyn FnMut(&Expr)) {
    f(e);
    match e {
        Expr::Int(_) | Expr::Float(_) | Expr::Bool(_) | Expr::Path(_) | Expr::Continue | Expr::Discard => {}
        Expr::Unary(_, x) | Expr::Field(x, _) | Expr::Cast(x, _) => walk_expr(x, f),
        Expr::Binary(_, a, b) | Expr::Index(a, b) => {
            walk_expr(a, f);
            walk_expr(b, f);
        }
        Expr::Assign { target, value, .. } => {
            walk_expr(target, f);
            walk_expr(value, f);
        }
        Expr::Call { args, .. } => args.iter().for_each(|a| walk_expr(a, f)),
        Expr::Method { recv, args, .. } => {
            walk_expr(recv, f);
            args.iter().for_each(|a| walk_expr(a, f));
        }
        Expr::StructLit { fields, base, .. } => {
            fields.iter().for_each(|(_, v)| walk_expr(v, f));
            if let Some(b) = base {
                walk_expr(b, f);
            }
        }
        Expr::ArrayLit(items) => items.iter().for_each(|a| walk_expr(a, f)),
        Expr::Block(b) | Expr::Loop(b) => walk_block(b, f),
        Expr::If { cond, then, els, .. } => {
            walk_expr(cond, f);
            walk_block(then, f);
            if let Some(e) = els {
                walk_expr(e, f);
            }
        }
        Expr::Match { scrutinee, arms } => {
            walk_expr(scrutinee, f);
            for a in arms {
                for p in &a.pats {
                    match p {
                        Pat::Wild => {}
                        Pat::Value(v) => walk_expr(v, f),
                        Pat::Range { lo, hi, .. } => {
                            walk_expr(lo, f);
                            walk_expr(hi, f);
                        }
                    }
                }
                walk_expr(&a.body, f);
            }
        }
        Expr::While { cond, body } => {
            walk_expr(cond, f);
            walk_block(body, f);
        }
        Expr::For { lo, hi, body, .. } => {
            walk_expr(lo, f);
            walk_expr(hi, f);
            walk_block(body, f);
        }
        Expr::Return(v) | Expr::Break(v) => {
            if let Some(v) = v {
                walk_expr(v, f);
            }
        }
    }
}

pub fn walk_block(b: &Block, f: &mut dyn FnMut(&Expr)) {
    for s in &b.stmts {
        match s {
            Stmt::Let { init, .. } => {
                if let Some(e) = init {
                    walk_expr(e, f);
                }
            }
            Stmt::Expr(e) => walk_expr(e, f),
        }
    }
    if let Some(t) = &b.tail {
        walk_expr(t, f);
    }
}
