//! Tokens to syntax tree (spec: Language › Grammar).

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::ast::*;
use crate::diag::{Diag, Pos};
use crate::lexer::{Kw, Lexer, Stream, Tok, Token, P};

pub struct Parser<'s> {
    toks: Stream<'s>,
    /// The current token and the one after it.
    cur: Token<'s>,
    ahead: Token<'s>,
    pub warnings: Vec<Diag>,
}

type R<T> = Result<T, Diag>;

const META_KEYS: [&str; 7] = ["title", "author", "version", "desc", "id", "license", "spoilers"];

/// Parses a whole cart source.
pub fn parse<'s>(src: &'s str) -> Result<(Program<'s>, Vec<Diag>), Diag> {
    Lexer::new(src).check()?;
    let mut p = Parser::new(Lexer::new(src));
    let mut prog = Program {
        items: Vec::new(),
        meta: metadata(src, &mut p.warnings),
    };
    p.skip_nl();
    while !p.at_eof() {
        prog.items.push(p.item()?);
        if !p.at_eof() {
            p.end_of_line("a declaration")?;
        }
        p.skip_nl();
    }
    Ok((prog, p.warnings))
}

/// Parses a function or handler body again, from the position of its `{`
/// that [`parse`] recorded. `src` is the source given to [`parse`].
pub fn parse_body<'s>(src: &'s str, at: Pos) -> R<Block<'s>> {
    let mut p = Parser::new(Lexer::at(src, at));
    p.block()
}

/// `// @key value` lines before the first declaration.
fn metadata<'s>(src: &'s str, warnings: &mut Vec<Diag>) -> Vec<(&'s str, &'s str, Pos)> {
    let mut out = Vec::new();
    for (n, line) in src.lines().enumerate() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let Some(c) = t.strip_prefix("//") else { break };
        let c = c.trim_start();
        let Some(kv) = c.strip_prefix('@') else { continue };
        let (k, v) = match kv.find(|ch: char| ch.is_whitespace()) {
            Some(i) => (&kv[..i], kv[i..].trim()),
            None => (kv, ""),
        };
        let pos = Pos {
            line: n as u32 + 1,
            col: 1,
        };
        if !META_KEYS.contains(&k) {
            warnings.push(Diag::warning(pos, format!("unknown metadata key '@{}'", k)));
        }
        out.push((k, v, pos));
    }
    out
}

