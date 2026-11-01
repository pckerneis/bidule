/* =====================================================================
   Host: shelf, screen, loop, system dialogs, input, editor
   ===================================================================== */
const canvas = document.getElementById("screen");
const ctx = canvas.getContext("2d");
const img = ctx.createImageData(W, H);
const out32 = new Uint32Array(img.data.buffer);
const lut = new Uint32Array(256);
// System status line (spec › Editor support, Proposed): drawn over the bottom 9 rows
// at flush time, in the default palette, without touching the cart's framebuffer.
let statusLine = "";
const STATUS_Y = H - 9;
const rgba = w => { const [r, g, b] = rgbOf(w); return (255 << 24 | b << 16 | g << 8 | r) >>> 0; };
function flush() {
  for (let i = 0; i < 256; i++) lut[i] = rgba(pal565[i]);
  for (let i = 0; i < fb.length; i++) out32[i] = lut[fb[i]];
  if (statusLine && !dialog && !halted) {
    const bg = rgba(PAL_DEFAULT[0]), fg = rgba(PAL_DEFAULT[6]);
    out32.fill(bg, STATUS_Y * W, H * W);
    let x = 3;
    for (let k = 0; k < statusLine.length && x < W; k++, x += CW) {
      const ch = statusLine.charCodeAt(k), g = (ch >= 32 && ch <= 126 ? ch : 63) - 32;
      for (let r = 0; r < CH; r++) {
        const bits = GLYPHS[g * 9 + r];
        for (let b = 0; b < 5; b++) if (bits & (1 << b) && x + b < W) out32[(STATUS_Y + r) * W + x + b] = fg;
      }
    }
  }
  ctx.putImageData(img, 0, 0);
}

const SHELF_KEY = "bidule01.shelf.v1", OLD_KEY = "bidule01.cart.v2", MODE_KEY = "bidule01.mode";
const store = { get(k) { try { return localStorage.getItem(k); } catch (_) { return null; } }, set(k, v) { try { localStorage.setItem(k, v); return true; } catch (_) { return false; } } };

/* ---------- shelf ---------- */
let shelf = null;
const cartTitle = c => { const m = /^[ \t]*\/\/[ \t]*@title[ \t]+(.+)$/m.exec(c.source); return m ? m[1].trim() : "Untitled"; };
const newId = () => "cart-" + Date.now().toString(36) + Math.random().toString(36).slice(2, 6);
const findCart = id => shelf.carts.find(c => c.id === id);
const currentCart = () => findCart(shelf.current) || shelf.carts[0];
function retitle(src, title) {
  if (/^[ \t]*\/\/[ \t]*@title[ \t]+.+$/m.test(src)) return src.replace(/^([ \t]*\/\/[ \t]*@title[ \t]+).+$/m, `$1${title}`);
  return `// @title  ${title}\n` + src;
}
function loadShelf() {
  const raw = store.get(SHELF_KEY);
  if (raw) {
    try {
      const p = JSON.parse(raw);
      shelf = { current: p.current, seen: p.seen || ["spark", "peek", "blink"], carts: p.carts.map(c => ({ id: c.id, sampleId: c.sampleId || null, source: c.source, sections: unpackSections(c.sections) })) };
      for (const id of SAMPLE_IDS) if (!shelf.seen.includes(id)) { shelf.carts.unshift(makeSampleCart(id)); shelf.seen.push(id); }
      p.carts.forEach((pc, k) => {
        const c = shelf.carts.find(x => x.id === pc.id); if (!c || !c.sampleId) return;
        const fresh = makeSampleCart(c.sampleId); if (!fresh) return;
        const hNow = srcHash(c.source);
        const untouched = pc.pristine === hNow || (OLD_SAMPLE_HASHES[c.sampleId] || []).includes(hNow);
        if (untouched && c.source !== fresh.source) c.source = fresh.source;
        c.pristine = srcHash(fresh.source);
      });
      if (shelf.carts.length) return;
    } catch (_) { /* rebuild below */ }
  }
  shelf = { current: "sample-spark", seen: SAMPLE_IDS.slice(), carts: SAMPLE_IDS.map(makeSampleCart) };
  const old = store.get(OLD_KEY);
  if (old && !/@id\s+(spark-catcher|blink-v2)\b/.test(old)) {
    const c = { id: newId(), sampleId: null, source: old, sections: emptySections() };
    shelf.carts.push(c); shelf.current = c.id;
  }
}
let persistTimer = 0;
function persistShelf(now) {
  clearTimeout(persistTimer);
  const doIt = () => {
    const data = JSON.stringify({ current: shelf.current, seen: shelf.seen, carts: shelf.carts.map(c => ({ id: c.id, sampleId: c.sampleId, pristine: c.pristine, source: c.source, sections: packSections(c.sections) })) });
    if (!store.set(SHELF_KEY, data)) shelfNote("Couldn't save the shelf in this browser; changes last until you close the page.");
  };
  if (now) doIt(); else persistTimer = setTimeout(doIt, 500);
}

