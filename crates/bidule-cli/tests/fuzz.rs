//! Hostile carts (threat model › Vérification continue): whatever the bytes,
//! loading either rejects the cart or the VM runs it without panicking and
//! without leaving the cart's memory. Deterministic, so failures reproduce.

use bidule_bytecode::image::Handler;
use bidule_bytecode::image::{cart_crc, section, write};
use bidule_bytecode::limits;
use bidule_bytecode::Cart;
use bidule_compiler::{compile, Assets, Options};
use bidule_runtime::Console;
use bidule_vm::{Exit, Vm};

struct Rng(u32);
impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() as usize) % n.max(1)
    }
}

/// Loads and runs a few frames with small budgets. Returns whether it loaded.
fn exercise(bytes: &[u8], debug: bool) -> bool {
    let mut scratch = vec![0u8; limits::FN_CODE_BYTES];
    let Ok(mut vm) = Vm::load(bytes, &mut scratch, debug, 7) else {
        return false;
    };
    let cart = Cart::parse(bytes).unwrap();
    let mut con = Console::new(&cart);
    for (h, args) in [
        (Handler::Init, &[][..]),
        (Handler::Update, &[0, 16][..]),
        (Handler::Draw, &[0, 0][..]),
        (Handler::Update, &[1, 0][..]),
    ] {
        if vm.start(h, args) {
            for _ in 0..4 {
                match vm.run(&mut con, 20_000) {
                    Exit::Budget => continue,
                    Exit::Suspend { .. } => vm.resume(-1),
                    _ => break,
                }
            }
        }
    }
    for t in 0..64 {
        vm.audio_sample(t, &mut con);
    }
    true
}

fn samples() -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for src in [
        include_str!("../../../carts/blink.bdcart"),
        include_str!("../../../carts/tile-peek.bdcart"),
        include_str!("../../../carts/sprite-editor.bdcart"),
    ] {
        out.push(compile(src, &Assets::default(), &Options::default()).unwrap().cart);
    }
    out
}

/// Byte flips anywhere, with the checksum fixed so they reach the parsers and verifier.
#[test]
fn mutated_carts() {
    let mut rng = Rng(0xB1D0_0001);
    let (mut loaded, mut rejected) = (0, 0);
    for base in samples() {
        assert!(exercise(&base, true));
        for _ in 0..1500 {
            let mut b = base.clone();
            for _ in 0..1 + rng.below(4) {
                let i = 24 + rng.below(b.len() - 24);
                b[i] = match rng.below(3) {
                    0 => b[i] ^ (1 << rng.below(8)),
                    1 => rng.next() as u8,
                    _ => [0, 1, 0x7F, 0x80, 0xFF][rng.below(5)],
                };
            }
            let crc = cart_crc(&b);
            b[16..20].copy_from_slice(&crc.to_le_bytes());
            if exercise(&b, rng.below(2) == 0) {
                loaded += 1
            } else {
                rejected += 1
            }
        }
    }
    eprintln!("mutated carts: {} loaded and ran, {} rejected", loaded, rejected);
    assert!(loaded > 0 && rejected > 0);
}

/// Random code in an otherwise valid program: one function, used as every handler.
#[test]
fn random_bytecode() {
    let mut rng = Rng(0xC0DE_0002);
    let mut loaded = 0;
    for round in 0..20_000 {
        // well-formed instructions with small operands; jumps go back to earlier
        // instructions, and most programs end with a return
        use bidule_bytecode::op::{Op, Operands};
        let mut code: Vec<u8> = Vec::new();
        let mut starts: Vec<u32> = Vec::new();
        for _ in 0..1 + rng.below(24) {
            let op = Op::ALL[rng.below(Op::ALL.len())];
            starts.push(code.len() as u32);
            code.push(op as u8);
            match op.operands() {
                Operands::None => {}
                Operands::U8 | Operands::I8 => code.push(rng.below(4) as u8),
                Operands::U16 => code.extend_from_slice(&(rng.below(3) as u16).to_le_bytes()),
                Operands::I32 => code.extend_from_slice(&[(rng.next() as i32 % 300), 0, -1, 4, 1 << 16][rng.below(5)].to_le_bytes()),
                Operands::Target => code.extend_from_slice(&starts[rng.below(starts.len())].to_le_bytes()),
                Operands::Builtin => {
                    code.push(rng.below(54) as u8);
                    code.push(rng.below(4) as u8);
                }
            }
        }
        if round % 4 != 0 {
            code.extend_from_slice(&[Op::PUSH_I8 as u8, 0, Op::RET as u8]);
        }
        let arrays = [bidule_bytecode::image::ArrayInfo {
            offset: 0,
            len: 8,
            cols: 4,
            init: bidule_bytecode::image::NO_INIT,
        }];
        let audio = round % 5 == 0;
        let handlers = if audio {
            [0xFFFF, 0xFFFF, 0xFFFF, 0]
        } else {
            [0, 0xFFFF, 0, 0xFFFF]
        };
        let build = |max: u8| {
            let fns = [write::FnEntry {
                start: 0,
                params: if audio { 1 } else { 0 },
                locals: 2,
                max_stack: max,
            }];
            write::program(&write::ProgramTables {
                globals: &[1, 2],
                audio_vars: &[0],
                arrays: &arrays,
                functions: &fns,
                handlers,
            })
        };
        let rom = write::rom(&[write::Table {
            values: vec![1, -2, 300, 4],
            cols: 2,
        }]);
        // declare the depth the verifier computes, as the compiler does
        let max = {
            let p0 = build(limits::MAX_STACK as u8);
            let r = bidule_bytecode::image::Rom::from_bytes(&rom).unwrap();
            match bidule_bytecode::Program::parse(&p0, &code, r, bidule_bytecode::image::Templates::EMPTY, None) {
                Ok(p) => bidule_bytecode::verify::max_stack(&p, 0, &mut vec![0u8; limits::FN_CODE_BYTES]).unwrap_or(0),
                Err(_) => 0,
            }
        };
        let mut secs = vec![(section::PROGRAM, build(max)), (section::CODE, code), (section::ROM, rom)];
        let bytes = write::cart(1, &mut secs);
        if exercise(&bytes, round % 2 == 0) {
            loaded += 1;
        }
    }
    eprintln!("random bytecode: {} of 20000 programs verified and ran", loaded);
    assert!(loaded > 0);
}