impl<'s> Parser<'s> {
    /// A parser reading `lex`, which must have passed [`Lexer::check`].
    fn new(lex: Lexer<'s>) -> Self {
        let mut toks = Stream::new(lex);
        let cur = toks.next();
        let ahead = toks.next();
        Parser {
            toks,
            cur,
            ahead,
            warnings: Vec::new(),
        }
    }
    fn peek(&self) -> &Tok<'s> {
        &self.cur.tok
    }
    /// The current token (`k` = 0) or the next one (`k` = 1).
    fn peek_at(&self, k: usize) -> &Tok<'s> {
        debug_assert!(k <= 1);
        if k == 0 {
            &self.cur.tok
        } else {
            &self.ahead.tok
        }
    }
    fn pos(&self) -> Pos {
        self.cur.pos
    }
    fn at_eof(&self) -> bool {
        *self.peek() == Tok::Eof
    }
    fn bump(&mut self) -> Token<'s> {
        let t = self.cur;
        if t.tok != Tok::Eof {
            self.cur = self.ahead;
            self.ahead = self.toks.next();
        }
        t
    }
    fn is(&self, p: P) -> bool {
        *self.peek() == Tok::P(p)
    }
    fn is_kw(&self, k: Kw) -> bool {
        *self.peek() == Tok::Kw(k)
    }
    fn eat(&mut self, p: P) -> bool {
        if self.is(p) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn eat_kw(&mut self, k: Kw) -> bool {
        if self.is_kw(k) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn skip_nl(&mut self) {
        while *self.peek() == Tok::Nl {
            self.bump();
        }
    }
    fn describe(t: &Tok<'_>) -> String {
        match t {
            Tok::Int(v) => format!("number {}", v),
            Tok::Fix(_) => String::from("a fix number"),
            Tok::Str(_) => String::from("a string"),
            Tok::Ident(s) => format!("'{}'", s),
            Tok::Kw(k) => format!("keyword '{}'", format!("{:?}", k).to_lowercase()),
            Tok::P(p) => format!("'{}'", p.text()),
            Tok::Nl => String::from("end of line"),
            Tok::Eof => String::from("end of file"),
        }
    }
    fn unexpected<T>(&self, what: &str) -> R<T> {
        Err(Diag::error(
            self.pos(),
            format!("expected {}, found {}", what, Self::describe(self.peek())),
        ))
    }
    fn expect(&mut self, p: P) -> R<Pos> {
        if self.is(p) {
            Ok(self.bump().pos)
        } else {
            self.unexpected(&format!("'{}'", p.text()))
        }
    }
    fn ident(&mut self, what: &str) -> R<Name<'s>> {
        match self.peek().clone() {
            Tok::Ident(s) => {
                let pos = self.bump().pos;
                Ok(Name { text: s, pos })
            }
            _ => self.unexpected(what),
        }
    }
    fn end_of_line(&mut self, after: &str) -> R<()> {
        match self.peek() {
            Tok::Nl => {
                self.skip_nl();
                Ok(())
            }
            Tok::Eof | Tok::P(P::RBrace) => Ok(()),
            _ => Err(Diag::error(
                self.pos(),
                format!("expected end of line after {}, found {}", after, Self::describe(self.peek())),
            )),
        }
    }

    // ---------- top level ----------

    fn item(&mut self) -> R<Item<'s>> {
        let pos = self.pos();
        match self.peek().clone() {
            Tok::Kw(Kw::Const) => {
                self.bump();
                let name = self.ident("a constant name")?;
                let shape = self.shape(false)?;
                self.expect(P::Assign)?;
                let init = self.init()?;
                Ok(Item::Const { name, shape, init })
            }
            Tok::Kw(Kw::Var) => {
                self.bump();
                let name = self.ident("a variable name")?;
                let shape = self.shape(false)?;
                let init = if self.eat(P::Assign) { Some(self.init()?) } else { None };
                Ok(Item::Var { name, shape, init })
            }
            Tok::Kw(Kw::Audio) if *self.peek_at(1) == Tok::Kw(Kw::Var) => {
                self.bump();
                self.bump();
                let name = self.ident("a variable name")?;
                let fix = if self.eat(P::Colon) { self.scalar_type()? } else { false };
                self.expect(P::Assign)?;
                let init = self.expr()?;
                Ok(Item::AudioVar { name, fix, init })
            }
            Tok::Kw(Kw::Audio) => {
                self.bump();
                self.handler(HandlerKind::Audio, pos)
            }
            Tok::Ident(s) if (s == "init" || s == "update" || s == "draw") && *self.peek_at(1) == Tok::P(P::LParen) => {
                self.bump();
                let kind = match s {
                    "init" => HandlerKind::Init,
                    "update" => HandlerKind::Update,
                    _ => HandlerKind::Draw,
                };
                self.handler(kind, pos)
            }
            Tok::Kw(Kw::Record) => {
                self.bump();
                let name = self.ident("a record name")?;
                self.expect(P::LBrace)?;
                let mut fields = Vec::new();
                self.list_body(|p| {
                    let name = p.ident("a field name")?;
                    let shape = p.shape(true)?;
                    fields.push(Field { name, shape });
                    Ok(())
                })?;
                Ok(Item::Record { name, fields })
            }
            Tok::Kw(Kw::Enum) => {
                self.bump();
                let name = self.ident("an enum name")?;
                self.expect(P::LBrace)?;
                let mut members = Vec::new();
                self.list_body(|p| {
                    let m = p.ident("an enum member")?;
                    let v = if p.eat(P::Assign) { Some(p.expr()?) } else { None };
                    members.push((m, v));
                    Ok(())
                })?;
                Ok(Item::Enum { name, members })
            }
            Tok::Kw(Kw::Sprite) => {
                self.bump();
                let name = self.ident("a sprite name")?;
                self.expect(P::Assign)?;
                let tile = self.expr()?;
                let size = if self.eat(P::Comma) {
                    let w = self.expr()?;
                    self.expect(P::Comma)?;
                    Some((w, self.expr()?))
                } else {
                    None
                };
                Ok(Item::Sprite { name, tile, size })
            }
            Tok::Kw(Kw::Fn) => {
                self.bump();
                let name = self.ident("a function name")?;
                let params = self.params()?;
                let ret_fix = if self.eat(P::Colon) { self.scalar_type()? } else { false };
                let body = self.skim_block()?;
                Ok(Item::Fn {
                    name,
                    params,
                    ret_fix,
                    body,
                })
            }
            Tok::Ident(s) if *self.peek_at(1) == Tok::P(P::LParen) => Err(Diag::error(
                pos,
                format!("'{}' looks like a function: declare it with 'fn {}(...)'", s, s),
            )),
            Tok::Ident(_) => Err(Diag::error(
                pos,
                "statements must be inside a function or handler; declare globals with 'var'",
            )),
            _ => self.unexpected("a declaration (var, const, fn, record, enum, sprite or a handler)"),
        }
    }

    /// A function or handler body: parsed to report syntax errors in order,
    /// then dropped. The compiler parses it again with [`parse_body`] when it
    /// needs it, so only one function's syntax tree is in memory at a time.
    fn skim_block(&mut self) -> R<Pos> {
        let pos = self.pos();
        self.block()?;
        Ok(pos)
    }

    /// Comma- or newline-separated items up to the closing brace (already past '{').
    fn list_body(&mut self, mut each: impl FnMut(&mut Self) -> R<()>) -> R<()> {
        self.skip_nl();
        while !self.eat(P::RBrace) {
            each(self)?;
            let sep = self.eat(P::Comma);
            let nl = *self.peek() == Tok::Nl;
            self.skip_nl();
            if !sep && !nl && !self.is(P::RBrace) {
                return self.unexpected("',' or a new line");
            }
        }
        Ok(())
    }

    fn handler(&mut self, kind: HandlerKind, pos: Pos) -> R<Item<'s>> {
        let params = self.params()?;
        let body = self.skim_block()?;
        Ok(Item::Handler { kind, pos, params, body })
    }

    fn params(&mut self) -> R<Vec<Param<'s>>> {
        self.expect(P::LParen)?;
        let mut v = Vec::new();
        if !self.eat(P::RParen) {
            loop {
                let name = self.ident("a parameter name")?;
                let shape = self.shape(false)?;
                v.push(Param { name, shape });
                if self.eat(P::RParen) {
                    break;
                }
                self.expect(P::Comma)?;
            }
        }
        Ok(v)
    }

    /// `fix` or `int` after a colon; returns true for fix.
    fn scalar_type(&mut self) -> R<bool> {
        if self.eat_kw(Kw::Fix) {
            return Ok(true);
        }
        if let Tok::Ident(s) = self.peek() {
            if *s == "int" {
                self.bump();
                return Ok(false);
            }
        }
        self.unexpected("'int' or 'fix'")
    }

    /// Dimensions after the name, or `: type dims`. Fields without a type are int.
    fn shape(&mut self, _field: bool) -> R<Shape<'s>> {
        let mut dims = self.dims()?;
        let mut ty = TypeName::Int;
        let mut explicit = false;
        if self.eat(P::Colon) {
            explicit = true;
            ty = if self.eat_kw(Kw::Fix) {
                TypeName::Fix
            } else {
                let n = self.ident("a type")?;
                if n.text == "int" {
                    TypeName::Int
                } else {
                    TypeName::Record(n)
                }
            };
            let more = self.dims()?;
            if !dims.is_empty() && !more.is_empty() {
                return Err(Diag::error(
                    self.pos(),
                    "give the dimensions either after the name or after the type",
                ));
            }
            if dims.is_empty() {
                dims = more;
            }
        }
        Ok(Shape { ty, dims, explicit })
    }

    fn dims(&mut self) -> R<Vec<Option<Expr<'s>>>> {
        let mut dims = Vec::new();
        while self.eat(P::LBrack) {
            if self.eat(P::RBrack) {
                dims.push(None);
            } else {
                dims.push(Some(self.expr()?));
                self.expect(P::RBrack)?;
            }
        }
        Ok(dims)
    }

    fn init(&mut self) -> R<Init<'s>> {
        if self.is(P::LBrace) {
            let pos = self.bump().pos;
            let mut items = Vec::new();
            self.skip_nl();
            while !self.eat(P::RBrace) {
                let field = if matches!(self.peek(), Tok::Ident(_)) && *self.peek_at(1) == Tok::P(P::Colon) {
                    let n = self.ident("a field name")?;
                    self.bump();
                    Some(n)
                } else {
                    None
                };
                self.skip_nl();
                let value = self.init()?;
                items.push(InitItem { field, value });
                self.skip_nl();
                if !self.eat(P::Comma) {
                    self.skip_nl();
                    self.expect(P::RBrace)?;
                    break;
                }
                self.skip_nl();
            }
            Ok(Init::List(items, pos))
        } else {
            Ok(Init::Expr(self.expr()?))
        }
    }

    // ---------- statements ----------

    fn block(&mut self) -> R<Block<'s>> {
        self.expect(P::LBrace)?;
        self.skip_nl();
        let mut stmts = Vec::new();
        loop {
            if self.is(P::RBrace) {
                let end = self.bump().pos;
                return Ok(Block { stmts, end });
            }
            if self.at_eof() {
                return self.unexpected("'}'");
            }
            stmts.push(self.stmt()?);
            self.end_of_line("a statement")?;
        }
    }

    fn stmt(&mut self) -> R<Stmt<'s>> {
        let pos = self.pos();
        let kind = match self.peek().clone() {
            Tok::Kw(Kw::Var) => self.local_decl()?,
            Tok::Kw(Kw::If) => return self.if_stmt(),
            Tok::Kw(Kw::While) => {
                self.bump();
                self.expect(P::LParen)?;
                let cond = self.expr()?;
                self.expect(P::RParen)?;
                StmtKind::While { cond, body: self.block()? }
            }
            Tok::Kw(Kw::For) => {
                self.bump();
                self.expect(P::LParen)?;
                let init = if self.is(P::Semi) {
                    None
                } else {
                    let p = self.pos();
                    let k = if self.is_kw(Kw::Var) {
                        self.local_decl()?
                    } else {
                        self.simple_stmt(false)?
                    };
                    Some(Box::new(Stmt { kind: k, pos: p }))
                };
                self.expect(P::Semi)?;
                let cond = if self.is(P::Semi) { None } else { Some(self.expr()?) };
                self.expect(P::Semi)?;
                let step = if self.is(P::RParen) {
                    None
                } else {
                    let p = self.pos();
                    Some(Box::new(Stmt {
                        kind: self.simple_stmt(false)?,
                        pos: p,
                    }))
                };
                self.expect(P::RParen)?;
                StmtKind::For {
                    init,
                    cond,
                    step,
                    body: self.block()?,
                }
            }
            Tok::Kw(Kw::Switch) => {
                self.bump();
                self.expect(P::LParen)?;
                let value = self.expr()?;
                self.expect(P::RParen)?;
                self.expect(P::LBrace)?;
                self.skip_nl();
                let mut cases = Vec::new();
                let mut default = None;
                while !self.eat(P::RBrace) {
                    if self.eat_kw(Kw::Else) {
                        if default.is_some() {
                            return Err(Diag::error(pos, "a switch has at most one else"));
                        }
                        default = Some(self.block()?);
                    } else {
                        if default.is_some() {
                            return Err(Diag::error(self.pos(), "else must be the last case of a switch"));
                        }
                        let mut vals = Vec::new();
                        loop {
                            vals.push(self.expr()?);
                            if !self.eat(P::Comma) {
                                break;
                            }
                        }
                        cases.push((vals, self.block()?));
                    }
                    self.skip_nl();
                }
                StmtKind::Switch { value, cases, default }
            }
            Tok::Kw(Kw::Break) => {
                self.bump();
                StmtKind::Break
            }
            Tok::Kw(Kw::Continue) => {
                self.bump();
                StmtKind::Continue
            }
            Tok::Kw(Kw::Return) => {
                self.bump();
                let v = if matches!(self.peek(), Tok::Nl | Tok::P(P::RBrace) | Tok::Eof) {
                    None
                } else {
                    Some(self.expr()?)
                };
                StmtKind::Return(v)
            }
            _ => self.simple_stmt(true)?,
        };
        Ok(Stmt { kind, pos })
    }

    fn local_decl(&mut self) -> R<StmtKind<'s>> {
        self.expect_kw(Kw::Var)?;
        let name = self.ident("a variable name")?;
        if self.is(P::LBrack) {
            return Err(Diag::error(
                self.pos(),
                "local arrays are not supported; declare the array at top level",
            ));
        }
        let fix = if self.eat(P::Colon) {
            if let Tok::Ident(s) = self.peek() {
                if *s != "int" {
                    return Err(Diag::error(self.pos(), "locals are int or fix; declare records at top level"));
                }
            }
            self.scalar_type()?
        } else {
            false
        };
        let init = if self.eat(P::Assign) { Some(self.expr()?) } else { None };
        Ok(StmtKind::Var { name, fix, init })
    }

    fn expect_kw(&mut self, k: Kw) -> R<()> {
        if self.eat_kw(k) {
            Ok(())
        } else {
            self.unexpected(&format!("'{}'", format!("{:?}", k).to_lowercase()))
        }
    }

    fn if_stmt(&mut self) -> R<Stmt<'s>> {
        let pos = self.pos();
        self.expect_kw(Kw::If)?;
        self.expect(P::LParen)?;
        let cond = self.expr()?;
        self.expect(P::RParen)?;
        let then = self.block()?;
        // `else` may follow on the same line or the next (newline runs are one token)
        if *self.peek() == Tok::Nl && *self.peek_at(1) == Tok::Kw(Kw::Else) {
            self.bump();
        }
        let els = if self.eat_kw(Kw::Else) {
            if self.is_kw(Kw::If) {
                Some(Box::new(Else::If(self.if_stmt()?)))
            } else {
                Some(Box::new(Else::Block(self.block()?)))
            }
        } else {
            None
        };
        Ok(Stmt {
            kind: StmtKind::If { cond, then, els },
            pos,
        })
    }

    /// Assignment, increment or call.
    fn simple_stmt(&mut self, allow_call: bool) -> R<StmtKind<'s>> {
        let pos = self.pos();
        let name = match self.peek() {
            Tok::Ident(_) => self.ident("a name")?,
            _ => return self.unexpected("a statement"),
        };
        if self.is(P::LParen) {
            let args = self.args()?;
            if !allow_call {
                return Err(Diag::error(pos, "expected an assignment"));
            }
            return Ok(StmtKind::Call(Expr {
                kind: ExprKind::Call(name, args),
                pos,
            }));
        }
        let target = self.path_rest(name)?;
        let op = match self.peek() {
            Tok::P(P::PlusPlus) => {
                self.bump();
                return Ok(StmtKind::IncDec { target, inc: true });
            }
            Tok::P(P::MinusMinus) => {
                self.bump();
                return Ok(StmtKind::IncDec { target, inc: false });
            }
            Tok::P(P::Assign) => AssignOp::Set,
            Tok::P(P::PlusEq) => AssignOp::Op(BinOp::Add),
            Tok::P(P::MinusEq) => AssignOp::Op(BinOp::Sub),
            Tok::P(P::StarEq) => AssignOp::Op(BinOp::Mul),
            Tok::P(P::SlashEq) => AssignOp::Op(BinOp::Div),
            Tok::P(P::PercentEq) => AssignOp::Op(BinOp::Mod),
            Tok::P(P::AmpEq) => AssignOp::Op(BinOp::BitAnd),
            Tok::P(P::PipeEq) => AssignOp::Op(BinOp::BitOr),
            Tok::P(P::CaretEq) => AssignOp::Op(BinOp::BitXor),
            Tok::P(P::ShlEq) => AssignOp::Op(BinOp::Shl),
            Tok::P(P::ShrEq) => AssignOp::Op(BinOp::Shr),
            Tok::P(P::EqEq) => return Err(Diag::error(self.pos(), "'==' compares; use '=' to assign")),
            _ => return self.unexpected("'=', an assignment operator, '++' or '--'"),
        };
        self.bump();
        let value = self.expr()?;
        Ok(StmtKind::Assign { target, op, value })
    }

    // ---------- expressions ----------

    fn args(&mut self) -> R<Vec<Expr<'s>>> {
        self.expect(P::LParen)?;
        let mut v = Vec::new();
        if !self.eat(P::RParen) {
            loop {
                v.push(self.expr()?);
                if self.eat(P::RParen) {
                    break;
                }
                self.expect(P::Comma)?;
            }
        }
        Ok(v)
    }

    fn path_rest(&mut self, root: Name<'s>) -> R<Path<'s>> {
        let mut sels = Vec::new();
        loop {
            if self.eat(P::LBrack) {
                sels.push(Sel::Index(self.expr()?));
                self.expect(P::RBrack)?;
            } else if self.eat(P::Dot) {
                sels.push(Sel::Field(self.ident("a field name")?));
            } else {
                return Ok(Path { root, sels });
            }
        }
    }

    pub fn expr(&mut self) -> R<Expr<'s>> {
        let c = self.or()?;
        if self.eat(P::Question) {
            let a = self.expr()?;
            self.expect(P::Colon)?;
            let b = self.expr()?;
            let pos = c.pos;
            return Ok(Expr {
                kind: ExprKind::Cond(Box::new(c), Box::new(a), Box::new(b)),
                pos,
            });
        }
        Ok(c)
    }

    fn or(&mut self) -> R<Expr<'s>> {
        let (mut l, mut mixed) = self.and()?;
        let mut any_or = false;
        while self.is(P::OrOr) {
            let pos = self.bump().pos;
            let (r, m2) = self.and()?;
            mixed |= m2;
            any_or = true;
            l = bin(BinOp::Or, l, r, pos);
        }
        if any_or && mixed {
            self.warnings
                .push(Diag::warning(l.pos, "'&&' and '||' mixed without parentheses; '&&' binds tighter"));
        }
        Ok(l)
    }

    /// Returns whether an unparenthesized `&&` appeared.
    fn and(&mut self) -> R<(Expr<'s>, bool)> {
        let mut l = self.binary(0)?;
        let mut any = false;
        while self.is(P::AndAnd) {
            let pos = self.bump().pos;
            let r = self.binary(0)?;
            any = true;
            l = bin(BinOp::And, l, r, pos);
        }
        Ok((l, any))
    }

    /// Levels from `|` (0) to `* / %` (7).
    fn binary(&mut self, level: u8) -> R<Expr<'s>> {
        if level > 7 {
            return self.unary();
        }
        let mut l = self.binary(level + 1)?;
        loop {
            let op = match (level, self.peek()) {
                (0, Tok::P(P::Pipe)) => BinOp::BitOr,
                (1, Tok::P(P::Caret)) => BinOp::BitXor,
                (2, Tok::P(P::Amp)) => BinOp::BitAnd,
                (3, Tok::P(P::EqEq)) => BinOp::Eq,
                (3, Tok::P(P::Ne)) => BinOp::Ne,
                (4, Tok::P(P::Lt)) => BinOp::Lt,
                (4, Tok::P(P::Le)) => BinOp::Le,
                (4, Tok::P(P::Gt)) => BinOp::Gt,
                (4, Tok::P(P::Ge)) => BinOp::Ge,
                (5, Tok::P(P::Shl)) => BinOp::Shl,
                (5, Tok::P(P::Shr)) => BinOp::Shr,
                (6, Tok::P(P::Plus)) => BinOp::Add,
                (6, Tok::P(P::Minus)) => BinOp::Sub,
                (7, Tok::P(P::Star)) => BinOp::Mul,
                (7, Tok::P(P::Slash)) => BinOp::Div,
                (7, Tok::P(P::Percent)) => BinOp::Mod,
                _ => return Ok(l),
            };
            let pos = self.bump().pos;
            let r = self.binary(level + 1)?;
            l = bin(op, l, r, pos);
        }
    }

    fn unary(&mut self) -> R<Expr<'s>> {
        let pos = self.pos();
        let op = match self.peek() {
            Tok::P(P::Minus) => UnOp::Neg,
            Tok::P(P::Bang) => UnOp::Not,
            Tok::P(P::Tilde) => UnOp::BitNot,
            _ => return self.primary(),
        };
        self.bump();
        let e = self.unary()?;
        Ok(Expr {
            kind: ExprKind::Unary(op, Box::new(e)),
            pos,
        })
    }

    fn primary(&mut self) -> R<Expr<'s>> {
        let pos = self.pos();
        match self.peek().clone() {
            Tok::Int(v) => {
                self.bump();
                Ok(Expr {
                    kind: ExprKind::Int(v),
                    pos,
                })
            }
            Tok::Fix(v) => {
                self.bump();
                Ok(Expr {
                    kind: ExprKind::Fix(v),
                    pos,
                })
            }
            Tok::Str(raw) => {
                self.bump();
                let parts = string_parts(&raw, pos)?;
                Ok(Expr {
                    kind: ExprKind::Str(parts),
                    pos,
                })
            }
            Tok::P(P::LParen) => {
                self.bump();
                let e = self.expr()?;
                self.expect(P::RParen)?;
                Ok(e)
            }
            Tok::Ident(_) => {
                let name = self.ident("a name")?;
                if self.is(P::LParen) {
                    let args = self.args()?;
                    return Ok(Expr {
                        kind: ExprKind::Call(name, args),
                        pos,
                    });
                }
                let path = self.path_rest(name)?;
                Ok(Expr {
                    kind: ExprKind::Path(path),
                    pos,
                })
            }
            Tok::Kw(Kw::Fix) => Err(Diag::error(pos, "'fix' is a type; numbers with a decimal point are fix")),
            _ => self.unexpected("an expression"),
        }
    }
}