/* ---------- state ---------- */
let vm = null, halted = null, frameNo = 0, debugMode = store.get(MODE_KEY) !== "release";
let task = null;            // a paused or running cart task (generator)
let dialog = null;          // the open system dialog, if any
const pointer = { x: 0, y: 0, b: 0 };       // the real mouse
const cartPointer = { x: 0, y: 0, b: 0 };   // what carts see
// Buttons and mouse held when a system dialog closes belong to the dialog:
// carts don't see them until they are released.
let suppressMask = 0, suppressMouse = false;
function syncCartPointer() {
  cartPointer.x = pointer.x; cartPointer.y = pointer.y;
  if (!pointer.b) suppressMouse = false;
  cartPointer.b = suppressMouse ? 0 : pointer.b;
}
function cartInput() { const inp = currentInput(); suppressMask &= inp; return inp & ~suppressMask; }
const led = document.getElementById("led"), ledtxt = document.getElementById("ledtxt");
const statusEl = document.getElementById("status"), statusText = document.getElementById("statusText");
const bezelTitle = document.getElementById("bezelTitle"), cartName = document.getElementById("cartName");
const modeTag = document.getElementById("modeTag");
const code = document.getElementById("code"), gutter = document.getElementById("gutter");
const cartSel = document.getElementById("cartSel"), shelfNoteEl = document.getElementById("shelfNote");
const restoreBtn = document.getElementById("restoreBtn"), delCartBtn = document.getElementById("delCartBtn");
let errLine = 0;

function shelfNote(msg) { shelfNoteEl.textContent = msg; clearTimeout(shelfNote.t); shelfNote.t = setTimeout(() => shelfNoteEl.textContent = "", 6000); }
function setStatus(msg, isErr, line) {
  statusText.textContent = msg;
  statusEl.classList.toggle("err", !!isErr);
  const old = statusEl.querySelector("button"); if (old) old.remove();
  errLine = isErr && line ? line : 0;
  if (errLine) {
    const b = document.createElement("button"); b.type = "button"; b.textContent = `Go to line ${line}`;
    b.addEventListener("click", () => gotoLine(line));
    statusEl.appendChild(b);
  }
  renderGutter();
}
function gotoLine(n) {
  const lines = code.value.split("\n");
  let start = 0; for (let i = 0; i < n - 1 && i < lines.length; i++) start += lines[i].length + 1;
  const end = start + (lines[n - 1] || "").length;
  code.focus({ preventScroll: true });
  code.setSelectionRange(start, end);
  document.getElementById("codeWrap").scrollTop = Math.max(0, (n - 4) * parseFloat(getComputedStyle(code).lineHeight));
}
function wrapText(s, cols) {
  const out = []; let line = "";
  for (const w of s.split(/\s+/)) {
    if ((line + " " + w).trim().length > cols) { if (line) out.push(line); line = w; while (line.length > cols) { out.push(line.slice(0, cols)); line = line.slice(cols); } }
    else line = (line + " " + w).trim();
  }
  if (line) out.push(line);
  return out;
}
function halt(stage, e) {
  task = null; closeDialog(null, true);
  const line = e instanceof BErr ? e.line : 0;
  const msg = e instanceof BErr ? e.message : `internal error: ${e && e.message || e}`;
  halted = { stage, msg, line };
  led.className = "led err"; ledtxt.textContent = "HALT";
  setStatus(`${stage === "load" ? "Can't load" : `Stopped in ${stage}()`}${line ? `, line ${line}` : ""}: ${msg}`, true, line);
  resetPalette(); fb.fill(1);
  fillRect(0, 0, W, 13, 8);
  drawText(stage === "load" ? "CARTRIDGE DID NOT LOAD" : "CARTRIDGE STOPPED", 4, 3, 7);
  let y = 20;
  if (line) { drawText(`${stage === "load" ? "Source" : stage + "()"} · line ${line}`, 4, y, 10); y += 13; }
  for (const l of wrapText(msg, 38)) { if (y > 130) break; drawText(l, 4, y, 7); y += 10; }
  drawText("Fix the code, then Run or Reset", 4, 147, 6);
  flush();
}

