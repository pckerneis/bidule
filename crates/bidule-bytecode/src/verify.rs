//! Load-time bytecode verification (spec: SEC-2; threat model › Vérification au chargement).
//!
//! For every function: each opcode is known, operands are in range, jumps land on
//! an instruction boundary inside the function, the stack depth is the same on
//! every path into an instruction, never underflows, never exceeds the declared
//! maximum, and every path ends in `RET` with exactly one value on the stack.
//! `audio(t)` may only use what the audio core offers.
//!
//! Verification needs one byte of scratch per byte of the largest function and
//! never allocates.

use crate::builtins;
use crate::image::{u16_at, u32_at, Handler, Program};
use crate::limits;
use crate::op::{Op, Operands};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VerifyError {
    pub function: u16,
    /// Code offset of the offending instruction (absolute).
    pub pc: u32,
    pub reason: &'static str,
}

impl core::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "function {} at offset {}: {}", self.function, self.pc, self.reason)
    }
}

const UNVISITED: u8 = 0xFF;
const OPERAND: u8 = 0xFE;

/// Decoded instruction: opcode and its raw operand value (sign-extended for I8/I32).
#[derive(Clone, Copy, Debug)]
pub struct Insn {
    pub op: Op,
    pub a: i32,
    pub b: u8,
    pub size: usize,
}

/// Decodes the instruction at `pc`, or `None` if the opcode is unknown or truncated.
pub fn decode(code: &[u8], pc: usize) -> Option<Insn> {
    let op = Op::from_u8(*code.get(pc)?)?;
    let n = op.operands().size();
    let ops = code.get(pc + 1..pc + 1 + n)?;
    let (a, b) = match op.operands() {
        Operands::None => (0, 0),
        Operands::U8 => (ops[0] as i32, 0),
        Operands::I8 => (ops[0] as i8 as i32, 0),
        Operands::U16 => (u16_at(ops, 0) as i32, 0),
        Operands::I32 | Operands::Target => (u32_at(ops, 0) as i32, 0),
        Operands::Builtin => (ops[0] as i32, ops[1]),
    };
    Some(Insn { op, a, b, size: 1 + n })
}

/// Verifies every function. `scratch` must be at least [`limits::FN_CODE_BYTES`] long.
pub fn verify(p: &Program, scratch: &mut [u8]) -> Result<(), VerifyError> {
    for f in 0..p.functions {
        verify_fn(p, f, scratch, true)?;
    }
    Ok(())
}

/// Checks function `f` as [`verify`] does, but returns the deepest stack it
/// reaches instead of comparing it with the declared one (for the compiler).
pub fn max_stack(p: &Program, f: u16, scratch: &mut [u8]) -> Result<u8, VerifyError> {
    verify_fn(p, f, scratch, false)
}

fn verify_fn(p: &Program, fi: u16, scratch: &mut [u8], exact: bool) -> Result<u8, VerifyError> {
    let f = p.function(fi);
    let start = f.start as usize;
    let end = f.end as usize;
    let err = |pc: usize, reason| VerifyError {
        function: fi,
        pc: pc as u32,
        reason,
    };
    let len = end - start;
    if len > limits::FN_CODE_BYTES || len > scratch.len() {
        return Err(err(start, "function too long"));
    }
    let cap = if exact { f.max_stack as usize } else { limits::MAX_STACK };
    if f.max_stack as usize > limits::MAX_STACK {
        return Err(err(start, "declared stack too deep"));
    }
    let depth = &mut scratch[..len];
    let audio = p.handler_of(fi) == Some(Handler::Audio);

    // Pass 1: decode linearly, mark operand bytes, check operands.
    depth.fill(UNVISITED);
    let mut pc = start;
    while pc < end {
        let i = decode(p.code, pc).ok_or(err(pc, "unknown opcode or truncated instruction"))?;
        if pc + i.size > end {
            return Err(err(pc, "instruction crosses the end of the function"));
        }
        for d in depth[pc - start + 1..pc - start + i.size].iter_mut() {
            *d = OPERAND;
        }
        check_operands(p, fi, f.locals, audio, &i).map_err(|r| err(pc, r))?;
        pc += i.size;
    }

    // Pass 2: propagate stack depths until stable.
    depth[0] = 0;
    let mut max_seen = 0u8;
    let mut changed = true;
    let mut passes = 0;
    while changed {
        changed = false;
        passes += 1;
        if passes > len + 1 {
            return Err(err(start, "stack analysis does not converge"));
        }
        let mut pc = start;
        while pc < end {
            let i = decode(p.code, pc).unwrap();
            let d = depth[pc - start];
            if d != UNVISITED {
                let (pop, push) = effect(p, &i);
                if (d as usize) < pop {
                    return Err(err(pc, "stack underflow"));
                }
                if i.op == Op::RET && d != 1 {
                    return Err(err(pc, "RET with values left on the stack"));
                }
                let out = d as usize - pop + push;
                if out > cap {
                    return Err(err(pc, "stack deeper than declared"));
                }
                // peak inside DUP2 etc. is `out`, since pushes follow pops
                max_seen = max_seen.max(out as u8).max(d);
                let mut flow = |target: usize, changed: &mut bool| -> Result<(), VerifyError> {
                    if target >= end {
                        return Err(err(pc, "control falls off the end of the function"));
                    }
                    let slot = &mut depth[target - start];
                    if *slot == OPERAND {
                        return Err(err(pc, "jump into the middle of an instruction"));
                    }
                    if *slot == UNVISITED {
                        *slot = out as u8;
                        *changed = true;
                    } else if *slot != out as u8 {
                        return Err(err(pc, "stack depth differs between paths"));
                    }
                    Ok(())
                };
                if i.op.is_jump() {
                    let t = i.a as u32 as usize;
                    if t < start || t >= end {
                        return Err(err(pc, "jump outside the function"));
                    }
                    flow(t, &mut changed)?;
                }
                if !i.op.is_terminal() {
                    flow(pc + i.size, &mut changed)?;
                }
            }
            pc += i.size;
        }
    }
    if exact && max_seen != f.max_stack {
        return Err(err(start, "declared stack depth is not the computed one"));
    }
    Ok(max_seen)
}

