// Assembles the emulator into a single self-contained page.
//
//   node scripts/build.mjs        → dist/index.html     (standalone page, open it in a browser)
//                                    dist/artifact.html  (body fragment for hosts that add their own <html>/<head>)
//
// The page has no runtime dependencies: sources are concatenated in a fixed order
// inside one IIFE, and the sample carts in carts/*.bdcart are inlined as strings.

import { readFileSync, writeFileSync, mkdirSync, readdirSync } from "node:fs";
import { dirname, join, basename } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const read = p => readFileSync(join(root, p), "utf8");

// Order matters: later modules use names declared by earlier ones.
export const CORE_MODULES = [
  "carts.js", "samples.js", "palette.js", "font.js", "framebuffer.js",
  "audio.js", "lexer.js", "parser.js", "values.js", "interpreter.js",
];
export const HOST_MODULES = ["host.js"];

// carts/<name>.bdcart → SAMPLE_SOURCES key used by samples.js
const CART_KEYS = {
  "spark-catcher": "spark", "blink": "blink", "tile-peek": "peek",
  "sprite-editor": "sprites", "new-cart": "new-cart",
};

export function sampleSources() {
  const out = {};
  for (const f of readdirSync(join(root, "carts")).filter(f => f.endsWith(".bdcart"))) {
    const key = CART_KEYS[basename(f, ".bdcart")];
    if (key) out[key] = read(join("carts", f));
  }
  return out;
}

/** JavaScript for the console. `withHost: false` gives the DOM-free core used by tests. */
export function assembleScript({ withHost = true } = {}) {
  const mods = withHost ? [...CORE_MODULES, ...HOST_MODULES] : CORE_MODULES;
  const body = [
    '"use strict";',
    "const W = 240, H = 160;",
    `const SAMPLE_SOURCES = ${JSON.stringify(sampleSources(), null, 2)};`,
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