/* ---------- running a cart ---------- */
function load() {
  const cart = currentCart();
  stopAllVoices(); closeDialog(null, true);
  halted = null; vm = null; task = null; frameNo = 0;
  bbSample = 0; bbLastT = -1; bbVal = 128;
  transparent = 0; resetPalette(); fb.fill(0); statusLine = "";
  if (cart.sections.palette) pal565.set(cart.sections.palette.words);
  led.className = "led on"; ledtxt.textContent = "POWER";
  setTitle(cartTitle(cart));
  try {
    vm = makeVM(cart.source, {
      debug: debugMode,
      pointer: cartPointer,
      sections: () => currentCart().sections,
      cartSections: id => findCart(id).sections,
      cartTitle: id => { const c = findCart(id); return c ? cartTitle(c) : ""; },
      setStatusLine: t => { statusLine = t.slice(0, 39); },
      audioFault: e => setStatus(`audio(t) fault${e.line ? `, line ${e.line}` : ""}: ${e.message} (audio is silent until the next reset)`, true, e.line)
    });
  } catch (e) { halt("load", e); return false; }
  task = vm.initTask();
  drive();
  if (!halted) setStatus(`Running “${cartTitle(cart)}”. Ctrl+Enter reloads after edits.`, false);
  return !halted;
}
function setTitle(t) { bezelTitle.textContent = t.toUpperCase(); cartName.textContent = t; }
function drive(answer) {
  try {
    const r = task.next(answer);
    if (r.done) { task = null; return true; }
    openDialog(r.value);
    return false;
  } catch (e) { halt(e && e.stage ? e.stage : "update", e); return false; }
}

