//! The `.bdb` cart image: container, sections, and read-in-place views.
//!
//! Parsing never allocates and never panics on hostile input: every offset and
//! length is checked once here, so later accessors can index without surprises.

use crate::limits;

pub const MAGIC: [u8; 4] = *b"BDB\x1a";
pub const FORMAT_VERSION: u16 = 2;
pub const HEADER_SIZE: usize = 24;
pub const DIR_ENTRY_SIZE: usize = 12;
pub const NO_FN: u16 = 0xFFFF;
pub const NO_INIT: u16 = 0xFFFF;

/// Section kinds. A cart holds each kind at most once, in increasing order.
pub mod section {
    pub const META: u16 = 0x01;
    pub const PROGRAM: u16 = 0x02;
    pub const CODE: u16 = 0x03;
    pub const ROM: u16 = 0x04;
    pub const TEXT: u16 = 0x05;
    pub const DEBUG: u16 = 0x06;
    pub const SOURCE: u16 = 0x07;
    pub const SPRITES: u16 = 0x10;
    pub const MAPS: u16 = 0x11;
    pub const SOUNDS: u16 = 0x12;
    pub const MUSIC: u16 = 0x13;
    pub const PALETTE: u16 = 0x14;
    pub const ALL: [u16; 12] = [META, PROGRAM, CODE, ROM, TEXT, DEBUG, SOURCE, SPRITES, MAPS, SOUNDS, MUSIC, PALETTE];
}

/// Lifecycle handler slots in the program header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handler {
    Init = 0,
    Update = 1,
    Draw = 2,
    Audio = 3,
}

impl Handler {
    pub const ALL: [Handler; 4] = [Handler::Init, Handler::Update, Handler::Draw, Handler::Audio];
    /// Most parameters the handler may declare.
    pub const fn max_params(self) -> u8 {
        match self {
            Handler::Init => 0,
            Handler::Update | Handler::Draw => 2,
            Handler::Audio => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadError {
    TooShort,
    BadMagic,
    UnsupportedVersion(u16),
    BadLength,
    BadChecksum,
    BadDirectory,
    MissingSection(u16),
    BadSection(u16, &'static str),
}

impl core::fmt::Display for LoadError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            LoadError::TooShort => write!(f, "file too short to be a cart"),
            LoadError::BadMagic => write!(f, "not a Bidule cart (bad magic)"),
            LoadError::UnsupportedVersion(v) => write!(f, "cart format version {} is not supported (expected {})", v, FORMAT_VERSION),
            LoadError::BadLength => write!(f, "cart length does not match its header"),
            LoadError::BadChecksum => write!(f, "cart checksum mismatch"),
            LoadError::BadDirectory => write!(f, "malformed section directory"),
            LoadError::MissingSection(k) => write!(f, "missing section 0x{:02x}", k),
            LoadError::BadSection(k, why) => write!(f, "section 0x{:02x}: {}", k, why),
        }
    }
}

#[inline]
pub(crate) fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
#[inline]
pub(crate) fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
#[inline]
pub(crate) fn i32_at(b: &[u8], o: usize) -> i32 {
    u32_at(b, o) as i32
}

/// CRC-32 (IEEE 802.3, reflected, as in zlib).
pub fn crc32(data: &[u8]) -> u32 {
    !crc32_update(!0, data)
}

fn crc32_update(mut crc: u32, data: &[u8]) -> u32 {
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (crc & 1).wrapping_neg());
        }
    }
    crc
}

/// Checksum of a whole cart, computed with the checksum field (bytes 16–19) as zero.
pub fn cart_crc(b: &[u8]) -> u32 {
    let c = crc32_update(!0, &b[..16]);
    let c = crc32_update(c, &[0; 4]);
    !crc32_update(c, &b[20..])
}

const SLOTS: usize = section::ALL.len();

fn slot_of(kind: u16) -> Option<usize> {
    section::ALL.iter().position(|&k| k == kind)
}

/// A cart image whose container has been checked: magic, version, length,
/// checksum, and a well-formed section directory.
#[derive(Clone, Copy, Debug)]
pub struct Cart<'a> {
    pub compiler_version: u16,
    sections: [Option<&'a [u8]>; SLOTS],
}

