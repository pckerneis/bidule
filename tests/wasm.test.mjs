// The console the page ships: the Rust compiler, VM and runtime, as WebAssembly.
import { test } from "node:test";
import assert from "node:assert/strict";
import { loadCore, makeHarness, runSource, BTN } from "./helpers.mjs";

const core = loadCore();
const { A, R, D, B } = BTN;
const run = (source, opts = {}) => {
  const sections = core.emptySections();
  const notes = [];
  const v = core.makeWasmVM(source, { debug: true, pointer: { x: 0, y: 0, b: 0 }, sections: () => sections, cartSections: () => sections, cartTitle: () => "T", setStatusLine: t => notes.push(t), audioFault: e => notes.push(e), ...opts });
  return { v, notes };
};
const FB = () => core.fb.slice();

// rnd in the VM: xorshift32 with this seed; Math.random below gives the reference the same draws.
const SEED = 0x12345678;
function withReferenceRandom(fn) {
  const saved = Math.random;
  let rng = SEED;
  Math.random = () => { rng ^= rng << 13; rng >>>= 0; rng ^= rng >>> 17; rng ^= rng << 5; rng >>>= 0; return rng / 4294967296; };
  try { return fn(); } finally { Math.random = saved; }
}

function framesOf(id, engine, inputs, answers = []) {
  const h = makeHarness(core, `sample-${id}`, [], engine);
  h.init(answers.slice());
  return inputs.map(i => { h.frame(i, []); return FB(); });
}

for (const [id, inputs, answers] of [
  ["blink", [0, 0, A, 0, 0, A, 0, 0, 0, A].concat(Array(50).fill(0)), []],
  ["spark", [0, 0, 0, A, 0].concat(Array(40).fill(R), Array(20).fill(R | A), Array(40).fill(D), [A], Array(30).fill(0)), []],
  ["sprites", [0, B, B | D, B, 0, A, 0, R, 0, A, 0, B, 0, R, 0, A, 0], ["sample-spark"]],
]) {
  test(`${id}: draws what the reference interpreter draws`, () => {
    const seeded = (src, host) => core.makeWasmVM(src, { ...host, seed: SEED });
    const expect = withReferenceRandom(() => framesOf(id, core.makeVM, inputs, answers));
    const got = framesOf(id, seeded, inputs, answers);
    got.forEach((f, n) => {
      const i = f.findIndex((c, k) => c !== expect[n][k]);
      assert.equal(i, -1, `frame ${n} differs first at (${i % 240}, ${Math.floor(i / 240)})`);
    });
  });
}

test("compile errors appear at load, for lines that never run", () => {
  assert.throws(() => run("fn never() {\n  nope()\n}\ndraw() {\n}"), e => e instanceof core.BErr && e.line === 2);
});

test("several errors are counted in the message", () => {
  assert.throws(() => run("draw() {\n  a()\n  b()\n}"), /and 1 more/);
});

test("non-ASCII source and messages survive the trip", () => {
  run("// résumé ✓\ndraw() {\n  cls(1)\n}");
  assert.throws(() => run("var é = 1"), core.BErr);
});

test("absurdly nested source fails to load instead of breaking the page", () => {
  const deep = `var a = ${"(".repeat(100000)}1${")".repeat(100000)}`;
  assert.throws(() => run(deep));
  run("draw() {\n}");   // the next cart gets a fresh instance
});

test("an endless loop never blocks: it reports late and can be abandoned", () => {
  const { v } = run("update() {\n  while (1) {\n  }\n}");
  const t = v.frameTask(0, 0);
  for (let i = 0; i < 3; i++) {
    const r = t.next();
    assert.equal(r.done, false);
    assert.equal(r.value.type, "late");
    assert.equal(r.value.stage, "update");
  }
});

test("a handler over budget finishes on a later frame", () => {
  const { v } = run("var i\nvar n = 0\nupdate() {\n  while (i < 3000000) {\n    i = i + 1\n  }\n  cls(9)\n}");
  const t = v.frameTask(0, 0);
  let r = t.next();
  assert.equal(r.value.type, "late");
  while (!r.done) r = t.next();
  assert.equal(core.fb[0], 9);
});

test("faults carry the stage and line", () => {
  const { v } = run("var a[2]\nvar i = 5\ndraw() {\n  cls(a[i])\n}");
  assert.throws(() => v.frameTask(0, 0).next(), e => e.stage === "draw" && e.line === 4 && /a\[5\], size 2/.test(e.message));
});

test("editing API errors keep their message and stop the cart on the line", () => {
  const { v } = run('init() {\n  cls(0)\n  assetcount(7)\n}');
  assert.throws(() => v.initTask().next(), e => e.stage === "init" && e.line === 3 && /7 is not a cart handle/.test(e.message));
  const u = run('init() {\n  pickcart("nope")\n}');
  assert.throws(() => u.v.initTask().next(), e => e.line === 2 && /unknown asset kind "nope"/.test(e.message));
});

test("the status line reaches the host", () => {
  const { v, notes } = run('init() {\n  status("hello")\n}');
  v.initTask().next();
  assert.deepEqual(notes, ["hello"]);
});

