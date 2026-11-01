//! Desktop toolchain around the compiler, VM and runtime.

use std::fmt::Write as _;
use std::path::Path;

use bidule_bytecode::asset::AssetSection;
use bidule_bytecode::builtins;
use bidule_bytecode::image::{meta_value, section, Cart, Handler, Program};
use bidule_bytecode::limits;
use bidule_bytecode::op::{Op, Operands};
use bidule_bytecode::verify::decode;
use bidule_compiler::{compile, Assets, Options};
use bidule_runtime::{Console, Dialog, H, W};
use bidule_vm::{Exit, Fault, Vm};

/// Instructions per handler call before the frame is late (Proposed in the spec).
pub const FRAME_BUDGET: u32 = 2_000_000;

/// Asset section files of an unpacked cart folder.
pub const ASSET_FILES: [(&str, u16); 5] = [
    ("sprites.bin", section::SPRITES),
    ("maps.bin", section::MAPS),
    ("sounds.bin", section::SOUNDS),
    ("music.bin", section::MUSIC),
    ("palette.bin", section::PALETTE),
];

/// Raw asset sections read from a folder; missing files are fine.
#[derive(Default)]
pub struct AssetFiles {
    pub data: Vec<(u16, Vec<u8>)>,
}

impl AssetFiles {
    pub fn read(dir: &Path) -> std::io::Result<Self> {
        let mut data = Vec::new();
        for (f, k) in ASSET_FILES {
            let p = dir.join(f);
            if p.exists() {
                data.push((k, std::fs::read(p)?));
            }
        }
        Ok(AssetFiles { data })
    }
    pub fn assets(&self) -> Assets<'_> {
        let get = |k| self.data.iter().find(|(x, _)| *x == k).map(|(_, b)| b.as_slice());
        Assets {
            sprites: get(section::SPRITES),
            maps: get(section::MAPS),
            sounds: get(section::SOUNDS),
            music: get(section::MUSIC),
            palette: get(section::PALETTE),
        }
    }
}

/// Compiles source text; on failure returns the diagnostics as text.
pub fn build(source: &str, assets: &AssetFiles, debug_info: bool, file: &str) -> Result<(Vec<u8>, String), String> {
    let render = |ds: &[bidule_compiler::Diag]| ds.iter().map(|d| format!("{}:{}\n", file, d)).collect::<String>();
    match compile(source, &assets.assets(), &Options { debug_info }) {
        Ok(o) => Ok((o.cart, render(&o.warnings))),
        Err(ds) => Err(render(&ds)),
    }
}

/// How a headless run ended.
#[derive(Debug)]
pub enum RunEnd {
    Frames,
    Fault { stage: &'static str, message: String },
}

/// Runs a cart without a display: init, then `frames` × (update, draw).
/// `on_frame` sees the console after each draw. Dialogs are cancelled.
pub fn run_headless(
    bytes: &[u8],
    debug: bool,
    frames: u32,
    inputs: &[u32],
    mut on_frame: impl FnMut(u32, &Console),
) -> Result<RunEnd, String> {
    let mut scratch = vec![0u8; limits::FN_CODE_BYTES];
    let mut vm = Vm::load(bytes, &mut scratch, debug, 0x1234_5678).map_err(|e| e.to_string())?;
    let cart = Cart::parse(bytes).map_err(|e| e.to_string())?;
    let mut con = Console::new(&cart);
    let call = |vm: &mut Vm, con: &mut Console, h: Handler, args: &[i32], stage: &'static str| -> Option<RunEnd> {
        if !vm.start(h, args) {
            return None;
        }
        loop {
            match vm.run(con, FRAME_BUDGET) {
                Exit::Done(_) => return None,
                Exit::Budget => continue,
                Exit::Suspend { .. } => {
                    let answer = match con.dialog.take() {
                        Some(Dialog::PickCart) => -1,
                        _ => 0,
                    };
                    vm.resume(answer);
                }
                Exit::Fault(f) => {
                    return Some(RunEnd::Fault {
                        stage,
                        message: describe_fault(vm, &f),
                    })
                }
            }
        }
    };
    if let Some(end) = call(&mut vm, &mut con, Handler::Init, &[], "init") {
        return Ok(end);
    }
    for f in 0..frames {
        let input = inputs.get(f as usize).copied().unwrap_or(0);
        con.begin_frame(input);
        con.now_ms = f * 1000 / 60;
        vm.snapshot_globals();
        if let Some(end) = call(&mut vm, &mut con, Handler::Update, &[f as i32, input as i32], "update") {
            return Ok(end);
        }
        if let Some(end) = call(&mut vm, &mut con, Handler::Draw, &[f as i32, input as i32], "draw") {
            return Ok(end);
        }
        on_frame(f, &con);
    }
    Ok(RunEnd::Frames)
}