impl<'a> Cart<'a> {
    pub fn parse(b: &'a [u8]) -> Result<Cart<'a>, LoadError> {
        if b.len() < HEADER_SIZE {
            return Err(LoadError::TooShort);
        }
        if b[0..4] != MAGIC {
            return Err(LoadError::BadMagic);
        }
        let version = u16_at(b, 4);
        if version != FORMAT_VERSION {
            return Err(LoadError::UnsupportedVersion(version));
        }
        if u16_at(b, 6) != 0 || u32_at(b, 20) != 0 {
            return Err(LoadError::BadDirectory);
        }
        let compiler_version = u16_at(b, 8);
        let count = u16_at(b, 10) as usize;
        let total = u32_at(b, 12) as usize;
        if total != b.len() || total > limits::CART_BYTES {
            return Err(LoadError::BadLength);
        }
        if cart_crc(b) != u32_at(b, 16) {
            return Err(LoadError::BadChecksum);
        }
        if count > SLOTS {
            return Err(LoadError::BadDirectory);
        }
        let dir_end = HEADER_SIZE + count * DIR_ENTRY_SIZE;
        if dir_end > b.len() {
            return Err(LoadError::BadDirectory);
        }
        let mut sections = [None; SLOTS];
        let mut prev_kind = 0u16;
        // Sections follow the directory, in order, 4-byte aligned, without gaps
        // beyond alignment padding: one canonical encoding per cart.
        let mut expect = align4(dir_end);
        for i in 0..count {
            let e = HEADER_SIZE + i * DIR_ENTRY_SIZE;
            let kind = u16_at(b, e);
            let flags = u16_at(b, e + 2);
            let off = u32_at(b, e + 4) as usize;
            let len = u32_at(b, e + 8) as usize;
            if kind <= prev_kind || flags != 0 {
                return Err(LoadError::BadDirectory);
            }
            prev_kind = kind;
            let slot = slot_of(kind).ok_or(LoadError::BadDirectory)?;
            if off != expect || off.checked_add(len).is_none_or(|end| end > b.len()) {
                return Err(LoadError::BadDirectory);
            }
            sections[slot] = Some(&b[off..off + len]);
            let end = off + len;
            expect = align4(end);
            if expect > b.len() || b[end..expect].iter().any(|&x| x != 0) {
                return Err(LoadError::BadDirectory);
            }
        }
        // the file ends right after the last section's padding
        if expect != b.len() {
            return Err(LoadError::BadLength);
        }
        Ok(Cart {
            compiler_version,
            sections,
        })
    }

    pub fn section(&self, kind: u16) -> Option<&'a [u8]> {
        slot_of(kind).and_then(|s| self.sections[s])
    }
}

pub const fn align4(n: usize) -> usize {
    (n + 3) & !3
}

/// One pool array of the program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArrayInfo {
    pub offset: u16,
    pub len: u16,
    /// 0 for a 1D array; otherwise the row length (len is a multiple of it).
    pub cols: u16,
    /// Constant table holding the initial values, or `NO_INIT`.
    pub init: u16,
}

