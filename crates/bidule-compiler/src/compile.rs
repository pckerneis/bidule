//! Declarations, constants, records, globals and the final cart image.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use bidule_bytecode::asset::{self, AssetSection};
use bidule_bytecode::image::{self, section, write, ArrayInfo, Handler, NO_FN, NO_INIT};
use bidule_bytecode::{limits, verify, COMPILER_VERSION};

use crate::ast::*;
use crate::diag::{suggest, Diag, Pos};
use crate::gen::FnState;
use crate::parser;
use crate::types::*;

/// Raw asset sections to pack into the cart; their name tables give `Sprites.NAME` etc.
#[derive(Clone, Copy, Debug, Default)]
pub struct Assets<'a> {
    pub sprites: Option<&'a [u8]>,
    pub maps: Option<&'a [u8]>,
    pub sounds: Option<&'a [u8]>,
    pub music: Option<&'a [u8]>,
    pub palette: Option<&'a [u8]>,
}

#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Emit the debug section (line table and names). On by default.
    pub debug_info: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { debug_info: true }
    }
}

#[derive(Clone, Debug)]
pub struct Output {
    pub cart: Vec<u8>,
    pub warnings: Vec<Diag>,
}

pub type R<T> = Result<T, Diag>;

/// Built-in constants (spec: Display › Default palette, Input, Language › Constants).
pub const BUILTIN_CONSTS: &[(&str, i32)] = &[
    ("BTN_LEFT", 0),
    ("BTN_RIGHT", 1),
    ("BTN_UP", 2),
    ("BTN_DOWN", 3),
    ("BTN_A", 4),
    ("BTN_B", 5),
    ("BTN_X", 6),
    ("BTN_Y", 7),
    ("SCREEN_W", 240),
    ("SCREEN_H", 160),
    ("BLACK", 0),
    ("DARK_BLUE", 1),
    ("PLUM", 2),
    ("DARK_GREEN", 3),
    ("BROWN", 4),
    ("DARK_GREY", 5),
    ("GREY", 6),
    ("WHITE", 7),
    ("RED", 8),
    ("ORANGE", 9),
    ("YELLOW", 10),
    ("GREEN", 11),
    ("BLUE", 12),
    ("CYAN", 13),
    ("MAGENTA", 14),
    ("SAND", 15),
    ("SQUARE", 0),
    ("TRIANGLE", 1),
    ("SAW", 2),
    ("NOISE", 3),
];

pub const ASSET_PREFIXES: [&str; 4] = ["Sprites", "Maps", "Sounds", "Music"];
/// Names reserved for conversions (`int(x)`, `round(x)`, `frac(x)`).
pub const INTRINSICS: [&str; 3] = ["int", "round", "frac"];

