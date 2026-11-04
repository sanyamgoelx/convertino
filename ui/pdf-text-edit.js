// Convertino: edit the text that is already inside a PDF (prototype, 27 Oct 2026).
//
// How it works, in one paragraph: each page's content stream is read operator by
// operator (a small PDF text interpreter: q/Q, cm, BT/ET, Tf, Tm, Td, TD, T*, TL, Tc,
// Tw, Tz, Ts, Tr, Tj, TJ, ' and "). Every text-showing operator becomes a "piece"
// with its position, font, size and real Unicode text. Pieces on one line in the
// same font and size are joined into a "run" (what the user clicks). Editing a run
// rewrites the bytes of the operators it came from:
//
//     [ <new text in the same font> (number that moves back by the new width)
//       (number that moves forward by the old width) ] TJ
//
// so the old glyphs are really gone from the file (not just covered), the new ones
// are drawn with the page's own font, size, colour and spacing, and everything
// after the run stays exactly where it was. If the page's font (usually a subset)
// lacks a character the new text needs, the run switches to a fallback font for the
// new text only (Tf before, the original Tf after) and the result says so.
//
// Not editable (each run says why): text in Type3 fonts, vertical (Identity-V)
// text, invisible text (Tr 3: the OCR layer of a scanned page), text inside form
// XObjects, CID fonts with an encoding other than Identity-H.
//
// Works with the same pdf-lib build the editor already ships (window.PDFLib).

const WS = new Set([0, 9, 10, 12, 13, 32]);
const DELIM = new Set([40, 41, 60, 62, 91, 93, 123, 125, 47, 37]);

// ---------- content stream tokenizer ----------

