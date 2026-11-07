//! Compile-time types and symbols. The VM never sees any of this.

use alloc::string::String;
use alloc::vec::Vec;

use crate::diag::Pos;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Num {
    Int,
    Fix,
}

impl Num {
    pub fn join(a: Num, b: Num) -> Num {
        if a == Num::Fix || b == Num::Fix {
            Num::Fix
        } else {
            Num::Int
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Num::Int => "int",
            Num::Fix => "fix",
        }
    }
}

pub type RecId = usize;

/// Element type of an array or table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Elem {
    Num(Num),
    Rec(RecId),
}

#[derive(Clone, Debug)]
pub enum FieldTy {
    Num(Num),
    Rec(RecId),
}

#[derive(Clone, Debug)]
pub struct FieldDef<'a> {
    pub name: &'a str,
    pub ty: FieldTy,
    pub dims: Vec<u32>,
    /// Product of `dims` (1 without dims).
    pub count: u32,
    pub leaf_off: u16,
    pub leaf_n: u16,
}

/// One flattened numeric field of a record: one array per leaf.
#[derive(Clone, Debug)]
pub struct LeafDef {
    pub num: Num,
    /// Numbers of this leaf per record instance.
    pub mult: u32,
    pub path: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecState {
    Pending,
    Resolving,
    Done,
}

#[derive(Clone, Debug)]
pub struct RecDef<'a> {
    pub name: &'a str,
    pub pos: Pos,
    pub item: usize,
    pub fields: Vec<FieldDef<'a>>,
    pub leaves: Vec<LeafDef>,
    pub state: RecState,
}

impl<'a> RecDef<'a> {
    pub fn field(&self, n: &str) -> Option<&FieldDef<'a>> {
        self.fields.iter().find(|f| f.name == n)
    }
    pub fn has_array_leaf(&self) -> bool {
        self.leaves.iter().any(|l| l.mult > 1)
    }
}

/// A pool array or constant table, possibly the first leaf of a record array.
#[derive(Clone, Debug)]
pub struct ArrSym {
    pub id: u16,
    pub rom: bool,
    pub elem: Elem,
    /// Declared dimensions; empty for a single record.
    pub dims: alloc::boxed::Box<[u32]>,
}

#[derive(Clone, Debug)]
pub enum Sym {
    Const(i32, Num),
    Scalar(u8, Num),
    Audio(u8, Num),
    Arr(ArrSym),
    Func(usize),
    Enum(usize),
    Record(RecId),
}

#[derive(Clone, Debug)]
pub struct EnumDef<'a> {
    pub name: &'a str,
    pub members: Vec<(&'a str, i32)>,
}

/// Kind of a function parameter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamKind {
    Num(Num),
    /// `a[]` or `a: fix[]`: one handle.
    Arr(Num),
    /// `e: Rec`: handle and index.
    Rec(RecId),
    /// `es: Rec[]`: one handle.
    RecArr(RecId),
}

impl ParamKind {
    pub fn slots(self) -> u8 {
        match self {
            ParamKind::Rec(_) => 2,
            _ => 1,
        }
    }
}

#[derive(Clone, Debug)]
pub struct FnSig<'a> {
    pub name: &'a str,
    pub item: usize,
    pub params: Vec<(&'a str, ParamKind)>,
    pub ret: Num,
    /// Per parameter: the function (or something it calls) writes through it.
    pub writes: Vec<bool>,
    pub index: u16,
}

impl<'a> FnSig<'a> {
    pub fn slots(&self) -> u8 {
        self.params.iter().map(|p| p.1.slots()).sum()
    }
}