/* ---------- system dialogs (drawn on the console screen) ---------- */
let savedFb = null, savedPal = null, dlgPrev = 0;
function openDialog(req) {
  savedFb = fb.slice(); savedPal = pal565.slice(); resetPalette();
  dlgPrev = currentInput();
  if (req.type === "pick") {
    dialog = { req, title: `CHOOSE A CART · ${req.kind.toUpperCase()}`, sel: 0, top: 0,
      items: shelf.carts.map(c => ({ label: cartTitle(c), value: c.id, note: `r${(c.sections[ASSET_KINDS[req.kind]] || { rev: 0 }).rev}` })),
      cancel: null, foot: "A choose   B cancel" };
  } else {
    const c = findCart(req.handle.pick.cartId);
    const t = c ? cartTitle(c) : "cart";
    const what = [...new Set(req.handles.map(h => h.secName))].join(", ").toUpperCase();
    dialog = { req, title: `SAVE ${what} TO ${t.toUpperCase()}`, sel: 0, top: 0,
      items: [{ label: `Overwrite ${t}`, value: "over" }, { label: "Save as a copy", value: "copy" }, { label: "Cancel", value: null }],
      cancel: null, foot: "A choose   B cancel" };
  }
  renderDialog();
}
function closeDialog(answer, silent) {
  if (!dialog) return;
  dialog = null;
  if (savedFb) { fb.set(savedFb); pal565.set(savedPal); savedFb = null; }
  if (silent) return;
  suppressMask = currentInput(); suppressMouse = true; syncCartPointer();
  vm.resumed();
  if (drive(answer) && !halted) { /* task finished; next frame runs normally */ }
  if (!halted) flush();
}
function finishDialog(value) {
  const req = dialog.req;
  let answer = value;
  if (req.type === "save") answer = value ? applySave(req, value) : 0;
  closeDialog(answer);
}
function applySave(req, mode) {
  const pick = req.handle.pick;
  let cart = findCart(pick.cartId);
  if (!cart) return 0;
  if (mode === "copy") {
    const copy = { id: newId(), sampleId: null, source: retitle(cart.source, cartTitle(cart) + " copy"), sections: cloneSections(cart.sections) };
    shelf.carts.splice(shelf.carts.indexOf(cart) + 1, 0, copy);
    cart = copy; pick.cartId = copy.id;      // later saves from this pick go to the copy
  }
  // one write per section: "tiles" and "sprites" handles share the sprites section
  const done = new Map();
  for (const Hd of req.handles) {
    if (done.has(Hd.secName)) { Hd.sec.rev = done.get(Hd.secName); Hd.dirty = false; continue; }
    const old = cart.sections[Hd.secName];
    const sec = cloneSection(Hd.secName, Hd.sec);
    if (!sec) continue;
    sec.rev = (old ? old.rev : 0) + 1; Hd.sec.rev = sec.rev; Hd.dirty = false;
    cart.sections[Hd.secName] = sec;
    done.set(Hd.secName, sec.rev);
  }
  persistShelf(true); renderShelf();
  shelfNote(`Saved ${[...done.keys()].join(" and ")} to “${cartTitle(cart)}”.`);
  return 1;
}
const ROW_H = 12, LIST_Y = 36, VISIBLE = 8;
function renderDialog() {
  const d = dialog;
  fillRect(10, 10, 220, 140, 0); strokeRect(10, 10, 220, 140, 6);
  fillRect(10, 10, 220, 14, 1);
  drawText(d.title.slice(0, 35), 16, 13, 10);
  if (d.sel < d.top) d.top = d.sel;
  if (d.sel >= d.top + VISIBLE) d.top = d.sel - VISIBLE + 1;
  for (let k = 0; k < VISIBLE; k++) {
    const i = d.top + k, it = d.items[i]; if (!it) break;
    const y = LIST_Y + k * ROW_H;
    if (i === d.sel) fillRect(14, y - 2, 212, ROW_H, 12);
    drawText(it.label.slice(0, 28), 20, y, i === d.sel ? 7 : 6);
    if (it.note) drawText(it.note, 222 - textWidth(it.note), y, i === d.sel ? 7 : 5);
  }
  if (!d.items.length) drawText("No carts on the shelf", 20, LIST_Y, 6);
  if (d.items.length > VISIBLE) drawText(`${d.sel + 1}/${d.items.length}`, 180, 136, 5);
  drawText(d.foot, 16, 136, 6);
  flush();
}
function dialogInput() {
  const inp = currentInput(), p = inp & ~dlgPrev; dlgPrev = inp;
  const d = dialog;
  if (p & 4) d.sel = (d.sel + d.items.length - 1) % Math.max(1, d.items.length);
  else if (p & 8) d.sel = (d.sel + 1) % Math.max(1, d.items.length);
  else if (p & 16) { if (d.items.length) finishDialog(d.items[d.sel].value); return; }
  else if (p & 32) { finishDialog(null); return; }
  if (p) renderDialog();
}
function dialogClick(x, y) {
  const d = dialog;
  const k = Math.floor((y - LIST_Y + 2) / ROW_H);
  if (x >= 14 && x < 226 && k >= 0 && k < VISIBLE && d.items[d.top + k]) { d.sel = d.top + k; finishDialog(d.items[d.sel].value); }
}

