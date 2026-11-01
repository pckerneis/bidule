import { test } from "node:test";
import assert from "node:assert/strict";
import { loadCore, makeHarness, BTN } from "./helpers.mjs";

const core = loadCore();
const { A, B, R, D } = BTN;

// Layout constants from carts/sprite-editor.bdcart
const CX = 6, CY = 16, Z = 11, TOOLX = 104, TOOLY = 68, FLAGX = 104, FLAGY = 98;

test("a scripted Sprite Editor session edits and saves another cart's tiles", () => {
  const h = makeHarness(core, "sample-sprites");
  const click = (x, y, answers = []) => { h.pointer.x = x; h.pointer.y = y; h.pointer.b = 1; h.frame(0, answers); h.pointer.b = 0; h.frame(0); };
  h.init(["sample-spark"]);
  h.frame();

  // B + down: next tile (tile 1), then paint (0,0) and (1,0) with colour 7
  h.frame(B); h.frame(B | D); h.frame(B); h.frame(0);
  h.frame(A); h.frame(0);
  h.frame(R); h.frame(0); h.frame(A); h.frame(0);

  // tap B: sheet panel; right: tile 2; A: back to the canvas
  h.frame(B); h.frame(0); h.frame(R); h.frame(0); h.frame(A); h.frame(0);

  // FILL tool, then flood fill from the top-left pixel
  click(TOOLX + 44 + 5, TOOLY + 3);
  click(CX + 2, CY + 2);
  // toggle flag 2
  click(FLAGX + 36 + 2 * 10 + 3, FLAGY + 3);
  // SAVE → overwrite
  click(TOOLX + 2 * 44 + 5, TOOLY + 13 + 3, ["over"]);

  assert.deepEqual(h.asked, ["pick", "save"]);
  const s = h.carts["sample-spark"].sections.sprites;
  assert.equal(s.rev, 1);
  assert.equal(s.sheet[64], 7);
  assert.equal(s.sheet[65], 7);
  assert.equal(Array.from(s.sheet.subarray(128 + 24, 128 + 32)).join(","), "7,9,10,7,10,9,7,7");
  assert.equal(s.flags[2], 4);
});

test("an idle editor frame stays cheap", () => {
  const h = makeHarness(core, "sample-sprites");
  h.init(["sample-spark"]);
  h.frame();
  const t = performance.now();
  for (let i = 0; i < 60; i++) h.frame();
  const per = (performance.now() - t) / 60;
  assert.ok(per < 8, `idle frame took ${per.toFixed(2)} ms`);
});
