// Convertino PDF editor.
//   Pages:   reorder (drag), rotate, delete, save selected pages, add pages from another PDF.
//   Mark up: text, highlight, draw, signature (typed or drawn), pictures, and filling in form fields.
//   Adjust:  straighten a photographed page with four corners (like Corner Pin), and filters.
// PDF.js draws the pages; pdf-lib writes a new copy. The original file is never changed:
// Save writes "<name> (edited).pdf" next to it.
//
// Marks are kept in PDF page coordinates (points, origin bottom-left, before any
// rotation), so they stay put when pages move or turn, and pdf-lib can draw them as is.

import * as pdfjs from "./vendor/pdfjs/pdf.min.mjs";
import { homography, mapPoint, warp, applyFilter, findEdges, convex, turnPoint, unturnPoint, IDENTITY, isIdentity, outputSize, FILTERS } from "./adjust-core.js";

const tauri = window.__TAURI__;
const { PDFDocument, PDFPage, rgb, degrees, BlendMode, LineCapStyle, PDFTextField, PDFCheckBox } = window.PDFLib;
const fontkit = window.fontkit;

const abs = (p) => new URL(p, location.href).href;
pdfjs.GlobalWorkerOptions.workerSrc = abs("vendor/pdfjs/pdf.worker.min.mjs");
const PDF_OPTS = {
  cMapUrl: abs("vendor/pdfjs/cmaps/"),
  cMapPacked: true,
  standardFontDataUrl: abs("vendor/pdfjs/standard_fonts/"),
  wasmUrl: abs("vendor/pdfjs/wasm/"),
  iccUrl: abs("vendor/pdfjs/iccs/"),
  isEvalSupported: false,
  enableXfa: false,
};
const FONT_FILES = {
  text: "vendor/fonts/NotoSans-Regular.ttf",
  deva: "vendor/fonts/NotoSansDevanagari-Regular.ttf",
  sign: "vendor/fonts/Caveat-SemiBold.ttf",
};
const SWATCHES = [
  { name: "Black", ink: "#1f2328", fill: "#9aa0a6" },
  { name: "Blue", ink: "#1d4ed8", fill: "#8ab4f8" },
  { name: "Red", ink: "#c62828", fill: "#f28b82" },
  { name: "Yellow", ink: "#b58900", fill: "#ffe066" },
];
const SIGN_INK = "#1f3a8a";
const TEXT_SIZE = 14, SIGN_SIZE = 30, IMAGE_WIDTH = 160, SIGN_IMAGE_WIDTH = 150;
const CHECK = "M5 12.5l4.5 4.5L19 7.5";
const INFO = "M12 8v5 M12 16.5v.5 M12 3a9 9 0 100 18 9 9 0 000-18z";

const $ = (s) => document.querySelector(s);
const $$ = (s) => [...document.querySelectorAll(s)];
const el = (tag, cls, attrs = {}) => {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v);
  return e;
};
const isMac = /Mac/.test(navigator.platform || navigator.userAgent);
if (isMac) document.documentElement.dataset.os = "mac";

// ---------- state ----------

const srcs = []; // { name, bytes: Uint8Array (for pdf-lib), pdf: PDF.js document }
let st = { pages: [], marks: {}, fields: {}, adjust: {} }; // everything Undo covers
// pages: { key, src, index, rot }  rot = rotation added here (0/90/180/270)
// adjust: key -> { on, q, size, filter, bright, contrast } (see the Adjust section)
const ui = { mode: "pages", sel: new Set(), anchor: null, current: null, tool: "select", color: 1, picked: null, place: null, busy: false };
const hist = [], fut = [];
let dirty = false;
let fileName = "document.pdf";
let nextKey = 1;
let nextMark = 1;

function snapshot() { return JSON.stringify(st); }
function record() {
  hist.push(snapshot());
  if (hist.length > 60) hist.shift();
  fut.length = 0;
  setDirty(true);
}
function setDirty(v) {
  dirty = v;
  document.title = `${v ? "• " : ""}${fileName} – Edit`;
}
function undo() {
  if (!hist.length) return;
  fut.push(snapshot());
  st = JSON.parse(hist.pop());
  afterHistory();
}
function redo() {
  if (!fut.length) return;
  hist.push(snapshot());
  st = JSON.parse(fut.pop());
  afterHistory();
}
function afterHistory() {
  const keys = new Set(st.pages.map((p) => p.key));
  ui.sel = new Set([...ui.sel].filter((k) => keys.has(k)));
  if (!keys.has(ui.current)) ui.current = st.pages[0] && st.pages[0].key;
  ui.picked = null;
  setDirty(true);
  refresh();
}

// ---------- PDF.js pages ----------

const pageCache = new Map(); // "src:index" -> PDFPageProxy
/** The page as it is in the PDF. */
async function realPage(item) {
  const k = `${item.src}:${item.index}`;
  if (!pageCache.has(k)) pageCache.set(k, srcs[item.src].pdf.getPage(item.index + 1));
  return pageCache.get(k);
}
/** The page as it will be saved: adjusted pages come back as their straightened, filtered picture. */
async function pdfPage(item) {
  const real = await realPage(item);
  const a = activeAdjust(item.key);
  return a ? adjustedPage(item, real, a) : real;
}
const itemOf = (key) => st.pages.find((p) => p.key === key);
const totalRot = (page, item) => (((page.rotate + item.rot) % 360) + 360) % 360;

/** Draws a page into `canvas` so it fits a `box`-px square. */
async function drawThumb(item, canvas, box) {
  const page = await pdfPage(item);
  const rotation = totalRot(page, item);
  const one = page.getViewport({ scale: 1, rotation });
  const dpr = window.devicePixelRatio || 1;
  const s = Math.min(box / one.width, box / one.height);
  const vp = page.getViewport({ scale: s * dpr, rotation });
  if (canvas._task) canvas._task.cancel();
  canvas.width = Math.max(1, Math.floor(vp.width));
  canvas.height = Math.max(1, Math.floor(vp.height));
  canvas.style.width = `${Math.floor(vp.width / dpr)}px`;
  canvas.style.height = `${Math.floor(vp.height / dpr)}px`;
  const task = page.render({ canvas, viewport: vp });
  canvas._task = task;
  try {
    await task.promise;
  } catch (e) {
    if (e && e.name !== "RenderingCancelledException") console.warn(e);
  }
}

// Thumbnails render when they scroll into view.
const thumbs = new IntersectionObserver((entries) => {
  for (const e of entries) {
    if (!e.isIntersecting) continue;
    const c = e.target;
    const item = itemOf(c.dataset.key);
    if (!item) continue;
    const want = thumbSig(item);
    if (c.dataset.drawn !== want) {
      c.dataset.drawn = want;
      drawThumb(item, c, Number(c.dataset.box));
    }
  }
});
function thumbCanvas(item, box) {
  const c = el("canvas");
  c.dataset.key = item.key;
  c.dataset.box = box;
  thumbs.observe(c);
  return c;
}
const thumbSig = (item) => `${item.rot}|${adjustSig(item.key)}`;
function redrawThumb(c, item) {
  if (c.dataset.drawn === thumbSig(item)) return;
  c.dataset.drawn = thumbSig(item);
  drawThumb(item, c, Number(c.dataset.box));
}

// ---------- loading ----------

async function openPdf(bytes, name) {
  const task = pdfjs.getDocument({ ...PDF_OPTS, data: bytes.slice() });
  task.onPassword = () => {
    task.destroy();
  };
  let pdf;
  try {
    pdf = await task.promise;
  } catch (e) {
    const msg = String((e && e.name) || e);
    if (/Password/i.test(msg) || /password/i.test(String(e && e.message))) {
      throw new Error("This PDF is password-protected. Convertino can't edit protected PDFs yet.");
    }
    throw new Error("This file couldn't be opened as a PDF. It may be damaged.");
  }
  srcs.push({ name, bytes, pdf });
  return srcs.length - 1;
}

async function start(bytes, name) {
  fileName = name;
  setDirty(false);
  const src = await openPdf(bytes, name);
  st.pages = Array.from({ length: srcs[src].pdf.numPages }, (_, i) => ({ key: `k${nextKey++}`, src, index: i, rot: 0 }));
  ui.current = st.pages[0].key;
  $("#loading").hidden = true;
  refresh();
}

// ---------- pages view ----------

const tileEls = new Map(); // key -> element

function tileFor(item) {
  let t = tileEls.get(item.key);
  if (t) return t;
  t = el("div", "tile", { draggable: "true" });
  t.dataset.key = item.key;
  const cell = el("button", "cell", { type: "button" });
  cell.append(thumbCanvas(item, 156));
  const num = el("span", "num");
  t.append(cell, num);
  if (item.src > 0) {
    const from = el("span", "from");
    from.textContent = srcs[item.src].name;
    t.append(from);
  }
  tileEls.set(item.key, t);
  return t;
}

function renderGrid() {
  const grid = $("#grid");
  const want = st.pages.map((item, i) => {
    const t = tileFor(item);
    t.classList.toggle("sel", ui.sel.has(item.key));
    const n = t.querySelector(".num");
    n.textContent = i + 1;
    t.querySelector(".cell").setAttribute("aria-label", `Page ${i + 1}${ui.sel.has(item.key) ? ", selected" : ""}`);
    t.querySelector(".cell").setAttribute("aria-pressed", ui.sel.has(item.key) ? "true" : "false");
    redrawThumb(t.querySelector("canvas"), item);
    return t;
  });
  // Put tiles in order without recreating them (keeps their pictures).
  want.forEach((t, i) => {
    if (grid.children[i] !== t) grid.insertBefore(t, grid.children[i] || null);
  });
  while (grid.children.length > want.length) grid.lastElementChild.remove();
  for (const [k, t] of tileEls) if (!want.includes(t)) tileEls.delete(k);
}

function selectTile(key, e) {
  const keys = st.pages.map((p) => p.key);
  if (e.ctrlKey || e.metaKey) {
    if (ui.sel.has(key)) ui.sel.delete(key);
    else ui.sel.add(key);
    ui.anchor = key;
  } else if (e.shiftKey && ui.anchor && keys.includes(ui.anchor)) {
    const a = keys.indexOf(ui.anchor), b = keys.indexOf(key);
    ui.sel = new Set(keys.slice(Math.min(a, b), Math.max(a, b) + 1));
  } else {
    ui.sel = new Set([key]);
    ui.anchor = key;
  }
  refresh();
}