/* ---------- main loop: fixed 60 Hz ---------- */
const STEP = 1000 / 60;
let acc = 0, last = performance.now();
function currentInput() { let inp = 0; for (let i = 0; i < 6; i++) if (keyHeld[i] || touchHeld[i] > 0) inp |= 1 << i; return inp; }
function loop(now) {
  acc += Math.min(250, now - last); last = now;
  let ran = false;
  while (acc >= STEP) {
    acc -= STEP;
    if (dialog) { dialogInput(); continue; }
    if (!vm || halted || task) continue;
    syncCartPointer();
    task = vm.frameTask(frameNo, cartInput());
    frameNo++;
    if (drive()) ran = true; else break;
  }
  if (ran && !halted && !dialog) flush();
  requestAnimationFrame(loop);
}

/* ---------- integer scaling ---------- */
const bezel = document.getElementById("bezel");
function fitScreen() {
  const cs = getComputedStyle(bezel);
  const avail = bezel.clientWidth - parseFloat(cs.paddingLeft) - parseFloat(cs.paddingRight);
  const s = Math.floor(avail / W);
  const w = s >= 2 ? s * W : avail >= W * 1.25 ? avail : Math.min(avail, W);
  canvas.style.width = Math.max(120, Math.floor(w)) + "px";
}
try { new ResizeObserver(fitScreen).observe(bezel); } catch (_) { window.addEventListener("resize", fitScreen); }
fitScreen();

/* ---------- input ---------- */
const keyHeld = new Array(6).fill(false), touchHeld = new Array(6).fill(0);
const KEYMAP = { ArrowLeft: 0, KeyA: 0, ArrowRight: 1, KeyD: 1, ArrowUp: 2, KeyW: 2, ArrowDown: 3, KeyS: 3, KeyX: 4, KeyK: 4, KeyZ: 5, KeyJ: 5 };
const buttonsById = [[], [], [], [], [], []];
document.querySelectorAll("[data-b]").forEach(el => buttonsById[+el.dataset.b].push(el));
const paintButton = i => { const on = keyHeld[i] || touchHeld[i] > 0; buttonsById[i].forEach(el => el.classList.toggle("down-state", on)); };
const isTyping = el => el && (el.tagName === "TEXTAREA" || el.tagName === "INPUT" || el.tagName === "SELECT");

window.addEventListener("keydown", e => {
  if (isTyping(document.activeElement) || e.ctrlKey || e.metaKey || e.altKey) return;
  ensureAudio();
  if (e.code === "KeyR") { e.preventDefault(); if (!e.repeat) pressReset(); return; }
  if (e.code === "KeyM") { e.preventDefault(); if (!e.repeat) toggleSound(); return; }
  const i = KEYMAP[e.code]; if (i === undefined) return;
  e.preventDefault(); keyHeld[i] = true; paintButton(i);
});
window.addEventListener("keyup", e => { const i = KEYMAP[e.code]; if (i === undefined) return; keyHeld[i] = false; paintButton(i); });
window.addEventListener("blur", () => { keyHeld.fill(false); touchHeld.fill(0); pointer.b = 0; for (let i = 0; i < 6; i++) paintButton(i); });
window.addEventListener("pointerdown", ensureAudio, { capture: true });

