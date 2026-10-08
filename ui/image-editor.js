// Convertino image editor (Image › Edit and RAW photo › Edit).
//   Draw crops on the picture: rectangles, or 4 corners for something photographed at
//   an angle (a sign, a page, a screen), which is saved straightened out.
//   Turn and flip apply to the whole picture. Each crop is saved as its own file next
//   to the original ("<name> - 1.jpg"); the original is never changed.
//
// Everything is kept in the picture's own pixels after the camera's turn and after the
// turn/flip made here. The window shows a smaller copy (at most PREVIEW px); Rust
// (image_edit.rs) cuts the crops from the full picture with ImageMagick.

import { homography, mapPoint, warp, findEdges, convex } from "./adjust-core.js";

const tauri = window.__TAURI__;
const PREVIEW = 2400;
const NS = "http://www.w3.org/2000/svg";
const $ = (id) => document.getElementById(id);
const isMac = /Mac/.test(navigator.platform || navigator.userAgent);
if (isMac) document.documentElement.dataset.os = "mac";

// ---------- state ----------

let fileName = "picture.jpg", stem = "picture";
let src = null;            // the decoded picture (the file itself, or the preview Rust made)
let W0 = 0, H0 = 0;        // the full picture's size, before any turn here
let W = 0, H = 0;          // the full size as turned here
let k = 1;                 // preview pixels per full pixel
let pixels = null;         // the preview's RGBA, for thumbnails and Find edges
let st = { crops: [], turn: 0, flip: false };
// crop: { kind: "rect", x, y, w, h, name, ratio } or { kind: "quad", pts: [[x,y]×4], name, ratio, outW?, outH? }
let sel = -1, drawKind = "rect", ratioKey = "free", nextNum = 1, dirty = false, busy = false;
const hist = [], fut = [];

const img = $("img"), shade = $("shade"), frame = $("frame"), overlay = $("overlay"), loupe = $("loupe"), fmt = $("fmt");

function snapshot() { return JSON.stringify({ st, nextNum }); }
function record() {
  hist.push(snapshot());
  if (hist.length > 80) hist.shift();
  fut.length = 0;
  setDirty(true);
}
function restore(json) {
  const s = JSON.parse(json);
  const turned = s.st.turn !== st.turn || s.st.flip !== st.flip;
  st = s.st; nextNum = s.nextNum;
  sel = Math.min(sel, st.crops.length - 1);
  if (turned) drawPreview();
  setDirty(true);
  render();
}
function undo() { if (hist.length) { fut.push(snapshot()); restore(hist.pop()); } }
function redo() { if (fut.length) { hist.push(snapshot()); restore(fut.pop()); } }
function setDirty(v) {
  dirty = v;
  document.title = `${v && st.crops.length ? "• " : ""}${fileName} – Edit`;
}

// ---------- geometry ----------

