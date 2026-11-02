use crate::ast::*;
use crate::error::{Error, Result};
use crate::lexer::{Tok, Token, lex};

pub fn parse(src: &str) -> Result<File> {
    let toks = lex(src)?;
    Parser { toks, pos: 0 }.file()
}

struct Parser {
    toks: Vec<Token>,
    pos: usize,
}

/// Names that take `<T>` generics in expression position, so `vec2<f32>(x)` is not read as a comparison.
fn takes_generics(name: &str) -> bool {
    matches!(name, "vec2" | "vec3" | "vec4" | "mat2" | "mat3" | "mat4" | "bitcast")
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }

    fn peek_at(&self, n: usize) -> &Tok {
        &self.toks[(self.pos + n).min(self.toks.len() - 1)].tok
    }

    fn err<T>(&self, msg: impl Into<String>) -> Result<T> {
        let t = &self.toks[self.pos];
        Err(Error::spanned(t.line, t.start, t.end, msg))
    }

    fn bump(&mut self) -> Tok {
        let t = self.toks[self.pos].tok.clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn is_p(&self, p: &str) -> bool {
        matches!(self.peek(), Tok::Punct(q) if *q == p)
    }

    fn is_kw(&self, k: &str) -> bool {
        matches!(self.peek(), Tok::Ident(s) if s == k)
    }

    fn eat_p(&mut self, p: &str) -> bool {
        if self.is_p(p) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn eat_kw(&mut self, k: &str) -> bool {
        if self.is_kw(k) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect_p(&mut self, p: &str) -> Result<()> {
        if self.eat_p(p) {
            Ok(())
        } else {
            self.err(format!("expected `{p}`, found {}", self.describe()))
        }
    }

    fn expect_kw(&mut self, k: &str) -> Result<()> {
        if self.eat_kw(k) {
            Ok(())
        } else {
            self.err(format!("expected `{k}`, found {}", self.describe()))
        }
    }

    fn ident(&mut self) -> Result<String> {
        match self.peek().clone() {
            Tok::Ident(s) => {
                self.bump();
                Ok(s)
            }
            _ => self.err(format!("expected identifier, found {}", self.describe())),
        }
    }

    fn describe(&self) -> String {
        match self.peek() {
            Tok::Ident(s) => format!("`{s}`"),
            Tok::Int(s) | Tok::Float(s) => format!("`{s}`"),
            Tok::Punct(p) => format!("`{p}`"),
            Tok::Eof => "end of file".into(),
        }
    }

    // ---- items ----

    fn file(mut self) -> Result<File> {
        let mut file = File::default();
        if self.is_kw("shader") && matches!(self.peek_at(1), Tok::Ident(_)) && matches!(self.peek_at(2), Tok::Punct(";")) {
            self.bump();
            file.shader = Some(self.ident()?);
            self.expect_p(";")?;
        } else if self.is_kw("module") {
            self.bump();
            file.module = Some(self.path()?);
            self.expect_p(";")?;
        }
        while *self.peek() != Tok::Eof {
            file.items.push(self.item()?);
        }
        Ok(file)
    }

    fn path(&mut self) -> Result<Vec<String>> {
        let mut p = vec![self.ident()?];
        while self.is_p("::") && matches!(self.peek_at(1), Tok::Ident(_)) {
            self.bump();
            p.push(self.ident()?);
        }
        Ok(p)
    }

    fn balanced_args(&mut self) -> Result<Vec<Tok>> {
        let mut out = Vec::new();
        if !self.eat_p("(") {
            return Ok(out);
        }
        let mut depth = 1;
        loop {
            match self.peek() {
                Tok::Eof => return self.err("unterminated `(`"),
                Tok::Punct("(") => depth += 1,
                Tok::Punct(")") => {
                    depth -= 1;
                    if depth == 0 {
                        self.bump();
                        return Ok(out);
                    }
                }
                _ => {}
            }
            out.push(self.bump());
        }
    }

    fn attr(&mut self) -> Result<Attr> {
        self.expect_p("#")?;
        self.expect_p("[")?;
        let name = self.path()?.join("::");
        let args = self.balanced_args()?;
        self.expect_p("]")?;
        Ok(Attr { name, args })
    }

    fn item(&mut self) -> Result<Item> {
        let mut attrs = Vec::new();
        while self.is_p("#") {
            attrs.push(self.attr()?);
        }
        let public = self.eat_kw("pub");
        let kind = match self.peek().clone() {
            Tok::Ident(k) => match k.as_str() {
                "import" => self.import()?,
                "const" => self.const_item()?,
                "struct" => self.struct_item(false)?,
                "uniform" => {
                    if matches!(self.peek_at(1), Tok::Ident(s) if s == "struct") {
                        self.bump();
                        self.struct_item(true)?
                    } else {
                        self.resource(ResourceKind::Uniform)?
                    }
                }
                "texture" => self.resource(ResourceKind::Texture)?,
                "sampler" => self.resource(ResourceKind::Sampler)?,
                "fn" => self.fn_item()?,
                _ => return self.err(format!("expected an item, found `{k}`")),
            },
            _ => return self.err(format!("expected an item, found {}", self.describe())),
        };
        Ok(Item { attrs, public, kind })
    }

    fn import(&mut self) -> Result<ItemKind> {
        self.expect_kw("import")?;
        let path = self.path()?;
        let glob = if self.is_p("::") && matches!(self.peek_at(1), Tok::Punct("*")) {
            self.bump();
            self.bump();
            true
        } else {
            false
        };
        self.expect_p(";")?;
        Ok(ItemKind::Import { path, glob })
    }

    fn const_item(&mut self) -> Result<ItemKind> {
        self.expect_kw("const")?;
        let name = self.ident()?;
        self.expect_p(":")?;
        let ty = self.ty()?;
        self.expect_p("=")?;
        let value = self.expr(false)?;
        self.expect_p(";")?;
        Ok(ItemKind::Const { name, ty, value })
    }

    fn struct_item(&mut self, uniform: bool) -> Result<ItemKind> {
        self.expect_kw("struct")?;
        let name = self.ident()?;
        self.expect_p("{")?;
        let mut fields = Vec::new();
        while !self.is_p("}") {
            let mut marks = Vec::new();
            while self.is_p("@") {
                self.bump();
                let name = self.ident()?;
                let args = self.balanced_args()?;
                marks.push(Mark { name, args });
            }
            let fname = self.ident()?;
            self.expect_p(":")?;
            let ty = self.ty()?;
            let format = if self.eat_kw("as") { Some(self.ident()?) } else { None };
            fields.push(Field { marks, name: fname, ty, format });
            if !self.eat_p(",") {
                break;
            }
        }
        self.expect_p("}")?;
        Ok(ItemKind::Struct { name, uniform, fields })
    }

    fn resource(&mut self, kind: ResourceKind) -> Result<ItemKind> {
        self.bump();
        let name = self.ident()?;
        let ty = if kind == ResourceKind::Sampler && !self.is_p(":") {
            None
        } else {
            self.expect_p(":")?;
            Some(self.ty()?)
        };
        self.expect_p(";")?;
        Ok(ItemKind::Resource { kind, name, ty })
    }

    fn fn_item(&mut self) -> Result<ItemKind> {
        self.expect_kw("fn")?;
        let name = self.ident()?;
        self.expect_p("(")?;
        let mut params = Vec::new();
        while !self.is_p(")") {
            let pname = self.ident()?;
            self.expect_p(":")?;
            let ty = self.ty()?;
            params.push(Param { name: pname, ty });
            if !self.eat_p(",") {
                break;
            }
        }
        self.expect_p(")")?;
        let ret = if self.eat_p("->") { Some(self.ty()?) } else { None };
        let body = self.block()?;
        Ok(ItemKind::Fn { name, params, ret, body })
    }

    // ---- types ----

    fn ty(&mut self) -> Result<Type> {
        if self.eat_p("@") {
            return Ok(Type::Builtin(self.ident()?));
        }
        if self.eat_p("(") {
            self.expect_p(")")?;
            return Ok(Type::Unit);
        }
        if self.eat_p("[") {
            let elem = self.ty()?;
            self.expect_p(";")?;
            let n = self.expr(false)?;
            self.expect_p("]")?;
            return Ok(Type::Array(Box::new(elem), Box::new(n)));
        }
        let name = self.ident()?;
        let mut args = Vec::new();
        if self.eat_p("<") {
            loop {
                args.push(self.ty()?);
                if !self.eat_p(",") {
                    break;
                }
            }
            self.expect_p(">")?;
        }
        Ok(Type::Named(name, args))
    }

    // ---- blocks and statements ----

    fn block(&mut self) -> Result<Block> {
        self.expect_p("{")?;
        let mut stmts = Vec::new();
        let mut tail = None;
        loop {
            if self.eat_p("}") {
                break;
            }
            if self.is_kw("let") {
                stmts.push(self.let_stmt()?);
                continue;
            }
            let e = self.expr(false)?;
            if self.eat_p(";") {
                stmts.push(Stmt::Expr(e));
            } else if self.eat_p("}") {
                tail = Some(Box::new(e));
                break;
            } else if e.is_block_like() {
                stmts.push(Stmt::Expr(e));
            } else {
                return self.err(format!("expected `;` or `}}`, found {}", self.describe()));
            }
        }
        Ok(Block { stmts, tail })
    }

    fn let_stmt(&mut self) -> Result<Stmt> {
        self.expect_kw("let")?;
        let mutable = self.eat_kw("mut");
        let name = self.ident()?;
        let ty = if self.eat_p(":") { Some(self.ty()?) } else { None };
        let init = if self.eat_p("=") { Some(self.expr(false)?) } else { None };
        self.expect_p(";")?;
        Ok(Stmt::Let { mutable, name, ty, init })
    }

    // ---- expressions ----

    /// `no_struct`: a `Name {` is not a struct literal (in `if`, `match`, `while`, `for` headers).
    fn expr(&mut self, no_struct: bool) -> Result<Expr> {
        let lhs = self.binary(0, no_struct)?;
        let op = match self.peek() {
            Tok::Punct("=") => Some(None),
            Tok::Punct("+=") => Some(Some(BinOp::Add)),
            Tok::Punct("-=") => Some(Some(BinOp::Sub)),
            Tok::Punct("*=") => Some(Some(BinOp::Mul)),
            Tok::Punct("/=") => Some(Some(BinOp::Div)),
            Tok::Punct("%=") => Some(Some(BinOp::Rem)),
            Tok::Punct("&=") => Some(Some(BinOp::BitAnd)),
            Tok::Punct("|=") => Some(Some(BinOp::BitOr)),
            Tok::Punct("^=") => Some(Some(BinOp::BitXor)),
            Tok::Punct("<<=") => Some(Some(BinOp::Shl)),
            Tok::Punct(">>=") => Some(Some(BinOp::Shr)),
            _ => None,
        };
        if let Some(op) = op {
            self.bump();
            let value = self.expr(no_struct)?;
            return Ok(Expr::Assign { op, target: Box::new(lhs), value: Box::new(value) });
        }
        Ok(lhs)
    }

    fn bin_op(&self) -> Option<BinOp> {
        Some(match self.peek() {
            Tok::Punct("*") => BinOp::Mul,
            Tok::Punct("/") => BinOp::Div,
            Tok::Punct("%") => BinOp::Rem,
            Tok::Punct("+") => BinOp::Add,
            Tok::Punct("-") => BinOp::Sub,
            Tok::Punct("<<") => BinOp::Shl,
            Tok::Punct(">>") => BinOp::Shr,
            Tok::Punct("&") => BinOp::BitAnd,
            Tok::Punct("^") => BinOp::BitXor,
            Tok::Punct("|") => BinOp::BitOr,
            Tok::Punct("==") => BinOp::Eq,
            Tok::Punct("!=") => BinOp::Ne,
            Tok::Punct("<") => BinOp::Lt,
            Tok::Punct(">") => BinOp::Gt,
            Tok::Punct("<=") => BinOp::Le,
            Tok::Punct(">=") => BinOp::Ge,
            Tok::Punct("&&") => BinOp::And,
            Tok::Punct("||") => BinOp::Or,
            _ => return None,
        })
    }

    /// Precedence climbing. Only operators with `prec > min` are consumed.
    fn binary(&mut self, min: u8, no_struct: bool) -> Result<Expr> {
        let mut lhs = self.cast(no_struct)?;
        while let Some(op) = self.bin_op() {
            if op.prec() <= min {
                break;
            }
            self.bump();
            let rhs = self.binary(op.prec(), no_struct)?;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn cast(&mut self, no_struct: bool) -> Result<Expr> {
        let mut e = self.unary(no_struct)?;
        while self.is_kw("as") {
            self.bump();
            let ty = self.ty()?;
            e = Expr::Cast(Box::new(e), ty);
        }
        Ok(e)
    }

    fn unary(&mut self, no_struct: bool) -> Result<Expr> {
        let op = match self.peek() {
            Tok::Punct("-") => Some(UnOp::Neg),
            Tok::Punct("!") => Some(UnOp::Not),
            Tok::Punct("~") => Some(UnOp::BitNot),
            _ => None,
        };
        if let Some(op) = op {
            self.bump();
            let e = self.unary(no_struct)?;
            return Ok(Expr::Unary(op, Box::new(e)));
        }
        self.postfix(no_struct)
    }

    fn postfix(&mut self, no_struct: bool) -> Result<Expr> {
        let mut e = self.primary(no_struct)?;
        loop {
            if self.is_p(".") {
                self.bump();
                let name = match self.peek().clone() {
                    Tok::Ident(s) => {
                        self.bump();
                        s
                    }
                    _ => return self.err("expected a field name after `.`"),
                };
                if self.is_p("(") {
                    let args = self.call_args()?;
                    e = Expr::Method { recv: Box::new(e), name, args };
                } else {
                    e = Expr::Field(Box::new(e), name);
                }
            } else if self.is_p("[") {
                self.bump();
                let idx = self.expr(false)?;
                self.expect_p("]")?;
                e = Expr::Index(Box::new(e), Box::new(idx));
            } else {
                return Ok(e);
            }
        }
    }

    fn call_args(&mut self) -> Result<Vec<Expr>> {
        self.expect_p("(")?;
        let mut args = Vec::new();
        while !self.is_p(")") {
            args.push(self.expr(false)?);
            if !self.eat_p(",") {
                break;
            }
        }
        self.expect_p(")")?;
        Ok(args)
    }

    fn primary(&mut self, no_struct: bool) -> Result<Expr> {
        match self.peek().clone() {
            Tok::Int(s) => {
                self.bump();
                Ok(Expr::Int(s))
            }
            Tok::Float(s) => {
                self.bump();
                Ok(Expr::Float(s))
            }
            Tok::Punct("(") => {
                self.bump();
                let e = self.expr(false)?;
                self.expect_p(")")?;
                Ok(e)
            }
            Tok::Punct("[") => {
                self.bump();
                let mut items = Vec::new();
                while !self.is_p("]") {
                    items.push(self.expr(false)?);
                    if !self.eat_p(",") {
                        break;
                    }
                }
                self.expect_p("]")?;
                Ok(Expr::ArrayLit(items))
            }
            Tok::Punct("{") => Ok(Expr::Block(self.block()?)),
            Tok::Punct("#") => {
                let attr = self.attr()?;
                let e = self.primary(no_struct)?;
                match (attr.name.as_str(), e) {
                    ("select", Expr::If { cond, then, els, .. }) => Ok(Expr::If { select: true, cond, then, els }),
                    ("branch", e @ Expr::If { .. }) => Ok(e),
                    (name, _) => self.err(format!("attribute `#[{name}]` is not valid on this expression")),
                }
            }
            Tok::Ident(name) => self.ident_expr(name, no_struct),
            _ => self.err(format!("expected an expression, found {}", self.describe())),
        }
    }

    fn ident_expr(&mut self, name: String, no_struct: bool) -> Result<Expr> {
        match name.as_str() {
            "true" => {
                self.bump();
                Ok(Expr::Bool(true))
            }
            "false" => {
                self.bump();
                Ok(Expr::Bool(false))
            }
            "if" => self.if_expr(),
            "match" => self.match_expr(),
            "loop" => {
                self.bump();
                Ok(Expr::Loop(self.block()?))
            }
            "while" => {
                self.bump();
                let cond = self.expr(true)?;
                let body = self.block()?;
                Ok(Expr::While { cond: Box::new(cond), body })
            }
            "for" => self.for_expr(),
            "return" => {
                self.bump();
                Ok(Expr::Return(self.opt_value()?))
            }
            "break" => {
                self.bump();
                Ok(Expr::Break(self.opt_value()?))
            }
            "continue" => {
                self.bump();
                Ok(Expr::Continue)
            }
            "discard" => {
                self.bump();
                Ok(Expr::Discard)
            }
            _ => {
                self.bump();
                let mut generics = Vec::new();
                if takes_generics(&name) && self.is_p("<") {
                    self.bump();
                    loop {
                        generics.push(self.ty()?);
                        if !self.eat_p(",") {
                            break;
                        }
                    }
                    self.expect_p(">")?;
                    if !self.is_p("(") {
                        return self.err("expected `(` after generic arguments");
                    }
                }
                if self.is_p("(") {
                    let args = self.call_args()?;
                    return Ok(Expr::Call { name, generics, args });
                }
                if !no_struct && self.is_p("{") {
                    return self.struct_lit(name);
                }
                Ok(Expr::Path(name))
            }
        }
    }

    fn opt_value(&mut self) -> Result<Option<Box<Expr>>> {
        if self.is_p(";") || self.is_p("}") || self.is_p(")") || self.is_p(",") {
            Ok(None)
        } else {
            Ok(Some(Box::new(self.expr(false)?)))
        }
    }

    fn struct_lit(&mut self, name: String) -> Result<Expr> {
        self.expect_p("{")?;
        let mut fields = Vec::new();
        let mut base = None;
        while !self.is_p("}") {
            if self.eat_p("..") {
                base = Some(Box::new(self.expr(false)?));
                break;
            }
            let f = self.ident()?;
            self.expect_p(":")?;
            fields.push((f, self.expr(false)?));
            if !self.eat_p(",") {
                break;
            }
        }
        self.expect_p("}")?;
        Ok(Expr::StructLit { name, fields, base })
    }

    fn if_expr(&mut self) -> Result<Expr> {
        self.expect_kw("if")?;
        let cond = self.expr(true)?;
        let then = self.block()?;
        let els = if self.eat_kw("else") {
            if self.is_kw("if") {
                Some(Box::new(self.if_expr()?))
            } else {
                Some(Box::new(Expr::Block(self.block()?)))
            }
        } else {
            None
        };
        Ok(Expr::If { select: false, cond: Box::new(cond), then, els })
    }

    fn match_expr(&mut self) -> Result<Expr> {
        self.expect_kw("match")?;
        let scrutinee = self.expr(true)?;
        self.expect_p("{")?;
        let mut arms = Vec::new();
        while !self.is_p("}") {
            let mut pats = vec![self.pat()?];
            while self.eat_p("|") {
                pats.push(self.pat()?);
            }
            self.expect_p("=>")?;
            let body = self.expr(false)?;
            let had_comma = self.eat_p(",");
            if !had_comma && !body.is_block_like() && !self.is_p("}") {
                return self.err("expected `,` after a match arm");
            }
            arms.push(Arm { pats, body });
        }
        self.expect_p("}")?;
        Ok(Expr::Match { scrutinee: Box::new(scrutinee), arms })
    }

    fn pat(&mut self) -> Result<Pat> {
        if self.is_kw("_") {
            self.bump();
            return Ok(Pat::Wild);
        }
        // Above `|` so alternatives are not swallowed.
        let lo = self.binary(5, true)?;
        let inclusive = if self.eat_p("..=") {
            true
        } else if self.eat_p("..") {
            false
        } else {
            return Ok(Pat::Value(lo));
        };
        let hi = self.binary(5, true)?;
        Ok(Pat::Range { lo, hi, inclusive })
    }

    fn for_expr(&mut self) -> Result<Expr> {
        self.expect_kw("for")?;
        let var = self.ident()?;
        self.expect_kw("in")?;
        let lo = self.binary(0, true)?;
        let inclusive = if self.eat_p("..=") {
            true
        } else {
            self.expect_p("..")?;
            false
        };
        let hi = self.binary(0, true)?;
        let body = self.block()?;
        Ok(Expr::For { var, lo: Box::new(lo), hi: Box::new(hi), inclusive, body })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_fn_body(src: &str) -> Block {
        let f = parse(&format!("fn f() -> f32 {{ {src} }}")).unwrap();
        match &f.items[0].kind {
            ItemKind::Fn { body, .. } => body.clone(),
            _ => panic!(),
        }
    }

    #[test]
    fn tail_vs_statement() {
        let b = parse_fn_body("let x = 1.0; x");
        assert_eq!(b.stmts.len(), 1);
        assert!(b.tail.is_some());
        let b = parse_fn_body("let x = 1.0; x;");
        assert!(b.tail.is_none());
        assert_eq!(b.stmts.len(), 2);
    }

    #[test]
    fn if_as_tail_and_statement() {
        let b = parse_fn_body("if a { return 1.0; } 2.0");
        assert_eq!(b.stmts.len(), 1);
        assert!(b.tail.is_some());
        let b = parse_fn_body("if a { 1.0 } else { 2.0 }");
        assert!(matches!(b.tail.as_deref(), Some(Expr::If { .. })));
    }

    #[test]
    fn precedence() {
        let b = parse_fn_body("a + b * c");
        match b.tail.as_deref() {
            Some(Expr::Binary(BinOp::Add, _, r)) => assert!(matches!(**r, Expr::Binary(BinOp::Mul, _, _))),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn struct_literal_not_in_header() {
        let b = parse_fn_body("if x { 1.0 } else { 2.0 }");
        assert!(b.tail.is_some());
        let b = parse_fn_body("P { a: 1.0, ..base }");
        assert!(matches!(b.tail.as_deref(), Some(Expr::StructLit { base: Some(_), .. })));
    }

    #[test]
    fn generics_and_match() {
        parse_fn_body("let p = vec2<i32>(a.xy); match n { 0u => 1.0, 1u | 2u => 2.0, 3u..=7u => 3.0, _ => 4.0 }");
    }

    #[test]
    fn items() {
        let f = parse(
            "shader t;\nimport aimer::color::*;\n#[bind(0, 0)]\nuniform v: V;\n#[varying]\nstruct S { @position p: vec4, @flat k: u32 }\nconst C: [vec2; 2] = [vec2(0.0), vec2(1.0)];",
        )
        .unwrap();
        assert_eq!(f.shader.as_deref(), Some("t"));
        assert_eq!(f.items.len(), 4);
    }
}
