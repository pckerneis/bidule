// Assembles the emulator into a single self-contained page.
//
//   node scripts/build.mjs        → dist/index.html     (standalone page, open it in a browser)
//                                    dist/artifact.html  (body fragment for hosts that add their own <html>/<head>)
//
// The page has no runtime dependencies: sources are concatenated in a fixed order
// inside one IIFE, the sample carts in carts/*.bdcart are inlined as strings, and
// the console itself (crates/bidule-web, built here with cargo) as base64 WebAssembly.

import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync, mkdirSync, readdirSync } from "node:fs";
import { dirname, join, basename } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const read = p => readFileSync(join(root, p), "utf8");

// Order matters: later modules use names declared by earlier ones.
export const CORE_MODULES = [
  "carts.js", "samples.js", "palette.js", "font.js", "framebuffer.js",
  "audio.js", "editing.js", "wasm.js",
];
// The source interpreter is no longer part of the page: it stays as the reference
// the conformance tests compare the Rust console against.
export const REFERENCE_MODULES = ["lexer.js", "parser.js", "values.js", "interpreter.js"];
export const HOST_MODULES = ["host.js", "sysmenu.js"];

// carts/<name>.bdcart → SAMPLE_SOURCES key used by samples.js
const CART_KEYS = {
  "spark-catcher": "spark", "blink": "blink", "tile-peek": "peek",
  "sprite-editor": "sprites", "new-cart": "new-cart",
};

// Monogram glyphs for ASCII 32–126. The source bitmap is 12 rows per glyph:
// rows 0–2 hold accents above capitals, which ASCII never uses, so the console
// keeps rows 3–11 — a 9-row cell, as the spec's 6×9 font requires.
export function monogramRows() {
  const bitmap = JSON.parse(read("fonts/monogram/monogram-bitmap.json"));
  const rows = [];
  for (let c = 32; c <= 126; c++) {
    const g = bitmap[String.fromCharCode(c)];
    if (!g || g.length !== 12) throw new Error(`Monogram bitmap has no 12-row glyph for ${JSON.stringify(String.fromCharCode(c))}`);
    if (g.slice(0, 3).some(Boolean)) throw new Error(`glyph ${c} uses the accent rows`);
    rows.push(...g.slice(3));
  }
  return rows;
}

export function sampleSources() {
  const out = {};
  for (const f of readdirSync(join(root, "carts")).filter(f => f.endsWith(".bdcart"))) {
    const key = CART_KEYS[basename(f, ".bdcart")];
    if (key) out[key] = read(join("carts", f));
  }
  return out;
}

let wasmBase64 = null;
/** Builds crates/bidule-web for the browser; cargo makes this a no-op when nothing changed. */
export function buildWasm() {
  if (wasmBase64) return wasmBase64;
  try {
    execFileSync("cargo", ["build", "--quiet", "--profile", "web", "-p", "bidule-web", "--target", "wasm32-unknown-unknown"], { cwd: root, stdio: "inherit" });
  } catch (e) {
    throw new Error("Building the console needs Rust with the WebAssembly target: rustup target add wasm32-unknown-unknown");
  }
  const target = process.env.CARGO_TARGET_DIR || join(root, "target");
  wasmBase64 = readFileSync(join(target, "wasm32-unknown-unknown/web/bidule_web.wasm")).toString("base64");
  return wasmBase64;
}

/**
 * JavaScript for the console. `withHost: false` gives the DOM-free core used by tests,
 * with the reference interpreter; `wasm: false` leaves the WebAssembly out, for tools
 * that only need the reference.
 */
export function assembleScript({ withHost = true, wasm = true } = {}) {
  const mods = withHost ? [...CORE_MODULES, ...HOST_MODULES] : [...CORE_MODULES, ...REFERENCE_MODULES];
  const body = [
    '"use strict";',
    "const W = 240, H = 160;",
    `const BIDULE_WASM = "${wasm ? buildWasm() : ""}";`,
    `const SAMPLE_SOURCES = ${JSON.stringify(sampleSources(), null, 2)};`,
    `const MONOGRAM_ROWS = [${monogramRows().join(",")}];`,
    ...mods.map(m => `\n/* ---- src/js/${m} ---- */\n` + read(join("src/js", m))),
  ].join("\n");
  return withHost ? `(() => {\n${body}\n})();\n` : body;
}

export function assembleFragment() {
  return read("src/shell.html")
    .replace("/* @styles */", () => read("src/styles.css"))
    .replace("/* @script */", () => assembleScript());
}

export function assemblePage() {
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
<style>:root{padding-top:env(safe-area-inset-top,0px);padding-bottom:env(safe-area-inset-bottom,0px)}body{margin:0;font:14px system-ui,sans-serif}[hidden]{display:none!important}</style>
</head>
<body>
${assembleFragment()}
</body>
</html>
`;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  mkdirSync(join(root, "dist"), { recursive: true });
  writeFileSync(join(root, "dist/index.html"), assemblePage());
  writeFileSync(join(root, "dist/artifact.html"), assembleFragment());
  console.log("Built dist/index.html and dist/artifact.html");
}