const RATIOS = { free: 0, 1: 1, 0.8: 0.8, 1.7778: 16 / 9, 0.5625: 9 / 16 };
function ratioValue(key) { return key === "orig" ? W / H : RATIOS[key] || 0; }
const rectPts = (c) => [[c.x, c.y], [c.x + c.w, c.y], [c.x + c.w, c.y + c.h], [c.x, c.y + c.h]];
const ptsOf = (c) => (c.kind === "quad" ? c.pts : rectPts(c));
const dist = (a, b) => Math.hypot(a[0] - b[0], a[1] - b[1]);
function bbox(pts) {
  const xs = pts.map((p) => p[0]), ys = pts.map((p) => p[1]);
  const x = Math.min(...xs), y = Math.min(...ys);
  return { x, y, w: Math.max(...xs) - x, h: Math.max(...ys) - y };
}
/** The size a crop is saved at. A 4-corner crop flattens to its longest top/bottom by its longest side (or the shape picked). */
function outSize(c) {
  if (c.kind === "rect") return [Math.round(c.w), Math.round(c.h)];
  if (c.outW) return [Math.round(c.outW), Math.round(c.outH)];
  const p = c.pts;
  let w = Math.max(dist(p[0], p[1]), dist(p[3], p[2])), h = Math.max(dist(p[0], p[3]), dist(p[1], p[2]));
  const r = ratioValue(c.ratio);
  if (r) { if (w / h > r) h = w / r; else w = h * r; }
  return [Math.max(1, Math.round(w)), Math.max(1, Math.round(h))];
}
function clampRect(c) {
  c.w = Math.max(1, Math.min(c.w, W)); c.h = Math.max(1, Math.min(c.h, H));
  c.x = Math.max(0, Math.min(c.x, W - c.w)); c.y = Math.max(0, Math.min(c.y, H - c.h));
}
/** Pins in reading order (top left, top right, bottom right, bottom left), whichever way they were dragged. */
function orderPins(pts) {
  const cx = pts.reduce((a, p) => a + p[0], 0) / 4, cy = pts.reduce((a, p) => a + p[1], 0) / 4;
  const byAngle = [...pts].sort((a, b) => Math.atan2(a[1] - cy, a[0] - cx) - Math.atan2(b[1] - cy, b[0] - cx));
  let first = 0;
  byAngle.forEach((p, i) => { if (p[0] + p[1] < byAngle[first][0] + byAngle[first][1]) first = i; });
  return [0, 1, 2, 3].map((i) => byAngle[(first + i) % 4]);
}
/** Fits a rectangle to its shape around its centre, inside the picture. */
function applyRatio(c) {
  const r = ratioValue(c.ratio);
  if (!r) return;
  if (c.kind === "quad") { delete c.outW; delete c.outH; return; }
  const cx = c.x + c.w / 2, cy = c.y + c.h / 2;
  if (c.w / c.h > r) c.w = c.h * r; else c.h = c.w / r;
  if (c.w > W) { c.w = W; c.h = W / r; }
  if (c.h > H) { c.h = H; c.w = H * r; }
  c.x = cx - c.w / 2; c.y = cy - c.h / 2;
  clampRect(c);
}
function toKind(c, kind) {
  if (c.kind === kind) return;
  if (kind === "quad") {
    c.pts = rectPts(c);
    delete c.x; delete c.y; delete c.w; delete c.h;
  } else {
    Object.assign(c, bbox(c.pts));
    delete c.pts; delete c.outW; delete c.outH;
    clampRect(c);
  }
  c.kind = kind;
}

// ---------- the picture ----------

/** Draws the preview turned and flipped as set, and keeps its pixels. */
function drawPreview() {
  const turned = st.turn % 2 === 1;
  W = turned ? H0 : W0; H = turned ? W0 : H0;
  k = Math.min(1, PREVIEW / Math.max(W0, H0));
  const pw = Math.max(1, Math.round(W * k)), ph = Math.max(1, Math.round(H * k));
  img.width = pw; img.height = ph;
  const g = img.getContext("2d", { willReadFrequently: true });
  g.save();
  g.translate(pw / 2, ph / 2);
  g.rotate((st.turn * Math.PI) / 2);
  if (st.flip) g.scale(-1, 1);
  const dw = W0 * k, dh = H0 * k;
  g.drawImage(src, -dw / 2, -dh / 2, dw, dh);
  g.restore();
  pixels = g.getImageData(0, 0, pw, ph).data;
  $("st-dims").textContent = `${W} × ${H} px`;
  fit();
}
/** Sizes the picture to the space it has. */
function fit() {
  const stage = $("stage");
  const cs = getComputedStyle(stage);
  const aw = stage.clientWidth - parseFloat(cs.paddingLeft) - parseFloat(cs.paddingRight);
  const ah = stage.clientHeight - parseFloat(cs.paddingTop) - parseFloat(cs.paddingBottom);
  const s = Math.min(aw / W, ah / H);
  img.style.width = Math.max(1, Math.floor(W * s)) + "px";
  img.style.height = Math.max(1, Math.floor(H * s)) + "px";
}
const scale = () => img.clientWidth / W;

// ---------- drawing the crops ----------

function svg(tag, attrs, parent) {
  const e = document.createElementNS(NS, tag);
  for (const a in attrs) e.setAttribute(a, attrs[a]);
  parent.appendChild(e);
  return e;
}

