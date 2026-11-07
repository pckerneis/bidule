//! Source text to tokens (spec: Language › Program structure and lexical rules).

use alloc::vec::Vec;

use crate::diag::{Diag, Pos};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tok<'s> {
    Int(i32),
    /// 16.16 raw value.
    Fix(i32),
    /// Raw text between the quotes, escapes not yet processed (interpolation is parsed later).
    Str(&'s str),
    Ident(&'s str),
    Kw(Kw),
    P(P),
    Nl,
    Eof,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kw {
    Var,
    Const,
    Fn,
    Audio,
    Fix,
    Record,
    Sprite,
    Enum,
    Switch,
    If,
    Else,
    While,
    For,
    Break,
    Continue,
    Return,
}

/// Punctuation and operators.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum P {
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBrack,
    RBrack,
    Comma,
    Semi,
    Colon,
    Dot,
    Question,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Amp,
    Pipe,
    Caret,
    Tilde,
    Bang,
    Shl,
    Shr,
    Lt,
    Le,
    Gt,
    Ge,
    EqEq,
    Ne,
    AndAnd,
    OrOr,
    Assign,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    AmpEq,
    PipeEq,
    CaretEq,
    ShlEq,
    ShrEq,
    PlusPlus,
    MinusMinus,
}

impl P {
    pub fn text(self) -> &'static str {
        use P::*;
        match self {
            LParen => "(",
            RParen => ")",
            LBrace => "{",
            RBrace => "}",
            LBrack => "[",
            RBrack => "]",
            Comma => ",",
            Semi => ";",
            Colon => ":",
            Dot => ".",
            Question => "?",
            Plus => "+",
            Minus => "-",
            Star => "*",
            Slash => "/",
            Percent => "%",
            Amp => "&",
            Pipe => "|",
            Caret => "^",
            Tilde => "~",
            Bang => "!",
            Shl => "<<",
            Shr => ">>",
            Lt => "<",
            Le => "<=",
            Gt => ">",
            Ge => ">=",
            EqEq => "==",
            Ne => "!=",
            AndAnd => "&&",
            OrOr => "||",
            Assign => "=",
            PlusEq => "+=",
            MinusEq => "-=",
            StarEq => "*=",
            SlashEq => "/=",
            PercentEq => "%=",
            AmpEq => "&=",
            PipeEq => "|=",
            CaretEq => "^=",
            ShlEq => "<<=",
            ShrEq => ">>=",
            PlusPlus => "++",
            MinusMinus => "--",
        }
    }
}

fn keyword(s: &str) -> Option<Kw> {
    Some(match s {
        "var" => Kw::Var,
        "const" => Kw::Const,
        "fn" => Kw::Fn,
        "audio" => Kw::Audio,
        "fix" => Kw::Fix,
        "record" => Kw::Record,
        "sprite" => Kw::Sprite,
        "enum" => Kw::Enum,
        "switch" => Kw::Switch,
        "if" => Kw::If,
        "else" => Kw::Else,
        "while" => Kw::While,
        "for" => Kw::For,
        "break" => Kw::Break,
        "continue" => Kw::Continue,
        "return" => Kw::Return,
        _ => return None,
    })
}

#[derive(Clone, Copy, Debug)]
pub struct Token<'s> {
    pub tok: Tok<'s>,
    pub pos: Pos,
}

/// Tokens on demand, as [`Lexer::tokenize`] would list them (newline runs
/// collapsed, none at the start), then `Eof` forever. Only for a source that
/// passed [`Lexer::check`]: a lexing error reads as the end of the file.
pub struct Stream<'s> {
    lex: Lexer<'s>,
    /// The last token given was a newline, or none was given yet.
    after_nl: bool,
    eof: Option<Token<'s>>,
}

impl<'s> Stream<'s> {
    pub fn new(lex: Lexer<'s>) -> Self {
        Stream {
            lex,
            after_nl: true,
            eof: None,
        }
    }

