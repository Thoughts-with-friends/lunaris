//! Lexer and parser for the synthesizable Verilog / SystemVerilog subset the
//! built-in simulator runs (see `super` for the list).
//!
//! ```text
//!  source --lex--> tokens --parse--> Module { ports, items }
//!  items: decl (wire/reg/logic/integer, memories), localparam/parameter,
//!         assign, always @(posedge clk) / @(*) / always_ff / always_comb,
//!         initial
//! ```

use super::VerilogError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tok {
    Ident(String),
    /// Raw literal text (`32'hFF`, `12`, `'b1`).
    Num(String),
    Sym(&'static str),
}

const SYMS: [&str; 44] = [
    "<<<", ">>>", "===", "!==", "<=", ">=", "==", "!=", "&&", "||", "<<", ">>", "~&", "~|", "~^",
    "^~", "+:", "-:", "(", ")", "[", "]", "{", "}", ";", ",", ":", "?", "=", "+", "-", "*", "/",
    "%", "&", "|", "^", "~", "!", "<", ">", "@", "#", ".",
];

/// Tokens with their 1-based line numbers.
pub fn lex(src: &str) -> Result<Vec<(Tok, usize)>, VerilogError> {
    let bytes = src.as_bytes();
    let (mut i, mut line) = (0, 1);
    let mut out = Vec::new();
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b'\n' => {
                line += 1;
                i += 1;
            }
            _ if c.is_ascii_whitespace() => i += 1,
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    line += usize::from(bytes[i] == b'\n');
                    i += 1;
                }
                i += 2;
            }
            // `default_nettype, `timescale ...: ignored to the end of the line.
            b'`' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            _ if c.is_ascii_alphabetic() || c == b'_' || c == b'$' => {
                let start = i;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'$')
                {
                    i += 1;
                }
                out.push((Tok::Ident(src[start..i].to_owned()), line));
            }
            _ if c.is_ascii_digit() || c == b'\'' => {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
                    i += 1;
                }
                if bytes.get(i) == Some(&b'\'') {
                    i += 1;
                    if matches!(bytes.get(i), Some(b's' | b'S')) {
                        i += 1;
                    }
                    i += 1; // base letter
                    while i < bytes.len()
                        && (bytes[i].is_ascii_alphanumeric()
                            || bytes[i] == b'_'
                            || bytes[i] == b'?')
                    {
                        i += 1;
                    }
                }
                out.push((Tok::Num(src[start..i].to_owned()), line));
            }
            _ => {
                let sym = SYMS.iter().find(|s| src[i..].starts_with(*s)).ok_or_else(|| {
                    VerilogError(format!("line {line}: unexpected character `{}`", c as char))
                })?;
                out.push((Tok::Sym(sym), line));
                i += sym.len();
            }
        }
    }
    Ok(out)
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
    AShl,
    AShr,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    And,
    Xor,
    Xnor,
    Or,
    LAnd,
    LOr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
    LNot,
    RedAnd,
    RedOr,
    RedXor,
    RedNand,
    RedNor,
    RedXnor,
    Plus,
}

#[derive(Debug, Clone)]
pub enum Expr {
    Num(String),
    Ident(String),
    /// `name[i]` (bit or memory word), `name[i][m:l]`.
    Index(String, Box<Self>),
    Part(String, Box<Self>, Box<Self>),
    /// `name[base +: width]` / `-:`.
    IndexedPart(String, Box<Self>, Box<Self>, bool),
    Unary(UnOp, Box<Self>),
    Binary(BinOp, Box<Self>, Box<Self>),
    Ternary(Box<Self>, Box<Self>, Box<Self>),
    Concat(Vec<Self>),
    Repl(Box<Self>, Vec<Self>),
    Call(String, Vec<Self>),
}

#[derive(Debug, Clone)]
pub enum LValue {
    Name(String),
    Index(String, Expr),
    Part(String, Expr, Expr),
}