/** `full`: also the list and thumbnails (not while dragging). */
function render(full = true) {
  const s = scale(), rw = img.clientWidth, rh = img.clientHeight;
  const dpr = window.devicePixelRatio || 1;
  shade.width = Math.round(rw * dpr); shade.height = Math.round(rh * dpr);
  shade.style.width = rw + "px"; shade.style.height = rh + "px";
  const g = shade.getContext("2d");
  g.setTransform(dpr, 0, 0, dpr, 0, 0);
  g.clearRect(0, 0, rw, rh);
  if (st.crops.length) {
    g.fillStyle = getComputedStyle(document.documentElement).getPropertyValue("--shade");
    g.fillRect(0, 0, rw, rh);
    g.globalCompositeOperation = "destination-out";
    g.fillStyle = "#000"; // fully opaque, so the crops are cut out completely
    for (const c of st.crops) {
      g.beginPath();
      ptsOf(c).forEach(([x, y], i) => (i ? g.lineTo(x * s, y * s) : g.moveTo(x * s, y * s)));
      g.closePath();
      g.fill();
    }
    g.globalCompositeOperation = "source-over";
  }

  frame.querySelectorAll(".box").forEach((b) => b.remove());
  overlay.setAttribute("width", rw); overlay.setAttribute("height", rh);
  overlay.innerHTML = "";
  st.crops.forEach((c, i) => {
    if (c.kind === "rect") {
      const b = document.createElement("div");
      b.className = "box" + (i === sel ? " sel" : "");
      Object.assign(b.style, { left: c.x * s + "px", top: c.y * s + "px", width: c.w * s + "px", height: c.h * s + "px" });
      b.innerHTML = `<div class="thirds"></div><span class="num">${i + 1}</span>` +
        ["nw", "n", "ne", "e", "se", "s", "sw", "w"].map((h) => `<span class="h ${h}" data-h="${h}"></span>`).join("");
      b.dataset.i = i;
      frame.insertBefore(b, overlay);
    } else {
      const p = c.pts.map(([x, y]) => [x * s, y * s]);
      const str = p.map((q) => q.join(",")).join(" ");
      svg("polygon", { points: str, class: "quad-shadow" }, overlay);
      if (i === sel) {
        try {
          const Hq = homography([[0, 0], [1, 0], [1, 1], [0, 1]], p);
          let d = "";
          for (const t of [1 / 3, 2 / 3]) {
            const a = mapPoint(Hq, t, 0), b = mapPoint(Hq, t, 1), e = mapPoint(Hq, 0, t), f = mapPoint(Hq, 1, t);
            d += `M${a}L${b}M${e}L${f}`;
          }
          svg("path", { d, class: "grid" }, overlay);
        } catch { /* corners in a line: no grid */ }
      }
      svg("polygon", { points: str, class: "quad" + (i === sel ? " sel" : ""), "data-i": i }, overlay);
      const top = p.reduce((a, b) => (b[1] < a[1] ? b : a));
      svg("rect", { x: top[0] - 2, y: top[1] - 30, width: 22, height: 20, rx: 4, class: "tag" + (i === sel ? " sel" : "") }, overlay);
      svg("text", { x: top[0] + 9, y: top[1] - 16, "text-anchor": "middle" }, overlay).textContent = i + 1;
      if (i === sel) p.forEach(([x, y], n) => svg("circle", { cx: x, cy: y, r: 9, class: "pin", "data-i": i, "data-p": n }, overlay));
    }
  });
  $("hint").hidden = st.crops.length > 0;
  if (full) renderList(); else renderFields();
}

/** A thumbnail of what will be saved: a copy for rectangles, straightened for 4 corners. */
function thumb(c) {
  const t = document.createElement("canvas");
  t.width = 112; t.height = 84;
  const [ow, oh] = outSize(c);
  const f = Math.min(112 / ow, 84 / oh), tw = Math.max(1, Math.round(ow * f)), th = Math.max(1, Math.round(oh * f));
  const g = t.getContext("2d"), ox = (112 - tw) >> 1, oy = (84 - th) >> 1;
  if (c.kind === "rect") {
    g.drawImage(img, c.x * k, c.y * k, Math.max(1, c.w * k), Math.max(1, c.h * k), ox, oy, tw, th);
    return t;
  }
  try {
    const Hm = homography([[0, 0], [tw, 0], [tw, th], [0, th]], c.pts.map(([x, y]) => [x * k, y * k]));
    const out = g.createImageData(tw, th);
    warp(pixels, img.width, img.height, Hm, out.data, tw, th);
    g.putImageData(out, ox, oy);
  } catch { /* corners in a line */ }
  return t;
}