/// "line 12:5: index out of bounds (enemies.hp[16], size 16)" and a call trace.
pub fn describe_fault(vm: &Vm, f: &Fault) -> String {
    let p = vm.program();
    let mut s = String::new();
    match vm.fault_line(f) {
        Some((l, c)) => write!(s, "line {}:{}: ", l, c).unwrap(),
        None => write!(s, "offset {}: ", f.pc).unwrap(),
    }
    s.push_str(f.kind.message());
    if let bidule_vm::FaultKind::OutOfBounds { array, index, size } = f.kind {
        let name = if array < 0 {
            String::from_utf8_lossy(p.array_name((array & 0xFF) as u16)).into_owned()
        } else {
            match bidule_bytecode::handle::Handle::decode(array) {
                Some(h) if h.kind == bidule_bytecode::handle::Kind::Pool => String::from_utf8_lossy(p.array_name(h.id)).into_owned(),
                Some(_) => String::from("constant table"),
                None => String::new(),
            }
        };
        if name.is_empty() {
            write!(s, " (index {}, size {})", index, size).unwrap();
        } else {
            write!(s, " ({}[{}], size {})", name, index, size).unwrap();
        }
    }
    let mut tr = [(0u16, 0u32); limits::CALL_DEPTH];
    let n = vm.trace(&mut tr);
    for &(func, pc) in &tr[..n] {
        let name = String::from_utf8_lossy(p.function_name(func)).into_owned();
        let line = p
            .debug
            .and_then(|d| d.line_at(pc))
            .map(|(l, _)| format!(" line {}", l))
            .unwrap_or_default();
        write!(
            s,
            "\n  in {}{}",
            if name.is_empty() { format!("function {}", func) } else { name },
            line
        )
        .unwrap();
    }
    s
}

/// The framebuffer as a binary PPM image, scaled up.
pub fn ppm(con: &Console, scale: usize) -> Vec<u8> {
    let (w, h) = (W as usize * scale, H as usize * scale);
    let mut out = format!("P6\n{} {}\n255\n", w, h).into_bytes();
    for y in 0..h {
        for x in 0..w {
            let i = con.screen.fb[(y / scale) * W as usize + x / scale];
            out.extend_from_slice(&bidule_runtime::gfx::rgb888(con.screen.pal[i as usize]));
        }
    }
    out
}

