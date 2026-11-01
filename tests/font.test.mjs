import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { loadCore, runSource } from "./helpers.mjs";

const core = loadCore();
const bitmap = JSON.parse(readFileSync(new URL("../fonts/monogram/monogram-bitmap.json", import.meta.url), "utf8"));

test("print draws Monogram glyphs pixel for pixel in a 6×9 cell", () => {
  const text = "Ag;9";
  runSource(core, `draw() {\n  cls(0)\n  print("${text}", 0, 0, 7)\n}`);
  [...text].forEach((ch, k) => {
    const rows = bitmap[ch].slice(3);
    for (let r = 0; r < 9; r++) for (let b = 0; b < 6; b++) {
      const want = b < 5 && (rows[r] >> b) & 1 ? 7 : 0;
      assert.equal(core.fb[r * core.W + k * 6 + b], want, `'${ch}' row ${r} column ${b}`);
    }
  });
});