const ext = () => {
  const o = fmt.selectedOptions[0];
  const m = o && /\(([A-Z0-9]+)\)/.exec(o.textContent);
  return (fmt.value === "same" ? (m ? m[1] : "jpg") : fmt.value).toLowerCase();
};
function ratioName(r) {
  const known = [[1, "1:1"], [0.8, "4:5"], [1.25, "5:4"], [16 / 9, "16:9"], [9 / 16, "9:16"], [4 / 3, "4:3"], [3 / 4, "3:4"], [1.5, "3:2"], [2 / 3, "2:3"]];
  const m = known.find(([v]) => Math.abs(v - r) < 0.01);
  return m ? m[1] : "";
}

function renderList() {
  const list = $("list");
  list.innerHTML = "";
  if (!st.crops.length) {
    list.innerHTML = `<div class="empty">No crops yet. Drag on the picture to draw one. For something photographed at an angle, pick <b>4 corners</b> first.</div>`;
  }
  st.crops.forEach((c, i) => {
    const it = document.createElement("div");
    it.className = "item" + (i === sel ? " sel" : "");
    it.appendChild(thumb(c));
    const [ow, oh] = outSize(c);
    const tag = c.kind === "quad" ? '<span class="kind">4 corners</span>' : ratioName(ow / oh);
    const t = document.createElement("div");
    t.className = "t";
    t.innerHTML = `<div class="n"></div><div class="d">${ow} × ${oh} px${tag ? " · " + tag : ""}</div>`;
    t.firstChild.textContent = `${i + 1} · ${c.name}.${ext()}`;
    it.appendChild(t);
    const del = document.createElement("button");
    del.className = "del"; del.type = "button"; del.textContent = "✕";
    del.setAttribute("aria-label", `Remove crop ${i + 1}`);
    it.appendChild(del);
    it.addEventListener("click", (e) => {
      if (e.target === del) removeCrop(i);
      else { sel = i; render(); }
    });
    list.appendChild(it);
  });
  $("count").textContent = st.crops.length;
  const whole = $("whole").checked, n = st.crops.length;
  const sv = $("save");
  sv.textContent = whole ? `Save ${n + 1} files` : n === 1 ? "Save 1 crop" : n ? `Save ${n} crops` : "Save";
  sv.disabled = busy || (n === 0 && !whole);
  $("undo").disabled = !hist.length; $("redo").disabled = !fut.length;
  setDirty(dirty);
  renderFields();
}

function renderFields() {
  const c = st.crops[sel];
  const quad = !!c && c.kind === "quad";
  $("fields").querySelectorAll("input:not(#whole), .seg button, .ghost").forEach((x) => (x.disabled = !c));
  document.querySelectorAll("[data-kind]").forEach((b) => b.setAttribute("aria-pressed", String(!!c && b.dataset.kind === c.kind)));
  $("quad-tools").hidden = !quad; $("quad-note").hidden = !quad; $("pos-row").hidden = quad;
  $("size-lbl").textContent = quad ? "Saved at" : "Size";
  document.querySelectorAll("[data-ratio]").forEach((b) => b.setAttribute("aria-pressed", String(b.dataset.ratio === (c ? c.ratio : ratioKey))));
  const nm = $("nm"), w = $("w"), h = $("h");
  if (!c) { nm.value = ""; w.value = h.value = $("px").value = $("py").value = ""; return; }
  const [ow, oh] = outSize(c);
  if (document.activeElement !== nm) nm.value = c.name;
  if (document.activeElement !== w) w.value = ow;
  if (document.activeElement !== h) h.value = oh;
  if (!quad) { $("px").value = Math.round(c.x); $("py").value = Math.round(c.y); }
}

function removeCrop(i) {
  record();
  st.crops.splice(i, 1);
  sel = Math.min(sel, st.crops.length - 1);
  render();
}

// ---------- loupe (while dragging a pin) ----------

