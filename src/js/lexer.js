/* =====================================================================
   Lexer
   ===================================================================== */
const KEYWORDS = new Set("var const fn audio fix record sprite enum switch if else while for break continue return".split(" "));
const OPS = ["<<=", ">>=", "&&", "||", "==", "!=", "<=", ">=", "<<", ">>", "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "++", "--",
             "+", "-", "*", "/", "%", "&", "|", "^", "~", "!", "<", ">", "=", "?", ":", ";", ",", ".", "(", ")", "[", "]", "{", "}"];

function lex(src, line0 = 1) {
  const toks = [], meta = {};
  let i = 0, line = line0, depth = 0;
  const at = j => src[j] || "";
  const push = (t, v) => toks.push({ t, v, line });
  while (i < src.length) {
    const c = src[i];
    if (c === "\n") { if (depth === 0) push("NL"); line++; i++; continue; }
    if (c === " " || c === "\t" || c === "\r") { i++; continue; }
    if (c === "/" && at(i + 1) === "/") {
      let j = src.indexOf("\n", i); if (j < 0) j = src.length;
      const m = /^\s*@(\w+)\s*(.*)$/.exec(src.slice(i + 2, j));
      if (m) meta[m[1]] = m[2].trim();
      i = j; continue;
    }
    if (/[0-9]/.test(c)) {
      let j = i;
      if (c === "0" && /[xXbB]/.test(at(i + 1))) {
        const hex = /[xX]/.test(at(i + 1)); j = i + 2;
        const re = hex ? /[0-9a-fA-F_]/ : /[01_]/;
        while (re.test(at(j))) j++;
        const digits = src.slice(i + 2, j).replace(/_/g, "");
        if (!digits) throw new BErr("number literal has no digits", line);
        const v = parseInt(digits, hex ? 16 : 2);
        if (v > 0xFFFFFFFF) throw new BErr("integer literal does not fit in 32 bits", line);
        push("int", v | 0);
      } else {
        while (/[0-9_]/.test(at(j))) j++;
        if (at(j) === "." && /[0-9]/.test(at(j + 1))) {
          j++; while (/[0-9_]/.test(at(j))) j++;
          const f = parseFloat(src.slice(i, j).replace(/_/g, ""));
          if (f >= 32768) throw new BErr("fix literal must fit in ±32,767.99", line);
          push("fix", Math.round(f * 65536));
        } else {
          const v = Number(src.slice(i, j).replace(/_/g, ""));
          if (v > 2147483647) throw new BErr("integer literal does not fit in 32 bits", line);
          push("int", v);
        }
      }
      i = j; continue;
    }
    if (c === "'") {
      let v, j;
      if (at(i + 1) === "\\") { const e = at(i + 2); if (e !== "'" && e !== "\\") throw new BErr(`unknown escape '\\${e}'`, line); v = e.charCodeAt(0); j = i + 3; }
      else { v = at(i + 1).charCodeAt(0); j = i + 2; }
      if (at(j) !== "'") throw new BErr("character literal must hold one character, like 'A'", line);
      push("int", v); i = j + 1; continue;
    }
    if (c === '"') {
      let j = i + 1, s = "";
      for (;;) {
        const d = at(j);
        if (d === "" || d === "\n") throw new BErr("string is missing its closing \"", line);
        if (d === '"') break;
        if (d === "\\") {
          const e = at(j + 1);
          if (e === '"') s += '"'; else if (e === "\\") s += "\\"; else if (e === "n") s += "\n";
          else throw new BErr(`unknown escape '\\${e}' in string`, line);
          j += 2; continue;
        }
        if (d < " " || d > "~") throw new BErr("strings may only contain printable ASCII characters", line);
        s += d; j++;
      }
      if (s.length > 255) throw new BErr("string literal is longer than 255 characters", line);
      push("str", s); i = j + 1; continue;
    }
    if (/[A-Za-z_]/.test(c)) {
      let j = i; while (/[A-Za-z0-9_]/.test(at(j))) j++;
      const w = src.slice(i, j);
      push(KEYWORDS.has(w) ? "kw" : "id", w); i = j; continue;
    }
    const op = OPS.find(o => src.startsWith(o, i));
    if (!op) throw new BErr(`unexpected character '${c}'`, line);
    if (op === "(" || op === "[") depth++;
    if ((op === ")" || op === "]") && depth > 0) depth--;
    push("op", op); i += op.length;
  }
  toks.push({ t: "EOF", v: "", line });
  return { toks, meta };
}
