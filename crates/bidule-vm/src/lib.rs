//! The Bidule 01 virtual machine (spec: Runtime, VM and bytecode; SEC-1 to SEC-4).
//!
//! `no_std`, no allocation: all cart memory lives in fixed arrays inside [`Vm`]
//! (about 30 KB). The VM runs only carts that passed [`bidule_bytecode::verify`],
//! checks every memory access against the cart's own RAM and tables, and stops
//! after a per-call instruction budget so a cart can never freeze its host.
//!
//! Drawing, input, sound and the editing API are the host's business: the VM
//! calls [`Host::call`] for them. Math, text formatting and array built-ins are
//! native, so they behave the same on every target.
#![no_std]

pub mod math;
pub mod text;

use bidule_bytecode::builtins::{self, id};
use bidule_bytecode::handle::{self, Handle, Kind};
use bidule_bytecode::image::{ArgType, Handler, Piece, Program, NO_INIT};
use bidule_bytecode::limits::*;
use bidule_bytecode::verify::{self, VerifyError};
use bidule_bytecode::{Cart, LoadError, Op};

use text::Buf;

/// Why a cart stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultKind {
    /// `array` is the raw handle or `0x8000_0000 | id` for a pool array.
    OutOfBounds {
        array: i32,
        index: i32,
        size: u32,
    },
    DivisionByZero,
    StackOverflow,
    CallDepth,
    LocalsExhausted,
    BadHandle,
    ReadOnly,
    TextStack,
    AudioBudget,
    Host(&'static str),
}

impl FaultKind {
    pub fn message(&self) -> &'static str {
        match self {
            FaultKind::OutOfBounds { .. } => "index out of bounds",
            FaultKind::DivisionByZero => "division by zero",
            FaultKind::StackOverflow => "value stack exceeded",
            FaultKind::CallDepth => "call depth exceeded",
            FaultKind::LocalsExhausted => "local variable stack exceeded",
            FaultKind::BadHandle => "not an array",
            FaultKind::ReadOnly => "write to read-only data",
            FaultKind::TextStack => "too many text values pending",
            FaultKind::AudioBudget => "audio(t) took too long",
            FaultKind::Host(m) => m,
        }
    }
    /// Faults that stop a cart in release mode too.
    pub fn fatal(&self) -> bool {
        matches!(
            self,
            FaultKind::StackOverflow | FaultKind::CallDepth | FaultKind::LocalsExhausted | FaultKind::TextStack | FaultKind::Host(_)
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fault {
    pub kind: FaultKind,
    /// Offset of the faulting instruction.
    pub pc: u32,
    pub function: u16,
}

/// Outcome of [`Vm::run`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
    /// The handler returned this value.
    Done(i32),
    /// The instruction budget ran out; call `run` again to continue.
    Budget,
    /// A built-in is waiting on a system dialog; call [`Vm::resume`] with its result.
    Suspend {
        builtin: u8,
    },
    Fault(Fault),
}

/// Result of a host built-in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Call {
    Value(i32),
    /// The built-in shows a system dialog; the cart waits for [`Vm::resume`].
    Suspend,
    Fault(&'static str),
}

/// What the VM needs from the platform.
pub trait Host {
    /// Runs host built-in `id` (see `bidule_bytecode::builtins`). `args` are the
    /// raw argument values; `text` is the formatted text argument, for the
    /// built-ins that take one (`print`, `status`, ...). `mem` gives access to
    /// array arguments by handle (`readasset`, `cartname`, ...).
    fn call(&mut self, id: u8, args: &[i32], text: &[u8], mem: &mut Mem) -> Call;
}

/// Cart RAM: the array pool, and read access to constant tables.
pub struct Mem<'p> {
    prog: Program<'p>,
    pool: [i32; POOL],
}

/// What a handle refers to.
#[derive(Clone, Copy, Debug)]
struct Target {
    rom: bool,
    id: u16,
    start: usize,
    extent: usize,
    /// Rows and columns of the whole array (cols = 0 for 1D).
    rows: usize,
    cols: usize,
    row: bool,
}

