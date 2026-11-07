import { test } from "node:test";
import assert from "node:assert/strict";
import { loadCore, makeHarness, BTN } from "./helpers.mjs";

const core = loadCore();
const { A, B, L, R, U, D } = { ...BTN, L: BTN.L };

// Layout constants from carts/sprite-editor.bdcart
const TBX = 1, TBY = 3, TBP = 9, NPX = 14, NPY = 22, VX = 24, VY = 3, CLX = 219, CLY = 3, CLP = 9;
const toolAt = i => [TBX + 5, TBY + i * TBP + 4];
const cell = k => [NPX + 6 + (k % 10) * 20 + 5, NPY + 34 + Math.floor(k / 10) * 12 + 5];
const T = { PEN: 0, ERASE: 1, FILL: 2, PICK: 3, LINE: 4, RECT: 5, SEL: 6, MOVE: 7, HAND: 8, ZIN: 9, ZOUT: 10, UNDO: 11, NAME: 12, OPEN: 13, SAVE: 14 };
const pixel = (c, x, y) => c.sections.sprites.sheet[((y >> 3) * 32 + (x >> 3)) * 64 + (y & 7) * 8 + (x & 7)];

/** An editor session on Spark Catcher's sheet, driven by the buttons. */
function session() {
  const h = makeHarness(core, "sample-sprites");
  const tap = (b, answers = []) => { h.frame(b, answers); h.frame(0); };
  let panel = "view", ts = 0;
  const to = target => {                                  // B cycles view → colours → tools → view
    const order = ["view", "colours", "tools"];
    while (panel !== target) { tap(B); panel = order[(order.indexOf(panel) + 1) % 3]; }
  };
  const tool = (i, answers = []) => {
    to("tools");
    while (ts !== i) { const dy = (i - ts + 15) % 15 <= 7 ? D : U; tap(dy); ts = (ts + (dy === D ? 1 : 14)) % 15; }
    tap(A, answers);
    if (i < 9) panel = "view";
  };
  const colour = n => { to("colours"); for (let k = 0; k < n; k++) tap(D); tap(A); panel = "view"; };
  const click = (x, y, answers = []) => { h.pointer.x = x; h.pointer.y = y; h.pointer.b = 1; h.frame(0, answers); h.pointer.b = 0; h.frame(0); };
  const settle = () => { for (let i = 0; i < 200; i++) h.frame(); };   // messages last three seconds
  h.init(["sample-spark"]);
  h.frame();
  return { h, tap, to, tool, colour, click, settle, spark: h.carts["sample-spark"] };
}

test("three panels: B cycles them and the arrows act in the focused one", () => {
  const s = session();
  s.to("colours"); s.tap(D); s.tap(D);                     // colour 7 → 9
  s.to("view"); s.tap(A);                                  // pen paints (0,0)
  s.settle();
  s.to("tools"); s.tap(D); s.tap(D);                       // tool 2: FILL
  assert.match(s.h.status.text, /Up\/down: tool/);
  s.tap(A);
  s.tap(R); s.tap(R); s.tap(R); s.tap(R); s.tap(R); s.tap(R); s.tap(R); s.tap(R); s.tap(R);   // into the second tile
  s.tap(D);
  s.settle();
  assert.match(s.h.status.text, /FIL 9,1/);
});

test("pen, line and rect edit the sheet; SAV saves to the other cart", () => {
  const s = session();
  const c = s.spark;
  const untouched = [pixel(c, 0, 2), pixel(c, 7, 5), pixel(c, 10, 6)];
  s.colour(1);                                             // colour 8
  s.h.frame(A); s.h.frame(A | R); s.h.frame(A); s.h.frame(0);   // pen: hold A while moving right, (0,0)-(1,0)
  s.tap(D); s.tap(D);                                      // cursor (1,2)
  s.tool(T.LINE);
  s.tap(A); for (let i = 0; i < 5; i++) s.tap(R); s.tap(A);     // line (1,2)-(6,2)
  for (let i = 0; i < 2; i++) s.tap(D);                    // cursor (6,4)
  s.colour(1);                                             // colour 9
  s.tool(T.RECT);
  s.tap(A); for (let i = 0; i < 3; i++) s.tap(R); for (let i = 0; i < 2; i++) s.tap(D); s.tap(A);   // rect (6,4)-(9,6)
  assert.deepEqual(s.h.asked, ["pick"]);                   // nothing is written without SAV
  s.tool(T.SAVE, ["over"]);

  assert.equal(s.h.asked.join(), "pick,save");
  assert.equal(c.sections.sprites.rev, 1);
  assert.deepEqual([0, 1, 2].map(x => pixel(c, x, 0)), [8, 8, pixel(c, 2, 0)]);
  assert.equal(pixel(c, 0, 2), untouched[0]);              // the line starts at x = 1
  assert.deepEqual([1, 2, 3, 4, 5, 6].map(x => pixel(c, x, 2)), [8, 8, 8, 8, 8, 8]);
  assert.deepEqual([[6, 4], [9, 4], [6, 6], [9, 6], [7, 4], [6, 5]].map(([x, y]) => pixel(c, x, y)), [9, 9, 9, 9, 9, 9]);
  assert.equal(pixel(c, 7, 5), untouched[1]);              // the inside of the rect is not filled
  assert.equal(pixel(c, 10, 6), untouched[2]);
});

