/* =====================================================================
   Parser → AST
   ===================================================================== */
const ASSIGN_OPS = new Set(["=", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<=", ">>="]);
const LEVELS = [["||"], ["&&"], ["|"], ["^"], ["&"], ["==", "!="], ["<", "<=", ">", ">="], ["<<", ">>"], ["+", "-"], ["*", "/", "%"]];
const HANDLERS = new Set(["init", "update", "draw"]);

class Parser {
  constructor(toks) { this.t = toks; this.p = 0; }
  peek(o = 0) { return this.t[Math.min(this.p + o, this.t.length - 1)]; }
  is(v, o = 0) { const k = this.peek(o); return (k.t === "op" || k.t === "kw") && k.v === v; }
  isT(t, o = 0) { return this.peek(o).t === t; }
  eat(v) { if (this.is(v)) { this.p++; return true; } return false; }
  desc(k) { return k.t === "EOF" ? "the end of the file" : k.t === "NL" ? "the end of the line" : k.t === "str" ? "a string" : `'${k.v}'`; }
  fail(msg) { const k = this.peek(); throw new BErr(`${msg}, found ${this.desc(k)}`, k.line); }
  expect(v, why) { if (!this.eat(v)) this.fail(`expected '${v}'${why ? " " + why : ""}`); }
  ident(what) { const k = this.peek(); if (k.t !== "id") this.fail(`expected ${what || "a name"}`); this.p++; return k.v; }
  nl() { while (this.isT("NL")) this.p++; }

  program() {
    const P = { consts: [], vars: [], avars: [], records: [], enums: [], sprites: [], fns: [], handlers: {} };
    this.nl();
    while (!this.isT("EOF")) {
      const tok = this.peek(), line = tok.line;
      if (this.eat("const")) {
        const n = this.ident("a constant name"); const sh = this.shape();
        this.expect("=", "after the constant name"); P.consts.push({ n, sh, init: this.init(), line });
      } else if (this.eat("var")) {
        const n = this.ident("a variable name"); const sh = this.shape();
        let init = null; if (this.eat("=")) init = this.init();
        P.vars.push({ n, sh, init, line });
      } else if (this.is("audio") && this.is("var", 1)) {
        this.p += 2; const n = this.ident("a variable name");
        let ty = "int"; if (this.eat(":")) ty = this.typeName();
        this.expect("=", "(audio variables need a starting value)");
        P.avars.push({ n, ty, e: this.expr(), line });
      } else if (this.eat("record")) {
        const n = this.ident("a record name"); this.expect("{"); this.nl();
        const fields = [];
        while (!this.is("}")) {
          const f = this.ident("a field name"); let ty = "int", dims = null;
          if (this.eat(":")) { ty = this.typeName(); if (this.is("[")) dims = this.dims(); }
          fields.push({ n: f, ty, dims, line: this.peek().line });
          this.eat(","); this.nl();
        }
        this.p++; P.records.push({ n, fields, line });
      } else if (this.eat("enum")) {
        const n = this.ident("an enum name"); this.expect("{"); this.nl();
        const members = [];
        while (!this.is("}")) {
          const m = this.ident("an enum member"); let e = null;
          if (this.eat("=")) e = this.expr();
          members.push({ n: m, e }); this.eat(","); this.nl();
        }
        this.p++; P.enums.push({ n, members, line });
      } else if (this.eat("sprite")) {
        const n = this.ident("a sprite name"); this.expect("=");
        const tile = this.expr(); let w = null, h = null;
        if (this.eat(",")) { w = this.expr(); this.expect(","); h = this.expr(); }
        P.sprites.push({ n, tile, w, h, line });
      } else if (this.eat("fn")) {
        const n = this.ident("a function name"); const params = this.params();
        let ret = "int"; if (this.eat(":")) ret = this.typeName();
        P.fns.push({ n, params, ret, body: this.block(), line });
      } else if ((tok.t === "id" && HANDLERS.has(tok.v) || this.is("audio")) && this.is("(", 1)) {
        this.p++;
        if (P.handlers[tok.v]) throw new BErr(`${tok.v}() is defined twice`, line);
        P.handlers[tok.v] = { n: tok.v, params: this.params(), body: this.block(), line };
      } else if (tok.t === "id" && this.is("(", 1)) {
        throw new BErr(`'${tok.v}(...)' at top level: functions start with 'fn' (fn ${tok.v}(...) { })`, line);
      } else this.fail("expected var, const, fn, record, enum, sprite or a handler (init, update, draw, audio)");
      if (!this.isT("EOF")) { if (!this.isT("NL")) this.fail("expected the end of the line"); this.nl(); }
    }
    return P;
  }
  typeName() { if (this.eat("fix")) return "fix"; return this.ident("a type (int, fix or a record name)"); }
  dims() {
    const d = [];
    while (this.eat("[")) { if (this.eat("]")) d.push(null); else { d.push(this.expr()); this.expect("]"); } }
    return d;
  }
  shape() {
    if (this.is("[")) return { ty: "int", dims: this.dims() };
    if (this.eat(":")) { const ty = this.typeName(); return { ty, dims: this.is("[") ? this.dims() : null }; }
    return { ty: "int", dims: null, implicit: true };
  }
  init() {
    if (this.is("{")) {
      const line = this.next().line; this.nl(); const items = [];
      while (!this.is("}")) {
        if (this.isT("id") && this.is(":", 1)) { const named = this.next().v; this.p++; items.push({ k: "named", named, v: this.init(), line }); }
        else items.push(this.init());
        this.nl(); if (!this.eat(",")) break; this.nl();
      }
      this.nl(); this.expect("}", "to close the list"); return { k: "list", items, line };
    }
    return this.expr();
  }
  next() { return this.t[this.p++]; }
  params() {
    this.expect("("); const ps = [];
    if (!this.is(")")) do {
      const n = this.ident("a parameter name"); const sh = this.shape();
      ps.push({ n, ty: sh.ty, dims: sh.dims });
    } while (this.eat(","));
    this.expect(")"); return ps;
  }
  block() {
    this.expect("{", "to open a block"); this.nl(); const body = [];
    while (!this.is("}")) {
      if (this.isT("EOF")) this.fail("missing '}' to close a block");
      body.push(this.stmt());
      if (this.is("}")) break;
      if (!this.isT("NL")) this.fail("expected the end of the line after this statement");
      this.nl();
    }
    this.p++; return body;
  }
  stmt() {
    const tok = this.peek(), line = tok.line;
    if (this.eat("var")) {
      const n = this.ident("a variable name"); let ty = "int";
      if (this.is("[")) throw new BErr("local arrays are not supported; declare arrays at top level", line);
      if (this.eat(":")) { ty = this.typeName(); if (this.is("[")) throw new BErr("local arrays are not supported; declare arrays at top level", line); }
      let e = null; if (this.eat("=")) e = this.expr();
      return { k: "local", n, ty, e, line };
    }
    if (this.eat("if")) return this.ifRest(line);
    if (this.eat("while")) { this.expect("("); const c = this.expr(); this.expect(")"); return { k: "while", c, body: this.block(), line }; }
    if (this.eat("for")) {
      this.expect("("); let init = null, c = null, step = null;
      if (!this.is(";")) init = this.is("var") ? this.stmt() : this.simple();
      this.expect(";"); if (!this.is(";")) c = this.expr();
      this.expect(";"); if (!this.is(")")) step = this.simple();
      this.expect(")"); return { k: "for", init, c, step, body: this.block(), line };
    }
    if (this.eat("switch")) {
      this.expect("("); const e = this.expr(); this.expect(")"); this.expect("{"); this.nl();
      const cases = []; let els = null;
      while (!this.is("}")) {
        if (this.isT("EOF")) this.fail("missing '}' to close the switch");
        if (this.eat("else")) els = this.block();
        else { const vals = [this.expr()]; while (this.eat(",")) vals.push(this.expr()); cases.push({ vals, body: this.block(), line: this.peek().line }); }
        this.nl();
      }
      this.p++; return { k: "switch", e, cases, els, line };
    }
    if (this.eat("break")) return { k: "break", line };
    if (this.eat("continue")) return { k: "continue", line };
    if (this.eat("return")) { let e = null; if (!this.isT("NL") && !this.is("}")) e = this.expr(); return { k: "return", e, line }; }
    return this.simple();
  }
  ifRest(line) {
    this.expect("(", "after if"); const c = this.expr(); this.expect(")"); const then = this.block();
    let q = this.p; while (this.t[q] && this.t[q].t === "NL") q++;
    let els = null;
    if (this.t[q] && this.t[q].t === "kw" && this.t[q].v === "else") {
      this.p = q + 1;
      if (this.is("if")) { const l = this.next().line; els = [this.ifRest(l)]; } else els = this.block();
    }
    return { k: "if", c, then, els, line };
  }
  simple() {
    const tok = this.peek(), line = tok.line;
    if (tok.t !== "id") this.fail("expected a statement");
    const lv = this.postfix();
    if (lv.k === "call") return { k: "expr", e: lv, line };
    const op = this.peek();
    if (op.t === "op" && ASSIGN_OPS.has(op.v)) { this.p++; return { k: "assign", lv, op: op.v, e: this.expr(), line }; }
    if (this.eat("++")) return { k: "inc", lv, d: 1, line };
    if (this.eat("--")) return { k: "inc", lv, d: -1, line };
    if (op.t === "op" && op.v === "==") this.fail("expected '=' to assign ('==' compares)");
    this.fail("expected =, +=, ++, -- or a function call");
  }
  expr() {
    const c = this.bin(0);
    if (this.eat("?")) { const a = this.expr(); this.expect(":", "in a ? b : c"); const b = this.expr(); return { k: "tern", c, a, b, line: c.line }; }
    return c;
  }
  bin(l) {
    if (l === LEVELS.length) return this.unary();
    let a = this.bin(l + 1);
    for (;;) {
      const t = this.peek();
      if (t.t === "op" && LEVELS[l].includes(t.v)) { this.p++; a = { k: "bin", op: t.v, a, b: this.bin(l + 1), line: t.line }; }
      else return a;
    }
  }
  unary() {
    const t = this.peek();
    if (t.t === "op" && (t.v === "-" || t.v === "!" || t.v === "~")) { this.p++; return { k: "un", op: t.v, a: this.unary(), line: t.line }; }
    return this.postfix();
  }
  postfix() {
    const t = this.next(); let n;
    if (t.t === "int") n = { k: "num", v: t.v, line: t.line };
    else if (t.t === "fix") n = { k: "fixlit", r: t.v, line: t.line };
    else if (t.t === "str") n = this.template(t);
    else if (t.t === "op" && t.v === "(") { n = this.expr(); this.expect(")"); }
    else if (t.t === "id") {
      if (this.eat("(")) {
        const args = [];
        if (!this.is(")")) do { args.push(this.expr()); } while (this.eat(","));
        this.expect(")", `to close the call to ${t.v}`);
        n = { k: "call", n: t.v, args, line: t.line };
      } else n = { k: "name", n: t.v, line: t.line };
    } else { this.p--; this.fail("expected a value"); }
    for (;;) {
      if (this.eat("[")) { const i = this.expr(); this.expect("]"); n = { k: "index", o: n, i, line: t.line }; }
      else if (this.is(".")) { this.p++; n = { k: "field", o: n, f: this.ident("a field name"), line: t.line }; }
      else return n;
    }
  }
  template(t) {
    const s = t.v, parts = []; let buf = "", i = 0;
    while (i < s.length) {
      const c = s[i];
      if (c === "{" && s[i + 1] === "{") { buf += "{"; i += 2; continue; }
      if (c === "}" && s[i + 1] === "}") { buf += "}"; i += 2; continue; }
      if (c === "{") {
        const j = s.indexOf("}", i);
        if (j < 0) throw new BErr("'{' in a string needs a closing '}' (write '{{' for a literal brace)", t.line);
        let inner = s.slice(i + 1, j), fmt = null;
        const k = inner.lastIndexOf(":");
        if (k >= 0 && (inner.match(/:/g) || []).length > (inner.match(/\?/g) || []).length) { fmt = inner.slice(k + 1).trim(); inner = inner.slice(0, k); }
        if (!inner.trim()) throw new BErr("empty {} in a string", t.line);
        if (buf) parts.push(buf); buf = "";
        const sub = new Parser(lex(inner, t.line).toks); const e = sub.expr();
        if (!sub.isT("EOF")) sub.fail("unexpected text inside {…}");
        let f = null;
        if (fmt !== null) {
          const m = /^(0)?(\d+)?(?:\.(\d+))?([xc])?$/.exec(fmt);
          if (!m || !fmt) throw new BErr(`unknown format '${fmt}' (try 4, 04, .1, x or c)`, t.line);
          f = { zero: !!m[1], width: m[2] ? +m[2] : 0, dec: m[3] != null ? +m[3] : null, kind: m[4] || null };
        }
        parts.push({ e, f }); i = j + 1; continue;
      }
      if (c === "}") throw new BErr("single '}' in a string; write '}}' for a literal brace", t.line);
      buf += c; i++;
    }
    if (buf || !parts.length) parts.push(buf);
    return { k: "str", parts, line: t.line, plain: parts.every(p => typeof p === "string") };
  }
}