impl<'p> Mem<'p> {
    fn target(&self, h: i32) -> Option<Target> {
        let h = Handle::decode(h)?;
        let (len, cols, rom) = match h.kind {
            Kind::Pool => {
                if h.id >= self.prog.arrays {
                    return None;
                }
                let a = self.prog.array(h.id as u8);
                (a.len as usize, a.cols as usize, false)
            }
            Kind::Rom => {
                let t = self.prog.rom.table(h.id)?;
                (t.count as usize, t.cols as usize, true)
            }
            Kind::Template => return None,
        };
        let off = h.offset as usize;
        let rows = if cols == 0 { len } else { len / cols };
        if h.row {
            if cols == 0 || !off.is_multiple_of(cols) || off + cols > len {
                return None;
            }
            Some(Target {
                rom,
                id: h.id,
                start: off,
                extent: cols,
                rows,
                cols,
                row: true,
            })
        } else {
            if off >= len {
                return None;
            }
            Some(Target {
                rom,
                id: h.id,
                start: off,
                extent: len - off,
                rows,
                cols,
                row: false,
            })
        }
    }

    fn read(&self, t: &Target, i: usize) -> i32 {
        if t.rom {
            let info = self.prog.rom.table(t.id).unwrap();
            self.prog.rom.get(&info, (t.start + i) as i32).unwrap_or(0)
        } else {
            let a = self.prog.array(t.id as u8);
            self.pool[a.offset as usize + t.start + i]
        }
    }

    /// Number of elements behind a handle, or `None` if it is not an array.
    pub fn extent(&self, h: i32) -> Option<usize> {
        self.target(h).map(|t| t.extent)
    }
    /// Element `i` of an array handle.
    pub fn get(&self, h: i32, i: usize) -> Option<i32> {
        let t = self.target(h)?;
        if i < t.extent {
            Some(self.read(&t, i))
        } else {
            None
        }
    }
    /// Writes element `i`; false for read-only data, bad handles or bad indexes.
    pub fn set(&mut self, h: i32, i: usize, v: i32) -> bool {
        match self.target(h) {
            Some(t) if !t.rom && i < t.extent => {
                let a = self.prog.array(t.id as u8);
                self.pool[a.offset as usize + t.start + i] = v;
                true
            }
            _ => false,
        }
    }
    /// The program, for hosts that need tables or debug names.
    pub fn program(&self) -> &Program<'p> {
        &self.prog
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Frame {
    func: u16,
    ret: u32,
    lbase: u16,
    sbase: u16,
}

/// Registers of one execution core (the main core or the audio core).
#[derive(Clone, Copy, Debug, Default)]
struct Regs {
    pc: u32,
    ipc: u32,
    sp: usize,
    lp: usize,
    nf: usize,
    running: bool,
    /// Built-in waiting for `resume`, and the fmt-stack level to restore after it.
    pending: Option<(u8, usize)>,
}

struct CoreRef<'c> {
    r: &'c mut Regs,
    stack: &'c mut [i32],
    locals: &'c mut [i32],
    frames: &'c mut [Frame],
}

struct Env<'p> {
    mem: Mem<'p>,
    globals: [i32; GLOBALS],
    snapshot: [i32; GLOBALS],
    audio_vars: [i32; AUDIO_VARS],
    fmt: [i32; FMT_STACK],
    ft: usize,
    rng: u32,
    tbuf: [u8; TEXT_BYTES],
    tbuf2: [u8; TEXT_BYTES],
    debug: bool,
}

/// A loaded cart and its memory.
pub struct Vm<'p> {
    env: Env<'p>,
    main: Regs,
    stack: [i32; VALUE_STACK],
    locals: [i32; LOCAL_STACK],
    frames: [Frame; CALL_DEPTH],
    audio: Regs,
    astack: [i32; AUDIO_STACK],
    alocals: [i32; AUDIO_LOCALS],
    aframes: [Frame; 1],
    /// First audio fault, kept for the debug log.
    pub audio_fault: Option<Fault>,
}

/// Why a cart could not be loaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reject {
    Load(LoadError),
    Verify(VerifyError),
}

impl core::fmt::Display for Reject {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Reject::Load(e) => write!(f, "{}", e),
            Reject::Verify(e) => write!(f, "bytecode rejected: {}", e),
        }
    }
}

/// Instructions one audio sample may run.
pub const AUDIO_BUDGET: u32 = 2048;

const ONE: i32 = 65536;

impl<'p> Vm<'p> {
    /// Parses and verifies a cart image. `scratch` must hold
    /// [`FN_CODE_BYTES`] bytes; it is only used during verification.
    pub fn load(bytes: &'p [u8], scratch: &mut [u8], debug: bool, seed: u32) -> Result<Self, Reject> {
        let cart = Cart::parse(bytes).map_err(Reject::Load)?;
        let prog = Program::from_cart(&cart).map_err(Reject::Load)?;
        verify::verify(&prog, scratch).map_err(Reject::Verify)?;
        Ok(Self::new(prog, debug, seed))
    }

