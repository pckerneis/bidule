//! Compile-time evaluation, with the VM's exact integer and 16.16 semantics.

use alloc::format;

use crate::ast::*;
use crate::compile::{Compiler, ASSET_PREFIXES, BUILTIN_CONSTS, R};
use crate::diag::Diag;
use crate::types::*;

/// (a * b) >> 16 with a 64-bit intermediate, as FMUL.
pub fn fmul(a: i32, b: i32) -> i32 {
    ((a as i64 * b as i64) >> 16) as i32
}

/// (a << 16) / b truncated, as FDIV; `None` when b = 0.
pub fn fdiv(a: i32, b: i32) -> Option<i32> {
    if b == 0 {
        None
    } else {
        Some((((a as i64) << 16) / b as i64) as i32)
    }
}

impl<'p, 'a> Compiler<'p, 'a> {
    /// The value of an expression if it is known at compile time.
    pub fn const_eval(&mut self, e: &Expr) -> R<Option<(i32, Num)>> {
        Ok(match &e.kind {
            ExprKind::Int(v) => Some((*v, Num::Int)),
            ExprKind::Fix(v) => Some((*v, Num::Fix)),
            ExprKind::Str(_) => None,
            ExprKind::Path(p) => self.const_path(p)?,
            ExprKind::Unary(op, a) => match self.const_eval(a)? {
                None => None,
                Some((v, t)) => Some(match op {
                    UnOp::Neg => (v.wrapping_neg(), t),
                    UnOp::Not => ((v == 0) as i32, Num::Int),
                    UnOp::BitNot => {
                        if t == Num::Fix {
                            return Err(Diag::error(e.pos, "'~' needs an int; convert with int(x)"));
                        }
                        (!v, Num::Int)
                    }
                }),
            },
            ExprKind::Binary(op, a, b) => {
                let Some((x, tx)) = self.const_eval(a)? else { return Ok(None) };
                // short-circuit operators fold only when both sides are constant
                let Some((y, ty)) = self.const_eval(b)? else { return Ok(None) };
                Some(fold_bin(*op, x, tx, y, ty).map_err(|m| Diag::error(e.pos, m))?)
            }
            ExprKind::Cond(c, a, b) => {
                let (Some((c, _)), Some((x, tx)), Some((y, ty))) = (self.const_eval(c)?, self.const_eval(a)?, self.const_eval(b)?) else {
                    return Ok(None);
                };
                let t = Num::join(tx, ty);
                let pick = if c != 0 { (x, tx) } else { (y, ty) };
                Some((if pick.1 != t { pick.0.wrapping_shl(16) } else { pick.0 }, t))
            }
            ExprKind::Call(name, args) => {
                let n = name.text;
                if !matches!(n, "int" | "round" | "frac" | "abs" | "min" | "max") {
                    return Ok(None);
                }
                // user functions with these names can't exist: the names are built in
                let mut vals = alloc::vec::Vec::new();
                for a in args {
                    match self.const_eval(a)? {
                        Some(v) => vals.push(v),
                        None => return Ok(None),
                    }
                }
                match (n, vals.as_slice()) {
                    ("int", [(v, Num::Fix)]) => Some((v >> 16, Num::Int)),
                    ("int", [(v, Num::Int)]) => Some((*v, Num::Int)),
                    ("round", [(v, Num::Fix)]) => Some((v.wrapping_add(32768) >> 16, Num::Int)),
                    ("round", [(v, Num::Int)]) => Some((*v, Num::Int)),
                    ("frac", [(v, Num::Fix)]) => Some((v & 0xFFFF, Num::Fix)),
                    ("frac", [(_, Num::Int)]) => Some((0, Num::Fix)),
                    ("abs", [(v, t)]) => Some((if *v < 0 { v.wrapping_neg() } else { *v }, *t)),
                    ("min" | "max", [(x, tx), (y, ty)]) => {
                        let t = Num::join(*tx, *ty);
                        let x = if *tx != t { x.wrapping_shl(16) } else { *x };
                        let y = if *ty != t { y.wrapping_shl(16) } else { *y };
                        Some((if (n == "min") == (x < y) { x } else { y }, t))
                    }
                    _ => None,
                }
            }
        })
    }

