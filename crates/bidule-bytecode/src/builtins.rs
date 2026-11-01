//! Built-in functions, called with `CALL id argc`.
//!
//! The ids are part of the bytecode format. Types exist only in the compiler, so
//! each argument carries a kind telling the compiler how to pass it:
//! the VM always receives plain 32-bit values.

/// How the compiler passes one argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arg {
    /// int; a fix argument is rounded down (`>> 16`) by the compiler.
    Int,
    /// fix; an int argument is promoted (`<< 16`).
    Fix,
    /// An angle in degrees: int is reduced modulo 360 then promoted, fix passes as is.
    Angle,
    /// int or fix, passed unchanged; all such arguments are promoted to fix if any is fix.
    Same,
    /// Read-only text: a string literal, an interpolated template, an array or table
    /// handle, or a number (printed as decimal).
    Text,
    /// A writable array handle.
    ArrW,
    /// A readable array handle (pool array, row or constant table).
    ArrR,
}

/// What the compiler should assume about the result type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ret {
    Int,
    Fix,
    /// Same type as the `Same` arguments.
    Same,
}

#[derive(Clone, Copy, Debug)]
pub struct Builtin {
    pub id: u8,
    pub name: &'static str,
    /// Argument kinds; the first `min_args` are required.
    pub args: &'static [Arg],
    pub min_args: u8,
    pub ret: Ret,
    /// May be called from `audio(t)`.
    pub audio_ok: bool,
    /// May suspend the cart while the system shows a dialog.
    pub suspends: bool,
    /// Internal: not callable by name from source.
    pub internal: bool,
}

impl Builtin {
    pub const fn max_args(&self) -> u8 {
        self.args.len() as u8
    }
}

use Arg::*;

macro_rules! b {
    ($id:literal, $name:literal, [$($a:ident),*], $min:literal, $ret:ident) => {
        Builtin { id: $id, name: $name, args: &[$($a),*], min_args: $min, ret: Ret::$ret, audio_ok: false, suspends: false, internal: false }
    };
    ($id:literal, $name:literal, [$($a:ident),*], $min:literal, $ret:ident, audio) => {
        Builtin { id: $id, name: $name, args: &[$($a),*], min_args: $min, ret: Ret::$ret, audio_ok: true, suspends: false, internal: false }
    };
    ($id:literal, $name:literal, [$($a:ident),*], $min:literal, $ret:ident, suspends) => {
        Builtin { id: $id, name: $name, args: &[$($a),*], min_args: $min, ret: Ret::$ret, audio_ok: false, suspends: true, internal: false }
    };
    ($id:literal, $name:literal, [$($a:ident),*], $min:literal, $ret:ident, internal) => {
        Builtin { id: $id, name: $name, args: &[$($a),*], min_args: $min, ret: Ret::$ret, audio_ok: false, suspends: false, internal: true }
    };
}

/// Built-in ids. Never renumber: carts store these ids.
pub mod id {
    pub const CLS: u8 = 0;
    pub const PSET: u8 = 1;
    pub const PGET: u8 = 2;
    pub const LINE: u8 = 3;
    pub const RECT: u8 = 4;
    pub const RECTFILL: u8 = 5;
    pub const CIRC: u8 = 6;
    pub const CIRCFILL: u8 = 7;
    pub const SPR: u8 = 8;
    pub const MAP: u8 = 9;
    pub const FGET: u8 = 10;
    pub const PRINT: u8 = 11;
    pub const TEXTWIDTH: u8 = 12;
    pub const FMT: u8 = 13;
    pub const BTN: u8 = 14;
    pub const BTNP: u8 = 15;
    pub const HASPOINTER: u8 = 16;
    pub const MOUSEX: u8 = 17;
    pub const MOUSEY: u8 = 18;
    pub const MOUSEB: u8 = 19;
    pub const SETPAL: u8 = 20;
    pub const GETPAL: u8 = 21;
    pub const PALT: u8 = 22;
    pub const SIN: u8 = 23;
    pub const COS: u8 = 24;
    pub const ATAN2: u8 = 25;
    pub const SQRT: u8 = 26;
    pub const ABS: u8 = 27;
    pub const MIN: u8 = 28;
    pub const MAX: u8 = 29;
    pub const CLAMP: u8 = 30;
    pub const RND: u8 = 31;
    pub const SRAND: u8 = 32;
    pub const COPY: u8 = 33;
    pub const STREQ: u8 = 34;
    pub const ARREQ: u8 = 35;
    pub const PLAY: u8 = 36;
    pub const STOP: u8 = 37;
    pub const STOPALL: u8 = 38;
    pub const PLAYING: u8 = 39;
    pub const CUE: u8 = 40;
    pub const CUECOUNT: u8 = 41;
    pub const PICKCART: u8 = 42;
    pub const OPENSECTION: u8 = 43;
    pub const CARTNAME: u8 = 44;
    pub const DRAWASSET: u8 = 45;
    pub const STATUS: u8 = 46;
    pub const READASSET: u8 = 47;
    pub const WRITEASSET: u8 = 48;
    pub const ASSETCOUNT: u8 = 49;
    pub const ASSETNAME: u8 = 50;
    pub const SETASSETNAME: u8 = 51;
    pub const SAVECART: u8 = 52;
    pub const MAPCELL: u8 = 53;
}