let dragKey = null;
let dropAt = null; // { key, after }
function wireGrid() {
  const grid = $("#grid");
  grid.addEventListener("click", (e) => {
    const t = e.target.closest(".tile");
    if (t) selectTile(t.dataset.key, e);
  });
  grid.addEventListener("dblclick", (e) => {
    const t = e.target.closest(".tile");
    if (!t) return;
    ui.current = t.dataset.key;
    setMode("markup");
  });
  $("#pages-view").addEventListener("click", (e) => {
    if (!e.target.closest(".tile")) {
      ui.sel.clear();
      refresh();
    }
  });
  grid.addEventListener("dragstart", (e) => {
    const t = e.target.closest(".tile");
    if (!t) return;
    dragKey = t.dataset.key;
    e.dataTransfer.effectAllowed = "move";
    e.dataTransfer.setData("text/plain", dragKey);
    const moving = ui.sel.has(dragKey) ? ui.sel : new Set([dragKey]);
    requestAnimationFrame(() => moving.forEach((k) => tileEls.get(k) && tileEls.get(k).classList.add("dragging")));
  });
  grid.addEventListener("dragover", (e) => {
    if (!dragKey) return;
    const t = e.target.closest(".tile");
    if (!t) return;
    e.preventDefault();
    const r = t.getBoundingClientRect();
    const after = e.clientX > r.left + r.width / 2;
    if (!dropAt || dropAt.key !== t.dataset.key || dropAt.after !== after) {
      $$(".drop-before, .drop-after").forEach((x) => x.classList.remove("drop-before", "drop-after"));
      t.classList.add(after ? "drop-after" : "drop-before");
      dropAt = { key: t.dataset.key, after };
    }
  });
  grid.addEventListener("drop", (e) => {
    e.preventDefault();
    if (dragKey && dropAt) moveTo(dragKey, dropAt.key, dropAt.after);
    endDrag();
  });
  grid.addEventListener("dragend", endDrag);
}
function endDrag() {
  dragKey = null;
  dropAt = null;
  $$(".drop-before, .drop-after, .dragging").forEach((x) => x.classList.remove("drop-before", "drop-after", "dragging"));
}
function moveTo(key, targetKey, after) {
  const moving = ui.sel.has(key) ? [...ui.sel] : [key];
  if (moving.includes(targetKey)) return;
  record();
  const keep = st.pages.filter((p) => !moving.includes(p.key));
  const moved = st.pages.filter((p) => moving.includes(p.key));
  const at = keep.findIndex((p) => p.key === targetKey) + (after ? 1 : 0);
  st.pages = [...keep.slice(0, at), ...moved, ...keep.slice(at)];
  ui.sel = new Set(moved.map((p) => p.key));
  refresh();
}
function nudgeSelected(by) {
  if (!ui.sel.size) return;
  const idx = st.pages.map((p, i) => (ui.sel.has(p.key) ? i : -1)).filter((i) => i >= 0);
  if (by < 0 ? idx[0] === 0 : idx[idx.length - 1] === st.pages.length - 1) return;
  record();
  const pages = st.pages.slice();
  const order = by < 0 ? idx : idx.slice().reverse();
  for (const i of order) [pages[i], pages[i + by]] = [pages[i + by], pages[i]];
  st.pages = pages;
  refresh();
}
function rotateSelected(by) {
  if (!ui.sel.size) return;
  record();
  for (const p of st.pages) if (ui.sel.has(p.key)) p.rot = (p.rot + by + 360) % 360;
  refresh();
}
function deleteSelected() {
  if (!ui.sel.size) return;
  if (ui.sel.size >= st.pages.length) return toast("A PDF needs at least one page.", true);
  record();
  st.pages = st.pages.filter((p) => !ui.sel.has(p.key));
  ui.sel.clear();
  refresh();
}
async function addFromFile(file) {
  try {
    const bytes = new Uint8Array(await file.arrayBuffer());
    const src = await openPdf(bytes, file.name);
    record();
    const lastSel = st.pages.reduce((acc, p, i) => (ui.sel.has(p.key) ? i : acc), st.pages.length - 1);
    const added = Array.from({ length: srcs[src].pdf.numPages }, (_, i) => ({ key: `k${nextKey++}`, src, index: i, rot: 0 }));
    st.pages.splice(lastSel + 1, 0, ...added);
    ui.sel = new Set(added.map((p) => p.key));
    refresh();
    toast(`Added ${added.length} page${added.length > 1 ? "s" : ""} from ${file.name}`);
  } catch (e) {
    toast(e.message || String(e), true);
  }
}

// ---------- mark up view ----------

let vp = null;           // viewport of the page on the stage
let stageItem = null;    // the page item shown
let baselineText = 0.88; // baseline from the top of a 1em box (measured once the fonts load)
let baselineSign = 0.8;
const measure = document.createElement("canvas").getContext("2d");

async function measureFonts() {
  try {
    await Promise.all([document.fonts.load('100px "Convertino Text"'), document.fonts.load('600 100px "Convertino Sign"')]);
    const base = (font) => {
      measure.font = font;
      const m = measure.measureText("Hg");
      const a = m.fontBoundingBoxAscent / 100, d = m.fontBoundingBoxDescent / 100;
      return a && d ? (1 - (a + d)) / 2 + a : null;
    };
    baselineText = base('100px "Convertino Text"') || baselineText;
    baselineSign = base('600 100px "Convertino Sign"') || baselineSign;
  } catch (e) {
    console.warn(e);
  }
}
function textWidth(m, px) {
  measure.font = m.type === "sig" ? `600 ${px}px "Convertino Sign"` : `${px}px "Convertino Text"`;
  return measure.measureText(m.text || " ").width;
}

function renderRail() {
  const rail = $("#rail");
  rail.textContent = "";
  st.pages.forEach((item, i) => {
    const b = el("button", "", { type: "button", "aria-label": `Show page ${i + 1}` });
    if (item.key === ui.current) b.setAttribute("aria-current", "page");
    let c = railCanvases.get(item.key);
    if (!c) {
      c = thumbCanvas(item, 104);
      railCanvases.set(item.key, c);
    }
    redrawThumb(c, item);
    const num = el("span", "num");
    num.textContent = i + 1;
    b.append(c, num);
    if ((st.marks[item.key] || []).length) b.append(el("span", "dot"));
    b.addEventListener("click", () => {
      commitEditing();
      ui.current = item.key;
      ui.picked = null;
      refresh();
    });
    rail.append(b);
  });
}
const railCanvases = new Map();

let stageToken = 0;
async function renderStage() {
  const item = itemOf(ui.current) || st.pages[0];
  if (!item) return;
  ui.current = item.key;
  const token = ++stageToken;
  const page = await pdfPage(item);
  if (token !== stageToken) return;
  const rotation = totalRot(page, item);
  const stage = $("#stage");
  const one = page.getViewport({ scale: 1, rotation });
  const fit = Math.min((stage.clientWidth - 48) / one.width, (stage.clientHeight - 48) / one.height);
  const scale = Math.max(0.3, Math.min(fit, 3));
  const view = page.getViewport({ scale, rotation });
  const sig = adjustSig(item.key);
  const changed = !vp || !stageItem || stageItem.key !== item.key || stageItem.rot !== item.rot || stageItem.sig !== sig || Math.abs(vp.scale - scale) > 0.001;
  vp = view;
  stageItem = { key: item.key, rot: item.rot, total: rotation, sig };
  const sheet = $("#sheet");
  sheet.style.width = `${Math.floor(vp.width)}px`;
  sheet.style.height = `${Math.floor(vp.height)}px`;
  if (changed) {
    const canvas = $("#page-canvas");
    const dpr = window.devicePixelRatio || 1;
    const hi = page.getViewport({ scale: scale * dpr, rotation });
    if (canvas._task) canvas._task.cancel();
    canvas.width = Math.floor(hi.width);
    canvas.height = Math.floor(hi.height);
    canvas.style.width = `${Math.floor(vp.width)}px`;
    canvas.style.height = `${Math.floor(vp.height)}px`;
    // Form fields are left off the picture: they're drawn as boxes you can type into.
    const task = page.render({ canvas, viewport: hi, annotationMode: pdfjs.AnnotationMode.ENABLE_FORMS });
    canvas._task = task;
    task.promise.catch((e) => e && e.name !== "RenderingCancelledException" && console.warn(e));
    await renderFields(page, item, token);
  }
  renderMarks();
}

// Form fields on the page become boxes you can type into.
async function renderFields(page, item, token) {
  const layer = $("#fields");
  layer.textContent = "";
  let annots = [];
  try {
    annots = await page.getAnnotations({ intent: "display" });
  } catch (e) {
    console.warn(e);
  }
  if (token !== stageToken) return;
  // Fields only work on pages of the original PDF (added pages keep theirs as printed).
  if (item.src !== 0) return;
  let count = 0;
  for (const a of annots) {
    if (a.subtype !== "Widget" || a.readOnly || !a.fieldName || a.hidden) continue;
    const [x1, y1] = vp.convertToViewportPoint(a.rect[0], a.rect[1]);
    const [x2, y2] = vp.convertToViewportPoint(a.rect[2], a.rect[3]);
    const left = Math.min(x1, x2), top = Math.min(y1, y2), w = Math.abs(x2 - x1), h = Math.abs(y2 - y1);
    let input;
    if (a.fieldType === "Tx") {
      input = a.multiLine ? el("textarea", "field") : el("input", "field", { type: "text" });
      input.value = st.fields[a.fieldName] ?? a.fieldValue ?? "";
      input.style.fontSize = `${Math.max(9, Math.min(h * 0.62, 14 * vp.scale))}px`;
      let before = null;
      input.addEventListener("focus", () => { before = snapshot(); });
      input.addEventListener("input", () => { st.fields[a.fieldName] = input.value; setDirty(true); });
      input.addEventListener("change", () => {
        if (before) { hist.push(before); fut.length = 0; before = null; }
      });
    } else if (a.fieldType === "Btn" && a.checkBox) {
      input = el("input", "field", { type: "checkbox" });
      const cur = st.fields[a.fieldName] ?? a.fieldValue;
      input.checked = cur != null && cur !== "Off" && cur !== false && cur !== "";
      input.addEventListener("change", () => {
        record();
        st.fields[a.fieldName] = input.checked ? (a.exportValue || "Yes") : "Off";
      });
    } else {
      continue;
    }
    input.setAttribute("aria-label", a.alternativeText || a.fieldName);
    input.title = a.alternativeText || a.fieldName;
    Object.assign(input.style, { left: `${left}px`, top: `${top}px`, width: `${w}px`, height: `${h}px`, pointerEvents: "auto" });
    input.addEventListener("pointerdown", (e) => e.stopPropagation());
    layer.append(input);
    count++;
  }
  ui.fieldCount = count;
  updateStatus();
}

