//! Functions, statements and expressions to bytecode.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use bidule_bytecode::builtins::{self, Arg, Ret};
use bidule_bytecode::handle::ID_SHIFT;
use bidule_bytecode::image::{fmt_flags, write, PIECE_ARG, PIECE_END, PIECE_LIT};
use bidule_bytecode::{limits, Op};

use crate::ast::*;
use crate::compile::{Compiler, ASSET_PREFIXES, BUILTIN_CONSTS, INTRINSICS, R};
use crate::diag::{Diag, Pos};
use crate::parser;
use crate::types::*;

// ---------- per-function state ----------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalKind {
    Num(Num),
    Arr(Num),
    /// Handle slot; the index is in the next slot.
    Rec(RecId),
    RecArr(RecId),
}

#[derive(Clone, Debug)]
pub struct LocalVar<'a> {
    pub name: &'a str,
    pub slot: u8,
    pub kind: LocalKind,
}

#[derive(Default, Debug)]
pub struct LoopCtx {
    breaks: Vec<usize>,
    continues: Vec<usize>,
}

#[derive(Default, Debug)]
pub struct FnState<'a> {
    pub locals: Vec<LocalVar<'a>>,
    pub next_slot: u8,
    pub max_slot: u8,
    pub depth: i32,
    pub loops: Vec<LoopCtx>,
    pub audio: bool,
    pub ret: Option<Num>,
    pub name: &'a str,
}

// ---------- places ----------

/// Where an array's elements live.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Base {
    Pool(u16),
    Rom(u16),
    /// A handle in a local (array or record-array parameter).
    Handle(u8),
    /// A record reference: handle in one local, instance index in another.
    RecRef(u8, u8),
}

#[derive(Clone, Copy, Debug)]
pub struct Term<'e, 's> {
    e: &'e Expr<'s>,
    stride: u32,
    bound: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shape {
    Num(Num),
    Rec(RecId),
    /// Remaining dimensions; 0 = unknown (array parameter).
    Arr(Elem, Vec<u32>),
}

/// A part of an array or record: base storage, leaf (record field array) offset,
/// and an index made of `terms` (plus the record-reference index times `scale`),
/// counted in units of the current shape's elements.
#[derive(Clone, Debug)]
pub struct View<'e, 's> {
    base: Base,
    leaf: u16,
    terms: Vec<Term<'e, 's>>,
    scale: u32,
    shape: Shape,
    in_array_field: bool,
    name: &'e str,
}

#[derive(Clone, Debug)]
pub enum Res<'e, 's> {
    Const(i32, Num),
    Local(u8, Num),
    Global(u8, Num),
    Audio(u8, Num),
    View(View<'e, 's>),
    Map(u16),
    MapRow(u16, &'e Expr<'s>),
    MapCell(u16, &'e Expr<'s>, &'e Expr<'s>),
    DynLen(View<'e, 's>),
}

/// What an element address left on the stack.
#[derive(Clone, Copy, Debug)]
enum Slot {
    Local(u8),
    Global(u8),
    Audio(u8),
    Pool1(u8),
    Pool2(u8),
    Rom1(u16),
    Rom2(u16),
    Dyn,
}

/// Static type of an expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ETy {
    Num(Num),
    Text,
    Arr(Elem),
    Rec(RecId),
    Map,
}

impl<'p, 'a> Compiler<'p, 'a> {
    // ---------- emission primitives ----------

    fn pc(&self) -> usize {
        self.code.len()
    }

    fn adjust(&mut self, pop: i32, push: i32) {
        self.f.depth -= pop;
        debug_assert!(self.f.depth >= 0, "stack underflow in codegen");
        self.f.depth += push;
    }

    pub fn op(&mut self, op: Op) {
        self.code.push(op as u8);
        let (p, q) = op.fixed_effect().expect("op with table effect");
        self.adjust(p as i32, q as i32);
    }
    fn op_u8(&mut self, op: Op, v: u8) {
        self.op(op);
        self.code.push(v);
    }
    fn op_u16(&mut self, op: Op, v: u16) {
        self.op(op);
        write::put_u16(&mut self.code, v);
    }
    pub fn push(&mut self, v: i32) {
        if (-128..=127).contains(&v) {
            self.op(Op::PUSH_I8);
            self.code.push(v as i8 as u8);
        } else {
            self.op(Op::PUSH_INT);
            write::put_i32(&mut self.code, v);
        }
    }
    fn jump(&mut self, op: Op) -> usize {
        self.op(op);
        let at = self.pc();
        write::put_u32(&mut self.code, 0);
        at
    }
    fn jump_to(&mut self, op: Op, target: usize) {
        self.op(op);
        write::put_u32(&mut self.code, target as u32);
    }
    fn patch(&mut self, at: usize, target: usize) {
        self.code[at..at + 4].copy_from_slice(&(target as u32).to_le_bytes());
    }
    fn patch_here(&mut self, at: usize) {
        let t = self.pc();
        self.patch(at, t);
    }
    fn call_builtin(&mut self, id: u8, argc: u8) {
        self.code.push(Op::CALL as u8);
        self.code.push(id);
        self.code.push(argc);
        self.adjust(argc as i32, 1);
    }
    fn mark(&mut self, pos: Pos) {
        let pc = self.pc() as u32;
        if let Some(l) = self.lines.last_mut() {
            if l.0 == pc {
                *l = (pc, pos.line as u16, pos.col as u16);
                return;
            }
        }
        self.lines.push((pc, pos.line as u16, pos.col as u16));
    }
    fn promote(&mut self, from: Num, to: Num) {
        if from == Num::Int && to == Num::Fix {
            self.push(16);
            self.op(Op::SHL);
        }
    }
    fn add_const(&mut self, v: i32) {
        if v != 0 {
            self.push(v);
            self.op(Op::ADD);
        }
    }

    // ---------- locals ----------

    fn alloc_local(&mut self, n: u8, pos: Pos) -> R<u8> {
        let s = self.f.next_slot;
        if s as usize + n as usize > limits::LOCALS {
            return Err(Diag::error(
                pos,
                format!("too many local variables (at most {} per function)", limits::LOCALS),
            ));
        }
        self.f.next_slot += n;
        self.f.max_slot = self.f.max_slot.max(self.f.next_slot);
        Ok(s)
    }