    /// A VM for an already verified program.
    pub fn new(prog: Program<'p>, debug: bool, seed: u32) -> Self {
        let mut vm = Vm {
            env: Env {
                mem: Mem { prog, pool: [0; POOL] },
                globals: [0; GLOBALS],
                snapshot: [0; GLOBALS],
                audio_vars: [0; AUDIO_VARS],
                fmt: [0; FMT_STACK],
                ft: 0,
                rng: 1,
                tbuf: [0; TEXT_BYTES],
                tbuf2: [0; TEXT_BYTES],
                debug,
            },
            main: Regs::default(),
            stack: [0; VALUE_STACK],
            locals: [0; LOCAL_STACK],
            frames: [Frame::default(); CALL_DEPTH],
            audio: Regs::default(),
            astack: [0; AUDIO_STACK],
            alocals: [0; AUDIO_LOCALS],
            aframes: [Frame::default(); 1],
            audio_fault: None,
        };
        vm.reset(seed);
        vm
    }

    /// Restores the cart's memory to its initial state (globals, arrays, audio
    /// variables) and seeds the random generator.
    pub fn reset(&mut self, seed: u32) {
        let p = self.env.mem.prog;
        self.env.globals = [0; GLOBALS];
        for i in 0..p.globals {
            self.env.globals[i as usize] = p.global_init(i);
        }
        self.env.snapshot = self.env.globals;
        self.env.audio_vars = [0; AUDIO_VARS];
        for i in 0..p.audio_vars {
            self.env.audio_vars[i as usize] = p.audio_init(i);
        }
        self.env.mem.pool = [0; POOL];
        for i in 0..p.arrays {
            let a = p.array(i as u8);
            if a.init != NO_INIT {
                let t = p.rom.table(a.init).unwrap();
                for k in 0..t.count {
                    self.env.mem.pool[a.offset as usize + k as usize] = p.rom.get(&t, k as i32).unwrap();
                }
            }
        }
        self.env.rng = if seed == 0 { 0x9E37_79B9 } else { seed };
        self.main = Regs::default();
        self.audio = Regs::default();
        self.env.ft = 0;
        self.audio_fault = None;
    }