function showLoupe(p) {
  const s = scale(), Z = 3, g = loupe.getContext("2d");
  loupe.hidden = false;
  const lx = p[0] * s, ly = p[1] * s;
  loupe.style.left = (lx + 24 + 128 > img.clientWidth ? lx - 152 : lx + 24) + "px";
  loupe.style.top = Math.max(0, ly - 152) + "px";
  const span = 128 / (s * Z) * k; // preview pixels across the loupe
  g.imageSmoothingEnabled = false;
  g.fillStyle = "#000"; g.fillRect(0, 0, 128, 128);
  g.drawImage(img, p[0] * k - span / 2, p[1] * k - span / 2, span, span, 0, 0, 128, 128);
  g.strokeStyle = "#ffb020"; g.lineWidth = 1;
  g.beginPath(); g.moveTo(64, 48); g.lineTo(64, 80); g.moveTo(48, 64); g.lineTo(80, 64); g.stroke();
}

// ---------- pointer: draw, move, resize, move a pin ----------

let drag = null;
function at(e) {
  const r = img.getBoundingClientRect(), s = scale();
  return [Math.max(0, Math.min(W, (e.clientX - r.left) / s)), Math.max(0, Math.min(H, (e.clientY - r.top) / s))];
}
frame.addEventListener("pointerdown", (e) => {
  if (e.button !== 0 || !src) return;
  const p = at(e), t = e.target, box = t.closest(".box");
  frame.setPointerCapture(e.pointerId);
  const before = snapshot();
  if (t.classList.contains("pin")) {
    sel = +t.dataset.i;
    drag = { mode: "pin", n: +t.dataset.p, before };
    showLoupe(st.crops[sel].pts[drag.n]);
  } else if (t.classList.contains("quad")) {
    sel = +t.dataset.i;
    drag = { mode: "qmove", p, o: st.crops[sel].pts.map((q) => [...q]), before };
  } else if (box) {
    sel = +box.dataset.i;
    drag = { mode: t.dataset.h || "move", p, o: { ...st.crops[sel] }, before };
  } else {
    st.crops.push({ kind: "rect", x: p[0], y: p[1], w: 0, h: 0, name: `${stem} - ${nextNum++}`, ratio: ratioKey });
    sel = st.crops.length - 1;
    drag = { mode: "new", p, o: { ...st.crops[sel] }, before };
  }
  drag.moved = false;
  render(false);
});
frame.addEventListener("pointermove", (e) => {
  if (!drag) return;
  const q = at(e), c = st.crops[sel];
  drag.moved = true;
  if (drag.mode === "pin") {
    const pts = c.pts.map((x) => [...x]);
    pts[drag.n] = q;
    if (convex(pts)) { c.pts = pts; delete c.outW; delete c.outH; }
    showLoupe(c.pts[drag.n]);
  } else if (drag.mode === "qmove") {
    const b = bbox(drag.o);
    const dx = Math.max(-b.x, Math.min(W - b.x - b.w, q[0] - drag.p[0]));
    const dy = Math.max(-b.y, Math.min(H - b.y - b.h, q[1] - drag.p[1]));
    c.pts = drag.o.map(([x, y]) => [x + dx, y + dy]);
  } else if (drag.mode === "move") {
    c.x = drag.o.x + q[0] - drag.p[0]; c.y = drag.o.y + q[1] - drag.p[1];
    clampRect(c);
  } else {
    resize(c, drag.mode === "new" ? "se" : drag.mode, drag.o, q, drag.mode === "new");
  }
  render(false);
});
function endDrag() {
  if (!drag) return;
  const c = st.crops[sel];
  loupe.hidden = true;
  if (drag.mode === "new") {
    if (!c || c.w * scale() < 8 || c.h * scale() < 8) { st.crops.pop(); nextNum--; sel = st.crops.length - 1; drag = null; render(); return; }
    if (drawKind === "quad") toKind(c, "quad"); // starts as the dragged box; the pins then go onto the corners
  }
  if (c && c.kind === "quad") c.pts = orderPins(c.pts);
  if (drag.moved || drag.mode === "new") {
    hist.push(drag.before);
    if (hist.length > 80) hist.shift();
    fut.length = 0;
    setDirty(true);
  }
  drag = null;
  render();
}
frame.addEventListener("pointerup", endDrag);
frame.addEventListener("pointercancel", endDrag);

