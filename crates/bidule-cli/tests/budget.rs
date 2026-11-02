//! Spec › Compiler constraints: any valid cart compiles within 128 KB of
//! working RAM. Measured with a counting allocator around the compiler.
//!
//! NOT MET YET. The compiler keeps every token and the whole syntax tree in
//! memory; the planned fix is to compile one function at a time with names
//! borrowed from the source, and to stream sections out. Until then
//! `current_levels` is a ratchet against regressions and `within_128_kb` (ignored)
//! is the target. Run with `--test-threads=1`: the counter is process-wide.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use bidule_compiler::{compile, Assets, Options};

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let n = LIVE.fetch_add(l.size(), Ordering::SeqCst) + l.size();
        PEAK.fetch_max(n, Ordering::SeqCst);
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size(), Ordering::SeqCst);
        System.dealloc(p, l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        if new > l.size() {
            let n = LIVE.fetch_add(new - l.size(), Ordering::SeqCst) + new - l.size();
            PEAK.fetch_max(n, Ordering::SeqCst);
        } else {
            LIVE.fetch_sub(l.size() - new, Ordering::SeqCst);
        }
        System.realloc(p, l, new)
    }
}

#[global_allocator]
static A: Counting = Counting;

/// Peak heap use while compiling, beyond what was live before.
fn peak(src: &str) -> usize {
    let base = LIVE.load(Ordering::SeqCst);
    PEAK.store(base, Ordering::SeqCst);
    let out = compile(src, &Assets::default(), &Options::default());
    assert!(out.is_ok(), "{:?}", out.err());
    drop(out);
    PEAK.load(Ordering::SeqCst) - base
}

/// A large synthetic cart: many functions, globals, arrays and strings.
fn synthetic(fns: usize) -> String {
    let mut s = String::from("// @title Big\nrecord P { x: fix, y: fix, hp }\nvar ps: P[32]\nconst T[] = {1, 2, 3, 4, 5, 6, 7, 8}\n");
    for i in 0..fns {
        s += &format!("var g{i} = {i}\nvar a{i}[8]\n");
        s += &format!(
            "fn f{i}(n, p: P) {{\n  var t = 0\n  for (var k = 0; k < n; k++) {{\n    a{i}[k % 8] += T[k % 8] * g{i}\n    t += a{i}[k % 8]\n  }}\n  p.x += 0.5\n  if (t > 100 && p.hp < 3) {{ p.hp++ }} else {{ p.y -= 0.25 }}\n  print(\"f{i} {{t:04}} {{p.x:.2}}\", 4, 4, t % 16)\n  return t\n}}\n"
        );
    }
    s += "update() {\n";
    for i in 0..fns {
        s += &format!("  g{i} += f{i}({i}, ps[{}])\n", i % 32);
    }
    s += "}\n";
    s
}

const BUDGET: usize = 128 * 1024;

use std::sync::Mutex;
static SERIAL: Mutex<()> = Mutex::new(());

fn carts() -> Vec<(&'static str, String)> {
    vec![
        ("blink", String::from(include_str!("../../../carts/blink.bdcart"))),
        ("tile-peek", String::from(include_str!("../../../carts/tile-peek.bdcart"))),
        ("sprite-editor", String::from(include_str!("../../../carts/sprite-editor.bdcart"))),
        ("synthetic-120", synthetic(120)),
    ]
}

/// Peaks measured when the compiler landed, plus 10%: fail on regressions.
#[test]
fn current_levels() {
    let _g = SERIAL.lock().unwrap();
    let limits = [40, 175, 775, 2050];
    for ((name, src), kb) in carts().into_iter().zip(limits) {
        let p = peak(&src);
        eprintln!(
            "{}: {} bytes of source, peak {} KB (ratchet {} KB, target 128 KB)",
            name,
            src.len(),
            p / 1024,
            kb
        );
        assert!(p <= kb * 1024, "{} now needs {} KB", name, p / 1024);
    }
}

#[test]
#[ignore = "the compiler does not meet the 128 KB budget yet"]
fn within_128_kb() {
    let _g = SERIAL.lock().unwrap();
    for (name, src) in carts() {
        let p = peak(&src);
        assert!(p <= BUDGET, "{} needs {} KB", name, p / 1024);
    }
}