    pub fn debug(&self) -> bool {
        self.env.debug
    }
    pub fn set_debug(&mut self, on: bool) {
        self.env.debug = on;
    }
    pub fn program(&self) -> &Program<'p> {
        &self.env.mem.prog
    }
    pub fn mem(&mut self) -> &mut Mem<'p> {
        &mut self.env.mem
    }
    pub fn global(&self, i: u8) -> i32 {
        self.env.globals[i as usize]
    }
    pub fn has(&self, h: Handler) -> bool {
        self.env.mem.prog.handler(h).is_some()
    }
    /// True while a handler has started and not yet returned or faulted.
    pub fn busy(&self) -> bool {
        self.main.running
    }

    /// Starts a lifecycle handler (not `Audio`). Arguments beyond the handler's
    /// parameters are dropped. Returns false if the cart doesn't define it.
    pub fn start(&mut self, h: Handler, args: &[i32]) -> bool {
        let Some(f) = self.env.mem.prog.handler(h) else { return false };
        let info = self.env.mem.prog.function(f);
        self.main = Regs {
            pc: info.start,
            ipc: info.start,
            sp: 0,
            lp: info.locals as usize,
            nf: 1,
            running: true,
            pending: None,
        };
        self.locals[..info.locals as usize].fill(0);
        for (i, &a) in args.iter().take(info.params as usize).enumerate() {
            self.locals[i] = a;
        }
        self.frames[0] = Frame {
            func: f,
            ret: 0,
            lbase: 0,
            sbase: 0,
        };
        self.env.ft = 0;
        true
    }

    /// Runs the started handler for at most `budget` instructions.
    pub fn run(&mut self, host: &mut dyn Host, budget: u32) -> Exit {
        if !self.main.running {
            return Exit::Done(0);
        }
        if let Some((b, _)) = self.main.pending {
            return Exit::Suspend { builtin: b };
        }
        let mut core = CoreRef {
            r: &mut self.main,
            stack: &mut self.stack,
            locals: &mut self.locals,
            frames: &mut self.frames,
        };
        let exit = exec(&mut core, &mut self.env, host, budget, false);
        if matches!(exit, Exit::Done(_) | Exit::Fault(_)) {
            self.main.running = false;
        }
        exit
    }

    /// Delivers the result of a suspended built-in (a system dialog).
    pub fn resume(&mut self, value: i32) {
        if let Some((_, ft)) = self.main.pending.take() {
            self.env.ft = ft;
            let sp = self.main.sp;
            self.stack[sp] = value;
            self.main.sp += 1;
        }
    }

    /// Copies globals for `audio(t)`, which reads them one frame late.
    pub fn snapshot_globals(&mut self) {
        self.env.snapshot = self.env.globals;
    }

    /// Computes one bytebeat sample (8-bit, 128 = silence). A fault or an
    /// overlong run gives silence; the first fault is kept in `audio_fault`.
    pub fn audio_sample(&mut self, t: i32, host: &mut dyn Host) -> u8 {
        let Some(f) = self.env.mem.prog.handler(Handler::Audio) else {
            return 128;
        };
        let info = self.env.mem.prog.function(f);
        self.audio = Regs {
            pc: info.start,
            ipc: info.start,
            sp: 0,
            lp: info.locals as usize,
            nf: 1,
            running: true,
            pending: None,
        };
        self.alocals[..info.locals as usize].fill(0);
        if info.params > 0 {
            self.alocals[0] = t;
        }
        self.aframes[0] = Frame {
            func: f,
            ret: 0,
            lbase: 0,
            sbase: 0,
        };
        let ft = self.env.ft;
        let mut core = CoreRef {
            r: &mut self.audio,
            stack: &mut self.astack,
            locals: &mut self.alocals,
            frames: &mut self.aframes,
        };
        let exit = exec(&mut core, &mut self.env, host, AUDIO_BUDGET, true);
        self.env.ft = ft;
        match exit {
            Exit::Done(v) => v as u8,
            Exit::Fault(fl) => {
                self.audio_fault.get_or_insert(fl);
                128
            }
            _ => {
                let pc = self.audio.ipc;
                self.audio_fault.get_or_insert(Fault {
                    kind: FaultKind::AudioBudget,
                    pc,
                    function: f,
                });
                128
            }
        }
    }

    /// Source line and column of a fault, when the cart has debug info.
    pub fn fault_line(&self, f: &Fault) -> Option<(u16, u16)> {
        self.env.mem.prog.debug.and_then(|d| d.line_at(f.pc))
    }

    /// A fault's message; for an array index, with the array's name and size.
    pub fn write_fault(&self, w: &mut dyn core::fmt::Write, f: &Fault) -> core::fmt::Result {
        w.write_str(f.kind.message())?;
        let FaultKind::OutOfBounds { array, index, size } = f.kind else {
            return Ok(());
        };
        let p = self.program();
        let name: &[u8] = if array < 0 {
            p.array_name((array & 0xFF) as u16)
        } else {
            match Handle::decode(array) {
                Some(h) if h.kind == Kind::Pool => p.array_name(h.id),
                Some(_) => b"constant table",
                None => b"",
            }
        };
        match core::str::from_utf8(name) {
            Ok(n) if !n.is_empty() => write!(w, " ({}[{}], size {})", n, index, size),
            _ => write!(w, " (index {}, size {})", index, size),
        }
    }

    /// Call trace of the main core, innermost first: (function, pc).
    pub fn trace(&self, out: &mut [(u16, u32)]) -> usize {
        let mut n = 0;
        let mut pc = self.main.ipc;
        for i in (0..self.main.nf).rev() {
            if n == out.len() {
                break;
            }
            out[n] = (self.frames[i].func, pc);
            pc = self.frames[i].ret.saturating_sub(3);
            n += 1;
        }
        n
    }
}

// ---------- the interpreter ----------

#[inline]
fn rd_u16(c: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([c[o], c[o + 1]])
}
#[inline]
fn rd_u32(c: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([c[o], c[o + 1], c[o + 2], c[o + 3]])
}