impl ArrayInfo {
    pub const fn rows(&self) -> u16 {
        if self.cols == 0 {
            self.len
        } else {
            self.len / self.cols
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FnInfo {
    pub start: u32,
    pub end: u32,
    pub params: u8,
    pub locals: u8,
    pub max_stack: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableInfo {
    pub offset: u32,
    pub count: u16,
    pub cols: u16,
    pub width: u8,
    pub signed: bool,
}

impl TableInfo {
    pub const fn rows(&self) -> u16 {
        if self.cols == 0 {
            self.count
        } else {
            self.count / self.cols
        }
    }
}

/// Constant tables, read in place.
#[derive(Clone, Copy, Debug)]
pub struct Rom<'a> {
    bytes: &'a [u8],
    count: u16,
}

impl<'a> Rom<'a> {
    pub const EMPTY: Rom<'static> = Rom { bytes: &[], count: 0 };
    const ENTRY: usize = 12;

    pub fn from_bytes(b: &'a [u8]) -> Result<Rom<'a>, LoadError> {
        let bad = |why| LoadError::BadSection(section::ROM, why);
        if b.len() < 4 {
            return Err(bad("too short"));
        }
        let count = u16_at(b, 0);
        if count as usize > limits::ROM_TABLES || u16_at(b, 2) != 0 {
            return Err(bad("too many tables"));
        }
        let dir_end = 4 + count as usize * Self::ENTRY;
        if dir_end > b.len() {
            return Err(bad("directory past end"));
        }
        let rom = Rom { bytes: b, count };
        for i in 0..count {
            let e = 4 + i as usize * Self::ENTRY;
            let t = rom.table_unchecked(i);
            if !matches!(t.width, 1 | 2 | 4) || b[e + 9] > 1 || u16_at(b, e + 10) != 0 {
                return Err(bad("bad table width"));
            }
            if t.cols != 0 && !t.count.is_multiple_of(t.cols) {
                return Err(bad("table size is not a whole number of rows"));
            }
            let size = t.count as usize * t.width as usize;
            let off = t.offset as usize;
            if off < dir_end || off.checked_add(size).is_none_or(|end| end > b.len()) {
                return Err(bad("table data out of range"));
            }
        }
        Ok(rom)
    }

    pub fn count(&self) -> u16 {
        self.count
    }

    fn table_unchecked(&self, i: u16) -> TableInfo {
        let e = 4 + i as usize * Self::ENTRY;
        let b = self.bytes;
        TableInfo {
            offset: u32_at(b, e),
            count: u16_at(b, e + 4),
            cols: u16_at(b, e + 6),
            width: b[e + 8],
            signed: b[e + 9] == 1,
        }
    }

    pub fn table(&self, i: u16) -> Option<TableInfo> {
        if i < self.count {
            Some(self.table_unchecked(i))
        } else {
            None
        }
    }

    /// Element `idx` of a table already known to exist; `None` out of bounds.
    pub fn get(&self, t: &TableInfo, idx: i32) -> Option<i32> {
        if idx < 0 || idx >= t.count as i32 {
            return None;
        }
        let o = t.offset as usize + idx as usize * t.width as usize;
        let b = self.bytes;
        Some(match (t.width, t.signed) {
            (1, false) => b[o] as i32,
            (1, true) => b[o] as i8 as i32,
            (2, false) => u16_at(b, o) as i32,
            (2, true) => u16_at(b, o) as i16 as i32,
            _ => i32_at(b, o),
        })
    }
}

/// Text templates for interpolated strings.
#[derive(Clone, Copy, Debug)]
pub struct Templates<'a> {
    bytes: &'a [u8],
    count: u16,
}

/// One piece of a template.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Piece<'a> {
    Lit(&'a [u8]),
    Arg(Format),
}

/// Value type of a template argument.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArgType {
    Int = 0,
    Fix = 1,
    Text = 2,
}

/// `{value:format}`: width, zero padding, decimals, hexadecimal, character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    pub ty: ArgType,
    pub zero: bool,
    pub hex: bool,
    pub chr: bool,
    pub width: u8,
    /// Fixed number of decimals, if given.
    pub decimals: Option<u8>,
}

pub mod fmt_flags {
    pub const ZERO: u8 = 1;
    pub const HEX: u8 = 2;
    pub const CHAR: u8 = 4;
    pub const DECIMALS: u8 = 8;
}

pub const PIECE_END: u8 = 0;
pub const PIECE_LIT: u8 = 1;
pub const PIECE_ARG: u8 = 2;
/// Most decimals a format may ask for.
pub const MAX_DECIMALS: u8 = 8;

impl<'a> Templates<'a> {
    pub const EMPTY: Templates<'static> = Templates { bytes: &[], count: 0 };

