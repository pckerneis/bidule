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
  const html = assemblePage().replace("loadShelf();\n", "window.__fb = fb; loadShelf();\n");
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