// ---------- marks ----------

const pageMarks = () => st.marks[ui.current] || (st.marks[ui.current] = []);
const toView = (x, y) => vp.convertToViewportPoint(x, y);
const toPdf = (x, y) => vp.convertToPdfPoint(x, y);

/** Screen geometry of a mark: anchor, its box before turning, and the extra turn. */
function geometry(m) {
  const s = vp.scale;
  const turn = ((stageItem.total - (m.rot || 0)) % 360 + 360) % 360;
  if (m.type === "text" || m.type === "sig") {
    const px = m.size * s;
    const [ax, ay] = toView(m.x, m.y);
    const base = (m.type === "sig" ? baselineSign : baselineText) * px;
    return { ax, ay, left: ax, top: ay - base, w: textWidth(m, px) + 2, h: px, origin: `0 ${base}px`, turn, px };
  }
  if (m.type === "img") {
    const [ax, ay] = toView(m.x, m.y);
    return { ax, ay, left: ax, top: ay - m.h * s, w: m.w * s, h: m.h * s, origin: "0 100%", turn };
  }
  if (m.type === "hl") {
    const [a1, b1] = toView(m.x1, m.y1), [a2, b2] = toView(m.x2, m.y2);
    return { left: Math.min(a1, a2), top: Math.min(b1, b2), w: Math.abs(a2 - a1), h: Math.abs(b2 - b1), turn: 0 };
  }
  const pts = m.pts.map(([x, y]) => toView(x, y));
  const xs = pts.map((p) => p[0]), ys = pts.map((p) => p[1]);
  return { pts, left: Math.min(...xs) - 3, top: Math.min(...ys) - 3, w: Math.max(...xs) - Math.min(...xs) + 6, h: Math.max(...ys) - Math.min(...ys) + 6, turn: 0 };
}
/** The on-screen bounding box, allowing for a turned mark. */
function bbox(g) {
  if (!g.turn) return { x: g.left, y: g.top, w: g.w, h: g.h };
  const [ox, oy] = [g.ax, g.ay];
  const r = (g.turn * Math.PI) / 180, c = Math.cos(r), s = Math.sin(r);
  const pts = [[g.left, g.top], [g.left + g.w, g.top], [g.left, g.top + g.h], [g.left + g.w, g.top + g.h]].map(([x, y]) => {
    const dx = x - ox, dy = y - oy;
    return [ox + dx * c - dy * s, oy + dx * s + dy * c];
  });
  const xs = pts.map((p) => p[0]), ys = pts.map((p) => p[1]);
  return { x: Math.min(...xs), y: Math.min(...ys), w: Math.max(...xs) - Math.min(...xs), h: Math.max(...ys) - Math.min(...ys) };
}

function renderMarks() {
  if (!vp) return;
  const layer = $("#marks");
  const ink = $("#ink-layer");
  layer.textContent = "";
  ink.textContent = "";
  ink.setAttribute("width", Math.floor(vp.width));
  ink.setAttribute("height", Math.floor(vp.height));
  const SVG = "http://www.w3.org/2000/svg";
  for (const m of pageMarks()) {
    const g = geometry(m);
    if (m.type === "ink") {
      const pl = document.createElementNS(SVG, "polyline");
      pl.setAttribute("points", g.pts.map((p) => p.join(",")).join(" "));
      Object.assign(pl.style, { fill: "none", stroke: m.color, strokeWidth: `${m.width * vp.scale}px`, strokeLinecap: "round", strokeLinejoin: "round" });
      ink.append(pl);
      continue;
    }
    if (m.id === ui.editing) continue;
    const d = el("div", `mark ${m.type}`);
    Object.assign(d.style, { left: `${g.left}px`, top: `${g.top}px` });
    if (m.type === "hl") {
      Object.assign(d.style, { width: `${g.w}px`, height: `${g.h}px`, background: m.color, opacity: "0.45", mixBlendMode: "multiply" });
    } else if (m.type === "img") {
      Object.assign(d.style, { width: `${g.w}px`, height: `${g.h}px`, backgroundImage: `url("${m.data}")` });
    } else {
      d.textContent = m.text;
      Object.assign(d.style, { fontSize: `${g.px}px`, color: m.color });
    }
    if (g.turn) Object.assign(d.style, { transformOrigin: g.origin, transform: `rotate(${g.turn}deg)` });
    layer.append(d);
  }
  const picked = pageMarks().find((m) => m.id === ui.picked);
  if (picked && ui.tool === "select") {
    const b = bbox(geometry(picked));
    const box = el("div", "picked");
    Object.assign(box.style, { left: `${b.x - 3}px`, top: `${b.y - 3}px`, width: `${b.w + 6}px`, height: `${b.h + 6}px` });
    const del = el("button", "del", { type: "button", "aria-label": "Delete this mark", title: "Delete (Del)" });
    del.innerHTML = '<svg class="ic" viewBox="0 0 24 24"><path d="M6 6l12 12M18 6L6 18"/></svg>';
    del.addEventListener("pointerdown", (e) => e.stopPropagation());
    del.addEventListener("click", () => removePicked());
    box.append(del);
    if (picked.type !== "ink" && picked.type !== "hl") {
      const grow = el("button", "grow", { type: "button", "aria-label": "Drag to resize", title: "Drag to resize" });
      grow.innerHTML = '<svg class="ic" viewBox="0 0 24 24"><path d="M9 20h11V9 M20 20L10 10"/></svg>';
      grow.addEventListener("pointerdown", (e) => startResize(e, picked));
      box.append(grow);
    }
    layer.append(box);
  }
  if (ui.editing) placeEditor();
}

function hit(x, y) {
  const list = pageMarks();
  for (let i = list.length - 1; i >= 0; i--) {
    const b = bbox(geometry(list[i]));
    const pad = list[i].type === "ink" ? 4 : 2;
    if (x >= b.x - pad && x <= b.x + b.w + pad && y >= b.y - pad && y <= b.y + b.h + pad) return list[i];
  }
  return null;
}

function localPoint(e) {
  const r = $("#sheet").getBoundingClientRect();
  return [e.clientX - r.left, e.clientY - r.top];
}

let gesture = null;
function wireSheet() {
  const sheet = $("#sheet");
  sheet.addEventListener("pointerdown", (e) => {
    if (e.button !== 0 || !vp) return;
    // Keep keyboard focus where we put it (the text box), and don't select page text.
    e.preventDefault();
    if (ui.editing) commitEditing();
    const [x, y] = localPoint(e);
    const sw = SWATCHES[ui.color];
    const rot = stageItem.total;
    if (ui.tool === "select") {
      const m = hit(x, y);
      ui.picked = m ? m.id : null;
      renderMarks();
      if (m) {
        gesture = { kind: "move", m, start: toPdf(x, y), orig: JSON.stringify(m), before: snapshot() };
        sheet.setPointerCapture(e.pointerId);
      }
      return;
    }
    if (ui.tool === "text") {
      const [px, py] = toPdf(x, y + TEXT_SIZE * vp.scale * 0.35);
      const m = { id: nextMark++, type: "text", x: px, y: py, size: TEXT_SIZE, text: "", color: sw.ink, rot };
      record();
      pageMarks().push(m);
      startEditing(m, true);
      return;
    }
    if (ui.tool === "place" && ui.place) {
      record();
      const p = ui.place;
      let m;
      if (p.kind === "sig-text") {
        const px = SIGN_SIZE * vp.scale;
        const [ax, ay] = toPdf(x - textWidth({ type: "sig", text: p.text }, px) / 2, y + px * 0.3);
        m = { id: nextMark++, type: "sig", x: ax, y: ay, size: SIGN_SIZE, text: p.text, color: SIGN_INK, rot };
      } else {
        const w = p.w, h = p.h;
        const [ax, ay] = toPdf(x - (w * vp.scale) / 2, y + (h * vp.scale) / 2);
        m = { id: nextMark++, type: "img", x: ax, y: ay, w, h, data: p.data, mime: p.mime, rot };
      }
      pageMarks().push(m);
      ui.place = null;
      setTool("select");
      ui.picked = m.id;
      refresh();
      return;
    }
    if (ui.tool === "highlight") {
      const [px, py] = toPdf(x, y);
      const m = { id: nextMark++, type: "hl", x1: px, y1: py, x2: px, y2: py, color: sw.fill };
      gesture = { kind: "hl", m, before: snapshot() };
      pageMarks().push(m);
      sheet.setPointerCapture(e.pointerId);
      return;
    }
    if (ui.tool === "draw") {
      const m = { id: nextMark++, type: "ink", pts: [toPdf(x, y)], color: sw.ink, width: 2 };
      gesture = { kind: "ink", m, before: snapshot(), last: [x, y] };
      pageMarks().push(m);
      sheet.setPointerCapture(e.pointerId);
    }
  });
  sheet.addEventListener("pointermove", (e) => {
    if (!gesture) return;
    const [x, y] = localPoint(e);
    const g = gesture;
    if (g.kind === "move") {
      const [px, py] = toPdf(x, y);
      const dx = px - g.start[0], dy = py - g.start[1];
      const o = JSON.parse(g.orig), m = g.m;
      if (m.type === "ink") m.pts = o.pts.map(([a, b]) => [a + dx, b + dy]);
      else if (m.type === "hl") Object.assign(m, { x1: o.x1 + dx, y1: o.y1 + dy, x2: o.x2 + dx, y2: o.y2 + dy });
      else Object.assign(m, { x: o.x + dx, y: o.y + dy });
      g.moved = true;
    } else if (g.kind === "hl") {
      const [px, py] = toPdf(x, y);
      Object.assign(g.m, { x2: px, y2: py });
    } else if (g.kind === "ink") {
      if (Math.hypot(x - g.last[0], y - g.last[1]) < 1.5) return;
      g.m.pts.push(toPdf(x, y));
      g.last = [x, y];
    } else if (g.kind === "resize") {
      const dist = Math.hypot(x - g.ax, y - g.ay);
      const f = Math.max(0.2, Math.min(8, dist / g.d0));
      const o = JSON.parse(g.orig), m = g.m;
      if (m.type === "img") Object.assign(m, { w: o.w * f, h: o.h * f });
      else m.size = Math.max(6, o.size * f);
      g.moved = true;
    }
    renderMarks();
  });
  const end = () => {
    const g = gesture;
    gesture = null;
    if (!g) return;
    const m = g.m;
    const tiny =
      (m.type === "hl" && (Math.abs(m.x2 - m.x1) < 4 || Math.abs(m.y2 - m.y1) < 3)) ||
      (m.type === "ink" && m.pts.length < 2);
    if (tiny && (g.kind === "hl" || g.kind === "ink")) {
      st.marks[ui.current] = pageMarks().filter((x) => x !== m);
    } else if (g.kind !== "move" && g.kind !== "resize") {
      hist.push(g.before); fut.length = 0; setDirty(true);
    } else if (g.moved) {
      hist.push(g.before); fut.length = 0; setDirty(true);
    }
    refresh();
  };
  sheet.addEventListener("pointerup", end);
  sheet.addEventListener("pointercancel", end);
  sheet.addEventListener("dblclick", (e) => {
    if (ui.tool !== "select") return;
    const [x, y] = localPoint(e);
    const m = hit(x, y);
    if (m && m.type === "text") startEditing(m, false);
  });
}