    pub fn from_bytes(b: &'a [u8]) -> Result<Templates<'a>, LoadError> {
        let bad = |why| LoadError::BadSection(section::TEXT, why);
        if b.len() < 4 {
            return Err(bad("too short"));
        }
        let count = u16_at(b, 0);
        if count as usize > limits::TEMPLATES || u16_at(b, 2) != 0 {
            return Err(bad("too many templates"));
        }
        if 4 + count as usize * 4 > b.len() {
            return Err(bad("directory past end"));
        }
        let t = Templates { bytes: b, count };
        for i in 0..count {
            let off = u32_at(b, 4 + i as usize * 4) as usize;
            if off >= b.len() {
                return Err(bad("template out of range"));
            }
            let argc = b[off];
            let mut n = 0u32;
            let mut p = off + 1;
            loop {
                let tag = *b.get(p).ok_or(bad("template past end"))?;
                match tag {
                    PIECE_END => break,
                    PIECE_LIT => {
                        let len = *b.get(p + 1).ok_or(bad("template past end"))? as usize;
                        if len == 0 || p + 2 + len > b.len() {
                            return Err(bad("bad literal"));
                        }
                        p += 2 + len;
                    }
                    PIECE_ARG => {
                        if p + 5 > b.len() {
                            return Err(bad("template past end"));
                        }
                        if b[p + 1] > 2 || b[p + 2] & !0x0F != 0 || b[p + 4] > MAX_DECIMALS {
                            return Err(bad("bad format"));
                        }
                        n += 1;
                        p += 5;
                    }
                    _ => return Err(bad("bad piece")),
                }
            }
            if n != argc as u32 || argc as usize > limits::TEMPLATE_ARGS {
                return Err(bad("argument count mismatch"));
            }
        }
        Ok(t)
    }

    pub fn count(&self) -> u16 {
        self.count
    }

    /// Argument count of a template already known to exist.
    pub fn argc(&self, i: u16) -> u8 {
        self.bytes[u32_at(self.bytes, 4 + i as usize * 4) as usize]
    }

    pub fn pieces(&self, i: u16) -> PieceIter<'a> {
        let off = u32_at(self.bytes, 4 + i as usize * 4) as usize;
        PieceIter { b: self.bytes, p: off + 1 }
    }
}

pub struct PieceIter<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Iterator for PieceIter<'a> {
    type Item = Piece<'a>;
    fn next(&mut self) -> Option<Piece<'a>> {
        let b = self.b;
        match b[self.p] {
            PIECE_LIT => {
                let len = b[self.p + 1] as usize;
                let s = &b[self.p + 2..self.p + 2 + len];
                self.p += 2 + len;
                Some(Piece::Lit(s))
            }
            PIECE_ARG => {
                let ty = match b[self.p + 1] {
                    0 => ArgType::Int,
                    1 => ArgType::Fix,
                    _ => ArgType::Text,
                };
                let fl = b[self.p + 2];
                let f = Format {
                    ty,
                    zero: fl & fmt_flags::ZERO != 0,
                    hex: fl & fmt_flags::HEX != 0,
                    chr: fl & fmt_flags::CHAR != 0,
                    width: b[self.p + 3],
                    decimals: if fl & fmt_flags::DECIMALS != 0 { Some(b[self.p + 4]) } else { None },
                };
                self.p += 5;
                Some(Piece::Arg(f))
            }
            _ => None,
        }
    }
}

/// Optional debug section: names and a line table.
#[derive(Clone, Copy, Debug)]
pub struct Debug<'a> {
    names: &'a [u8],
    name_count: u16,
    lines: &'a [u8],
    line_count: u32,
}

impl<'a> Debug<'a> {
    fn parse(b: &'a [u8], arrays: u16, functions: u16) -> Result<Debug<'a>, LoadError> {
        let bad = |why| LoadError::BadSection(section::DEBUG, why);
        if b.len() < 2 {
            return Err(bad("too short"));
        }
        let name_count = u16_at(b, 0);
        if name_count != arrays + functions {
            return Err(bad("name count mismatch"));
        }
        let mut p = 2;
        for _ in 0..name_count {
            let len = *b.get(p).ok_or(bad("names past end"))? as usize;
            p += 1 + len;
        }
        if p > b.len() {
            return Err(bad("names past end"));
        }
        let names = &b[2..p];
        p = align4(p);
        if p + 4 > b.len() {
            return Err(bad("no line table"));
        }
        let line_count = u32_at(b, p);
        let lines = b.get(p + 4..).ok_or(bad("lines past end"))?;
        if (line_count as usize).checked_mul(8) != Some(lines.len()) {
            return Err(bad("line table size"));
        }
        Ok(Debug {
            names,
            name_count,
            lines,
            line_count,
        })
    }

    /// Debug name `i`: arrays first (by id), then functions.
    pub fn name(&self, i: u16) -> &'a [u8] {
        if i >= self.name_count {
            return &[];
        }
        let mut p = 0;
        for _ in 0..i {
            p += 1 + self.names[p] as usize;
        }
        let len = self.names[p] as usize;
        &self.names[p + 1..p + 1 + len]
    }

