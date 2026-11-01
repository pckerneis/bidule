/* =====================================================================
   Values
   ===================================================================== */
class Fx { constructor(r) { this.r = r | 0; } }
class TextV { constructor(s) { this.s = s; } }
class Arr { constructor(et, dims, data, name) { this.et = et; this.dims = dims; this.data = data; this.ro = false; this.name = name; } }
class Row { constructor(arr, base, n) { this.arr = arr; this.base = base; this.n = n; } }
class Rec { constructor(def) { this.def = def; this.f = {}; this.ro = false; } }
const isFx = v => v instanceof Fx;
const isNum = v => typeof v === "number" || v instanceof Fx;
const toFx = v => v instanceof Fx ? v : new Fx(v << 16);
const FX0 = new Fx(0);

function fmul(x, y) { const ah = x >> 16, al = x & 0xFFFF; return (ah * y + Math.floor(al * y / 65536)) | 0; }
const SIN = Array.from({ length: 91 }, (_, d) => Math.round(Math.sin(d * Math.PI / 180) * 65536));
const TURN = 360 * 65536;
function sinRaw(a) {
  a %= TURN; if (a < 0) a += TURN;
  let sign = 1;
  if (a >= 180 * 65536) { a -= 180 * 65536; sign = -1; }
  if (a > 90 * 65536) a = 180 * 65536 - a;
  const d = a >> 16, f = a & 0xFFFF;
  const v = d >= 90 ? SIN[90] : SIN[d] + Math.floor((SIN[d + 1] - SIN[d]) * f / 65536);
  return sign * v;
}
const angleRaw = v => v instanceof Fx ? v.r : ((v % 360) << 16);

function fmtNum(v, f) {
  let s;
  if (f && f.kind === "c") return String.fromCharCode((v instanceof Fx ? v.r >> 16 : v) & 255);
  if (f && f.kind === "x") s = ((v instanceof Fx ? v.r : v) >>> 0).toString(16);
  else if (v instanceof Fx) {
    const x = v.r / 65536;
    s = f && f.dec != null ? x.toFixed(f.dec) : x.toFixed(4).replace(/0+$/, "").replace(/\.$/, "");
    if (s === "-0") s = "0";
  } else s = f && f.dec != null ? v.toFixed(f.dec) : String(v);
  if (f && f.width && s.length < f.width) {
    if (f.zero) s = s[0] === "-" ? "-" + s.slice(1).padStart(f.width - 1, "0") : s.padStart(f.width, "0");
    else s = s.padStart(f.width, " ");
  }
  return s;
}
function arrText(a) {
  const d = a instanceof Row ? a.arr.data : a.data, base = a instanceof Row ? a.base : 0, n = a instanceof Row ? a.n : a.data.length;
  let s = "";
  for (let i = 0; i < n; i++) { let c = d[base + i]; if (c instanceof Fx) c = c.r >> 16; if (!c) break; s += String.fromCharCode(c & 255); }
  return s;
}
function levenshtein(a, b) {
  const m = Array.from({ length: a.length + 1 }, (_, i) => [i]);
  for (let j = 1; j <= b.length; j++) m[0][j] = j;
  for (let i = 1; i <= a.length; i++) for (let j = 1; j <= b.length; j++)
    m[i][j] = Math.min(m[i - 1][j] + 1, m[i][j - 1] + 1, m[i - 1][j - 1] + (a[i - 1] === b[j - 1] ? 0 : 1));
  return m[a.length][b.length];
}