function startResize(e, m) {
  e.stopPropagation();
  e.preventDefault();
  const g = geometry(m);
  const [x, y] = localPoint(e);
  gesture = { kind: "resize", m, ax: g.left, ay: g.top, d0: Math.max(10, Math.hypot(x - g.left, y - g.top)), orig: JSON.stringify(m), before: snapshot() };
  $("#sheet").setPointerCapture(e.pointerId);
}

function removePicked() {
  if (!ui.picked) return;
  record();
  st.marks[ui.current] = pageMarks().filter((m) => m.id !== ui.picked);
  ui.picked = null;
  refresh();
}

// Typing text on the page.
let editor = null;
function startEditing(m, isNew) {
  ui.editing = m.id;
  ui.picked = null;
  editor = { m, isNew, before: isNew ? null : snapshot(), text: m.text };
  renderMarks();
}
function placeEditor() {
  const m = pageMarks().find((x) => x.id === ui.editing);
  if (!m) return;
  const g = geometry(m);
  let input = $("#mark-edit");
  if (!input) {
    input = el("input", "mark-edit", { id: "mark-edit", type: "text", placeholder: "Type here", "aria-label": "Text on the page" });
    input.addEventListener("pointerdown", (e) => e.stopPropagation());
    input.addEventListener("input", () => { m.text = input.value; });
    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter" || e.key === "Escape") { e.preventDefault(); commitEditing(); }
    });
    input.addEventListener("blur", () => commitEditing());
    input.value = m.text;
    // Outside the marks layer, which is redrawn while you type.
    $("#sheet").append(input);
    input.focus();
  }
  Object.assign(input.style, { left: `${g.left - 4}px`, top: `${g.top - 3}px`, height: `${g.h + 6}px`, fontSize: `${g.px}px`, color: m.color, width: `${Math.max(160, g.w + 40)}px` });
}
function commitEditing() {
  if (!ui.editing || !editor) return;
  const { m, before } = editor;
  const input = $("#mark-edit");
  if (input) m.text = input.value;
  ui.editing = null;
  editor = null;
  if (input) input.remove();
  if (!m.text.trim()) {
    st.marks[ui.current] = pageMarks().filter((x) => x !== m);
    if (!before) hist.pop(); // an empty new box leaves no trace
  } else if (before && before !== snapshot()) {
    hist.push(before); fut.length = 0;
  }
  setDirty(true);
  refresh();
}

// ---------- signature & pictures ----------

const SIG_KEY = "convertino.signature";
function savedSignature() {
  try { return JSON.parse(localStorage.getItem(SIG_KEY) || "null"); } catch { return null; }
}
function saveSignature(sig) {
  try { localStorage.setItem(SIG_KEY, JSON.stringify(sig)); } catch { /* private mode: fine */ }
}
let padDrawn = false;
function openSignDialog() {
  const d = $("#sig-dialog");
  const saved = savedSignature();
  const btn = $('[data-tool="sign"]').getBoundingClientRect();
  d.style.left = `${Math.max(8, Math.min(window.innerWidth - 376, btn.left - 40))}px`;
  d.hidden = false;
  setSigTab(saved && saved.kind === "sig-img" ? "draw" : "type");
  if (saved && saved.kind === "sig-text") $("#sig-name").value = saved.text;
  if (saved && saved.kind === "sig-img") drawSavedOnPad(saved);
  updateSigPreview();
  (saved && saved.kind === "sig-img" ? $("#sig-place") : $("#sig-name")).focus();
}
function closeSignDialog() {
  $("#sig-dialog").hidden = true;
  if (!ui.place) setTool("select");
}
function setSigTab(which) {
  $("#sig-tab-type").setAttribute("aria-selected", which === "type");
  $("#sig-tab-draw").setAttribute("aria-selected", which === "draw");
  $("#sig-type").hidden = which !== "type";
  $("#sig-draw").hidden = which !== "draw";
  updateSigPreview();
}
function updateSigPreview() {
  const name = $("#sig-name").value.trim();
  const p = $("#sig-preview");
  p.textContent = name || "Your name";
  p.classList.toggle("empty", !name);
  const drawTab = !$("#sig-draw").hidden;
  $("#sig-place").disabled = drawTab ? !padDrawn : !name;
}
function wireSignature() {
  $("#sig-tab-type").addEventListener("click", () => setSigTab("type"));
  $("#sig-tab-draw").addEventListener("click", () => setSigTab("draw"));
  $("#sig-name").addEventListener("input", updateSigPreview);
  $("#sig-name").addEventListener("keydown", (e) => { if (e.key === "Enter" && !$("#sig-place").disabled) $("#sig-place").click(); });
  $("#sig-cancel").addEventListener("click", closeSignDialog);
  $("#sig-clear").addEventListener("click", clearPad);
  $("#sig-place").addEventListener("click", () => {
    if (!$("#sig-draw").hidden) {
      const img = padImage();
      if (!img) return;
      const place = { kind: "sig-img", data: img.data, mime: "image/png", w: SIGN_IMAGE_WIDTH, h: (SIGN_IMAGE_WIDTH * img.h) / img.w };
      saveSignature(place);
      ui.place = place;
    } else {
      const text = $("#sig-name").value.trim();
      if (!text) return;
      ui.place = { kind: "sig-text", text };
      saveSignature(ui.place);
    }
    $("#sig-dialog").hidden = true;
    setTool("place");
  });
  const pad = $("#sig-pad");
  const ctx = pad.getContext("2d");
  let drawing = false, last = null;
  const pos = (e) => {
    const r = pad.getBoundingClientRect();
    return [((e.clientX - r.left) / r.width) * pad.width, ((e.clientY - r.top) / r.height) * pad.height];
  };
  pad.addEventListener("pointerdown", (e) => {
    drawing = true;
    last = pos(e);
    pad.setPointerCapture(e.pointerId);
  });
  pad.addEventListener("pointermove", (e) => {
    if (!drawing) return;
    const p = pos(e);
    ctx.strokeStyle = SIGN_INK;
    ctx.lineWidth = 4;
    ctx.lineCap = ctx.lineJoin = "round";
    ctx.beginPath();
    ctx.moveTo(...last);
    ctx.lineTo(...p);
    ctx.stroke();
    last = p;
    padDrawn = true;
    updateSigPreview();
  });
  pad.addEventListener("pointerup", () => { drawing = false; });
}
function clearPad() {
  const pad = $("#sig-pad");
  pad.getContext("2d").clearRect(0, 0, pad.width, pad.height);
  padDrawn = false;
  updateSigPreview();
}
function drawSavedOnPad(saved) {
  clearPad();
  const img = new Image();
  img.onload = () => {
    const pad = $("#sig-pad");
    const s = Math.min((pad.width - 20) / img.width, (pad.height - 20) / img.height, 1);
    pad.getContext("2d").drawImage(img, (pad.width - img.width * s) / 2, (pad.height - img.height * s) / 2, img.width * s, img.height * s);
    padDrawn = true;
    updateSigPreview();
  };
  img.src = saved.data;
}
/** The drawn signature, cropped to its ink. */
function padImage() {
  const pad = $("#sig-pad");
  const { data, width, height } = pad.getContext("2d").getImageData(0, 0, pad.width, pad.height);
  let x0 = width, y0 = height, x1 = -1, y1 = -1;
  for (let y = 0; y < height; y++)
    for (let x = 0; x < width; x++)
      if (data[(y * width + x) * 4 + 3] > 10) {
        if (x < x0) x0 = x;
        if (x > x1) x1 = x;
        if (y < y0) y0 = y;
        if (y > y1) y1 = y;
      }
  if (x1 < 0) return null;
  const pad2 = 6;
  x0 = Math.max(0, x0 - pad2); y0 = Math.max(0, y0 - pad2);
  x1 = Math.min(width - 1, x1 + pad2); y1 = Math.min(height - 1, y1 + pad2);
  const out = document.createElement("canvas");
  out.width = x1 - x0 + 1;
  out.height = y1 - y0 + 1;
  out.getContext("2d").drawImage(pad, x0, y0, out.width, out.height, 0, 0, out.width, out.height);
  return { data: out.toDataURL("image/png"), w: out.width, h: out.height };
}
async function pickImage(file) {
  const mime = /png$/i.test(file.type) || /\.png$/i.test(file.name) ? "image/png" : "image/jpeg";
  const data = await new Promise((res, rej) => {
    const r = new FileReader();
    r.onload = () => res(r.result);
    r.onerror = rej;
    r.readAsDataURL(file);
  });
  const img = new Image();
  await new Promise((res, rej) => { img.onload = res; img.onerror = () => rej(new Error("That picture couldn't be read.")); img.src = data; });
  ui.place = { kind: "img", data, mime, w: IMAGE_WIDTH, h: (IMAGE_WIDTH * img.naturalHeight) / img.naturalWidth };
  setTool("place");
}

// ---------- saving ----------