document.querySelectorAll("[data-b]").forEach(el => {
  const i = +el.dataset.b, pointers = new Set();
  const release = ev => { if (!pointers.delete(ev.pointerId)) return; touchHeld[i] = Math.max(0, touchHeld[i] - 1); paintButton(i); };
  el.addEventListener("pointerdown", ev => {
    ev.preventDefault();
    try { el.releasePointerCapture(ev.pointerId); } catch (_) {}
    pointers.add(ev.pointerId); touchHeld[i]++; paintButton(i);
  });
  el.addEventListener("pointerup", release); el.addEventListener("pointercancel", release); el.addEventListener("pointerleave", release);
  el.addEventListener("keydown", ev => { if ((ev.code === "Space" || ev.code === "Enter") && !ev.repeat) { ev.preventDefault(); ev.stopPropagation(); touchHeld[i]++; paintButton(i); } });
  el.addEventListener("keyup", ev => { if (ev.code === "Space" || ev.code === "Enter") { ev.stopPropagation(); touchHeld[i] = Math.max(0, touchHeld[i] - 1); paintButton(i); } });
  el.addEventListener("contextmenu", ev => ev.preventDefault());
});

// pointer on the screen (spec: optional pointer, present in the emulator)
function screenXY(ev) {
  const r = canvas.getBoundingClientRect();
  return [Math.max(0, Math.min(W - 1, Math.floor((ev.clientX - r.left) / r.width * W))), Math.max(0, Math.min(H - 1, Math.floor((ev.clientY - r.top) / r.height * H)))];
}
const BTN_BITS = ev => (ev.buttons & 1 ? 1 : 0) | (ev.buttons & 2 ? 2 : 0) | (ev.buttons & 4 ? 4 : 0);
canvas.addEventListener("pointermove", ev => { [pointer.x, pointer.y] = screenXY(ev); pointer.b = BTN_BITS(ev); });
canvas.addEventListener("pointerdown", ev => {
  if (isTyping(document.activeElement)) document.activeElement.blur();
  try { canvas.setPointerCapture(ev.pointerId); } catch (_) {}
  [pointer.x, pointer.y] = screenXY(ev); pointer.b = BTN_BITS(ev) || 1;
  if (dialog) dialogClick(pointer.x, pointer.y);
  ev.preventDefault();
});
canvas.addEventListener("pointerup", ev => { [pointer.x, pointer.y] = screenXY(ev); pointer.b = BTN_BITS(ev); });
canvas.addEventListener("pointercancel", () => { pointer.b = 0; });
canvas.addEventListener("contextmenu", ev => ev.preventDefault());
canvas.style.touchAction = "none";

const resetEl = document.getElementById("resetBtn");
function pressReset() { resetEl.classList.add("down-state"); setTimeout(() => resetEl.classList.remove("down-state"), 120); load(); }
resetEl.addEventListener("click", pressReset);

const soundBtn = document.getElementById("soundBtn");
function toggleSound() {
  muted = !muted;
  if (master) master.gain.value = muted ? 0 : 1;
  soundBtn.setAttribute("aria-pressed", String(!muted));
  soundBtn.textContent = muted ? "Sound off" : "Sound on";
}
soundBtn.addEventListener("click", toggleSound);

