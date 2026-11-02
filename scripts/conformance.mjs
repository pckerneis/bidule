// Bridges the JS emulator and the Rust toolchain for conformance checks.
//
//   node scripts/conformance.mjs assets <cart> <outdir>
//       Writes the sample cart's asset sections in the spec's common container
//       layout: sprites.bin, maps.bin, sounds.bin, palette.bin (when present).
//
//   node scripts/conformance.mjs frames <cart> <count> <outfile> [inputs]
//       Runs the cart in the JS emulator for <count> frames and writes each
//       framebuffer (240×160 bytes) one after another. [inputs] is an optional
//       comma-separated list of button bitmasks, one per frame (default 0).
//
// <cart> is a sample cart: blink, spark-catcher, tile-peek, sprite-editor.

import { writeFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { loadCore, makeHarness } from "../tests/helpers.mjs";

const IDS = { "blink": "blink", "spark-catcher": "spark", "tile-peek": "peek", "sprite-editor": "sprites" };

function exportAssets(core, cartId, dir) {
  const img = core.sectionImages(core.makeSampleCart(cartId).sections);
  mkdirSync(dir, { recursive: true });
  for (const name of ["sprites", "maps", "sounds", "palette"]) if (img[name]) writeFileSync(join(dir, `${name}.bin`), img[name]);
}

function dumpFrames(core, cartId, count, out, inputs) {
  const h = makeHarness(core, `sample-${cartId}`, [], core.makeVM);
  h.init();
  const frames = new Uint8Array(count * core.W * core.H);
  for (let f = 0; f < count; f++) {
    h.frame(inputs[f] ?? 0);
    frames.set(core.fb, f * core.W * core.H);
  }
  writeFileSync(out, frames);
}

// The Rust VM's rnd(): xorshift32 seeded with 0x12345678 by the headless runner,
// then a multiply-shift into 0..n-1. JS rnd(n) computes floor(Math.random() * n),
// which gives the same integer when Math.random() returns x / 2^32.
let rng = 0x12345678;
Math.random = () => {
  rng ^= rng << 13; rng >>>= 0;
  rng ^= rng >>> 17;
  rng ^= rng << 5; rng >>>= 0;
  return rng / 4294967296;
};

const [cmd, name, ...rest] = process.argv.slice(2);
const cartId = IDS[name];
if (!cartId) {
  console.error(`unknown cart '${name}'; use one of ${Object.keys(IDS).join(", ")}`);
  process.exit(2);
}
const core = loadCore({ wasm: false });
if (cmd === "assets") exportAssets(core, cartId, rest[0]);
else if (cmd === "frames") dumpFrames(core, cartId, +rest[0], rest[1], (rest[2] || "").split(",").filter(Boolean).map(Number));
else {
  console.error("usage: conformance.mjs assets <cart> <outdir> | frames <cart> <count> <outfile> [inputs]");
  process.exit(2);
}
