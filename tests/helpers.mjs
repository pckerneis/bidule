// Loads the DOM-free core of the console into a fresh context for tests.
import vm from "node:vm";
import { assembleScript } from "../scripts/build.mjs";

const EXPORTS = ["makeVM", "makeWasmVM", "sectionImages", "makeSampleCart", "SAMPLE_IDS", "cloneSection", "emptySections", "fb", "BErr", "Fx", "W", "H"];

/** The console core. Tests of the shipped console use `core.makeWasmVM`; `makeVM` is the reference interpreter. */
export function loadCore({ wasm = true } = {}) {
  const ctx = vm.createContext({ console, performance, btoa, atob, Math, Date, WebAssembly });
  vm.runInContext(assembleScript({ withHost: false, wasm }) + `\n;globalThis.__core = { ${EXPORTS.join(", ")} };`, ctx);
  return ctx.__core;
}

/**
 * A minimal host around makeVM: a shelf of carts, a running cart and a pointer.
 * System dialogs are answered from `answers` in order (a cart id for pickcart,
 * "over" / "copy" / null for savecart).
 */
export function makeHarness(core, runningId, extraCarts = [], engine = core.makeWasmVM) {
  const carts = {};
  for (const id of core.SAMPLE_IDS) { const c = core.makeSampleCart(id); carts[c.id] = c; }
  for (const c of extraCarts) carts[c.id] = c;
  const pointer = { x: 0, y: 0, b: 0 };
  let running = carts[runningId];
  const status = { text: "" };
  const host = {
    debug: true, pointer,
    sections: () => running.sections,
    cartSections: id => carts[id].sections,
    cartTitle: id => (/^\/\/\s*@title\s+(.+)$/m.exec(carts[id].source) || [, "Untitled"])[1].trim(),
    setStatusLine: t => { status.text = t; },
    audioFault: e => { throw e; },
  };
  const vmi = engine(running.source, host);
  let frameNo = 0;
  const asked = [];
  function drain(task, answers) {
    let r = task.next();
    while (!r.done) {
      if (r.value.type === "late") { r = task.next(); continue; }
      asked.push(r.value.type);
      const a = answers.shift();
      if (r.value.type === "save") {
        if (a) {
          const c = carts[r.value.handle.pick.cartId], seen = new Set();
          for (const Hd of r.value.handles) {
            if (seen.has(Hd.secName)) continue;
            seen.add(Hd.secName);
            const rev = (c.sections[Hd.secName]?.rev ?? 0) + 1;
            c.sections[Hd.secName] = core.cloneSection(Hd.secName, Hd.sec);
            c.sections[Hd.secName].rev = rev;
          }
        }
        r = task.next(a ? 1 : 0);
      } else r = task.next(a ?? null);
    }
  }
  return {
    carts, pointer, vm: vmi, asked, status,
    init(answers = []) { drain(vmi.initTask(), answers); },
    frame(input = 0, answers = []) { drain(vmi.frameTask(frameNo++, input), answers); },
  };
}

export const BTN = { L: 1, R: 2, U: 4, D: 8, A: 16, B: 32 };

/** Runs a cart from source alone (no assets) and returns the console core and VM. */
export function runSource(core, source, { frames = 1, debug = true, engine = core.makeWasmVM } = {}) {
  const sections = core.emptySections();
  const v = engine(source, { debug, pointer: { x: 0, y: 0, b: 0 }, sections: () => sections, cartSections: () => sections, cartTitle: () => "Test", setStatusLine: () => {}, audioFault: e => { throw e; } });
  const drain = t => { const r = t.next(); if (!r.done) throw new Error("unexpected system dialog"); };
  drain(v.initTask());
  for (let f = 0; f < frames; f++) drain(v.frameTask(f, 0));
  return v;
}