async function fetchBytes(url) {
  const r = await fetch(url);
  if (!r.ok) throw new Error(`Couldn't load ${url}`);
  return new Uint8Array(await r.arrayBuffer());
}
const hex = (h) => {
  const n = parseInt(h.slice(1), 16);
  return rgb(((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255);
};
const fontFor = (text) => (/[ऀ-ॿ]/.test(text) ? "deva" : "text");

/** Builds the edited PDF with pdf-lib and returns its bytes. */
async function buildPdf() {
  let out;
  try {
    out = await PDFDocument.load(srcs[0].bytes, { updateMetadata: false });
  } catch (e) {
    if (/encrypt/i.test(String(e && e.message))) throw new Error("This PDF is protected, so Convertino can't save changes to it.");
    throw e;
  }
  out.registerFontkit(fontkit);
  const fonts = {};
  // Whole fonts: pdf-lib's subsetting drops glyphs with these fonts. Caveat's
  // contextual alternates are off because pdf-lib doesn't position them.
  const FEATURES = { sign: { calt: false, liga: false, clig: false, dlig: false } };
  const font = async (k) => (fonts[k] ||= out.embedFont(await fetchBytes(FONT_FILES[k]), { subset: false, features: FEATURES[k] }));

  // Form fields first, while every page is still in place.
  const names = Object.keys(st.fields);
  if (names.length) {
    const form = out.getForm();
    for (const n of names) {
      let f = null;
      try { f = form.getField(n); } catch { continue; }
      const v = st.fields[n];
      if (f instanceof PDFTextField) f.setText(String(v ?? ""));
      else if (f instanceof PDFCheckBox) (v && v !== "Off" ? f.check() : f.uncheck());
    }
    form.updateFieldAppearances(await font(fontFor(names.map((n) => st.fields[n]).join(""))));
  }

  // Pages in their new order: the original's pages, plus copies from added PDFs.
  const originals = out.getPages();
  const added = {};
  const final = [];
  for (const it of st.pages) {
    const adj = activeAdjust(it.key);
    if (adj) {
      // Straightened or filtered: a new page holding the picture.
      const { bytes: pic, png, size } = await adjustedForSave(it, adj);
      const img = png ? await out.embedPng(pic) : await out.embedJpg(pic);
      const pg = PDFPage.create(out);
      pg.setSize(size[0], size[1]);
      pg.drawImage(img, { x: 0, y: 0, width: size[0], height: size[1] });
      final.push(pg);
    } else if (it.src === 0) {
      final.push(originals[it.index]);
    } else {
      added[it.src] ||= await PDFDocument.load(srcs[it.src].bytes, { ignoreEncryption: false });
      const [copy] = await out.copyPages(added[it.src], [it.index]);
      final.push(copy);
    }
  }
  for (let i = out.getPageCount() - 1; i >= 0; i--) out.removePage(i);
  final.forEach((p) => out.addPage(p));

  const images = {};
  for (let i = 0; i < st.pages.length; i++) {
    const it = st.pages[i];
    const pg = out.getPage(i);
    if (it.rot) pg.setRotation(degrees((pg.getRotation().angle + it.rot) % 360));
    for (const m of st.marks[it.key] || []) {
      if (m.type === "text" && m.text.trim()) {
        pg.drawText(m.text, { x: m.x, y: m.y, size: m.size, font: await font(fontFor(m.text)), color: hex(m.color), rotate: degrees(m.rot || 0) });
      } else if (m.type === "sig") {
        pg.drawText(m.text, { x: m.x, y: m.y, size: m.size, font: await font("sign"), color: hex(m.color), rotate: degrees(m.rot || 0) });
      } else if (m.type === "img") {
        images[m.data] ||= await (m.mime === "image/png" ? out.embedPng(m.data) : out.embedJpg(m.data));
        pg.drawImage(images[m.data], { x: m.x, y: m.y, width: m.w, height: m.h, rotate: degrees(m.rot || 0) });
      } else if (m.type === "hl") {
        pg.drawRectangle({
          x: Math.min(m.x1, m.x2), y: Math.min(m.y1, m.y2), width: Math.abs(m.x2 - m.x1), height: Math.abs(m.y2 - m.y1),
          color: hex(m.color), opacity: 0.45, blendMode: BlendMode.Multiply,
        });
      } else if (m.type === "ink" && m.pts.length > 1) {
        // drawSvgPath's y axis points down; flip it back.
        const d = m.pts.map(([x, y], j) => `${j ? "L" : "M"} ${x.toFixed(2)} ${(-y).toFixed(2)}`).join(" ");
        pg.drawSvgPath(d, { x: 0, y: 0, borderColor: hex(m.color), borderWidth: m.width, borderLineCap: LineCapStyle.Round });
      }
    }
  }
  return out.save();
}

function pageList(nums) {
  // [1,2,3,5] -> "1-3, 5"
  const parts = [];
  for (let i = 0; i < nums.length; i++) {
    let j = i;
    while (j + 1 < nums.length && nums[j + 1] === nums[j] + 1) j++;
    parts.push(j > i ? `${nums[i]}-${nums[j]}` : `${nums[i]}`);
    i = j;
  }
  return parts.join(", ");
}

async function save(selectedOnly) {
  if (ui.busy) return;
  commitEditing();
  ui.busy = true;
  const btn = $('[data-act="save"]');
  btn.disabled = true;
  try {
    let bytes = await buildPdf();
    let suffix = "edited";
    if (selectedOnly) {
      const nums = st.pages.map((p, i) => (ui.sel.has(p.key) ? i + 1 : 0)).filter(Boolean);
      if (!nums.length) return;
      const full = await PDFDocument.load(bytes);
      const part = await PDFDocument.create();
      const copies = await part.copyPages(full, nums.map((n) => n - 1));
      copies.forEach((p) => part.addPage(p));
      bytes = await part.save();
      suffix = `page${nums.length > 1 ? "s" : ""} ${pageList(nums)}`;
    }
    const name = await deliver(bytes, suffix);
    if (!selectedOnly) setDirty(false);
    toast(`Saved ${name} · the original is untouched`);
  } catch (e) {
    console.error(e);
    toast(e.message || String(e), true);
  } finally {
    ui.busy = false;
    btn.disabled = false;
  }
}

async function deliver(bytes, suffix) {
  if (tauri) {
    const r = await tauri.core.invoke("editor_save", bytes, { headers: { "x-convertino-suffix": suffix } });
    return r.name;
  }
  // In a browser (design checks): download the file instead.
  window.__saved = { bytes, suffix };
  const name = `${fileName.replace(/\.pdf$/i, "")} (${suffix}).pdf`;
  const a = el("a", "", { href: URL.createObjectURL(new Blob([bytes], { type: "application/pdf" })), download: name });
  a.click();
  return name;
}

// ---------- adjust: straighten (Corner Pin) and filter pages ----------
//
// Each page can have { on, q, size, filter, bright, contrast } in st.adjust. q holds the
// four corners as fractions of the page as it is in the PDF (before turns added here),
// top-left, top-right, bottom-right, bottom-left. An adjusted page is drawn from a picture:
// PDF.js renders the page, warp() straightens it, applyFilter() runs the filter, and the
// saved PDF gets that picture (200 or 300 dpi) in place of the page.

const PREVIEW_DPI = 130;
const SOURCE_MAX = 4200; // longest side of a page picture, in pixels
const adj = { view: "corners", drag: -1, focus: -1, recorded: false, nudgeAt: 0, want: "", rendering: "", chipsSig: "", chipsToken: 0, sliding: false, frame: 0 };
let saveDpi = 200;
try {
  const v = Number(localStorage.getItem("convertino.adjust.dpi"));
  if (v === 200 || v === 300) saveDpi = v;
} catch {}

const freshAdjust = () => ({ on: true, q: IDENTITY.map((p) => p.slice()), size: "corners", filter: "original", bright: 0, contrast: 0 });
function adjustOf(key) { return (st.adjust && st.adjust[key]) || freshAdjust(); }
function editAdjust(key) {
  st.adjust ||= {};
  return st.adjust[key] || (st.adjust[key] = freshAdjust());
}
function isAdjusted(a) {
  if (!a) return false;
  if (a.filter !== "original" || a.bright || a.contrast) return true;
  return !!a.on && (!isIdentity(a.q) || a.size === "a4" || a.size === "letter");
}
function activeAdjust(key) {
  const a = st.adjust && st.adjust[key];
  return isAdjusted(a) ? a : null;
}
function adjustSig(key) {
  const a = activeAdjust(key);
  return a ? JSON.stringify(a) : "";
}
const geomOf = (a) => ({ on: a.on, q: a.on ? a.q : null, size: a.on ? a.size : null });
function trim(map, max) { while (map.size > max) map.delete(map.keys().next().value); }

/** The page (as in the PDF, its own turn applied) as pixels at `scale`. */
const sourceCache = new Map();
function pageSource(item, real, scale) {
  const k = `${item.src}:${item.index}:${scale.toFixed(4)}`;
  if (!sourceCache.has(k)) {
    const job = (async () => {
      const v = real.getViewport({ scale });
      const c = document.createElement("canvas");
      c.width = Math.max(1, Math.round(v.width));
      c.height = Math.max(1, Math.round(v.height));
      await real.render({ canvas: c, viewport: v, background: "#ffffff" }).promise;
      const data = c.getContext("2d").getImageData(0, 0, c.width, c.height).data;
      return { w: c.width, h: c.height, data };
    })();
    job.catch(() => sourceCache.delete(k));
    sourceCache.set(k, job);
    trim(sourceCache, 3);
  }
  return sourceCache.get(k);
}

/** Straightened (not yet filtered) pixels for a page, cached by its corners. */
const warpCache = new Map();
function warped(item, real, a, dpi) {
  const one = real.getViewport({ scale: 1 });
  const base = [one.width, one.height];
  const size = outputSize(a, base);
  const k = `${item.src}:${item.index}|${JSON.stringify(geomOf(a))}|${dpi}`;
  if (!warpCache.has(k)) {
    const job = (async () => {
      const ow = Math.max(1, Math.round((size[0] * dpi) / 72)), oh = Math.max(1, Math.round((size[1] * dpi) / 72));
      let scale = dpi / 72;
      if (a.on) {
        // Enough source pixels for the part inside the corners.
        const q = a.q.map(([u, v]) => [u * base[0], v * base[1]]);
        const len = (p, r) => Math.hypot(p[0] - r[0], p[1] - r[1]);
        const cw = Math.max(1, (len(q[0], q[1]) + len(q[3], q[2])) / 2), ch = Math.max(1, (len(q[0], q[3]) + len(q[1], q[2])) / 2);
        scale *= Math.max(size[0] / cw, size[1] / ch);
      }
      scale = Math.min(scale, SOURCE_MAX / Math.max(base[0], base[1]));
      scale = Math.max(0.05, Math.round(scale * 1000) / 1000);
      const src = await pageSource(item, real, scale);
      const quad = (a.on ? a.q : IDENTITY).map(([u, v]) => [u * src.w, v * src.h]);
      const H = homography([[0, 0], [ow, 0], [ow, oh], [0, oh]], quad);
      const out = new Uint8ClampedArray(ow * oh * 4);
      warp(src.data, src.w, src.h, H, out, ow, oh);
      return { w: ow, h: oh, data: out, size };
    })();
    job.catch(() => warpCache.delete(k));
    warpCache.set(k, job);
    trim(warpCache, 4);
  }
  return warpCache.get(k);
}

/** The finished picture of an adjusted page: { canvas, size (points) }. */
async function makeRaster(item, real, a, dpi) {
  const wp = await warped(item, real, a, dpi);
  const img = new ImageData(new Uint8ClampedArray(wp.data), wp.w, wp.h);
  if (a.filter !== "original" || a.bright || a.contrast) applyFilter(img.data, wp.w, wp.h, a.filter, a.bright, a.contrast, dpi);
  const c = document.createElement("canvas");
  c.width = wp.w;
  c.height = wp.h;
  c.getContext("2d").putImageData(img, 0, 0);
  return { canvas: c, size: wp.size };
}
const rasterCache = new Map();
function previewRaster(item, real, a) {
  const k = `${item.src}:${item.index}|${JSON.stringify(a)}`;
  if (!rasterCache.has(k)) {
    const job = makeRaster(item, real, a, PREVIEW_DPI);
    job.catch(() => rasterCache.delete(k));
    rasterCache.set(k, job);
    trim(rasterCache, 8);
  }
  return rasterCache.get(k);
}

/** What PDF.js's PageViewport does, for a page W × H points (origin bottom-left). */
function makeViewport(W, H, scale, rotation) {
  rotation = ((rotation % 360) + 360) % 360;
  const [A, B, C, D] = { 0: [1, 0, 0, -1], 90: [0, 1, 1, 0], 180: [-1, 0, 0, 1], 270: [0, -1, -1, 0] }[rotation];
  const cx = W / 2, cy = H / 2;
  const turned = A === 0;
  const ox = (turned ? cy : cx) * scale, oy = (turned ? cx : cy) * scale;
  const t = [A * scale, B * scale, C * scale, D * scale, ox - A * scale * cx - C * scale * cy, oy - B * scale * cx - D * scale * cy];
  const det = t[0] * t[3] - t[1] * t[2];
  const inv = [t[3] / det, -t[1] / det, -t[2] / det, t[0] / det, (t[2] * t[5] - t[3] * t[4]) / det, (t[1] * t[4] - t[0] * t[5]) / det];
  return {
    width: (turned ? H : W) * scale,
    height: (turned ? W : H) * scale,
    scale, rotation, transform: t, viewBox: [0, 0, W, H],
    convertToViewportPoint: (x, y) => [x * t[0] + y * t[2] + t[4], x * t[1] + y * t[3] + t[5]],
    convertToPdfPoint: (x, y) => [x * inv[0] + y * inv[2] + inv[4], x * inv[1] + y * inv[3] + inv[5]],
  };
}

/** Stands in for a PDF.js page: the adjusted picture, W × H points, no turn of its own. */
function adjustedPage(item, real, a) {
  const one = real.getViewport({ scale: 1 });
  const [W, H] = outputSize(a, [one.width, one.height]);
  return {
    rotate: 0,
    adjusted: true,
    getViewport: ({ scale = 1, rotation = 0 } = {}) => makeViewport(W, H, scale, rotation),
    getAnnotations: async () => [],
    render({ canvas, viewport }) {
      let cancelled = false;
      const promise = (async () => {
        const r = await previewRaster(item, real, a);
        if (cancelled) return;
        const g = canvas.getContext("2d");
        g.save();
        g.setTransform(1, 0, 0, 1, 0, 0);
        g.fillStyle = "#ffffff";
        g.fillRect(0, 0, canvas.width, canvas.height);
        const t = viewport.transform;
        g.setTransform(t[0], t[1], t[2], t[3], t[4], t[5]);
        g.transform(1, 0, 0, -1, 0, H); // pictures go top-down, PDF space goes bottom-up
        g.imageSmoothingQuality = "high";
        g.drawImage(r.canvas, 0, 0, W, H);
        g.restore();
      })();
      return { promise, cancel() { cancelled = true; } };
    },
  };
}

/** The picture that goes into the saved PDF for an adjusted page. */
async function adjustedForSave(item, a) {
  const real = await realPage(item);
  const r = await makeRaster(item, real, a, saveDpi);
  const png = a.filter === "bw"; // sharp edges: PNG keeps them clean and small
  const blob = await new Promise((res) => r.canvas.toBlob(res, png ? "image/png" : "image/jpeg", 0.9));
  if (!blob) throw new Error("Couldn't make the picture for an adjusted page.");
  return { bytes: new Uint8Array(await blob.arrayBuffer()), png, size: r.size };
}

// ----- the Adjust view -----

function renderAdjust() {
  const item = itemOf(ui.current) || st.pages[0];
  if (!item) return;
  ui.current = item.key;
  renderAdjRail();
  renderAdjStage();
  renderAdjPanel();
}

const adjRailCanvases = new Map();
function renderAdjRail() {
  const rail = $("#adj-rail");
  rail.textContent = "";
  st.pages.forEach((item, i) => {
    const b = el("button", "", { type: "button", "aria-label": `Page ${i + 1}` });
    if (item.key === ui.current) b.setAttribute("aria-current", "page");
    if (ui.sel.size > 1 && ui.sel.has(item.key)) b.classList.add("sel");
    let c = adjRailCanvases.get(item.key);
    if (!c) {
      c = thumbCanvas(item, 104);
      adjRailCanvases.set(item.key, c);
    }
    redrawThumb(c, item);
    const num = el("span", "num");
    num.textContent = i + 1;
    b.append(c, num);
    if (activeAdjust(item.key)) b.append(el("span", "dot", { title: "Adjusted" }));
    b.addEventListener("click", (e) => {
      const keys = st.pages.map((p) => p.key);
      if (e.ctrlKey || e.metaKey) {
        if (!ui.sel.has(ui.current)) ui.sel.add(ui.current);
        if (ui.sel.has(item.key) && item.key !== ui.current) ui.sel.delete(item.key);
        else ui.sel.add(item.key);
        ui.anchor = item.key;
      } else if (e.shiftKey && ui.anchor && keys.includes(ui.anchor)) {
        const x = keys.indexOf(ui.anchor), y = keys.indexOf(item.key);
        ui.sel = new Set(keys.slice(Math.min(x, y), Math.max(x, y) + 1));
      } else {
        ui.sel = new Set([item.key]);
        ui.anchor = item.key;
      }
      ui.current = item.key;
      adj.focus = -1;
      refresh();
    });
    rail.append(b);
  });
  const cur = rail.querySelector('[aria-current="page"]');
  if (cur && cur.scrollIntoView) cur.scrollIntoView({ block: "nearest" });
}

async function renderAdjStage() {
  const item = itemOf(ui.current);
  if (!item) return;
  const result = adj.view === "result";
  $("#adj-v-corners").setAttribute("aria-selected", !result);
  $("#adj-v-result").setAttribute("aria-selected", result);
  const page = result ? await pdfPage(item) : await realPage(item);
  if (itemOf(ui.current) !== item || (adj.view === "result") !== result) return;
  const rotation = totalRot(page, item);
  const view = $("#adj-view"), sheet = $("#adj-sheet"), canvas = $("#adj-canvas");
  const one = page.getViewport({ scale: 1, rotation });
  const fit = Math.min((view.clientWidth - 48) / one.width, (view.clientHeight - 40) / one.height);
  const scale = Math.max(0.1, Math.min(fit, 4));
  const dpr = window.devicePixelRatio || 1;
  const hi = page.getViewport({ scale: scale * dpr, rotation });
  sheet.style.width = `${Math.floor(hi.width / dpr)}px`;
  sheet.style.height = `${Math.floor(hi.height / dpr)}px`;
  drawPins();
  const want = `${item.key}|${item.src}:${item.index}|${rotation}|${result ? adjustSig(item.key) || "plain" : "page"}|${Math.round(hi.width)}x${Math.round(hi.height)}`;
  adj.want = want;
  if (canvas.dataset.drawn === want || adj.rendering === want) return;
  adj.rendering = want;
  // Draw off screen, then swap, so the old picture stays until the new one is ready.
  const c = document.createElement("canvas");
  c.width = Math.max(1, Math.floor(hi.width));
  c.height = Math.max(1, Math.floor(hi.height));
  sheet.classList.add("busy");
  try {
    await page.render({ canvas: c, viewport: hi, background: "#ffffff" }).promise;
  } catch (e) {
    if (e && e.name !== "RenderingCancelledException") { console.warn(e); toast(e.message || String(e), true); }
  } finally {
    if (adj.rendering === want) adj.rendering = "";
  }
  if (adj.want !== want) return;
  sheet.classList.remove("busy");
  canvas.width = c.width;
  canvas.height = c.height;
  canvas.getContext("2d").drawImage(c, 0, 0);
  canvas.dataset.drawn = want;
}

/** Corners as shown: the stored corners turned like the page on screen. */
function shownCorners(item) {
  return adjustOf(item.key).q.map((p) => turnPoint(p, item.rot));
}
function drawPins() {
  const item = itemOf(ui.current);
  const ov = $("#adj-ov"), pins = $("#adj-pins");
  pins.textContent = "";
  ov.textContent = "";
  if (!item || adj.view !== "corners" || !adjustOf(item.key).on) return;
  const sheet = $("#adj-sheet");
  const W = sheet.clientWidth || 1, Hh = sheet.clientHeight || 1;
  ov.setAttribute("viewBox", `0 0 ${W} ${Hh}`);
  const q = shownCorners(item).map(([u, v]) => [u * W, v * Hh]);
  // A 3 × 3 grid inside the corners shows how straight the result will be.
  let grid = "";
  try {
    const Hm = homography([[0, 0], [1, 0], [1, 1], [0, 1]], q);
    for (const t of [1 / 3, 2 / 3]) {
      const a = mapPoint(Hm, t, 0), b = mapPoint(Hm, t, 1), c = mapPoint(Hm, 0, t), d = mapPoint(Hm, 1, t);
      grid += `M${a}L${b}M${c}L${d}`;
    }
  } catch {}
  const NS = "http://www.w3.org/2000/svg";
  const path = (cls, d) => { const p = document.createElementNS(NS, "path"); p.setAttribute("class", cls); p.setAttribute("d", d); ov.append(p); };
  path("quad-shade", `M0,0H${W}V${Hh}H0Z M${q.map((c) => c.join(",")).join("L")}Z`);
  if (grid) path("quad-grid", grid);
  path("quad-line", `M${q.map((c) => c.join(",")).join("L")}Z`);
  const names = ["Top left", "Top right", "Bottom right", "Bottom left"];
  q.forEach(([x, y], i) => {
    const b = el("button", "pin" + (i === adj.drag || i === adj.focus ? " active" : ""), { type: "button", "aria-label": `${names[i]} corner`, title: `${names[i]} corner` });
    b.dataset.i = i;
    b.style.left = `${x}px`;
    b.style.top = `${y}px`;
    pins.append(b);
  });
}

function showLoupe(u, v) {
  const canvas = $("#adj-canvas"), sheet = $("#adj-sheet"), loupe = $("#adj-loupe");
  const g = $("#adj-loupe-canvas").getContext("2d");
  const W = sheet.clientWidth, H = sheet.clientHeight;
  const Z = 3, side = (124 / Z) * (canvas.width / W);
  g.imageSmoothingEnabled = false;
  g.fillStyle = "#000";
  g.fillRect(0, 0, 124, 124);
  g.drawImage(canvas, u * canvas.width - side / 2, v * canvas.height - side / 2, side, side, 0, 0, 124, 124);
  g.strokeStyle = "#ffb020";
  g.lineWidth = 1;
  g.beginPath();
  g.moveTo(62, 46); g.lineTo(62, 78); g.moveTo(46, 62); g.lineTo(78, 62);
  g.stroke();
  const x = u * W, y = v * H;
  let lx = x + 28, ly = y - 152;
  if (ly < -16) ly = y + 28;
  if (lx + 124 > W + 16) lx = x - 152;
  loupe.style.left = `${lx}px`;
  loupe.style.top = `${ly}px`;
  loupe.hidden = false;
}

/** Moves corner i to a point on the page as shown (fractions); refuses shapes that fold over. */
function moveCorner(i, u, v) {
  const item = itemOf(ui.current);
  u = Math.max(0, Math.min(1, u));
  v = Math.max(0, Math.min(1, v));
  const q = adjustOf(item.key).q.map((p) => p.slice());
  q[i] = unturnPoint([u, v], item.rot);
  if (!convex(q)) return false;
  editAdjust(item.key).q = q;
  return true;
}

function renderAdjPanel() {
  const item = itemOf(ui.current);
  if (!item) return;
  const a = adjustOf(item.key);
  $("#adj-persp").setAttribute("aria-checked", a.on ? "true" : "false");
  $("#adj-size").value = a.size;
  $("#adj-size").disabled = !a.on;
  $("#adj-persp-hint").hidden = !a.on;
  if (!adj.sliding) {
    $("#adj-bright").value = a.bright;
    $("#adj-contrast").value = a.contrast;
  }
  $("#adj-bright-out").textContent = a.bright;
  $("#adj-contrast-out").textContent = a.contrast;
  $("#adj-dpi").value = String(saveDpi);
  $("#adj-apply-sel").disabled = ui.sel.size < 2;
  $("#adj-apply-sel").title = ui.sel.size < 2 ? "Ctrl+click pages on the left to pick several" : `Copy this filter to the ${ui.sel.size} selected pages`;
  $("#adj-clear").hidden = !(st.adjust && st.adjust[item.key]);
  $$("#adj-filters .filter").forEach((b) => b.setAttribute("aria-pressed", b.dataset.f === a.filter ? "true" : "false"));
  renderChips(item, a);
}

/** Filter buttons with a small picture of the page in each filter. */
async function renderChips(item, a) {
  const box = $("#adj-filters");
  if (!box.children.length) {
    for (const [id, name] of FILTERS) {
      const b = el("button", "filter", { type: "button", "aria-pressed": "false" });
      b.dataset.f = id;
      const c = el("canvas", "", { width: "60", height: "80" });
      const label = el("span");
      label.textContent = name;
      b.append(c, label);
      b.addEventListener("click", () => setFilter(id));
      box.append(b);
    }
  }
  $$("#adj-filters .filter").forEach((b) => b.setAttribute("aria-pressed", b.dataset.f === a.filter ? "true" : "false"));
  const sig = `${item.src}:${item.index}|${item.rot}|${JSON.stringify(geomOf(a))}`;
  if (sig === adj.chipsSig) return;
  adj.chipsSig = sig;
  const token = ++adj.chipsToken;
  try {
    const real = await realPage(item);
    const small = { ...a, filter: "original", bright: 0, contrast: 0 };
    const wp = await warped(item, real, small, 24);
    if (token !== adj.chipsToken) return;
    for (const b of $$("#adj-filters .filter")) {
      const img = new ImageData(new Uint8ClampedArray(wp.data), wp.w, wp.h);
      applyFilter(img.data, wp.w, wp.h, b.dataset.f, 0, 0, 24);
      const tmp = el("canvas");
      tmp.width = wp.w;
      tmp.height = wp.h;
      tmp.getContext("2d").putImageData(img, 0, 0);
      const c = b.querySelector("canvas");
      const turned = item.rot % 180 !== 0;
      c.width = turned ? wp.h : wp.w;
      c.height = turned ? wp.w : wp.h;
      const g = c.getContext("2d");
      g.translate(c.width / 2, c.height / 2);
      g.rotate((item.rot * Math.PI) / 180);
      g.drawImage(tmp, -wp.w / 2, -wp.h / 2);
    }
  } catch (e) {
    console.warn(e);
  }
}

function adjustChanged() {
  refresh();
}
function setFilter(id) {
  const a = adjustOf(ui.current);
  if (a.filter === id) return;
  record();
  editAdjust(ui.current).filter = id;
  adj.view = "result";
  adjustChanged();
}
async function findEdgesHere() {
  const item = itemOf(ui.current);
  if (!item) return;
  try {
    const real = await realPage(item);
    const one = real.getViewport({ scale: 1 });
    const scale = Math.round((560 / Math.max(one.width, one.height)) * 1000) / 1000;
    const src = await pageSource(item, real, scale);
    const q = findEdges(src.data, src.w, src.h);
    if (!q) return toast("Couldn't find the edges of a page in this picture. Drag the corners onto them instead.", true);
    record();
    const e = editAdjust(item.key);
    e.q = q;
    e.on = true;
    adj.view = "corners";
    adjustChanged();
    toast("Corners placed on the page's edges. Check them, then look at Result.");
  } catch (e) {
    toast(e.message || String(e), true);
  }
}
function resetCorners() {
  const item = itemOf(ui.current);
  if (!item || isIdentity(adjustOf(item.key).q)) return;
  record();
  editAdjust(item.key).q = IDENTITY.map((p) => p.slice());
  adj.view = "corners";
  adjustChanged();
}
function copyFilterTo(keys) {
  const a = adjustOf(ui.current);
  const others = keys.filter((k) => k !== ui.current);
  if (!others.length) return;
  record();
  for (const k of others) {
    const e = editAdjust(k);
    e.filter = a.filter;
    e.bright = a.bright;
    e.contrast = a.contrast;
  }
  adjustChanged();
  const name = FILTERS.find((f) => f[0] === a.filter)[1];
  toast(`${name} copied to ${others.length} more page${others.length > 1 ? "s" : ""}`);
}

function wireAdjust() {
  $("#adj-v-corners").addEventListener("click", () => { adj.view = "corners"; renderAdjStage(); updateStatus(); });
  $("#adj-v-result").addEventListener("click", () => { adj.view = "result"; renderAdjStage(); updateStatus(); });
  $("#adj-persp").addEventListener("click", () => {
    record();
    const e = editAdjust(ui.current);
    e.on = !e.on;
    adjustChanged();
  });
  $("#adj-size").addEventListener("change", (ev) => {
    record();
    editAdjust(ui.current).size = ev.target.value;
    adj.view = "result";
    adjustChanged();
  });
  for (const id of ["bright", "contrast"]) {
    const input = $(`#adj-${id}`);
    input.addEventListener("input", () => {
      if (!adj.sliding) { record(); adj.sliding = true; }
      editAdjust(ui.current)[id] = Number(input.value);
      $(`#adj-${id}-out`).textContent = input.value;
      adj.view = "result";
      cancelAnimationFrame(adj.frame);
      adj.frame = requestAnimationFrame(() => { renderAdjStage(); updateStatus(); });
    });
    input.addEventListener("change", () => { adj.sliding = false; adjustChanged(); });
  }
  $("#adj-dpi").addEventListener("change", (ev) => {
    saveDpi = Number(ev.target.value) === 300 ? 300 : 200;
    try { localStorage.setItem("convertino.adjust.dpi", String(saveDpi)); } catch {}
  });
  $("#adj-apply-sel").addEventListener("click", () => copyFilterTo([...ui.sel]));
  $("#adj-apply-all").addEventListener("click", () => copyFilterTo(st.pages.map((p) => p.key)));
  $("#adj-clear").addEventListener("click", () => {
    if (!st.adjust || !st.adjust[ui.current]) return;
    record();
    delete st.adjust[ui.current];
    adj.view = "corners";
    adjustChanged();
  });

  // Dragging a corner, with a magnifier.
  const pins = $("#adj-pins");
  const pointAt = (e) => {
    const r = $("#adj-sheet").getBoundingClientRect();
    return [(e.clientX - r.left) / r.width, (e.clientY - r.top) / r.height];
  };
  pins.addEventListener("pointerdown", (e) => {
    const b = e.target.closest(".pin");
    if (!b || e.button !== 0) return;
    e.preventDefault();
    adj.drag = adj.focus = Number(b.dataset.i);
    adj.recorded = false;
    pins.setPointerCapture(e.pointerId);
    const [u, v] = shownCorners(itemOf(ui.current))[adj.drag];
    drawPins();
    showLoupe(u, v);
  });
  pins.addEventListener("pointermove", (e) => {
    if (adj.drag < 0) return;
    const [u, v] = pointAt(e);
    const before = snapshot();
    if (moveCorner(adj.drag, u, v)) {
      if (!adj.recorded) { hist.push(before); if (hist.length > 60) hist.shift(); fut.length = 0; setDirty(true); adj.recorded = true; }
      drawPins();
    }
    const [su, sv] = shownCorners(itemOf(ui.current))[adj.drag];
    showLoupe(su, sv);
  });
  const end = () => {
    if (adj.drag < 0) return;
    const i = adj.drag;
    adj.drag = -1;
    $("#adj-loupe").hidden = true;
    if (adj.recorded) adjustChanged();
    else drawPins();
    const b = $(`#adj-pins .pin[data-i="${i}"]`);
    if (b) b.focus({ preventScroll: true });
  };
  pins.addEventListener("pointerup", end);
  pins.addEventListener("pointercancel", end);
  pins.addEventListener("focusin", (e) => {
    const b = e.target.closest(".pin");
    if (b) adj.focus = Number(b.dataset.i);
  });
  pins.addEventListener("keydown", (e) => {
    const b = e.target.closest(".pin");
    const d = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] }[e.key];
    if (!b || !d) return;
    e.preventDefault();
    const i = Number(b.dataset.i);
    const sheet = $("#adj-sheet"), step = e.shiftKey ? 10 : 1;
    const [u, v] = shownCorners(itemOf(ui.current))[i];
    const before = snapshot();
    if (moveCorner(i, u + (d[0] * step) / sheet.clientWidth, v + (d[1] * step) / sheet.clientHeight)) {
      // One Undo step per burst of nudges.
      if (Date.now() - adj.nudgeAt > 800) { hist.push(before); if (hist.length > 60) hist.shift(); fut.length = 0; setDirty(true); }
      adj.nudgeAt = Date.now();
      adj.focus = i;
      drawPins();
      updateStatus();
      clearTimeout(adj.nudgeTimer);
      adj.nudgeTimer = setTimeout(() => { renderAdjRail(); renderAdjPanel(); }, 500);
      $(`#adj-pins .pin[data-i="${i}"]`).focus({ preventScroll: true });
    }
  });
}

