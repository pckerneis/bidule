/* =====================================================================
   System menu (spec: System menu), cart switching and the home shelf.

   Start belongs to the system: the host takes it before the cart sees any input, so
   the menu opens even when the cart is stuck. While it is open the cart does not run
   (no update or draw), audio is frozen, and when it closes every button still held
   reads as released until it is let go. Entries: Resume, Restart, Volume, Carts,
   Project, Edit log, Quit.

   Switching carts keeps a stack of the carts that switched to the running one, so Quit
   goes back to the last of them, and to the shelf when there is none.
   Functions only: this file runs after host.js, whose state it uses.
   ===================================================================== */

const BTN_L = 1, BTN_R = 2, BTN_U = 4, BTN_D = 8, BTN_A = 16, BTN_B = 32;
const clip = (s, n) => (s.length > n ? s.slice(0, n - 1) + "." : s);

/** Start: open the system menu, or close it. Ignored while a cart's own dialog is open. */
function pressStart() {
  if (dialog || home) return;
  if (sysmenu) { closeSysMenu(); return; }
  openSysMenu();
}

function openSysMenu() {
  sysmenu = { page: "main", d: null, savedFb: fb.slice(), savedPal: pal565.slice(), prev: currentInput(), home: false };
  pauseAudio(true);
  resetPalette();
  smPage("main");
}

function closeSysMenu() {
  const m = sysmenu; if (!m) return;
  sysmenu = null;
  if (m.savedFb) { fb.set(m.savedFb); pal565.set(m.savedPal); }
  pauseAudio(false);
  // buttons held now belong to the menu: the cart sees them released until they are let go
  suppressMask = currentInput(); suppressMouse = true; syncCartPointer();
  flush();
}

/** The shelf screen: no cart runs. Quit with nothing to go back to lands here. */
function goHome() {
  stopAllVoices(); closeDialog(null, true); pauseAudio(false);
  vm = null; task = null; halted = null; late = false; frameNo = 0; launchStack.length = 0;
  transparent = 0; resetPalette(); fb.fill(0); statusLine = "";
  home = true;
  led.className = "led"; ledtxt.textContent = "STANDBY";
  setTitle("Bidule 01");
  setStatus("No cartridge is running. Choose one on the console, or from the shelf below.", false);
  sysmenu = { page: "carts", d: null, savedFb: null, savedPal: null, prev: currentInput(), home: true };
  smPage("carts");
}

// ---------- pages ----------

function smPage(page, extra) {
  const m = sysmenu, prev = m.d, same = prev && m.page === page;
  m.page = page;
  const sel = same ? prev.sel : 0, top = same ? prev.top : 0;
  const volumeRow = {
    id: "volume", label: "Volume",
    draw: (y, on) => { for (let i = 0; i < 10; i++) fillRect(138 + i * 8, y + 1, 6, 7, i < volume ? (on ? 7 : 10) : 5); }
  };
  switch (page) {
    case "main": {
      const proj = findCart(shelf.project);
      const items = [
        { id: "resume", label: "Resume" },
        { id: "restart", label: "Restart" },
        volumeRow,
        { id: "carts", label: "Carts", note: ">" },
        { id: "project", label: "Project", note: proj ? clip(cartTitle(proj), 14) : "none" },
        { id: "log", label: "Edit log", note: ">" },
        { id: "quit", label: launchStack.length ? "Quit (back)" : "Quit" }
      ];
      m.d = { title: clip(cartTitle(currentCart()).toUpperCase(), 33), items, sel, top, foot: "A choose   Start resume" };
      break;
    }
    case "carts": {
      const items = shelf.carts.map(c => ({
        id: "cart:" + c.id, label: cartTitle(c),
        note: !m.home && c.id === currentCart().id ? "running" : c.id === shelf.project ? "project" : ""
      }));
      const at = shelf.carts.findIndex(c => c.id === currentCart().id);
      m.d = { title: m.home ? "BIDULE 01 · CARTS" : "SWITCH TO A CART", items, sel: same ? sel : Math.max(0, at), top, foot: m.home ? "A run" : "A switch   B back" };
      break;
    }
    case "log": {
      const rows = shelf.journal.slice().reverse().map(e => {
        const t = new Date(e.at), hm = String(t.getHours()).padStart(2, "0") + ":" + String(t.getMinutes()).padStart(2, "0");
        return { id: "none", label: `${hm} ${clip(e.who, 10)}>${clip(e.target, 11)}` };
      });
      m.d = { title: "EDIT LOG · SAVES TO OTHER CARTS", items: rows, sel, top, empty: "Nothing saved yet", foot: "B back" };
      break;
    }
    case "confirm":
      m.d = {
        title: "UNSAVED CHANGES", info: ["Edits not saved with the cart", "will be lost if you go on."],
        items: [{ id: "yes", label: "Go on without saving" }, { id: "no", label: "Stay" }], sel: 1, top: 0, foot: "A choose   B stay"
      };
      m.then = extra;
      break;
  }
  renderList(m.d);
}