    fn find_local(&self, name: &str) -> Option<&LocalVar<'a>> {
        self.f.locals.iter().rev().find(|l| l.name == name)
    }

    fn declare_local(&mut self, name: &Name<'a>, kind: LocalKind, slot: u8) -> R<()> {
        let n = name.text;
        if self.find_local(n).is_some() {
            return Err(Diag::error(name.pos, format!("'{}' is already a local here; pick another name", n)));
        }
        let _ = self.const_sym(n);
        if self.syms.contains_key(n) {
            return Err(Diag::error(
                name.pos,
                format!("'{}' is already declared at top level; locals can't reuse its name", n),
            ));
        }
        if BUILTIN_CONSTS.iter().any(|c| c.0 == n)
            || builtins::by_name(n).is_some()
            || INTRINSICS.contains(&n)
            || ASSET_PREFIXES.contains(&n)
        {
            return Err(Diag::error(name.pos, format!("'{}' is a built-in name; pick another name", n)));
        }
        self.f.locals.push(LocalVar { name: n, slot, kind });
        Ok(())
    }

    // ---------- functions ----------

    pub fn gen_functions(&mut self) {
        let prog = self.prog;
        // user functions first, in declaration order, then handlers
        let mut order: Vec<usize> = Vec::new();
        for (i, it) in prog.items.iter().enumerate() {
            if it.kind == ItemKind::Fn {
                order.push(i);
            }
        }
        let mut seen = [false; 4];
        for (i, it) in prog.items.iter().enumerate() {
            if it.kind != ItemKind::Handler {
                continue;
            }
            if let Ok(Item::Handler { kind, pos, .. }) = parser::item_at(self.source, it) {
                let k = kind as usize;
                if seen[k] {
                    self.error(Diag::error(pos, "this handler is defined twice"));
                    continue;
                }
                seen[k] = true;
                order.push(i);
            }
        }
        for (n, &i) in order.iter().enumerate() {
            let it = &prog.items[i];
            if it.kind == ItemKind::Fn {
                if let Some(Sym::Func(id)) = self.sym(it.name) {
                    let id = *id;
                    self.funcs[id].index = n as u16;
                }
            }
        }
        for i in order {
            let Ok(item) = parser::item_at(self.source, &prog.items[i]) else {
                continue;
            };
            match &item {
                Item::Fn { name, params, body, .. } => {
                    let Some(Sym::Func(id)) = self.sym(name.text).cloned() else {
                        continue;
                    };
                    self.gen_fn(Some(id), name.text, None, params, *body);
                }
                Item::Handler { kind, pos, params, body } => {
                    let max = Self::handler_slot(*kind).max_params();
                    if params.len() > max as usize {
                        self.error(Diag::error(*pos, format!("this handler takes at most {} parameters", max)));
                        continue;
                    }
                    self.handlers[Self::handler_slot(*kind) as usize] = self.fn_entries.len() as u16;
                    let name = match kind {
                        HandlerKind::Init => "init",
                        HandlerKind::Update => "update",
                        HandlerKind::Draw => "draw",
                        HandlerKind::Audio => "audio",
                    };
                    self.gen_fn(None, name, Some(*kind), params, *body);
                }
                _ => {}
            }
        }
    }

    fn gen_fn<'b>(&mut self, id: Option<usize>, name: &'a str, handler: Option<HandlerKind>, params: &'b [Param<'a>], body: BodyPos) {
        let start = self.pc() as u32;
        self.f = FnState {
            name,
            audio: handler == Some(HandlerKind::Audio),
            ..Default::default()
        };
        self.f.ret = match id {
            Some(i) => Some(self.funcs[i].ret),
            None => None,
        };
        let mut nparams = 0u8;
        for (k, p) in params.iter().enumerate() {
            let kind = match id {
                Some(i) => self.funcs[i].params[k].1,
                None => {
                    if !p.shape.dims.is_empty() || p.shape.explicit && !matches!(p.shape.ty, TypeName::Int) {
                        self.error(Diag::error(p.name.pos, "handler parameters are ints"));
                    }
                    ParamKind::Num(Num::Int)
                }
            };
            let (lk, n) = match kind {
                ParamKind::Num(x) => (LocalKind::Num(x), 1),
                ParamKind::Arr(x) => (LocalKind::Arr(x), 1),
                ParamKind::Rec(r) => (LocalKind::Rec(r), 2),
                ParamKind::RecArr(r) => (LocalKind::RecArr(r), 1),
            };
            let slot = match self.alloc_local(n, p.name.pos) {
                Ok(s) => s,
                Err(d) => {
                    self.error(d);
                    return;
                }
            };
            if let Err(d) = self.declare_local(&p.name, lk, slot) {
                self.error(d);
            }
            nparams += n;
        }
        self.mark(body.end);
        self.body(body);
        // falling off the end returns 0
        self.mark(body.end);
        self.push(0);
        self.op(Op::RET);
        self.fn_entries.push(write::FnEntry {
            start,
            params: nparams,
            locals: self.f.max_slot,
            max_stack: limits::MAX_STACK as u8,
        });
        self.fn_names.push(name);
        if self.pc() as u32 - start > limits::FN_CODE_BYTES as u32 {
            self.error(Diag::error(
                body.end,
                format!(
                    "'{}' compiles to more than {} bytes; split it into smaller functions",
                    name,
                    limits::FN_CODE_BYTES
                ),
            ));
        }
    }

    // ---------- statements ----------

    fn block<'b>(&mut self, b: &'b Block<'a>) {
        let save_locals = self.f.locals.len();
        let save_slot = self.f.next_slot;
        for s in &b.stmts {
            self.block_stmt(s);
        }
        self.f.locals.truncate(save_locals);
        self.f.next_slot = save_slot;
    }

    /// Grows the code and the line table by about an eighth when they run
    /// low, instead of letting them double: they hold the whole program.
    fn room(&mut self) {
        // more than most statements emit; a longer one just doubles the vector
        const SLACK: usize = 256;
        if self.code.capacity() - self.code.len() < SLACK {
            self.code.reserve_exact(self.code.len() / 8 + 2 * SLACK);
        }
        if self.lines.capacity() - self.lines.len() < 4 {
            self.lines.reserve_exact(self.lines.len() / 8 + 32);
        }
    }

    /// A function body, read from the source one statement at a time so only
    /// that statement's syntax tree is in memory.
    fn body(&mut self, body: BodyPos) {
        let save_locals = self.f.locals.len();
        let save_slot = self.f.next_slot;
        // the whole source parsed: reading the body again can't fail
        if let Ok(mut r) = parser::BodyReader::new(self.source, body) {
            while let Ok(Some(s)) = r.next_stmt() {
                self.block_stmt(&s);
            }
        }
        self.f.locals.truncate(save_locals);
        self.f.next_slot = save_slot;
    }

    fn block_stmt<'b>(&mut self, s: &'b Stmt<'a>) {
        self.room();
        if let Err(d) = self.stmt(s) {
            self.error(d);
            // keep the stack model consistent after an error
            self.f.depth = 0;
        }
        debug_assert!(
            self.diags.iter().any(|d| d.is_error()) || self.f.depth == 0,
            "stack not empty after statement at {:?}",
            s.pos
        );
    }

    fn stmt<'b>(&mut self, s: &'b Stmt<'a>) -> R<()> {
        self.mark(s.pos);
        match &s.kind {
            StmtKind::Var { name, fix, init } => {
                let num = if *fix { Num::Fix } else { Num::Int };
                match init {
                    Some(e) => self.expr_as(e, num)?,
                    None => self.push(0),
                }
                let slot = self.alloc_local(1, name.pos)?;
                self.op_u8(Op::LOCAL_STORE, slot);
                self.declare_local(name, LocalKind::Num(num), slot)?;
            }
            StmtKind::Assign { target, op, value } => self.assign(target, *op, value, s.pos)?,
            StmtKind::IncDec { target, inc } => self.incdec(target, *inc, s.pos)?,
            StmtKind::If { cond, then, els } => {
                self.cond(cond)?;
                let jz = self.jump(Op::JZ);
                self.block(then);
                match els {
                    None => self.patch_here(jz),
                    Some(e) => {
                        let jend = self.jump(Op::JMP);
                        self.patch_here(jz);
                        match &**e {
                            Else::If(st) => self.stmt(st)?,
                            Else::Block(b) => self.block(b),
                        }
                        self.patch_here(jend);
                    }
                }
            }
            StmtKind::While { cond, body } => {
                let top = self.pc();
                self.cond(cond)?;
                let jz = self.jump(Op::JZ);
                self.f.loops.push(LoopCtx::default());
                self.block(body);
                self.jump_to(Op::JMP, top);
                let l = self.f.loops.pop().unwrap();
                self.patch_here(jz);
                let end = self.pc();
                for b in l.breaks {
                    self.patch(b, end);
                }
                for c in l.continues {
                    self.patch(c, top);
                }
            }
            StmtKind::For { init, cond, step, body } => {
                let save_locals = self.f.locals.len();
                let save_slot = self.f.next_slot;
                let r = (|| -> R<()> {
                    if let Some(i) = init {
                        self.stmt(i)?;
                    }
                    let top = self.pc();
                    let jz = match cond {
                        Some(c) => {
                            self.mark(c.pos);
                            self.cond(c)?;
                            Some(self.jump(Op::JZ))
                        }
                        None => None,
                    };
                    self.f.loops.push(LoopCtx::default());
                    self.block(body);
                    let cont = self.pc();
                    if let Some(st) = step {
                        self.stmt(st)?;
                    }
                    self.jump_to(Op::JMP, top);
                    let l = self.f.loops.pop().unwrap();
                    if let Some(j) = jz {
                        self.patch_here(j);
                    }
                    let end = self.pc();
                    for b in l.breaks {
                        self.patch(b, end);
                    }
                    for c in l.continues {
                        self.patch(c, cont);
                    }
                    Ok(())
                })();
                self.f.locals.truncate(save_locals);
                self.f.next_slot = save_slot;
                r?;
            }
            StmtKind::Switch { value, cases, default } => self.switch(value, cases, default.as_ref(), s.pos)?,
            StmtKind::Break | StmtKind::Continue => {
                if self.f.loops.is_empty() {
                    return Err(Diag::error(s.pos, "break and continue belong inside a loop"));
                }
                let j = self.jump(Op::JMP);
                let l = self.f.loops.last_mut().unwrap();
                if matches!(s.kind, StmtKind::Break) {
                    l.breaks.push(j)
                } else {
                    l.continues.push(j)
                }
            }
            StmtKind::Return(v) => {
                match (v, self.f.ret) {
                    (Some(e), Some(want)) => {
                        let t = self.expr(e)?;
                        if t == Num::Fix && want == Num::Int {
                            return Err(Diag::error(
                                e.pos,
                                format!(
                                    "'{}' returns int; declare it 'fn {}(...): fix' or return int(x)",
                                    self.f.name, self.f.name
                                ),
                            ));
                        }
                        self.promote(t, want);
                    }
                    (Some(e), None) => {
                        let t = self.expr(e)?;
                        if t == Num::Fix && self.f.audio {
                            return Err(Diag::error(e.pos, "audio(t) returns an int sample (0–255); use int(x)"));
                        }
                    }
                    (None, _) => self.push(0),
                }
                self.op(Op::RET);
                self.f.depth = 0;
            }
            StmtKind::Call(e) => {
                let ExprKind::Call(name, args) = &e.kind else { unreachable!() };
                self.call(name, args, e.pos)?;
                self.op(Op::POP);
            }
        }
        Ok(())
    }

    /// A condition: any number (fix compares to 0 like int).
    fn cond<'b>(&mut self, e: &'b Expr<'a>) -> R<()> {
        self.expr(e)?;
        Ok(())
    }

    fn switch<'b>(
        &mut self,
        value: &'b Expr<'a>,
        cases: &'b [(Vec<Expr<'a>>, Block<'a>)],
        default: Option<&'b Block<'a>>,
        pos: Pos,
    ) -> R<()> {
        let t = self.expr(value)?;
        let tmp = self.alloc_local(1, pos)?;
        self.op_u8(Op::LOCAL_STORE, tmp);
        let mut seen: BTreeMap<i32, Pos> = BTreeMap::new();
        let mut enum_of: Option<usize> = None;
        let mut all_enum = true;
        let mut jumps = Vec::new();
        for (vals, _) in cases {
            let mut js = Vec::new();
            for v in vals {
                let (c, ct) = self
                    .const_eval(v)?
                    .ok_or_else(|| Diag::error(v.pos, "case values must be constants"))?;
                if ct == Num::Fix && t == Num::Int {
                    return Err(Diag::error(v.pos, "a fix case value can't match an int"));
                }
                let c = if ct == Num::Int && t == Num::Fix { c.wrapping_shl(16) } else { c };
                if let Some(p) = seen.insert(c, v.pos) {
                    return Err(Diag::error(v.pos, format!("duplicate case value (already on line {})", p.line)));
                }
                match self.enum_member(v) {
                    Some(e) if enum_of.is_none_or(|x| x == e) => enum_of = Some(e),
                    _ => all_enum = false,
                }
                self.op_u8(Op::LOCAL_LOAD, tmp);
                self.push(c);
                self.op(Op::EQ);
                js.push(self.jump(Op::JNZ));
            }
            jumps.push(js);
        }
        if default.is_none() && all_enum {
            if let Some(e) = enum_of {
                let missing: Vec<&str> = self.enums[e]
                    .members
                    .iter()
                    .filter(|m| !seen.contains_key(&m.1))
                    .map(|m| m.0)
                    .collect();
                if !missing.is_empty() {
                    let name = self.enums[e].name;
                    let list: Vec<String> = missing.iter().map(|m| format!("{}.{}", name, m)).collect();
                    self.warn(pos, format!("switch doesn't handle {}; add them or an else", list.join(", ")));
                }
            }
        }
        let to_default = self.jump(Op::JMP);
        let mut ends = Vec::new();
        for ((_, body), js) in cases.iter().zip(jumps) {
            for j in js {
                self.patch_here(j);
            }
            self.block(body);
            ends.push(self.jump(Op::JMP));
        }
        self.patch_here(to_default);
        if let Some(b) = default {
            self.block(b);
        }
        for e in ends {
            self.patch_here(e);
        }
        Ok(())
    }

    fn enum_member(&self, e: &Expr<'a>) -> Option<usize> {
        if let ExprKind::Path(p) = &e.kind {
            if p.sels.len() == 1 {
                if let Some(Sym::Enum(id)) = self.sym(p.root.text) {
                    return Some(*id);
                }
            }
        }
        None
    }

    // ---------- assignment ----------

    fn check_scalar_store(&mut self, r: &Res, pos: Pos) -> R<()> {
        match r {
            Res::Global(..) if self.f.audio => Err(Diag::error(pos, "audio(t) can read globals but not write them")),
            Res::Audio(..) if !self.f.audio => Err(Diag::error(pos, "audio variables belong to audio(t)")),
            Res::Const(..) => Err(Diag::error(pos, "constants can't be assigned")),
            Res::Map(_) | Res::MapRow(..) | Res::MapCell(..) => {
                Err(Diag::error(pos, "maps are read-only; copy the cells you change into a var array"))
            }
            Res::DynLen(_) => Err(Diag::error(pos, ".length can't be assigned")),
            Res::View(v) if matches!(v.base, Base::Rom(_)) => {
                Err(Diag::error(pos, format!("'{}' is a constant table and is read-only", v.name)))
            }
            Res::View(v) if matches!(v.shape, Shape::Arr(..)) => {
                Err(Diag::error(pos, "arrays can't be assigned; use copy(dst, src) or a loop"))
            }
            _ => Ok(()),
        }
    }

    fn assign<'b>(&mut self, target: &'b Path<'a>, op: AssignOp, value: &'b Expr<'a>, pos: Pos) -> R<()> {
        let r = self.resolve(target)?;
        self.check_scalar_store(&r, target.root.pos)?;
        if let Res::View(v) = &r {
            if let Shape::Rec(rec) = v.shape {
                if op != AssignOp::Set {
                    return Err(Diag::error(pos, "records can only be copied with '='"));
                }
                return self.record_copy(v.clone(), rec, value, pos);
            }
        }
        let (slot, num) = self.address(&r, pos)?;
        match op {
            AssignOp::Set => {
                let t = self.expr(value)?;
                self.store_conv(t, num, value)?;
            }
            AssignOp::Op(bop) => {
                self.dup_addr(slot);
                self.load(slot);
                let t = self.ty(value)?;
                let ETy::Num(t) = t else {
                    return Err(Diag::error(value.pos, "expected a number"));
                };
                if num == Num::Int && t == Num::Fix {
                    return Err(Diag::error(value.pos, "the result would be fix, stored in an int; use int(x)"));
                }
                if bop.is_bitwise() && num == Num::Fix || bop.is_bitwise() && t == Num::Fix {
                    return Err(Diag::error(
                        pos,
                        format!("'{}' needs int operands; convert with int(x)", bop.text()),
                    ));
                }
                self.expr(value)?;
                self.promote(t, num);
                self.arith(bop, num);
            }
        }
        self.store(slot);
        Ok(())
    }

    /// Converts the value on the stack (of type `t`) for a store of type `want`.
    fn store_conv<'b>(&mut self, t: Num, want: Num, value: &'b Expr<'a>) -> R<()> {
        if t == Num::Fix && want == Num::Int {
            return Err(Diag::error(
                value.pos,
                "a fix value can't be stored in an int; use int(x) or round(x)",
            ));
        }
        if t == Num::Int && want == Num::Fix {
            self.check_int_div(value, want);
        }
        self.promote(t, want);
        Ok(())
    }

    /// Warns when `int / int` lands in a fix: the division truncates first.
    pub fn check_int_div<'b>(&mut self, e: &'b Expr<'a>, want: Num) {
        if want != Num::Fix {
            return;
        }
        if let ExprKind::Binary(BinOp::Div, a, b) = &e.kind {
            if let (Ok(Some((x, Num::Int))), Ok(Some((y, Num::Int)))) = (self.const_eval(a), self.const_eval(b)) {
                if y != 0 && x % y == 0 {
                    return;
                }
            }
            if matches!(self.ty(a), Ok(ETy::Num(Num::Int))) && matches!(self.ty(b), Ok(ETy::Num(Num::Int))) {
                self.warn(
                    e.pos,
                    "integer division happens before the value becomes fix; write a fix operand (7.0 / 2) to keep the fraction",
                );
            }
        }
    }

    fn incdec<'b>(&mut self, target: &'b Path<'a>, inc: bool, pos: Pos) -> R<()> {
        let r = self.resolve(target)?;
        self.check_scalar_store(&r, target.root.pos)?;
        if let Res::View(v) = &r {
            if matches!(v.shape, Shape::Rec(_)) {
                return Err(Diag::error(pos, "'++' and '--' apply to numbers"));
            }
        }
        let (slot, num) = self.address(&r, pos)?;
        self.dup_addr(slot);
        self.load(slot);
        self.push(if num == Num::Fix { 65536 } else { 1 });
        self.op(if inc { Op::ADD } else { Op::SUB });
        self.store(slot);
        Ok(())
    }

    fn dup_addr(&mut self, s: Slot) {
        match s {
            Slot::Pool1(_) | Slot::Rom1(_) => self.op(Op::DUP),
            Slot::Pool2(_) | Slot::Rom2(_) | Slot::Dyn => self.op(Op::DUP2),
            _ => {}
        }
    }

    fn load(&mut self, s: Slot) {
        match s {
            Slot::Local(n) => self.op_u8(Op::LOCAL_LOAD, n),
            Slot::Global(n) => self.op_u8(Op::GLOBAL_LOAD, n),
            Slot::Audio(n) => self.op_u8(Op::AUDIO_LOAD, n),
            Slot::Pool1(n) => self.op_u8(Op::ARR_GET, n),
            Slot::Pool2(n) => self.op_u8(Op::ARR_GET2, n),
            Slot::Rom1(n) => self.op_u16(Op::ROM_GET, n),
            Slot::Rom2(n) => self.op_u16(Op::ROM_GET2, n),
            Slot::Dyn => self.op(Op::ARR_GET_DYN),
        }
    }

    fn store(&mut self, s: Slot) {
        match s {
            Slot::Local(n) => self.op_u8(Op::LOCAL_STORE, n),
            Slot::Global(n) => self.op_u8(Op::GLOBAL_STORE, n),
            Slot::Audio(n) => self.op_u8(Op::AUDIO_STORE, n),
            Slot::Pool1(n) => self.op_u8(Op::ARR_SET, n),
            Slot::Pool2(n) => self.op_u8(Op::ARR_SET2, n),
            Slot::Dyn => self.op(Op::ARR_SET_DYN),
            Slot::Rom1(_) | Slot::Rom2(_) => unreachable!("store to ROM rejected earlier"),
        }
    }

    /// Emits the address of a numeric place.
    fn address<'b>(&mut self, r: &Res<'b, 'a>, pos: Pos) -> R<(Slot, Num)> {
        match r {
            Res::Local(s, n) => Ok((Slot::Local(*s), *n)),
            Res::Global(s, n) => Ok((Slot::Global(*s), *n)),
            Res::Audio(s, n) => {
                if !self.f.audio {
                    return Err(Diag::error(pos, "audio variables belong to audio(t)"));
                }
                Ok((Slot::Audio(*s), *n))
            }
            Res::View(v) => match v.shape {
                Shape::Num(n) => Ok((self.elem_addr(v)?, n)),
                Shape::Rec(_) => Err(Diag::error(pos, format!("'{}' is a record; use one of its fields", v.name))),
                Shape::Arr(..) => Err(Diag::error(pos, format!("'{}' is an array; index it", v.name))),
            },
            _ => Err(Diag::error(pos, "not a variable")),
        }
    }

    fn record_copy<'b>(&mut self, dst: View<'b, 'a>, rec: RecId, value: &'b Expr<'a>, pos: Pos) -> R<()> {
        let ExprKind::Path(p) = &value.kind else {
            return Err(Diag::error(value.pos, format!("expected a {} to copy", self.records[rec].name)));
        };
        let src = match self.resolve(p)? {
            Res::View(v) if v.shape == Shape::Rec(rec) => v,
            _ => return Err(Diag::error(value.pos, format!("expected a {} to copy", self.records[rec].name))),
        };
        let tmp = self.alloc_local(4, pos)?;
        self.rec_ref(&dst)?;
        self.op_u8(Op::LOCAL_STORE, tmp + 1);
        self.op_u8(Op::LOCAL_STORE, tmp);
        self.rec_ref(&src)?;
        self.op_u8(Op::LOCAL_STORE, tmp + 3);
        self.op_u8(Op::LOCAL_STORE, tmp + 2);
        let leaves = self.records[rec].leaves.clone();
        for (k, l) in leaves.iter().enumerate() {
            for j in 0..l.mult {
                for (h, i) in [(tmp, tmp + 1), (tmp + 2, tmp + 3)] {
                    self.op_u8(Op::LOCAL_LOAD, h);
                    self.add_const((k as i32) << ID_SHIFT);
                    self.op_u8(Op::LOCAL_LOAD, i);
                    if l.mult > 1 {
                        self.push(l.mult as i32);
                        self.op(Op::MUL);
                    }
                    self.add_const(j as i32);
                    if h == tmp + 2 {
                        self.op(Op::ARR_GET_DYN);
                    }
                }
                self.op(Op::ARR_SET_DYN);
            }
        }
        self.f.next_slot -= 4;
        Ok(())
    }

    // ---------- resolving names ----------

    pub fn resolve<'b>(&mut self, p: &'b Path<'a>) -> R<Res<'b, 'a>> {
        let name = p.root.text;
        let mut cur: Res<'b, 'a> = if let Some(l) = self.find_local(name).cloned() {
            match l.kind {
                LocalKind::Num(n) => Res::Local(l.slot, n),
                LocalKind::Arr(n) => Res::View(View {
                    base: Base::Handle(l.slot),
                    leaf: 0,
                    terms: Vec::new(),
                    scale: 0,
                    shape: Shape::Arr(Elem::Num(n), vec![0]),
                    in_array_field: false,
                    name,
                }),
                LocalKind::Rec(r) => Res::View(View {
                    base: Base::RecRef(l.slot, l.slot + 1),
                    leaf: 0,
                    terms: Vec::new(),
                    scale: 1,
                    shape: Shape::Rec(r),
                    in_array_field: false,
                    name,
                }),
                LocalKind::RecArr(r) => Res::View(View {
                    base: Base::Handle(l.slot),
                    leaf: 0,
                    terms: Vec::new(),
                    scale: 0,
                    shape: Shape::Arr(Elem::Rec(r), vec![0]),
                    in_array_field: false,
                    name,
                }),
            }
        } else {
            let sym = match self.const_sym(name)? {
                Some(s) => Some(s),
                None => self.sym(name).cloned(),
            };
            match sym {
                Some(Sym::Const(v, n)) => Res::Const(v, n),
                Some(Sym::Scalar(s, n)) => Res::Global(s, n),
                Some(Sym::Audio(s, n)) => Res::Audio(s, n),
                Some(Sym::Arr(a)) => {
                    let base = if a.rom { Base::Rom(a.id) } else { Base::Pool(a.id) };
                    let shape = match a.elem {
                        Elem::Rec(r) if a.dims.is_empty() => Shape::Rec(r),
                        e => Shape::Arr(e, a.dims.to_vec()),
                    };
                    Res::View(View {
                        base,
                        leaf: 0,
                        terms: Vec::new(),
                        scale: 0,
                        shape,
                        in_array_field: false,
                        name,
                    })
                }
                Some(Sym::Enum(e)) => {
                    let Some(Sel::Field(m)) = p.sels.first() else {
                        return Err(Diag::error(
                            p.root.pos,
                            format!("'{}' is an enum; name a member: {}.MEMBER", name, name),
                        ));
                    };
                    let def = &self.enums[e];
                    let v = def.members.iter().find(|x| x.0 == m.text).map(|x| x.1);
                    let Some(v) = v else {
                        let cands: Vec<&str> = def.members.iter().map(|x| x.0).collect();
                        return Err(match crate::diag::suggest(m.text, cands.iter().copied()) {
                            Some(s) => Diag::error(m.pos, format!("enum {} has no member '{}' — did you mean '{}'?", name, m.text, s)),
                            None => Diag::error(m.pos, format!("enum {} has no member '{}'", name, m.text)),
                        });
                    };
                    return self.finish_const(p, 1, v);
                }
                Some(Sym::Func(_)) => return Err(Diag::error(p.root.pos, format!("'{}' is a function; call it with ()", name))),
                Some(Sym::Record(_)) => return Err(Diag::error(p.root.pos, format!("'{}' is a record type, not a variable", name))),
                None => {
                    if let Some(c) = BUILTIN_CONSTS.iter().find(|c| c.0 == name) {
                        Res::Const(c.1, Num::Int)
                    } else if let Some(k) = ASSET_PREFIXES.iter().position(|x| *x == name) {
                        return self.resolve_asset(p, k);
                    } else if builtins::by_name(name).is_some() || INTRINSICS.contains(&name) {
                        return Err(Diag::error(
                            p.root.pos,
                            format!("'{}' is a built-in function; call it with ()", name),
                        ));
                    } else {
                        let locals: Vec<&str> = self.f.locals.iter().map(|l| l.name).collect();
                        return Err(self.unknown("name", &p.root, &locals));
                    }
                }
            }
        };
        for (k, sel) in p.sels.iter().enumerate() {
            cur = self.select(cur, sel, k + 1 == p.sels.len())?;
        }
        Ok(cur)
    }

    fn finish_const<'b>(&self, p: &'b Path<'a>, used: usize, v: i32) -> R<Res<'b, 'a>> {
        if let Some(s) = p.sels.get(used) {
            let pos = match s {
                Sel::Index(e) => e.pos,
                Sel::Field(n) => n.pos,
            };
            return Err(Diag::error(pos, "a constant has no fields or elements"));
        }
        Ok(Res::Const(v, Num::Int))
    }

    fn resolve_asset<'b>(&mut self, p: &'b Path<'a>, k: usize) -> R<Res<'b, 'a>> {
        let prefix = ASSET_PREFIXES[k];
        let Some(Sel::Field(n)) = p.sels.first() else {
            return Err(Diag::error(p.root.pos, format!("name an asset: {}.NAME", prefix)));
        };
        let Some(sec) = self.assets[k] else {
            return Err(Diag::error(n.pos, format!("this cart has no {} assets", prefix.to_lowercase())));
        };
        let mut found = None;
        let mut names: Vec<&str> = Vec::new();
        for i in 0..sec.slots {
            let nm = sec.name(i);
            if nm == n.text.as_bytes() {
                found = Some(i);
            }
            if let Ok(s) = core::str::from_utf8(nm) {
                names.push(s);
            }
        }
        let Some(slot) = found else {
            return Err(match crate::diag::suggest(n.text, names.iter().copied()) {
                Some(s) => Diag::error(
                    n.pos,
                    format!("no asset {}.{} in this cart — did you mean {}.{}?", prefix, n.text, prefix, s),
                ),
                None => Diag::error(n.pos, format!("no asset {}.{} in this cart", prefix, n.text)),
            });
        };
        if sec.slot(slot).is_empty() {
            return Err(Diag::error(n.pos, format!("asset {}.{} was emptied", prefix, n.text)));
        }
        if prefix != "Maps" {
            return self.finish_const(p, 1, slot as i32);
        }
        let mut cur = Res::Map(slot);
        for (j, sel) in p.sels[1..].iter().enumerate() {
            cur = self.select(cur, sel, j + 2 == p.sels.len())?;
        }
        Ok(cur)
    }

    fn select<'b>(&mut self, cur: Res<'b, 'a>, sel: &'b Sel<'a>, last: bool) -> R<Res<'b, 'a>> {
        match (cur, sel) {
            (Res::Map(m), Sel::Index(e)) => Ok(Res::MapRow(m, e)),
            (Res::MapRow(m, y), Sel::Index(e)) => Ok(Res::MapCell(m, y, e)),
            (Res::View(v), Sel::Field(f)) if f.text == "length" => {
                let Shape::Arr(_, dims) = &v.shape else {
                    return Err(Diag::error(f.pos, ".length applies to arrays"));
                };
                if !last {
                    return Err(Diag::error(f.pos, ".length is a number"));
                }
                if dims[0] > 0 {
                    return Ok(Res::Const(dims[0] as i32, Num::Int));
                }
                Ok(Res::DynLen(v))
            }
            (Res::View(mut v), Sel::Field(f)) => {
                let Shape::Rec(r) = v.shape else {
                    return Err(Diag::error(f.pos, format!("'{}' has no fields", v.name)));
                };
                let rec = &self.records[r];
                let Some(fd) = rec.field(f.text).cloned() else {
                    let cands: Vec<&str> = rec.fields.iter().map(|x| x.name).collect();
                    return Err(match crate::diag::suggest(f.text, cands.iter().copied()) {
                        Some(s) => Diag::error(f.pos, format!("{} has no field '{}' — did you mean '{}'?", rec.name, f.text, s)),
                        None => Diag::error(f.pos, format!("{} has no field '{}'", rec.name, f.text)),
                    });
                };
                v.leaf += fd.leaf_off;
                let elem = match fd.ty {
                    FieldTy::Num(n) => Elem::Num(n),
                    FieldTy::Rec(s) => Elem::Rec(s),
                };
                if fd.dims.is_empty() {
                    v.shape = match elem {
                        Elem::Num(n) => Shape::Num(n),
                        Elem::Rec(s) => Shape::Rec(s),
                    };
                } else {
                    for t in v.terms.iter_mut() {
                        t.stride *= fd.count;
                    }
                    v.scale *= fd.count;
                    v.shape = Shape::Arr(elem, fd.dims.clone());
                    v.in_array_field = true;
                }
                Ok(Res::View(v))
            }
            (Res::View(mut v), Sel::Index(e)) => {
                let Shape::Arr(elem, dims) = v.shape.clone() else {
                    return Err(Diag::error(e.pos, format!("'{}' is not an array", v.name)));
                };
                let inner: u32 = dims[1..].iter().product();
                v.terms.push(Term {
                    e,
                    stride: inner,
                    bound: if dims[0] > 0 { Some(dims[0]) } else { None },
                });
                v.shape = if dims.len() > 1 {
                    Shape::Arr(elem, dims[1..].to_vec())
                } else {
                    match elem {
                        Elem::Num(n) => Shape::Num(n),
                        Elem::Rec(r) => Shape::Rec(r),
                    }
                };
                Ok(Res::View(v))
            }
            (Res::Local(..) | Res::Global(..) | Res::Audio(..) | Res::Const(..), s) => {
                let pos = match s {
                    Sel::Index(e) => e.pos,
                    Sel::Field(n) => n.pos,
                };
                Err(Diag::error(pos, "a number has no fields or elements"))
            }
            (_, s) => {
                let pos = match s {
                    Sel::Index(e) => e.pos,
                    Sel::Field(n) => n.pos,
                };
                Err(Diag::error(pos, "unexpected selector"))
            }
        }
    }

    // ---------- emitting places ----------

    /// (len, cols) of a static leaf array or table.
    fn static_info(&self, base: Base, leaf: u16) -> Option<(u32, u32)> {
        match base {
            Base::Pool(id) => self.arrays.get((id + leaf) as usize).map(|a| (a.len as u32, a.cols as u32)),
            Base::Rom(id) => self
                .tables
                .get((id + leaf) as usize)
                .map(|t| (t.values.len() as u32, t.cols as u32)),
            _ => None,
        }
    }

    /// Pushes the flat index (in current units) of a view.
    fn emit_index<'b>(&mut self, v: &View<'b, 'a>, div: u32) -> R<()> {
        let parts = v.terms.len() + (v.scale > 0) as usize;
        let check = parts > 1;
        let mut first = true;
        if let Base::RecRef(_, i) = v.base {
            self.op_u8(Op::LOCAL_LOAD, i);
            if v.scale / div != 1 {
                self.push((v.scale / div) as i32);
                self.op(Op::MUL);
            }
            first = false;
        }
        for t in &v.terms {
            self.expr_as(t.e, Num::Int)?;
            if let (true, Some(b)) = (check, t.bound) {
                self.op_u16(Op::CHK, b.min(65535) as u16);
            }
            if t.stride / div != 1 {
                self.push((t.stride / div) as i32);
                self.op(Op::MUL);
            }
            if !first {
                self.op(Op::ADD);
            }
            first = false;
        }
        if first {
            self.push(0);
        }
        Ok(())
    }

    /// Pushes the handle of a view's base array plus its leaf.
    fn emit_base_handle<'b>(&mut self, v: &View<'b, 'a>) {
        match v.base {
            Base::Pool(id) => self.op_u8(Op::PUSH_ARR, (id + v.leaf) as u8),
            Base::Rom(id) => self.op_u16(Op::PUSH_ROM, id + v.leaf),
            Base::Handle(s) | Base::RecRef(s, _) => {
                self.op_u8(Op::LOCAL_LOAD, s);
                self.add_const((v.leaf as i32) << ID_SHIFT);
            }
        }
    }

    fn elem_addr<'b>(&mut self, v: &View<'b, 'a>) -> R<Slot> {
        match v.base {
            Base::Pool(id) | Base::Rom(id) => {
                let rom = matches!(v.base, Base::Rom(_));
                let a = id + v.leaf;
                let (_, cols) = self.static_info(v.base, v.leaf).unwrap();
                if cols > 0 && v.terms.len() == 2 && v.terms[0].stride == cols && v.terms[1].stride == 1 {
                    self.expr_as(v.terms[0].e, Num::Int)?;
                    self.expr_as(v.terms[1].e, Num::Int)?;
                    return Ok(if rom { Slot::Rom2(a) } else { Slot::Pool2(a as u8) });
                }
                self.emit_index(v, 1)?;
                Ok(if rom { Slot::Rom1(a) } else { Slot::Pool1(a as u8) })
            }
            Base::Handle(_) | Base::RecRef(..) => {
                self.emit_base_handle(v);
                self.emit_index(v, 1)?;
                Ok(Slot::Dyn)
            }
        }
    }

    /// Pushes a handle to a one-dimensional array view (a whole 1D array or a row).
    fn arr_handle<'b>(&mut self, v: &View<'b, 'a>, pos: Pos) -> R<()> {
        let Shape::Arr(Elem::Num(_), dims) = &v.shape else {
            return Err(Diag::error(pos, format!("'{}' is not an array of numbers", v.name)));
        };
        if dims.len() != 1 {
            return Err(Diag::error(
                pos,
                format!("'{}' is two-dimensional; pass one row: {}[i]", v.name, v.name),
            ));
        }
        let row_len = dims[0];
        let whole = v.terms.is_empty() && v.scale == 0;
        let two_d = match v.base {
            Base::Pool(_) | Base::Rom(_) => self.static_info(v.base, v.leaf).unwrap().1 > 0,
            // a record field array (from a parameter) is a row of the caller's leaf array
            Base::RecRef(..) => true,
            Base::Handle(_) => !whole || v.leaf > 0 || v.in_array_field,
        };
        if !two_d {
            if !whole {
                return Err(Diag::error(pos, "can't refer to this part of an array"));
            }
            self.emit_base_handle(v);
            return Ok(());
        }
        if v.in_array_field && row_len == 0 {
            return Err(Diag::error(pos, "can't refer to this part of an array"));
        }
        if v.terms.iter().any(|t| t.stride % row_len.max(1) != 0) || !v.scale.is_multiple_of(row_len.max(1)) {
            return Err(Diag::error(pos, "can't refer to this part of an array"));
        }
        self.emit_base_handle(v);
        self.emit_index(v, row_len.max(1))?;
        self.op(Op::ROW);
        Ok(())
    }

    /// Pushes (handle, index) for a record view.
    fn rec_ref<'b>(&mut self, v: &View<'b, 'a>) -> R<()> {
        let Shape::Rec(r) = v.shape else { unreachable!() };
        if v.in_array_field && self.records[r].has_array_leaf() {
            return Err(Diag::error(
                Pos::default(),
                format!(
                    "passing a {} that sits in an array field of another record isn't supported yet",
                    self.records[r].name
                ),
            ));
        }
        self.emit_base_handle(v);
        self.emit_index(v, 1)?;
        Ok(())
    }

    // ---------- types ----------

    pub fn ty<'b>(&mut self, e: &'b Expr<'a>) -> R<ETy> {
        Ok(match &e.kind {
            ExprKind::Int(_) => ETy::Num(Num::Int),
            ExprKind::Fix(_) => ETy::Num(Num::Fix),
            ExprKind::Str(_) => ETy::Text,
            ExprKind::Path(p) => match self.resolve(p)? {
                Res::Const(_, n) | Res::Local(_, n) | Res::Global(_, n) | Res::Audio(_, n) => ETy::Num(n),
                Res::DynLen(_) | Res::MapCell(..) => ETy::Num(Num::Int),
                Res::Map(_) => ETy::Num(Num::Int),
                Res::MapRow(..) => ETy::Map,
                Res::View(v) => match v.shape {
                    Shape::Num(n) => ETy::Num(n),
                    Shape::Rec(r) => ETy::Rec(r),
                    Shape::Arr(el, _) => ETy::Arr(el),
                },
            },
            ExprKind::Call(name, args) => ETy::Num(self.call_type(name, args)?),
            ExprKind::Unary(op, a) => {
                let t = self.num_ty(a)?;
                ETy::Num(if *op == UnOp::Neg { t } else { Num::Int })
            }
            ExprKind::Binary(op, a, b) => {
                let ta = self.num_ty(a)?;
                let tb = self.num_ty(b)?;
                if op.is_compare() || matches!(op, BinOp::And | BinOp::Or) || op.is_bitwise() {
                    ETy::Num(Num::Int)
                } else {
                    ETy::Num(Num::join(ta, tb))
                }
            }
            ExprKind::Cond(_, a, b) => ETy::Num(Num::join(self.num_ty(a)?, self.num_ty(b)?)),
        })
    }

    fn num_ty<'b>(&mut self, e: &'b Expr<'a>) -> R<Num> {
        match self.ty(e)? {
            ETy::Num(n) => Ok(n),
            other => Err(self.not_a_number(e, &other)),
        }
    }

    fn not_a_number(&self, e: &Expr<'a>, t: &ETy) -> Diag {
        let what = match t {
            ETy::Text => "text",
            ETy::Arr(_) => "an array",
            ETy::Rec(_) => "a record",
            ETy::Map => "a map",
            ETy::Num(_) => "a number",
        };
        let hint = match (t, &e.kind) {
            (ETy::Arr(_), _) => "; use streq/arreq to compare, or index it",
            (ETy::Rec(_), _) => "; use one of its fields",
            _ => "",
        };
        Diag::error(e.pos, format!("expected a number, found {}{}", what, hint))
    }

    fn call_type<'b>(&mut self, name: &'b Name<'a>, args: &'b [Expr<'a>]) -> R<Num> {
        let n = name.text;
        match n {
            "int" | "round" => return Ok(Num::Int),
            "frac" => return Ok(Num::Fix),
            _ => {}
        }
        if let Some(Sym::Func(f)) = self.sym(n) {
            return Ok(self.funcs[*f].ret);
        }
        if let Some(b) = builtins::by_name(n) {
            return Ok(match b.ret {
                Ret::Int => Num::Int,
                Ret::Fix => Num::Fix,
                Ret::Same => {
                    let mut t = Num::Int;
                    for a in args {
                        t = Num::join(t, self.num_ty(a)?);
                    }
                    t
                }
            });
        }
        Err(self.unknown("function", name, &INTRINSICS))
    }

    // ---------- expressions ----------

    /// Emits an expression whose value must be of type `want` (int promotes to fix).
    pub fn expr_as<'b>(&mut self, e: &'b Expr<'a>, want: Num) -> R<()> {
        let t = self.expr(e)?;
        self.store_conv(t, want, e)
    }

    /// Emits a numeric expression and returns its type.
    pub fn expr<'b>(&mut self, e: &'b Expr<'a>) -> R<Num> {
        if let Some((v, n)) = self.const_eval(e)? {
            self.push(v);
            return Ok(n);
        }
        match &e.kind {
            ExprKind::Int(v) => {
                self.push(*v);
                Ok(Num::Int)
            }
            ExprKind::Fix(v) => {
                self.push(*v);
                Ok(Num::Fix)
            }
            ExprKind::Str(_) => Err(Diag::error(
                e.pos,
                "text can't be used as a number; pass it to print, fmt or a text parameter",
            )),
            ExprKind::Path(p) => {
                let r = self.resolve(p)?;
                match r {
                    Res::Const(v, n) => {
                        self.push(v);
                        Ok(n)
                    }
                    Res::DynLen(v) => {
                        self.emit_base_handle(&v);
                        self.op(Op::ARR_LEN_DYN);
                        Ok(Num::Int)
                    }
                    Res::MapCell(m, y, x) => {
                        if self.f.audio {
                            return Err(Diag::error(e.pos, "audio(t) can't read maps"));
                        }
                        self.push(m as i32);
                        self.expr_as(x, Num::Int)?;
                        self.expr_as(y, Num::Int)?;
                        self.call_builtin(builtins::id::MAPCELL, 3);
                        Ok(Num::Int)
                    }
                    // a map is its slot index, as for other assets
                    Res::Map(m) => {
                        self.push(m as i32);
                        Ok(Num::Int)
                    }
                    Res::MapRow(..) => Err(Diag::error(e.pos, "read a map cell with Maps.NAME[y][x]")),
                    Res::View(ref v) if !matches!(v.shape, Shape::Num(_)) => {
                        let t = self.ty(e)?;
                        Err(self.not_a_number(e, &t))
                    }
                    Res::View(ref v) if self.f.audio && matches!(v.base, Base::Pool(_)) => Err(Diag::error(
                        e.pos,
                        "audio(t) can't read arrays in RAM; use constant tables or globals",
                    )),
                    r => {
                        let (slot, n) = self.address(&r, e.pos)?;
                        self.load(slot);
                        Ok(n)
                    }
                }
            }
            ExprKind::Call(name, args) => self.call(name, args, e.pos),
            ExprKind::Unary(op, a) => {
                let t = self.expr(a)?;
                match op {
                    UnOp::Neg => {
                        self.op(Op::NEG);
                        Ok(t)
                    }
                    UnOp::Not => {
                        self.op(Op::NOT);
                        Ok(Num::Int)
                    }
                    UnOp::BitNot => {
                        if t == Num::Fix {
                            return Err(Diag::error(e.pos, "'~' needs an int; convert with int(x)"));
                        }
                        self.op(Op::BNOT);
                        Ok(Num::Int)
                    }
                }
            }
            ExprKind::Binary(op @ (BinOp::And | BinOp::Or), a, b) => {
                let d = self.f.depth;
                self.expr(a)?;
                let j = self.jump(if *op == BinOp::And { Op::JZ } else { Op::JNZ });
                self.expr(b)?;
                self.op(Op::BOOL);
                let jend = self.jump(Op::JMP);
                self.patch_here(j);
                self.f.depth = d;
                self.push(if *op == BinOp::And { 0 } else { 1 });
                self.patch_here(jend);
                Ok(Num::Int)
            }
            ExprKind::Binary(op, a, b) => {
                let ta = self.num_ty(a)?;
                let tb = self.num_ty(b)?;
                let t = Num::join(ta, tb);
                if op.is_bitwise() && t == Num::Fix {
                    return Err(Diag::error(
                        e.pos,
                        format!("'{}' needs int operands; convert with int(x)", op.text()),
                    ));
                }
                self.expr(a)?;
                self.promote(ta, t);
                self.expr(b)?;
                self.promote(tb, t);
                self.arith(*op, t);
                Ok(if op.is_compare() { Num::Int } else { t })
            }
            ExprKind::Cond(c, a, b) => {
                let t = Num::join(self.num_ty(a)?, self.num_ty(b)?);
                self.expr(c)?;
                let jz = self.jump(Op::JZ);
                let d = self.f.depth;
                let ta = self.expr(a)?;
                self.promote(ta, t);
                let jend = self.jump(Op::JMP);
                self.patch_here(jz);
                self.f.depth = d;
                let tb = self.expr(b)?;
                self.promote(tb, t);
                self.patch_here(jend);
                Ok(t)
            }
        }
    }

    fn arith(&mut self, op: BinOp, t: Num) {
        let o = match (op, t) {
            (BinOp::Add, _) => Op::ADD,
            (BinOp::Sub, _) => Op::SUB,
            (BinOp::Mul, Num::Int) => Op::MUL,
            (BinOp::Mul, Num::Fix) => Op::FMUL,
            (BinOp::Div, Num::Int) => Op::DIV,
            (BinOp::Div, Num::Fix) => Op::FDIV,
            (BinOp::Mod, _) => Op::MOD,
            (BinOp::Shl, _) => Op::SHL,
            (BinOp::Shr, _) => Op::SHR,
            (BinOp::Lt, _) => Op::LT,
            (BinOp::Le, _) => Op::LE,
            (BinOp::Gt, _) => Op::GT,
            (BinOp::Ge, _) => Op::GE,
            (BinOp::Eq, _) => Op::EQ,
            (BinOp::Ne, _) => Op::NE,
            (BinOp::BitAnd, _) => Op::BAND,
            (BinOp::BitXor, _) => Op::BXOR,
            (BinOp::BitOr, _) => Op::BOR,
            (BinOp::And | BinOp::Or, _) => unreachable!(),
        };
        self.op(o);
    }

    // ---------- calls ----------

    pub fn call<'b>(&mut self, name: &'b Name<'a>, args: &'b [Expr<'a>], pos: Pos) -> R<Num> {
        let n = name.text;
        if INTRINSICS.contains(&n) {
            if args.len() != 1 {
                return Err(Diag::error(pos, format!("{}(x) takes one argument", n)));
            }
            let t = self.expr(&args[0])?;
            return Ok(match (n, t) {
                ("int", Num::Fix) => {
                    self.push(16);
                    self.op(Op::SHR);
                    Num::Int
                }
                ("round", Num::Fix) => {
                    self.push(32768);
                    self.op(Op::ADD);
                    self.push(16);
                    self.op(Op::SHR);
                    Num::Int
                }
                ("frac", Num::Fix) => {
                    self.push(0xFFFF);
                    self.op(Op::BAND);
                    Num::Fix
                }
                ("frac", Num::Int) => {
                    self.op(Op::POP);
                    self.push(0);
                    Num::Fix
                }
                _ => Num::Int,
            });
        }
        if let Some(Sym::Func(f)) = self.sym(n).cloned() {
            return self.call_user(f, name, args, pos);
        }
        let Some(b) = builtins::by_name(n) else {
            if builtins::is_reserved_name(n) {
                return Err(Diag::error(
                    pos,
                    format!("{}() is reserved for a later version and can't be called yet", n),
                ));
            }
            return Err(self.unknown("function", name, &INTRINSICS));
        };
        if self.f.audio && !b.audio_ok {
            return Err(Diag::error(pos, format!("{}() isn't available in audio(t)", n)));
        }
        if (args.len() as u8) < b.min_args || args.len() as u8 > b.max_args() {
            let want = if b.min_args == b.max_args() {
                format!("{}", b.min_args)
            } else {
                format!("{} to {}", b.min_args, b.max_args())
            };
            return Err(Diag::error(pos, format!("{}() takes {} arguments, got {}", n, want, args.len())));
        }
        // a common type for the `Same` arguments
        let mut same = Num::Int;
        for (a, k) in args.iter().zip(b.args) {
            if *k == Arg::Same {
                same = Num::join(same, self.num_ty(a)?);
            }
        }
        for (a, k) in args.iter().zip(b.args) {
            match k {
                Arg::Int => {
                    let t = self.expr(a)?;
                    if t == Num::Fix {
                        self.push(16);
                        self.op(Op::SHR);
                    }
                }
                Arg::Fix => self.expr_as(a, Num::Fix)?,
                Arg::Same => {
                    let t = self.expr(a)?;
                    self.promote(t, same);
                }
                Arg::Angle => {
                    let t = self.expr(a)?;
                    if t == Num::Int {
                        self.push(360);
                        self.op(Op::MOD);
                        self.push(16);
                        self.op(Op::SHL);
                    }
                }
                Arg::Text => self.text_arg(a)?,
                Arg::ArrW => self.arr_arg(a, true, None)?,
                Arg::ArrR => self.arr_arg(a, false, None)?,
            }
        }
        self.call_builtin(b.id, args.len() as u8);
        Ok(match b.ret {
            Ret::Int => Num::Int,
            Ret::Fix => Num::Fix,
            Ret::Same => same,
        })
    }

    fn call_user<'b>(&mut self, f: usize, name: &'b Name<'a>, args: &'b [Expr<'a>], pos: Pos) -> R<Num> {
        if self.f.audio {
            return Err(Diag::error(pos, "audio(t) can't call functions"));
        }
        let sig = self.funcs[f].clone();
        if args.len() != sig.params.len() {
            return Err(Diag::error(
                pos,
                format!("{}() takes {} arguments, got {}", name.text, sig.params.len(), args.len()),
            ));
        }
        for (k, (a, (pname, kind))) in args.iter().zip(sig.params.iter()).enumerate() {
            let writes = sig.writes[k];
            match *kind {
                ParamKind::Num(n) => {
                    let t = self.expr(a)?;
                    if t == Num::Fix && n == Num::Int {
                        return Err(Diag::error(
                            a.pos,
                            format!(
                                "parameter '{}' of {}() is int; pass int(x) or declare '{}: fix'",
                                pname, name.text, pname
                            ),
                        ));
                    }
                    self.promote(t, n);
                }
                ParamKind::Arr(n) => {
                    if writes && a.plain_str().is_some() {
                        return Err(Diag::error(
                            a.pos,
                            format!("{}() writes to '{}'; pass a var array, not text", name.text, pname),
                        ));
                    }
                    self.arr_arg(a, writes, Some((n, name, *pname)))?;
                }
                ParamKind::Rec(r) => {
                    let v = self.view_arg(a, &Shape::Rec(r), &self.records[r].name.into())?;
                    self.check_ro(&v, writes, name, pname, a.pos)?;
                    self.rec_ref(&v).map_err(|d| Diag { pos: a.pos, ..d })?;
                }
                ParamKind::RecArr(r) => {
                    let v = self.view_arg(a, &Shape::Arr(Elem::Rec(r), Vec::new()), &format!("{}[]", self.records[r].name))?;
                    self.check_ro(&v, writes, name, pname, a.pos)?;
                    if !v.terms.is_empty() || v.scale != 0 || v.in_array_field {
                        return Err(Diag::error(a.pos, "pass a whole array of records"));
                    }
                    self.emit_base_handle(&v);
                }
            }
        }
        self.code.push(Op::CALL_FN as u8);
        write::put_u16(&mut self.code, sig.index);
        self.adjust(sig.slots() as i32, 1);
        Ok(sig.ret)
    }

    fn check_ro(&self, v: &View, writes: bool, f: &Name<'a>, p: &str, pos: Pos) -> R<()> {
        if writes && matches!(v.base, Base::Rom(_)) {
            return Err(Diag::error(
                pos,
                format!("{}() writes to '{}'; '{}' is a constant table", f.text, p, v.name),
            ));
        }
        Ok(())
    }

    /// Resolves an argument that must be a record or record array of a given shape.
    fn view_arg<'b>(&mut self, a: &'b Expr<'a>, want: &Shape, what: &String) -> R<View<'b, 'a>> {
        let bad = || Diag::error(a.pos, format!("expected a {}", what));
        let ExprKind::Path(p) = &a.kind else { return Err(bad()) };
        let Res::View(v) = self.resolve(p)? else { return Err(bad()) };
        let ok = match (&v.shape, want) {
            (Shape::Rec(x), Shape::Rec(y)) => x == y,
            (Shape::Arr(Elem::Rec(x), d), Shape::Arr(Elem::Rec(y), _)) => x == y && d.len() == 1,
            _ => false,
        };
        if !ok {
            return Err(bad());
        }
        Ok(v)
    }

    /// An array argument: a 1D array, a row, a constant table, or (read-only) text.
    fn arr_arg<'b>(&mut self, a: &'b Expr<'a>, write: bool, param: Option<(Num, &Name<'a>, &str)>) -> R<()> {
        if let Some(s) = a.plain_str() {
            if write {
                return Err(Diag::error(a.pos, "text literals are read-only; pass a var array"));
            }
            if let Some((Num::Fix, f, p)) = param {
                return Err(Diag::error(
                    a.pos,
                    format!("parameter '{}' of {}() holds fix values, not text", p, f.text),
                ));
            }
            let id = self.text_table(&s, a.pos)?;
            self.op_u16(Op::PUSH_ROM, id);
            return Ok(());
        }
        let ExprKind::Path(p) = &a.kind else {
            return Err(Diag::error(a.pos, "expected an array"));
        };
        let Res::View(v) = self.resolve(p)? else {
            return Err(Diag::error(a.pos, "expected an array"));
        };
        let Shape::Arr(Elem::Num(n), _) = &v.shape else {
            return Err(Diag::error(a.pos, format!("'{}' is not an array of numbers", v.name)));
        };
        if let Some((want, f, pn)) = param {
            if *n != want {
                return Err(Diag::error(
                    a.pos,
                    format!(
                        "parameter '{}' of {}() takes {} elements; '{}' holds {}",
                        pn,
                        f.text,
                        want.name(),
                        v.name,
                        n.name()
                    ),
                ));
            }
            if write && matches!(v.base, Base::Rom(_)) {
                return Err(Diag::error(
                    a.pos,
                    format!("{}() writes to '{}'; '{}' is a constant table", f.text, pn, v.name),
                ));
            }
        } else if write && matches!(v.base, Base::Rom(_)) {
            return Err(Diag::error(a.pos, format!("'{}' is a constant table and is read-only", v.name)));
        }
        if self.f.audio {
            return Err(Diag::error(a.pos, "audio(t) can't pass arrays"));
        }
        self.arr_handle(&v, a.pos)
    }

    /// A text argument: literal, template, array, table, or a number printed as decimal.
    fn text_arg<'b>(&mut self, a: &'b Expr<'a>) -> R<()> {
        match &a.kind {
            ExprKind::Str(parts) => {
                if let Some(s) = a.plain_str() {
                    let id = self.text_table(&s, a.pos)?;
                    self.op_u16(Op::PUSH_ROM, id);
                    return Ok(());
                }
                let mut tpl: Vec<u8> = vec![0];
                let mut argc = 0u8;
                for p in parts {
                    match p {
                        StrPart::Lit(b) => lit_pieces(&mut tpl, b),
                        StrPart::Interp(e, spec) => {
                            let ty = self.interp_value(e)?;
                            if ty == 2 && (spec.hex || spec.chr || spec.decimals.is_some()) {
                                return Err(Diag::error(e.pos, "text values take only a width"));
                            }
                            push_arg(&mut tpl, ty, spec);
                            argc += 1;
                        }
                    }
                }
                if argc as usize > limits::TEMPLATE_ARGS {
                    return Err(Diag::error(
                        a.pos,
                        format!("at most {} values in one string", limits::TEMPLATE_ARGS),
                    ));
                }
                tpl[0] = argc;
                tpl.push(PIECE_END);
                let id = self.template(tpl, a.pos)?;
                self.text_op(id, argc);
                Ok(())
            }
            _ => match self.ty(a)? {
                ETy::Arr(Elem::Num(_)) => self.arr_arg(a, false, None),
                ETy::Num(_) => {
                    let ty = self.interp_value(a)?;
                    let mut tpl = vec![1];
                    push_arg(&mut tpl, ty, &Spec::default());
                    tpl.push(PIECE_END);
                    let id = self.template(tpl, a.pos)?;
                    self.text_op(id, 1);
                    Ok(())
                }
                t => Err(self.not_a_number(a, &t)),
            },
        }
    }

    fn text_op(&mut self, id: u16, argc: u8) {
        self.code.push(Op::TEXT as u8);
        write::put_u16(&mut self.code, id);
        self.adjust(argc as i32, 1);
    }

    /// Emits an interpolated value; returns its template type (0 int, 1 fix, 2 text).
    fn interp_value<'b>(&mut self, e: &'b Expr<'a>) -> R<u8> {
        if let Some(s) = e.plain_str() {
            let id = self.text_table(&s, e.pos)?;
            self.op_u16(Op::PUSH_ROM, id);
            return Ok(2);
        }
        match self.ty(e)? {
            ETy::Num(_) => Ok(match self.expr(e)? {
                Num::Int => 0,
                Num::Fix => 1,
            }),
            ETy::Arr(Elem::Num(_)) => {
                self.arr_arg(e, false, None)?;
                Ok(2)
            }
            ETy::Text => Err(Diag::error(e.pos, "strings can't be nested inside strings")),
            t => Err(self.not_a_number(e, &t)),
        }
    }

    // ---------- parameter write analysis ----------

    /// Marks, for each function, which array and record parameters it writes to,
    /// directly or through the functions it calls.
    pub fn analyse_writes(&mut self) {
        let prog = self.prog;
        // (function, param) -> calls passing it on: (callee, callee param)
        let mut edges: Vec<(usize, usize, usize, usize)> = Vec::new();
        for f in 0..self.funcs.len() {
            let Ok(Item::Fn { body, .. }) = parser::item_at(self.source, &prog.items[self.funcs[f].item]) else {
                continue;
            };
            // parsed when the whole source was: it can't fail now
            let Ok(mut body) = parser::BodyReader::new(self.source, body) else {
                continue;
            };
            let names: Vec<&str> = self.funcs[f].params.iter().map(|p| p.0).collect();
            let mut direct = vec![false; names.len()];
            let mut w = WriteScan {
                names: &names,
                direct: &mut direct,
                calls: Vec::new(),
                syms: &self.syms,
            };
            while let Ok(Some(s)) = body.next_stmt() {
                w.stmt(&s);
            }
            let calls = core::mem::take(&mut w.calls);
            for (p, callee, cp) in calls {
                edges.push((f, p, callee, cp));
            }
            self.funcs[f].writes = direct;
        }
        loop {
            let mut changed = false;
            for &(f, p, g, q) in &edges {
                if self.funcs[g].writes.get(q).copied().unwrap_or(false) && !self.funcs[f].writes[p] {
                    self.funcs[f].writes[p] = true;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }
}

/// Splits literal text into template pieces of at most 255 bytes.
fn lit_pieces(tpl: &mut Vec<u8>, b: &[u8]) {
    for chunk in b.chunks(255) {
        tpl.push(PIECE_LIT);
        tpl.push(chunk.len() as u8);
        tpl.extend_from_slice(chunk);
    }
}

fn push_arg(tpl: &mut Vec<u8>, ty: u8, s: &Spec) {
    let mut fl = 0;
    if s.zero {
        fl |= fmt_flags::ZERO;
    }
    if s.hex {
        fl |= fmt_flags::HEX;
    }
    if s.chr {
        fl |= fmt_flags::CHAR;
    }
    if s.decimals.is_some() {
        fl |= fmt_flags::DECIMALS;
    }
    tpl.extend_from_slice(&[PIECE_ARG, ty, fl, s.width, s.decimals.unwrap_or(0)]);
}

/// Finds writes through parameters in a function body.
struct WriteScan<'s, 'a> {
    names: &'s [&'s str],
    direct: &'s mut Vec<bool>,
    calls: Vec<(usize, usize, usize)>,
    syms: &'s BTreeMap<&'a str, (Sym, Pos)>,
}

impl<'s, 'a> WriteScan<'s, 'a> {
    fn param_of(&self, e: &Expr<'a>) -> Option<usize> {
        match &e.kind {
            ExprKind::Path(p) => self.names.iter().position(|n| *n == p.root.text),
            _ => None,
        }
    }
    fn target(&mut self, p: &Path<'a>) {
        if let Some(i) = self.names.iter().position(|n| *n == p.root.text) {
            if !p.sels.is_empty() {
                self.direct[i] = true;
            }
        }
        for s in &p.sels {
            if let Sel::Index(e) = s {
                self.expr(e);
            }
        }
    }
    fn block(&mut self, b: &Block<'a>) {
        for s in &b.stmts {
            self.stmt(s);
        }
    }
    fn stmt(&mut self, s: &Stmt<'a>) {
        match &s.kind {
            StmtKind::Var { init: Some(e), .. } => self.expr(e),
            StmtKind::Var { .. } | StmtKind::Break | StmtKind::Continue => {}
            StmtKind::Assign { target, value, .. } => {
                self.target(target);
                self.expr(value);
            }
            StmtKind::IncDec { target, .. } => self.target(target),
            StmtKind::If { cond, then, els } => {
                self.expr(cond);
                self.block(then);
                if let Some(e) = els {
                    match &**e {
                        Else::If(s) => self.stmt(s),
                        Else::Block(b) => self.block(b),
                    }
                }
            }
            StmtKind::While { cond, body } => {
                self.expr(cond);
                self.block(body);
            }
            StmtKind::For { init, cond, step, body } => {
                if let Some(i) = init {
                    self.stmt(i);
                }
                if let Some(c) = cond {
                    self.expr(c);
                }
                if let Some(s) = step {
                    self.stmt(s);
                }
                self.block(body);
            }
            StmtKind::Switch { value, cases, default } => {
                self.expr(value);
                for (_, b) in cases {
                    self.block(b);
                }
                if let Some(b) = default {
                    self.block(b);
                }
            }
            StmtKind::Return(Some(e)) | StmtKind::Call(e) => self.expr(e),
            StmtKind::Return(None) => {}
        }
    }
    fn expr(&mut self, e: &Expr<'a>) {
        match &e.kind {
            ExprKind::Call(name, args) => {
                if let Some(b) = builtins::by_name(name.text) {
                    for (a, k) in args.iter().zip(b.args) {
                        if *k == Arg::ArrW {
                            if let Some(i) = self.param_of(a) {
                                self.direct[i] = true;
                            }
                        }
                    }
                } else if let Some((Sym::Func(g), _)) = self.syms.get(name.text) {
                    for (q, a) in args.iter().enumerate() {
                        if let Some(i) = self.param_of(a) {
                            self.calls.push((i, *g, q));
                        }
                    }
                }
                for a in args {
                    self.expr(a);
                }
            }
            ExprKind::Path(p) => {
                for s in &p.sels {
                    if let Sel::Index(e) = s {
                        self.expr(e);
                    }
                }
            }
            ExprKind::Str(parts) => {
                for p in parts {
                    if let StrPart::Interp(e, _) = p {
                        self.expr(e);
                    }
                }
            }
            ExprKind::Unary(_, a) => self.expr(a),
            ExprKind::Binary(_, a, b) => {
                self.expr(a);
                self.expr(b);
            }
            ExprKind::Cond(c, a, b) => {
                self.expr(c);
                self.expr(a);
                self.expr(b);
            }
            ExprKind::Int(_) | ExprKind::Fix(_) => {}
        }
    }
}
