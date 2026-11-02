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

const CRC_TABLE = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xEDB88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
function crc32(bytes) {
  let c = 0xFFFFFFFF;
  for (const b of bytes) c = CRC_TABLE[(c ^ b) & 255] ^ (c >>> 8);
  return (c ^ 0xFFFFFFFF) >>> 0;
}

/** Spec › Asset section layout › Common container. */
function container(kind, rev, fixed, slots) {
  const payload = [...fixed];
  const table = [];
  for (const s of slots) {
    table.push(s.bytes.length ? payload.length : 0, s.bytes.length);
    payload.push(...s.bytes);
  }
  const names = slots.flatMap(s => [s.name.length, ...[...s.name].map(c => c.charCodeAt(0))]);
  const out = new Uint8Array(16 + names.length + table.length * 4 + payload.length);
  const dv = new DataView(out.buffer);
  out[0] = kind; out[1] = 1; dv.setUint16(2, rev, true); dv.setUint16(4, slots.length, true);
  dv.setUint16(6, 0, true); dv.setUint32(8, crc32(payload), true); dv.setUint32(12, 0, true);
  let p = 16;
  out.set(names, p); p += names.length;
  for (const v of table) { dv.setUint32(p, v, true); p += 4; }
  out.set(payload, p);
  return out;
}

const u16le = v => [v & 255, (v >> 8) & 255];

function exportAssets(core, cartId, dir) {
  const cart = core.makeSampleCart(cartId);
  const s = cart.sections;
  mkdirSync(dir, { recursive: true });
  const spr = s.sprites;
  writeFileSync(join(dir, "sprites.bin"), container(0x10, spr.rev, [...spr.sheet, ...spr.flags],
    spr.slots.map(x => ({ name: x.name || "", bytes: x.empty ? [] : [...u16le(x.tile), x.w, x.h] }))));
  writeFileSync(join(dir, "maps.bin"), container(0x11, s.maps.rev, [],
    s.maps.slots.map(m => ({ name: m.name || "", bytes: m.empty ? [] : [...u16le(m.w), ...u16le(m.h), ...[...m.cells].flatMap(u16le)] }))));
  writeFileSync(join(dir, "sounds.bin"), container(0x12, s.sounds.rev, [],
    s.sounds.slots.map(x => ({ name: x.name || "", bytes: x.empty ? [] : [...x.bytes] }))));
  if (s.palette) writeFileSync(join(dir, "palette.bin"), container(0x14, s.palette.rev, [...s.palette.words].flatMap(u16le), []));
}

function dumpFrames(core, cartId, count, out, inputs) {
  const h = makeHarness(core, `sample-${cartId}`);
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
const core = loadCore();
if (cmd === "assets") exportAssets(core, cartId, rest[0]);
else if (cmd === "frames") dumpFrames(core, cartId, +rest[0], rest[1], (rest[2] || "").split(",").filter(Boolean).map(Number));
else {
  console.error("usage: conformance.mjs assets <cart> <outdir> | frames <cart> <count> <outfile> [inputs]");
  process.exit(2);
}
