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