test("fill stops at other colours, and undo restores the last action", () => {
  const s = session();
  const before = pixel(s.spark, 3, 3);
  s.tool(T.FILL);
  s.tap(A);                                                // flood from (0,0) inside the loaded tiles
  s.tool(T.SAVE, ["over"]);
  assert.equal(pixel(s.spark, 0, 0), 7);
  assert.equal(pixel(s.spark, 3, 3), 7);
  assert.notEqual(before, 7);
  s.tool(T.UNDO);
  s.tool(T.SAVE, ["over"]);
  assert.equal(pixel(s.spark, 0, 0), 0);
  assert.equal(pixel(s.spark, 3, 3), before);
});

test("select, lift, move and drop a block of pixels", () => {
  const s = session();
  s.colour(1);
  s.h.frame(A); s.h.frame(A | R); s.h.frame(0);            // (0,0) and (1,0) in 8
  s.tap(L);
  s.tool(T.SEL);
  s.tap(A); s.tap(R); s.tap(A);                            // select (0,0)-(1,0)
  assert.match(s.h.status.text, /Selected 2x1/);
  s.tap(L);
  s.tool(T.MOVE);
  s.tap(A);                                                // lift
  for (let i = 0; i < 3; i++) s.tap(R);
  s.tap(A);                                                // drop
  s.tool(T.SAVE, ["over"]);
  assert.deepEqual([0, 1, 2, 3, 4].map(x => pixel(s.spark, x, 0)), [0, 0, 0, 8, 8]);     // the block moved from x 0-1 to x 3-4
});

test("zoom out to the tile overview, move by tiles, A zooms back in", () => {
  const s = session();
  s.tool(T.ZOUT); s.tool(T.ZOUT); s.tool(T.ZOUT);
  assert.match(s.h.status.text, /Overview/);
  s.to("view");
  s.tap(R); s.tap(R); s.tap(D);
  s.tap(A);
  assert.match(s.h.status.text, /Zoom x12/);
});

test("naming a tile appends a 1×1 sprite and renames existing ones", () => {
  const s = session();
  for (let i = 0; i < 17; i++) s.tap(R);                   // cursor in tile 2, which is SPARK_A
  s.click(...toolAt(T.NAME));
  for (let k = 0; k < 7; k++) s.click(...cell(37));        // DEL the prefilled SPARK_A
  for (const ch of "HERO") s.click(...cell(ch.charCodeAt(0) - 65));
  s.click(...cell(38));
  assert.match(s.h.status.text, /Sprites\.HERO/);
  s.click(...toolAt(T.SAVE), ["over"]);
  assert.deepEqual([...s.spark.sections.sprites.slots.map(x => x.name)], ["PLAYER", "HERO", "SPARK_B"]);

  // a tile without a sprite gets a new 1×1 sprite
  s.tap(D); s.tap(D); s.tap(D); s.tap(D); s.tap(D); s.tap(D); s.tap(D); s.tap(D); s.tap(D);
  for (let i = 0; i < 16; i++) s.tap(R);
  s.click(...toolAt(T.NAME));
  for (const ch of "DOT") s.click(...cell(ch.charCodeAt(0) - 65));
  s.click(...cell(38));
  s.click(...toolAt(T.SAVE), ["over"]);
  assert.deepEqual({ ...s.spark.sections.sprites.slots.at(-1) }, { name: "DOT", tile: 36, w: 1, h: 1 });
});

test("the mouse draws, picks colours and uses the toolbar", () => {
  const s = session();
  s.click(CLX + 5, CLY + 8 * CLP + 3);                    // colour 8
  s.h.pointer.x = VX + 12 * 2 + 3; s.h.pointer.y = VY + 12 * 1 + 3; s.h.pointer.b = 1; s.h.frame(0);
  s.h.pointer.x = VX + 12 * 4 + 3; s.h.frame(0);          // drag: a line from (2,1) to (4,1)
  s.h.pointer.b = 0; s.h.frame(0);
  s.click(...toolAt(T.SAVE), ["over"]);
  assert.deepEqual([1, 2, 3, 4, 5].map(x => pixel(s.spark, x, 1)), [0, 8, 8, 8, 0]);     // dragged from (2,1) to (4,1)
});

test("the project the editor opened is remembered; opening it again is read-only until the user allows it", () => {
  const h = makeHarness(core, "sample-sprites");
  h.state.project = "sample-spark";
  h.init([false]);                                         // the user refuses the write request
  h.frame();
  assert.deepEqual(h.asked, ["write"]);
  assert.match(h.status.text, /read only/i);
  h.frame(A); h.frame(0);                                  // the pen tries to paint
  assert.match(h.status.text, /Read only/);
  assert.equal(h.carts["sample-spark"].sections.sprites.sheet[0], 0);

  const g = makeHarness(core, "sample-sprites");
  g.state.project = "sample-spark";
  g.init([true]);                                          // allowed
  g.frame(); g.frame(A); g.frame(0);
  assert.match(g.status.text, /Editing|PEN/);
});

test("an idle editor frame stays cheap", () => {
  const s = session();
  const t = performance.now();
  for (let i = 0; i < 60; i++) s.h.frame();
  const per = (performance.now() - t) / 60;
  assert.ok(per < 8, `idle frame took ${per.toFixed(2)} ms`);
});