fn exec(core: &mut CoreRef, env: &mut Env, host: &mut dyn Host, budget: u32, audio: bool) -> Exit {
    let prog = env.mem.prog;
    let code = prog.code;
    let mut left = budget;
    macro_rules! fault {
        ($k:expr) => {{
            let f = Fault {
                kind: $k,
                pc: core.r.ipc,
                function: core.frames[core.r.nf - 1].func,
            };
            return Exit::Fault(f);
        }};
    }
    // a recoverable fault: stop in debug mode, otherwise go on with `$v`
    macro_rules! soft {
        ($k:expr, $v:expr) => {{
            if env.debug {
                fault!($k)
            }
            $v
        }};
    }
    macro_rules! pop {
        () => {{
            core.r.sp -= 1;
            core.stack[core.r.sp]
        }};
    }
    macro_rules! push {
        ($v:expr) => {{
            let v = $v;
            core.stack[core.r.sp] = v;
            core.r.sp += 1;
        }};
    }
    macro_rules! oob {
        ($arr:expr, $i:expr, $n:expr) => {
            FaultKind::OutOfBounds {
                array: $arr,
                index: $i,
                size: $n as u32,
            }
        };
    }
    loop {
        if left == 0 {
            return Exit::Budget;
        }
        left -= 1;
        let pc = core.r.pc as usize;
        core.r.ipc = core.r.pc;
        let op = Op::from_u8(code[pc]).unwrap();
        let next = pc + 1 + op.operands().size();
        core.r.pc = next as u32;
        let o = pc + 1;
        match op {
            Op::NOP => {}
            Op::PUSH_INT => push!(rd_u32(code, o) as i32),
            Op::PUSH_I8 => push!(code[o] as i8 as i32),
            Op::POP => {
                core.r.sp -= 1;
            }
            Op::DUP => {
                let v = core.stack[core.r.sp - 1];
                push!(v);
            }
            Op::DUP2 => {
                let a = core.stack[core.r.sp - 2];
                let b = core.stack[core.r.sp - 1];
                push!(a);
                push!(b);
            }
            Op::GLOBAL_LOAD => {
                let g = code[o] as usize;
                push!(if audio { env.snapshot[g] } else { env.globals[g] });
            }
            Op::GLOBAL_STORE => {
                let v = pop!();
                env.globals[code[o] as usize] = v;
            }
            Op::LOCAL_LOAD => {
                let base = core.frames[core.r.nf - 1].lbase as usize;
                push!(core.locals[base + code[o] as usize]);
            }
            Op::LOCAL_STORE => {
                let base = core.frames[core.r.nf - 1].lbase as usize;
                let v = pop!();
                core.locals[base + code[o] as usize] = v;
            }
            Op::AUDIO_LOAD => push!(env.audio_vars[code[o] as usize]),
            Op::AUDIO_STORE => {
                let v = pop!();
                env.audio_vars[code[o] as usize] = v;
            }
            Op::ADD
            | Op::SUB
            | Op::MUL
            | Op::DIV
            | Op::MOD
            | Op::FMUL
            | Op::FDIV
            | Op::BAND
            | Op::BOR
            | Op::BXOR
            | Op::SHL
            | Op::SHR
            | Op::EQ
            | Op::NE
            | Op::LT
            | Op::LE
            | Op::GT
            | Op::GE => {
                let b = pop!();
                let a = pop!();
                let v = match op {
                    Op::ADD => a.wrapping_add(b),
                    Op::SUB => a.wrapping_sub(b),
                    Op::MUL => a.wrapping_mul(b),
                    Op::DIV => {
                        if b == 0 {
                            soft!(FaultKind::DivisionByZero, 0)
                        } else {
                            a.wrapping_div(b)
                        }
                    }
                    Op::MOD => {
                        if b == 0 {
                            soft!(FaultKind::DivisionByZero, 0)
                        } else {
                            a.wrapping_rem(b)
                        }
                    }
                    Op::FMUL => math::fmul(a, b),
                    Op::FDIV => match math::fdiv(a, b) {
                        Some(v) => v,
                        None => soft!(FaultKind::DivisionByZero, 0),
                    },
                    Op::BAND => a & b,
                    Op::BOR => a | b,
                    Op::BXOR => a ^ b,
                    Op::SHL => a.wrapping_shl(b as u32 & 31),
                    Op::SHR => a.wrapping_shr(b as u32 & 31),
                    Op::EQ => (a == b) as i32,
                    Op::NE => (a != b) as i32,
                    Op::LT => (a < b) as i32,
                    Op::LE => (a <= b) as i32,
                    Op::GT => (a > b) as i32,
                    _ => (a >= b) as i32,
                };
                push!(v);
            }
            Op::NEG => {
                let a = pop!();
                push!(a.wrapping_neg());
            }
            Op::BNOT => {
                let a = pop!();
                push!(!a);
            }
            Op::NOT => {
                let a = pop!();
                push!((a == 0) as i32);
            }
            Op::BOOL => {
                let a = pop!();
                push!((a != 0) as i32);
            }
            Op::JMP => core.r.pc = rd_u32(code, o),
            Op::JZ => {
                if pop!() == 0 {
                    core.r.pc = rd_u32(code, o);
                }
            }
            Op::JNZ => {
                if pop!() != 0 {
                    core.r.pc = rd_u32(code, o);
                }
            }
            Op::CALL_FN => {
                let f = rd_u16(code, o);
                let info = prog.function(f);
                if core.r.nf >= core.frames.len() {
                    fault!(FaultKind::CallDepth);
                }
                let n = info.params as usize;
                let lbase = core.r.lp;
                if lbase + info.locals as usize > core.locals.len() {
                    fault!(FaultKind::LocalsExhausted);
                }
                let sbase = core.r.sp - n;
                if sbase + info.max_stack as usize > core.stack.len() {
                    fault!(FaultKind::StackOverflow);
                }
                core.locals[lbase..lbase + n].copy_from_slice(&core.stack[sbase..sbase + n]);
                core.locals[lbase + n..lbase + info.locals as usize].fill(0);
                core.r.sp = sbase;
                core.frames[core.r.nf] = Frame {
                    func: f,
                    ret: next as u32,
                    lbase: lbase as u16,
                    sbase: sbase as u16,
                };
                core.r.nf += 1;
                core.r.lp = lbase + info.locals as usize;
                core.r.pc = info.start;
            }
            Op::RET => {
                let v = pop!();
                core.r.nf -= 1;
                let fr = core.frames[core.r.nf];
                core.r.lp = fr.lbase as usize;
                core.r.sp = fr.sbase as usize;
                if core.r.nf == 0 {
                    return Exit::Done(v);
                }
                core.r.pc = fr.ret;
                push!(v);
            }
            Op::CALL => {
                let b = code[o];
                let argc = code[o + 1] as usize;
                let base = core.r.sp - argc;
                // template arguments are consumed by this call
                let mut ft = env.ft;
                for &a in &core.stack[base..base + argc] {
                    if let Some(Handle {
                        kind: Kind::Template,
                        offset,
                        ..
                    }) = Handle::decode(a)
                    {
                        ft = ft.min(offset as usize);
                    }
                }
                let mut args = [0i32; 8];
                args[..argc].copy_from_slice(&core.stack[base..base + argc]);
                core.r.sp = base;
                match builtin(env, host, b, &args[..argc], audio) {
                    Ok(Call::Value(v)) => {
                        env.ft = ft;
                        push!(v);
                    }
                    Ok(Call::Suspend) => {
                        core.r.pending = Some((b, ft));
                        return Exit::Suspend { builtin: b };
                    }
                    Ok(Call::Fault(m)) => fault!(FaultKind::Host(m)),
                    Err(k) => {
                        if k.fatal() || env.debug {
                            fault!(k)
                        }
                        env.ft = ft;
                        push!(0);
                    }
                }
            }
            Op::ARR_GET | Op::ARR_SET | Op::ARR_GET2 | Op::ARR_SET2 => {
                let a = prog.array(code[o]);
                let v = if matches!(op, Op::ARR_SET | Op::ARR_SET2) { pop!() } else { 0 };
                let (idx, ok, shown, size) = if matches!(op, Op::ARR_GET2 | Op::ARR_SET2) {
                    let c = pop!();
                    let r = pop!();
                    let rows = a.rows() as i32;
                    let cols = a.cols as i32;
                    if r < 0 || r >= rows {
                        (0, false, r, rows)
                    } else if c < 0 || c >= cols {
                        (0, false, c, cols)
                    } else {
                        ((r * cols + c) as usize, true, 0, 0)
                    }
                } else {
                    let i = pop!();
                    (i as usize, i >= 0 && i < a.len as i32, i, a.len as i32)
                };
                let id = (0x8000_0000u32 | code[o] as u32) as i32;
                if matches!(op, Op::ARR_SET | Op::ARR_SET2) {
                    if ok {
                        env.mem.pool[a.offset as usize + idx] = v;
                    } else {
                        soft!(oob!(id, shown, size), ())
                    }
                } else if ok {
                    push!(env.mem.pool[a.offset as usize + idx]);
                } else {
                    push!(soft!(oob!(id, shown, size), 0));
                }
            }
            Op::PUSH_ARR => push!(Handle::pool(code[o] as u16).encode()),
            Op::PUSH_ROM => push!(Handle::rom(rd_u16(code, o)).encode()),
            Op::ROM_GET | Op::ROM_GET2 => {
                let tid = rd_u16(code, o);
                let t = prog.rom.table(tid).unwrap();
                let (i, ok, shown, size) = if op == Op::ROM_GET2 {
                    let c = pop!();
                    let r = pop!();
                    let (rows, cols) = (t.rows() as i32, t.cols as i32);
                    if r < 0 || r >= rows {
                        (0, false, r, rows)
                    } else if c < 0 || c >= cols {
                        (0, false, c, cols)
                    } else {
                        (r * cols + c, true, 0, 0)
                    }
                } else {
                    let i = pop!();
                    (i, i >= 0 && i < t.count as i32, i, t.count as i32)
                };
                if ok {
                    push!(prog.rom.get(&t, i).unwrap());
                } else {
                    push!(soft!(oob!(Handle::rom(tid).encode(), shown, size), 0));
                }
            }
            Op::ROW => {
                let r = pop!();
                let h = pop!();
                match env.mem.target(h) {
                    Some(t) if !t.row && t.start == 0 && t.cols > 0 => {
                        if r >= 0 && (r as usize) < t.rows {
                            let hd = Handle {
                                kind: if t.rom { Kind::Rom } else { Kind::Pool },
                                id: t.id,
                                offset: (r as usize * t.cols) as u16,
                                row: true,
                            };
                            push!(hd.encode());
                        } else {
                            push!(soft!(oob!(h, r, t.rows), handle::NULL));
                        }
                    }
                    _ => push!(soft!(FaultKind::BadHandle, handle::NULL)),
                }
            }
            Op::CHK => {
                let n = rd_u16(code, o) as i32;
                let v = core.stack[core.r.sp - 1];
                if env.debug && (v < 0 || v >= n) {
                    fault!(oob!(0, v, n));
                }
            }
            Op::ARR_GET_DYN => {
                let i = pop!();
                let h = pop!();
                match env.mem.target(h) {
                    Some(t) if !(audio && !t.rom) => {
                        if i >= 0 && (i as usize) < t.extent {
                            push!(env.mem.read(&t, i as usize));
                        } else {
                            push!(soft!(oob!(h, i, t.extent), 0));
                        }
                    }
                    _ => push!(soft!(FaultKind::BadHandle, 0)),
                }
            }
            Op::ARR_SET_DYN => {
                let v = pop!();
                let i = pop!();
                let h = pop!();
                match env.mem.target(h) {
                    Some(t) if t.rom => soft!(FaultKind::ReadOnly, ()),
                    Some(t) => {
                        if i >= 0 && (i as usize) < t.extent {
                            let a = prog.array(t.id as u8);
                            env.mem.pool[a.offset as usize + t.start + i as usize] = v;
                        } else {
                            soft!(oob!(h, i, t.extent), ())
                        }
                    }
                    None => soft!(FaultKind::BadHandle, ()),
                }
            }
            Op::ARR_LEN_DYN => {
                let h = pop!();
                let n = match env.mem.target(h) {
                    Some(t) if t.row => t.cols as i32,
                    Some(t) if t.cols > 0 && t.start == 0 => t.rows as i32,
                    Some(t) => t.extent as i32,
                    None => soft!(FaultKind::BadHandle, 0),
                };
                push!(n);
            }
            Op::TEXT => {
                let tid = rd_u16(code, o);
                let argc = prog.text.argc(tid) as usize;
                if env.ft + argc > FMT_STACK {
                    fault!(FaultKind::TextStack);
                }
                let base = core.r.sp - argc;
                env.fmt[env.ft..env.ft + argc].copy_from_slice(&core.stack[base..base + argc]);
                core.r.sp = base;
                push!(Handle {
                    kind: Kind::Template,
                    id: tid,
                    offset: env.ft as u16,
                    row: false
                }
                .encode());
                env.ft += argc;
            }
        }
    }
}