/** Moves the edges named in `mode` (n, s, e, w) to `q`; keeps the crop's shape if it has one. */
function resize(c, mode, o, q, fromAnchor) {
  let x1 = o.x, y1 = o.y, x2 = o.x + o.w, y2 = o.y + o.h;
  if (fromAnchor) { x2 = q[0]; y2 = q[1]; }
  else {
    if (mode.includes("w")) x1 = q[0];
    if (mode.includes("e")) x2 = q[0];
    if (mode.includes("n")) y1 = q[1];
    if (mode.includes("s")) y2 = q[1];
  }
  const r = ratioValue(c.ratio);
  if (r) {
    const horiz = /[ew]/.test(mode), vert = /[ns]/.test(mode);
    let w = Math.abs(x2 - x1), h = Math.abs(y2 - y1);
    if (fromAnchor || (horiz && vert)) {
      if (w / h > r) w = h * r; else h = w / r;
      const ax = fromAnchor ? o.x : mode.includes("w") ? o.x + o.w : o.x;
      const ay = fromAnchor ? o.y : mode.includes("n") ? o.y + o.h : o.y;
      const right = fromAnchor ? q[0] >= ax : mode.includes("e");
      const down = fromAnchor ? q[1] >= ay : mode.includes("s");
      x1 = right ? ax : ax - w; y1 = down ? ay : ay - h; x2 = x1 + w; y2 = y1 + h;
    } else if (horiz) {
      h = w / r;
      const cy = o.y + o.h / 2;
      x1 = Math.min(x1, x2); x2 = x1 + w; y1 = cy - h / 2; y2 = cy + h / 2;
    } else {
      w = h * r;
      const cx = o.x + o.w / 2;
      y1 = Math.min(y1, y2); y2 = y1 + h; x1 = cx - w / 2; x2 = cx + w / 2;
    }
    if (x1 < -0.5 || y1 < -0.5 || x2 > W + 0.5 || y2 > H + 0.5) return; // would leave the picture: stay as it is
  }
  c.x = Math.max(0, Math.min(x1, x2)); c.y = Math.max(0, Math.min(y1, y2));
  c.w = Math.min(W, Math.max(x1, x2)) - c.x; c.h = Math.min(H, Math.max(y1, y2)) - c.y;
}

// ---------- keys ----------

document.addEventListener("keydown", (e) => {
  const typing = e.target.closest && e.target.closest("input, select");
  const mod = isMac ? e.metaKey : e.ctrlKey;
  if (mod && e.key.toLowerCase() === "z" && !typing) { e.preventDefault(); e.shiftKey ? redo() : undo(); return; }
  if (mod && e.key.toLowerCase() === "y" && !typing) { e.preventDefault(); redo(); return; }
  if (mod && e.key.toLowerCase() === "s") { e.preventDefault(); save(); return; }
  if (typing) return;
  if ((e.key === "Delete" || e.key === "Backspace") && st.crops[sel]) { e.preventDefault(); removeCrop(sel); return; }
  if (e.key === "Escape") { sel = -1; render(); return; }
  if (e.key === "r" || e.key === "R") return setDraw("rect");
  if (e.key === "q" || e.key === "Q") return setDraw("quad");
  const c = st.crops[sel];
  if (!c) return;
  const step = e.shiftKey ? 50 : 1;
  const mv = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, -step], ArrowDown: [0, step] }[e.key];
  if (!mv) return;
  e.preventDefault();
  record();
  if (c.kind === "quad") {
    const b = bbox(c.pts);
    const dx = Math.max(-b.x, Math.min(W - b.x - b.w, mv[0])), dy = Math.max(-b.y, Math.min(H - b.y - b.h, mv[1]));
    c.pts = c.pts.map(([x, y]) => [x + dx, y + dy]);
  } else { c.x += mv[0]; c.y += mv[1]; clampRect(c); }
  render();
});

// ---------- toolbar and panel ----------