    /// Source (line, column) for a code offset: the last entry at or before it.
    pub fn line_at(&self, pc: u32) -> Option<(u16, u16)> {
        let (mut lo, mut hi) = (0u32, self.line_count);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if u32_at(self.lines, mid as usize * 8) <= pc {
                lo = mid + 1
            } else {
                hi = mid
            }
        }
        if lo == 0 {
            return None;
        }
        let e = (lo - 1) as usize * 8;
        Some((u16_at(self.lines, e + 4), u16_at(self.lines, e + 6)))
    }
}

/// The program of a cart: everything the VM needs, checked for internal
/// consistency (but the code itself is checked by [`crate::verify`]).
#[derive(Clone, Copy, Debug)]
pub struct Program<'a> {
    pub globals: u16,
    pub arrays: u16,
    pub functions: u16,
    pub audio_vars: u16,
    pub handlers: [u16; 4],
    global_init: &'a [u8],
    audio_init: &'a [u8],
    array_table: &'a [u8],
    fn_table: &'a [u8],
    pub code: &'a [u8],
    pub rom: Rom<'a>,
    pub text: Templates<'a>,
    pub debug: Option<Debug<'a>>,
}

pub const PROGRAM_HEADER: usize = 16;
pub const ARRAY_ENTRY: usize = 8;
pub const FN_ENTRY: usize = 8;

