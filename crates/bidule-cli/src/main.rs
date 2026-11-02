//! `bidule`: build, run, inspect and unpack Bidule 01 carts.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use bidule_cli::*;

const USAGE: &str = "\
bidule — Bidule 01 toolchain

USAGE
  bidule build <cart.bdcart> [-o out.bdb] [--assets DIR] [--no-debug]
  bidule run   <cart.bdb | cart.bdcart> [--frames N] [--input A,B,...] [--release]
               [--assets DIR] [--ppm FILE] [--dump FILE]
  bidule dis   <cart.bdb | cart.bdcart> [--assets DIR]
  bidule check <cart.bdb>
  bidule unpack <cart.bdb> <DIR>

--assets DIR reads sprites.bin, maps.bin, sounds.bin, music.bin and palette.bin
from DIR (the layout `unpack` writes). Without it, a folder named like the
cart (cart/ next to cart.bdcart) is used when it exists.";

struct Args {
    pos: Vec<String>,
    flags: Vec<(String, Option<String>)>,
}

impl Args {
    fn parse(raw: &[String]) -> Args {
        let takes = ["-o", "--assets", "--frames", "--input", "--ppm", "--dump"];
        let mut a = Args {
            pos: Vec::new(),
            flags: Vec::new(),
        };
        let mut i = 0;
        while i < raw.len() {
            let r = &raw[i];
            if takes.contains(&r.as_str()) {
                a.flags.push((r.clone(), raw.get(i + 1).cloned()));
                i += 2;
            } else if r.starts_with('-') {
                a.flags.push((r.clone(), None));
                i += 1;
            } else {
                a.pos.push(r.clone());
                i += 1;
            }
        }
        a
    }
    fn get(&self, k: &str) -> Option<&str> {
        self.flags.iter().find(|f| f.0 == k).and_then(|f| f.1.as_deref())
    }
    fn has(&self, k: &str) -> bool {
        self.flags.iter().any(|f| f.0 == k)
    }
}

fn fail(msg: impl AsRef<str>) -> ExitCode {
    eprintln!("{}", msg.as_ref());
    ExitCode::FAILURE
}

/// Compiles a .bdcart, or reads a .bdb.
fn load(path: &str, args: &Args, debug_info: bool) -> Result<Vec<u8>, String> {
    let p = Path::new(path);
    if p.extension().and_then(|e| e.to_str()) != Some("bdcart") {
        return std::fs::read(p).map_err(|e| format!("{}: {}", path, e));
    }
    let src = std::fs::read_to_string(p).map_err(|e| format!("{}: {}", path, e))?;
    let dir: Option<PathBuf> = match args.get("--assets") {
        Some(d) => Some(PathBuf::from(d)),
        None => {
            let d = p.with_extension("");
            if d.is_dir() {
                Some(d)
            } else {
                None
            }
        }
    };
    let assets = match dir {
        Some(d) => AssetFiles::read(&d).map_err(|e| format!("{}: {}", d.display(), e))?,
        None => AssetFiles::default(),
    };
    let (cart, warnings) = build(&src, &assets, debug_info, path)?;
    eprint!("{}", warnings);
    Ok(cart)
}

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = raw.first() else { return fail(USAGE) };
    let args = Args::parse(&raw[1..]);
    let Some(input) = args.pos.first() else { return fail(USAGE) };
    match cmd.as_str() {
        "build" => {
            let cart = match load(input, &args, !args.has("--no-debug")) {
                Ok(c) => c,
                Err(e) => return fail(e),
            };
            let out = args
                .get("-o")
                .map(PathBuf::from)
                .unwrap_or_else(|| Path::new(input).with_extension("bdb"));
            if let Err(e) = std::fs::write(&out, &cart) {
                return fail(format!("{}: {}", out.display(), e));
            }
            eprintln!("wrote {} ({} bytes)", out.display(), cart.len());
            ExitCode::SUCCESS
        }
        "run" => {
            let cart = match load(input, &args, true) {
                Ok(c) => c,
                Err(e) => return fail(e),
            };
            let frames: u32 = args.get("--frames").and_then(|f| f.parse().ok()).unwrap_or(60);
            let inputs: Vec<u32> = args
                .get("--input")
                .map(|s| s.split(',').filter_map(|x| x.trim().parse().ok()).collect())
                .unwrap_or_default();
            let mut dump = Vec::new();
            let mut last = None;
            let res = run_headless(&cart, !args.has("--release"), frames, &inputs, |_, con| {
                if args.has("--dump") {
                    dump.extend_from_slice(&con.screen.fb);
                }
                last = Some(ppm(con, 2));
            });
            if let Some(f) = args.get("--dump") {
                if let Err(e) = std::fs::write(f, &dump) {
                    return fail(format!("{}: {}", f, e));
                }
            }
            if let (Some(f), Some(img)) = (args.get("--ppm"), &last) {
                if let Err(e) = std::fs::write(f, img) {
                    return fail(format!("{}: {}", f, e));
                }
            }
            match res {
                Ok(RunEnd::Frames) => {
                    eprintln!("ran {} frames", frames);
                    ExitCode::SUCCESS
                }
                Ok(RunEnd::Fault { stage, message }) => fail(format!("{} stopped: {}", stage, message)),
                Err(e) => fail(e),
            }
        }
        "dis" => match load(input, &args, true).and_then(|c| disassemble(&c)) {
            Ok(s) => {
                print!("{}", s);
                ExitCode::SUCCESS
            }
            Err(e) => fail(e),
        },
        "check" => {
            let bytes = match std::fs::read(input) {
                Ok(b) => b,
                Err(e) => return fail(format!("{}: {}", input, e)),
            };
            let mut scratch = vec![0u8; bidule_bytecode::limits::FN_CODE_BYTES];
            match bidule_vm::Vm::load(&bytes, &mut scratch, false, 1) {
                Ok(_) => {
                    eprintln!("{}: ok", input);
                    ExitCode::SUCCESS
                }
                Err(e) => fail(format!("{}: {}", input, e)),
            }
        }
        "unpack" => {
            let Some(dir) = args.pos.get(1) else { return fail(USAGE) };
            let bytes = match std::fs::read(input) {
                Ok(b) => b,
                Err(e) => return fail(format!("{}: {}", input, e)),
            };
            match unpack(&bytes, Path::new(dir)) {
                Ok(files) => {
                    for f in files {
                        eprintln!("{}/{}", dir, f);
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => fail(e),
            }
        }
        _ => fail(USAGE),
    }
}
