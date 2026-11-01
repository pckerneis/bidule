// Loads the DOM-free core of the console into a fresh context for tests.
import vm from "node:vm";
import { assembleScript } from "../scripts/build.mjs";

const EXPORTS = ["makeVM", "makeSampleCart", "SAMPLE_IDS", "cloneSection", "emptySections", "fb", "BErr", "Fx", "W", "H"];

export function loadCore() {
  const ctx = vm.createContext({ console, performance, btoa, atob, Math, Date });
  vm.runInContext(assembleScript({ withHost: false }) + `\n;globalThis.__core = { ${EXPORTS.join(", ")} };`, ctx);
  return ctx.__core;
}

/**
 * A minimal host around makeVM: a shelf of carts, a running cart and a pointer.
 * System dialogs are answered from `answers` in order (a cart id for pickcart,
 * "over" / "copy" / null for savecart).
 */
export function makeHarness(core, runningId, extraCarts = []) {
  const carts = {};
  for (const id of core.SAMPLE_IDS) { const c = core.makeSampleCart(id); carts[c.id] = c; }
  for (const c of extraCarts) carts[c.id] = c;
  const pointer = { x: 0, y: 0, b: 0 };
  let running = carts[runningId];
  const host = {
    debug: true, pointer,
    sections: () => running.sections,
    cartSections: id => carts[id].sections,
    audioFault: e => { throw e; },
  };
  const vmi = core.makeVM(running.source, host);
  let frameNo = 0;
  const asked = [];
  function drain(task, answers) {
    let r = task.next();
    while (!r.done) {
      asked.push(r.value.type);
      const a = answers.shift();
      if (r.value.type === "save") {
        if (a) {
          const Hd = r.value.handle, c = carts[Hd.cartId];
          c.sections[Hd.secName] = core.cloneSection(Hd.secName, Hd.sec);
          c.sections[Hd.secName].rev++;
        }
        r = task.next(a ? 1 : 0);
      } else r = task.next(a ?? null);
    }
  }
  return {
    carts, pointer, vm: vmi, asked,
    init(answers = []) { drain(vmi.initTask(), answers); },
    frame(input = 0, answers = []) { drain(vmi.frameTask(frameNo++, input), answers); },
  };
}

export const BTN = { L: 1, R: 2, U: 4, D: 8, A: 16, B: 32 };

/** Runs a cart from source alone (no assets) and returns the console core and VM. */
export function runSource(core, source, { frames = 1, debug = true } = {}) {
  const sections = core.emptySections();
  const v = core.makeVM(source, { debug, pointer: { x: 0, y: 0, b: 0 }, sections: () => sections, cartSections: () => sections, audioFault: e => { throw e; } });
  const drain = t => { const r = t.next(); if (!r.done) throw new Error("unexpected system dialog"); };
  drain(v.initTask());
  for (let f = 0; f < frames; f++) drain(v.frameTask(f, 0));
  return v;
}