#[derive(Debug, Clone)]
pub enum Stmt {
    Block(Vec<Self>),
    If(Expr, Box<Self>, Option<Box<Self>>),
    Case(Expr, Vec<(Vec<Expr>, Self)>, Option<Box<Self>>),
    Assign(LValue, Expr, bool),
    Empty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Input,
    Output,
}

#[derive(Debug, Clone)]
pub struct Decl {
    pub name: String,
    pub range: Option<(Expr, Expr)>,
    /// Memory: `name [a:b]`.
    pub array: Option<(Expr, Expr)>,
    pub init: Option<Expr>,
    pub is_reg: bool,
    pub signed: bool,
    pub dir: Option<Dir>,
}

#[derive(Debug, Clone)]
pub enum Item {
    Decl(Decl),
    Param(String, Expr),
    Assign(LValue, Expr),
    /// `true`: clocked (posedge), `false`: combinational.
    Always(bool, Stmt),
    Initial(Stmt),
}

#[derive(Debug, Clone)]
pub struct Module {
    pub name: String,
    pub items: Vec<Item>,
}

struct Parser {
    toks: Vec<(Tok, usize)>,
    pos: usize,
}

type R<T> = Result<T, VerilogError>;

impl Parser {
    fn line(&self) -> usize {
        self.toks.get(self.pos).or_else(|| self.toks.last()).map_or(0, |t| t.1)
    }