impl<'a> Program<'a> {
    pub fn from_cart(cart: &Cart<'a>) -> Result<Program<'a>, LoadError> {
        let p = cart.section(section::PROGRAM).ok_or(LoadError::MissingSection(section::PROGRAM))?;
        let code = cart.section(section::CODE).ok_or(LoadError::MissingSection(section::CODE))?;
        let rom = match cart.section(section::ROM) {
            Some(b) => Rom::from_bytes(b)?,
            None => Rom::EMPTY,
        };
        let text = match cart.section(section::TEXT) {
            Some(b) => Templates::from_bytes(b)?,
            None => Templates::EMPTY,
        };
        Self::parse(p, code, rom, text, cart.section(section::DEBUG))
    }

    pub fn parse(
        p: &'a [u8],
        code: &'a [u8],
        rom: Rom<'a>,
        text: Templates<'a>,
        debug: Option<&'a [u8]>,
    ) -> Result<Program<'a>, LoadError> {
        let bad = |why| LoadError::BadSection(section::PROGRAM, why);
        if p.len() < PROGRAM_HEADER {
            return Err(bad("too short"));
        }
        let globals = u16_at(p, 0);
        let arrays = u16_at(p, 2);
        let functions = u16_at(p, 4);
        let audio_vars = u16_at(p, 6);
        if globals as usize > limits::GLOBALS
            || arrays as usize > limits::ARRAYS
            || functions as usize > limits::FUNCTIONS
            || audio_vars as usize > limits::AUDIO_VARS
        {
            return Err(bad("limit exceeded"));
        }
        if code.len() > limits::CODE_BYTES {
            return Err(LoadError::BadSection(section::CODE, "code too large"));
        }
        let mut handlers = [NO_FN; 4];
        for (i, h) in handlers.iter_mut().enumerate() {
            *h = u16_at(p, 8 + i * 2);
            if *h != NO_FN && *h >= functions {
                return Err(bad("handler is not a function"));
            }
        }
        let mut o = PROGRAM_HEADER;
        let take = |o: &mut usize, n: usize| -> Result<&'a [u8], LoadError> {
            let s = p.get(*o..*o + n).ok_or(bad("tables past end"))?;
            *o += n;
            Ok(s)
        };
        let global_init = take(&mut o, globals as usize * 4)?;
        let audio_init = take(&mut o, audio_vars as usize * 4)?;
        let array_table = take(&mut o, arrays as usize * ARRAY_ENTRY)?;
        let fn_table = take(&mut o, functions as usize * FN_ENTRY)?;
        if o != p.len() {
            return Err(bad("trailing bytes"));
        }
        let debug = match debug {
            Some(b) => Some(Debug::parse(b, arrays, functions)?),
            None => None,
        };
        let prog = Program {
            globals,
            arrays,
            functions,
            audio_vars,
            handlers,
            global_init,
            audio_init,
            array_table,
            fn_table,
            code,
            rom,
            text,
            debug,
        };
        // arrays: inside the pool, initializers fit
        for i in 0..arrays {
            let a = prog.array(i as u8);
            if a.offset as usize + a.len as usize > limits::POOL || a.len == 0 {
                return Err(bad("array outside the pool"));
            }
            if a.cols != 0 && !a.len.is_multiple_of(a.cols) {
                return Err(bad("array size is not a whole number of rows"));
            }
            if a.init != NO_INIT {
                let t = rom.table(a.init).ok_or(bad("array initializer is not a table"))?;
                if t.count > a.len {
                    return Err(bad("array initializer too long"));
                }
            }
        }
        // functions: contiguous code ranges covering the code section, in order
        let mut expect = 0u32;
        for i in 0..functions {
            let f = prog.function(i);
            if f.start != expect || f.end <= f.start || f.locals < f.params || f.locals as usize > limits::LOCALS {
                return Err(bad("bad function table"));
            }
            expect = f.end;
        }
        if expect as usize != code.len() {
            return Err(bad("functions don't cover the code"));
        }
        for h in Handler::ALL {
            let f = prog.handlers[h as usize];
            if f != NO_FN && prog.function(f).params > h.max_params() {
                return Err(bad("handler takes too many parameters"));
            }
        }
        Ok(prog)
    }

    pub fn global_init(&self, i: u16) -> i32 {
        i32_at(self.global_init, i as usize * 4)
    }
    pub fn audio_init(&self, i: u16) -> i32 {
        i32_at(self.audio_init, i as usize * 4)
    }
    pub fn array(&self, i: u8) -> ArrayInfo {
        let e = i as usize * ARRAY_ENTRY;
        let b = self.array_table;
        ArrayInfo {
            offset: u16_at(b, e),
            len: u16_at(b, e + 2),
            cols: u16_at(b, e + 4),
            init: u16_at(b, e + 6),
        }
    }
    /// Function `i`; its end is the next function's start.
    pub fn function(&self, i: u16) -> FnInfo {
        let e = i as usize * FN_ENTRY;
        let b = self.fn_table;
        let end = if i + 1 < self.functions {
            u32_at(b, e + FN_ENTRY)
        } else {
            self.code.len() as u32
        };
        FnInfo {
            start: u32_at(b, e),
            end,
            params: b[e + 4],
            locals: b[e + 5],
            max_stack: b[e + 6],
        }
    }
    pub fn handler(&self, h: Handler) -> Option<u16> {
        let f = self.handlers[h as usize];
        if f == NO_FN {
            None
        } else {
            Some(f)
        }
    }
    /// Which handler, if any, function `f` is.
    pub fn handler_of(&self, f: u16) -> Option<Handler> {
        Handler::ALL.into_iter().find(|&h| self.handlers[h as usize] == f)
    }
    /// Debug name of array `i`, if the cart has a debug section.
    pub fn array_name(&self, i: u16) -> &'a [u8] {
        self.debug.map_or(&[], |d| d.name(i))
    }
    pub fn function_name(&self, i: u16) -> &'a [u8] {
        self.debug.map_or(&[], |d| d.name(self.arrays + i))
    }
}

/// Byte-level writers, used by the compiler and tests.
#[cfg(feature = "alloc")]
pub mod write {
    use super::*;
    use alloc::vec::Vec;

    pub fn put_u16(v: &mut Vec<u8>, x: u16) {
        v.extend_from_slice(&x.to_le_bytes());
    }
    pub fn put_u32(v: &mut Vec<u8>, x: u32) {
        v.extend_from_slice(&x.to_le_bytes());
    }
    pub fn put_i32(v: &mut Vec<u8>, x: i32) {
        v.extend_from_slice(&x.to_le_bytes());
    }

