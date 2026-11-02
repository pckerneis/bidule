// Runs the built page in a simulated browser: shelf, picker dialog, input hand-off.
import { test, after } from "node:test";
import assert from "node:assert/strict";
import { assemblePage } from "../scripts/build.mjs";

let JSDOM;
try { ({ JSDOM } = await import("jsdom")); } catch { /* optional */ }

const opened = [];
after(() => opened.forEach(w => w.close()));
const wait = ms => new Promise(r => setTimeout(r, ms));
function boot(seed) {
  const html = assemblePage().replace("loadShelf();\n", "window.__fb = fb; window.__ptr = pointer; loadShelf();\n");
  const dom = new JSDOM(html, { url: "https://bidule.test/", runScripts: "outside-only", pretendToBeVisual: true });
  const w = dom.window;
  if (seed) w.localStorage.setItem("bidule01.shelf.v1", JSON.stringify(seed));
  w.HTMLCanvasElement.prototype.getContext = () => ({ createImageData: (a, b) => ({ data: new Uint8ClampedArray(a * b * 4) }), putImageData() {} });
  w.HTMLCanvasElement.prototype.getBoundingClientRect = () => ({ left: 0, top: 0, width: 240, height: 160 });
  w.ResizeObserver = class { observe() {} };
  const errors = []; w.addEventListener("error", e => errors.push(e.message));
  const script = [...w.document.querySelectorAll("script")].pop().textContent;
  w.eval(script);
  opened.push(w);
  return { w, errors, $: id => w.document.getElementById(id) };
}

test("page boots with the sample shelf", { skip: !JSDOM && "jsdom not installed" }, async () => {
  const { $, errors } = boot();
  await wait(150);
  assert.match($("statusText").textContent, /Running “Spark Catcher”/);
  assert.deepEqual([...$("cartSel").options].map(o => o.value), ["sample-sprites", "sample-spark", "sample-peek", "sample-blink"]);
  assert.deepEqual(errors, []);
});

test("a picker click is not seen by the cart until released", { skip: !JSDOM && "jsdom not installed" }, async () => {
  const { w, $ } = boot();
  const cv = $("screen");
  const pe = (type, x, y, buttons) => cv.dispatchEvent(new w.MouseEvent(type, { clientX: x, clientY: y, buttons, bubbles: true }));
  await wait(150);
  $("cartSel").value = "sample-sprites"; $("cartSel").dispatchEvent(new w.Event("change"));
  await wait(150);
  const cell = () => w.__fb[(16 + 3) * 240 + (6 + 3)];
  pe("pointerdown", 40, 50, 1);         // second picker row, over the canvas: Spark Catcher
  pe("pointermove", 9, 19, 1); await wait(250);
  assert.match($("shelfNote").textContent + $("statusText").textContent, /Sprite Editor/);
  assert.equal(cell(), 0, "held click must not paint");
  pe("pointerup", 9, 19, 0); await wait(80);
  pe("pointerdown", 9, 19, 1); await wait(200); pe("pointerup", 9, 19, 0); await wait(80);
  assert.equal(cell(), 7, "a new click paints");
});

test("untouched samples update, edited ones are kept", { skip: !JSDOM && "jsdom not installed" }, async () => {
  const { $ } = boot({ current: "sample-blink", seen: ["sprites", "spark", "peek", "blink"], carts: [
    { id: "sample-blink", sampleId: "blink", source: "// @title  Blink edited\ndraw() {\n  cls(2)\n}", sections: null },
  ] });
  await wait(150);
  assert.equal($("cartSel").options[0].textContent, "Blink edited · sample");
});