/** Runs `action`, after a warning when the running cart holds edits it has not saved. */
function smGuard(action) {
  if (!home && vm && !halted && vm.dirty && vm.dirty()) smPage("confirm", action);
  else action();
}

function smSwitch(id) {
  if (!home && id === currentCart().id) { closeSysMenu(); return; }
  smGuard(() => {
    if (!home) { launchStack.push(currentCart().id); if (launchStack.length > 8) launchStack.shift(); }
    switchCart(id, true);
  });
}

function smQuit() {
  smGuard(() => {
    const id = launchStack.pop();
    if (id && findCart(id)) switchCart(id, true); else goHome();
  });
}

function smActivate(it) {
  const m = sysmenu;
  if (m.page === "confirm") {
    if (it.id === "yes") { const go = m.then; go(); } else smPage("main");
    return;
  }
  if (m.page === "carts") { if (it.id.startsWith("cart:")) smSwitch(it.id.slice(5)); return; }
  if (m.page === "log") return;
  switch (it.id) {
    case "resume": closeSysMenu(); break;
    case "restart": smGuard(() => load()); break;
    case "volume": smVolume(1); break;
    case "carts": smPage("carts"); break;
    case "project": if (shelf.project) { shelf.project = null; persistShelf(); } smPage("main"); break;
    case "log": smPage("log"); break;
    case "quit": smQuit(); break;
  }
}

function smVolume(d) {
  volume = (volume + d + 11) % 11;          // A on the row wraps round; left and right stop at the ends
  applyGain(); store.set(VOL_KEY, String(volume));
  smPage("main");
}

function smBack() {
  const m = sysmenu;
  if (m.page === "main") closeSysMenu();
  else if (m.home) return;
  else smPage("main");
}

// ---------- input ----------

function sysmenuInput() {
  const m = sysmenu, inp = currentInput(), p = inp & ~m.prev; m.prev = inp;
  const d = m.d, n = Math.max(1, d.items.length);
  if (!p) return;
  const row = d.items[d.sel];
  if (p & BTN_U) d.sel = (d.sel + n - 1) % n;
  else if (p & BTN_D) d.sel = (d.sel + 1) % n;
  else if ((p & (BTN_L | BTN_R)) && m.page === "main" && row && row.id === "volume") {
    const v = Math.max(0, Math.min(10, volume + (p & BTN_R ? 1 : -1)));
    if (v !== volume) { volume = v; applyGain(); store.set(VOL_KEY, String(v)); }
  }
  else if (p & BTN_A) { if (row) smActivate(row); return; }
  else if (p & BTN_B) { smBack(); return; }
  if (sysmenu) renderList(sysmenu.d);
}

function sysmenuClick(x, y) {
  const m = sysmenu, d = m.d;
  const k = Math.floor((y - (d.y0 || LIST_Y) + 2) / ROW_H);
  if (x >= 14 && x < 226 && k >= 0 && k < (d.rows || VISIBLE) && d.items[d.top + k]) {
    d.sel = d.top + k;
    smActivate(d.items[d.sel]);
    if (sysmenu) renderList(sysmenu.d);
  }
}
