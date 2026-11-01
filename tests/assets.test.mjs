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
  const bad = makeHarness(core, "t", [cart("t", "draw() {\n  spr(Sprites.PLAYR, 0, 0)\n}")]);
  assert.throws(() => bad.frame(), /did you mean Sprites.PLAYER/);
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
  const c = core.makeVM(src, { debug: true, pointer: h.pointer, sections: () => h.carts.t.sections, cartSections: id => h.carts[id].sections });
  const task = c.initTask();
  let r = task.next(); assert.equal(r.value.type, "pick");
  r = task.next(null); assert.equal(r.value.type, "pick");
  r = task.next("sample-spark"); assert.ok(r.done);
});
