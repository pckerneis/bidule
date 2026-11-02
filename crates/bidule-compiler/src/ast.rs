//! Syntax tree.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use crate::diag::Pos;

#[derive(Clone, Debug)]
pub struct Name {
    pub text: String,
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
pub enum StrPart {
    Lit(Vec<u8>),
    Interp(Expr, Spec),
}

#[derive(Clone, Debug)]
pub enum Sel {
    Index(Expr),
    Field(Name),
}

#[derive(Clone, Debug)]
pub struct Path {
    pub root: Name,
    pub sels: Vec<Sel>,
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Int(i32),
    Fix(i32),
    Str(Vec<StrPart>),
    Path(Path),
    Call(Name, Vec<Expr>),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub pos: Pos,
}

impl Expr {
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
pub enum TypeName {
    Int,
    Fix,
    Record(Name),
}

/// Type and dimensions of a declaration. `dims` entries are `None` for `[]`.
#[derive(Clone, Debug)]
pub struct Shape {
    pub ty: TypeName,
    pub dims: Vec<Option<Expr>>,
    /// The type was written explicitly (`: T`).
    pub explicit: bool,
}

#[derive(Clone, Debug)]
pub enum Init {
    Expr(Expr),
    List(Vec<InitItem>, Pos),
}

impl Init {
    pub fn pos(&self) -> Pos {
        match self {
            Init::Expr(e) => e.pos,
            Init::List(_, p) => *p,
        }
    }
}

#[derive(Clone, Debug)]
pub struct InitItem {
    pub field: Option<Name>,
    pub value: Init,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssignOp {
    Set,
    Op(BinOp),
}

#[derive(Clone, Debug)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub end: Pos,
}

#[derive(Clone, Debug)]
pub enum StmtKind {
    Var {
        name: Name,
        fix: bool,
        init: Option<Expr>,
    },
    Assign {
        target: Path,
        op: AssignOp,
        value: Expr,
    },
    IncDec {
        target: Path,
        inc: bool,
    },
    If {
        cond: Expr,
        then: Block,
        els: Option<Box<Else>>,
    },
    While {
        cond: Expr,
        body: Block,
    },
    For {
        init: Option<Box<Stmt>>,
        cond: Option<Expr>,
        step: Option<Box<Stmt>>,
        body: Block,
    },
    Switch {
        value: Expr,
        cases: Vec<(Vec<Expr>, Block)>,
        default: Option<Block>,
    },
    Break,
    Continue,
    Return(Option<Expr>),
    Call(Expr),
}

#[derive(Clone, Debug)]
pub enum Else {
    If(Stmt),
    Block(Block),
}

#[derive(Clone, Debug)]
pub struct Stmt {
    pub kind: StmtKind,
    pub pos: Pos,
}

#[derive(Clone, Debug)]
pub struct Param {
    pub name: Name,
    pub shape: Shape,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandlerKind {
    Init,
    Update,
    Draw,
    Audio,
}

#[derive(Clone, Debug)]
pub struct Field {
    pub name: Name,
    pub shape: Shape,
}

#[derive(Clone, Debug)]
pub enum Item {
    Const {
        name: Name,
        shape: Shape,
        init: Init,
    },
    Var {
        name: Name,
        shape: Shape,
        init: Option<Init>,
    },
    AudioVar {
        name: Name,
        fix: bool,
        init: Expr,
    },
    Record {
        name: Name,
        fields: Vec<Field>,
    },
    Enum {
        name: Name,
        members: Vec<(Name, Option<Expr>)>,
    },
    Sprite {
        name: Name,
        tile: Expr,
        size: Option<(Expr, Expr)>,
    },
    Fn {
        name: Name,
        params: Vec<Param>,
        ret_fix: bool,
        body: Block,
    },
    Handler {
        kind: HandlerKind,
        pos: Pos,
        params: Vec<Param>,
        body: Block,
    },
}

#[derive(Clone, Debug, Default)]
pub struct Program {
    pub items: Vec<Item>,
    /// `// @key value` lines before the first declaration.
    pub meta: Vec<(String, String, Pos)>,
}