// ---------- chrome: tabs, tools, status, toast, keys ----------

function setMode(mode) {
  commitEditing();
  ui.mode = mode;
  if (mode === "markup" && ui.sel.size && !ui.sel.has(ui.current)) ui.current = [...ui.sel][0];
  $("#tab-pages").setAttribute("aria-selected", mode === "pages");
  if (mode === "adjust" && ui.sel.size && !ui.sel.has(ui.current)) ui.current = [...ui.sel][0];
  $("#tab-markup").setAttribute("aria-selected", mode === "markup");
  $("#tab-adjust").setAttribute("aria-selected", mode === "adjust");
  $("#pages-tools").hidden = mode !== "pages";
  $("#markup-tools").hidden = mode !== "markup";
  $("#adjust-tools").hidden = mode !== "adjust";
  $("#pages-view").hidden = mode !== "pages";
  $("#markup-view").hidden = mode !== "markup";
  $("#adjust-view").hidden = mode !== "adjust";
  $("#sig-dialog").hidden = true;
  vp = null;
  refresh();
}
function setTool(tool) {
  commitEditing();
  ui.tool = tool;
  if (tool !== "place") ui.place = null;
  ui.picked = null;
  $$("[data-tool]").forEach((b) => b.setAttribute("aria-pressed", b.dataset.tool === tool || (tool === "place" && ((ui.place && ui.place.kind === "img" && b.dataset.tool === "image") || (ui.place && ui.place.kind !== "img" && b.dataset.tool === "sign")))));
  $("#sheet").dataset.tool = tool;
  if (tool !== "sign") $("#sig-dialog").hidden = true;
  if (tool === "sign") openSignDialog();
  if (tool === "image") $("#image-file").click();
  renderMarks();
  updateStatus();
}
function renderSwatches() {
  const box = $("#swatches");
  box.textContent = "";
  SWATCHES.forEach((s, i) => {
    const b = el("button", "swatch", { type: "button", "aria-label": s.name, title: s.name, "aria-pressed": ui.color === i });
    const dot = el("span");
    dot.style.background = s.ink;
    b.append(dot);
    b.addEventListener("click", () => { ui.color = i; renderSwatches(); });
    box.append(b);
  });
}

