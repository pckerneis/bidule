/* =====================================================================
   Carts: source + asset sections (spec: Asset section layout)
   ===================================================================== */
const SHEET_COLS = 32, TILE_COUNT = 1024;
function emptySections() {
  return {
    sprites: { rev: 0, sheet: new Uint8Array(TILE_COUNT * 64), flags: new Uint8Array(TILE_COUNT), slots: [] },
    maps: { rev: 0, slots: [] },
    sounds: { rev: 0, slots: [] },
    palette: null
  };
}
function cloneSection(name, s) {
  if (!s) return null;
  switch (name) {
    case "sprites": return { rev: s.rev, sheet: s.sheet.slice(), flags: s.flags.slice(), slots: s.slots.map(x => ({ ...x })) };
    case "maps": return { rev: s.rev, slots: s.slots.map(x => ({ ...x, cells: x.cells.slice() })) };
    case "sounds": return { rev: s.rev, slots: s.slots.map(x => ({ ...x, bytes: x.bytes.slice() })) };
    case "palette": return { rev: s.rev, words: s.words.slice() };
  }
  return null;
}
function cloneSections(sec) {
  return { sprites: cloneSection("sprites", sec.sprites), maps: cloneSection("maps", sec.maps), sounds: cloneSection("sounds", sec.sounds), palette: cloneSection("palette", sec.palette) };
}
const b64 = u8 => { let s = ""; for (let i = 0; i < u8.length; i += 0x8000) s += String.fromCharCode.apply(null, u8.subarray(i, i + 0x8000)); return btoa(s); };
const unb64 = s => Uint8Array.from(atob(s), c => c.charCodeAt(0));
const u16bytes = a => new Uint8Array(a.buffer, a.byteOffset, a.byteLength);
const bytesU16 = u8 => { const c = u8.slice(); return new Uint16Array(c.buffer, 0, c.length >> 1); };
function packSections(sec) {
  return {
    sprites: { rev: sec.sprites.rev, sheet: b64(sec.sprites.sheet), flags: b64(sec.sprites.flags), slots: sec.sprites.slots },
    maps: { rev: sec.maps.rev, slots: sec.maps.slots.map(m => ({ name: m.name, w: m.w, h: m.h, cells: b64(u16bytes(m.cells)) })) },
    sounds: { rev: sec.sounds.rev, slots: sec.sounds.slots.map(s => ({ name: s.name, bytes: b64(s.bytes) })) },
    palette: sec.palette ? { rev: sec.palette.rev, words: b64(u16bytes(sec.palette.words)) } : null
  };
}
function unpackSections(p) {
  const sec = emptySections();
  if (!p) return sec;
  if (p.sprites) { sec.sprites = { rev: p.sprites.rev | 0, sheet: unb64(p.sprites.sheet), flags: unb64(p.sprites.flags), slots: p.sprites.slots || [] }; }
  if (p.maps) sec.maps = { rev: p.maps.rev | 0, slots: p.maps.slots.map(m => ({ name: m.name, w: m.w, h: m.h, cells: bytesU16(unb64(m.cells)) })) };
  if (p.sounds) sec.sounds = { rev: p.sounds.rev | 0, slots: p.sounds.slots.map(s => ({ name: s.name, bytes: unb64(s.bytes) })) };
  if (p.palette) sec.palette = { rev: p.palette.rev | 0, words: bytesU16(unb64(p.palette.words)) };
  return sec;
}
function soundBytes(o) {
  const n = o.notes || [];
  const b = new Uint8Array(16 + n.length);
  const put16 = (i, v) => { b[i] = v & 255; b[i + 1] = (v >> 8) & 255; };
  b[0] = o.wave | 0; b[1] = o.vol ?? 255; b[2] = o.duty ?? 128; b[3] = n.length;
  put16(4, o.from | 0); put16(6, o.to | 0); put16(8, o.time | 0); put16(10, o.attack | 0); put16(12, o.release | 0); put16(14, o.step | 0);
  n.forEach((v, i) => b[16 + i] = v);
  return b;
}
function paintTiles(spr, tiles) {
  for (const [t, rows] of Object.entries(tiles))
    rows.forEach((r, y) => [...r].forEach((ch, x) => spr.sheet[(+t) * 64 + y * 8 + x] = parseInt(ch, 16)));
}