    fn err<T>(&self, msg: impl AsRef<str>) -> R<T> {
        Err(VerilogError(format!("line {}: {}", self.line(), msg.as_ref())))
    }

    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|t| &t.0)
    }

    fn is_sym(&self, s: &str) -> bool {
        matches!(self.peek(), Some(Tok::Sym(x)) if *x == s)
    }

    fn is_kw(&self, k: &str) -> bool {
        matches!(self.peek(), Some(Tok::Ident(x)) if x == k)
    }

    fn eat_sym(&mut self, s: &str) -> bool {
        let hit = self.is_sym(s);
        self.pos += usize::from(hit);
        hit
    }

    fn eat_kw(&mut self, k: &str) -> bool {
        let hit = self.is_kw(k);
        self.pos += usize::from(hit);
        hit
    }

    fn expect_sym(&mut self, s: &str) -> R<()> {
        if self.eat_sym(s) {
            Ok(())
        } else {
            self.err(format!("expected `{s}`, found {:?}", self.peek()))
        }
    }

    fn ident(&mut self) -> R<String> {
        match self.peek().cloned() {
            Some(Tok::Ident(s)) => {
                self.pos += 1;
                Ok(s)
            }
            other => self.err(format!("expected a name, found {other:?}")),
        }
    }

    fn range(&mut self) -> R<Option<(Expr, Expr)>> {
        if !self.eat_sym("[") {
            return Ok(None);
        }
        let msb = self.expr()?;
        self.expect_sym(":")?;
        let lsb = self.expr()?;
        self.expect_sym("]")?;
        Ok(Some((msb, lsb)))
    }

    fn module(&mut self) -> R<Module> {
        while !self.eat_kw("module") {
            if self.peek().is_none() {
                return self.err("no `module` found");
            }
            self.pos += 1;
        }
        let name = self.ident()?;
        let mut items = Vec::new();
        if self.eat_sym("#") {
            self.expect_sym("(")?;
            while !self.eat_sym(")") {
                self.eat_kw("parameter");
                self.range()?;
                let p = self.ident()?;
                self.expect_sym("=")?;
                items.push(Item::Param(p, self.expr()?));
                self.eat_sym(",");
            }
        }
        self.expect_sym("(")?;
        // ANSI (`input wire [3:0] a, b`) or bare names declared in the body.
        let mut dir = None;
        let mut kind = (false, false, None);
        while !self.eat_sym(")") {
            if self.eat_kw("input") {
                dir = Some(Dir::Input);
                kind = self.net_kind()?;
            } else if self.eat_kw("output") {
                dir = Some(Dir::Output);
                kind = self.net_kind()?;
            }
            let name = self.ident()?;
            if dir.is_some() {
                items.push(Item::Decl(Decl {
                    name,
                    range: kind.2.clone(),
                    array: None,
                    init: None,
                    is_reg: kind.0,
                    signed: kind.1,
                    dir,
                }));
            }
            self.eat_sym(",");
        }
        self.expect_sym(";")?;
        while !self.eat_kw("endmodule") {
            if self.peek().is_none() {
                return self.err("missing `endmodule`");
            }
            self.item(&mut items)?;
        }
        Ok(Module { name, items })
    }

    /// `[wire|reg|logic] [signed] [range]` -> (is_reg, signed, range).
    #[expect(clippy::type_complexity, reason = "(is_reg, signed, range), used twice")]
    fn net_kind(&mut self) -> R<(bool, bool, Option<(Expr, Expr)>)> {
        let is_reg = self.eat_kw("reg") || self.eat_kw("logic");
        self.eat_kw("wire");
        let signed = self.eat_kw("signed");
        Ok((is_reg, signed, self.range()?))
    }

    fn item(&mut self, items: &mut Vec<Item>) -> R<()> {
        let Some(Tok::Ident(kw)) = self.peek().cloned() else {
            return if self.eat_sym(";") {
                Ok(())
            } else {
                self.err(format!("unexpected {:?}", self.peek()))
            };
        };
        self.pos += 1;
        match kw.as_str() {
            "localparam" | "parameter" => {
                self.eat_kw("signed");
                self.range()?;
                loop {
                    let name = self.ident()?;
                    self.expect_sym("=")?;
                    items.push(Item::Param(name, self.expr()?));
                    if !self.eat_sym(",") {
                        break;
                    }
                }
                self.expect_sym(";")
            }
            "input" | "output" | "wire" | "reg" | "logic" | "integer" => {
                let dir = match kw.as_str() {
                    "input" => Some(Dir::Input),
                    "output" => Some(Dir::Output),
                    _ => None,
                };
                let (mut is_reg, signed, mut range) = if dir.is_some() {
                    self.net_kind()?
                } else {
                    let signed = self.eat_kw("signed");
                    (kw != "wire", signed, self.range()?)
                };
                if kw == "integer" {
                    is_reg = true;
                    range = Some((Expr::Num("31".into()), Expr::Num("0".into())));
                }
                loop {
                    let name = self.ident()?;
                    let array = self.range()?;
                    let init = if self.eat_sym("=") { Some(self.expr()?) } else { None };
                    // A body `input`/`output` completes a bare port name.
                    if let Some(Item::Decl(d)) =
                        items.iter_mut().find(|i| matches!(i, Item::Decl(d) if d.name == name))
                    {
                        d.dir = d.dir.or(dir);
                        d.is_reg |= is_reg;
                        d.range = d.range.take().or_else(|| range.clone());
                    } else {
                        items.push(Item::Decl(Decl {
                            name,
                            range: range.clone(),
                            array,
                            init,
                            is_reg,
                            signed: signed || kw == "integer",
                            dir,
                        }));
                    }
                    if !self.eat_sym(",") {
                        break;
                    }
                }
                self.expect_sym(";")
            }
            "assign" => {
                loop {
                    let lv = self.lvalue()?;
                    self.expect_sym("=")?;
                    items.push(Item::Assign(lv, self.expr()?));
                    if !self.eat_sym(",") {
                        break;
                    }
                }
                self.expect_sym(";")
            }
            "always" | "always_ff" | "always_comb" | "always_latch" => {
                let mut clocked = kw == "always_ff";
                if self.eat_sym("@") {
                    if self.eat_sym("*") {
                    } else {
                        self.expect_sym("(")?;
                        while !self.eat_sym(")") {
                            if self.eat_kw("posedge") || self.eat_kw("negedge") {
                                clocked = true;
                            }
                            self.pos += 1;
                        }
                    }
                }
                items.push(Item::Always(clocked, self.stmt()?));
                Ok(())
            }
            "initial" => {
                items.push(Item::Initial(self.stmt()?));
                Ok(())
            }
            "genvar" | "function" | "task" | "generate" | "for" => {
                self.err(format!("`{kw}` is not supported by the built-in simulator"))
            }
            _ => self.err(format!("unexpected `{kw}` (module instances are not supported)")),
        }
    }

    fn lvalue(&mut self) -> R<LValue> {
        let name = self.ident()?;
        if !self.eat_sym("[") {
            return Ok(LValue::Name(name));
        }
        let a = self.expr()?;
        if self.eat_sym(":") {
            let b = self.expr()?;
            self.expect_sym("]")?;
            return Ok(LValue::Part(name, a, b));
        }
        self.expect_sym("]")?;
        Ok(LValue::Index(name, a))
    }

    fn stmt(&mut self) -> R<Stmt> {
        if self.eat_sym(";") {
            return Ok(Stmt::Empty);
        }
        if self.eat_kw("begin") {
            if self.eat_sym(":") {
                self.ident()?;
            }
            let mut body = Vec::new();
            while !self.eat_kw("end") {
                if self.peek().is_none() {
                    return self.err("missing `end`");
                }
                body.push(self.stmt()?);
            }
            return Ok(Stmt::Block(body));
        }
        if self.eat_kw("if") {
            self.expect_sym("(")?;
            let cond = self.expr()?;
            self.expect_sym(")")?;
            let then = Box::new(self.stmt()?);
            let other = if self.eat_kw("else") { Some(Box::new(self.stmt()?)) } else { None };
            return Ok(Stmt::If(cond, then, other));
        }
        self.eat_kw("unique");
        self.eat_kw("priority");
        if self.eat_kw("case") || self.eat_kw("casez") || self.eat_kw("casex") {
            self.expect_sym("(")?;
            let sel = self.expr()?;
            self.expect_sym(")")?;
            let (mut arms, mut default) = (Vec::new(), None);
            while !self.eat_kw("endcase") {
                if self.eat_kw("default") {
                    self.eat_sym(":");
                    default = Some(Box::new(self.stmt()?));
                    continue;
                }
                let mut labels = vec![self.expr()?];
                while self.eat_sym(",") {
                    labels.push(self.expr()?);
                }
                self.expect_sym(":")?;
                arms.push((labels, self.stmt()?));
            }
            return Ok(Stmt::Case(sel, arms, default));
        }
        if matches!(self.peek(), Some(Tok::Ident(s)) if s.starts_with('$')) {
            // $display & co: skipped.
            while !self.eat_sym(";") {
                self.pos += 1;
            }
            return Ok(Stmt::Empty);
        }
        let lv = self.lvalue()?;
        let nonblocking = if self.eat_sym("<=") {
            true
        } else {
            self.expect_sym("=")?;
            false
        };
        let rhs = self.expr()?;
        self.expect_sym(";")?;
        Ok(Stmt::Assign(lv, rhs, nonblocking))
    }

    pub fn expr(&mut self) -> R<Expr> {
        let cond = self.binary(0)?;
        if self.eat_sym("?") {
            let a = self.expr()?;
            self.expect_sym(":")?;
            let b = self.expr()?;
            return Ok(Expr::Ternary(Box::new(cond), Box::new(a), Box::new(b)));
        }
        Ok(cond)
    }

    fn binop(&self, level: usize) -> Option<BinOp> {
        let Some(Tok::Sym(s)) = self.peek() else { return None };
        let op = match (level, *s) {
            (0, "||") => BinOp::LOr,
            (1, "&&") => BinOp::LAnd,
            (2, "|") => BinOp::Or,
            (3, "^") => BinOp::Xor,
            (3, "^~" | "~^") => BinOp::Xnor,
            (4, "&") => BinOp::And,
            (5, "==" | "===") => BinOp::Eq,
            (5, "!=" | "!==") => BinOp::Ne,
            (6, "<") => BinOp::Lt,
            (6, "<=") => BinOp::Le,
            (6, ">") => BinOp::Gt,
            (6, ">=") => BinOp::Ge,
            (7, "<<") => BinOp::Shl,
            (7, ">>") => BinOp::Shr,
            (7, "<<<") => BinOp::AShl,
            (7, ">>>") => BinOp::AShr,
            (8, "+") => BinOp::Add,
            (8, "-") => BinOp::Sub,
            (9, "*") => BinOp::Mul,
            (9, "/") => BinOp::Div,
            (9, "%") => BinOp::Rem,
            _ => return None,
        };
        Some(op)
    }

    fn binary(&mut self, level: usize) -> R<Expr> {
        if level == 10 {
            return self.unary();
        }
        let mut lhs = self.binary(level + 1)?;
        while let Some(op) = self.binop(level) {
            self.pos += 1;
            let rhs = self.binary(level + 1)?;
            lhs = Expr::Binary(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> R<Expr> {
        let op = match self.peek() {
            Some(Tok::Sym("-")) => UnOp::Neg,
            Some(Tok::Sym("+")) => UnOp::Plus,
            Some(Tok::Sym("~")) => UnOp::Not,
            Some(Tok::Sym("!")) => UnOp::LNot,
            Some(Tok::Sym("&")) => UnOp::RedAnd,
            Some(Tok::Sym("|")) => UnOp::RedOr,
            Some(Tok::Sym("^")) => UnOp::RedXor,
            Some(Tok::Sym("~&")) => UnOp::RedNand,
            Some(Tok::Sym("~|")) => UnOp::RedNor,
            Some(Tok::Sym("~^" | "^~")) => UnOp::RedXnor,
            _ => return self.primary(),
        };
        self.pos += 1;
        Ok(Expr::Unary(op, Box::new(self.unary()?)))
    }

    fn primary(&mut self) -> R<Expr> {
        match self.peek().cloned() {
            Some(Tok::Num(n)) => {
                self.pos += 1;
                Ok(Expr::Num(n))
            }
            Some(Tok::Sym("(")) => {
                self.pos += 1;
                let e = self.expr()?;
                self.expect_sym(")")?;
                Ok(e)
            }
            Some(Tok::Sym("{")) => {
                self.pos += 1;
                let first = self.expr()?;
                if self.eat_sym("{") {
                    // {n{a, b}}
                    let mut parts = vec![self.expr()?];
                    while self.eat_sym(",") {
                        parts.push(self.expr()?);
                    }
                    self.expect_sym("}")?;
                    self.expect_sym("}")?;
                    return Ok(Expr::Repl(Box::new(first), parts));
                }
                let mut parts = vec![first];
                while self.eat_sym(",") {
                    parts.push(self.expr()?);
                }
                self.expect_sym("}")?;
                Ok(Expr::Concat(parts))
            }
            Some(Tok::Ident(name)) => {
                self.pos += 1;
                if name.starts_with('$') {
                    let mut args = Vec::new();
                    if self.eat_sym("(") {
                        while !self.eat_sym(")") {
                            args.push(self.expr()?);
                            self.eat_sym(",");
                        }
                    }
                    return Ok(Expr::Call(name, args));
                }
                if !self.eat_sym("[") {
                    return Ok(Expr::Ident(name));
                }
                let a = self.expr()?;
                let e = if self.eat_sym(":") {
                    let b = self.expr()?;
                    Expr::Part(name, Box::new(a), Box::new(b))
                } else if self.eat_sym("+:") {
                    Expr::IndexedPart(name, Box::new(a), Box::new(self.expr()?), true)
                } else if self.eat_sym("-:") {
                    Expr::IndexedPart(name, Box::new(a), Box::new(self.expr()?), false)
                } else {
                    Expr::Index(name, Box::new(a))
                };
                self.expect_sym("]")?;
                Ok(e)
            }
            other => self.err(format!("expected an expression, found {other:?}")),
        }
    }
}

/// Parses the first module of `src`.
pub fn parse(src: &str) -> R<Module> {
    Parser { toks: lex(src)?, pos: 0 }.module()
}