/* ---------- shelf UI ---------- */
function renderShelf() {
  const cur = currentCart();
  cartSel.textContent = "";
  for (const c of shelf.carts) {
    const o = document.createElement("option"); o.value = c.id; o.textContent = cartTitle(c) + (c.sampleId ? " · sample" : "");
    cartSel.appendChild(o);
  }
  cartSel.value = cur.id;
  restoreBtn.hidden = !cur.sampleId;
  armDelete(false); armRestore(false);
}
function switchCart(id) {
  shelf.current = id; persistShelf();
  const c = currentCart();
  code.value = c.source; renderGutter();
  renderShelf(); load();
}
cartSel.addEventListener("change", () => switchCart(cartSel.value));
document.getElementById("newCartBtn").addEventListener("click", () => {
  const c = { id: newId(), sampleId: null, source: NEW_CART_SRC, sections: emptySections() };
  shelf.carts.push(c); persistShelf(true); switchCart(c.id);
  shelfNote("New cart created. Open Edit code to write it.");
});
document.getElementById("dupCartBtn").addEventListener("click", () => {
  const cur = currentCart();
  const c = { id: newId(), sampleId: null, source: retitle(cur.source, cartTitle(cur) + " copy"), sections: cloneSections(cur.sections) };
  shelf.carts.splice(shelf.carts.indexOf(cur) + 1, 0, c); persistShelf(true); switchCart(c.id);
});
let delArmed = false, restoreArmed = false;
function armDelete(on) { delArmed = on; delCartBtn.textContent = on ? "Delete for good?" : "Delete"; delCartBtn.classList.toggle("warn", on); }
function armRestore(on) { restoreArmed = on; restoreBtn.textContent = on ? "Replace with the original?" : "Restore sample"; restoreBtn.classList.toggle("warn", on); }
delCartBtn.addEventListener("click", () => {
  if (shelf.carts.length <= 1) { shelfNote("The shelf needs at least one cart."); return; }
  if (!delArmed) { armDelete(true); return; }
  const cur = currentCart(), i = shelf.carts.indexOf(cur);
  shelf.carts.splice(i, 1);
  persistShelf(true);
  shelfNote(`Deleted “${cartTitle(cur)}”.`);
  switchCart(shelf.carts[Math.max(0, i - 1)].id);
});
restoreBtn.addEventListener("click", () => {
  const cur = currentCart(); if (!cur.sampleId) return;
  if (!restoreArmed) { armRestore(true); return; }
  const fresh = makeSampleCart(cur.sampleId);
  cur.source = fresh.source; cur.sections = fresh.sections; cur.pristine = fresh.pristine;
  persistShelf(true); switchCart(cur.id);
  shelfNote("Sample restored.");
});

/* ---------- editor ---------- */
const editor = document.getElementById("editor"), toggle = document.getElementById("toggleCode");
const modeSel = document.getElementById("modeSel");
function renderGutter() {
  const n = code.value.split("\n").length;
  let html = "";
  for (let i = 1; i <= n; i++) html += i === errLine ? `<span class="hot">${i}</span>\n` : i + "\n";
  gutter.innerHTML = html;
  code.style.height = "auto"; code.style.height = code.scrollHeight + "px";
}
modeSel.value = debugMode ? "debug" : "release";
modeTag.textContent = debugMode ? "DEBUG" : "RELEASE";
toggle.addEventListener("click", () => {
  const open = editor.hidden;
  editor.hidden = !open;
  toggle.setAttribute("aria-expanded", String(open));
  toggle.textContent = open ? "Hide code" : "Edit code";
  if (open) { renderGutter(); code.focus({ preventScroll: true }); }
});
code.addEventListener("input", () => {
  const cur = currentCart(); cur.source = code.value; persistShelf();
  renderGutter(); armRestore(false);
});
code.addEventListener("keydown", e => {
  if (e.key === "Tab" && !e.shiftKey) { e.preventDefault(); code.setRangeText("  ", code.selectionStart, code.selectionEnd, "end"); code.dispatchEvent(new Event("input")); }
  else if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) { e.preventDefault(); run(); }
  else if (e.key === "Escape") code.blur();
});
function run() {
  ensureAudio();
  const cur = currentCart(); cur.source = code.value; persistShelf(true);
  renderShelf();
  if (load()) setStatus("Loaded. Press Esc or click the screen to play with the keyboard.", false);
}
document.getElementById("runBtn").addEventListener("click", run);
modeSel.addEventListener("change", () => {
  debugMode = modeSel.value === "debug"; store.set(MODE_KEY, modeSel.value);
  modeTag.textContent = debugMode ? "DEBUG" : "RELEASE"; run();
});

// palette reference
const palEl = document.getElementById("pal");
PAL_DEFAULT.forEach((w, i) => {
  const [r, g, b] = rgbOf(w);
  const s = document.createElement("span");
  s.style.background = `rgb(${r},${g},${b})`;
  s.style.color = r * .3 + g * .59 + b * .11 > 120 ? "#16141c" : "#f2f0f6";
  s.textContent = `${i} ${PAL_NAMES[i]}`;
  palEl.appendChild(s);
});

loadShelf();
code.value = currentCart().source;
renderShelf();
load();
flush();
requestAnimationFrame(loop);