test("the palette reaches the host; asset palettes apply at load", () => {
  const sections = core.emptySections();
  sections.palette = { rev: 0, words: new Uint16Array(256).fill(0x07E0) };
  const { v } = run("draw() {\n  cls(3)\n}", { sections: () => sections });
  v.frameTask(0, 0).next();
  assert.equal(core.fb[0], 3);
});

test("bytebeat audio runs, and a fault silences it once", () => {
  const ok = runSource(core, "audio var pitch = 4\naudio(t) {\n  return (t * pitch) & 255\n}");
  assert.equal(ok.hasAudio, true);
  assert.equal(ok.audioSample(3), 12);
  const { v, notes } = run("audio(t) {\n  var z = 0\n  return t / z\n}");
  assert.equal(v.audioSample(1), 128);
  assert.equal(v.audioSample(2), 128);
  assert.equal(notes.length, 1);
  assert.equal(notes[0].line, 3);
  assert.match(notes[0].message, /division by zero/);
});

test("rnd is seeded: same seed, same frames; the host picks a new one otherwise", () => {
  const src = "draw() {\n  cls(0)\n  for (var i = 0; i < 200; i += 1) {\n    pset(rnd(240), rnd(160), 7)\n  }\n}";
  const shot = seed => { const { v } = run(src, { seed }); v.frameTask(0, 0).next(); return FB(); };
  assert.deepEqual(shot(7), shot(7));
  assert.notDeepEqual(shot(7), shot(8));
});

test("reloading many times is fine", () => {
  const h = makeHarness(core, "sample-sprites");
  for (let i = 0; i < 100; i++) run(h.carts["sample-sprites"].source, { sections: () => h.carts["sample-sprites"].sections });
});

test("X and Y are reserved: the constants exist and always read as released", () => {
  const { v } = run("var x = 9\nvar y = 9\ndraw() {\n  cls(btn(BTN_X) + btn(BTN_Y) * 2 + btnp(BTN_X) + 1)\n}\nupdate() {\n  x = BTN_X\n  y = BTN_Y\n}");
  const drain = t => { const r = t.next(); assert.ok(r.done); };
  drain(v.initTask());
  drain(v.frameTask(0, 64 | 128 | 16));                  // X and Y pressed on the pad: bits 6 and 7
  assert.equal(core.fb[0], 1, "cls(1): both read 0");
});

test("menuitem and menuchosen are reserved names that can't be called or declared yet", () => {
  assert.throws(() => run('draw() {\n  menuitem(0, "Save")\n}'), /menuitem\(\) is reserved/);
  assert.throws(() => run("draw() {\n  var n = menuchosen()\n}"), /menuchosen\(\) is reserved/);
  assert.throws(() => run("var menuitem = 1"), /reserved/);
});

test("cartcount, cartinfo and opencart read other carts without a dialog; writes need requestwrite", () => {
  const src = `
var h
var t[65]
var name[20]
var wrote
var allowed
var n
var len
init() {
  n = cartcount()
  len = cartinfo(1, name)
  h = opencart(-1, "tiles")
  if (h < 0) { h = opencart(1, "tiles") }
  readasset(h, t, 0)
  wrote = writeasset(h, t, 0)
  allowed = requestwrite(h)
  if (allowed) { wrote = writeasset(h, t, 0) }
}`;
  const h = makeHarness(core, "sample-sprites");
  const c = core.makeWasmVM(src, { debug: true, pointer: h.pointer, sections: () => h.carts["sample-sprites"].sections, cartSections: id => h.carts[id].sections, cartTitle: id => (id === "sample-spark" ? "Spark Catcher" : "x"), cartIds: () => Object.keys(h.carts), projectId: () => null, setProject() {}, runningTitle: () => "T" });
  const task = c.initTask();
  let r = task.next();
  assert.equal(r.value.type, "write", "reading needed no dialog; writing asks the user");
  assert.equal(r.value.target, "Spark Catcher");
  assert.equal(c.dirty(), false);
  r = task.next(true);
  assert.ok(r.done);
  assert.equal(c.dirty(), true, "after the user allowed it, the write went through");
});

test("a refused write request leaves the handle read-only", () => {
  const src = `
var h
var t[65]
var wrote = 5
var allowed = 5
init() {
  h = opencart(1, "tiles")
  readasset(h, t, 0)
  allowed = requestwrite(h)
  wrote = writeasset(h, t, 0)
  status("{allowed} {wrote}")
}`;
  const h = makeHarness(core, "sample-sprites");
  const notes = [];
  const c = core.makeWasmVM(src, { debug: true, pointer: h.pointer, sections: () => h.carts["sample-sprites"].sections, cartSections: id => h.carts[id].sections, cartTitle: () => "T", cartIds: () => Object.keys(h.carts), projectId: () => null, setProject() {}, runningTitle: () => "T", setStatusLine: t => notes.push(t) });
  const task = c.initTask();
  let r = task.next();
  assert.equal(r.value.type, "write");
  r = task.next(false);
  assert.ok(r.done);
  assert.equal(notes.at(-1), "0 -1", "refused: requestwrite is 0 and writeasset reports -1");
  assert.equal(c.dirty(), false);
});