pub const BUILTINS: &[Builtin] = &[
    b!(0, "cls", [Int], 0, Int),
    b!(1, "pset", [Int, Int, Int], 3, Int),
    b!(2, "pget", [Int, Int], 2, Int),
    b!(3, "line", [Int, Int, Int, Int, Int], 5, Int),
    b!(4, "rect", [Int, Int, Int, Int, Int], 5, Int),
    b!(5, "rectfill", [Int, Int, Int, Int, Int], 5, Int),
    b!(6, "circ", [Int, Int, Int, Int], 4, Int),
    b!(7, "circfill", [Int, Int, Int, Int], 4, Int),
    b!(8, "spr", [Int, Int, Int], 3, Int),
    b!(9, "map", [Int, Int, Int, Int, Int, Int, Int], 7, Int),
    b!(10, "fget", [Int, Int], 2, Int),
    b!(11, "print", [Text, Int, Int, Int], 3, Int),
    b!(12, "textwidth", [Text], 1, Int),
    b!(13, "fmt", [ArrW, Text], 2, Int),
    b!(14, "btn", [Int], 1, Int),
    b!(15, "btnp", [Int], 1, Int),
    b!(16, "haspointer", [], 0, Int),
    b!(17, "mousex", [], 0, Int),
    b!(18, "mousey", [], 0, Int),
    b!(19, "mouseb", [], 0, Int),
    b!(20, "setpal", [Int, Int, Int, Int], 4, Int),
    b!(21, "getpal", [Int, Int], 2, Int),
    b!(22, "palt", [Int], 1, Int),
    b!(23, "sin", [Angle], 1, Fix, audio),
    b!(24, "cos", [Angle], 1, Fix, audio),
    b!(25, "atan2", [Fix, Fix], 2, Fix, audio),
    b!(26, "sqrt", [Fix], 1, Fix, audio),
    b!(27, "abs", [Same], 1, Same, audio),
    b!(28, "min", [Same, Same], 2, Same, audio),
    b!(29, "max", [Same, Same], 2, Same, audio),
    b!(30, "clamp", [Same, Same, Same], 3, Same, audio),
    b!(31, "rnd", [Int], 1, Int),
    b!(32, "srand", [Int], 1, Int),
    b!(33, "copy", [ArrW, ArrR], 2, Int),
    b!(34, "streq", [Text, Text], 2, Int),
    b!(35, "arreq", [ArrR, ArrR, Int], 3, Int),
    b!(36, "play", [Int, Int], 1, Int),
    b!(37, "stop", [Int], 1, Int),
    b!(38, "stopall", [], 0, Int),
    b!(39, "playing", [Int], 1, Int),
    b!(40, "cue", [Int], 1, Int),
    b!(41, "cuecount", [Int], 1, Int, audio),
    b!(42, "pickcart", [Text], 1, Int, suspends),
    b!(43, "opensection", [Int, Text], 2, Int),
    b!(44, "cartname", [Int, ArrW], 2, Int),
    b!(45, "drawasset", [Int, Int, Int, Int], 4, Int),
    b!(46, "status", [Text], 1, Int),
    b!(47, "readasset", [Int, ArrW, Int], 3, Int),
    b!(48, "writeasset", [Int, ArrR, Int], 3, Int),
    b!(49, "assetcount", [Int], 1, Int),
    b!(50, "assetname", [Int, Int, ArrW], 3, Int),
    b!(51, "setassetname", [Int, Int, Text], 3, Int),
    b!(52, "savecart", [Int], 1, Int, suspends),
    // Maps.NAME[ty][tx]: map slot, cell x, cell y. Not callable by name.
    b!(53, "__mapcell", [Int, Int, Int], 3, Int, internal),
];

pub fn by_id(id: u8) -> Option<&'static Builtin> {
    BUILTINS.get(id as usize)
}

pub fn by_name(name: &str) -> Option<&'static Builtin> {
    BUILTINS.iter().find(|b| !b.internal && b.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ids_match_positions() {
        for (i, b) in BUILTINS.iter().enumerate() {
            assert_eq!(b.id as usize, i, "{}", b.name);
            assert!(b.min_args <= b.max_args());
        }
        assert_eq!(by_name("print").unwrap().id, id::PRINT);
        assert_eq!(by_id(id::MAPCELL).unwrap().name, "__mapcell");
        assert!(by_name("__mapcell").is_none());
    }
}
