//! Language semantics, end to end: source → compiler → verifier → VM.

use bidule_bytecode::builtins::id;
use bidule_bytecode::image::Handler;
use bidule_compiler::{compile, Assets, Options};
use bidule_vm::{Call, Exit, FaultKind, Host, Mem, Vm};

#[derive(Default)]
struct Rec {
    out: Vec<String>,
    input: i32,
}

impl Host for Rec {
    fn call(&mut self, b: u8, args: &[i32], text: &[u8], _mem: &mut Mem) -> Call {
        match b {
            id::PRINT => {
                self.out.push(String::from_utf8_lossy(text).into_owned());
                Call::Value(args[1] + 6 * text.len() as i32)
            }
            id::BTN => Call::Value((self.input >> args[0]) & 1),
            _ => Call::Value(0),
        }
    }
}

fn cart(src: &str) -> Vec<u8> {
    match compile(src, &Assets::default(), &Options::default()) {
        Ok(o) => o.cart,
        Err(ds) => panic!("{}", ds.iter().map(|d| d.to_string()).collect::<Vec<_>>().join("\n")),
    }
}

fn errors(src: &str) -> String {
    match compile(src, &Assets::default(), &Options::default()) {
        Ok(_) => panic!("expected a compile error"),
        Err(ds) => ds
            .iter()
            .filter(|d| d.is_error())
            .map(|d| d.to_string())
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// Runs init() then one update() and draw(); returns printed lines or the fault.
fn run_mode(src: &str, debug: bool) -> Result<Vec<String>, (FaultKind, Option<(u16, u16)>)> {
    let bytes = cart(src);
    let mut scratch = vec![0u8; bidule_bytecode::limits::FN_CODE_BYTES];
    let mut vm = Vm::load(&bytes, &mut scratch, debug, 1).unwrap_or_else(|e| panic!("{}", e));
    let mut host = Rec::default();
    for (h, args) in [
        (Handler::Init, &[][..]),
        (Handler::Update, &[0, 0][..]),
        (Handler::Draw, &[0, 0][..]),
    ] {
        if vm.start(h, args) {
            match vm.run(&mut host, 10_000_000) {
                Exit::Done(_) => {}
                Exit::Fault(f) => return Err((f.kind, vm.fault_line(&f))),
                e => panic!("unexpected {:?}", e),
            }
        }
    }
    Ok(host.out)
}

fn run(src: &str) -> Vec<String> {
    run_mode(src, true).unwrap_or_else(|f| panic!("fault {:?}", f))
}

fn p(src: &str) -> String {
    run(&format!("init() {{\n{}\n}}", src)).join("|")
}

#[test]
fn integers() {
    assert_eq!(
        p(r#"print("{7 / 2} {-7 / 2} {-7 % 2} {1 << 33} {-16 >> 2} {0x7FFFFFFF + 1}", 0, 0)"#),
        "3 -3 -1 2 -4 -2147483648"
    );
    assert_eq!(
        p(r#"print("{3 < 4} {!5} {~0} {2 && 3} {0 || 7} {1 + 2 * 3} {(1 + 2) * 3}", 0, 0)"#),
        "1 0 -1 1 1 7 9"
    );
    assert_eq!(p(r#"print("{1 == 1 & 0}", 0, 0)"#), "0");
}

#[test]
fn fix_math() {
    assert_eq!(
        p(r#"var a: fix = 1.5
var b: fix = a * 2
print("{a} {b} {a / 2} {int(-1.5)} {round(2.5)} {frac(3.25)} {-7.5 % 2}", 0, 0)"#),
        "1.5 3 0.75 -2 3 0.25 -1.5"
    );
    assert_eq!(
        p(r#"print("{sin(90)} {cos(180)} {sqrt(2.0):.3} {atan2(1, 1)} {abs(-2.5)} {min(3, 2.5)}", 0, 0)"#),
        "1 -1 1.414 45 2.5 2.5"
    );
    assert_eq!(
        p(r#"var x: fix = 1
x += 0.25
x++
print("{x:.2} {x:6.1}", 0, 0)"#),
        "2.25    2.3"
    );
}

#[test]
fn short_circuit_and_conditional() {
    let out = run(r#"
var calls = 0
fn side(v) {
  calls++
  return v
}
init() {
  var a = side(0) && side(1)
  var b = side(1) || side(1)
  var c: fix = calls > 1 ? 2 : 0.5
  print("{a} {b} {calls} {c}", 0, 0)
}"#);
    assert_eq!(out, ["0 1 2 2"]);
}

#[test]
fn locals_recursion_and_frames() {
    assert_eq!(
        run(r#"
fn fib(n) {
  if (n < 2) { return n }
  return fib(n - 1) + fib(n - 2)
}
fn sum(a[], n) {
  var total = 0
  for (var i = 0; i < n; i++) {
    total += a[i]
  }
  return total
}
var xs[] = {1, 2, 3, 4}
init() {
  print("{fib(15)} {sum(xs, xs.length)}", 0, 0)
}"#),
        ["610 10"]
    );
}

#[test]
fn loops_break_continue() {
    assert_eq!(
        p(r#"var s = 0
var i = 0
while (1) {
  i++
  if (i > 10) { break }
  if (i % 2 == 0) { continue }
  s += i
}
for (var j = 0; j < 3; j++) { s += 100 }
print("{s}", 0, 0)"#),
        "325"
    );
}

#[test]
fn enums_and_switch() {
    assert_eq!(
        run(r#"
enum State { TITLE, PLAYING = 5, PAUSED }
fn name(s) {
  switch (s) {
    State.TITLE { return 1 }
    State.PLAYING, State.PAUSED { return 2 }
    else { return 3 }
  }
  return 0
}
init() {
  print("{State.PAUSED} {name(State.TITLE)} {name(6)} {name(42)}", 0, 0)
}"#),
        ["6 1 2 3"]
    );
}

#[test]
fn arrays_2d_and_strings() {
    assert_eq!(
        run(r#"
var grid[3][4]
var name = "bob"
var msg[16]
init() {
  grid[2][3] = 7
  grid[1][0] += 2
  fmt(msg, "{name}:{grid[2][3]}:{grid[1][0]}:{grid.length}:{grid[0].length}")
  print(msg, 0, 0)
  print(name, 0, 0)
  print("{name:5}|", 0, 0)
}"#),
        ["bob:7:2:3:4", "bob", "  bob|"]
    );
}

#[test]
fn constant_tables() {
    assert_eq!(
        run(r#"
const STEPS[] = {0, 3, 7, 12, 1000}
const G: fix[] = {0.1, 0.25}
const ROOM[2][4] = {
  "=..=",
  "====",
}
const TITLE = "HELLO"
record Weapon { damage, speed: fix }
const WEAPONS: Weapon[] = {
  { damage: 1, speed: 4.0 },
  { 3, 2.5 },
}
fn show(t[]) { print(t, 0, 0) }
init() {
  print("{STEPS[4]} {G[1]} {ROOM[1][2] == '='} {ROOM[0][1]:c} {STEPS.length} {WEAPONS[1].speed} {WEAPONS[0].damage}", 0, 0)
  show(TITLE)
  show(ROOM[0])
  print(TITLE, 0, 0)
}"#),
        ["1000 0.25 1 . 5 2.5 1", "HELLO", "=..=", "HELLO"]
    );
}

#[test]
fn records() {
    assert_eq!(
        run(r#"
record Vec2 { x: fix, y: fix }
record Enemy {
  pos: Vec2
  vel: Vec2
  path: int[4]
  hp
}
var enemies: Enemy[3]
var player: Enemy

fn hurt(e: Enemy, dmg) {
  e.hp -= dmg
  e.path[1] = 9
}
fn nudge(p: Vec2) { p.x += 0.5 }
fn count(es: Enemy[]) { return es.length }
fn remove(es: Enemy[], i, n) {
  es[i] = es[n - 1]
}
init() {
  enemies[1].hp = 10
  enemies[1].vel.x = 1.5
  enemies[2].hp = 30
  enemies[2].path[3] = 4
  hurt(enemies[1], 3)
  nudge(enemies[1].pos)
  enemies[1].pos.x += enemies[1].vel.x
  player = enemies[1]
  player.hp++
  remove(enemies, 1, 3)
  print("{enemies[1].hp} {enemies[1].path[3]} {player.hp} {player.path[1]} {player.pos.x} {count(enemies)}", 0, 0)
}"#),
        ["30 4 8 9 2 3"]
    );
}

#[test]
fn text_formats() {
    assert_eq!(
        p(r#"print("[{42:5}] [{42:05}] [{-5:04}] [{255:x}] [{65:c}] [{1.25:.1}] {{x}}", 0, 0)"#),
        "[   42] [00042] [-005] [ff] [A] [1.3] {x}"
    );
    assert_eq!(
        p(r#"var w = textwidth("ab\ncdef")
print(w, 0, 0)
var x = print("HP ", 4, 2)
print(x, 0, 0)"#),
        "24|HP |22"
    );
}

#[test]
fn faults_in_debug_and_release() {
    let oob = "var a[4]\ninit() {\n  var i = 4\n  a[i] = 1\n  print(\"{a[i]}\", 0, 0)\n}";
    let (k, line) = run_mode(oob, true).unwrap_err();
    assert!(matches!(k, FaultKind::OutOfBounds { index: 4, size: 4, .. }), "{:?}", k);
    assert_eq!(line.unwrap().0, 4);
    assert_eq!(run_mode(oob, false).unwrap(), ["0"]);

    let div = "init() {\n  var z = 0\n  print(\"{7 / z} {7 % z}\", 0, 0)\n}";
    assert_eq!(run_mode(div, true).unwrap_err().0, FaultKind::DivisionByZero);
    assert_eq!(run_mode(div, false).unwrap(), ["0 0"]);

    let grid = "var g[2][3]\ninit() {\n  var x = 3\n  g[0][x] = 1\n}";
    assert!(matches!(
        run_mode(grid, true).unwrap_err().0,
        FaultKind::OutOfBounds { index: 3, size: 3, .. }
    ));

    let deep = "fn f(n) { return f(n + 1) }\ninit() { f(0) }";
    assert_eq!(run_mode(deep, false).unwrap_err().0, FaultKind::CallDepth);
}

#[test]
fn budget_and_resume() {
    let bytes = cart("var n = 0\nupdate() {\n  for (var i = 0; i < 1000; i++) { n++ }\n}");
    let mut scratch = vec![0u8; bidule_bytecode::limits::FN_CODE_BYTES];
    let mut vm = Vm::load(&bytes, &mut scratch, true, 1).unwrap();
    let mut host = Rec::default();
    assert!(vm.start(Handler::Update, &[0, 0]));
    let mut slices = 0;
    loop {
        slices += 1;
        match vm.run(&mut host, 100) {
            Exit::Budget => continue,
            Exit::Done(_) => break,
            e => panic!("{:?}", e),
        }
    }
    assert!(slices > 10);
    assert_eq!(vm.global(0), 1000);
}

#[test]
fn audio_handler() {
    let bytes = cart("var pitch = 3\naudio var phase = 0\naudio(t) {\n  phase += 1\n  return (t * pitch + phase) & 255\n}");
    let mut scratch = vec![0u8; bidule_bytecode::limits::FN_CODE_BYTES];
    let mut vm = Vm::load(&bytes, &mut scratch, true, 1).unwrap();
    let mut host = Rec::default();
    assert_eq!(vm.audio_sample(10, &mut host), 31);
    assert_eq!(vm.audio_sample(10, &mut host), 32);
    assert!(vm.audio_fault.is_none());
}

#[test]
fn compile_errors() {
    assert!(errors("init() {\n  scroe = 1\n}\nvar score").contains("did you mean 'score'"));
    assert!(errors("var x\ninit() { x = 1.5 }").contains("int(x)"));
    assert!(errors("var a[4]\nvar b[4]\ninit() { a = b }").contains("copy"));
    assert!(errors("const T[] = {1}\ninit() { T[0] = 2 }").contains("read-only"));
    assert!(errors("fn w(a[]) { a[0] = 1 }\ninit() { w(\"hi\") }").contains("writes"));
    assert!(errors("var g\naudio(t) {\n  g = t\n  return 0\n}").contains("audio(t)"));
    assert!(errors("record A { b: B }\nrecord B { a: A }").contains("itself"));
    assert!(errors("init() {\n  var x = 1\n  var x = 2\n}").contains("already"));
    assert!(errors("init() { print(\"{1:q}\", 0, 0) }").contains("format"));
}

/// A record whose fields fail used to crash the compiler wherever it was used.
#[test]
fn broken_records() {
    for src in [
        "record R {}\nvar r: R",
        "record R {}\nvar rs: R[4]",
        "record R {}\nconst T: R[] = { {} }",
        "record R {}\nfn f(r: R) {}",
        "record R { p: Nope }\nvar r: R",
    ] {
        let e = errors(src);
        assert!(e.contains("record R has errors"), "{}: {}", src, e);
    }
    // a record that contains a failed one says so, and is not "contained in itself"
    for src in [
        "record B {}\nrecord A { b: B }\nvar a: A",
        "record A { b: B }\nrecord B {}\nvar a: A",
    ] {
        let e = errors(src);
        assert!(
            e.contains("at least one field") && e.contains("record A has errors"),
            "{}: {}",
            src,
            e
        );
        assert!(!e.contains("itself"), "{}: {}", src, e);
    }
}