    /// The next token and the byte offset where it starts.
    pub fn next_token(&mut self) -> (Token<'s>, u32) {
        if let Some(t) = self.eof {
            return (t, self.lex.start as u32);
        }
        loop {
            let t = match self.lex.next() {
                Ok(t) => t,
                Err(d) => Token { tok: Tok::Eof, pos: d.pos },
            };
            if t.tok == Tok::Nl && self.after_nl {
                continue;
            }
            self.after_nl = t.tok == Tok::Nl;
            if t.tok == Tok::Eof {
                self.eof = Some(t);
            }
            return (t, self.lex.start as u32);
        }
    }
}

pub struct Lexer<'s> {
    src: &'s [u8],
    i: usize,
    /// Where the last token started.
    start: usize,
    line: u32,
    line_start: usize,
    /// Parentheses and brackets open: newlines inside them are not statement ends.
    depth: u32,
    /// Base position, for re-lexing an interpolated expression inside a string.
    base: Option<Pos>,
}

impl<'s> Lexer<'s> {
    pub fn new(src: &'s str) -> Self {
        Lexer {
            src: src.as_bytes(),
            i: 0,
            start: 0,
            line: 1,
            line_start: 0,
            depth: 0,
            base: None,
        }
    }

    /// Lexes from a token the lexer found in `src` (not in a fragment): its
    /// position and byte offset.
    pub fn at(src: &'s str, at: Pos, offset: u32) -> Self {
        let i = offset as usize;
        Lexer {
            src: src.as_bytes(),
            i,
            start: i,
            line: at.line,
            line_start: i + 1 - at.col as usize,
            depth: 0,
            base: None,
        }
    }

    /// Lexes a fragment whose first character is at `base` (all on one line).
    pub fn fragment(src: &'s str, base: Pos) -> Self {
        Lexer {
            src: src.as_bytes(),
            i: 0,
            start: 0,
            line: 1,
            line_start: 0,
            depth: 1,
            base: Some(base),
        }
    }

    fn pos(&self) -> Pos {
        let col = (self.i - self.line_start) as u32 + 1;
        match self.base {
            Some(b) => Pos {
                line: b.line,
                col: b.col + col - 1,
            },
            None => Pos { line: self.line, col },
        }
    }

    fn peek(&self, k: usize) -> u8 {
        *self.src.get(self.i + k).unwrap_or(&0)
    }

    /// Lexes the whole source without keeping the tokens: the first lexing
    /// error, if any. The parser checks first, then reads a [`Stream`].
    pub fn check(mut self) -> Result<(), Diag> {
        while self.next()?.tok != Tok::Eof {}
        Ok(())
    }

