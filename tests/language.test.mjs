import { test } from "node:test";
import assert from "node:assert/strict";
import { loadCore, runSource } from "./helpers.mjs";

const core = loadCore();

// Language checks written as a cart: check(c) divides by zero when c is false,
// so debug mode stops on the failing line.
const SELF_CHECK = `
record Vec2 { x: fix, y: fix }
record Enemy { pos: Vec2, path: int[3], hp }
enum State { A, B = 5, C }
const ROOM[2][4] = {
  "#..#",
  "####",
}
record Weapon { damage, speed: fix, cooldown }
const WEAPONS: Weapon[] = {
  { damage: 1, speed: 4.0, cooldown: 8 },
  {3, 2.5, 20},
}
const TITLE = "BQ"
var es: Enemy[2]
var out[64]

fn check(c) {
  if (!c) {
    var z = 0
    var fail = 1 / z
  }
}

fn hurt(e: Enemy, d) {
  e.hp -= d
}

fn fact(n) {
  if (n <= 1) { return 1 }
  return n * fact(n - 1)
}

fn half(v: fix): fix {
  return v / 2
}

init() {
  check(7 / 2 == 3)
  check(-7 / 2 == -3)
  check(-7 % 2 == -1)
  check((0x7FFFFFFF + 1) == 0x80000000)
  check((1 << 33) == 2)
  check(-8 >> 1 == -4)
  check((1 < 2) + (2 < 1) == 1)
  check((3 && 4) == 1)
  check(fact(5) == 120)

  var f: fix = 1.25
  check(f * 4 == 5)
  check(half(3) == 1.5)
  check(int(-1.5) == -2)
  check(round(2.5) == 3)
  check(-7.5 % 2 == -1.5)
  check(sin(30) + cos(60) == 1)
  check(sin(90) == 1)
  check(sin(-30) == -sin(30))
  check(sqrt(4.0) == 2)
  check(atan2(1, 1) == 45)

  es[0].pos.x = 1.25
  es[0].path[2] = 9
  es[1] = es[0]
  hurt(es[1], 3)
  check(es[1].pos.x == 1.25 && es[1].path[2] == 9 && es[1].hp == -3)
  check(es.length == 2)

  check(State.C == 6)
  var s = 0
  switch (State.C) {
    State.A { s = 1 }
    State.B, State.C { s = 2 }
    else { s = 3 }
  }
  check(s == 2)

  check(ROOM[0][1] == '.' && ROOM[1][3] == '#')
  check(ROOM.length == 2 && ROOM[0].length == 4)
  check(WEAPONS[1].speed == 2.5 && WEAPONS.length == 2)
  check(textwidth(TITLE) == 12)

  fmt(out, "{3:04}|{0x2A:x}|{65:c}|{sqrt(2.0):.3}|{1.5}|{1 < 2 ? 10 : 20}")
  check(streq(out, "0003|2a|A|1.414|1.5|10"))
}
`;

test("language self-check cart passes", () => {
  runSource(core, SELF_CHECK);
});

const errorCases = [
  ["typo suggestion", "var score = 0\nupdate() {\n  scroe += 1\n}", 3, /did you mean 'score'/],
  ["fix into int", "var a = 0\nupdate() {\n  a = 1.5\n}", 3, /int\(x\) or round\(x\)/],
  ["out of bounds", "var a[4]\nupdate(f) {\n  a[f + 4] = 1\n}", 3, /out of bounds \(a\[4\], size 4\)/],
  ["division by zero", "update() {\n  var z = 0\n  var q = 4 / z\n}", 3, /division by zero/],
  ["parse error", "update() {\n  if (x > 1 {\n  }\n}", 2, /expected '\)'/],
  ["read-only table", "const T[] = {1,2}\ninit() {\n  T[0] = 5\n}", 3, /read-only/],
  ["call depth", "fn f(n) {\n  return f(n + 1)\n}\ninit() {\n  f(0)\n}", 2, /call depth/],
  ["no shadowing", "var x = 1\nfn f() {\n  var x = 2\n}\ninit() {\n  f()\n}", 3, /locals can't reuse its name/],
  ["fn without 'fn'", "foo() {\n}", 1, /declare it with 'fn foo/],
];
for (const [name, src, line, re] of errorCases) {
  test(`error: ${name}`, () => {
    assert.throws(() => runSource(core, src), e => { assert.equal(e.line, line); assert.match(e.message, re); return true; });
  });
}

test("release mode returns 0 instead of trapping", () => {
  runSource(core, "var a[4]\nupdate(f) {\n  var z = 0\n  var q = 4 / z\n  a[9] = 1\n  q = a[9]\n}", { debug: false });
});

test("bytebeat audio(t) and audio vars", () => {
  const v = runSource(core, "audio var pitch = 4\naudio(t) {\n  return (t * pitch) & 255\n}");
  assert.deepEqual([0, 1, 2, 100].map(t => v.audioSample(t)), [0, 4, 8, 144]);
});