test("the page shows compile errors, survives an endless loop, and resets", { skip: !JSDOM && "jsdom not installed" }, async () => {
  const { $, w, errors } = boot();
  await wait(100);
  $("code").value = "draw() {\n  nope()\n}";
  $("runBtn").click();
  assert.match($("statusText").textContent, /Can't load, line 2/);
  $("code").value = "update() {\n  while (1) {\n  }\n}";
  $("runBtn").click();
  await wait(150);
  assert.match($("statusText").textContent, /update\(\) is taking more than one frame/);
  $("code").value = "// @title  Fine\ndraw() {\n  cls(4)\n}";
  $("runBtn").click();
  await wait(100);
  assert.equal(w.__fb[0], 4);
  assert.deepEqual(errors, []);
});

const SKIP_JSDOM = !JSDOM && "jsdom not installed";
// A finger on the full-screen overlay: a mouse event with a pointer id, which is all the page reads.
function finger(w, el, type, id, x, y, extra = {}) {
  const e = new w.MouseEvent(type, { clientX: x, clientY: y, buttons: extra.buttons || 0, bubbles: true, cancelable: true });
  Object.defineProperty(e, "pointerId", { value: id });
  if (extra.pointerType) Object.defineProperty(e, "pointerType", { value: extra.pointerType });
  el.dispatchEvent(e);
}

test("full screen moves the screen into the overlay and back", { skip: SKIP_JSDOM }, async () => {
  const { w, $, errors } = boot();
  await wait(100);
  const home = $("screen").parentElement;
  assert.equal($("fs").hidden, true);
  $("fsBtn").click();
  assert.equal($("fs").hidden, false);
  assert.equal($("screen").parentElement, $("fsStage"));
  assert.ok(w.document.documentElement.classList.contains("fs-on"));
  assert.equal($("fsBtn").getAttribute("aria-pressed"), "true");
  $("fsExit").click();
  assert.equal($("fs").hidden, true);
  assert.equal($("screen").parentElement, home);
  assert.ok(!w.document.documentElement.classList.contains("fs-on"));
  w.dispatchEvent(new w.KeyboardEvent("keydown", { code: "KeyF", bubbles: true }));
  assert.equal($("fs").hidden, false, "F enters full screen");
  w.dispatchEvent(new w.KeyboardEvent("keydown", { code: "Escape", bubbles: true }));
  assert.equal($("fs").hidden, true, "Escape leaves it when the browser has no native full screen");
  assert.deepEqual(errors, []);
});

test("full screen overlay: multi-touch, sliding between buttons, diagonals", { skip: SKIP_JSDOM }, async () => {
  const { w, $, errors } = boot();
  await wait(100);
  $("fsBtn").click();
  const [dpad, ab] = [...w.document.querySelectorAll(".fs-zone")];
  const rect = (l, t, wd, ht) => ({ left: l, top: t, width: wd, height: ht, right: l + wd, bottom: t + ht });
  dpad.getBoundingClientRect = () => rect(0, 0, 120, 120);
  const btn = n => w.document.querySelector(`.fs [data-fb="${n}"]`);
  btn(5).getBoundingClientRect = () => rect(300, 100, 60, 60);   // B
  btn(4).getBoundingClientRect = () => rect(374, 60, 60, 60);    // A
  ab.getBoundingClientRect = () => rect(300, 60, 134, 100);
  const on = n => btn(n).classList.contains("down-state");

  finger(w, dpad, "pointerdown", 1, 10, 60);                     // left of centre
  assert.deepEqual([0, 1, 2, 3].map(on), [true, false, false, false]);
  finger(w, dpad, "pointermove", 1, 110, 60);                    // slides across: left off, right on
  assert.deepEqual([0, 1, 2, 3].map(on), [false, true, false, false]);
  finger(w, dpad, "pointermove", 1, 100, 10);                    // up-right diagonal holds two buttons
  assert.deepEqual([0, 1, 2, 3].map(on), [false, true, true, false]);
  finger(w, dpad, "pointermove", 1, 60, 60);                     // dead centre
  assert.deepEqual([0, 1, 2, 3].map(on), [false, false, false, false]);

  finger(w, dpad, "pointermove", 1, 110, 60);
  finger(w, ab, "pointerdown", 2, 330, 130);                     // second finger on B while right is held
  assert.deepEqual([1, 4, 5].map(on), [true, false, true]);
  finger(w, ab, "pointermove", 2, 404, 90);                      // B to A without lifting
  assert.deepEqual([1, 4, 5].map(on), [true, true, false]);
  finger(w, dpad, "pointerup", 1, 110, 60);
  assert.deepEqual([1, 4, 5].map(on), [false, true, false]);
  finger(w, ab, "pointercancel", 2, 404, 90);
  assert.deepEqual([1, 4, 5].map(on), [false, false, false]);
  assert.deepEqual(errors, []);
});

test("full screen overlay buttons reach the cart and are released on exit", { skip: SKIP_JSDOM }, async () => {
  const { w, $, errors } = boot();
  await wait(100);
  $("code").value = "draw() {\n  cls(1)\n  if (btn(BTN_A)) {\n    cls(4)\n  }\n}";
  $("runBtn").click();
  $("fsBtn").click();
  const ab = w.document.querySelectorAll(".fs-zone")[1];
  const a = w.document.querySelector('.fs [data-fb="4"]');
  a.getBoundingClientRect = () => ({ left: 374, top: 60, width: 60, height: 60, right: 434, bottom: 120 });
  await wait(100);
  assert.equal(w.__fb[0], 1);
  finger(w, ab, "pointerdown", 7, 404, 90); await wait(100);
  assert.equal(w.__fb[0], 4, "A on the overlay is BTN_A for the cart");
  $("fsExit").click(); await wait(100);
  assert.equal(w.__fb[0], 1, "leaving full screen releases held buttons");
  assert.deepEqual(errors, []);
});

test("full screen mode switch: buttons hidden and the screen is the pointer in mouse mode", { skip: SKIP_JSDOM }, async () => {
  const { w, $, errors } = boot();
  await wait(100);
  const cv = $("screen"), touch = (type, x, y, buttons) => finger(w, cv, type, 3, x, y, { pointerType: "touch", buttons });
  $("fsBtn").click();
  assert.equal($("fsMode").textContent, "⇄ Buttons");
  assert.ok(!$("fs").classList.contains("fs-mouse"));

  touch("pointerdown", 50, 60, 1);                               // button mode: a finger on the screen is not input
  assert.equal(w.__ptr.b, 0);
  touch("pointerup", 50, 60, 0);
  finger(w, cv, "pointerdown", 4, 20, 30, { pointerType: "mouse", buttons: 1 });
  assert.deepEqual([w.__ptr.x, w.__ptr.y, w.__ptr.b], [20, 30, 1], "a physical mouse always works");
  finger(w, cv, "pointerup", 4, 20, 30, { pointerType: "mouse" });

  $("fsMode").click();
  assert.ok($("fs").classList.contains("fs-mouse"));
  assert.equal($("fsMode").textContent, "⇄ Mouse");
  touch("pointerdown", 50, 60, 1);
  assert.deepEqual([w.__ptr.x, w.__ptr.y, w.__ptr.b], [50, 60, 1]);
  touch("pointermove", 70, 80, 1);
  assert.deepEqual([w.__ptr.x, w.__ptr.y, w.__ptr.b], [70, 80, 1]);
  $("fsMode").click();                                           // switching back lets go of a held click
  assert.equal(w.__ptr.b, 0);
  touch("pointerup", 70, 80, 0);
  assert.equal(w.__ptr.b, 0);
  assert.ok(!$("fs").classList.contains("fs-mouse"));
  assert.deepEqual(errors, []);
});