// ---------- text ----------

/// Formats a text value (template, array or table handle) into `out`.
fn text_into(mem: &Mem, fmt: &[i32], v: i32, out: &mut Buf, nested: bool) {
    let Some(h) = Handle::decode(v) else { return };
    if h.kind == Kind::Template {
        if nested || h.id >= mem.prog.text.count() {
            return;
        }
        let argc = mem.prog.text.argc(h.id) as usize;
        let base = h.offset as usize;
        if base + argc > fmt.len() {
            return;
        }
        let mut k = base;
        for p in mem.prog.text.pieces(h.id) {
            match p {
                Piece::Lit(s) => out.extend(s),
                Piece::Arg(f) => {
                    let a = fmt[k];
                    k += 1;
                    if f.ty == ArgType::Text {
                        let start = out.len;
                        text_into(mem, fmt, a, out, true);
                        let n = out.len - start;
                        pad_left(out, start, n, f.width as usize);
                    } else {
                        text::format_value(out, a, &f);
                    }
                }
            }
        }
        return;
    }
    let Some(t) = mem.target(v) else { return };
    for i in 0..t.extent {
        let c = mem.read(&t, i);
        if c == 0 {
            break;
        }
        out.push((c & 255) as u8);
    }
}

/// Right-aligns the last `n` bytes written at `start` to `width`.
fn pad_left(out: &mut Buf, start: usize, n: usize, width: usize) {
    if n >= width {
        return;
    }
    let pad = width - n;
    for _ in 0..pad {
        out.push(b' ');
    }
    let end = out.len;
    if end - start < n + pad {
        return; // truncated: leave as is
    }
    out.bytes[start..end].rotate_right(pad);
}