    fn const_path(&mut self, p: &Path) -> R<Option<(i32, Num)>> {
        let name = p.root.text;
        // locals shadow nothing (no shadowing), but a local may share no name with a constant
        if self.f.locals.iter().any(|l| l.name == name) {
            return Ok(None);
        }
        let sym = match self.const_sym(name)? {
            Some(s) => Some(s),
            None => self.sym(name).cloned(),
        };
        match (sym, p.sels.as_slice()) {
            (Some(Sym::Const(v, n)), []) => Ok(Some((v, n))),
            (Some(Sym::Enum(e)), [Sel::Field(m)]) => Ok(self.enums[e].members.iter().find(|x| x.0 == m.text).map(|x| (x.1, Num::Int))),
            (Some(Sym::Arr(a)), sels) if !sels.is_empty() => {
                // `.length` and `[i].length` of declared arrays
                if let Some(Sel::Field(f)) = sels.last() {
                    if f.text == "length" && a.elem != Elem::Rec(usize::MAX) {
                        let depth = sels.len() - 1;
                        if sels[..depth].iter().all(|s| matches!(s, Sel::Index(_))) && depth < a.dims.len() {
                            return Ok(Some((a.dims[depth] as i32, Num::Int)));
                        }
                    }
                }
                Ok(None)
            }
            (None, []) => Ok(BUILTIN_CONSTS.iter().find(|c| c.0 == name).map(|c| (c.1, Num::Int))),
            (None, [Sel::Field(n)]) => {
                let Some(k) = ASSET_PREFIXES.iter().position(|x| *x == name) else {
                    return Ok(None);
                };
                if ASSET_PREFIXES[k] == "Maps" {
                    return Ok(None);
                }
                let Some(sec) = self.assets[k] else { return Ok(None) };
                for i in 0..sec.slots {
                    if sec.name(i) == n.text.as_bytes() && !sec.slot(i).is_empty() {
                        return Ok(Some((i as i32, Num::Int)));
                    }
                }
                Ok(None)
            }
            _ => Ok(None),
        }
    }
}

pub fn fold_bin(op: BinOp, x: i32, tx: Num, y: i32, ty: Num) -> Result<(i32, Num), alloc::string::String> {
    use BinOp::*;
    if matches!(op, And | Or) {
        let v = if op == And { x != 0 && y != 0 } else { x != 0 || y != 0 };
        return Ok((v as i32, Num::Int));
    }
    let t = Num::join(tx, ty);
    if op.is_bitwise() && t == Num::Fix {
        return Err(format!("'{}' needs int operands; convert with int(x)", op.text()));
    }
    let x = if tx != t { x.wrapping_shl(16) } else { x };
    let y = if ty != t { y.wrapping_shl(16) } else { y };
    let b = |c: bool| (c as i32, Num::Int);
    Ok(match op {
        Add => (x.wrapping_add(y), t),
        Sub => (x.wrapping_sub(y), t),
        Mul if t == Num::Fix => (fmul(x, y), t),
        Mul => (x.wrapping_mul(y), t),
        Div if y == 0 => return Err(alloc::string::String::from("division by zero")),
        Div if t == Num::Fix => (fdiv(x, y).unwrap(), t),
        Div => (x.wrapping_div(y), t),
        Mod if y == 0 => return Err(alloc::string::String::from("modulo by zero")),
        Mod => (x.wrapping_rem(y), t),
        Shl => (x.wrapping_shl(y as u32 & 31), t),
        Shr => (x.wrapping_shr(y as u32 & 31), t),
        BitAnd => (x & y, t),
        BitOr => (x | y, t),
        BitXor => (x ^ y, t),
        Lt => b(x < y),
        Le => b(x <= y),
        Gt => b(x > y),
        Ge => b(x >= y),
        Eq => b(x == y),
        Ne => b(x != y),
        And | Or => unreachable!(),
    })
}
