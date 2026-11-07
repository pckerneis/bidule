/* =====================================================================
   The console engine: the Rust compiler, VM and runtime, built to WebAssembly
   (crates/bidule-web). Same shape as a task-driven VM for the host: tasks yield
   system dialog requests, and { type: "late" } when a handler runs out of budget.
   ===================================================================== */
let wasmModule = null;
const utf8 = s => Uint8Array.from(unescape(encodeURIComponent(s)), c => c.charCodeAt(0));
const latin1 = b => { let s = ""; for (let i = 0; i < b.length; i++) s += String.fromCharCode(b[i]); return s; };
const fromUtf8 = b => { try { return decodeURIComponent(escape(latin1(b))); } catch (_) { return latin1(b); } };

/** Spec › Asset section layout › Common container. */
function assetContainer(kind, rev, fixed, slots) {
  let size = fixed.length; for (const s of slots) size += s.bytes.length;
  const payload = new Uint8Array(size);
  payload.set(fixed);
  const table = [];
  let at = fixed.length;
  for (const s of slots) {
    table.push(s.bytes.length ? at : 0, s.bytes.length);
    payload.set(s.bytes, at); at += s.bytes.length;
  }
  const names = slots.flatMap(s => [s.name.length, ...[...s.name].map(c => c.charCodeAt(0))]);
  const out = new Uint8Array(16 + names.length + table.length * 4 + payload.length);
  const dv = new DataView(out.buffer);
  out[0] = kind; out[1] = 1; dv.setUint16(2, rev & 0xFFFF, true); dv.setUint16(4, slots.length, true);
  dv.setUint32(8, crc32(payload), true);
  let p = 16;
  out.set(names, p); p += names.length;
  for (const v of table) { dv.setUint32(p, v, true); p += 4; }
  out.set(payload, p);
  return out;
}
const CRC_TABLE = Int32Array.from({ length: 256 }, (_, n) => { let c = n; for (let k = 0; k < 8; k++) c = c & 1 ? 0xEDB88320 ^ (c >>> 1) : c >>> 1; return c; });
function crc32(bytes) { let c = -1; for (let i = 0; i < bytes.length; i++) c = CRC_TABLE[(c ^ bytes[i]) & 255] ^ (c >>> 8); return (c ^ -1) >>> 0; }
/** A cart's asset sections in their on-disk layout. */
function sectionImages(s) {
  const u16 = v => [v & 255, (v >> 8) & 255];
  const sheet = new Uint8Array(s.sprites.sheet.length + s.sprites.flags.length);
  sheet.set(s.sprites.sheet); sheet.set(s.sprites.flags, s.sprites.sheet.length);
  const out = {
    sprites: assetContainer(0x10, s.sprites.rev, sheet, s.sprites.slots.map(x => ({ name: x.name || "", bytes: x.empty ? [] : [...u16(x.tile), x.w, x.h] }))),
    maps: assetContainer(0x11, s.maps.rev, [], s.maps.slots.map(m => {
      if (m.empty || !m.w) return { name: m.name || "", bytes: [] };
      const b = new Uint8Array(4 + m.cells.length * 2);
      b.set([...u16(m.w), ...u16(m.h)]); b.set(u16bytes(m.cells), 4);
      return { name: m.name || "", bytes: b };
    })),
    sounds: assetContainer(0x12, s.sounds.rev, [], s.sounds.slots.map(x => ({ name: x.name || "", bytes: x.empty ? [] : x.bytes }))),
    palette: null,
  };
  if (s.palette) out.palette = assetContainer(0x14, s.palette.rev, u16bytes(s.palette.words), []);
  return out;
}

const DIAG = /^(\d+):\d+: (error|warning): (.*)$/;
const STEP_DONE = 0, STEP_LATE = 1, STEP_DIALOG = 2, STEP_FAULT = 3;
const HANDLER_INIT = 0, HANDLER_UPDATE = 1, HANDLER_DRAW = 2;
// Built-in ids (crates/bidule-bytecode/src/builtins.rs)
const B_PICKCART = 42, B_OPENSECTION = 43, B_CARTNAME = 44, B_DRAWASSET = 45, B_READASSET = 47, B_WRITEASSET = 48,
  B_ASSETCOUNT = 49, B_ASSETNAME = 50, B_SETASSETNAME = 51, B_SAVECART = 52,
  B_CARTCOUNT = 56, B_CARTINFO = 57, B_OPENCART = 58, B_REQUESTWRITE = 59;

