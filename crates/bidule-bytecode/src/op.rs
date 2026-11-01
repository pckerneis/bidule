//! The opcode table (spec: Runtime, VM and bytecode › Bytecode format).
//!
//! Every instruction is one opcode byte followed by its operands, little-endian.
//! Values on the stack are 32-bit; the VM does not tag them with types.

/// Operand layout of an instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operands {
    None,
    U8,
    I8,
    U16,
    I32,
    /// Absolute code offset inside the current function's code range.
    Target,
    /// Built-in id (u8) then argument count (u8).
    Builtin,
}

impl Operands {
    pub const fn size(self) -> usize {
        match self {
            Operands::None => 0,
            Operands::U8 | Operands::I8 => 1,
            Operands::U16 | Operands::Builtin => 2,
            Operands::I32 | Operands::Target => 4,
        }
    }
}

macro_rules! opcodes {
    ($( $name:ident = $code:literal, $ops:ident, $pop:literal, $push:literal, $doc:literal; )*) => {
        /// Bidule 01 opcodes. Stack effects are given as (pops, pushes); `CALL`,
        /// `CALL_FN` and `TEXT` pop a count read from their operand or a table.
        #[repr(u8)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[allow(non_camel_case_types)]
        pub enum Op { $( #[doc = $doc] $name = $code, )* }

        impl Op {
            pub const fn from_u8(b: u8) -> Option<Op> {
                match b { $( $code => Some(Op::$name), )* _ => None }
            }
            pub const fn operands(self) -> Operands {
                match self { $( Op::$name => Operands::$ops, )* }
            }
            /// Fixed stack effect; `None` for instructions whose effect depends on a table.
            #[allow(unreachable_patterns)]
            pub const fn fixed_effect(self) -> Option<(u8, u8)> {
                match self {
                    Op::CALL | Op::CALL_FN | Op::TEXT => None,
                    $( Op::$name => Some(($pop, $push)), )*
                }
            }
            pub const fn name(self) -> &'static str {
                match self { $( Op::$name => stringify!($name), )* }
            }
            pub const ALL: &'static [Op] = &[ $( Op::$name, )* ];
        }
    };
}

opcodes! {
    NOP          = 0x00, None,    0, 0, "Does nothing.";
    PUSH_INT     = 0x01, I32,     0, 1, "Pushes a 32-bit constant.";
    PUSH_I8      = 0x02, I8,      0, 1, "Pushes a sign-extended 8-bit constant.";
    POP          = 0x03, None,    1, 0, "Discards the top value.";
    DUP          = 0x04, None,    1, 2, "Duplicates the top value.";
    GLOBAL_LOAD  = 0x05, U8,      0, 1, "Pushes global int `n`.";
    GLOBAL_STORE = 0x06, U8,      1, 0, "Pops into global int `n`.";
    LOCAL_LOAD   = 0x07, U8,      0, 1, "Pushes local `n` of the current frame.";
    LOCAL_STORE  = 0x08, U8,      1, 0, "Pops into local `n` of the current frame.";
    AUDIO_LOAD   = 0x09, U8,      0, 1, "Pushes audio-local `n` (audio handler only).";
    AUDIO_STORE  = 0x0A, U8,      1, 0, "Pops into audio-local `n` (audio handler only).";
    DUP2         = 0x0B, None,    2, 4, "Duplicates the top two values: a b -> a b a b.";

    ADD          = 0x10, None,    2, 1, "a + b, wrapping.";
    SUB          = 0x11, None,    2, 1, "a - b, wrapping.";
    MUL          = 0x12, None,    2, 1, "a * b, wrapping (int).";
    DIV          = 0x13, None,    2, 1, "a / b truncated toward zero; b = 0 is a fault (result 0).";
    MOD          = 0x14, None,    2, 1, "a % b with the sign of a; b = 0 is a fault (result 0). Also used for fix.";
    NEG          = 0x15, None,    1, 1, "-a, wrapping.";
    FMUL         = 0x16, None,    2, 1, "(a * b) >> 16 with a 64-bit intermediate.";
    FDIV         = 0x17, None,    2, 1, "(a << 16) / b with a 64-bit intermediate, truncated; b = 0 is a fault.";

    BAND         = 0x20, None,    2, 1, "a & b.";
    BOR          = 0x21, None,    2, 1, "a | b.";
    BXOR         = 0x22, None,    2, 1, "a ^ b.";
    SHL          = 0x23, None,    2, 1, "a << (b & 31).";
    SHR          = 0x24, None,    2, 1, "a >> (b & 31), arithmetic.";
    BNOT         = 0x25, None,    1, 1, "~a.";

    EQ           = 0x30, None,    2, 1, "1 if a == b else 0.";
    NE           = 0x31, None,    2, 1, "1 if a != b else 0.";
    LT           = 0x32, None,    2, 1, "1 if a < b else 0 (signed).";
    LE           = 0x33, None,    2, 1, "1 if a <= b else 0.";
    GT           = 0x34, None,    2, 1, "1 if a > b else 0.";
    GE           = 0x35, None,    2, 1, "1 if a >= b else 0.";
    NOT          = 0x36, None,    1, 1, "1 if a == 0 else 0.";
    BOOL         = 0x37, None,    1, 1, "1 if a != 0 else 0.";

    JMP          = 0x40, Target,  0, 0, "Jumps to an absolute code offset.";
    JZ           = 0x41, Target,  1, 0, "Pops; jumps if zero.";
    JNZ          = 0x42, Target,  1, 0, "Pops; jumps if not zero.";

    CALL_FN      = 0x50, U16,     0, 1, "Calls user function `n`: its parameters are popped into locals 0..argc-1 of a new frame.";
    CALL         = 0x51, Builtin, 0, 1, "Calls built-in `id` with `argc` arguments; always pushes one result.";
    RET          = 0x52, None,    1, 0, "Pops the return value and leaves the frame.";

    ARR_GET      = 0x60, U8,      1, 1, "i -> pool array `n`[i].";
    ARR_SET      = 0x61, U8,      2, 0, "i v -> pool array `n`[i] = v.";
    ARR_GET2     = 0x62, U8,      2, 1, "r c -> pool array `n`[r][c], each index checked.";
    ARR_SET2     = 0x63, U8,      3, 0, "r c v -> pool array `n`[r][c] = v.";
    PUSH_ARR     = 0x64, U8,      0, 1, "Pushes a handle to pool array `n`.";
    ROW          = 0x65, None,    2, 1, "h r -> handle to row r of 2D array h.";
    CHK          = 0x66, U16,     1, 1, "Debug mode: faults if the top value is outside 0..n-1. Release: no effect.";

    ARR_GET_DYN  = 0x74, None,    2, 1, "h i -> element i of the array behind handle h.";
    ARR_SET_DYN  = 0x75, None,    3, 0, "h i v -> element i of h = v (pool handles only).";
    ARR_LEN_DYN  = 0x76, None,    1, 1, "h -> element count of h.";

    ROM_GET      = 0x78, U16,     1, 1, "i -> constant table `n`[i].";
    PUSH_ROM     = 0x79, U16,     0, 1, "Pushes a read-only handle to constant table `n`.";
    ROM_GET2     = 0x7A, U16,     2, 1, "r c -> constant table `n`[r][c], each index checked.";
    TEXT         = 0x7B, U16,     0, 1, "Pops the arguments of text template `n` and pushes a template handle.";
}

impl Op {
    /// Ends straight-line flow: control never falls through to the next instruction.
    pub const fn is_terminal(self) -> bool {
        matches!(self, Op::JMP | Op::RET)
    }
    pub const fn is_jump(self) -> bool {
        matches!(self, Op::JMP | Op::JZ | Op::JNZ)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn codes_round_trip_and_are_unique() {
        let mut seen = [false; 256];
        for &op in Op::ALL {
            assert!(!seen[op as usize], "duplicate {:?}", op);
            seen[op as usize] = true;
            assert_eq!(Op::from_u8(op as u8), Some(op));
        }
    }
    #[test]
    fn spec_fixed_numbers() {
        assert_eq!(Op::LOCAL_LOAD as u8, 0x07);
        assert_eq!(Op::LOCAL_STORE as u8, 0x08);
        assert_eq!(Op::FMUL as u8, 0x16);
        assert_eq!(Op::FDIV as u8, 0x17);
        assert_eq!(Op::BNOT as u8, 0x25);
        assert_eq!(Op::BOOL as u8, 0x37);
        assert_eq!(Op::ARR_GET_DYN as u8, 0x74);
        assert_eq!(Op::ARR_SET_DYN as u8, 0x75);
        assert_eq!(Op::ROM_GET as u8, 0x78);
        assert_eq!(Op::PUSH_ROM as u8, 0x79);
    }
}