    /// Assembles a cart from `(kind, bytes)` sections (any order; sorted here).
    pub fn cart(compiler_version: u16, sections: &mut [(u16, Vec<u8>)]) -> Vec<u8> {
        sections.sort_by_key(|s| s.0);
        let mut out = Vec::new();
        out.extend_from_slice(&MAGIC);
        put_u16(&mut out, FORMAT_VERSION);
        put_u16(&mut out, 0);
        put_u16(&mut out, compiler_version);
        put_u16(&mut out, sections.len() as u16);
        put_u32(&mut out, 0); // total, patched below
        put_u32(&mut out, 0); // crc, patched below
        put_u32(&mut out, 0);
        let mut off = align4(HEADER_SIZE + sections.len() * DIR_ENTRY_SIZE);
        for (kind, data) in sections.iter() {
            put_u16(&mut out, *kind);
            put_u16(&mut out, 0);
            put_u32(&mut out, off as u32);
            put_u32(&mut out, data.len() as u32);
            off = align4(off + data.len());
        }
        let last = sections.len().saturating_sub(1);
        for (i, (_, data)) in sections.iter().enumerate() {
            while out.len() % 4 != 0 {
                out.push(0);
            }
            out.extend_from_slice(data);
            if i == last {
                while out.len() % 4 != 0 {
                    out.push(0);
                }
            }
        }
        let total = out.len() as u32;
        out[12..16].copy_from_slice(&total.to_le_bytes());
        let crc = cart_crc(&out);
        out[16..20].copy_from_slice(&crc.to_le_bytes());
        out
    }

    /// One constant table before encoding.
    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub struct Table {
        pub values: Vec<i32>,
        pub cols: u16,
    }

    /// Narrowest (width, signed) that holds every value.
    pub fn packing(values: &[i32]) -> (u8, bool) {
        let (lo, hi) = values.iter().fold((0i32, 0i32), |(lo, hi), &v| (lo.min(v), hi.max(v)));
        if lo >= 0 && hi <= 255 {
            (1, false)
        } else if lo >= -128 && hi <= 127 {
            (1, true)
        } else if lo >= 0 && hi <= 65535 {
            (2, false)
        } else if lo >= -32768 && hi <= 32767 {
            (2, true)
        } else {
            (4, true)
        }
    }

    pub fn rom(tables: &[Table]) -> Vec<u8> {
        let mut out = Vec::new();
        put_u16(&mut out, tables.len() as u16);
        put_u16(&mut out, 0);
        let mut data = Vec::new();
        let base = 4 + tables.len() * 12;
        for t in tables {
            let (w, s) = packing(&t.values);
            while !(base + data.len()).is_multiple_of(w as usize) {
                data.push(0);
            }
            put_u32(&mut out, (base + data.len()) as u32);
            put_u16(&mut out, t.values.len() as u16);
            put_u16(&mut out, t.cols);
            out.push(w);
            out.push(s as u8);
            put_u16(&mut out, 0);
            for &v in &t.values {
                match w {
                    1 => data.push(v as u8),
                    2 => put_u16(&mut data, v as u16),
                    _ => put_i32(&mut data, v),
                }
            }
        }
        out.extend_from_slice(&data);
        out
    }

    pub fn templates(templates: &[Vec<u8>]) -> Vec<u8> {
        let mut out = Vec::new();
        put_u16(&mut out, templates.len() as u16);
        put_u16(&mut out, 0);
        let mut off = 4 + templates.len() * 4;
        for t in templates {
            put_u32(&mut out, off as u32);
            off += t.len();
        }
        for t in templates {
            out.extend_from_slice(t);
        }
        out
    }

    pub struct FnEntry {
        pub start: u32,
        pub params: u8,
        pub locals: u8,
        pub max_stack: u8,
    }

