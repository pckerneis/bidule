//! Syntax tree.

use alloc::boxed::Box;
use alloc::vec::Vec;

use crate::diag::Pos;

#[derive(Clone, Copy, Debug)]
pub struct Name<'s> {
    pub text: &'s str,
    pub pos: Pos,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Shl,
    Shr,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    BitAnd,
    BitXor,
    BitOr,
    And,
    Or,
}

impl BinOp {
    pub fn text(self) -> &'static str {
        use BinOp::*;
        match self {
            Add => "+",
            Sub => "-",
            Mul => "*",
            Div => "/",
            Mod => "%",
            Shl => "<<",
            Shr => ">>",
            Lt => "<",
            Le => "<=",
            Gt => ">",
            Ge => ">=",
            Eq => "==",
            Ne => "!=",
            BitAnd => "&",
            BitXor => "^",
            BitOr => "|",
            And => "&&",
            Or => "||",
        }
    }
    pub fn is_bitwise(self) -> bool {
        matches!(self, BinOp::Shl | BinOp::Shr | BinOp::BitAnd | BinOp::BitXor | BinOp::BitOr)
    }
    pub fn is_compare(self) -> bool {
        matches!(self, BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::Eq | BinOp::Ne)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
    BitNot,
}

/// Format of an interpolated value: `{n:04}`, `{v:.1}`, `{n:x}`, `{c:c}`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Spec {
    pub zero: bool,
    pub width: u8,
    pub decimals: Option<u8>,
    pub hex: bool,
    pub chr: bool,
}

#[derive(Clone, Debug)]
pub enum StrPart<'s> {
    Lit(Vec<u8>),
    Interp(Expr<'s>, Spec),
}

#[derive(Clone, Debug)]
pub enum Sel<'s> {
    Index(Expr<'s>),
    Field(Name<'s>),
}

#[derive(Clone, Debug)]
pub struct Path<'s> {
    pub root: Name<'s>,
    pub sels: Vec<Sel<'s>>,
}

#[derive(Clone, Debug)]
pub enum ExprKind<'s> {
    Int(i32),
    Fix(i32),
    Str(Vec<StrPart<'s>>),
    Path(Path<'s>),
    Call(Name<'s>, Vec<Expr<'s>>),
    Unary(UnOp, Box<Expr<'s>>),
    Binary(BinOp, Box<Expr<'s>>, Box<Expr<'s>>),
    Cond(Box<Expr<'s>>, Box<Expr<'s>>, Box<Expr<'s>>),
}

#[derive(Clone, Debug)]
pub struct Expr<'s> {
    pub kind: ExprKind<'s>,
    pub pos: Pos,
}

impl<'s> Expr<'s> {
    /// A string literal with no interpolation: its bytes.
    pub fn plain_str(&self) -> Option<Vec<u8>> {
        match &self.kind {
            ExprKind::Str(parts) => {
                let mut out = Vec::new();
                for p in parts {
                    match p {
                        StrPart::Lit(b) => out.extend_from_slice(b),
                        StrPart::Interp(..) => return None,
                    }
                }
                Some(out)
            }
            _ => None,
        }
    }
}

/// `int`, `fix` or a record type name.
#[derive(Clone, Debug)]
pub enum TypeName<'s> {
    Int,
    Fix,
    Record(Name<'s>),
}

/// Type and dimensions of a declaration. `dims` entries are `None` for `[]`.
#[derive(Clone, Debug)]
pub struct Shape<'s> {
    pub ty: TypeName<'s>,
    pub dims: Vec<Option<Expr<'s>>>,
    /// The type was written explicitly (`: T`).
    pub explicit: bool,
}

#[derive(Clone, Debug)]
pub enum Init<'s> {
    Expr(Expr<'s>),
    List(Vec<InitItem<'s>>, Pos),
}

impl<'s> Init<'s> {
    pub fn pos(&self) -> Pos {
        match self {
            Init::Expr(e) => e.pos,
            Init::List(_, p) => *p,
        }
    }
}

#[derive(Clone, Debug)]
pub struct InitItem<'s> {
    pub field: Option<Name<'s>>,
    pub value: Init<'s>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssignOp {
    Set,
    Op(BinOp),
}

#[derive(Clone, Debug)]
pub struct Block<'s> {
    pub stmts: Vec<Stmt<'s>>,
    pub end: Pos,
}

#[derive(Clone, Debug)]
pub enum StmtKind<'s> {
    Var {
        name: Name<'s>,
        fix: bool,
        init: Option<Expr<'s>>,
    },
    Assign {
        target: Path<'s>,
        op: AssignOp,
        value: Expr<'s>,
    },
    IncDec {
        target: Path<'s>,
        inc: bool,
    },
    If {
        cond: Expr<'s>,
        then: Block<'s>,
        els: Option<Box<Else<'s>>>,
    },
    While {
        cond: Expr<'s>,
        body: Block<'s>,
    },
    For {
        init: Option<Box<Stmt<'s>>>,
        cond: Option<Expr<'s>>,
        step: Option<Box<Stmt<'s>>>,
        body: Block<'s>,
    },
    Switch {
        value: Expr<'s>,
        cases: Vec<(Vec<Expr<'s>>, Block<'s>)>,
        default: Option<Block<'s>>,
    },
    Break,
    Continue,
    Return(Option<Expr<'s>>),
    Call(Expr<'s>),
}

#[derive(Clone, Debug)]
pub enum Else<'s> {
    If(Stmt<'s>),
    Block(Block<'s>),
}

#[derive(Clone, Debug)]
pub struct Stmt<'s> {
    pub kind: StmtKind<'s>,
    pub pos: Pos,
}

#[derive(Clone, Debug)]
pub struct Param<'s> {
    pub name: Name<'s>,
    pub shape: Shape<'s>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandlerKind {
    Init,
    Update,
    Draw,
    Audio,
}

#[derive(Clone, Debug)]
pub struct Field<'s> {
    pub name: Name<'s>,
    pub shape: Shape<'s>,
}

#[derive(Clone, Debug)]
pub enum Item<'s> {
    Const {
        name: Name<'s>,
        shape: Shape<'s>,
        init: Init<'s>,
    },
    Var {
        name: Name<'s>,
        shape: Shape<'s>,
        init: Option<Init<'s>>,
    },
    AudioVar {
        name: Name<'s>,
        fix: bool,
        init: Expr<'s>,
    },
    Record {
        name: Name<'s>,
        fields: Vec<Field<'s>>,
    },
    Enum {
        name: Name<'s>,
        members: Vec<(Name<'s>, Option<Expr<'s>>)>,
    },
    Sprite {
        name: Name<'s>,
        tile: Expr<'s>,
        size: Option<(Expr<'s>, Expr<'s>)>,
    },
    Fn {
        name: Name<'s>,
        params: Vec<Param<'s>>,
        ret_fix: bool,
        body: Block<'s>,
    },
    Handler {
        kind: HandlerKind,
        pos: Pos,
        params: Vec<Param<'s>>,
        body: Block<'s>,
    },
}

#[derive(Clone, Debug, Default)]
pub struct Program<'s> {
    pub items: Vec<Item<'s>>,
    /// `// @key value` lines before the first declaration.
    pub meta: Vec<(&'s str, &'s str, Pos)>,
}
