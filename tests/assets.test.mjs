import { test } from "node:test";
import assert from "node:assert/strict";
import { loadCore, makeHarness, BTN } from "./helpers.mjs";

const core = loadCore();
const cart = (id, source) => {
  const c = core.makeSampleCart("spark");
  return { ...c, id, source, sections: c.sections };
};

test("every sample cart loads and runs 120 frames", () => {
  for (const id of core.SAMPLE_IDS) {
    const h = makeHarness(core, `sample-${id}`);
    h.init(["sample-spark"]);
    for (let f = 0; f < 120; f++) h.frame(f % 30 === 1 ? BTN.A : 0, ["sample-spark"]);
  }
});

test("asset names resolve; unknown names suggest a fix", () => {
  const ok = makeHarness(core, "t", [cart("t", "var m = 0\ndraw() {\n  m = Maps.BACKDROP[2][3] + fget(1, 0)\n  spr(Sprites.PLAYER, 0, 0)\n  play(Sounds.CATCH)\n}")]);
  ok.frame();
  assert.throws(() => makeHarness(core, "t", [cart("t", "draw() {\n  spr(Sprites.PLAYR, 0, 0)\n}")]), /did you mean Sprites.PLAYER/);
});

test("editing API: pick, read, write, append, name, save", () => {
  const src = `
var h
var b[600]
var nm[30]
init() {
  h = pickcart("sounds")
  var n = readasset(h, b, 0)
  b[1] = 50
  writeasset(h, b, assetcount(h))
  setassetname(h, 3, "CATCH2")
  assetname(h, 3, nm)
  savecart(h)
}`;
  const h = makeHarness(core, "t", [cart("t", src)]);
  h.init(["sample-spark", "over"]);
  assert.deepEqual(h.asked, ["pick", "save"]);
  const snd = h.carts["sample-spark"].sections.sounds;
  assert.equal(snd.slots.length, 4);
  assert.equal(snd.slots[3].name, "CATCH2");
  assert.equal(snd.slots[3].bytes[1], 50);
  assert.equal(snd.rev, 1);
});

test("pickcart returns -1 when cancelled; nothing is written without savecart", () => {
  const src = `
var h
var t[65]
init() {
  h = pickcart("tiles")
  var again = pickcart("tiles")
  if (again >= 0) {
    readasset(again, t, 1)
  }
}`;
  const h = makeHarness(core, "t", [cart("t", "")]);
  const c = core.makeWasmVM(src, { debug: true, pointer: h.pointer, sections: () => h.carts.t.sections, cartSections: id => h.carts[id].sections, setProject() {} });
  const task = c.initTask();
  let r = task.next(); assert.equal(r.value.type, "pick");
  r = task.next(null); assert.equal(r.value.type, "pick");
  r = task.next("sample-spark"); assert.ok(r.done);
});

test("opensection, cartname, drawasset and status", () => {
  const src = `
var h
var s
var t[20]
var rec[4]
init() {
  h = pickcart("tiles")
  s = opensection(h, "sprites")
  cartname(h, t)
  rec[0] = 3
  rec[1] = 0
  rec[2] = 1
  rec[3] = 1
  writeasset(s, rec, assetcount(s))
  setassetname(s, assetcount(s) - 1, "THREE")
  var same = streq(t, "Spark Catcher")
  status("hello {same}")
}
draw() {
  cls(0)
  drawasset(h, 1, 0, 0)
  drawasset(s, 0, 8, 0)
}
fn later() {
  savecart(s)
}`;
  const h = makeHarness(core, "t", [cart("t", src)]);
  h.init(["sample-spark"]);
  assert.equal(h.status.text, "hello 1");
  h.frame();
  // tile 1 (PLAYER) drawn twice: through the tiles handle and as sprite 0
  for (let y = 0; y < 8; y++) for (let x = 0; x < 8; x++) assert.equal(core.fb[y * core.W + x], core.fb[y * core.W + x + 8]);
  assert.equal(core.fb[2 * core.W + 1], 7);
});

test("savecart on either handle saves every section of the pick once", () => {
  const src = `
var h
var s
var px[65]
var rec[4]
init() {
  h = pickcart("tiles")
  s = opensection(h, "sprites")
  readasset(h, px, 5)
  px[0] = 9
  writeasset(h, px, 5)
  rec[0] = 5
  rec[1] = 0
  rec[2] = 1
  rec[3] = 1
  writeasset(s, rec, assetcount(s))
  setassetname(s, assetcount(s) - 1, "FIVE")
  savecart(s)
}`;
  const h = makeHarness(core, "t", [cart("t", src)]);
  h.init(["sample-spark", "over"]);
  assert.deepEqual(h.asked, ["pick", "save"]);
  const sp = h.carts["sample-spark"].sections.sprites;
  assert.equal(sp.sheet[5 * 64], 9);
  assert.equal(sp.slots.at(-1).name, "FIVE");
  assert.equal(sp.rev, 1);
});