function makeWasmVM(src, host) {
  if (!wasmModule) wasmModule = new WebAssembly.Module(Uint8Array.from(atob(BIDULE_WASM), c => c.charCodeAt(0)));
  const fail = (line, msg) => { throw new BErr(msg, line); };
  const edit = makeEditing(host, fail);
  let shelfError = null, pendingDialog = null;
  let x = null;   // the instance's exports
  const bytes = (ptr, len) => new Uint8Array(x.memory.buffer, ptr, len);

  // Editing API built-ins. Errors stay here: the VM only learns that the cart stops.
  function shelfCall(id, argp, nargs, textp, textLen, buf, cap, len, faultp) {
    const a = new Int32Array(x.memory.buffer, argp, nargs);
    const text = () => latin1(bytes(textp, textLen));
    const fill = b => { bytes(buf, cap).set(b.length > cap ? b.subarray(0, cap) : b); return b.length; };
    try {
      switch (id) {
        case B_PICKCART: pendingDialog = edit.pickFinish(text(), 0); return 0;
        case B_SAVECART: pendingDialog = edit.saveGroup(edit.slotOf(a[0], 0)); return 0;
        case B_OPENSECTION: return edit.openSection(a[0], text(), 0);
        case B_CARTCOUNT: return host.cartIds().length;
        case B_CARTINFO: {
          const id = host.cartIds()[a[0]];
          if (id == null) fail(0, `cart ${a[0]} doesn't exist; cartcount() says how many there are`);
          return fill(utf8(host.cartTitle(id)));
        }
        case B_OPENCART: return edit.openCart(a[0], text(), 0);
        case B_REQUESTWRITE: pendingDialog = edit.requestWrite(edit.slotOf(a[0], 0)); return 0;
        case B_CARTNAME: return fill(utf8(host.cartTitle(edit.slotOf(a[0], 0).pick.cartId)));
        case B_ASSETNAME: return fill(utf8(edit.nameOf(edit.slotOf(a[0], 0), a[1], 0)));
        case B_ASSETCOUNT: return edit.slotCount(edit.slotOf(a[0], 0));
        case B_SETASSETNAME: return edit.setName(edit.slotOf(a[0], 0), a[1], text(), 0);
        case B_READASSET: return fill(edit.slotBytes(edit.slotOf(a[0], 0), a[2], 0));
        case B_WRITEASSET: return edit.writeSlot(edit.slotOf(a[0], 0), Array.from(bytes(buf, len)), a[2], 0);
        case B_DRAWASSET: {
          const H_ = edit.slotOf(a[0], 0), d = edit.drawInfo(H_, a[1], 0), out = bytes(buf, cap);
          for (let ty = 0; ty < d.h; ty++) for (let tx = 0; tx < d.w; tx++) {
            const from = (d.tile + ty * SHEET_COLS + tx) * 64;
            out.set(H_.sec.sheet.subarray(from, from + 64), (ty * d.w + tx) * 64);
          }
          return d.w | d.h << 8;
        }
      }
      fail(0, `unknown editing call ${id}`);
    } catch (e) {
      if (!(e instanceof BErr)) throw e;
      shelfError = e.message;
      new Int32Array(x.memory.buffer, faultp, 1)[0] = 1;
      return 0;
    }
  }

  // A new instance per cart: fresh memory, nothing left behind by the last one.
  x = new WebAssembly.Instance(wasmModule, { bidule: { shelf_call: shelfCall } }).exports;
  const put = b => { const p = x.alloc(b.length); bytes(p, b.length).set(b); return [p, b.length]; };
  const text = () => fromUtf8(bytes(x.text_ptr(), x.text_len()));

  const img = sectionImages(host.sections());
  const none = [0, 0];
  const ok = x.compile(...put(utf8(src)), ...put(img.sprites), ...put(img.maps), ...put(img.sounds), ...(img.palette ? put(img.palette) : none));
  const diags = text().split("\n").map(l => DIAG.exec(l)).filter(Boolean);
  if (!ok) {
    const first = diags.find(d => d[2] === "error");
    const more = diags.filter(d => d[2] === "error").length - 1;
    fail(first ? +first[1] : 0, (first ? first[3] : text()) + (more > 0 ? ` (and ${more} more)` : ""));
  }
  const seed = host.seed ?? (Math.random() * 0xFFFFFFFF) >>> 0;
  if (!x.load(host.debug ? 1 : 0, seed)) fail(0, text());

  let statusSeen = "";
  function sync() {
    fb.set(bytes(x.fb_ptr(), W * H));
    pal565.set(new Uint16Array(x.memory.buffer, x.pal_ptr(), 256));
    const st = latin1(bytes(x.status_ptr(), x.status_len()));
    if (st !== statusSeen) { statusSeen = st; host.setStatusLine(st); }
    for (let ev = x.next_sound(); ev >= 0; ev = x.next_sound()) {
      const v = (ev >> 16) & 255;
      stopVoice(v);
      if (ev >> 24 === 1) {
        const sl = host.sections().sounds.slots[ev & 0xFFFF];
        if (sl && sl.bytes && sl.bytes.length >= 16) startSound(soundDesc(sl.bytes), v);
      }
    }
  }
  function* runHandler(handler, stage, frameNo, inp) {
    if (!x.start(handler, frameNo, inp)) return;
    for (;;) {
      const p = host.pointer;
      if (p) x.set_pointer(p.x, p.y, p.b);
      let step;
      try { step = x.run(); }
      catch (e) { const err = new BErr(`the console crashed: ${e.message}`, 0); err.stage = stage; throw err; }
      sync();
      if (step === STEP_DONE) return;
      if (step === STEP_LATE) { yield { type: "late", stage }; continue; }
      if (step === STEP_DIALOG) {
        const req = pendingDialog; pendingDialog = null;
        x.resume(req.finish(yield req));
        continue;
      }
      const err = new BErr(shelfError || text(), x.fault_line());
      err.stage = stage;
      throw err;
    }
  }

  let audioFault = false;
  return {
    warnings: diags.filter(d => d[2] === "warning").map(d => ({ line: +d[1], message: d[3] })),
    hasAudio: !!x.has_audio(),
    initTask: () => runHandler(HANDLER_INIT, "init", 0, 0),
    frameTask: function* (frameNo, inp) {
      const p = host.pointer || { x: 0, y: 0, b: 0 };
      x.begin_frame(inp, frameNo, p.x, p.y, p.b);
      yield* runHandler(HANDLER_UPDATE, "update", frameNo, inp);
      yield* runHandler(HANDLER_DRAW, "draw", frameNo, inp);
    },
    resumed() {},
    /** True while an editing session holds changes that were not saved with savecart. */
    dirty: () => edit.handles.some(h => h.dirty),
    audioSample(t) {
      if (audioFault) return 128;
      const v = x.audio_sample(t);
      if (v & 256) {
        audioFault = true;
        if (host.debug) host.audioFault(new BErr(text(), x.fault_line()));
      }
      return v & 255;
    },
  };
}
