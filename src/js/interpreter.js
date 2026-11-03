/* =====================================================================
   Interpreter (resumable: system dialogs pause the cart mid-call)
   ===================================================================== */
const BRK = 1, CONT = 2, RET = 3;
const BUILTIN_CONSTS = new Map([["BTN_LEFT", 0], ["BTN_RIGHT", 1], ["BTN_UP", 2], ["BTN_DOWN", 3], ["BTN_A", 4], ["BTN_B", 5],
  ["SCREEN_W", W], ["SCREEN_H", H], ["SQUARE", 0], ["TRIANGLE", 1], ["SAW", 2], ["NOISE", 3]]);
PAL_NAMES.forEach((n, i) => BUILTIN_CONSTS.set(n, i));
const ASSET_PREFIXES = new Set(["Sprites", "Maps", "Sounds", "Music"]);
function makeVM(src, host) {
  const { toks, meta } = lex(src);
  const P = new Parser(toks).program();
  const debug = host.debug;
  let frame = null, depth = 0, steps = 0, budget = 5_000_000, audioMode = false;
  let input = 0, prevInput = 0;
  const cues = new Int32Array(8);

  const globals = new Map(), consts = new Map(), avars = new Map(), records = new Map(), enums = new Map(), fns = new Map(), sprites = new Map();
  const err = (line, msg) => { throw new BErr(msg, line); };
  const runSync = g => { const r = g.next(); if (!r.done) err(0, "system dialogs can't open while a cart is loading"); return r.value; };
  const evS = n => runSync(ev(n));

  // ---- asset names, bound at load (spec: compile-time constants) ----
  const live = () => host.sections();
  const assetNames = { Sprites: new Map(), Maps: new Map(), Sounds: new Map() };

  // ---- built-ins ----
  function ti(v, line) { if (typeof v === "number") return v; if (v instanceof Fx) return v.r >> 16; err(line, "expected a number"); }
  function ci(v, line) { return ti(v, line) & 255; }
  function sameType(args, line, op) {
    if (args.some(a => !isNum(a))) err(line, "expected numbers");
    if (args.some(isFx)) { const r = op(args.map(a => toFx(a).r)); return new Fx(r); }
    return op(args);
  }
  function soundOf(v, line) {
    if (!(v instanceof Rec) || v.def !== SOUND) err(line, "play needs a Sound: Sounds.NAME or a const Sound record");
    if (v.desc) return v.desc;
    const f = v.f, num = x => x instanceof Fx ? x.r / 65536 : x;
    const notes = arrText(f.notes).trim();
    let seq = null;
    if (notes) seq = notes.split(/\s+/).map(tok => { const fq = noteFreq(tok); if (fq === undefined) err(line, `unknown note '${tok}' (use names like C4, F#5, Bb3 or - for a rest)`); return fq; });
    return { wave: f.wave & 3, from: num(f.from), to: num(f.to), time: Math.max(0, num(f.time)), vol: num(f.vol), duty: num(f.duty), attack: Math.max(0, num(f.attack)), release: Math.max(0, num(f.release)), step: Math.max(0.01, num(f.step)), seq };
  }
  const voiceIdx = (v, line) => { const i = ti(v, line); if (i < 0 || i > 3) err(line, `voice must be 0 to 3, got ${i}`); return i; };
  function drawTile(sheet, t, x, y) {
    if (t < 0 || t >= TILE_COUNT || x <= -8 || y <= -8 || x >= W || y >= H) return;
    const base = t * 64;
    for (let j = 0; j < 8; j++) {
      const yy = y + j; if (yy < 0 || yy >= H) continue;
      for (let i = 0; i < 8; i++) {
        const c = sheet[base + j * 8 + i];
        if (c !== transparent) { const xx = x + i; if (xx >= 0 && xx < W) fb[yy * W + xx] = c; }
      }
    }
  }
  function textArg(v, line) { if (v instanceof TextV) return v.s; if (v instanceof Arr || v instanceof Row) return arrText(v); err(line, "expected text"); }
  const { handles, slotCount, slotBytes, slotOf, writeSlot, pickFinish, openCart, requestWrite, openSection, nameOf, setName, drawInfo, saveGroup } = makeEditing(host, err);
  function hget(v, line) { return slotOf(ti(v, line), line); }
  const B = (arity, fn, audioOnly) => ({ arity, fn, audioOnly });
  const BUILTINS_IMPL = {
    cls: B([0, 1], (a, l) => { fb.fill(a.length ? ci(a[0], l) : 0); return 0; }),
    pset: B([3, 3], (a, l) => { px(ti(a[0], l), ti(a[1], l), ci(a[2], l)); return 0; }),
    pget: B([2, 2], (a, l) => { const x = ti(a[0], l), y = ti(a[1], l); return x >= 0 && y >= 0 && x < W && y < H ? fb[y * W + x] : 0; }),
    line: B([5, 5], (a, l) => { drawLine(ti(a[0], l), ti(a[1], l), ti(a[2], l), ti(a[3], l), ci(a[4], l)); return 0; }),
    rect: B([5, 5], (a, l) => { strokeRect(ti(a[0], l), ti(a[1], l), ti(a[2], l), ti(a[3], l), ci(a[4], l)); return 0; }),
    rectfill: B([5, 5], (a, l) => { fillRect(ti(a[0], l), ti(a[1], l), ti(a[2], l), ti(a[3], l), ci(a[4], l)); return 0; }),
    circ: B([4, 4], (a, l) => { circle(ti(a[0], l), ti(a[1], l), ti(a[2], l), ci(a[3], l), false); return 0; }),
    circfill: B([4, 4], (a, l) => { circle(ti(a[0], l), ti(a[1], l), ti(a[2], l), ci(a[3], l), true); return 0; }),
    spr: B([3, 3], (a, l) => {
      const s = a[0], spr = live().sprites;
      let tile, w, h;
      if (s && s.sprite) ({ tile, w, h } = s);
      else if (typeof s === "number") {
        const sl = spr.slots[s];
        if (!sl) err(l, `sprite ${s} doesn't exist; use Sprites.NAME`);
        if (sl.empty) return 0;
        ({ tile, w, h } = sl);
      } else err(l, "spr needs a sprite: Sprites.NAME or a sprite declaration");
      const x = ti(a[1], l), y = ti(a[2], l);
      for (let ty = 0; ty < h; ty++) for (let tx = 0; tx < w; tx++) drawTile(spr.sheet, tile + ty * SHEET_COLS + tx, x + tx * 8, y + ty * 8);
      return 0;
    }),
    map: B([7, 7], (a, l) => {
      const m = a[0];
      if (!(m instanceof Arr) || m.mapSlot == null) err(l, "map needs a map: Maps.NAME");
      const [cx, cy, x, y, w, h] = a.slice(1).map(v => ti(v, l));
      const sheet = live().sprites.sheet, mw = m.dims[1], mh = m.dims[0];
      for (let j = 0; j < h; j++) {
        const r = cy + j; if (r < 0 || r >= mh) continue;
        for (let i = 0; i < w; i++) {
          const c = cx + i; if (c < 0 || c >= mw) continue;
          const t = m.data[r * mw + c];
          if (t) drawTile(sheet, t, x + i * 8, y + j * 8);
        }
      }
      return 0;
    }),
    fget: B([2, 2], (a, l) => { const t = ti(a[0], l), f = ti(a[1], l); if (t < 0 || t >= TILE_COUNT) return 0; return (live().sprites.flags[t] >> (f & 7)) & 1; }),
    print: B([3, 4], (a, l) => drawText(textOf(a[0], l), ti(a[1], l), ti(a[2], l), a.length > 3 ? ci(a[3], l) : 7)),
    textwidth: B([1, 1], (a, l) => textWidth(textOf(a[0], l))),
    fmt: B([2, 2], (a, l) => {
      const d = a[0]; if (!(d instanceof Arr || d instanceof Row)) err(l, "fmt writes into an array: fmt(dst, text)");
      const arr = d instanceof Row ? d.arr : d, base = d instanceof Row ? d.base : 0, n = d instanceof Row ? d.n : d.data.length;
      if (arr.ro) err(l, `'${arr.name}' is read-only`);
      const s = textOf(a[1], l).slice(0, Math.max(0, n - 1));
      for (let i = 0; i < s.length; i++) arr.data[base + i] = s.charCodeAt(i);
      if (n > 0) arr.data[base + s.length] = 0;
      return s.length;
    }),
    btn: B([1, 1], (a, l) => (input >> (ti(a[0], l) & 31)) & 1),
    btnp: B([1, 1], (a, l) => { const b = ti(a[0], l) & 31; return ((input >> b) & 1) && !((prevInput >> b) & 1) ? 1 : 0; }),
    haspointer: B([0, 0], () => host.pointer ? 1 : 0),
    mousex: B([0, 0], () => host.pointer ? host.pointer.x : 0),
    mousey: B([0, 0], () => host.pointer ? host.pointer.y : 0),
    mouseb: B([0, 0], () => host.pointer ? host.pointer.b : 0),
    setpal: B([4, 4], (a, l) => {
      const i = ti(a[0], l); if (i < 0 || i > 255) err(l, `palette index must be 0 to 255, got ${i}`);
      const c = v => Math.max(0, Math.min(255, ti(v, l)));
      pal565[i] = (Math.round(c(a[1]) * 31 / 255) << 11) | (Math.round(c(a[2]) * 63 / 255) << 5) | Math.round(c(a[3]) * 31 / 255);
      return 0;
    }),
    getpal: B([2, 2], (a, l) => { const i = ti(a[0], l) & 255, ch = ti(a[1], l); if (ch < 0 || ch > 2) err(l, "channel must be 0 (red), 1 (green) or 2 (blue)"); return rgbOf(pal565[i])[ch]; }),
    palt: B([1, 1], (a, l) => { transparent = ci(a[0], l); return 0; }),
    sin: B([1, 1], (a, l) => { if (!isNum(a[0])) err(l, "sin needs a number"); return new Fx(sinRaw(angleRaw(a[0]))); }),
    cos: B([1, 1], (a, l) => { if (!isNum(a[0])) err(l, "cos needs a number"); return new Fx(sinRaw(((angleRaw(a[0]) % TURN) + 90 * 65536))); }),
    atan2: B([2, 2], (a, l) => {
      const y = toFx(a[0]).r, x = toFx(a[1]).r;
      let d = Math.atan2(y, x) * 180 / Math.PI; if (d < 0) d += 360;
      let r = Math.round(d * 65536); if (r >= TURN) r = 0; return new Fx(r);
    }),
    sqrt: B([1, 1], (a) => { const r = toFx(a[0]).r; return new Fx(r <= 0 ? 0 : Math.floor(Math.sqrt(r * 65536))); }),
    int: B([1, 1], (a, l) => { if (!isNum(a[0])) err(l, "int needs a number"); return a[0] instanceof Fx ? a[0].r >> 16 : a[0]; }),
    round: B([1, 1], (a, l) => { if (!isNum(a[0])) err(l, "round needs a number"); return a[0] instanceof Fx ? (a[0].r + 32768) >> 16 : a[0]; }),
    frac: B([1, 1], (a) => new Fx(toFx(a[0]).r & 0xFFFF)),
    abs: B([1, 1], (a, l) => sameType(a, l, ([x]) => x < 0 ? (-x) | 0 : x)),
    min: B([2, 2], (a, l) => sameType(a, l, ([x, y]) => x < y ? x : y)),
    max: B([2, 2], (a, l) => sameType(a, l, ([x, y]) => x > y ? x : y)),
    clamp: B([3, 3], (a, l) => sameType(a, l, ([v, lo, hi]) => v < lo ? lo : v > hi ? hi : v)),
    rnd: B([1, 1], (a, l) => { const n = ti(a[0], l); return n > 0 ? Math.floor(Math.random() * n) : 0; }),
    copy: B([2, 2], (a, l) => {
      const [d, s] = a;
      if (!(d instanceof Arr || d instanceof Row) || !(s instanceof Arr || s instanceof Row || s instanceof TextV)) err(l, "copy(dst, src) needs two arrays");
      const da = d instanceof Row ? d.arr : d, db = d instanceof Row ? d.base : 0, dn = d instanceof Row ? d.n : d.data.length;
      if (da.ro) err(l, `'${da.name}' is read-only`);
      const srcv = s instanceof TextV ? textCodes(s.s).concat(0) : s instanceof Row ? s.arr.data.slice(s.base, s.base + s.n) : s.data;
      const n = Math.min(dn, srcv.length);
      for (let i = 0; i < n; i++) { const v = srcv[i]; if (v instanceof Rec) copyRec(da.data[db + i], v); else da.data[db + i] = coerce(da.et, v, l, "element"); }
      return n;
    }),
    streq: B([2, 2], (a, l) => textOf(a[0], l) === textOf(a[1], l) ? 1 : 0),
    arreq: B([3, 3], (a, l) => {
      const n = ti(a[2], l);
      for (let i = 0; i < n; i++) if (!arith("==", indexGet(a[0], i, l), indexGet(a[1], i, l), l)) return 0;
      return 1;
    }),
    play: B([1, 2], (a, l) => {
      const snd = soundOf(a[0], l); const t = nowMs();
      let v;
      if (a.length > 1) v = voiceIdx(a[1], l);
      else { v = voices.findIndex(x => x.end <= t); if (v < 0) { v = 0; for (let k = 1; k < 4; k++) if (voices[k].start < voices[v].start) v = k; } }
      stopVoice(v); startSound(snd, v); return v;
    }),
    stop: B([1, 1], (a, l) => { stopVoice(voiceIdx(a[0], l)); return 0; }),
    stopall: B([0, 0], () => { stopAllVoices(); return 0; }),
    playing: B([1, 1], (a, l) => voices[voiceIdx(a[0], l)].end > nowMs() ? 1 : 0),
    cue: B([1, 1], (a, l) => { const n = ti(a[0], l); if (n < 0 || n > 7) err(l, `cue must be 0 to 7, got ${n}`); cues[n]++; return 0; }),
    cuecount: B([1, 1], (a, l) => { const n = ti(a[0], l); if (n < 0 || n > 7) err(l, `cue must be 0 to 7, got ${n}`); return cues[n]; }, true),

    // ---- editing API (spec: Storage, USB and cartridge format › Editing API) ----
    pickcart: B([1, 1], (a, l) => pickFinish(textArg(a[0], l), l)),
    // Proposed (spec › Editor support): another asset section of an already-picked cart
    opensection: B([2, 2], (a, l) => openSection(ti(a[0], l), textArg(a[1], l), l)),
    // Proposed (spec › Editing API): other carts, read-only until the user allows writing
    cartcount: B([0, 0], () => host.cartIds().length),
    cartinfo: B([2, 2], (a, l) => {
      const id = host.cartIds()[ti(a[0], l)];
      if (id == null) err(l, `cart ${ti(a[0], l)} doesn't exist; cartcount() says how many there are`);
      return BUILTINS_IMPL.fmt.fn([a[1], new TextV(host.cartTitle(id))], l);
    }),
    opencart: B([2, 2], (a, l) => openCart(ti(a[0], l), textArg(a[1], l), l)),
    requestwrite: B([1, 1], (a, l) => requestWrite(hget(a[0], l))),
    cartname: B([2, 2], (a, l) => {
      return BUILTINS_IMPL.fmt.fn([a[1], new TextV(host.cartTitle(hget(a[0], l).pick.cartId))], l);
    }),
    drawasset: B([4, 4], (a, l) => {
      const H_ = hget(a[0], l), x = ti(a[2], l), y = ti(a[3], l), d = drawInfo(H_, ti(a[1], l), l);
      for (let ty = 0; ty < d.h; ty++) for (let tx = 0; tx < d.w; tx++) drawTile(H_.sec.sheet, d.tile + ty * SHEET_COLS + tx, x + tx * 8, y + ty * 8);
      return 0;
    }),
    status: B([1, 1], (a, l) => { host.setStatusLine(textOf(a[0], l)); return 0; }),
    readasset: B([3, 3], (a, l) => {
      const H_ = hget(a[0], l), d = a[1];
      if (!(d instanceof Arr || d instanceof Row)) err(l, "readasset reads into an array: readasset(h, dst, i)");
      const arr = d instanceof Row ? d.arr : d, base = d instanceof Row ? d.base : 0, n = d instanceof Row ? d.n : d.data.length;
      if (arr.ro) err(l, `'${arr.name}' is read-only`);
      const b = slotBytes(H_, ti(a[2], l), l);
      const m = Math.min(n, b.length);
      for (let i = 0; i < m; i++) arr.data[base + i] = arr.et === "fix" ? new Fx(b[i] << 16) : b[i];
      return b.length;
    }),
    writeasset: B([3, 3], (a, l) => {
      const H_ = hget(a[0], l), s = a[1];
      if (!(s instanceof Arr || s instanceof Row)) err(l, "writeasset writes from an array: writeasset(h, src, i)");
      const vals = (s instanceof Row ? s.arr.data.slice(s.base, s.base + s.n) : s.data).map(v => (v instanceof Fx ? v.r >> 16 : v) & 255);
      return writeSlot(H_, vals, ti(a[2], l), l);
    }),
    assetcount: B([1, 1], (a, l) => slotCount(hget(a[0], l))),
    assetname: B([3, 3], (a, l) => {
      const H_ = hget(a[0], l), i = ti(a[1], l), d = a[2];
      if (!(d instanceof Arr || d instanceof Row)) err(l, "assetname writes into an array: assetname(h, i, dst)");
      return BUILTINS_IMPL.fmt.fn([d, new TextV(nameOf(H_, i, l))], l);
    }),
    setassetname: B([3, 3], (a, l) => setName(hget(a[0], l), ti(a[1], l), textArg(a[2], l), l)),
    savecart: B([1, 1], (a, l) => saveGroup(hget(a[0], l))),
  };
  const BI = BUILTINS_IMPL;

  // ---- registration ----
  const taken = new Map();
  const claim = (n, what, line) => {
    if (BUILTIN_CONSTS.has(n)) err(line, `'${n}' is a built-in name; pick another name`);
    if (BI[n]) err(line, `'${n}' is a built-in function; pick another name`);
    if (ASSET_PREFIXES.has(n)) err(line, `'${n}' is reserved for asset names; pick another name`);
    if (taken.has(n)) err(line, `'${n}' is already declared as a ${taken.get(n)}`);
    taken.set(n, what);
  };
  const SOUND = { n: "Sound", fields: [
    { n: "wave", ty: "int" }, { n: "from", ty: "fix" }, { n: "to", ty: "fix" }, { n: "time", ty: "fix" }, { n: "vol", ty: "fix" },
    { n: "duty", ty: "fix" }, { n: "attack", ty: "fix" }, { n: "release", ty: "fix" }, { n: "notes", ty: "int", dims: [64], dimsV: [64] }, { n: "step", ty: "fix" }],
    defaults: { vol: new Fx(65536), time: new Fx(6554), step: new Fx(6554), duty: new Fx(32768) } };
  SOUND.map = Object.fromEntries(SOUND.fields.map(f => [f.n, f]));
  records.set("Sound", SOUND); taken.set("Sound", "record type");

  for (const r of P.records) { claim(r.n, "record type", r.line); const def = { n: r.n, fields: r.fields, line: r.line }; def.map = {}; for (const f of r.fields) { if (def.map[f.n]) err(r.line, `field '${f.n}' appears twice in ${r.n}`); def.map[f.n] = f; } records.set(r.n, def); }
  for (const e of P.enums) claim(e.n, "enum", e.line);
  for (const c of P.consts) { claim(c.n, "constant", c.line); consts.set(c.n, { d: c, state: 0, v: null }); }
  for (const s of P.sprites) claim(s.n, "sprite", s.line);
  for (const f of P.fns) { claim(f.n, "function", f.line); fns.set(f.n, f); }
  for (const v of P.vars) claim(v.n, "variable", v.line);
  for (const a of P.avars) claim(a.n, "audio variable", a.line);

  for (const def of records.values()) {
    if (def === SOUND) continue;
    for (const f of def.fields) {
      if (f.ty !== "int" && f.ty !== "fix" && !records.has(f.ty)) err(def.line, `unknown type '${f.ty}' for field '${f.n}'`);
      if (f.dims) f.dimsV = f.dims.map(d => { if (!d) err(def.line, `field '${f.n}' needs a size`); return constInt(d); });
    }
  }
  const checkCycle = (def, path) => {
    for (const f of def.fields) if (records.has(f.ty)) {
      if (path.includes(f.ty)) err(def.line, `record ${path[0]} contains itself through '${f.n}'`);
      checkCycle(records.get(f.ty), path.concat(f.ty));
    }
  };
  for (const def of records.values()) checkCycle(def, [def.n]);

  for (const e of P.enums) {
    const m = new Map(); let next = 0;
    for (const mem of e.members) {
      if (m.has(mem.n)) err(e.line, `${e.n}.${mem.n} is listed twice`);
      const v = mem.e ? constInt(mem.e) : next; m.set(mem.n, v); next = v + 1;
    }
    enums.set(e.n, m);
  }
  for (const s of P.sprites) {
    const sp = { sprite: true, n: s.n, tile: constInt(s.tile), w: s.w ? constInt(s.w) : 1, h: s.h ? constInt(s.h) : 1 };
    if (sp.tile < 0 || (sp.tile % SHEET_COLS) + sp.w > SHEET_COLS || Math.floor(sp.tile / SHEET_COLS) + sp.h > TILE_COUNT / SHEET_COLS) err(s.line, `sprite ${s.n} extends past the edge of the sheet`);
    sprites.set(s.n, sp);
  }

  // asset name tables of the running cart
  {
    const sec = live();
    sec.sprites.slots.forEach((s, i) => { if (s.name && !s.empty) assetNames.Sprites.set(s.name, i); });
    sec.maps.slots.forEach((m, i) => {
      if (!m.name || !m.w) return;
      const a = new Arr("int", [m.h, m.w], Array.from(m.cells), "Maps." + m.name); a.ro = true; a.mapSlot = i;
      assetNames.Maps.set(m.name, a);
    });
    sec.sounds.slots.forEach((s, i) => {
      if (!s.name || s.bytes.length < 16) return;
      const r = new Rec(SOUND); r.ro = true; r.desc = soundDesc(s.bytes);
      for (const f of SOUND.fields) r.f[f.n] = f.dimsV ? new Arr("int", [64], new Array(64).fill(0), "notes") : (f.ty === "fix" ? FX0 : 0);
      assetNames.Sounds.set(s.name, r);
    });
  }
  function assetRef(prefix, name, line) {
    if (prefix === "Music") err(line, "music assets aren't supported yet");
    const m = assetNames[prefix];
    if (m.has(name)) return m.get(name);
    const kinds = { Sprites: "sprite", Maps: "map", Sounds: "sound" };
    const names = [...m.keys()];
    let hint = "";
    for (const k of names) if (levenshtein(name, k) <= 2) { hint = ` — did you mean ${prefix}.${k}?`; break; }
    err(line, `this cart has no ${kinds[prefix]} named '${name}'${hint}${names.length ? "" : ` (its ${kinds[prefix]} list is empty)`}`);
  }

  function constInt(node) {
    const v = withNoFrame(() => evS(node));
    if (v instanceof Fx) err(node.line, "this size or value must be an int");
    if (typeof v !== "number") err(node.line, "expected a number here");
    return v;
  }
  function withNoFrame(fn) { const f = frame; frame = null; try { return fn(); } finally { frame = f; } }

  // ---- construction ----
  function defVal(ty, line) {
    if (ty === "int") return 0;
    if (ty === "fix") return FX0;
    const def = records.get(ty);
    if (!def) err(line, `unknown type '${ty}'`);
    const r = new Rec(def);
    for (const f of def.fields) r.f[f.n] = f.dimsV ? newArr(f.ty, f.dimsV, f.n, line) : defVal(f.ty, line);
    if (def.defaults) Object.assign(r.f, def.defaults);
    return r;
  }
  function newArr(ty, dims, name, line) {
    const n = dims.reduce((a, b) => a * b, 1);
    if (n > 65535) err(line, `array '${name}' is too large (${n} elements)`);
    const data = new Array(n);
    for (let i = 0; i < n; i++) data[i] = defVal(ty, line);
    return new Arr(ty, dims, data, name);
  }
  function freeze(v) {
    if (v instanceof Arr) { v.ro = true; v.data.forEach(freeze); }
    else if (v instanceof Rec) { v.ro = true; Object.values(v.f).forEach(freeze); }
    return v;
  }
  function coerce(ty, v, line, what) {
    if (ty === "int") {
      if (v instanceof Fx) err(line, `${what || "this value"} is int but the value is fix; convert with int(x) or round(x)`);
      if (typeof v !== "number") err(line, `${what || "this value"} must be a number`);
      return v | 0;
    }
    if (ty === "fix") {
      if (typeof v === "number") return new Fx(v << 16);
      if (v instanceof Fx) return v;
      err(line, `${what || "this value"} must be a number`);
    }
    err(line, `can't store a value of type ${ty} here`);
  }
  const textCodes = s => Array.from(s, ch => ch.charCodeAt(0));
  function textArr(s, ro) { const a = new Arr("int", [s.length + 1], textCodes(s).concat(0), "text"); a.ro = !!ro; return a; }

  function flatValues(ty, items, line) {
    const out = [];
    for (const it of items) {
      if (it.k === "str") out.push(...textCodes(runSync(renderTpl(it))));
      else if (records.has(ty)) out.push(recFrom(ty, it, line));
      else if (it.k === "list" || it.k === "named") err(line, "nested { } here needs a 2D array or a record type");
      else out.push(coerce(ty, evS(it), line, "element"));
    }
    return out;
  }
  function buildArr(ty, dimNodes, init, name, line, isConst) {
    if (ty !== "int" && ty !== "fix" && !records.has(ty)) err(line, `unknown type '${ty}'`);
    const items = init ? (init.k === "list" ? init.items : [init]) : [];
    const dims = dimNodes.map(d => d ? constInt(d) : null);
    if (dims.length > 2) err(line, "arrays have at most two dimensions");
    if (dims.length === 1) {
      const vals = flatValues(ty, items, line);
      const n = dims[0] ?? vals.length;
      if (dims[0] == null && !vals.length) err(line, `'${name}[]' needs initial values to size it`);
      if (vals.length > n) err(line, `'${name}' has room for ${n} values but ${vals.length} are given`);
      const a = newArr(ty, [n], name, line);
      vals.forEach((v, i) => a.data[i] = v);
      return a;
    }
    const cols = dims[1];
    if (cols == null) err(line, `'${name}' needs a column count: ${name}[rows][cols]`);
    const rows = dims[0] ?? items.length;
    if (items.length > rows) err(line, `'${name}' has ${rows} rows but ${items.length} are given`);
    const a = newArr(ty, [rows, cols], name, line);
    items.forEach((it, r) => {
      let vals;
      if (it.k === "str") {
        vals = textCodes(runSync(renderTpl(it)));
        if (isConst && vals.length !== cols) err(it.line || line, `row ${r} of '${name}' has ${vals.length} characters; each row needs exactly ${cols}`);
      } else if (it.k === "list") vals = flatValues(ty, it.items, line);
      else err(line, `each row of '${name}' must be a { } list or a string`);
      if (vals.length > cols) err(line, `row ${r} of '${name}' has ${vals.length} values; the row size is ${cols}`);
      vals.forEach((v, c) => a.data[r * cols + c] = v);
    });
    return a;
  }
  function recFrom(ty, init, line) {
    const def = records.get(ty);
    const r = defVal(ty, line);
    if (!init) return r;
    if (init.k !== "list") {
      const v = evS(init);
      if (v instanceof Rec && v.def === def) { copyRec(r, v); return r; }
      err(line, `expected { … } to fill a ${ty}`);
    }
    init.items.forEach((it, i) => {
      let fd, node;
      if (it.k === "named") { fd = def.map[it.named]; if (!fd) err(line, `${ty} has no field '${it.named}'`); node = it.v; }
      else { fd = def.fields[i]; if (!fd) err(line, `too many values for a ${ty} (it has ${def.fields.length} fields)`); node = it; }
      if (fd.dimsV) {
        const tgt = r.f[fd.n];
        const vals = node.k === "list" ? flatValues(fd.ty, node.items, line) : node.k === "str" ? textCodes(runSync(renderTpl(node))) : err(line, `field '${fd.n}' is an array; give { … } or a string`);
        if (vals.length > tgt.data.length - (node.k === "str" ? 1 : 0)) err(line, `field '${fd.n}' holds ${tgt.data.length} values`);
        vals.forEach((v, k) => tgt.data[k] = v);
      } else if (records.has(fd.ty)) r.f[fd.n] = recFrom(fd.ty, node, line);
      else r.f[fd.n] = coerce(fd.ty, evS(node), line, `field '${fd.n}'`);
    });
    return r;
  }
  function copyRec(dst, src) {
    for (const f of dst.def.fields) {
      const d = dst.f[f.n], s = src.f[f.n];
      if (d instanceof Rec) copyRec(d, s);
      else if (d instanceof Arr) for (let i = 0; i < d.data.length; i++) { if (d.data[i] instanceof Rec) copyRec(d.data[i], s.data[i]); else d.data[i] = s.data[i]; }
      else dst.f[f.n] = s;
    }
    if (src.desc) dst.desc = src.desc;
  }
  function makeDecl(d, isConst) {
    const { n, sh, init, line } = d;
    if (sh.dims) return { ty: "arr", v: buildArr(sh.ty, sh.dims, init, n, line, isConst) };
    if (records.has(sh.ty)) return { ty: sh.ty, v: recFrom(sh.ty, init, line) };
    if (sh.ty !== "int" && sh.ty !== "fix") err(line, `unknown type '${sh.ty}'`);
    if (init && init.k === "str" && sh.implicit) {
      if (!init.plain) err(line, "a string with {…} can't be stored; it is only accepted by print, fmt and textwidth");
      return { ty: "arr", v: textArr(runSync(renderTpl(init)), false) };
    }
    if (init && init.k === "list") err(line, `'${n}' is a single value; declare it as an array to use { … }`);
    return { ty: sh.ty, v: init ? coerce(sh.ty, evS(init), line, `'${n}'`) : defVal(sh.ty, line) };
  }
  function constVal(n, line) {
    const c = consts.get(n);
    if (c.state === 1) err(line, `constant '${n}' refers to itself`);
    if (c.state === 0) {
      c.state = 1;
      const slot = withNoFrame(() => makeDecl(c.d, true));
      c.v = freeze(slot.v); c.state = 2;
    }
    return c.v;
  }

  // ---- name resolution ----
  function findSlot(n) {
    if (frame) for (let s = frame.scopes.length - 1; s >= 0; s--) { const v = frame.scopes[s].get(n); if (v) return v; }
    if (audioMode && avars.has(n)) return avars.get(n);
    return globals.get(n);
  }
  function suggest(n) {
    const names = new Set([...globals.keys(), ...consts.keys(), ...fns.keys(), ...BUILTIN_CONSTS.keys(), ...Object.keys(BI), ...enums.keys()]);
    if (frame) frame.scopes.forEach(s => s.forEach((_, k) => names.add(k)));
    let best = null, bd = 3;
    for (const k of names) { const d = levenshtein(n, k); if (d < bd) { bd = d; best = k; } }
    return best ? ` — did you mean '${best}'?` : "";
  }
  function readName(node) {
    const n = node.n;
    const s = findSlot(n);
    if (s) return s.v;
    if (consts.has(n)) return constVal(n, node.line);
    if (BUILTIN_CONSTS.has(n)) return BUILTIN_CONSTS.get(n);
    if (sprites.has(n)) return sprites.get(n);
    if (avars.has(n)) err(node.line, `'${n}' is an audio variable; only audio(t) can use it`);
    if (enums.has(n)) err(node.line, `'${n}' is an enum; use a member like ${n}.${enums.get(n).keys().next().value}`);
    if (ASSET_PREFIXES.has(n)) err(node.line, `'${n}' needs an asset name after it, like ${n}.NAME`);
    if (fns.has(n)) err(node.line, `'${n}' is a function; call it with ${n}(…)`);
    err(node.line, `unknown name '${n}'${suggest(n)}`);
  }
  const idx = (v, line) => { if (typeof v !== "number") err(line, v instanceof Fx ? "array index must be int; convert with int(x)" : "array index must be a number"); return v; };
  function oob(name, i, n, line) { err(line, `index ${i} is out of bounds for '${name}' (size ${n})`); }
  function zeroOf(a) { return a.et === "fix" ? FX0 : 0; }

  function indexGet(o, i, line) {
    if (o instanceof Arr) {
      if (o.dims.length === 2) {
        if (i < 0 || i >= o.dims[0]) { if (debug) oob(o.name, i, o.dims[0], line); return new Row(new Arr(o.et, [o.dims[1]], new Array(o.dims[1]).fill(zeroOf(o)), o.name), 0, o.dims[1]); }
        return new Row(o, i * o.dims[1], o.dims[1]);
      }
      if (i < 0 || i >= o.data.length) { if (debug) oob(o.name, i, o.data.length, line); return zeroOf(o); }
      return o.data[i];
    }
    if (o instanceof Row) {
      if (i < 0 || i >= o.n) { if (debug) oob(o.arr.name, i, o.n, line); return zeroOf(o.arr); }
      return o.arr.data[o.base + i];
    }
    if (o instanceof TextV) { const c = o.s.charCodeAt(i); return isNaN(c) ? 0 : c; }
    err(line, "only arrays can be indexed with [ ]");
  }
  function* lref(n) {
    if (n.k === "name") {
      const s = findSlot(n.n);
      if (!s) {
        if (consts.has(n.n) || BUILTIN_CONSTS.has(n.n)) err(n.line, `'${n.n}' is a constant and can't be changed`);
        err(n.line, `unknown variable '${n.n}'${suggest(n.n)} (declare it with var)`);
      }
      if (audioMode && !avars.has(n.n) && globals.get(n.n) === s) err(n.line, `audio(t) may not write globals; '${n.n}' is a global (use an audio var)`);
      return { c: s, k: "v", ty: s.ty, name: n.n };
    }
    if (n.k === "index") {
      const o = yield* ev(n.o), i = idx(yield* ev(n.i), n.line);
      let arr, base, size;
      if (o instanceof Arr) { if (o.dims.length === 2) err(n.line, `'${o.name}[${i}]' is a whole row; index it again to change one element`); arr = o; base = 0; size = o.data.length; }
      else if (o instanceof Row) { arr = o.arr; base = o.base; size = o.n; }
      else err(n.line, "only arrays can be indexed with [ ]");
      if (arr.ro) err(n.line, `'${arr.name}' is read-only and can't be written`);
      if (i < 0 || i >= size) { if (debug) oob(arr.name, i, size, n.line); return null; }
      return { c: arr.data, k: base + i, ty: arr.et, name: arr.name };
    }
    if (n.k === "field") {
      const o = yield* ev(n.o);
      if (!(o instanceof Rec)) err(n.line, `'.${n.f}' needs a record on its left`);
      const fd = o.def.map[n.f];
      if (!fd) err(n.line, `${o.def.n} has no field '${n.f}'`);
      if (o.ro) err(n.line, `this ${o.def.n} is a constant and can't be changed`);
      return { c: o.f, k: n.f, ty: fd.dimsV ? "arr" : fd.ty, name: n.f };
    }
    err(n.line, "this can't be assigned to");
  }
  function store(r, v, line) {
    if (!r) return;
    const cur = r.c[r.k];
    if (cur instanceof Rec) {
      if (!(v instanceof Rec) || v.def !== cur.def) err(line, `expected a ${cur.def.n} record here`);
      copyRec(cur, v); return;
    }
    if (cur instanceof Arr || r.ty === "arr") err(line, `arrays can't be assigned; copy them with a loop or copy(dst, src)`);
    r.c[r.k] = coerce(r.ty, v, line, `'${r.name}'`);
  }

  // ---- arithmetic ----
  function truth(v, line) {
    if (typeof v === "number") return v !== 0;
    if (v instanceof Fx) return v.r !== 0;
    err(line, "a condition must be a number");
  }
  function arith(op, a, b, line) {
    if (!isNum(a) || !isNum(b)) {
      if ((op === "==" || op === "!=") && (a instanceof Arr || b instanceof Arr || a instanceof TextV || b instanceof TextV)) err(line, "'==' can't compare arrays or text; use streq(a, b) or arreq(a, b, n)");
      if (a instanceof Rec || b instanceof Rec) err(line, "records can't be compared or used in arithmetic");
      err(line, `'${op}' needs numbers on both sides`);
    }
    if (a instanceof Fx || b instanceof Fx) {
      if (op === "&" || op === "|" || op === "^" || op === "<<" || op === ">>") err(line, `'${op}' needs int operands; convert with int(x)`);
      const x = toFx(a).r, y = toFx(b).r;
      switch (op) {
        case "+": return new Fx(x + y);
        case "-": return new Fx(x - y);
        case "*": return new Fx(fmul(x, y));
        case "/": if (y === 0) { if (debug) err(line, "division by zero"); return FX0; } return new Fx(Math.trunc(x * 65536 / y));
        case "%": if (y === 0) { if (debug) err(line, "modulo by zero"); return FX0; } return new Fx(x % y);
        case "<": return x < y ? 1 : 0; case "<=": return x <= y ? 1 : 0;
        case ">": return x > y ? 1 : 0; case ">=": return x >= y ? 1 : 0;
        case "==": return x === y ? 1 : 0; case "!=": return x !== y ? 1 : 0;
      }
    }
    switch (op) {
      case "+": return (a + b) | 0;
      case "-": return (a - b) | 0;
      case "*": return Math.imul(a, b);
      case "/": if (b === 0) { if (debug) err(line, "division by zero"); return 0; } return (a / b) | 0;
      case "%": if (b === 0) { if (debug) err(line, "modulo by zero"); return 0; } return (a % b) | 0;
      case "&": return a & b; case "|": return a | b; case "^": return a ^ b;
      case "<<": return a << (b & 31); case ">>": return a >> (b & 31);
      case "<": return a < b ? 1 : 0; case "<=": return a <= b ? 1 : 0;
      case ">": return a > b ? 1 : 0; case ">=": return a >= b ? 1 : 0;
      case "==": return a === b ? 1 : 0; case "!=": return a !== b ? 1 : 0;
    }
    err(line, `unknown operator '${op}'`);
  }

  function* renderTpl(n) {
    let s = "";
    for (const p of n.parts) {
      if (typeof p === "string") { s += p; continue; }
      const v = yield* ev(p.e);
      if (v instanceof Arr || v instanceof Row) s += arrText(v);
      else if (v instanceof TextV) s += v.s;
      else if (isNum(v)) s += fmtNum(v, p.f);
      else err(n.line, "only numbers and text can go inside {…}");
    }
    return s;
  }
  function textOf(v, line) {
    if (v instanceof TextV) return v.s;
    if (v instanceof Arr || v instanceof Row) return arrText(v);
    if (isNum(v)) return fmtNum(v, null);
    err(line, "expected text or a number");
  }

  // ---- evaluation ----
  function tick(line) { if (++steps > budget) err(line, audioMode ? "audio(t) took too long for one sample" : `this frame ran more than ${budget.toLocaleString("en")} steps (endless loop?)`); }
  function* ev(n) {
    switch (n.k) {
      case "num": return n.v;
      case "fixlit": return n.val || (n.val = new Fx(n.r));
      case "str": return new TextV(yield* renderTpl(n));
      case "name": return readName(n);
      case "index": { const o = yield* ev(n.o); return indexGet(o, idx(yield* ev(n.i), n.line), n.line); }
      case "field": {
        if (n.o.k === "name" && !findSlot(n.o.n)) {
          if (enums.has(n.o.n)) {
            const m = enums.get(n.o.n);
            if (!m.has(n.f)) err(n.line, `${n.o.n} has no member '${n.f}'`);
            return m.get(n.f);
          }
          if (ASSET_PREFIXES.has(n.o.n)) return assetRef(n.o.n, n.f, n.line);
        }
        const o = yield* ev(n.o);
        if (n.f === "length") {
          if (o instanceof Arr) return o.dims[0];
          if (o instanceof Row) return o.n;
          err(n.line, "'.length' works on arrays, record arrays and constant tables");
        }
        if (!(o instanceof Rec)) err(n.line, `'.${n.f}' needs a record on its left`);
        if (!(n.f in o.f)) err(n.line, `${o.def.n} has no field '${n.f}'`);
        return o.f[n.f];
      }
      case "call": tick(n.line); return yield* call(n);
      case "un": {
        const a = yield* ev(n.a);
        if (n.op === "!") return truth(a, n.line) ? 0 : 1;
        if (!isNum(a)) err(n.line, `'${n.op}' needs a number`);
        if (n.op === "-") return a instanceof Fx ? new Fx(-a.r) : (-a) | 0;
        if (a instanceof Fx) err(n.line, "'~' needs an int; convert with int(x)");
        return ~a;
      }
      case "bin":
        if (n.op === "&&") return truth(yield* ev(n.a), n.line) && truth(yield* ev(n.b), n.line) ? 1 : 0;
        if (n.op === "||") return truth(yield* ev(n.a), n.line) || truth(yield* ev(n.b), n.line) ? 1 : 0;
        { const a = yield* ev(n.a); const b = yield* ev(n.b); return arith(n.op, a, b, n.line); }
      case "tern": return truth(yield* ev(n.c), n.line) ? yield* ev(n.a) : yield* ev(n.b);
    }
    err(n.line, "this is not a value");
  }

  function* call(n) {
    const f = fns.get(n.n);
    if (f) return yield* callUser(f, n.args, n.line);
    const b = BI[n.n];
    if (!b) {
      if (HANDLERS.has(n.n)) err(n.line, `${n.n}() is called by the console, not by the cart`);
      err(n.line, `unknown function '${n.n}'${suggest(n.n)}`);
    }
    const [min, max] = b.arity;
    if (n.args.length < min || n.args.length > max) err(n.line, `${n.n} takes ${min === max ? min : `${min} to ${max}`} argument${max === 1 ? "" : "s"}, got ${n.args.length}`);
    if (b.audioOnly && !audioMode) err(n.line, `${n.n}() only works inside audio(t)`);
    const args = [];
    for (const a of n.args) args.push(yield* ev(a));
    const r = b.fn(args, n.line);
    if (r instanceof SysReq) {
      if (audioMode) err(n.line, `${n.n}() can't be used inside audio(t)`);
      const answer = yield r;
      return r.finish(answer);
    }
    return r;
  }
  function bindParams(params, args, line, who) {
    const scope = new Map();
    params.forEach((p, i) => {
      let v = args[i];
      if (p.dims) {
        if (v instanceof TextV) v = textArr(v.s, true);
        if (!(v instanceof Arr || v instanceof Row)) err(line, `argument ${i + 1} of ${who} must be an array`);
        const et = v instanceof Row ? v.arr.et : v.et;
        if (et !== p.ty) err(line, `argument ${i + 1} of ${who} must be an array of ${p.ty}`);
        scope.set(p.n, { ty: "arr", v });
      } else if (records.has(p.ty)) {
        if (!(v instanceof Rec) || v.def.n !== p.ty) err(line, `argument ${i + 1} of ${who} must be a ${p.ty}`);
        scope.set(p.n, { ty: p.ty, v });
      } else if (p.ty === "int" || p.ty === "fix") {
        scope.set(p.n, { ty: p.ty, v: coerce(p.ty, v, line, `parameter '${p.n}' of ${who}`) });
      } else err(line, `unknown type '${p.ty}' for parameter '${p.n}'`);
    });
    return scope;
  }
  function* callUser(f, argNodes, line) {
    if (audioMode) err(line, "audio(t) may not call user functions");
    if (argNodes.length !== f.params.length) err(line, `${f.n} takes ${f.params.length} argument${f.params.length === 1 ? "" : "s"}, got ${argNodes.length}`);
    if (depth >= 16) err(line, `call depth exceeded (16 frames) in ${f.n}`);
    const args = [];
    for (const a of argNodes) args.push(yield* ev(a));
    const scope = bindParams(f.params, args, line, f.n);
    const saved = frame; frame = { scopes: [scope], ret: null }; depth++;
    let sig, ret;
    try { sig = yield* execList(f.body); ret = frame.ret; } finally { frame = saved; depth--; }
    if (sig === RET && ret != null) {
      if (f.ret === "fix") return toFx(coerce("fix", ret, line));
      if (ret instanceof Fx) err(line, `${f.n} returns a fix value but is declared int; add ': fix' after its parameters`);
      if (typeof ret !== "number") err(line, `${f.n} must return a number`);
      return ret;
    }
    return f.ret === "fix" ? FX0 : 0;
  }

  // ---- statements ----
  function* execList(list) { for (let i = 0; i < list.length; i++) { const r = yield* exec(list[i]); if (r) return r; } return 0; }
  function* execBlock(list) { frame.scopes.push(new Map()); try { return yield* execList(list); } finally { frame.scopes.pop(); } }
  function* exec(s) {
    tick(s.line);
    switch (s.k) {
      case "local": {
        const n = s.n;
        if (taken.has(n)) err(s.line, `'${n}' is already a ${taken.get(n)}; locals can't reuse names`);
        if (BUILTIN_CONSTS.has(n) || BI[n] || ASSET_PREFIXES.has(n)) err(s.line, `'${n}' is a built-in name`);
        for (const sc of frame.scopes) if (sc.has(n)) err(s.line, `'${n}' is already declared in this function`);
        if (s.ty !== "int" && s.ty !== "fix") err(s.line, records.has(s.ty) ? "local records are not supported; declare records at top level" : `unknown type '${s.ty}'`);
        const v = s.e ? coerce(s.ty, yield* ev(s.e), s.line, `'${n}'`) : defVal(s.ty);
        frame.scopes[frame.scopes.length - 1].set(n, { ty: s.ty, v });
        return 0;
      }
      case "assign": {
        const r = yield* lref(s.lv);
        if (s.op === "=") { store(r, yield* ev(s.e), s.line); return 0; }
        const v = yield* ev(s.e);
        if (!r) return 0;
        store(r, arith(s.op.slice(0, -1), r.c[r.k], v, s.line), s.line);
        return 0;
      }
      case "inc": {
        const r = yield* lref(s.lv); if (!r) return 0;
        const cur = r.c[r.k];
        if (!isNum(cur)) err(s.line, "'++' and '--' need a number");
        store(r, cur instanceof Fx ? new Fx(cur.r + s.d * 65536) : (cur + s.d) | 0, s.line);
        return 0;
      }
      case "expr": yield* ev(s.e); return 0;
      case "if": return truth(yield* ev(s.c), s.line) ? yield* execBlock(s.then) : s.els ? yield* execBlock(s.els) : 0;
      case "while":
        while (truth(yield* ev(s.c), s.line)) { tick(s.line); const r = yield* execBlock(s.body); if (r === BRK) break; if (r === RET) return RET; }
        return 0;
      case "for":
        frame.scopes.push(new Map());
        try {
          if (s.init) yield* exec(s.init);
          while (!s.c || truth(yield* ev(s.c), s.line)) {
            tick(s.line);
            const r = yield* execBlock(s.body);
            if (r === BRK) break; if (r === RET) return RET;
            if (s.step) yield* exec(s.step);
          }
        } finally { frame.scopes.pop(); }
        return 0;
      case "switch": {
        const v = yield* ev(s.e);
        for (const c of s.cases) for (const cv of c.vals) {
          if (arith("==", v, yield* ev(cv), s.line)) return yield* execBlock(c.body);
        }
        return s.els ? yield* execBlock(s.els) : 0;
      }
      case "break": return BRK;
      case "continue": return CONT;
      case "return": frame.ret = s.e ? yield* ev(s.e) : null; return RET;
    }
    err(s.line, "unknown statement");
  }

  // ---- load ----
  for (const c of consts.keys()) constVal(c, consts.get(c).d.line);
  for (const v of P.vars) globals.set(v.n, withNoFrame(() => makeDecl(v, false)));
  for (const a of P.avars) {
    if (a.ty !== "int" && a.ty !== "fix") err(a.line, "audio variables are int or fix");
    avars.set(a.n, { ty: a.ty, v: coerce(a.ty, withNoFrame(() => evS(a.e)), a.line, `'${a.n}'`) });
  }
  const HS = P.handlers;
  for (const h of Object.values(HS)) {
    const max = h.n === "audio" ? 1 : h.n === "init" ? 0 : 2;
    if (h.params.length > max) err(h.line, `${h.n}() takes at most ${max} parameter${max === 1 ? "" : "s"}`);
    for (const p of h.params) if (p.dims || p.ty !== "int") err(h.line, `${h.n}() parameters are plain ints`);
  }
  function* runHandler(h, args, limit, stage) {
    if (!h) return null;
    steps = 0; budget = limit;
    frame = { scopes: [new Map()], ret: null }; depth = 0;
    h.params.forEach((p, i) => frame.scopes[0].set(p.n, { ty: "int", v: args[i] | 0 }));
    try { yield* execList(h.body); return frame.ret; }
    catch (e) { if (e && !e.stage) e.stage = stage; throw e; }
    finally { frame = null; }
  }

  let audioFault = false;
  return {
    meta, hasAudio: !!HS.audio,
    initTask: () => runHandler(HS.init, [], 20_000_000, "init"),
    frameTask: function* (frameNo, inp) {
      prevInput = input; input = inp;
      yield* runHandler(HS.update, [frameNo, inp], 5_000_000, "update");
      yield* runHandler(HS.draw, [frameNo, inp], 5_000_000, "draw");
    },
    resumed() { steps = 0; },
    audioSample(t) {
      if (audioFault) return 128;
      const saved = { frame, depth, steps, budget };
      audioMode = true;
      try {
        const v = runSync(runHandler(HS.audio, [t], 20_000, "audio"));
        if (v == null) return 128;
        return (v instanceof Fx ? v.r >> 16 : v) & 255;
      } catch (e) {
        audioFault = true;
        if (debug) host.audioFault(e);
        return 128;
      } finally { audioMode = false; ({ frame, depth, steps, budget } = saved); }
    }
  };
}