fn bin<'s>(op: BinOp, l: Expr<'s>, r: Expr<'s>, pos: Pos) -> Expr<'s> {
    let _ = pos;
    let p = l.pos;
    Expr {
        kind: ExprKind::Binary(op, Box::new(l), Box::new(r)),
        pos: p,
    }
}

/// Splits a raw string literal into text and `{expr:format}` parts.
fn string_parts<'s>(raw: &'s str, pos: Pos) -> R<Vec<StrPart<'s>>> {
    let b = raw.as_bytes();
    let mut parts = Vec::new();
    let mut lit: Vec<u8> = Vec::new();
    let mut i = 0;
    // column of raw[0] is one past the opening quote
    let at = |i: usize| Pos {
        line: pos.line,
        col: pos.col + 1 + i as u32,
    };
    while i < b.len() {
        match b[i] {
            b'\\' => {
                let c = match b.get(i + 1) {
                    Some(b'"') => b'"',
                    Some(b'\\') => b'\\',
                    Some(b'n') => b'\n',
                    _ => return Err(Diag::error(at(i), "unknown escape; strings allow \\\" \\\\ and \\n")),
                };
                lit.push(c);
                i += 2;
            }
            b'{' if b.get(i + 1) == Some(&b'{') => {
                lit.push(b'{');
                i += 2;
            }
            b'}' if b.get(i + 1) == Some(&b'}') => {
                lit.push(b'}');
                i += 2;
            }
            b'}' => return Err(Diag::error(at(i), "'}' in a string: write '}}' for a brace")),
            b'{' => {
                let end = match b[i + 1..].iter().position(|&c| c == b'}') {
                    Some(e) => i + 1 + e,
                    None => {
                        return Err(Diag::error(
                            at(i),
                            "'{' in a string starts a value; close it with '}' or write '{{'",
                        ))
                    }
                };
                let inner = &raw[i + 1..end];
                if !lit.is_empty() {
                    parts.push(StrPart::Lit(core::mem::take(&mut lit)));
                }
                parts.push(interp(inner, at(i + 1))?);
                i = end + 1;
            }
            c => {
                lit.push(c);
                i += 1;
            }
        }
    }
    if !lit.is_empty() || parts.is_empty() {
        parts.push(StrPart::Lit(lit));
    }
    let len: usize = parts.iter().map(|p| if let StrPart::Lit(l) = p { l.len() } else { 0 }).sum();
    if len > 255 {
        return Err(Diag::error(pos, "string literals hold at most 255 characters"));
    }
    Ok(parts)
}