function setDraw(kind) {
  drawKind = kind;
  document.querySelectorAll("[data-draw]").forEach((b) => b.setAttribute("aria-pressed", String(b.dataset.draw === kind)));
  $("hint").textContent = kind === "quad" ? "Drag to add a 4-corner crop, then move its pins onto the corners" : "Drag on the picture to add a crop";
}
document.querySelectorAll("[data-draw]").forEach((b) => b.addEventListener("click", () => setDraw(b.dataset.draw)));
document.querySelectorAll("[data-kind]").forEach((b) => b.addEventListener("click", () => {
  const c = st.crops[sel];
  if (!c || c.kind === b.dataset.kind) return;
  record(); toKind(c, b.dataset.kind); render();
}));
document.querySelectorAll("[data-ratio]").forEach((b) => b.addEventListener("click", () => {
  ratioKey = b.dataset.ratio;
  const c = st.crops[sel];
  if (c) { record(); c.ratio = ratioKey; applyRatio(c); }
  render();
}));
$("square").addEventListener("click", () => { const c = st.crops[sel]; if (c) { record(); toKind(c, "rect"); render(); } });
$("edges").addEventListener("click", () => {
  const c = st.crops[sel];
  if (!c || c.kind !== "quad") return;
  // Looks around the crop (a quarter more on each side) for a light shape on a darker background.
  const b = bbox(c.pts), mx = b.w * 0.25, my = b.h * 0.25;
  const x0 = Math.max(0, b.x - mx), y0 = Math.max(0, b.y - my), x1 = Math.min(W, b.x + b.w + mx), y1 = Math.min(H, b.y + b.h + my);
  const f = Math.min(1, 480 / Math.max((x1 - x0) * k, (y1 - y0) * k));
  const tw = Math.max(8, Math.round((x1 - x0) * k * f)), th = Math.max(8, Math.round((y1 - y0) * k * f));
  const t = document.createElement("canvas");
  t.width = tw; t.height = th;
  const g = t.getContext("2d", { willReadFrequently: true });
  g.drawImage(img, x0 * k, y0 * k, (x1 - x0) * k, (y1 - y0) * k, 0, 0, tw, th);
  const q = findEdges(g.getImageData(0, 0, tw, th).data, tw, th);
  if (!q) return toast("Couldn't find clear edges there. Drag the pins onto the corners instead.");
  record();
  c.pts = orderPins(q.map(([u, v]) => [x0 + u * (x1 - x0), y0 + v * (y1 - y0)]));
  delete c.outW; delete c.outH;
  render();
});

/** Turns every crop with the picture: `fp` maps a point, `order` re-reads the pins so the first stays top left. */
function turnAll(fp, order, swapOut) {
  for (const c of st.crops) {
    if (c.kind === "quad") {
      const moved = c.pts.map(fp);
      c.pts = order.map((i) => moved[i]);
      if (swapOut && c.outW) [c.outW, c.outH] = [c.outH, c.outW];
    } else Object.assign(c, bbox(rectPts(c).map(fp)));
  }
}
$("rotr").addEventListener("click", () => { record(); const h = H; turnAll(([x, y]) => [h - y, x], [3, 0, 1, 2], true); st.turn = (st.turn + 1) % 4; drawPreview(); render(); });
$("rotl").addEventListener("click", () => { record(); const w = W; turnAll(([x, y]) => [y, w - x], [1, 2, 3, 0], true); st.turn = (st.turn + 3) % 4; drawPreview(); render(); });
$("flip").addEventListener("click", () => {
  record();
  const w = W;
  turnAll(([x, y]) => [w - x, y], [1, 0, 3, 2], false);
  // A flip after a quarter turn is a flip the other way round before it: keep "flip, then turn".
  if (st.turn % 2) st.turn = (st.turn + 2) % 4;
  st.flip = !st.flip;
  drawPreview(); render();
});
$("undo").addEventListener("click", undo);
$("redo").addEventListener("click", redo);