    pub fn tokenize(mut self) -> Result<Vec<Token<'s>>, Diag> {
        let mut out: Vec<Token<'s>> = Vec::new();
        loop {
            let t = self.next()?;
            let eof = t.tok == Tok::Eof;
            // collapse runs of newlines
            if t.tok == Tok::Nl && out.last().is_none_or(|p| p.tok == Tok::Nl) {
                continue;
            }
            out.push(t);
            if eof {
                return Ok(out);
            }
        }
    }

    fn next(&mut self) -> Result<Token<'s>, Diag> {
        // skip spaces, comments, and newlines inside ( ) or [ ]
        loop {
            match self.peek(0) {
                b' ' | b'\t' | b'\r' => self.i += 1,
                b'/' if self.peek(1) == b'/' => {
                    while self.i < self.src.len() && self.src[self.i] != b'\n' {
                        self.i += 1;
                    }
                }
                b'\n' if self.depth > 0 => self.newline(),
                _ => break,
            }
        }
        let pos = self.pos();
        self.start = self.i;
        let c = self.peek(0);
        if self.i >= self.src.len() {
            return Ok(Token { tok: Tok::Eof, pos });
        }
        if c == b'\n' {
            self.newline();
            return Ok(Token { tok: Tok::Nl, pos });
        }
        if c.is_ascii_digit() {
            return self.number(pos);
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            let s = self.i;
            while self.peek(0).is_ascii_alphanumeric() || self.peek(0) == b'_' {
                self.i += 1;
            }
            let word = core::str::from_utf8(&self.src[s..self.i]).unwrap();
            let tok = match keyword(word) {
                Some(k) => Tok::Kw(k),
                None => Tok::Ident(word),
            };
            return Ok(Token { tok, pos });
        }
        if c == b'\'' {
            return self.char_lit(pos);
        }
        if c == b'"' {
            return self.string(pos);
        }
        self.punct(pos)
    }

    fn newline(&mut self) {
        self.i += 1;
        self.line += 1;
        self.line_start = self.i;
    }

    fn number(&mut self, pos: Pos) -> Result<Token<'s>, Diag> {
        let s = self.i;
        let (radix, start) = match (self.peek(0), self.peek(1)) {
            (b'0', b'x' | b'X') => (16, s + 2),
            (b'0', b'b' | b'B') => (2, s + 2),
            _ => (10, s),
        };
        self.i = start;
        let mut v: u64 = 0;
        let mut digits = 0;
        loop {
            let c = self.peek(0);
            if c == b'_' {
                self.i += 1;
                continue;
            }
            let d = match (c as char).to_digit(radix) {
                Some(d) => d,
                None => break,
            };
            v = v * radix as u64 + d as u64;
            digits += 1;
            if v > u32::MAX as u64 {
                return Err(Diag::error(pos, "integer literal doesn't fit in 32 bits"));
            }
            self.i += 1;
        }
        if digits == 0 {
            return Err(Diag::error(pos, "malformed number"));
        }
        if radix == 10 && self.peek(0) == b'.' && self.peek(1).is_ascii_digit() {
            // fix literal: round to the nearest 1/65536
            self.i += 1;
            let mut num: u128 = 0;
            let mut den: u128 = 1;
            while self.peek(0).is_ascii_digit() || self.peek(0) == b'_' {
                let c = self.peek(0);
                self.i += 1;
                if c == b'_' {
                    continue;
                }
                if den < 10u128.pow(30) {
                    num = num * 10 + (c - b'0') as u128;
                    den *= 10;
                }
            }
            let frac = (num * 65536 * 2 + den) / (den * 2);
            let raw = v as u128 * 65536 + frac;
            if raw > i32::MAX as u128 {
                return Err(Diag::error(pos, "fix literal out of range (at most 32767.99)"));
            }
            return Ok(Token {
                tok: Tok::Fix(raw as i32),
                pos,
            });
        }
        if self.peek(0).is_ascii_alphanumeric() {
            return Err(Diag::error(pos, "malformed number"));
        }
        Ok(Token {
            tok: Tok::Int(v as u32 as i32),
            pos,
        })
    }

    fn char_lit(&mut self, pos: Pos) -> Result<Token<'s>, Diag> {
        self.i += 1;
        let c = match self.peek(0) {
            b'\\' => {
                self.i += 1;
                match self.peek(0) {
                    b'\'' => b'\'',
                    b'\\' => b'\\',
                    b'n' => b'\n',
                    _ => return Err(Diag::error(pos, "unknown escape in character literal; use \\' \\\\ or \\n")),
                }
            }
            c if (32..=126).contains(&c) && c != b'\'' => c,
            _ => return Err(Diag::error(pos, "malformed character literal")),
        };
        self.i += 1;
        if self.peek(0) != b'\'' {
            return Err(Diag::error(pos, "character literal must hold exactly one character"));
        }
        self.i += 1;
        Ok(Token {
            tok: Tok::Int(c as i32),
            pos,
        })
    }

    fn string(&mut self, pos: Pos) -> Result<Token<'s>, Diag> {
        self.i += 1;
        let s = self.i;
        loop {
            if self.i >= self.src.len() || self.peek(0) == b'\n' {
                return Err(Diag::error(pos, "string literal is not closed on this line"));
            }
            match self.peek(0) {
                b'"' => break,
                b'\\' if self.peek(1) == b'\n' || self.peek(1) == 0 => self.i += 1,
                b'\\' => self.i += 2,
                c if !(32..=126).contains(&c) => return Err(Diag::error(self.pos(), "strings hold printable ASCII only")),
                _ => self.i += 1,
            }
        }
        let raw = core::str::from_utf8(&self.src[s..self.i]).unwrap();
        self.i += 1;
        Ok(Token { tok: Tok::Str(raw), pos })
    }

    fn punct(&mut self, pos: Pos) -> Result<Token<'s>, Diag> {
        use P::*;
        let (a, b, c) = (self.peek(0), self.peek(1), self.peek(2));
        let (p, n) = match (a, b, c) {
            (b'<', b'<', b'=') => (ShlEq, 3),
            (b'>', b'>', b'=') => (ShrEq, 3),
            (b'<', b'<', _) => (Shl, 2),
            (b'>', b'>', _) => (Shr, 2),
            (b'<', b'=', _) => (Le, 2),
            (b'>', b'=', _) => (Ge, 2),
            (b'=', b'=', _) => (EqEq, 2),
            (b'!', b'=', _) => (Ne, 2),
            (b'&', b'&', _) => (AndAnd, 2),
            (b'|', b'|', _) => (OrOr, 2),
            (b'+', b'=', _) => (PlusEq, 2),
            (b'-', b'=', _) => (MinusEq, 2),
            (b'*', b'=', _) => (StarEq, 2),
            (b'/', b'=', _) => (SlashEq, 2),
            (b'%', b'=', _) => (PercentEq, 2),
            (b'&', b'=', _) => (AmpEq, 2),
            (b'|', b'=', _) => (PipeEq, 2),
            (b'^', b'=', _) => (CaretEq, 2),
            (b'+', b'+', _) => (PlusPlus, 2),
            (b'-', b'-', _) => (MinusMinus, 2),
            (b'(', ..) => (LParen, 1),
            (b')', ..) => (RParen, 1),
            (b'{', ..) => (LBrace, 1),
            (b'}', ..) => (RBrace, 1),
            (b'[', ..) => (LBrack, 1),
            (b']', ..) => (RBrack, 1),
            (b',', ..) => (Comma, 1),
            (b';', ..) => (Semi, 1),
            (b':', ..) => (Colon, 1),
            (b'.', ..) => (Dot, 1),
            (b'?', ..) => (Question, 1),
            (b'+', ..) => (Plus, 1),
            (b'-', ..) => (Minus, 1),
            (b'*', ..) => (Star, 1),
            (b'/', ..) => (Slash, 1),
            (b'%', ..) => (Percent, 1),
            (b'&', ..) => (Amp, 1),
            (b'|', ..) => (Pipe, 1),
            (b'^', ..) => (Caret, 1),
            (b'~', ..) => (Tilde, 1),
            (b'!', ..) => (Bang, 1),
            (b'<', ..) => (Lt, 1),
            (b'>', ..) => (Gt, 1),
            (b'=', ..) => (Assign, 1),
            _ => {
                let msg = if a.is_ascii_graphic() {
                    "unexpected character"
                } else {
                    "unexpected non-ASCII character"
                };
                return Err(Diag::error(pos, msg));
            }
        };
        self.i += n;
        match p {
            LParen | LBrack => self.depth += 1,
            RParen | RBrack => self.depth = self.depth.saturating_sub(1),
            _ => {}
        }
        Ok(Token { tok: Tok::P(p), pos })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn toks(s: &str) -> Vec<Tok<'_>> {
        Lexer::new(s).tokenize().unwrap().into_iter().map(|t| t.tok).collect()
    }
    #[test]
    fn numbers() {
        assert_eq!(
            toks("42 0x2A 0b101010 1_000")[..4],
            [Tok::Int(42), Tok::Int(42), Tok::Int(42), Tok::Int(1000)]
        );
        assert_eq!(toks("0xFFFFFFFF")[0], Tok::Int(-1));
        assert_eq!(toks("0.5")[0], Tok::Fix(32768));
        assert_eq!(toks("1.0")[0], Tok::Fix(65536));
        assert_eq!(toks("0.05")[0], Tok::Fix(3277));
        assert_eq!(toks("'A'")[0], Tok::Int(65));
        assert!(Lexer::new("40000.0").tokenize().is_err());
    }
    #[test]
    fn newlines_inside_parens_are_skipped() {
        let t = toks("f(1,\n2)\nx");
        assert_eq!(t.iter().filter(|t| **t == Tok::Nl).count(), 1);
    }
}