    pub struct ProgramTables<'a> {
        pub globals: &'a [i32],
        pub audio_vars: &'a [i32],
        pub arrays: &'a [ArrayInfo],
        pub functions: &'a [FnEntry],
        pub handlers: [u16; 4],
    }

    pub fn program(t: &ProgramTables) -> Vec<u8> {
        let mut out = Vec::new();
        put_u16(&mut out, t.globals.len() as u16);
        put_u16(&mut out, t.arrays.len() as u16);
        put_u16(&mut out, t.functions.len() as u16);
        put_u16(&mut out, t.audio_vars.len() as u16);
        for h in t.handlers {
            put_u16(&mut out, h);
        }
        for &g in t.globals {
            put_i32(&mut out, g);
        }
        for &g in t.audio_vars {
            put_i32(&mut out, g);
        }
        for a in t.arrays {
            put_u16(&mut out, a.offset);
            put_u16(&mut out, a.len);
            put_u16(&mut out, a.cols);
            put_u16(&mut out, a.init);
        }
        for f in t.functions {
            put_u32(&mut out, f.start);
            out.push(f.params);
            out.push(f.locals);
            out.push(f.max_stack);
            out.push(0);
        }
        out
    }

    /// Debug section: array names, function names, then (code offset, line, column).
    pub fn debug(names: &[&[u8]], lines: &[(u32, u16, u16)]) -> Vec<u8> {
        let mut out = Vec::new();
        put_u16(&mut out, names.len() as u16);
        for n in names {
            let n = &n[..n.len().min(255)];
            out.push(n.len() as u8);
            out.extend_from_slice(n);
        }
        while out.len() % 4 != 0 {
            out.push(0);
        }
        put_u32(&mut out, lines.len() as u32);
        for &(pc, line, col) in lines {
            put_u32(&mut out, pc);
            put_u16(&mut out, line);
            put_u16(&mut out, col);
        }
        out
    }

    /// Metadata: `key=value` pairs, each length-prefixed.
    pub fn meta(pairs: &[(&str, &str)]) -> Vec<u8> {
        let mut out = Vec::new();
        out.push(pairs.len() as u8);
        for (k, v) in pairs {
            let v = &v.as_bytes()[..v.len().min(255)];
            out.push(k.len() as u8);
            out.extend_from_slice(k.as_bytes());
            out.push(v.len() as u8);
            out.extend_from_slice(v);
        }
        out
    }
}

/// Reads one metadata value (`title`, `author`, ...) from a META section.
pub fn meta_value<'a>(meta: &'a [u8], key: &str) -> Option<&'a [u8]> {
    let n = *meta.first()? as usize;
    let mut p = 1;
    for _ in 0..n {
        let kl = *meta.get(p)? as usize;
        let k = meta.get(p + 1..p + 1 + kl)?;
        p += 1 + kl;
        let vl = *meta.get(p)? as usize;
        let v = meta.get(p + 1..p + 1 + vl)?;
        p += 1 + vl;
        if k == key.as_bytes() {
            return Some(v);
        }
    }
    None
}

#[cfg(all(test, feature = "alloc"))]
mod tests {
    use super::write::*;
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    #[test]
    fn crc_matches_zlib() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn container_round_trip_and_tamper() {
        let mut secs = vec![(section::CODE, vec![1u8, 2, 3]), (section::META, meta(&[("title", "T")]))];
        let bytes = cart(0x0001, &mut secs);
        let c = Cart::parse(&bytes).unwrap();
        assert_eq!(c.section(section::CODE), Some(&[1u8, 2, 3][..]));
        assert_eq!(meta_value(c.section(section::META).unwrap(), "title"), Some(&b"T"[..]));
        for i in 0..bytes.len() {
            let mut t: Vec<u8> = bytes.clone();
            t[i] ^= 0x40;
            assert!(Cart::parse(&t).is_err(), "flip at {} accepted", i);
        }
        assert!(Cart::parse(&bytes[..bytes.len() - 1]).is_err());
    }

    #[test]
    fn rom_packs_narrowest() {
        let tables = [
            Table {
                values: vec![0, 3, 7, 255],
                cols: 0,
            },
            Table {
                values: vec![-1, 300],
                cols: 0,
            },
            Table {
                values: vec![70000],
                cols: 0,
            },
        ];
        let b = rom(&tables);
        let r = Rom::from_bytes(&b).unwrap();
        assert_eq!(r.table(0).unwrap().width, 1);
        assert_eq!(r.table(1).unwrap().width, 2);
        assert!(r.table(1).unwrap().signed);
        let t = r.table(1).unwrap();
        assert_eq!(r.get(&t, 0), Some(-1));
        assert_eq!(r.get(&t, 1), Some(300));
        assert_eq!(r.get(&t, 2), None);
        let t = r.table(2).unwrap();
        assert_eq!(r.get(&t, 0), Some(70000));
    }
}
