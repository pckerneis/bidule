/* =====================================================================
   Framebuffer (8-bit indexes) and drawing primitives
   ===================================================================== */
const fb = new Uint8Array(W * H);
let transparent = 0;
function px(x, y, c) { if (x >= 0 && y >= 0 && x < W && y < H) fb[y * W + x] = c; }
function hspan(x0, x1, y, c) {
  if (y < 0 || y >= H) return;
  if (x0 > x1) { const t = x0; x0 = x1; x1 = t; }
  x0 = Math.max(0, x0); x1 = Math.min(W - 1, x1);
  if (x0 <= x1) fb.fill(c, y * W + x0, y * W + x1 + 1);
}
function fillRect(x, y, w, h, c) { for (let j = 0; j < h; j++) hspan(x, x + w - 1, y + j, c); }
function strokeRect(x, y, w, h, c) {
  if (w <= 0 || h <= 0) return;
  hspan(x, x + w - 1, y, c); hspan(x, x + w - 1, y + h - 1, c);
  for (let j = 1; j < h - 1; j++) { px(x, y + j, c); px(x + w - 1, y + j, c); }
}
function drawLine(x0, y0, x1, y1, c) {
  const dx = Math.abs(x1 - x0), dy = -Math.abs(y1 - y0), sx = x0 < x1 ? 1 : -1, sy = y0 < y1 ? 1 : -1;
  let err = dx + dy;
  for (let n = 0; n < 4096; n++) {
    px(x0, y0, c);
    if (x0 === x1 && y0 === y1) break;
    const e2 = 2 * err;
    if (e2 >= dy) { err += dy; x0 += sx; }
    if (e2 <= dx) { err += dx; y0 += sy; }
  }
}
function circle(cx, cy, r, c, fill) {
  if (r < 0) return;
  let x = r, y = 0, e = 1 - r;
  while (x >= y) {
    if (fill) {
      hspan(cx - x, cx + x, cy + y, c); hspan(cx - x, cx + x, cy - y, c);
      hspan(cx - y, cx + y, cy + x, c); hspan(cx - y, cx + y, cy - x, c);
    } else {
      for (const [a, b] of [[x,y],[y,x],[-y,x],[-x,y],[-x,-y],[-y,-x],[y,-x],[x,-y]]) px(cx + a, cy + b, c);
    }
    y++;
    if (e < 0) e += 2 * y + 1; else { x--; e += 2 * (y - x) + 1; }
  }
}
function drawText(s, x, y, c) {
  const x0 = x; let maxX = x;
  for (let k = 0; k < s.length; k++) {
    const ch = s.charCodeAt(k);
    if (ch === 10) { x = x0; y += CH; continue; }
    const g = (ch >= 32 && ch <= 126 ? ch : 63) - 32;
    for (let r = 0; r < CH; r++) {
      const bits = GLYPHS[g * 9 + r];
      if (!bits) continue;
      for (let b = 0; b < CW; b++) if (bits & (32 >> b)) px(x + b, y + r, c);
    }
    x += CW; if (x > maxX) maxX = x;
  }
  return x;
}
function textWidth(s) { let w = 0; for (const line of s.split("\n")) w = Math.max(w, line.length * CW); return w; }