/// Human-readable listing of a cart's code and tables.
pub fn disassemble(bytes: &[u8]) -> Result<String, String> {
    let cart = Cart::parse(bytes).map_err(|e| e.to_string())?;
    let p = Program::from_cart(&cart).map_err(|e| e.to_string())?;
    let mut s = String::new();
    if let Some(m) = cart.section(section::META) {
        for k in ["title", "author", "id", "version"] {
            if let Some(v) = meta_value(m, k) {
                writeln!(s, "; @{} {}", k, String::from_utf8_lossy(v)).unwrap();
            }
        }
    }
    writeln!(
        s,
        "; compiler {}.{}, {} bytes",
        cart.compiler_version >> 8,
        cart.compiler_version & 255,
        bytes.len()
    )
    .unwrap();
    writeln!(
        s,
        "; {} globals, {} arrays, {} functions, {} tables, {} templates, {} audio vars",
        p.globals,
        p.arrays,
        p.functions,
        p.rom.count(),
        p.text.count(),
        p.audio_vars
    )
    .unwrap();
    for i in 0..p.arrays {
        let a = p.array(i as u8);
        let shape = if a.cols == 0 {
            format!("[{}]", a.len)
        } else {
            format!("[{}][{}]", a.rows(), a.cols)
        };
        writeln!(
            s,
            ";   array {:3} {}{} at {}{}",
            i,
            String::from_utf8_lossy(p.array_name(i)),
            shape,
            a.offset,
            if a.init != bidule_bytecode::image::NO_INIT {
                format!(" init table {}", a.init)
            } else {
                String::new()
            }
        )
        .unwrap();
    }
    for i in 0..p.rom.count() {
        let t = p.rom.table(i).unwrap();
        writeln!(
            s,
            ";   table {:3} {} x {}-byte{}{}",
            i,
            t.count,
            t.width,
            if t.signed { " signed" } else { "" },
            if t.cols > 0 { format!(", {} cols", t.cols) } else { String::new() }
        )
        .unwrap();
    }
    for f in 0..p.functions {
        let info = p.function(f);
        let name = String::from_utf8_lossy(p.function_name(f)).into_owned();
        let tag = p.handler_of(f).map(|h| format!(" [{:?}]", h).to_lowercase()).unwrap_or_default();
        writeln!(
            s,
            "\nfn {} {}{}  ; params {}, locals {}, stack {}",
            f, name, tag, info.params, info.locals, info.max_stack
        )
        .unwrap();
        let mut pc = info.start as usize;
        let mut last_line = 0;
        while pc < info.end as usize {
            let i = decode(p.code, pc).ok_or("undecodable")?;
            if let Some((l, _)) = p.debug.and_then(|d| d.line_at(pc as u32)) {
                if l != last_line {
                    writeln!(s, "            ; line {}", l).unwrap();
                    last_line = l;
                }
            }
            let operand = match (i.op, i.op.operands()) {
                (_, Operands::None) => String::new(),
                (Op::CALL, _) => format!("{} {}", builtins::by_id(i.a as u8).map_or("?", |b| b.name), i.b),
                (Op::CALL_FN, _) => format!("{} {}", i.a, String::from_utf8_lossy(p.function_name(i.a as u16))),
                (Op::ARR_GET | Op::ARR_SET | Op::ARR_GET2 | Op::ARR_SET2 | Op::PUSH_ARR, _) => {
                    format!("{} {}", i.a, String::from_utf8_lossy(p.array_name(i.a as u16)))
                }
                (_, Operands::Builtin) => format!("{} {}", i.a, i.b),
                (_, _) => format!("{}", i.a),
            };
            writeln!(s, "  {:6}  {:12} {}", pc, i.op.name(), operand).unwrap();
            pc += i.size;
        }
    }
    Ok(s)
}

/// Writes a cart's source and asset sections into a folder.
pub fn unpack(bytes: &[u8], dir: &Path) -> Result<Vec<String>, String> {
    let cart = Cart::parse(bytes).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut written = Vec::new();
    let title = cart
        .section(section::META)
        .and_then(|m| meta_value(m, "id"))
        .map(|v| String::from_utf8_lossy(v).into_owned())
        .unwrap_or_else(|| String::from("cart"));
    if let Some(src) = cart.section(section::SOURCE) {
        let f = format!("{}.bdcart", title);
        std::fs::write(dir.join(&f), src).map_err(|e| e.to_string())?;
        written.push(f);
    }
    for (f, k) in ASSET_FILES {
        if let Some(b) = cart.section(k) {
            AssetSection::parse(b).map_err(|e| format!("{}: {}", f, e))?;
            std::fs::write(dir.join(f), b).map_err(|e| e.to_string())?;
            written.push(String::from(f));
        }
    }
    Ok(written)
}