/// Source `sprite` declarations are passed to `spr` as this tagged value:
/// bit 30 set, tile in bits 0–9, width − 1 in bits 10–14, height − 1 in bits 15–19.
pub fn sprite_value(tile: i32, w: i32, h: i32) -> i32 {
    0x4000_0000 | ((h - 1) << 15) | ((w - 1) << 10) | tile
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConstState {
    Pending(usize),
    Visiting,
    Done,
}

pub struct Compiler<'p, 'a> {
    pub prog: &'p Program<'a>,
    pub source: &'a str,
    pub opts: Options,
    pub diags: Vec<Diag>,
    /// Every declared name, with where it was declared.
    pub syms: BTreeMap<&'a str, (Sym, Pos)>,
    consts: BTreeMap<&'a str, ConstState>,
    pub records: Vec<RecDef<'a>>,
    pub enums: Vec<EnumDef<'a>>,
    pub assets: [Option<AssetSection<'a>>; 4],
    raw_assets: Assets<'a>,
    pub globals: Vec<i32>,
    pub audio_vars: Vec<i32>,
    pub arrays: Vec<ArrayInfo>,
    pub array_names: Vec<String>,
    pool_used: u32,
    pub tables: Vec<write::Table>,
    /// The tables made for string literals, shared by equal literals.
    text_tables: Vec<u16>,
    pub templates: Vec<Vec<u8>>,
    pub funcs: Vec<FnSig<'a>>,
    pub code: Vec<u8>,
    pub lines: Vec<(u32, u16, u16)>,
    pub fn_entries: Vec<write::FnEntry>,
    pub fn_names: Vec<&'a str>,
    pub handlers: [u16; 4],
    pub f: FnState<'a>,
}

/// A compiled program waiting to be written out as a `.bdb` image.
pub struct Parts<'a> {
    source: &'a str,
    opts: Options,
    raw_assets: Assets<'a>,
    meta: Vec<(&'a str, &'a str)>,
    globals: Vec<i32>,
    audio_vars: Vec<i32>,
    arrays: Vec<ArrayInfo>,
    array_names: Vec<String>,
    tables: Vec<write::Table>,
    templates: Vec<Vec<u8>>,
    code: Vec<u8>,
    lines: Vec<(u32, u16, u16)>,
    fn_entries: Vec<write::FnEntry>,
    fn_names: Vec<&'a str>,
    handlers: [u16; 4],
}

impl<'a> Parts<'a> {
    /// The image. Sections are written straight into it, the source and the
    /// asset sections included, without intermediate copies.
    pub fn assemble(self) -> R<Vec<u8>> {
        let (has_rom, has_text) = (!self.tables.is_empty(), !self.templates.is_empty());
        let rom = write::rom(&self.tables);
        drop(self.tables);
        let text = write::templates(&self.templates);
        drop(self.templates);
        let tables = write::ProgramTables {
            globals: &self.globals,
            audio_vars: &self.audio_vars,
            arrays: &self.arrays,
            functions: &self.fn_entries,
            handlers: self.handlers,
        };
        let mut program = write::program(&tables);
        {
            // the stack depth the verifier computes is the one we declare
            let rom_v = image::Rom::from_bytes(&rom).map_err(|e| Diag::error(Pos::default(), format!("internal: {}", e)))?;
            let text_v = image::Templates::from_bytes(&text).map_err(|e| Diag::error(Pos::default(), format!("internal: {}", e)))?;
            let p = image::Program::parse(&program, &self.code, rom_v, text_v, None)
                .map_err(|e| Diag::error(Pos::default(), format!("internal: {}", e)))?;
            // as long as the longest function, not the longest allowed
            let mut longest = 0;
            for (i, e) in self.fn_entries.iter().enumerate() {
                let end = self.fn_entries.get(i + 1).map_or(self.code.len(), |n| n.start as usize);
                longest = longest.max(end - e.start as usize);
            }
            let mut scratch = vec![0u8; longest.min(limits::FN_CODE_BYTES)];
            let mut maxes = Vec::with_capacity(p.functions as usize);
            for f in 0..p.functions {
                match verify::max_stack(&p, f, &mut scratch) {
                    Ok(m) => maxes.push(m),
                    Err(e) => {
                        let name = self.fn_names.get(f as usize).copied().unwrap_or("?");
                        return Err(Diag::error(Pos::default(), format!("internal compiler error in {}: {}", name, e)));
                    }
                }
            }
            let base = image::PROGRAM_HEADER + (self.globals.len() + self.audio_vars.len()) * 4 + self.arrays.len() * image::ARRAY_ENTRY;
            for (i, m) in maxes.into_iter().enumerate() {
                program[base + i * image::FN_ENTRY + 6] = m;
            }
        }
        let meta = write::meta(&self.meta);
        let debug = if self.opts.debug_info {
            let mut names: Vec<&[u8]> = self.array_names.iter().map(|s| s.as_bytes()).collect();
            names.extend(self.fn_names.iter().map(|s| s.as_bytes()));
            Some(write::debug(&names, &self.lines))
        } else {
            None
        };
        // only the sections themselves from here on
        drop((self.lines, self.array_names, self.fn_names, self.fn_entries, self.meta));
        drop((self.globals, self.audio_vars, self.arrays));
        let mut secs: Vec<(u16, &[u8])> = Vec::with_capacity(12);
        secs.push((section::META, &meta));
        secs.push((section::PROGRAM, &program));
        secs.push((section::CODE, &self.code));
        if has_rom {
            secs.push((section::ROM, &rom));
        }
        if has_text {
            secs.push((section::TEXT, &text));
        }
        if let Some(d) = &debug {
            secs.push((section::DEBUG, d));
        }
        secs.push((section::SOURCE, self.source.as_bytes()));
        let raw = self.raw_assets;
        for (b, k) in [
            (raw.sprites, section::SPRITES),
            (raw.maps, section::MAPS),
            (raw.sounds, section::SOUNDS),
            (raw.music, section::MUSIC),
            (raw.palette, section::PALETTE),
        ] {
            if let Some(b) = b {
                secs.push((k, b));
            }
        }
        Ok(write::cart(COMPILER_VERSION, &mut secs))
    }
}

impl<'p, 'a> Compiler<'p, 'a> {
    pub fn new(prog: &'p Program<'a>, source: &'a str, assets: &Assets<'a>, opts: Options) -> Self {
        Compiler {
            prog,
            source,
            opts,
            diags: Vec::new(),
            syms: BTreeMap::new(),
            consts: BTreeMap::new(),
            records: Vec::new(),
            enums: Vec::new(),
            assets: [None; 4],
            raw_assets: *assets,
            globals: Vec::new(),
            audio_vars: Vec::new(),
            arrays: Vec::new(),
            array_names: Vec::new(),
            pool_used: 0,
            tables: Vec::new(),
            text_tables: Vec::new(),
            templates: Vec::new(),
            funcs: Vec::new(),
            code: Vec::new(),
            lines: Vec::new(),
            fn_entries: Vec::new(),
            fn_names: Vec::new(),
            handlers: [NO_FN; 4],
            f: FnState::default(),
        }
    }

    pub fn error(&mut self, d: Diag) {
        self.diags.push(d);
    }
    pub fn warn(&mut self, pos: Pos, msg: impl Into<String>) {
        self.diags.push(Diag::warning(pos, msg));
    }

    // ---------- names ----------

    fn claim(&mut self, name: &Name<'a>, sym: Sym) -> R<()> {
        let n = name.text;
        if BUILTIN_CONSTS.iter().any(|c| c.0 == n) {
            return Err(Diag::error(name.pos, format!("'{}' is a built-in constant; pick another name", n)));
        }
        if bidule_bytecode::builtins::by_name(n).is_some() || INTRINSICS.contains(&n) {
            return Err(Diag::error(name.pos, format!("'{}' is a built-in function; pick another name", n)));
        }
        if ASSET_PREFIXES.contains(&n) || n == "Sound" || bidule_bytecode::builtins::is_reserved_name(n) {
            return Err(Diag::error(name.pos, format!("'{}' is reserved; pick another name", n)));
        }
        if let Some((_, p)) = self.syms.get(n) {
            return Err(Diag::error(name.pos, format!("'{}' is already declared on line {}", n, p.line)));
        }
        self.syms.insert(n, (sym, name.pos));
        Ok(())
    }

    pub fn sym(&self, name: &str) -> Option<&Sym> {
        self.syms.get(name).map(|e| &e.0)
    }

    /// "unknown X 'n' — did you mean 'm'?"
    pub fn unknown(&self, what: &str, name: &Name<'a>, extra: &[&str]) -> Diag {
        let cands = self
            .syms
            .keys()
            .copied()
            .chain(extra.iter().copied())
            .chain(BUILTIN_CONSTS.iter().map(|c| c.0));
        match suggest(name.text, cands) {
            Some(s) => Diag::error(name.pos, format!("unknown {} '{}' — did you mean '{}'?", what, name.text, s)),
            None => Diag::error(name.pos, format!("unknown {} '{}'", what, name.text)),
        }
    }

    // ---------- the whole program ----------

    /// Everything up to code generation. False if there were errors: then
    /// there is nothing to assemble.
    pub fn run(&mut self) -> bool {
        self.load_assets();
        self.declare_types();
        self.declare_values();
        self.analyse_writes();
        self.gen_functions();
        !self.diags.iter().any(|d| d.is_error())
    }

    /// What assembling the image needs, so the syntax tree and the symbol
    /// tables can be freed first.
    pub fn into_parts(self) -> Parts<'a> {
        Parts {
            source: self.source,
            opts: self.opts,
            raw_assets: self.raw_assets,
            meta: self.prog.meta.iter().map(|(k, v, _)| (*k, *v)).collect(),
            globals: self.globals,
            audio_vars: self.audio_vars,
            arrays: self.arrays,
            array_names: self.array_names,
            tables: self.tables,
            templates: self.templates,
            code: {
                let mut c = self.code;
                c.shrink_to_fit();
                c
            },
            lines: self.lines,
            fn_entries: self.fn_entries,
            fn_names: self.fn_names,
            handlers: self.handlers,
        }
    }

    fn load_assets(&mut self) {
        let raw = self.raw_assets;
        let secs = [
            (raw.sprites, section::SPRITES),
            (raw.maps, section::MAPS),
            (raw.sounds, section::SOUNDS),
            (raw.music, section::MUSIC),
        ];
        for (i, (b, kind)) in secs.into_iter().enumerate() {
            if let Some(b) = b {
                match AssetSection::parse(b) {
                    Ok(s) if s.kind as u16 == kind => self.assets[i] = Some(s),
                    Ok(_) => self.error(Diag::error(
                        Pos::default(),
                        format!("asset section for {} has the wrong kind", ASSET_PREFIXES[i]),
                    )),
                    Err(e) => self.error(Diag::error(Pos::default(), format!("{} section: {}", ASSET_PREFIXES[i], e))),
                }
            }
        }
        if let Some(p) = raw.palette {
            if let Err(e) = AssetSection::parse(p) {
                self.error(Diag::error(Pos::default(), format!("palette section: {}", e)));
            }
        }
    }

    /// Records and enums first: other declarations refer to them.
    fn declare_types(&mut self) {
        let prog = self.prog;
        for (i, it) in prog.items.iter().enumerate() {
            match it.kind {
                ItemKind::Record => {
                    let Ok(Item::Record { name, .. }) = parser::item_at(self.source, it) else {
                        continue;
                    };
                    let id = self.records.len();
                    self.records.push(RecDef {
                        name: name.text,
                        pos: name.pos,
                        item: i,
                        fields: Vec::new(),
                        leaves: Vec::new(),
                        state: RecState::Pending,
                    });
                    if let Err(d) = self.claim(&name, Sym::Record(id)) {
                        self.error(d);
                    }
                }
                ItemKind::Const => {
                    self.consts.insert(it.name, ConstState::Pending(i));
                }
                _ => {}
            }
        }
        for id in 0..self.records.len() {
            if let Err(d) = self.resolve_record(id) {
                self.error(d);
            }
        }
        for it in prog.items.iter().filter(|it| it.kind == ItemKind::Enum) {
            if let Ok(Item::Enum { name, members }) = parser::item_at(self.source, it) {
                let mut def = EnumDef {
                    name: name.text,
                    members: Vec::new(),
                };
                let mut next = 0i32;
                for (m, v) in &members {
                    if let Some(v) = v {
                        match self.const_int(v) {
                            Ok(x) => next = x,
                            Err(d) => self.error(d),
                        }
                    }
                    if def.members.iter().any(|x| x.0 == m.text) {
                        self.error(Diag::error(m.pos, format!("'{}' appears twice in enum {}", m.text, name.text)));
                    }
                    def.members.push((m.text, next));
                    next = next.wrapping_add(1);
                }
                let id = self.enums.len();
                self.enums.push(def);
                if let Err(d) = self.claim(&name, Sym::Enum(id)) {
                    self.error(d);
                }
            }
        }
    }

    pub fn resolve_record(&mut self, id: RecId) -> R<()> {
        match self.records[id].state {
            RecState::Done => return Ok(()),
            RecState::Resolving => {
                return Err(Diag::error(
                    self.records[id].pos,
                    format!("record {} contains itself", self.records[id].name),
                ));
            }
            RecState::Pending => {}
        }
        self.records[id].state = RecState::Resolving;
        let Ok(Item::Record { fields, .. }) = parser::item_at(self.source, &self.prog.items[self.records[id].item]) else {
            unreachable!()
        };
        let mut defs = Vec::new();
        let mut leaves = Vec::new();
        for f in &fields {
            if defs.iter().any(|d: &FieldDef| d.name == f.name.text) {
                return Err(Diag::error(f.name.pos, format!("field '{}' declared twice", f.name.text)));
            }
            let dims = self.dims_of(&f.shape.dims, f.name.pos, false)?;
            let count = dims.iter().product::<u32>().max(1);
            let leaf_off = leaves.len() as u16;
            let ty = match &f.shape.ty {
                TypeName::Int => FieldTy::Num(Num::Int),
                TypeName::Fix => FieldTy::Num(Num::Fix),
                TypeName::Record(n) => match self.sym(n.text) {
                    Some(Sym::Record(r)) => FieldTy::Rec(*r),
                    _ => return Err(self.unknown("record type", n, &[])),
                },
            };
            match ty {
                FieldTy::Num(num) => leaves.push(LeafDef {
                    num,
                    mult: count,
                    path: String::from(f.name.text),
                }),
                FieldTy::Rec(r) => {
                    self.resolve_record(r)?;
                    for l in &self.records[r].leaves {
                        leaves.push(LeafDef {
                            num: l.num,
                            mult: count * l.mult,
                            path: format!("{}.{}", f.name.text, l.path),
                        });
                    }
                }
            }
            let leaf_n = leaves.len() as u16 - leaf_off;
            defs.push(FieldDef {
                name: f.name.text,
                ty,
                dims,
                count,
                leaf_off,
                leaf_n,
            });
        }
        if leaves.is_empty() {
            return Err(Diag::error(self.records[id].pos, "a record needs at least one field"));
        }
        let r = &mut self.records[id];
        r.fields = defs;
        r.leaves = leaves;
        r.state = RecState::Done;
        Ok(())
    }

    /// Declared dimensions as numbers. `[]` is allowed only when `open`.
    pub fn dims_of(&mut self, dims: &[Option<Expr<'a>>], pos: Pos, open: bool) -> R<Vec<u32>> {
        let mut out = Vec::new();
        for d in dims {
            match d {
                None if open => out.push(0),
                None => return Err(Diag::error(pos, "this array needs a size")),
                Some(e) => {
                    let v = self.const_int(e)?;
                    if v <= 0 || v as usize > limits::TABLE_ELEMS {
                        return Err(Diag::error(
                            e.pos,
                            format!("array size must be 1 to {}, got {}", limits::TABLE_ELEMS, v),
                        ));
                    }
                    out.push(v as u32);
                }
            }
        }
        Ok(out)
    }

    // ---------- constants ----------

    /// Value of a constant scalar, evaluating it on first use (constants may be
    /// declared after their first use).
    pub fn const_sym(&mut self, name: &str) -> R<Option<Sym>> {
        let st = match self.consts.get(name) {
            Some(s) => *s,
            None => return Ok(None),
        };
        match st {
            ConstState::Done => Ok(self.sym(name).cloned()),
            ConstState::Visiting => Err(Diag::error(
                self.syms.get(name).map(|e| e.1).unwrap_or_default(),
                format!("constant '{}' depends on itself", name),
            )),
            ConstState::Pending(i) => {
                let Ok(Item::Const { name: n, shape, init }) = parser::item_at(self.source, &self.prog.items[i]) else {
                    unreachable!()
                };
                self.consts.insert(n.text, ConstState::Visiting);
                let r = self.declare_const(&n, &shape, &init);
                self.consts.insert(n.text, ConstState::Done);
                r?;
                Ok(self.sym(name).cloned())
            }
        }
    }

    fn declare_const(&mut self, name: &Name<'a>, shape: &Shape<'a>, init: &Init<'a>) -> R<()> {
        // a table: brackets, a record type, or text
        let is_text = matches!(init, Init::Expr(e) if matches!(e.kind, ExprKind::Str(_)));
        if !shape.dims.is_empty() || matches!(shape.ty, TypeName::Record(_)) || is_text || matches!(init, Init::List(..)) {
            let sym = self.const_table(name, shape, init)?;
            return self.claim(name, Sym::Arr(sym));
        }
        let Init::Expr(e) = init else { unreachable!() };
        let (v, n) = self
            .const_eval(e)?
            .ok_or_else(|| Diag::error(e.pos, "a constant needs a value known at compile time"))?;
        let want = if matches!(shape.ty, TypeName::Fix) {
            Num::Fix
        } else if shape.explicit {
            Num::Int
        } else {
            n
        };
        let v = self.convert_const(v, n, want, e.pos)?;
        self.claim(name, Sym::Const(v, want))
    }

    /// Converts a constant to the declared type: int → fix promotes; fix → int is an error.
    pub fn convert_const(&mut self, v: i32, from: Num, to: Num, pos: Pos) -> R<i32> {
        match (from, to) {
            (Num::Fix, Num::Int) => Err(Diag::error(pos, "a fix value can't be stored as int; use int(x) or round(x)")),
            (Num::Int, Num::Fix) => {
                if !(-32768..=32767).contains(&v) {
                    self.warn(pos, format!("{} is outside the fix range and wraps", v));
                }
                Ok(v.wrapping_shl(16))
            }
            _ => Ok(v),
        }
    }

    pub fn const_int(&mut self, e: &Expr<'a>) -> R<i32> {
        match self.const_eval(e)? {
            Some((v, Num::Int)) => Ok(v),
            Some((_, Num::Fix)) => Err(Diag::error(e.pos, "expected an int constant; use int(x)")),
            None => Err(Diag::error(e.pos, "expected a value known at compile time")),
        }
    }

    // ---------- tables and arrays ----------

    fn alloc_table(&mut self, values: Vec<i32>, cols: u16, pos: Pos) -> R<u16> {
        if self.tables.len() >= limits::ROM_TABLES {
            return Err(Diag::error(
                pos,
                format!("too many constant tables (at most {})", limits::ROM_TABLES),
            ));
        }
        if values.len() > limits::TABLE_ELEMS {
            return Err(Diag::error(pos, format!("a table holds at most {} elements", limits::TABLE_ELEMS)));
        }
        self.tables.push(write::Table { values, cols });
        Ok(self.tables.len() as u16 - 1)
    }

    /// A read-only text table for a string literal (null-terminated, shared).
    pub fn text_table(&mut self, bytes: &[u8], pos: Pos) -> R<u16> {
        // compared with the tables themselves: an index would keep a copy
        let same = |t: &write::Table| {
            t.values.len() == bytes.len() + 1 && t.values.iter().zip(bytes.iter().chain([&0])).all(|(&v, &b)| v == b as i32)
        };
        if let Some(&id) = self.text_tables.iter().find(|&&id| same(&self.tables[id as usize])) {
            return Ok(id);
        }
        let mut v: Vec<i32> = bytes.iter().map(|&b| b as i32).collect();
        v.push(0);
        let id = self.alloc_table(v, 0, pos)?;
        self.text_tables.push(id);
        Ok(id)
    }

    pub fn template(&mut self, bytes: Vec<u8>, pos: Pos) -> R<u16> {
        // a linear search rather than an index: templates are few, and an
        // index would keep a second copy of each
        if let Some(id) = self.templates.iter().position(|t| *t == bytes) {
            return Ok(id as u16);
        }
        if self.templates.len() >= limits::TEMPLATES {
            return Err(Diag::error(
                pos,
                format!("too many interpolated strings (at most {})", limits::TEMPLATES),
            ));
        }
        let id = self.templates.len() as u16;
        let mut bytes = bytes;
        bytes.shrink_to_fit();
        self.templates.push(bytes);
        Ok(id)
    }

    fn alloc_array(&mut self, len: u32, cols: u32, init: u16, name: String, pos: Pos) -> R<u16> {
        if self.arrays.len() >= limits::ARRAYS {
            return Err(Diag::error(
                pos,
                format!("too many arrays (at most {}; each record field counts as one)", limits::ARRAYS),
            ));
        }
        if self.pool_used + len > limits::POOL as u32 {
            return Err(Diag::error(
                pos,
                format!("arrays need {} ints, more than the {}-int pool", self.pool_used + len, limits::POOL),
            ));
        }
        self.arrays.push(ArrayInfo {
            offset: self.pool_used as u16,
            len: len as u16,
            cols: cols as u16,
            init,
        });
        self.pool_used += len;
        self.array_names.push(name);
        Ok(self.arrays.len() as u16 - 1)
    }

    /// Element type and dimensions declared by a shape.
    fn elem_of(&mut self, shape: &Shape<'a>) -> R<Elem> {
        Ok(match &shape.ty {
            TypeName::Int => Elem::Num(Num::Int),
            TypeName::Fix => Elem::Num(Num::Fix),
            TypeName::Record(n) => match self.sym(n.text) {
                Some(Sym::Record(r)) => Elem::Rec(*r),
                _ => return Err(self.unknown("record type", n, &[])),
            },
        })
    }

    /// Fills `out` (row-major, `dims`) from an initializer of numbers.
    fn num_values(&mut self, init: &Init<'a>, num: Num, dims: &[u32], text_rows: bool, out: &mut Vec<i32>, at: usize) -> R<usize> {
        match (init, dims.len()) {
            (Init::Expr(e), _) if matches!(e.kind, ExprKind::Str(_)) => {
                let s = e
                    .plain_str()
                    .ok_or_else(|| Diag::error(e.pos, "interpolated strings can't initialize data"))?;
                let n = dims.iter().product::<u32>() as usize;
                if dims.len() == 2 {
                    return Err(Diag::error(e.pos, "a 2D initializer needs one row per string, inside { }"));
                }
                if text_rows && dims.len() == 1 && s.len() != n {
                    return Err(Diag::error(
                        e.pos,
                        format!("each row holds {} characters; this string has {}", n, s.len()),
                    ));
                }
                if s.len() > n {
                    return Err(Diag::error(e.pos, format!("{} characters don't fit in {}", s.len(), n)));
                }
                for (k, &c) in s.iter().enumerate() {
                    let v = if num == Num::Fix { (c as i32) << 16 } else { c as i32 };
                    out[at + k] = v;
                }
                Ok(s.len())
            }
            (Init::Expr(e), 0) => {
                let (v, n) = self
                    .const_eval(e)?
                    .ok_or_else(|| Diag::error(e.pos, "initial values must be known at compile time"))?;
                out[at] = self.convert_const(v, n, num, e.pos)?;
                Ok(1)
            }
            (Init::Expr(e), _) => Err(Diag::error(e.pos, "expected { values } for an array")),
            (Init::List(_, p), 0) => Err(Diag::error(*p, "expected a single value, not a list")),
            (Init::List(items, p), _) => {
                let inner: u32 = dims[1..].iter().product();
                if items.len() > dims[0] as usize {
                    return Err(Diag::error(*p, format!("{} values for {} elements", items.len(), dims[0])));
                }
                for (k, it) in items.iter().enumerate() {
                    if let Some(f) = &it.field {
                        return Err(Diag::error(f.pos, "named values are for records"));
                    }
                    self.num_values(
                        &it.value,
                        num,
                        &dims[1..],
                        text_rows || dims.len() == 2,
                        out,
                        at + k * inner as usize,
                    )?;
                }
                Ok(items.len() * inner as usize)
            }
        }
    }

    /// Fills per-leaf value vectors for record instance `inst` from `{ ... }`.
    fn rec_values(&mut self, rec: RecId, init: &Init<'a>, leaves: &mut [Vec<i32>], inst: usize) -> R<()> {
        let Init::List(items, p) = init else {
            return Err(Diag::error(
                init.pos(),
                format!("expected {{ fields }} for a {}", self.records[rec].name),
            ));
        };
        let fields = self.records[rec].fields.clone();
        let named = items.iter().any(|i| i.field.is_some());
        if named && items.iter().any(|i| i.field.is_none()) {
            return Err(Diag::error(*p, "name every field or none"));
        }
        if items.len() > fields.len() {
            return Err(Diag::error(*p, format!("{} has {} fields", self.records[rec].name, fields.len())));
        }
        for (k, it) in items.iter().enumerate() {
            let f = match &it.field {
                Some(n) => fields
                    .iter()
                    .find(|f| f.name == n.text)
                    .ok_or_else(|| Diag::error(n.pos, format!("{} has no field '{}'", self.records[rec].name, n.text)))?,
                None => &fields[k],
            };
            let lo = f.leaf_off as usize;
            match f.ty {
                FieldTy::Num(num) => {
                    let mult = self.records[rec].leaves[lo].mult as usize;
                    let mut buf = vec![0; mult];
                    self.num_values(&it.value, num, &f.dims, false, &mut buf, 0)?;
                    leaves[lo][inst * mult..inst * mult + mult].copy_from_slice(&buf);
                }
                FieldTy::Rec(r) => {
                    let n = f.leaf_n as usize;
                    let sub: Vec<Vec<i32>> = Vec::new();
                    let _ = sub;
                    // sub-record values land in this record's leaves at the sub-instance index
                    let elems: Vec<&Init<'a>> = if f.dims.is_empty() {
                        vec![&it.value]
                    } else {
                        match &it.value {
                            Init::List(xs, p2) => {
                                if xs.len() > f.count as usize {
                                    return Err(Diag::error(*p2, format!("{} values for {} elements", xs.len(), f.count)));
                                }
                                xs.iter().map(|x| &x.value).collect()
                            }
                            other => return Err(Diag::error(other.pos(), "expected { values } for an array field")),
                        }
                    };
                    for (j, e) in elems.into_iter().enumerate() {
                        let sub_inst = inst * f.count as usize + j;
                        self.rec_values(r, e, &mut leaves[lo..lo + n], sub_inst)?;
                    }
                }
            }
        }
        Ok(())
    }

    fn const_table(&mut self, name: &Name<'a>, shape: &Shape<'a>, init: &Init<'a>) -> R<ArrSym> {
        let elem = self.elem_of(shape)?;
        let is_text = matches!(init, Init::Expr(e) if matches!(e.kind, ExprKind::Str(_)));
        let mut dims = self.dims_of(&shape.dims, name.pos, true)?;
        if dims.is_empty() {
            dims.push(0);
        }
        if dims.len() > 2 {
            return Err(Diag::error(name.pos, "tables have at most two dimensions"));
        }
        // open first dimension: count the values
        if dims[0] == 0 {
            dims[0] = match init {
                Init::List(items, _) => items.len() as u32,
                Init::Expr(e) => e.plain_str().map(|s| s.len() as u32 + is_text as u32).unwrap_or(1),
            };
            if dims[0] == 0 {
                return Err(Diag::error(name.pos, "an empty table needs an explicit size"));
            }
        }
        if dims.len() == 2 && dims[1] == 0 {
            return Err(Diag::error(name.pos, "only the first dimension may be left open"));
        }
        let count: u32 = dims.iter().product();
        if count as usize > limits::TABLE_ELEMS {
            return Err(Diag::error(
                name.pos,
                format!("a table holds at most {} elements", limits::TABLE_ELEMS),
            ));
        }
        match elem {
            Elem::Num(num) => {
                let mut v = vec![0; count as usize];
                self.num_values(init, num, &dims, false, &mut v, 0)?;
                let cols = if dims.len() == 2 { dims[1] as u16 } else { 0 };
                let id = self.alloc_table(v, cols, name.pos)?;
                Ok(ArrSym {
                    id,
                    rom: true,
                    elem,
                    dims: dims.into(),
                })
            }
            Elem::Rec(r) => {
                if dims.len() != 1 {
                    return Err(Diag::error(name.pos, "record tables have one dimension"));
                }
                let n = dims[0] as usize;
                let leaves = self.records[r].leaves.clone();
                let mut vals: Vec<Vec<i32>> = leaves.iter().map(|l| vec![0; n * l.mult as usize]).collect();
                match init {
                    Init::List(items, p) => {
                        if items.len() > n {
                            return Err(Diag::error(*p, format!("{} values for {} elements", items.len(), n)));
                        }
                        for (k, it) in items.iter().enumerate() {
                            self.rec_values(r, &it.value, &mut vals, k)?;
                        }
                    }
                    Init::Expr(e) => return Err(Diag::error(e.pos, "expected { records }")),
                }
                let mut first = None;
                for (l, v) in leaves.iter().zip(vals) {
                    let id = self.alloc_table(v, if l.mult > 1 { l.mult as u16 } else { 0 }, name.pos)?;
                    first.get_or_insert(id);
                }
                Ok(ArrSym {
                    id: first.unwrap(),
                    rom: true,
                    elem,
                    dims: dims.into(),
                })
            }
        }
    }

    // ---------- globals, functions, sprites, audio vars ----------

    fn declare_values(&mut self) {
        let prog = self.prog;
        // constants (lazily, so order doesn't matter)
        for it in prog.items.iter().filter(|it| it.kind == ItemKind::Const) {
            if let Err(d) = self.const_sym(it.name) {
                self.error(d);
            }
        }
        for (i, it) in prog.items.iter().enumerate() {
            if matches!(it.kind, ItemKind::Const | ItemKind::Record | ItemKind::Enum | ItemKind::Handler) {
                continue;
            }
            let Ok(item) = parser::item_at(self.source, it) else {
                continue;
            };
            let r = match &item {
                Item::Var { name, shape, init } => self.declare_var(name, shape, init.as_ref()),
                Item::AudioVar { name, fix, init } => self.declare_audio_var(name, *fix, init),
                Item::Sprite { name, tile, size } => self.declare_sprite(name, tile, size.as_deref()),
                Item::Fn { name, params, ret_fix, .. } => self.declare_fn(i, name, params, *ret_fix),
                _ => Ok(()),
            };
            if let Err(d) = r {
                self.error(d);
            }
        }
    }

    fn declare_var(&mut self, name: &Name<'a>, shape: &Shape<'a>, init: Option<&Init<'a>>) -> R<()> {
        let elem = self.elem_of(shape)?;
        let text = init.and_then(|i| match i {
            Init::Expr(e) => e.plain_str(),
            _ => None,
        });
        let is_array = !shape.dims.is_empty() || text.is_some() || matches!(init, Some(Init::List(..))) || matches!(elem, Elem::Rec(_));
        if !is_array {
            let Elem::Num(num) = elem else { unreachable!() };
            let v = match init {
                Some(Init::Expr(e)) => match self.const_eval(e)? {
                    Some((v, n)) => {
                        if n == Num::Int && num == Num::Fix {
                            self.check_int_div(e, num);
                        }
                        self.convert_const(v, n, num, e.pos)?
                    }
                    None => {
                        return Err(Diag::error(
                            e.pos,
                            "a global's initial value must be known at compile time; assign it in init()",
                        ))
                    }
                },
                _ => 0,
            };
            if self.globals.len() >= limits::GLOBALS {
                return Err(Diag::error(
                    name.pos,
                    format!("too many global variables (at most {})", limits::GLOBALS),
                ));
            }
            self.globals.push(v);
            return self.claim(name, Sym::Scalar(self.globals.len() as u8 - 1, num));
        }
        let mut dims = self.dims_of(&shape.dims, name.pos, true)?;
        if let Some(t) = &text {
            if dims.is_empty() {
                dims.push(t.len() as u32 + 1);
            }
        }
        if !dims.is_empty() && dims[0] == 0 {
            dims[0] = match init {
                Some(Init::List(items, _)) => items.len() as u32,
                Some(Init::Expr(_)) => text.as_ref().map(|t| t.len() as u32 + 1).unwrap_or(1),
                None => {
                    return Err(Diag::error(
                        name.pos,
                        "'[]' takes its size from the initial values; give a size or values",
                    ))
                }
            };
        }
        if dims.len() > 2 || (dims.len() == 2 && dims[1] == 0) {
            return Err(Diag::error(name.pos, "arrays have one or two dimensions, with sizes"));
        }
        match elem {
            Elem::Num(num) => {
                let count: u32 = dims.iter().product();
                let cols = if dims.len() == 2 { dims[1] } else { 0 };
                let init_id = match init {
                    Some(i) => {
                        let mut v = vec![0; count as usize];
                        let n = self.num_values(i, num, &dims, false, &mut v, 0)?;
                        let _ = n;
                        // trailing zeros needn't be stored
                        while v.last() == Some(&0) {
                            v.pop();
                        }
                        if v.is_empty() {
                            NO_INIT
                        } else {
                            self.alloc_table(v, 0, name.pos)?
                        }
                    }
                    None => NO_INIT,
                };
                let id = self.alloc_array(count, cols, init_id, String::from(name.text), name.pos)?;
                self.claim(
                    name,
                    Sym::Arr(ArrSym {
                        id,
                        rom: false,
                        elem,
                        dims: dims.into(),
                    }),
                )
            }
            Elem::Rec(r) => {
                if dims.len() > 1 {
                    return Err(Diag::error(name.pos, "arrays of records have one dimension"));
                }
                if let Some(i) = init {
                    return Err(Diag::error(i.pos(), "record variables start at zero; set their fields in init()"));
                }
                let n = dims.first().copied().unwrap_or(1);
                let leaves = self.records[r].leaves.clone();
                let mut first = None;
                for l in &leaves {
                    let cols = if l.mult > 1 { l.mult } else { 0 };
                    let id = self.alloc_array(n * l.mult, cols, NO_INIT, format!("{}.{}", name.text, l.path), name.pos)?;
                    first.get_or_insert(id);
                }
                self.claim(
                    name,
                    Sym::Arr(ArrSym {
                        id: first.unwrap(),
                        rom: false,
                        elem,
                        dims: dims.into(),
                    }),
                )
            }
        }
    }

    fn declare_audio_var(&mut self, name: &Name<'a>, fix: bool, init: &Expr<'a>) -> R<()> {
        let num = if fix { Num::Fix } else { Num::Int };
        let (v, n) = self
            .const_eval(init)?
            .ok_or_else(|| Diag::error(init.pos, "an audio variable starts with a constant"))?;
        let v = self.convert_const(v, n, num, init.pos)?;
        if self.audio_vars.len() >= limits::AUDIO_VARS {
            return Err(Diag::error(
                name.pos,
                format!("too many audio variables (at most {})", limits::AUDIO_VARS),
            ));
        }
        self.audio_vars.push(v);
        self.claim(name, Sym::Audio(self.audio_vars.len() as u8 - 1, num))
    }

    fn declare_sprite(&mut self, name: &Name<'a>, tile: &Expr<'a>, size: Option<&(Expr<'a>, Expr<'a>)>) -> R<()> {
        let t = self.const_int(tile)?;
        let (w, h) = match size {
            Some((w, h)) => (self.const_int(w)?, self.const_int(h)?),
            None => (1, 1),
        };
        if !(0..asset::TILE_COUNT as i32).contains(&t) {
            return Err(Diag::error(tile.pos, format!("tile must be 0 to {}", asset::TILE_COUNT - 1)));
        }
        let cols = asset::SHEET_COLS as i32;
        if w < 1 || h < 1 || t % cols + w > cols || t / cols + h > cols {
            return Err(Diag::error(name.pos, "this sprite extends past the edge of the sheet"));
        }
        self.claim(name, Sym::Const(sprite_value(t, w, h), Num::Int))
    }

    fn declare_fn(&mut self, item: usize, name: &Name<'a>, params: &[Param<'a>], ret_fix: bool) -> R<()> {
        if self.funcs.len() >= limits::FUNCTIONS - 4 {
            return Err(Diag::error(name.pos, "too many functions"));
        }
        let mut ps = Vec::with_capacity(params.len());
        for p in params {
            let k = self.param_kind(p)?;
            if ps.iter().any(|q: &(&str, ParamKind)| q.0 == p.name.text) {
                return Err(Diag::error(p.name.pos, format!("parameter '{}' appears twice", p.name.text)));
            }
            ps.push((p.name.text, k));
        }
        let sig = FnSig {
            name: name.text,
            item,
            writes: vec![false; ps.len()],
            params: ps,
            ret: if ret_fix { Num::Fix } else { Num::Int },
            index: 0,
        };
        if sig.slots() as usize > limits::LOCALS {
            return Err(Diag::error(name.pos, "too many parameters"));
        }
        let id = self.funcs.len();
        self.funcs.push(sig);
        self.claim(name, Sym::Func(id))
    }

    pub fn param_kind(&mut self, p: &Param<'a>) -> R<ParamKind> {
        let elem = self.elem_of(&p.shape)?;
        if p.shape.dims.len() > 1 {
            return Err(Diag::error(p.name.pos, "array parameters have one dimension"));
        }
        if let Some(Some(e)) = p.shape.dims.first() {
            return Err(Diag::error(e.pos, "array parameters take any size: write name[]"));
        }
        Ok(match (elem, p.shape.dims.len()) {
            (Elem::Num(n), 0) => ParamKind::Num(n),
            (Elem::Num(n), _) => ParamKind::Arr(n),
            (Elem::Rec(r), 0) => ParamKind::Rec(r),
            (Elem::Rec(r), _) => ParamKind::RecArr(r),
        })
    }

    pub fn handler_slot(k: HandlerKind) -> Handler {
        match k {
            HandlerKind::Init => Handler::Init,
            HandlerKind::Update => Handler::Update,
            HandlerKind::Draw => Handler::Draw,
            HandlerKind::Audio => Handler::Audio,
        }
    }
}