function updateStatus() {
  const n = st.pages.length;
  const left = $("#status-left"), right = $("#status-right");
  if (ui.mode === "pages") {
    left.textContent = `${n} page${n === 1 ? "" : "s"}${ui.sel.size ? ` · ${ui.sel.size} selected` : ""}`;
    right.textContent = `Drag pages (or ${isMac ? "⌥←/→" : "Alt+←/→"}) to reorder · Ctrl+click or Shift+click to pick several · double-click to mark up`;
  } else if (ui.mode === "adjust") {
    const i = st.pages.findIndex((p) => p.key === ui.current) + 1;
    const a = adjustOf(ui.current);
    left.textContent = `Page ${i} of ${n}${ui.sel.size > 1 ? ` · ${ui.sel.size} selected` : ""}${activeAdjust(ui.current) ? " · adjusted" : ""}`;
    right.textContent = adj.view === "result"
      ? "This is how the page will be saved"
      : a.on ? `Drag the corners onto the page's corners · arrow keys nudge a corner (Shift ×10)` : "Perspective is off for this page";
  } else {
    const i = st.pages.findIndex((p) => p.key === ui.current) + 1;
    left.textContent = `Page ${i} of ${n}${ui.fieldCount ? " · the blue boxes are form fields you can fill in" : ""}`;
    right.textContent = {
      select: "Click a mark to select it · drag to move · double-click text to edit",
      text: "Click on the page to add text",
      highlight: "Drag over the page to highlight",
      draw: "Drag to draw",
      sign: "Type or draw your signature",
      image: "Choose a picture",
      place: ui.place && ui.place.kind === "img" ? "Click where the picture goes" : "Click where your signature goes",
    }[ui.tool];
  }
  const none = !ui.sel.size;
  ["rotl", "rotr", "delete", "extract"].forEach((a) => ($(`[data-act="${a}"]`).disabled = none));
  $("#selall-label").textContent = ui.sel.size === n && n ? "Select none" : "Select all";
  $('[data-act="undo"]').disabled = !hist.length;
  $('[data-act="redo"]').disabled = !fut.length;
}