fn parse_fragment<'s>(s: &'s str, pos: Pos) -> R<Expr<'s>> {
    Lexer::fragment(s, pos).check()?;
    let mut p = Parser::new(Lexer::fragment(s, pos));
    let e = p.expr()?;
    if !p.at_eof() {
        return p.unexpected("'}'");
    }
    Ok(e)
}

fn interp<'s>(inner: &'s str, pos: Pos) -> R<StrPart<'s>> {
    if inner.trim().is_empty() {
        return Err(Diag::error(pos, "empty '{}' in a string"));
    }
    // A whole expression first: `{a ? b : c}` has a colon but no format.
    let whole = parse_fragment(inner, pos);
    if let Ok(e) = whole {
        return Ok(StrPart::Interp(e, Spec::default()));
    }
    if let Some(c) = inner.rfind(':') {
        if let Some(spec) = format_spec(&inner[c + 1..]) {
            let e = parse_fragment(&inner[..c], pos)?;
            return Ok(StrPart::Interp(e, spec));
        }
        if !inner[c + 1..].contains([' ', '?']) && inner[c + 1..].len() <= 6 {
            return Err(Diag::error(
                pos,
                format!(
                    "unknown format '{}'; use width (4), zero padding (04), decimals (.2), x or c",
                    &inner[c + 1..]
                ),
            ));
        }
    }
    whole.map(|e| StrPart::Interp(e, Spec::default()))
}