function tokenize(s) {
  // s: latin1 string. Returns ops: { op, args, start, end }.
  const ops = [];
  let i = 0, args = [], argStart = -1;
  const n = s.length;
  const skipWs = () => {
    for (;;) {
      while (i < n && WS.has(s.charCodeAt(i))) i++;
      if (s[i] === "%") { while (i < n && s[i] !== "\n" && s[i] !== "\r") i++; continue; }
      return;
    }
  };
  const readString = () => {
    // at "("
    let depth = 0, out = [];
    i++;
    for (; i < n; i++) {
      const c = s[i];
      if (c === "\\") {
        const d = s[++i];
        const map = { n: 10, r: 13, t: 9, b: 8, f: 12, "(": 40, ")": 41, "\\": 92 };
        if (d in map) out.push(map[d]);
        else if (d >= "0" && d <= "7") {
          let o = d;
          if (s[i + 1] >= "0" && s[i + 1] <= "7") { o += s[++i]; if (s[i + 1] >= "0" && s[i + 1] <= "7") o += s[++i]; }
          out.push(parseInt(o, 8) & 255);
        } else if (d === "\r") { if (s[i + 1] === "\n") i++; }
        else if (d === "\n") { /* line continuation */ }
        else out.push(d.charCodeAt(0));
      } else if (c === "(") { depth++; out.push(40); }
      else if (c === ")") { if (depth === 0) { i++; break; } depth--; out.push(41); }
      else out.push(c.charCodeAt(0));
    }
    return { t: "str", bytes: out };
  };
  const readHex = () => {
    i++;
    let h = "";
    while (i < n && s[i] !== ">") { if (!WS.has(s.charCodeAt(i))) h += s[i]; i++; }
    i++;
    if (h.length % 2) h += "0";
    const out = [];
    for (let k = 0; k < h.length; k += 2) out.push(parseInt(h.substr(k, 2), 16));
    return { t: "str", bytes: out, hex: true };
  };
  const readValue = () => {
    skipWs();
    const c = s[i];
    if (c === "(") return readString();
    if (c === "<") {
      if (s[i + 1] === "<") {
        i += 2;
        const d = {};
        for (;;) {
          skipWs();
          if (s[i] === ">" && s[i + 1] === ">") { i += 2; break; }
          if (i >= n) break;
          const k = readValue();
          const v = readValue();
          if (k && k.t === "name") d[k.v] = v;
        }
        return { t: "dict", v: d };
      }
      return readHex();
    }
    if (c === "[") {
      i++;
      const arr = [];
      for (;;) {
        skipWs();
        if (s[i] === "]") { i++; break; }
        if (i >= n) break;
        arr.push(readValue());
      }
      return { t: "arr", v: arr };
    }
    if (c === "/") {
      i++;
      let name = "";
      while (i < n && !WS.has(s.charCodeAt(i)) && !DELIM.has(s.charCodeAt(i))) name += s[i++];
      name = name.replace(/#([0-9a-fA-F]{2})/g, (m, h) => String.fromCharCode(parseInt(h, 16)));
      return { t: "name", v: name };
    }
    let w = "";
    while (i < n && !WS.has(s.charCodeAt(i)) && !DELIM.has(s.charCodeAt(i))) w += s[i++];
    if (w === "") { i++; return null; } // stray delimiter
    if (/^[+-]?(\d+\.?\d*|\.\d+)$/.test(w)) return { t: "num", v: parseFloat(w) };
    if (w === "true" || w === "false") return { t: "bool", v: w === "true" };
    if (w === "null") return { t: "null" };
    return { t: "op", v: w };
  };

  while (i < n) {
    skipWs();
    if (i >= n) break;
    const at = i;
    const v = readValue();
    if (!v) continue;
    if (v.t === "op") {
      const start = argStart >= 0 ? argStart : at;
      if (v.v === "BI") {
        // Inline image: dictionary up to ID, then binary data up to EI.
        const idAt = s.indexOf("ID", i);
        let j = idAt + 3;
        let eiAt = -1;
        while ((j = s.indexOf("EI", j)) >= 0) {
          const before = s.charCodeAt(j - 1), after = s.charCodeAt(j + 2);
          if (WS.has(before) && (j + 2 >= n || WS.has(after))) { eiAt = j; break; }
          j += 2;
        }
        i = eiAt >= 0 ? eiAt + 2 : n;
        ops.push({ op: "BI", args: [], start, end: i });
      } else {
        ops.push({ op: v.v, args, start, end: i });
      }
      args = [];
      argStart = -1;
    } else {
      if (argStart < 0) argStart = at;
      args.push(v);
    }
  }
  return ops;
}

// ---------- fonts ----------

const WIN_ANSI = (() => {
  const m = {};
  const td = new TextDecoder("windows-1252");
  for (let c = 32; c < 256; c++) { const u = td.decode(new Uint8Array([c])); if (u !== "\ufffd") m[c] = u; }
  return m;
})();
const MAC_ROMAN = (() => {
  const m = {};
  let td = null;
  try { td = new TextDecoder("macintosh"); } catch (e) { return WIN_ANSI; }
  for (let c = 32; c < 256; c++) m[c] = td.decode(new Uint8Array([c]));
  return m;
})();
// Glyph names (Adobe Glyph List, the common Latin part) for /Differences.
const AGL = (() => {
  const m = {
    space: " ", exclam: "!", quotedbl: '"', numbersign: "#", dollar: "$", percent: "%", ampersand: "&", quotesingle: "'",
    quoteright: "’", quoteleft: "‘", parenleft: "(", parenright: ")", asterisk: "*", plus: "+", comma: ",", hyphen: "-",
    minus: "−", period: ".", slash: "/", colon: ":", semicolon: ";", less: "<", equal: "=", greater: ">", question: "?", at: "@",
    bracketleft: "[", backslash: "\\", bracketright: "]", asciicircum: "^", underscore: "_", grave: "`", braceleft: "{", bar: "|",
    braceright: "}", asciitilde: "~", endash: "–", emdash: "—", bullet: "•", ellipsis: "…", quotedblleft: "“",
    quotedblright: "”", quotesinglbase: "‚", quotedblbase: "„", dagger: "†", daggerdbl: "‡", fi: "fi", fl: "fl",
    ff: "ff", ffi: "ffi", ffl: "ffl", copyright: "©", registered: "®", trademark: "™", degree: "°", section: "§",
    paragraph: "¶", sterling: "£", Euro: "€", yen: "¥", cent: "¢", periodcentered: "·", multiply: "×",
    divide: "÷", plusminus: "±", dotlessi: "ı", nbspace: " ", rupee: "₹", rupeeindian: "₹",
  };
  "zero one two three four five six seven eight nine".split(" ").forEach((w, k) => { m[w] = String(k); });
  for (let c = 65; c <= 90; c++) { m[String.fromCharCode(c)] = String.fromCharCode(c); m[String.fromCharCode(c + 32)] = String.fromCharCode(c + 32); }
  const acc = { acute: "́", grave: "̀", circumflex: "̂", dieresis: "̈", tilde: "̃", ring: "̊", cedilla: "̧", caron: "̌" };
  for (const base of "AEIOUYaeiouyNnCcSsZz") for (const [k, mark] of Object.entries(acc)) {
    const u = (base + mark).normalize("NFC");
    if (u.length === 1) m[base + k] = u;
  }
  m.germandbls = "ß"; m.AE = "Æ"; m.ae = "æ"; m.Oslash = "Ø"; m.oslash = "ø"; m.OE = "Œ"; m.oe = "œ";
  return m;
})();
const glyphToUnicode = (name) => {
  if (AGL[name]) return AGL[name];
  let mm = /^uni([0-9A-Fa-f]{4,})$/.exec(name);
  if (mm) return String.fromCodePoint(...mm[1].match(/.{4}/g).map((h) => parseInt(h, 16)));
  mm = /^u([0-9A-Fa-f]{4,6})$/.exec(name);
  if (mm) return String.fromCodePoint(parseInt(mm[1], 16));
  const base = name.split(".")[0];
  if (base !== name && AGL[base]) return AGL[base];
  return null;
};

function parseCMap(text) {
  // ToUnicode: code (as hex string, keeps its byte length) -> unicode string.
  const map = new Map();
  let bytes = 1;
  const hexToStr = (h) => {
    const units = [];
    for (let k = 0; k + 4 <= h.length; k += 4) units.push(parseInt(h.substr(k, 4), 16));
    if (h.length === 2) units.push(parseInt(h, 16));
    return String.fromCharCode(...units);
  };
  const cs = /begincodespacerange([\s\S]*?)endcodespacerange/.exec(text);
  if (cs) { const h = /<([0-9a-fA-F]+)>/.exec(cs[1]); if (h) bytes = h[1].length / 2; }
  for (const blk of text.matchAll(/beginbfchar([\s\S]*?)endbfchar/g)) {
    for (const m of blk[1].matchAll(/<([0-9a-fA-F]+)>\s*<([0-9a-fA-F]*)>/g)) map.set(parseInt(m[1], 16), hexToStr(m[2]));
  }
  for (const blk of text.matchAll(/beginbfrange([\s\S]*?)endbfrange/g)) {
    for (const m of blk[1].matchAll(/<([0-9a-fA-F]+)>\s*<([0-9a-fA-F]+)>\s*(<[0-9a-fA-F]*>|\[[^\]]*\])/g)) {
      const lo = parseInt(m[1], 16), hi = parseInt(m[2], 16);
      if (hi - lo > 65535) continue;
      if (m[3][0] === "[") {
        const list = [...m[3].matchAll(/<([0-9a-fA-F]*)>/g)].map((x) => x[1]);
        for (let c = lo; c <= hi && c - lo < list.length; c++) map.set(c, hexToStr(list[c - lo]));
      } else {
        const h = m[3].slice(1, -1);
        const base = hexToStr(h);
        for (let c = lo; c <= hi; c++) {
          const last = base.charCodeAt(base.length - 1) + (c - lo);
          map.set(c, base.slice(0, -1) + String.fromCharCode(last));
        }
      }
    }
  }
  return { map, bytes };
}