$("nm").addEventListener("input", () => {
  const c = st.crops[sel];
  if (!c) return;
  c.name = $("nm").value;
  setDirty(true);
  renderList();
});
$("nm").addEventListener("change", () => { const c = st.crops[sel]; if (c && !c.name.trim()) { c.name = `${stem} - ${sel + 1}`; renderList(); } });
for (const id of ["w", "h", "px", "py"]) {
  $(id).addEventListener("change", () => {
    const c = st.crops[sel], v = Math.round(+$(id).value);
    if (!c || !(v >= 0)) return renderFields();
    record();
    const r = ratioValue(c.ratio);
    if (c.kind === "quad") {
      const [ow, oh] = outSize(c);
      c.outW = id === "w" ? Math.max(1, v) : ow;
      c.outH = id === "h" ? Math.max(1, v) : oh;
      if (r) { if (id === "w") c.outH = Math.round(c.outW / r); else c.outW = Math.round(c.outH * r); }
    } else {
      if (id === "w") { c.w = Math.max(1, v); if (r) c.h = c.w / r; }
      if (id === "h") { c.h = Math.max(1, v); if (r) c.w = c.h * r; }
      if (id === "px") c.x = v;
      if (id === "py") c.y = v;
      clampRect(c);
    }
    render();
  });
}
fmt.addEventListener("change", renderList);
$("whole").addEventListener("change", renderList);

// ---------- save ----------

let toastTimer;
function toast(text) {
  const t = $("toast");
  t.textContent = text;
  t.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (t.hidden = true), 4500);
}

async function save() {
  if (busy || !src) return;
  const items = st.crops.map((c) => {
    const name = c.name.trim();
    if (c.kind === "rect") return { kind: "rect", name, x: Math.round(c.x), y: Math.round(c.y), w: Math.max(1, Math.round(c.w)), h: Math.max(1, Math.round(c.h)) };
    const [w, h] = outSize(c);
    return { kind: "quad", name, pts: c.pts.map(([x, y]) => [x, y]), w, h };
  });
  if ($("whole").checked) items.push({ kind: "whole", name: "" });
  if (!items.length) return;
  busy = true; renderList();
  try {
    await tauri.core.invoke("imgedit_save", { request: { turn: st.turn, flip: st.flip, format: fmt.value, items } });
    const n = items.length;
    toast(`Saving ${n === 1 ? "1 file" : n + " files"} next to ${fileName}. The corner card shows when they're done.`);
    setDirty(false);
  } catch (e) {
    toast(String(e && e.message ? e.message : e));
  } finally {
    busy = false; renderList();
  }
}
$("save").addEventListener("click", save);

// ---------- start ----------

function loadImage(bytes) {
  return new Promise((resolve, reject) => {
    const data = bytes instanceof ArrayBuffer ? new Uint8Array(bytes) : Uint8Array.from(bytes);
    const url = URL.createObjectURL(new Blob([data]));
    const im = new Image();
    im.onload = () => resolve(im);
    im.onerror = () => { URL.revokeObjectURL(url); reject(new Error("This picture can't be shown here.")); };
    im.src = url;
  });
}

async function main() {
  addEventListener("resize", () => { if (src) { fit(); render(false); } });
  tauri.event.listen("editor-close-requested", () => {
    if (!dirty || !st.crops.length || window.confirm("Close without saving these crops?")) tauri.core.invoke("imgedit_close");
  });
  try {
    const info = await tauri.core.invoke("imgedit_file");
    if (info.accent) document.documentElement.style.setProperty("--accent", info.accent);
    fileName = info.name;
    stem = fileName.replace(/\.[^.]+$/, "") || fileName;
    $("st-name").textContent = fileName;
    setDirty(false);
    for (const f of info.formats) {
      const o = document.createElement("option");
      o.value = f.id; o.textContent = f.label;
      fmt.appendChild(o);
    }
    fmt.value = info.format;
    if (info.asIs) {
      try {
        src = await loadImage(await tauri.core.invoke("imgedit_bytes"));
        W0 = src.naturalWidth; H0 = src.naturalHeight;
      } catch { src = null; }
    }
    if (!src) {
      $("loading-text").textContent = info.raw ? "Developing the RAW photo…" : "Getting the picture ready…";
      const size = await tauri.core.invoke("imgedit_prepare");
      src = await loadImage(await tauri.core.invoke("imgedit_preview_bytes"));
      W0 = size.width; H0 = size.height;
    }
    $("loading").hidden = true;
    frame.hidden = false;
    $("hint").hidden = false;
    drawPreview();
    render();
  } catch (e) {
    $("loading").querySelector(".spin").hidden = true;
    $("loading-text").textContent = String(e && e.message ? e.message : e);
  }
}
main();