/// `04`, `6.2`, `.1`, `x`, `08x`, `c`.
fn format_spec(s: &str) -> Option<Spec> {
    let b = s.as_bytes();
    if b.is_empty() {
        return None;
    }
    let mut sp = Spec::default();
    let mut i = 0;
    if b[0] == b'0' && b.len() > 1 && b[1].is_ascii_digit() {
        sp.zero = true;
        i = 1;
    }
    let mut w: u32 = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        w = w * 10 + (b[i] - b'0') as u32;
        i += 1;
    }
    if w > 64 {
        return None;
    }
    sp.width = w as u8;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let s = i;
        let mut d: u32 = 0;
        while i < b.len() && b[i].is_ascii_digit() {
            d = d * 10 + (b[i] - b'0') as u32;
            i += 1;
        }
        if i == s || d > bidule_bytecode::image::MAX_DECIMALS as u32 {
            return None;
        }
        sp.decimals = Some(d as u8);
    }
    if i < b.len() && (b[i] == b'x' || b[i] == b'c') {
        if sp.decimals.is_some() {
            return None;
        }
        sp.hex = b[i] == b'x';
        sp.chr = b[i] == b'c';
        i += 1;
    }
    if i != b.len() {
        return None;
    }
    Some(sp)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_spec_example() {
        let src = include_str!("../../../carts/blink.bdcart");
        let (p, w) = parse(src).unwrap();
        assert!(w.is_empty(), "{:?}", w);
        assert_eq!(p.meta.len(), 3);
        assert!(p.items.len() >= 10);
    }
    #[test]
    fn interpolation() {
        let parts = string_parts("Score {score:04} {a ? b : c} {{x}}", Pos::default()).unwrap();
        assert_eq!(parts.len(), 5);
        match &parts[1] {
            StrPart::Interp(_, s) => assert!(s.zero && s.width == 4),
            _ => panic!(),
        }
        assert!(matches!(&parts[3], StrPart::Interp(e, s) if matches!(e.kind, ExprKind::Cond(..)) && *s == Spec::default()));
        assert!(matches!(&parts[4], StrPart::Lit(l) if l == b" {x}"));
    }
    #[test]
    fn all_sample_carts_parse() {
        for src in [
            include_str!("../../../carts/spark-catcher.bdcart"),
            include_str!("../../../carts/tile-peek.bdcart"),
            include_str!("../../../carts/sprite-editor.bdcart"),
            include_str!("../../../carts/new-cart.bdcart"),
        ] {
            if let Err(e) = parse(src) {
                panic!("{}", e);
            }
        }
    }
}
