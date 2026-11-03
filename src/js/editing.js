/* =====================================================================
   Editing API, the shelf's side (spec: Storage › Editing API): handles on the
   picked carts' asset sections. Nothing reaches the shelf until savecart.
   `err(line, message)` must throw.
   ===================================================================== */
class BErr extends Error { constructor(msg, line) { super(msg); this.line = line || 0; } }
const ASSET_KINDS = { tiles: "sprites", sprites: "sprites", maps: "maps", sounds: "sounds", palette: "palette" };
class SysReq { constructor(o) { Object.assign(this, o); } }
const midiFreq = n => 440 * Math.pow(2, (n - 69) / 12);
function soundDesc(b) {
  const u16 = i => b[i] | (b[i + 1] << 8);
  const n = b[3];
  return {
    wave: b[0] & 3, vol: b[1] / 255, duty: b[2] / 255, from: u16(4), to: u16(6), time: u16(8) / 1000,
    attack: u16(10) / 1000, release: u16(12) / 1000, step: Math.max(0.01, u16(14) / 1000),
    seq: n ? Array.from(b.subarray(16, 16 + n), m => m ? midiFreq(m) : null) : null
  };
}

function makeEditing(host, err) {
  const handles = [];
  function slotOf(h, line) {
    const H_ = handles[h];
    if (!H_) err(line, `${h} is not a cart handle; get one from pickcart(kind)`);
    return H_;
  }
  function sectionOf(kind, line) {
    if (kind === "music") err(line, "music assets aren't supported yet");
    const secName = ASSET_KINDS[kind];
    if (!secName) err(line, `unknown asset kind "${kind}"; use "tiles", "sprites", "maps", "sounds" or "palette"`);
    return secName;
  }
  function slotCount(H_) {
    switch (H_.kind) { case "tiles": return TILE_COUNT; case "palette": return 1; default: return H_.sec.slots.length; }
  }
  function slotBytes(H_, i, line) {
    const n = slotCount(H_);
    if (i < 0 || i >= n) err(line, `slot ${i} doesn't exist; this ${H_.kind} section has ${n}`);
    const s = H_.sec;
    switch (H_.kind) {
      case "tiles": { const b = new Uint8Array(65); b.set(s.sheet.subarray(i * 64, i * 64 + 64)); b[64] = s.flags[i]; return b; }
      case "sprites": { const sl = s.slots[i]; return sl.empty ? new Uint8Array(0) : Uint8Array.of(sl.tile & 255, sl.tile >> 8, sl.w, sl.h); }
      case "maps": { const m = s.slots[i]; if (!m.w) return new Uint8Array(0); const b = new Uint8Array(4 + m.cells.length * 2); b.set([m.w & 255, m.w >> 8, m.h & 255, m.h >> 8]); b.set(u16bytes(m.cells), 4); return b; }
      case "sounds": return s.slots[i].bytes.slice();
      case "palette": { const w = s ? s.words : new Uint16Array(256); if (!s) PAL_DEFAULT.forEach((v, k) => w[k] = v); return u16bytes(w).slice(); }
    }
  }
  // vals: one byte per array element
  function writeSlot(H_, vals, i, line) {
    const need = n => { if (vals.length < n) err(line, `writeasset needs ${n} values for this ${H_.kind} slot; the array holds ${vals.length}`); return Uint8Array.from(vals.slice(0, n)); };
    if (!H_.pick.writable) return -1;          // read-only until the user allows it
    const n = slotCount(H_), s = H_.sec;
    const fixed = H_.kind === "tiles" || H_.kind === "palette";
    if (i < 0 || i > n || (fixed && i === n)) err(line, `slot ${i} can't be written; this ${H_.kind} section has ${n} slots${fixed ? "" : " (use " + n + " to append)"}`);
    switch (H_.kind) {
      case "tiles": { const b = need(65); s.sheet.set(b.subarray(0, 64), i * 64); s.flags[i] = b[64]; break; }
      case "sprites": {
        const b = need(4); const tile = b[0] | (b[1] << 8);
        if (!b[2] || !b[3]) err(line, "a sprite needs a width and height of at least 1 tile");
        if ((tile % SHEET_COLS) + b[2] > SHEET_COLS || Math.floor(tile / SHEET_COLS) + b[3] > TILE_COUNT / SHEET_COLS) err(line, "this sprite extends past the edge of the sheet");
        const old = s.slots[i]; s.slots[i] = { name: old ? old.name : "", tile, w: b[2], h: b[3] }; break;
      }
      case "maps": {
        if (vals.length < 4) err(line, "a map slot starts with width and height (2 bytes each)");
        const w = vals[0] | (vals[1] << 8), h = vals[2] | (vals[3] << 8);
        const b = need(4 + w * h * 2);
        const old = s.slots[i]; s.slots[i] = { name: old ? old.name : "", w, h, cells: bytesU16(b.subarray(4)) }; break;
      }
      case "sounds": {
        if (vals.length < 4) err(line, "a sound slot is a 16-byte record followed by its notes");
        const b = need(16 + vals[3]);
        const old = s.slots[i]; s.slots[i] = { name: old ? old.name : "", bytes: b }; break;
      }
      case "palette": { const b = need(512); H_.sec = s || { rev: 0, words: null }; H_.sec.words = bytesU16(b); break; }
    }
    H_.dirty = true;
    return 1;
  }
  // pickcart: the system dialog's request; finish(cartId) gives the handle or -1.
  // Choosing the cart is the user's authorization to modify it (spec: Editing API).
  function pickFinish(kind, line) {
    const secName = sectionOf(kind, line);
    return new SysReq({ type: "pick", kind, finish: cartId => {
      if (cartId == null) return -1;
      host.setProject(cartId);
      const pick = { cartId, writable: true };   // shared by every handle opened from this pick
      const sec = cloneSection(secName, host.cartSections(cartId)[secName]);
      handles.push({ pick, kind, secName, sec, dirty: false });
      return handles.length - 1;
    } });
  }
  // opencart: another cart's asset section, read-only, no dialog. c = -1 is the current project.
  function openCart(c, kind, line) {
    const secName = sectionOf(kind, line);
    const ids = host.cartIds();
    const cartId = c === -1 ? host.projectId() : ids[c];
    if (cartId == null || !ids.includes(cartId)) return -1;
    handles.push({ pick: { cartId, writable: false }, kind, secName, sec: cloneSection(secName, host.cartSections(cartId)[secName]), dirty: false });
    return handles.length - 1;
  }
  // requestwrite: the system asks the user to let this cart modify the cart behind h
  function requestWrite(H_) {
    if (H_.pick.writable) return new SysReq({ type: "noop", finish: () => 1 });
    return new SysReq({ type: "write", asker: host.runningTitle(), target: host.cartTitle(H_.pick.cartId), finish: ok => {
      if (ok) { H_.pick.writable = true; host.setProject(H_.pick.cartId); }
      return ok ? 1 : 0;
    } });
  }
  function openSection(h, kind, line) {
    const H_ = slotOf(h, line), secName = sectionOf(kind, line);
    // tiles and sprites live in one section: share it so edits through either handle agree
    const same = handles.find(x => x.pick === H_.pick && x.secName === secName);
    const sec = same ? same.sec : cloneSection(secName, host.cartSections(H_.pick.cartId)[secName]);
    handles.push({ pick: H_.pick, kind, secName, sec, dirty: false });
    return handles.length - 1;
  }
  function nameOf(H_, i, line) {
    if (i < 0 || i >= slotCount(H_)) err(line, `slot ${i} doesn't exist`);
    return H_.kind !== "tiles" && H_.kind !== "palette" ? (H_.sec.slots[i].name || "") : "";
  }
  function setName(H_, i, name, line) {
    if (H_.kind === "tiles" || H_.kind === "palette") err(line, `${H_.kind} have no names; name sprites, maps or sounds`);
    if (i < 0 || i >= slotCount(H_)) err(line, `slot ${i} doesn't exist`);
    if (name && !/^[A-Za-z_][A-Za-z0-9_]{0,23}$/.test(name)) err(line, `"${name}" is not a valid asset name (letters, digits and _, at most 24 characters)`);
    if (name && H_.sec.slots.some((s, k) => k !== i && s.name === name)) err(line, `another asset is already named ${name}`);
    if (!H_.pick.writable) return -1;
    H_.sec.slots[i].name = name; H_.dirty = true; return 1;
  }
  // drawasset: the first tile and size in tiles of slot i
  function drawInfo(H_, i, line) {
    if (H_.secName !== "sprites") err(line, `drawasset draws tiles or sprites, not ${H_.kind}`);
    if (H_.kind === "tiles") {
      if (i < 0 || i >= TILE_COUNT) err(line, `tile ${i} doesn't exist`);
      return { tile: i, w: 1, h: 1 };
    }
    const sl = H_.sec.slots[i];
    if (!sl) err(line, `sprite ${i} doesn't exist`);
    return sl.empty ? { tile: 0, w: 0, h: 0 } : { tile: sl.tile, w: sl.w, h: sl.h };
  }
  // savecart: one confirmation saves every section opened through this pick
  function saveGroup(H_) {
    if (!H_.pick.writable) return new SysReq({ type: "noop", finish: () => 0 });
    return new SysReq({ type: "save", handle: H_, asker: host.runningTitle(), handles: handles.filter(x => x.pick === H_.pick), finish: r => r ? 1 : 0 });
  }
  return { handles, slotOf, slotCount, slotBytes, writeSlot, pickFinish, openCart, requestWrite, openSection, nameOf, setName, drawInfo, saveGroup };
}