function refresh() {
  if (ui.mode === "pages") renderGrid();
  else if (ui.mode === "adjust") renderAdjust();
  else {
    renderRail();
    renderStage();
  }
  updateStatus();
}

let toastTimer = null;
function toast(text, bad) {
  const t = $("#toast");
  $("#toast-text").textContent = text;
  $("#toast-icon").setAttribute("d", bad ? INFO : CHECK);
  t.classList.toggle("bad", !!bad);
  t.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { t.hidden = true; }, bad ? 5000 : 3600);
}

function wireChrome() {
  $("#tab-pages").addEventListener("click", () => setMode("pages"));
  $("#tab-markup").addEventListener("click", () => setMode("markup"));
  $("#tab-adjust").addEventListener("click", () => setMode("adjust"));
  $$("[data-tool]").forEach((b) => b.addEventListener("click", () => setTool(b.dataset.tool)));
  const acts = {
    rotl: () => rotateSelected(-90),
    rotr: () => rotateSelected(90),
    delete: deleteSelected,
    extract: () => save(true),
    add: () => $("#add-file").click(),
    selall: () => { ui.sel = ui.sel.size === st.pages.length ? new Set() : new Set(st.pages.map((p) => p.key)); refresh(); },
    undo, redo,
    save: () => save(false),
    edges: findEdgesHere,
    resetq: resetCorners,
  };
  $$("[data-act]").forEach((b) => b.addEventListener("click", () => acts[b.dataset.act]()));
  $("#add-file").addEventListener("change", (e) => { const f = e.target.files[0]; e.target.value = ""; if (f) addFromFile(f); });
  $("#image-file").addEventListener("change", (e) => {
    const f = e.target.files[0];
    e.target.value = "";
    if (f) pickImage(f).catch((err) => { toast(err.message, true); setTool("select"); });
    else setTool("select");
  });
  document.addEventListener("keydown", (e) => {
    const typing = e.target.closest && e.target.closest("input, textarea");
    const mod = e.ctrlKey || e.metaKey;
    const k = e.key.toLowerCase();
    if (mod && k === "s") { e.preventDefault(); return save(false); }
    if (typing) return;
    if (mod && k === "z" && !e.shiftKey) { e.preventDefault(); return undo(); }
    if (mod && (k === "y" || (k === "z" && e.shiftKey))) { e.preventDefault(); return redo(); }
    if (ui.mode === "pages") {
      if (mod && k === "a") { e.preventDefault(); ui.sel = new Set(st.pages.map((p) => p.key)); return refresh(); }
      if (e.key === "Delete" || e.key === "Backspace") { e.preventDefault(); return deleteSelected(); }
      if (e.key === "Escape") { ui.sel.clear(); return refresh(); }
      // Alt+←/→ (Option on Mac) moves the selected pages one place.
      if (e.altKey && (e.key === "ArrowLeft" || e.key === "ArrowRight")) { e.preventDefault(); return nudgeSelected(e.key === "ArrowLeft" ? -1 : 1); }
    } else if (ui.mode === "adjust") {
      if (e.key === "PageDown" || e.key === "PageUp") {
        const i = st.pages.findIndex((p) => p.key === ui.current) + (e.key === "PageDown" ? 1 : -1);
        if (st.pages[i]) { ui.current = st.pages[i].key; ui.sel = new Set([ui.current]); refresh(); }
      }
    } else {
      if ((e.key === "Delete" || e.key === "Backspace") && ui.picked) { e.preventDefault(); return removePicked(); }
      if (e.key === "Escape") { $("#sig-dialog").hidden = true; return setTool("select"); }
      const tool = { v: "select", t: "text", h: "highlight", d: "draw" }[k];
      if (tool && !mod) return setTool(tool);
      if (e.key === "PageDown" || e.key === "PageUp") {
        const i = st.pages.findIndex((p) => p.key === ui.current) + (e.key === "PageDown" ? 1 : -1);
        if (st.pages[i]) { ui.current = st.pages[i].key; refresh(); }
      }
    }
  });
  new ResizeObserver(() => { if (ui.mode === "markup") { vp = null; renderStage(); } }).observe($("#stage"));
  new ResizeObserver(() => { if (ui.mode === "adjust") renderAdjStage(); }).observe($("#adj-view"));
}

// ---------- start ----------

async function main() {
  wireChrome();
  wireGrid();
  wireSheet();
  wireSignature();
  wireAdjust();
  renderSwatches();
  setTool("select");
  measureFonts().then(() => ui.mode === "markup" && renderMarks());
  if (!tauri) {
    // Opened in a browser: pick a PDF to try the editor.
    $("#loading-text").textContent = "Convertino PDF editor";
    const open = $("#open-demo");
    open.hidden = false;
    open.addEventListener("click", () => $("#demo-file").click());
    $("#demo-file").addEventListener("change", async (e) => {
      const f = e.target.files[0];
      if (!f) return;
      try { await start(new Uint8Array(await f.arrayBuffer()), f.name); } catch (err) { $("#loading-text").textContent = err.message; }
    });
    return;
  }
  tauri.event.listen("editor-close-requested", () => {
    if (!dirty || window.confirm("Close without saving? Your changes will be lost.")) tauri.core.invoke("editor_close");
  });
  try {
    const info = await tauri.core.invoke("editor_file");
    if (info.accent) document.documentElement.style.setProperty("--accent", info.accent);
    const buf = await tauri.core.invoke("editor_bytes");
    await start(new Uint8Array(buf), info.name);
  } catch (e) {
    $("#loading-text").textContent = e.message || String(e);
  }
}
main();