/// (pops, pushes) for an instruction whose operands were already checked.
pub fn effect(p: &Program, i: &Insn) -> (usize, usize) {
    match i.op {
        Op::CALL => (i.b as usize, 1),
        Op::CALL_FN => (p.function(i.a as u16).params as usize, 1),
        Op::TEXT => (p.text.argc(i.a as u16) as usize, 1),
        op => {
            let (a, b) = op.fixed_effect().unwrap();
            (a as usize, b as usize)
        }
    }
}

fn check_operands(p: &Program, fi: u16, locals: u8, audio: bool, i: &Insn) -> Result<(), &'static str> {
    let a = i.a;
    match i.op {
        Op::GLOBAL_LOAD if a as u16 >= p.globals => return Err("no such global"),
        Op::GLOBAL_STORE if a as u16 >= p.globals => return Err("no such global"),
        Op::GLOBAL_STORE if audio => return Err("audio(t) may not write globals"),
        Op::LOCAL_LOAD | Op::LOCAL_STORE if a >= locals as i32 => return Err("no such local"),
        Op::AUDIO_LOAD | Op::AUDIO_STORE if !audio => return Err("audio-local variable outside audio(t)"),
        Op::AUDIO_LOAD | Op::AUDIO_STORE if a as u16 >= p.audio_vars => return Err("no such audio-local variable"),
        Op::ARR_GET | Op::ARR_SET | Op::ARR_GET2 | Op::ARR_SET2 | Op::PUSH_ARR if a as u16 >= p.arrays => return Err("no such array"),
        Op::ARR_GET2 | Op::ARR_SET2 if p.array(a as u8).cols == 0 => return Err("2D access to a 1D array"),
        Op::ARR_GET | Op::ARR_SET | Op::ARR_GET2 | Op::ARR_SET2 | Op::PUSH_ARR | Op::ARR_SET_DYN if audio => {
            return Err("audio(t) may not use arrays in RAM")
        }
        Op::ROM_GET | Op::PUSH_ROM | Op::ROM_GET2 if a as u16 >= p.rom.count() => return Err("no such constant table"),
        Op::ROM_GET2 if p.rom.table(a as u16).unwrap().cols == 0 => return Err("2D access to a 1D table"),
        Op::TEXT if a as u16 >= p.text.count() => return Err("no such text template"),
        Op::CALL_FN => {
            let t = a as u16;
            if t >= p.functions {
                return Err("no such function");
            }
            if audio {
                return Err("audio(t) may not call functions");
            }
            if p.handler_of(t).is_some() {
                return Err("handlers can't be called");
            }
            let _ = fi;
        }
        Op::CALL => {
            let b = builtins::by_id(a as u8).ok_or("no such built-in")?;
            if i.b < b.min_args || i.b > b.max_args() {
                return Err("wrong argument count for built-in");
            }
            if audio && !b.audio_ok {
                return Err("built-in not available in audio(t)");
            }
        }
        _ => {}
    }
    Ok(())
}