export function createTextEditor(PDFLib, pdfDoc, opts = {}) {
  const { PDFName, PDFArray, PDFDict, PDFNumber, PDFRawStream, PDFRef, PDFStream, decodePDFRawStream, PDFString, PDFHexString } = PDFLib;
  const ctx = pdfDoc.context;
  const deref = (o) => (o instanceof PDFRef ? ctx.lookup(o) : o);
  const name = (o) => { o = deref(o); return o instanceof PDFName ? o.decodeText() : null; };
  const num = (o) => { o = deref(o); return o instanceof PDFNumber ? o.asNumber() : null; };
  const get = (dict, key) => (dict instanceof PDFDict ? deref(dict.get(PDFName.of(key))) : undefined);
  const arr = (o) => { o = deref(o); return o instanceof PDFArray ? o.asArray().map(deref) : null; };
  const streamText = (st) => {
    st = deref(st);
    if (!st) return "";
    let bytes;
    if (st instanceof PDFRawStream) bytes = decodePDFRawStream(st).decode();
    else if (st.getContents) bytes = st.getContents();
    else return "";
    let out = "";
    for (let k = 0; k < bytes.length; k += 8192) out += String.fromCharCode.apply(null, bytes.subarray(k, k + 8192));
    return out;
  };

  const fontCache = new Map(); // font dict -> info
  function fontInfo(fdict) {
    if (fontCache.has(fdict)) return fontCache.get(fdict);
    const f = { dict: fdict, subtype: name(get(fdict, "Subtype")), base: name(get(fdict, "BaseFont")) || "", twoByte: false, problem: null };
    f.subset = /^[A-Z]{6}\+/.test(f.base);
    f.display = f.base.replace(/^[A-Z]{6}\+/, "");
    const tu = get(fdict, "ToUnicode");
    f.toUni = tu && !(tu instanceof PDFName) ? parseCMap(streamText(tu)).map : null;
    f.codeToUni = new Map();
    f.width = new Map();
    f.dw = 0;
    let desc;
    if (f.subtype === "Type0") {
      const enc = name(get(fdict, "Encoding"));
      f.twoByte = true;
      if (enc !== "Identity-H") f.problem = enc === "Identity-V" ? "Vertical text can't be changed yet" : "Uses a font encoding Convertino can't rewrite yet";
      const df = arr(get(fdict, "DescendantFonts"));
      const cid = df && df[0];
      desc = get(cid, "FontDescriptor");
      f.dw = num(get(cid, "DW")) ?? 1000;
      const w = arr(get(cid, "W")) || [];
      for (let k = 0; k < w.length;) {
        const first = num(w[k]);
        const next = w[k + 1];
        if (next instanceof PDFArray) {
          next.asArray().forEach((x, j) => f.width.set(first + j, num(x)));
          k += 2;
        } else {
          const last = num(next), wd = num(w[k + 2]);
          for (let c = first; c <= last; c++) f.width.set(c, wd);
          k += 3;
        }
      }
      if (f.toUni) for (const [c, u] of f.toUni) f.codeToUni.set(c, u);
    } else {
      if (f.subtype === "Type3") f.problem = "Drawn as shapes, not as typed text";
      desc = get(fdict, "FontDescriptor");
      const first = num(get(fdict, "FirstChar")) ?? 0;
      const widths = arr(get(fdict, "Widths"));
      if (widths) widths.forEach((x, j) => f.width.set(first + j, num(x)));
      f.dw = num(get(desc, "MissingWidth")) ?? 0;
      // Built-in encoding, then /Differences, then ToUnicode on top (most reliable).
      const encObj = get(fdict, "Encoding");
      let baseEnc = WIN_ANSI;
      let diffs = null;
      if (encObj instanceof PDFName) { if (encObj.decodeText() === "MacRomanEncoding") baseEnc = MAC_ROMAN; }
      else if (encObj instanceof PDFDict) {
        if (name(get(encObj, "BaseEncoding")) === "MacRomanEncoding") baseEnc = MAC_ROMAN;
        diffs = arr(get(encObj, "Differences"));
      }
      for (const [c, u] of Object.entries(baseEnc)) f.codeToUni.set(Number(c), u);
      if (diffs) {
        let code = 0;
        for (const d of diffs) {
          if (d instanceof PDFNumber) code = d.asNumber();
          else if (d instanceof PDFName) { const u = glyphToUnicode(d.decodeText()); if (u) f.codeToUni.set(code, u); else f.codeToUni.delete(code); code++; }
        }
      }
      if (f.toUni) for (const [c, u] of f.toUni) f.codeToUni.set(c, u);
      if (!widths) {
        // Standard 14 font without widths: borrow pdf-lib's metrics (AFM files built in).
        const std = Object.values(PDFLib.StandardFonts).find((s) => s === f.base);
        if (std) {
          const em = PDFLib.StandardFontEmbedder.for(std);
          f.stdWidths = new Map();
          for (const [c, u] of f.codeToUni) { try { const w = em.widthOfTextAtSize(u, 1000); if (w > 0) f.stdWidths.set(c, w); } catch (e) { /* no glyph */ } }
        }
      }
    }
    f.embedded = !!(desc && (get(desc, "FontFile") || get(desc, "FontFile2") || get(desc, "FontFile3")));
    f.used = new Set(); // codes this font actually shows somewhere (filled while reading)
    f.has = null;       // codes that really have a glyph in the embedded (subset) font, when known
    f.style = styleOf(f.display, desc);
    try { glyphPresence(f, desc); } catch (e) { f.has = null; }
    fontCache.set(fdict, f);
    return f;
  }
  // What a font is called and how it looks, for finding the same font on this computer.
  function styleOf(display, desc) {
    const flags = num(get(desc, "Flags")) || 0;
    const weight = num(get(desc, "FontWeight")) || 0;
    const raw = display.replace(/,/g, "-");
    const bold = /bold|black|heavy|semibold|demi/i.test(raw) || weight >= 600 || !!(flags & (1 << 18));
    const italic = /italic|oblique/i.test(raw) || (num(get(desc, "ItalicAngle")) || 0) !== 0 || !!(flags & (1 << 6));
    const family = raw.replace(/-(Bold|Italic|BoldItalic|Regular|Roman|Oblique|BoldOblique|Black|Semibold|SemiBold|Light|Medium)$/i, "")
      .replace(/(PSMT|PS|MT)$/,"").replace(/([a-z])([A-Z])/g, "$1 $2").trim();
    return { family, bold, italic, serif: !!(flags & 2), mono: !!(flags & 1) };
  }

  function glyphPresence(f, desc) {
    const ff1 = get(desc, "FontFile"), ff2 = get(desc, "FontFile2");
    if (ff1) {
      // Type1: the font program's own encoding (dup <code> /<name> put) and the subset's CharSet.
      const prog = streamText(ff1);
      const names = new Map();
      for (const m of prog.slice(0, 200000).matchAll(/dup (\d+) \/([^\s]+) put/g)) names.set(Number(m[1]), m[2]);
      const encObj = get(f.dict, "Encoding");
      const diffs = encObj instanceof PDFDict ? arr(get(encObj, "Differences")) : null;
      if (diffs) { let code = 0; for (const d of diffs) { if (d instanceof PDFNumber) code = d.asNumber(); else if (d instanceof PDFName) names.set(code++, d.decodeText()); } }
      const cs = get(desc, "CharSet");
      const charset = cs ? new Set((cs.decodeText ? cs.decodeText() : String(cs)).split("/").filter(Boolean)) : null;
      if (charset && names.size) {
        f.has = new Set([...names].filter(([, n]) => charset.has(n)).map(([c]) => c));
        // Without ToUnicode, glyph names are the best guess at the text.
        if (!f.toUni) for (const [c, n] of names) { const u = glyphToUnicode(n); if (u) f.codeToUni.set(c, u); }
      }
    } else if (ff2 && opts.fontkit) {
      // TrueType: ask the font file which glyphs it really has.
      const bytes = deref(ff2) instanceof PDFRawStream ? decodePDFRawStream(deref(ff2)).decode() : null;
      if (!bytes) return;
      const tt = opts.fontkit.create(bytes);
      const has = new Set();
      const real = (gid) => gid > 0 && gid < tt.numGlyphs && (() => { const g = tt.getGlyph(gid); return g.advanceWidth > 0; })();
      const codes = f.twoByte ? [...f.width.keys(), ...(f.toUni ? f.toUni.keys() : [])] : [...Array(256).keys()];
      for (const code of codes) {
        let gid = 0;
        if (f.twoByte) gid = code; // CIDToGIDMap /Identity (what Chrome, Word and LibreOffice write)
        else { const cm = tt._cmapProcessor; gid = cm.lookup(code) || cm.lookup(0xf000 + code) || 0; if (!gid && f.codeToUni.get(code)) gid = cm.lookup(f.codeToUni.get(code).codePointAt(0)) || 0; }
        if (real(gid)) has.add(code);
      }
      const cidmap = f.twoByte ? get(arr(get(f.dict, "DescendantFonts"))[0], "CIDToGIDMap") : null;
      if (cidmap && !(cidmap instanceof PDFName)) return; // a mapping stream: don't guess
      if (has.size) f.has = has;
    }
  }

  // A font this session embedded (written into the file only on save): read it from fontkit.
  function fallbackInfo(fb) {
    if (fb.info) return fb.info;
    const fk = fb.font.embedder.font;
    const f = { dict: null, subtype: "Type0", base: fb.name, display: fb.name, twoByte: true, problem: null, subset: false, embedded: false,
      toUni: null, codeToUni: new Map(), width: new Map(), dw: 0, used: new Set(), has: null, style: null };
    const scale = 1000 / fk.unitsPerEm;
    for (const cp of fk.characterSet) {
      const g = fk.glyphForCodePoint(cp);
      if (!g || !g.id) continue;
      if (!f.codeToUni.has(g.id)) f.codeToUni.set(g.id, String.fromCodePoint(cp));
      f.width.set(g.id, Math.round(g.advanceWidth * scale));
    }
    fb.info = f;
    return f;
  }

  const glyphWidth = (f, code) => (f.width.has(code) ? f.width.get(code) : f.stdWidths && f.stdWidths.has(code) ? f.stdWidths.get(code) : f.dw);
  const codesOf = (f, bytes) => {
    const out = [];
    if (f.twoByte) for (let k = 0; k + 1 < bytes.length; k += 2) out.push((bytes[k] << 8) | bytes[k + 1]);
    else for (const b of bytes) out.push(b);
    return out;
  };

  // ---------- reading a page ----------

  const mul = (a, b) => [a[0] * b[0] + a[1] * b[2], a[0] * b[1] + a[1] * b[3], a[2] * b[0] + a[3] * b[2], a[2] * b[1] + a[3] * b[3], a[4] * b[0] + a[5] * b[2] + b[4], a[4] * b[1] + a[5] * b[3] + b[5]];
  const pages = pdfDoc.getPages();
  const pageCache = new Map();

  function readPage(pi) {
    if (pageCache.has(pi)) return pageCache.get(pi);
    const page = pages[pi];
    const res = page.node.Resources();
    const fonts = get(res, "Font");
    const contents = deref(page.node.get(PDFName.of("Contents")));
    const parts = contents instanceof PDFArray ? contents.asArray() : contents ? [contents] : [];
    const src = parts.map(streamText).join("\n");
    const ops = tokenize(src);
    const xobjects = get(res, "XObject");
    let gs = { ctm: [1, 0, 0, 1, 0, 0], font: null, fontRes: null, size: 0, Tc: 0, Tw: 0, Th: 1, TL: 0, Ts: 0, Tr: 0 };
    const stack = [];
    let Tm = [1, 0, 0, 1, 0, 0], Tlm = [1, 0, 0, 1, 0, 0], inBT = false, btIndex = -1;
    const pieces = [];
    const formText = []; // text in form XObjects: shown as "can't edit"
    const nextLine = (tx, ty) => { Tlm = mul([1, 0, 0, 1, tx, ty], Tlm); Tm = Tlm.slice(); };

    ops.forEach((o, oi) => {
      const a = o.args.map((x) => (x && x.t === "num" ? x.v : x));
      switch (o.op) {
        case "q": stack.push({ ...gs, ctm: gs.ctm.slice() }); break;
        case "Q": if (stack.length) gs = stack.pop(); break;
        case "cm": gs.ctm = mul(a.slice(0, 6), gs.ctm); break;
        case "BT": inBT = true; btIndex++; Tm = [1, 0, 0, 1, 0, 0]; Tlm = Tm.slice(); break;
        case "ET": inBT = false; break;
        case "Tf": {
          gs.fontRes = a[0] && a[0].v; gs.size = a[1];
          const raw = fonts && gs.fontRes ? fonts.get(PDFName.of(gs.fontRes)) : null;
          const fb = raw && [...fallbacks.values()].find((x) => x.ref === raw);
          const fd = fb ? null : deref(raw);
          gs.font = fb ? fallbackInfo(fb) : fd ? fontInfo(fd) : null;
          break;
        }
        case "Tc": gs.Tc = a[0]; break;
        case "Tw": gs.Tw = a[0]; break;
        case "Tz": gs.Th = a[0] / 100; break;
        case "TL": gs.TL = a[0]; break;
        case "Ts": gs.Ts = a[0]; break;
        case "Tr": gs.Tr = a[0]; break;
        case "Td": nextLine(a[0], a[1]); break;
        case "TD": gs.TL = -a[1]; nextLine(a[0], a[1]); break;
        case "Tm": Tlm = a.slice(0, 6); Tm = Tlm.slice(); break;
        case "T*": nextLine(0, -gs.TL); break;
        case "Do": {
          const xo = xobjects && a[0] ? get(xobjects, a[0].v) : null;
          if (xo && name(get(xo.dict || xo, "Subtype")) === "Form") formText.push(oi);
          break;
        }
        case "'": case '"': case "Tj": case "TJ": {
          if (o.op === '"') { gs.Tw = a[0]; gs.Tc = a[1]; }
          if (o.op === "'" || o.op === '"') nextLine(0, -gs.TL);
          if (!gs.font || !inBT) break;
          const f = gs.font;
          const elems = o.op === "TJ" ? (o.args[0] && o.args[0].t === "arr" ? o.args[0].v : []) : [o.args[o.args.length - 1]];
          // Walk the elements, splitting a TJ where a big gap jumps to another column.
          let piece = null;
          const flush = () => { if (piece && piece.glyphs.length) pieces.push(piece); piece = null; };
          const startPiece = (ei) => {
            const trm = mul([gs.size * gs.Th, 0, 0, gs.size, 0, gs.Ts], mul(Tm, gs.ctm));
            piece = {
              op: oi, opName: o.op, bt: btIndex, e0: ei, e1: ei, font: f, fontRes: gs.fontRes, size: gs.size, Tc: gs.Tc, Tw: gs.Tw, Th: gs.Th, Tr: gs.Tr,
              x: trm[4], y: trm[5], scale: Math.hypot(trm[0], trm[1]) || gs.size, dir: Math.atan2(trm[1], trm[0]), glyphs: [], adv: 0, ctm: gs.ctm.slice(),
            };
          };
          elems.forEach((el, ei) => {
            if (!el) return;
            if (el.t === "num") {
              const tx = (-el.v / 1000) * gs.size * gs.Th;
              if (piece && Math.abs(tx) > gs.size * 3) flush();
              if (piece) {
                // A forward jump of a word space or more is a space (TeX and some
                // other makers never draw a space glyph).
                // Only counted once more glyphs follow (a step back after new text isn't a space).
                if (tx > gs.size * 0.15 && piece.glyphs.length && piece.glyphs[piece.glyphs.length - 1].uni !== " ") piece.pendingGap = tx;
                else if (tx < 0) piece.pendingGap = 0;
                piece.adv += tx; piece.e1 = ei;
              }
              Tm = mul([1, 0, 0, 1, tx, 0], Tm);
              return;
            }
            if (el.t !== "str") return;
            if (!piece) startPiece(ei);
            if (piece.pendingGap && el.bytes.length) { piece.glyphs.push({ code: null, uni: " ", adv: 0, virtual: true, gap: piece.pendingGap }); }
            piece.pendingGap = 0;
            for (const code of codesOf(f, el.bytes)) {
              f.used.add(code);
              const w0 = glyphWidth(f, code) / 1000;
              const space = !f.twoByte && code === 32 ? gs.Tw : 0;
              const tx = (w0 * gs.size + gs.Tc + space) * gs.Th;
              piece.glyphs.push({ code, uni: f.codeToUni.get(code) ?? "\ufffd", adv: tx });
              piece.adv += tx;
              Tm = mul([1, 0, 0, 1, tx, 0], Tm);
            }
            piece.e1 = ei;
          });
          flush();
          break;
        }
        default:
      }
    });

    // Lines: pieces on one baseline, in order, same font and size, without a big gap.
    const runs = [];
    for (const p of pieces) {
      p.text = p.glyphs.map((g) => g.uni).join("");
      const last = runs[runs.length - 1];
      const lp = last && last.pieces[last.pieces.length - 1];
      if (lp) {
        const endX = lp.x + lp.adv * (lp.scale / lp.size) * Math.cos(lp.dir);
        const endY = lp.y + lp.adv * (lp.scale / lp.size) * Math.sin(lp.dir);
        const gap = (p.x - endX) * Math.cos(p.dir) + (p.y - endY) * Math.sin(p.dir);
        const drift = -(p.x - endX) * Math.sin(p.dir) + (p.y - endY) * Math.cos(p.dir);
        const same = lp.font === p.font && Math.abs(lp.size - p.size) < 0.01 && Math.abs(lp.dir - p.dir) < 0.01 && lp.bt === p.bt && lp.Tr === p.Tr;
        if (same && Math.abs(drift) < p.scale * 0.15 && gap > -p.scale * 0.3 && gap < p.scale * 1.2) {
          if (gap > p.scale * 0.15 && !/\s$/.test(last.text) && !/^\s/.test(p.text)) last.text += " ";
          last.text += p.text;
          last.pieces.push(p);
          last.width = (p.x - last.x) * Math.cos(p.dir) + (p.y - last.y) * Math.sin(p.dir) + p.adv * (p.scale / p.size);
          continue;
        }
      }
      runs.push({ id: `${pi}@${p.x.toFixed(1)},${p.y.toFixed(1)}`, page: pi, pieces: [p], text: p.text, x: p.x, y: p.y, size: p.scale, dir: p.dir, width: p.adv * (p.scale / p.size), font: p.font });
    }
    // Room to grow: up to the next text on the same line, else up to the right margin
    // (taken as wide as the page's left margin).
    const pageW = page.getWidth();
    const margin = runs.length ? Math.max(18, Math.min(...runs.map((r) => r.x))) : 36;
    for (const r of runs) {
      const ahead = runs.filter((o) => o !== r && Math.abs(o.dir - r.dir) < 0.01 && Math.abs(o.y - r.y) < r.size * 0.5 && o.x > r.x + r.width * 0.5);
      r.nextText = ahead.length > 0;
      const limit = ahead.length ? Math.min(...ahead.map((o) => o.x)) - r.size * 0.25 : pageW - margin;
      r.room = Math.max(0, limit - (r.x + r.width));
    }
    for (const r of runs) {
      const f = r.font;
      r.fontName = f.display || r.pieces[0].fontRes;
      r.editable = !f.problem && r.pieces[0].Tr !== 3 && r.pieces[0].Tr !== 7;
      r.why = f.problem ? f.problem : r.editable ? null : "Hidden search text on a scanned page: change the picture instead";
      r.text = r.text.replace(/ /g, " ");
    }
    const info = { page, src, ops, pieces, runs, formText: formText.length, parts };
    pageCache.set(pi, info);
    return info;
  }

  // ---------- encoding new text in the page's own font ----------

  function encoderFor(f) {
    // unicode char -> code, only for codes that really have a glyph in this font.
    if (f._enc) return f._enc;
    const enc = new Map();
    const usable = (code) => {
      if (f.has && !f.used.has(code) && !f.has.has(code)) return false;
      if (f.subset || (f.embedded && !f.toUni)) {
        // A subset only has the glyphs it was made with: the ones in ToUnicode / used on the page.
        if (!(f.toUni && f.toUni.has(code)) && !f.used.has(code)) return false;
      }
      const w = glyphWidth(f, code);
      return w > 0 || (f.toUni && f.toUni.get(code) === " ");
    };
    for (const [code, u] of f.codeToUni) {
      if (!u || [...u].length !== 1) continue; // ligatures (fi, ffl) are never chosen for new text
      if (!usable(code)) continue;
      if (!enc.has(u)) enc.set(u, code);
    }
    f._enc = enc;
    return enc;
  }
  const NORMALISE = { " ": " ", " ": " " };

  function encodeText(f, text, spaceGap) {
    // Returns TJ elements: { codes } runs and { gap } (text-space units) for spaces
    // the font has no glyph for (TeX fonts), plus the characters it can't draw.
    const enc = encoderFor(f);
    const parts = [], missing = [];
    let cur = null;
    for (const ch of text) {
      const c = NORMALISE[ch] || ch;
      if (enc.has(c)) { if (!cur) parts.push((cur = { codes: [] })); cur.codes.push(enc.get(c)); }
      else if (c === " " && spaceGap) { parts.push({ gap: spaceGap }); cur = null; }
      else missing.push(ch);
    }
    return { parts, codes: parts.flatMap((p) => p.codes || []), missing: [...new Set(missing)] };
  }

  const hex = (codes, two) => "<" + codes.map((c) => c.toString(16).padStart(two ? 4 : 2, "0")).join("").toUpperCase() + ">";
  const fmt = (v) => (Math.abs(v) < 1e-6 ? "0" : String(Math.round(v * 1000) / 1000));

  // ---------- fallback font (only when the page's font lacks a character) ----------

  // Fonts are embedded whole: pdf-lib's subsetting drops glyphs (seen with Carlito
  // and Noto Sans, 27 Oct 2026; the editor's Text tool embeds whole fonts for the
  // same reason). A Rust subsetter could shrink them later.
  const fallbacks = new Map(); // key -> { font, ref, name, match }
  async function fallbackFont(f, text) {
    // 1. The same family and style installed on this computer (Rust looks it up by name).
    // 2. The bundled font. Either way it's embedded as a subset of what's needed.
    const st = f.style;
    let found = opts.findFont ? await opts.findFont(st) : null;
    const key = found ? found.name : "bundled";
    if (fallbacks.has(key)) {
      const fb = fallbacks.get(key);
      if ([...text].every((ch) => ch === " " || fb.font.embedder.font.hasGlyphForCodePoint(ch.codePointAt(0)))) return fb;
    }
    if (opts.fontkit) pdfDoc.registerFontkit(opts.fontkit);
    if (found) {
      const font = await pdfDoc.embedFont(found.bytes, { subset: !!opts.subsetFonts });
      const fb = { font, ref: font.ref, name: found.name, match: true };
      if ([...text].every((ch) => ch === " " || font.embedder.font.hasGlyphForCodePoint(ch.codePointAt(0)))) { fallbacks.set(key, fb); return fb; }
    }
    if (!opts.fallbackFontBytes) throw new Error("No font has these characters");
    if (!fallbacks.has("bundled")) {
      const font = await pdfDoc.embedFont(opts.fallbackFontBytes, { subset: !!opts.subsetFonts });
      fallbacks.set("bundled", { font, ref: font.ref, name: opts.fallbackFontName || "Noto Sans", match: false });
    }
    return fallbacks.get("bundled");
  }

  // ---------- editing ----------

  const edits = new Map(); // page -> [{ start, end, text }]

  async function editRun(runId, newText, how = {}) {
    const pi = Number(runId.split("@")[0]);
    const info = readPage(pi);
    const run = info.runs.find((r) => r.id === runId);
    if (!run) throw new Error("No such text on the page");
    if (!run.editable) throw new Error(run.why);
    const f = run.font;
    const p0 = run.pieces[0];
    const gaps = run.pieces.flatMap((p) => p.glyphs.filter((g) => g.virtual).map((g) => g.gap));
    const spaceGap = gaps.length ? gaps.sort((a, b) => a - b)[gaps.length >> 1] : p0.size * 0.33 * p0.Th;
    let { parts, missing } = encodeText(f, newText, spaceGap);
    let usedFallback = false;
    let newAdvText, insert;
    if (!missing.length) {
      // Width in this font, with the run's own character and word spacing.
      newAdvText = 0;
      const els = [];
      for (const part of parts) {
        if (part.gap) { newAdvText += part.gap; els.push(fmt((-part.gap * 1000) / (p0.size * p0.Th))); continue; }
        for (const c of part.codes) newAdvText += ((glyphWidth(f, c) / 1000) * p0.size + p0.Tc + (!f.twoByte && c === 32 ? p0.Tw : 0)) * p0.Th;
        els.push(hex(part.codes, f.twoByte));
      }
      insert = els.join(" ");
    } else {
      const fb = await fallbackFont(f, newText);
      usedFallback = fb;
      const key = fallbackResourceName(info, fb);
      const w = fb.font.widthOfTextAtSize(newText, p0.size);
      newAdvText = (w + p0.Tc * [...newText].length) * p0.Th;
      insert = { fbKey: key, hexText: fb.font.encodeText(newText).toString() };
    }
    // Too wide for the space before the next text: squeeze it (down to 80%) if asked.
    const room = run.width + run.room;
    const newW = (newAdvText * p0.scale) / p0.size;
    let squeeze = 1;
    if (how.fit && newW > room + 0.01) squeeze = Math.max(0.8, room / newW);
    if (squeeze < 1) {
      newAdvText *= squeeze;
      if (typeof insert === "string") insert = { tz: Math.round(p0.Th * squeeze * 10000) / 100, plain: insert };
      else insert.tz = Math.round(p0.Th * squeeze * 10000) / 100;
    }
    // Every piece's glyphs become a plain move by their old width (layout unchanged);
    // the first piece also draws the new text and steps back over it.
    const opEdits = new Map(); // op index -> list of { e0, e1, replacement elements string }
    for (const [k, p] of run.pieces.entries()) {
      const oldUnits = (-p.adv * 1000) / (p.size * p.Th); // TJ number that moves forward by the old width
      let rep;
      if (k === 0) {
        const back = (newAdvText * 1000) / (p.size * p.Th);
        rep = typeof insert === "string" ? `${insert} ${fmt(back + oldUnits)}` : { fb: insert, back, oldUnits };
        if (insert && insert.plain) rep = { fb: { ...insert, fbKey: p.fontRes, hexText: insert.plain }, back, oldUnits };
      } else rep = fmt(oldUnits);
      if (!opEdits.has(p.op)) opEdits.set(p.op, []);
      opEdits.get(p.op).push({ e0: p.e0, e1: p.e1, rep });
    }
    const list = edits.get(pi) || [];
    for (const [oi, es] of opEdits) {
      const o = info.ops[oi];
      list.push({ start: o.start, end: o.end, text: rewriteOp(o, es, p0) });
    }
    edits.set(pi, list);
    // The page is read again from the edited bytes before the next edit.
    applyPage(pi);
    return {
      ok: true,
      // "page": the page's own font · "installed": the same font, from this computer · "other": a different font
      font: !usedFallback ? "page" : usedFallback.match ? "installed" : "other",
      fontName: usedFallback ? usedFallback.name : run.fontName,
      missing, widthChange: ((newAdvText * p0.scale) / p0.size) - run.width,
      squeezed: squeeze < 1 ? Math.round(squeeze * 100) : null,
      // Wider than the free space: runs into the next text, or past the right margin.
      overlaps: run.nextText && (newAdvText * p0.scale) / p0.size > run.width + run.room + 0.5,
      pastMargin: !run.nextText && (newAdvText * p0.scale) / p0.size > run.width + run.room + 0.5,
    };
  }

  function rewriteOp(o, es, p0) {
    // Rebuild the operator as one TJ with the chosen elements replaced.
    let elems;
    if (o.op === "TJ") elems = o.args[0].v.slice();
    else elems = [o.args[o.args.length - 1]];
    const out = [];
    const prefix = [];
    let suffix = "";
    if (o.op === "'") prefix.push("T*");
    if (o.op === '"') prefix.push(`${fmt(o.args[0].v)} Tw ${fmt(o.args[1].v)} Tc T*`);
    for (let ei = 0; ei < elems.length; ei++) {
      const e = es.find((x) => x.e0 === ei);
      if (e) {
        if (typeof e.rep === "string") out.push(e.rep);
        else {
          // Fallback font: switch font for the new text only, then back to the page's font.
          prefix.push(`[${out.join(" ")}] TJ`);
          out.length = 0;
          // (the back-step is in units of the squeezed width, so it's written under the same Tz)
          const tz = e.rep.fb.tz;
          prefix.push(`/${e.rep.fb.fbKey} ${fmt(p0.size)} Tf${tz ? ` ${fmt(tz)} Tz` : ""} [${e.rep.fb.hexText} ${fmt(tz ? (e.rep.back * p0.Th * 100) / tz : e.rep.back)}] TJ${tz ? ` ${fmt(p0.Th * 100)} Tz` : ""} /${p0.fontRes} ${fmt(p0.size)} Tf`);
          out.push(fmt(e.rep.oldUnits));
        }
        ei = e.e1;
        continue;
      }
      out.push(serialize(elems[ei]));
    }
    return `${prefix.join(" ")} [${out.join(" ")}] TJ${suffix}`;
  }
  const serialize = (v) => {
    if (!v) return "";
    if (v.t === "num") return fmt(v.v);
    if (v.t === "str") return "<" + v.bytes.map((b) => b.toString(16).padStart(2, "0")).join("") + ">";
    return "";
  };

  function fallbackResourceName(info, fb) {
    const res = info.page.node.Resources();
    let fonts = get(res, "Font");
    if (!fonts) { fonts = ctx.obj({}); res.set(PDFName.of("Font"), fonts); }
    for (const [k, v] of fonts.entries()) if (v === fb.ref) return k.decodeText();
    let n = 1;
    while (fonts.get(PDFName.of(`CvFb${n}`))) n++;
    fonts.set(PDFName.of(`CvFb${n}`), fb.ref);
    return `CvFb${n}`;
  }

  function applyPage(pi) {
    const info = readPage(pi);
    const list = (edits.get(pi) || []).sort((a, b) => b.start - a.start);
    let s = info.src;
    for (const e of list) s = s.slice(0, e.start) + e.text + s.slice(e.end);
    const bytes = new Uint8Array(s.length);
    for (let k = 0; k < s.length; k++) bytes[k] = s.charCodeAt(k) & 255;
    const stream = ctx.flateStream(bytes);
    const ref = ctx.register(stream);
    // The old content streams still hold the old text: drop them from the file
    // (unless another page shares them), so "edited" really means gone.
    const oldContents = info.page.node.get(PDFName.of("Contents"));
    const oldRefs = [];
    if (oldContents instanceof PDFRef) {
      oldRefs.push(oldContents);
      const o = ctx.lookup(oldContents);
      if (o instanceof PDFArray) oldRefs.push(...o.asArray().filter((x) => x instanceof PDFRef));
    } else if (oldContents instanceof PDFArray) oldRefs.push(...oldContents.asArray().filter((x) => x instanceof PDFRef));
    info.page.node.set(PDFName.of("Contents"), ref);
    const shared = new Set();
    pages.forEach((pg, k) => {
      if (k === pi) return;
      const c = pg.node.get(PDFName.of("Contents"));
      const list = c instanceof PDFRef ? [c, ...((ctx.lookup(c) instanceof PDFArray) ? ctx.lookup(c).asArray() : [])] : c instanceof PDFArray ? c.asArray() : [];
      list.forEach((x) => { if (x instanceof PDFRef) shared.add(x.toString()); });
    });
    for (const r of oldRefs) if (!shared.has(r.toString())) ctx.delete(r);
    edits.delete(pi);
    pageCache.delete(pi);
    for (const f of fontCache.values()) f._enc = null;
  }

  return {
    pageCount: pages.length,
    runs: (pi) => readPage(pi).runs.map(({ id, text, x, y, size, width, fontName, editable, why, dir, room, font }) => ({ id, text, x, y, size, width, fontName, editable, why, dir, room, style: font.style })),
    pageSize: (pi) => { const pg = pages[pi]; return { width: pg.getWidth(), height: pg.getHeight() }; },
    formTextCount: (pi) => readPage(pi).formText,
    editRun,
    // Before applying: same font or not, which characters force another font, and
    // whether it still fits (no file changes; the other font's width is estimated).
    check(runId, text) {
      const pi = Number(runId.split("@")[0]);
      const run = readPage(pi).runs.find((r) => r.id === runId);
      if (!run || !run.editable) return { ok: false, why: run ? run.why : "This text has changed; click it again" };
      const f = run.font, p0 = run.pieces[0];
      const gaps = run.pieces.flatMap((p) => p.glyphs.filter((g) => g.virtual).map((g) => g.gap));
      const spaceGap = gaps.length ? gaps.sort((a, b) => a - b)[gaps.length >> 1] : p0.size * 0.33 * p0.Th;
      const { parts, missing } = encodeText(f, text, spaceGap);
      let adv = 0;
      if (!missing.length) {
        for (const part of parts) {
          if (part.gap) { adv += part.gap; continue; }
          for (const c of part.codes) adv += ((glyphWidth(f, c) / 1000) * p0.size + p0.Tc + (!f.twoByte && c === 32 ? p0.Tw : 0)) * p0.Th;
        }
      } else if (opts.fontkit && opts.fallbackFontBytes) {
        const fk = (this._fk ||= opts.fontkit.create(opts.fallbackFontBytes));
        const w = fk.layout(text).glyphs.reduce((a, g) => a + g.advanceWidth, 0) / fk.unitsPerEm;
        adv = (w * p0.size + p0.Tc * [...text].length) * p0.Th;
      }
      const newW = (adv * p0.scale) / p0.size;
      const room = run.width + run.room;
      const tooWide = newW > room + 0.5;
      return {
        ok: true, sameFont: !missing.length, missing, fontName: run.fontName,
        widthChange: newW - run.width, tooWide, nextText: run.nextText,
        fits: tooWide ? room / newW >= 0.8 : true, squeeze: tooWide ? Math.max(80, Math.floor((room / newW) * 100)) : 100,
      };
    },
  };
}