fn format_text(env_mem: &Mem, fmt: &[i32], v: i32, buf: &mut [u8]) -> usize {
    let mut b = Buf::new(buf);
    text_into(env_mem, fmt, v, &mut b, false);
    b.len
}

// ---------- built-ins ----------

fn builtin(env: &mut Env, host: &mut dyn Host, b: u8, a: &[i32], audio: bool) -> Result<Call, FaultKind> {
    let v = |x: i32| Ok(Call::Value(x));
    match b {
        id::SIN => v(math::sin(a[0])),
        id::COS => v(math::cos(a[0])),
        id::ATAN2 => v(math::atan2(a[0], a[1])),
        id::SQRT => v(math::sqrt(a[0])),
        id::ABS => v(a[0].wrapping_abs()),
        id::MIN => v(a[0].min(a[1])),
        id::MAX => v(a[0].max(a[1])),
        id::CLAMP => v(if a[0] < a[1] {
            a[1]
        } else if a[0] > a[2] {
            a[2]
        } else {
            a[0]
        }),
        id::RND => {
            // xorshift32, then a multiply-shift into 0..n-1
            let mut x = env.rng;
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            env.rng = x;
            v(if a[0] <= 0 { 0 } else { ((x as u64 * a[0] as u64) >> 32) as i32 })
        }
        id::SRAND => {
            env.rng = if a[0] == 0 { 0x9E37_79B9 } else { a[0] as u32 };
            v(0)
        }
        id::COPY => {
            let (Some(dn), Some(sn)) = (env.mem.extent(a[0]), env.mem.extent(a[1])) else {
                return Err(FaultKind::BadHandle);
            };
            let n = dn.min(sn);
            for i in 0..n {
                let x = env.mem.get(a[1], i).unwrap();
                if !env.mem.set(a[0], i, x) {
                    return Err(FaultKind::ReadOnly);
                }
            }
            v(n as i32)
        }
        id::STREQ => {
            let n1 = format_text(&env.mem, &env.fmt, a[0], &mut env.tbuf);
            let n2 = format_text(&env.mem, &env.fmt, a[1], &mut env.tbuf2);
            v((env.tbuf[..n1] == env.tbuf2[..n2]) as i32)
        }
        id::ARREQ => {
            let n = a[2].max(0) as usize;
            for i in 0..n {
                if env.mem.get(a[0], i).unwrap_or(0) != env.mem.get(a[1], i).unwrap_or(0) {
                    return v(0);
                }
            }
            v(1)
        }
        id::FMT => {
            let Some(cap) = env.mem.extent(a[0]) else {
                return Err(FaultKind::BadHandle);
            };
            let n = format_text(&env.mem, &env.fmt, a[1], &mut env.tbuf);
            if cap == 0 {
                return v(0);
            }
            let m = n.min(cap - 1);
            for i in 0..m {
                if !env.mem.set(a[0], i, env.tbuf[i] as i32) {
                    return Err(FaultKind::ReadOnly);
                }
            }
            env.mem.set(a[0], m, 0);
            v(m as i32)
        }
        id::TEXTWIDTH => {
            let n = format_text(&env.mem, &env.fmt, a[0], &mut env.tbuf);
            v(text::width(&env.tbuf[..n]))
        }
        _ => {
            if audio && b != id::CUECOUNT {
                return Err(FaultKind::Host("built-in not available in audio(t)"));
            }
            let info = builtins::by_id(b).ok_or(FaultKind::Host("unknown built-in"))?;
            let mut n = 0;
            if let Some(k) = info.args.iter().position(|x| *x == builtins::Arg::Text) {
                if k < a.len() {
                    n = format_text(&env.mem, &env.fmt, a[k], &mut env.tbuf);
                }
            }
            Ok(host.call(b, a, &env.tbuf[..n], &mut env.mem))
        }
    }
}

/// `ONE` exported for hosts converting fix values.
pub const FIX_ONE: i32 = ONE;

/// Decodes a source `sprite` value: (tile, width, height) in tiles.
pub fn sprite_decl(v: i32) -> Option<(u16, u8, u8)> {
    if v & 0x4000_0000 == 0 || v < 0 {
        return None;
    }
    Some(((v & 0x3FF) as u16, (((v >> 10) & 31) + 1) as u8, (((v >> 15) & 31) + 1) as u8))
}
