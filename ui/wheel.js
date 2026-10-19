// Convertino wheel. Rust sends a model ("wheel-open") with the slots to show;
// this file draws the ring, handles mouse and keyboard, and reports the pick.

const CX = 210, CY = 190;          // wheel centre inside the window (see wheel.css)
const D = 340, R = D / 2;
const RI = R * 0.38;               // inner radius of the ring (hub edge)
const RO = R - 7;                  // outer radius of the ring
const FAR = R + 44;                // pointing further than this cancels the highlight

const ICONS = {
  image: '<rect x="4" y="5" width="16" height="14" rx="2"/><path d="M4 16l5-5 4 4 3-3 4 4"/><circle cx="15.5" cy="9" r="1.4"/>',
  audio: '<path d="M9 18V6l10-2v12"/><circle cx="6.5" cy="18" r="2.5"/><circle cx="16.5" cy="16" r="2.5"/>',
  video: '<rect x="3" y="6" width="13" height="12" rx="2"/><path d="M16 10l5-3v10l-5-3z"/>',
  doc: '<path d="M6 3h8l4 4v14H6z"/><path d="M14 3v4h4M9 11h6M9 14h6M9 17h4"/>',
  table: '<rect x="4" y="5" width="16" height="14" rx="2"/><path d="M4 10h16M4 14.5h16M10 5v14"/>',
  code: '<path d="M9 7l-5 5 5 5M15 7l5 5-5 5"/>',
  archive: '<rect x="4" y="4" width="16" height="16" rx="2"/><path d="M12 4v2M12 8v2M12 12v2"/><rect x="10.5" y="15" width="3" height="3" rx=".5"/>',
  compress: '<path d="M9 4v5H4M15 4v5h5M9 20v-5H4M15 20v-5h5"/>',
  split: '<circle cx="6" cy="7" r="2.5"/><circle cx="6" cy="17" r="2.5"/><path d="M8 8.5L20 17M8 15.5L20 7"/>',
  merge: '<path d="M6 4v5c0 3 6 3 6 6v5M18 4v5c0 3-6 3-6 6"/>',
  resize: '<path d="M4 14v6h6M20 10V4h-6M4 20l6-6M20 4l-6 6"/>',
  trim: '<path d="M4 12h1M7 9v6M10 5v14M13 8v8M16 10v4M19 12h1"/>',
  extract: '<path d="M3 7a2 2 0 012-2h4l2 2h8a2 2 0 012 2v8a2 2 0 01-2 2H5a2 2 0 01-2-2z"/><path d="M12 10v6M9 13l3 3 3-3"/>',
  frames: '<rect x="3" y="5" width="18" height="14" rx="2"/><path d="M7 5v14M17 5v14M3 9h4M3 15h4M17 9h4M17 15h4"/>',
  gif: '<rect x="3" y="6" width="18" height="12" rx="2"/><path d="M8 10.5a2 2 0 100 3h1v-1.5M12 10v4M15 14v-4h2.5M15 12h2"/>',
  level: '<path d="M4 12h2M8 8v8M12 4v16M16 8v8"/><path d="M3 20h18"/>',
  more: '<circle cx="6" cy="12" r="1.2"/><circle cx="12" cy="12" r="1.2"/><circle cx="18" cy="12" r="1.2"/>',
  compare: '<rect x="3" y="5" width="18" height="14" rx="2"/><path d="M12 3v18M7 10l-2 2 2 2M17 10l2 2-2 2"/>',
  back: '<path d="M10 6l-6 6 6 6M4 12h16"/>',
  pdf: '<path d="M6 3h8l4 4v14H6z"/><path d="M14 3v4h4"/><path d="M8.5 16.5c2-1 4.5-5.5 3.5-6.5s-1 3 3.5 5"/>',
  camera: '<path d="M4 8h3l2-3h6l2 3h3v11H4z"/><circle cx="12" cy="13.5" r="3.2"/>',
  edit: '<path d="M4 20h4L19 9l-4-4L4 16z"/><path d="M13.5 6.5l4 4"/>',
  join: '<rect x="3" y="7" width="7" height="10" rx="1.5"/><rect x="14" y="7" width="7" height="10" rx="1.5"/><path d="M10 12h4"/>',
  fit: '<rect x="4" y="4" width="16" height="16" rx="2"/><path d="M9 9l6 6M15 9v6H9"/>',
  pin: '<path d="M12 21s-6-5.6-6-11a6 6 0 0112 0c0 5.4-6 11-6 11z"/><path d="M4 4l16 16"/>',
  eye: '<path d="M2 12s3.6-7 10-7 10 7 10 7-3.6 7-10 7S2 12 2 12z"/><circle cx="12" cy="12" r="3"/>',
  folder: '<path d="M3 7a2 2 0 012-2h4l2 2h8a2 2 0 012 2v8a2 2 0 01-2 2H5a2 2 0 01-2-2z"/>',
  font: '<path d="M3.5 19L8.5 5l5 14M5.3 14h6.4"/><circle cx="17.5" cy="16.2" r="2.8"/><path d="M20.3 13v6"/>',
  tag: '<path d="M3.5 12.5l8.5-8.5h8v8l-8.5 8.5z"/><circle cx="15.8" cy="8.2" r="1.5"/>',
  mute: '<path d="M4 9.5h3.5L12 6v12l-4.5-3.5H4z"/><path d="M16 9.5l5 5M21 9.5l-5 5"/>',
  install: '<path d="M12 4v11M7 10l5 5 5-5M5 20h14"/>',
  subtitles: '<rect x="3" y="5" width="18" height="14" rx="2"/><path d="M7 11h10M7 15h4M13 15h4"/>',
  spark: '<path d="M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8z"/><path d="M18.5 15.5l.7 1.8 1.8.7-1.8.7-.7 1.8-.7-1.8-1.8-.7 1.8-.7z"/>',
};
const icon = (name, size = 20) =>
  `<svg viewBox="0 0 24 24" width="${size}" height="${size}" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${ICONS[name] || ICONS.doc}</svg>`;
const fileTile = (color, size = 28) =>
  `<svg width="${size}" height="${size * 1.2}" viewBox="0 0 20 24" aria-hidden="true"><path d="M3.5 1h9L17 5.5v17H3.5z" fill="var(--hub-bg)" stroke="var(--ink-3)"/><path d="M12.5 1v4.5H17" fill="none" stroke="var(--ink-3)"/><rect x="3.5" y="15" width="13.5" height="7.5" fill="${color}"/></svg>`;
// A folder's tile in the hub.
const folderTile = (size = 34) =>
  `<svg width="${size}" height="${Math.round(size * 0.82)}" viewBox="0 0 44 36" aria-hidden="true"><path d="M2 6a3 3 0 013-3h11l4 4h19a3 3 0 013 3v21a3 3 0 01-3 3H5a3 3 0 01-3-3z" fill="#E8B84A"/><path d="M2 12h40v19a3 3 0 01-3 3H5a3 3 0 01-3-3z" fill="#F6CF65"/></svg>`;
const isFolder = () => !!(model && model.folder);
const tileFor = (size) => (isFolder() ? folderTile(size ? size + 6 : 34) : fileTile(model.hubColor, size));
const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

const wheelEl = document.getElementById("wheel");
const hintEl = document.getElementById("hint");
const ringEl = document.getElementById("ring");
const ringProg = document.getElementById("ring-prog");
const ringIn = document.getElementById("ring-in");
const ringText = document.getElementById("ring-text");
const ringUndo = document.getElementById("ring-undo");
const ringCompare = document.getElementById("ring-compare");
const ringShow = document.getElementById("ring-show");
const RING_C = 2 * Math.PI * 32;     // circumference of the ring's arc
const HANDOFF_MS = 2000;             // default; Settings can change it (model.handoffMs)
const tauri = window.__TAURI__;

let model = null;
let slots = [];
let page = "main";
let active = -1;
let busy = false; // true while closing, so a double click can't pick twice

// ---------- drawing ----------

function point(r, deg) {
  const a = (deg * Math.PI) / 180;
  return `${(R + r * Math.cos(a)).toFixed(2)} ${(R + r * Math.sin(a)).toFixed(2)}`;
}
function sector(a0, a1) {
  const large = a1 - a0 > 180 ? 1 : 0;
  return `M${point(RO, a0)} A${RO} ${RO} 0 ${large} 1 ${point(RO, a1)} L${point(RI, a1)} A${RI} ${RI} 0 ${large} 0 ${point(RI, a0)} Z`;
}
function arc(r, a0, a1) {
  return `M${point(r, a0)} A${r} ${r} 0 ${a1 - a0 > 180 ? 1 : 0} 1 ${point(r, a1)}`;
}

function draw() {
  const n = slots.length;
  const step = 360 / n;
  const gap = n > 1 ? 1.1 : 0;
  const segs = slots
    .map((s, i) => {
      const c = -90 + i * step;
      const cls = s.smart ? (s.blocked ? " smart blocked" : " smart") : "";
      return `<path class="seg${cls}" data-i="${i}" d="${sector(c - step / 2 + gap, c + step / 2 - gap)}"/>` +
        `<path class="arc" data-i="${i}" d="${arc(R - 2.5, c - step / 2 + 3, c + step / 2 - 3)}"/>`;
    })
    .join("");
  const rm = (RI + RO) / 2 + 2;
  const labels = slots
    .map((s, i) => {
      const a = ((-90 + i * step) * Math.PI) / 180;
      const x = (R + rm * Math.cos(a)).toFixed(1), y = (R + rm * Math.sin(a)).toFixed(1);
      return `<div class="slot${s.icon ? "" : " sized"}${s.smart ? " smart" : ""}${s.blocked ? " blocked" : ""}" role="menuitem"${s.blocked ? ' aria-disabled="true"' : ""} data-i="${i}" style="left:${x}px;top:${y}px;transition-delay:${i * 14}ms">` +
        (s.tag ? `<span class="tag">${esc(s.tag)}</span>` : "") +
        (s.icon ? icon(s.icon) : "") + `<span class="lbl">${esc(s.label)}</span>` +
        (s.sub ? `<span class="sub">${esc(s.sub)}</span>` : "") +
        (i < 9 && !s.tag && !size ? `<span class="key">${i + 1}</span>` : "") + `</div>`;
    })
    .join("");
  const hub = size ? sizeHub() : `<div class="hub">${tileFor()}<div class="hn" title="${esc(model.hubTitle)}">${esc(model.hubTitle)}</div><div class="hm">${esc(model.hubSubtitle)}</div>${isFolder() && model.folder.blank ? '<div class="hm sub">this folder</div>' : ""}</div>`;
  const askBtn = showAsk() ? `<button type="button" class="ask" id="ask" aria-label="Ask Claude about ${esc(selectedWords())}">${icon("spark", 18)}<span>Ask Claude</span><span class="key">C</span></button>` : "";
  document.body.classList.toggle("with-ask", !!askBtn);
  wheelEl.innerHTML = `<svg viewBox="0 0 ${D} ${D}" aria-hidden="true">${segs}</svg>${labels}${hub}${askBtn}`;
  if (size) wireSizeHub();
  setActive(size && size.custom ? slots.length - 1 : -1);
}

// ---------- Ask Claude ----------
// Shown only when Convertino is connected to Claude (Rust sends model.ask).
// A click starts a new Claude chat with the paths of everything selected;
// with Claude Code connected too, Shift+click (or Shift+C) asks which.

let askHover = false;
const showAsk = () => !!(model && model.ask && !size);
const selectedCount = () => (model ? model.files.length + model.skipped.length : 0);
function selectedWords() {
  const n = selectedCount();
  return n === 1 ? (model.files[0] ? model.files[0].name : model.hubTitle) : `these ${n} items`;
}
const askMode = (id) => (model.ask.modes.find((m) => m.id === id) || model.ask.modes[0]);

function askHint() {
  const m = askMode(model.ask.default);
  const n = selectedCount();
  const more = model.ask.modes.length > 1 ? " · Shift+click to choose where" : "";
  hintEl.innerHTML = `<span class="ht">Ask Claude</span>` +
    `<span class="hs">${esc(m.does.replace("the file paths", n === 1 ? "the file's path" : `the ${n} file paths`))}; you say what to do</span>` +
    `<span class="hk">Click or press C${more}</span>`;
}

function setAskHover(on) {
  if (askHover === on) return;
  askHover = on;
  const b = document.getElementById("ask");
  if (b) b.classList.toggle("on", on);
  if (on) { active = -1; wheelEl.querySelectorAll("[data-i]").forEach((el) => el.classList.remove("on")); askHint(); }
  else setActive(active);
}

function ask(withChoice) {
  if (busy || !showAsk()) return;
  feel("pick");
  if (withChoice && model.ask.modes.length > 1) return showAskOptions();
  askGo(null, false);
}

function askGo(mode, remember) {
  animateOut(() => tauri && tauri.core.invoke("ask_claude", { mode, remember }).catch((e) => console.error(e)));
}

// ---------- feel ----------
// A soft click when the wheel opens, a detent tick when the highlight moves to
// another option, a firmer press when one is picked (Settings > Wheel). Rust decides from the settings whether anything
// plays (feel.rs), so the switch and what you hear can't disagree. Ticks less
// than 35 ms apart are dropped there too; this only saves the round trip.

let lastTick = 0;
function feel(kind) {
  if (!tauri) return;
  if (kind === "tick") {
    const now = performance.now();
    if (now - lastTick < 35) return;
    lastTick = now;
  }
  tauri.core.invoke("wheel_feel", { kind }).catch(() => {});
}

function setActive(i) {
  active = i;
  wheelEl.querySelectorAll("[data-i]").forEach((el) => el.classList.toggle("on", Number(el.dataset.i) === i));
  const s = slots[i];
  if (size) return sizeHint(s);
  if (isFolder()) return folderHint(s);
  if (!s) {
    const skipped = model && model.skipped.length ? ` · ${model.skipped.length} item${model.skipped.length > 1 ? "s" : ""} skipped` : "";
    const smart = slots[0] && slots[0].smart && !slots[0].blocked ? ` · Enter: ${esc(slots[0].label)}` : "";
    hintEl.innerHTML = `<span class="hs">Point at a format${skipped}</span><span class="hk">1–${Math.min(9, slots.length)} to pick${smart} · arrows to move · Esc to cancel</span>`;
    return;
  }
  if (s.smart) return smartHint(s);
  if (s.kind === "compare") return compareHint(s);
  const files = s.kind === "target" && s.count > 1 ? ` · ${s.count} files` : "";
  const how = s.kind !== "target" ? "Click to open" : OPTIONS[s.id] ? "Click to convert · Shift+click for options" : "Click to convert";
  hintEl.innerHTML =
    `<span class="ht">${s.tag ? esc(s.tag) + " → " : ""}${esc(s.label)}</span>` +
    `<span class="hs">${esc(s.hint)}${files}</span>` +
    `<span class="hk">${how}</span>`;
}

// ---------- the smart slot ----------
// What the file is decides it (smart.rs): install the fonts in a zip, join
// an archive's parts, fix a wrong extension… It's first on the ring, tinted,
// and Enter picks it. Its hint says everything it will do.

function smartHint(s) {
  const lines = (s.lines || []).map((l, i) => `<span class="${i === 0 ? "hs hl" : "hk"}">${esc(l)}</span>`).join("");
  const warn = s.warn ? `<span class="hw">${esc(s.warn)}</span>` : "";
  const how = s.blocked ? "Can't be done as it is" : (s.fields && s.fields.length ? "Click or Enter · Shift+click for options · Esc to cancel" : "Click or Enter · Esc to cancel");
  hintEl.innerHTML = `<span class="ht">${esc(s.label)}</span><span class="hs">${esc(s.hint)}</span>` + lines + warn + `<span class="hk">${how}</span>`;
}

function startSmart(s, opts, remember) {
  launch(s, { targetId: s.id, label: s.label, family: "smart", quality: null, remember: !!remember, size: null, smart: opts });
}

function fieldValues(fields) {
  const v = {};
  for (const f of fields || []) v[f.key] = f.value;
  return v;
}

function showSmartOptions(s) {
  optsFor = { smart: s, values: fieldValues(s.fields) };
  document.getElementById("opts-title").textContent = s.label;
  document.querySelector(".opts-sub").textContent = model.files.length === 1 ? model.files[0].name : `${model.files.length} files`;
  optsKeep.nextElementSibling.textContent = "Use these every time";
  optsKeep.checked = false;
  renderSmartFields(s);
  optsEl.hidden = false;
  document.body.classList.add("with-opts");
  hintEl.classList.remove("show");
  const first = optsFields.querySelector("input:checked, select, input");
  if (first) first.focus();
}

function renderSmartFields(s) {
  const v = optsFor.values;
  const html = (s.fields || []).map((f) => {
    if (f.kind === "radio") {
      return `<fieldset class="opts-modes"><legend>${esc(f.label)}</legend>` + f.choices.map((c) =>
        `<label class="opts-mode"><input type="radio" name="s-${esc(f.key)}" data-s="${esc(f.key)}" value="${esc(c.value)}"${String(v[f.key]) === c.value ? " checked" : ""}>` +
        `<span><b>${esc(c.label)}</b>${c.sub ? `<span>${esc(c.sub)}</span>` : ""}</span></label>`).join("") + `</fieldset>`;
    }
    if (f.kind === "select") {
      return `<label class="opts-field"><span>${esc(f.label)}</span><select data-s="${esc(f.key)}">` +
        f.choices.map((c) => `<option value="${esc(c.value)}"${String(v[f.key]) === c.value ? " selected" : ""}>${esc(c.label)}</option>`).join("") + `</select></label>`;
    }
    if (f.kind === "check") {
      return `<label class="opts-check"><input type="checkbox" data-s="${esc(f.key)}"${v[f.key] ? " checked" : ""}><span>${esc(f.label)}${f.sub ? `<small>${esc(f.sub)}</small>` : ""}</span></label>`;
    }
    // A folder: "folder" with a checkbox, "path" without one; Change… picks it.
    const path = v[f.key];
    const shown = path ? `<small class="opts-path">${esc(path)}</small>` : `<small>No folder chosen yet</small>`;
    const box = f.kind === "folder" ? `<input type="checkbox" data-s="${esc(f.key)}" data-folder="1"${path ? " checked" : ""}>` : "";
    return `<div class="opts-check opts-folder">${box}<span>${esc(f.label)}${shown}</span><button type="button" class="opts-change" data-pick="${esc(f.key)}">Change…</button></div>`;
  }).join("");
  optsFields.innerHTML = html + `<p class="opts-note" id="smart-lines"></p>`;
  smartPreviewShow(s);
}

/// The button and the lines under the options, as Rust works them out for these choices.
function smartPreviewShow(o) {
  document.getElementById("opts-go").textContent = o.go || o.label;
  document.getElementById("opts-go").disabled = !!o.blocked || /^Nothing to /.test(o.go || "");
  const lines = [o.hint].concat(o.lines || []).filter(Boolean);
  const el = document.getElementById("smart-lines");
  if (el) el.textContent = lines.join(" · ");
}

let smartPreviewN = 0;
function smartPreview() {
  if (!optsFor || !optsFor.smart || !tauri) return;
  const n = ++smartPreviewN;
  tauri.core.invoke("smart_preview", { targetId: optsFor.smart.id, opts: optsFor.values }).then((o) => {
    if (n !== smartPreviewN || !optsFor || !optsFor.smart || !o) return;
    smartPreviewShow(o);
  }).catch((e) => console.error(e));
}

function smartOptionInput(e) {
  const t = e.target;
  const k = t.dataset.s;
  if (!k) return;
  if (t.dataset.folder) {
    if (!t.checked) { optsFor.values[k] = null; renderSmartFields(optsFor.smart); return smartPreview(); }
    if (!optsFor.values[k]) return pickSmartFolder(k, t);
    return;
  }
  optsFor.values[k] = t.type === "checkbox" ? t.checked : t.value;
  smartPreview();
}

function pickSmartFolder(k, box) {
  if (!tauri) return;
  const f = (optsFor.smart.fields || []).find((x) => x.key === k);
  tauri.core.invoke("smart_pick_folder", { title: f ? f.label : "Choose a folder" }).then((p) => {
    if (!optsFor || !optsFor.smart) return;
    if (p) optsFor.values[k] = p;
    else if (box) box.checked = !!optsFor.values[k];
    renderSmartFields(optsFor.smart);
    smartPreview();
  }).catch((e) => console.error(e));
}

// ---------- Compare ----------
// The last slot when compressed copies were found among what was clicked
// (pairs.rs): opens the Compare window with each copy and its original.

function compareHint(s) {
  const lines = (s.detail || "").split(" · ").filter(Boolean);
  hintEl.innerHTML = `<span class="ht">${esc(s.hint)}</span>` +
    lines.map((l) => `<span class="${/didn't pass/.test(l) ? "hw" : "hs hl"}">${esc(l)}</span>`).join("") +
    `<span class="hk">Click to open Compare</span>`;
}

// ---------- folders ----------
// A folder's wheel: slots do something to everything inside (folder.rs).
// Each slot's hint says what it touches, what it leaves alone and where the
// result goes; Shift+click opens the folder options.

const joinCheck = { words: null, pending: false, subfolders: null };

function folderHint(s) {
  const f = model.folder;
  if (f.scanning !== null && f.scanning !== undefined) {
    hintEl.innerHTML = `<span class="ht">Reading ${esc(f.name)}…</span><span class="hs">${esc(f.scanning.toLocaleString())} files so far</span><span class="hk">Esc to cancel</span>`;
    return;
  }
  if (!s) {
    const alone = f.alone ? ` · left alone: ${f.aloneWords}` : "";
    const earlier = f.earlier ? ` · ${f.earlier} made by Convertino earlier` : "";
    const subs = f.subfolders ? ` · ${f.subfolders} subfolder${f.subfolders === 1 ? "" : "s"}` : "";
    const cut = f.truncated ? " · only the first 200,000 files were read" : "";
    hintEl.innerHTML = `<span class="hs">Point at an action · ${esc(model.hubSubtitle)}${esc(subs)}${esc(earlier)}${esc(alone)}${esc(cut)}</span>` +
      `<span class="hk">1–${Math.min(9, slots.length)} to pick · Shift+click for options · Esc to cancel</span>`;
    return;
  }
  if (s.kind === "compare") return compareHint(s);
  if (s.kind !== "target") {
    hintEl.innerHTML = `<span class="ht">${esc(s.label)}</span><span class="hs">${esc(s.hint)}</span><span class="hk">Click to open</span>`;
    return;
  }
  let what = s.hint;
  if (s.id === "folder.join-video") {
    if (joinCheck.words && joinCheck.subfolders === f.defaults.subfolders) what = joinCheck.words;
    else { what = "Checking the clips…"; checkJoin(); }
  }
  const how = s.id === "folder.watch" ? "Click to set it up"
    : s.id === "folder.fit" ? "Click to pick a size"
    : FOLDER_OPTIONS[folderKind(s.id)] ? "Click to start · Shift+click for options" : "Click to start";
  hintEl.innerHTML =
    `<span class="ht">${esc(s.label)}</span>` + (s.sub && s.id !== "folder.watch" ? `<span class="hs">${esc(s.sub)}</span>` : "") +
    `<span class="hc">${esc(what)}</span>` +
    (s.detail ? `<span class="hk">${esc(s.detail)}</span>` : "") +
    (s.out ? `<span class="hk">${esc(s.out)}</span>` : "") +
    `<span class="hk">${how}</span>`;
}

function checkJoin() {
  if (joinCheck.pending || !tauri) return;
  joinCheck.pending = true;
  const sub = model.folder.defaults.subfolders;
  tauri.core.invoke("folder_join_check", { subfolders: sub }).then((r) => {
    joinCheck.words = r.words; joinCheck.subfolders = sub; joinCheck.pending = false;
    if (active >= 0 && slots[active] && slots[active].id === "folder.join-video" && !size) setActive(active);
  }).catch((e) => { joinCheck.words = String(e); joinCheck.subfolders = sub; joinCheck.pending = false; });
}

/// The options group a folder slot uses.
function folderKind(id) {
  if (id === "folder.compress") return "compress";
  if (id.startsWith("folder.to:") || id === "folder.audio" || id === "folder.normalize" || id === "folder.gps") return "batch";
  if (id === "folder.pdf" || id === "folder.merge" || id === "folder.join-video" || id === "folder.join-audio" || id === "folder.sheets") return "ordered";
  if (id === "folder.sequence") return "sequence";
  if (id === "folder.zip") return "zip";
  return "";
}
const FOLDER_OPTIONS = { compress: true, batch: true, ordered: true, sequence: true, zip: true };

function folderDefaults() {
  const d = (model.folder && model.folder.defaults) || { into: "new", subfolders: true, skipDone: true };
  return { into: d.into, subfolders: d.subfolders, skipDone: d.skipDone, order: "name", fps: 24, format: "mp4", sound: false, gaps: "hold", pack: "zip", password: null, parts: null, fit: null };
}

/// How many files a slot touches with or without subfolders (the count on the ring is with the saved choice).
function folderCount(s, subfolders) {
  if (subfolders === model.folder.defaults.subfolders) return s.count;
  const top = model.folder.topCounts || {};
  if (!subfolders && top[s.id] !== undefined) return top[s.id];
  return s.count;
}

function folderGo(s, opts, quality, remember) {
  launch(s, { cmd: "folder_pick", actionId: s.id, label: s.label, opts, quality, remember: !!remember });
}

const FOLDER_VERB = { compress: "Compress", batch: "Convert", ordered: "Make", sequence: "Make", zip: "Zip" };
const PARTS = [[0, "One file"], [18e6, "18 MB pieces (email)"], [100e6, "100 MB pieces"], [500e6, "500 MB pieces"], [2e9, "2 GB pieces (WhatsApp)"], [4e9, "4 GB pieces (USB stick)"]];

function showFolderOptions(s) {
  const kind = folderKind(s.id);
  const f = model.folder;
  const o = folderDefaults();
  const q = Object.assign({}, DEFAULT_Q, (model && model.quality) || {});
  optsFor = { slot: s, folder: o, q, kind };
  document.getElementById("opts-title").textContent = s.label;
  document.querySelector(".opts-sub").textContent = f.name;
  optsKeep.nextElementSibling.textContent = "Use these every time";
  optsKeep.checked = false;
  optsKeep.closest("label").hidden = !(kind === "compress" || kind === "batch");
  const radio = (name, v, label, sub, on) => `<label class="opts-mode"><input type="radio" name="${name}" value="${v}"${on ? " checked" : ""}><span><b>${esc(label)}</b><span>${esc(sub)}</span></span></label>`;
  const check = (k, label, sub, on, disabled) => `<label class="opts-check"><input type="checkbox" data-f="${k}"${on ? " checked" : ""}${disabled ? " disabled" : ""}><span>${esc(label)}${sub ? `<small>${esc(sub)}</small>` : ""}</span></label>`;
  const select = (k, label, options, value) => `<label class="opts-field"><span>${esc(label)}</span><select data-f="${k}">${options.map(([v, l]) => `<option value="${esc(v)}"${String(v) === String(value) ? " selected" : ""}>${esc(l)}</option>`).join("")}</select></label>`;
  const suffix = s.id === "folder.compress" ? "compressed" : s.id === "folder.normalize" ? "normalized" : s.id === "folder.gps" ? "no GPS" : s.id === "folder.audio" ? "mp3" : s.label.replace(/^.* → /, "").toLowerCase();
  const subs = f.subfolders ? check("subfolders", "Include subfolders", `${f.subfolders} subfolder${f.subfolders === 1 ? "" : "s"}, ${f.inSubfolders} of the ${f.files} files`, o.subfolders) : "";
  let html = "";
  if (kind === "compress" || kind === "batch") {
    html += `<fieldset class="opts-modes"><legend>Put the results</legend>` +
      radio("into", "new", "In a new folder next to it", `${f.name} (${suffix}), same subfolders inside`, o.into === "new") +
      radio("into", "beside", "Next to each original", `“name (${suffix})” beside each file`, o.into === "beside") +
      radio("into", "replace", "Replace the originals", "Each original moves to the Holding Area once its new copy looks, sounds and plays the same", o.into === "replace") +
      `</fieldset>` + subs +
      check("skipDone", "Skip files already done", "A result from an earlier run that's still there and newer than the original", o.skipDone);
    if (kind === "compress") html += select("look", "Look", [["small", "Smaller: looks very close"], ["balanced", "Balanced: looks the same"], ["best", "Best: the same, even side by side"]], q.video);
  } else if (kind === "ordered") {
    html += select("order", "Order", [["name", "By name (2 before 10)"], ["date", "By date modified"]], o.order) + subs;
  } else if (kind === "sequence") {
    const seq = f.sequence || { frames: s.count, gaps: 0 };
    html += select("fps", "Frame rate", [[23.976, "23.976 fps"], [24, "24 fps"], [25, "25 fps"], [30, "30 fps"], [50, "50 fps"], [60, "60 fps"]], o.fps) +
      select("format", "Make", [["mp4", "MP4 (H.264), looks the same"], ["h265", "MP4 (H.265), smaller"], ["prores", "ProRes 422 HQ (.mov, for editing)"], ["webm", "WebM (VP9, keeps transparency)"], ["gif", "GIF (480 px, 15 fps)"]], o.format) +
      (f.sound ? check("sound", "Add sound", `Uses the audio file in the folder: ${f.sound}`, o.sound) : "") +
      (seq.gaps ? select("gaps", `${seq.gaps} frame${seq.gaps === 1 ? " is" : "s are"} missing`, [["hold", "Hold the previous frame"], ["stop", "Stop and say which"]], o.gaps) : "");
  } else if (kind === "zip") {
    html += select("pack", "Format", [["zip", "ZIP (opens anywhere)"], ["7z", "7Z (smaller, needs 7-Zip)"], ["targz", "TAR.GZ"]], o.pack) +
      `<label class="opts-field"><span>Password <small>(optional)</small></span><input type="password" data-f="password" autocomplete="new-password" placeholder="No password"></label>` +
      `<p class="opts-note" data-note="pw" hidden>Password-protected archives use AES-256: open them with 7-Zip or Convertino (Windows' built-in zip can't).</p>` +
      select("parts", "Split into", PARTS, 0) + subs;
  }
  optsFields.innerHTML = html;
  folderGoLabel();
  optsEl.hidden = false;
  document.body.classList.add("with-opts");
  hintEl.classList.remove("show");
  const first = optsFields.querySelector("input:checked, select, input");
  if (first) first.focus();
}

function folderGoLabel() {
  const { slot, folder, kind } = optsFor;
  const n = folderCount(slot, folder.subfolders);
  const verb = FOLDER_VERB[kind] || "Start";
  const what = kind === "sequence" ? `${(slot.label.includes("GIF") || folder.format === "gif") ? "the GIF" : "the video"}` : `${n} file${n === 1 ? "" : "s"}`;
  document.getElementById("opts-go").textContent = `${verb} ${what}`;
}

function folderOptionInput(e) {
  const t = e.target;
  const o = optsFor.folder;
  if (t.name === "into") o.into = t.value;
  const k = t.dataset.f;
  if (k === "subfolders" || k === "skipDone" || k === "sound") o[k] = t.checked;
  else if (k === "fps") o.fps = Number(t.value);
  else if (k === "parts") o.parts = Number(t.value) || null;
  else if (k === "password") {
    o.password = t.value || null;
    const note = optsFields.querySelector('[data-note="pw"]');
    if (note) note.hidden = !t.value;
  } else if (k === "pack") {
    o.pack = t.value;
    const pw = optsFields.querySelector('[data-f="password"]');
    if (pw) { pw.disabled = t.value === "targz"; if (pw.disabled) { pw.value = ""; o.password = null; } }
    const parts = optsFields.querySelector('[data-f="parts"]');
    if (parts) { parts.disabled = t.value === "targz"; if (parts.disabled) { parts.value = "0"; o.parts = null; } }
  } else if (k === "look") {
    const g = t.value;
    optsFor.q.video = g; optsFor.q.image = g; optsFor.q.pdfCompress = g === "best" ? "high" : g;
  } else if (k) o[k] = t.value;
  folderGoLabel();
}

// ---------- open / close / pick ----------

let openCount = 0;          // bumps on every open, so a stale pick can tell

function open(m) {
  openCount++;
  feel("open");
  hideOptions();
  resetRing();
  early.clear();
  model = m;
  busy = false;
  joinCheck.words = null; joinCheck.pending = false;
  document.documentElement.dataset.os = m.os;
  if (m.accent) document.documentElement.style.setProperty("--accent", m.accent);
  page = "main";
  size = null;
  askHover = false;
  slots = m.main;
  wheelEl.classList.remove("open", "closing");
  hintEl.classList.remove("show");
  draw();
  requestAnimationFrame(() => requestAnimationFrame(() => {
    wheelEl.classList.add("open");
    hintEl.classList.add("show");
  }));
  window.focus();
}

function animateOut(then) {
  busy = true;
  wheelEl.classList.remove("open");
  wheelEl.classList.add("closing");
  hintEl.classList.remove("show");
  setTimeout(then, 110);
}

function close() {
  if (busy) return;
  animateOut(() => tauri && tauri.core.invoke("wheel_close"));
}

function showPage(p) {
  size = null;
  page = p;
  slots = p === "more" ? model.more : model.main;
  draw();
}

/// `withOptions`: Shift held, so first ask for this conversion's options.
function choose(i, withOptions = false) {
  if (busy) return;
  const s = slots[i];
  if (!s) return;
  if (!(s.smart && s.blocked)) feel("pick");
  if (s.kind === "more") return showPage("more");
  if (s.kind === "back") return showPage("main");
  if (s.kind === "size") return startSize(s.bytes, s.label, null);
  if (s.kind === "custom") return openCustom("");
  if (s.smart) {
    if (s.blocked) return;
    if (withOptions && s.fields && s.fields.length) return showSmartOptions(s);
    return startSmart(s, null, false);
  }
  if (s.kind === "compare") return animateOut(() => tauri && tauri.core.invoke("wheel_compare").catch((e) => console.error(e)));
  if (s.kind === "target" && COMPRESS.has(s.id)) return enterSize(s);
  if (isFolder()) {
    if (s.id === "folder.fit") return enterSize(s);
    if (withOptions && FOLDER_OPTIONS[folderKind(s.id)]) return showFolderOptions(s);
    return folderGo(s, folderDefaults(), null, false);
  }
  if (withOptions && OPTIONS[s.id]) return showOptions(s);
  start(s, null, false);
}

function start(s, quality, remember) {
  launch(s, { targetId: s.id, label: s.label, family: s.family, quality, remember, size: null });
}

function launch(s, args) {
  const cmd = args.cmd || "wheel_pick";
  const call = Object.assign({}, args);
  delete call.cmd;
  // Edit opens an editor window (PDF or image) instead of converting; Watch
  // opens Settings; with the ring turned off in Settings, progress goes
  // straight to the corner card.
  if (/\.edit$/.test(args.targetId || "") || args.actionId === "folder.watch" || (model && model.ring === false)) {
    return animateOut(() => tauri && tauri.core.invoke(cmd, call).catch((e) => console.error(e)));
  }
  pick(s, cmd, call);
}

// ---------- Shift+click options ----------
// The values start from Settings (model.quality); "Use these every time" saves them there.

const SIZES = [[0, "Don't resize"], [3840, "Up to 3840 px"], [2560, "Up to 2560 px"], [1920, "Up to 1920 px"], [1280, "Up to 1280 px"]];
const FIELDS = {
  image: { label: "Picture quality", options: [["small", "Smaller: looks very close"], ["balanced", "Balanced: looks the same"], ["best", "Best: the same, even side by side"], ["fixed", "Fixed quality"]] },
  jpg: { label: "Quality (Fixed)", min: 50, max: 100 },
  webp: { label: "Quality (Fixed)", min: 50, max: 100 },
  convertMax: { label: "Resize large pictures", options: SIZES },
  resize: { label: "Longest side", options: [[1280, "1280 px"], [1920, "1920 px"], [2560, "2560 px"], [3840, "3840 px (4K)"]] },
  dpi: { label: "Page resolution", options: [[72, "Screen (72 DPI)"], [150, "Standard (150 DPI)"], [300, "Print (300 DPI)"]] },
  pdfCompress: { label: "PDF compression", options: [["small", "Smaller: looks very close"], ["balanced", "Balanced: looks the same"], ["high", "Best: the same, even side by side"]] },
  mp3: { label: "Quality", options: [[128, "About 130 kbps"], [160, "About 165 kbps"], [192, "About 190 kbps"], [320, "Highest (about 245 kbps)"]] },
  video: { label: "Video quality", options: [["small", "Smaller: looks very close"], ["balanced", "Balanced: looks the same"], ["best", "Best: the same, even side by side"]] },
  gifWidth: { label: "Width", options: [[320, "320 px"], [480, "480 px"], [640, "640 px"]] },
  gifSeconds: { label: "Length", options: [[10, "First 10 s"], [30, "First 30 s"], [60, "First 60 s"]] },
};
const OPTIONS = {
  "image.jpg": ["image", "jpg", "convertMax"], "image.webp": ["image", "webp", "convertMax"],
  "image.png": ["convertMax"], "image.avif": ["image", "convertMax"], "image.tiff": ["convertMax"],
  "image.bmp": ["convertMax"], "image.gif": ["convertMax"], "image.pdf": ["convertMax"],
  "pdf.jpg": ["dpi", "image", "jpg"], "pdf.png": ["dpi"], "pdf.webp": ["dpi", "image", "webp"],
  "audio.mp3": ["mp3"], "video.mp3": ["mp3"],
  "video.mp4": ["video"], "video.mov": ["video"], "video.720p": ["video"], "video.webm": ["video"],
  "video.gif": ["gifWidth", "gifSeconds"],
};
const DEFAULT_Q = { jpg: 90, webp: 85, resize: 1920, convertMax: 0, dpi: 150, pdfCompress: "balanced", mp3: 320, video: "balanced", gifWidth: 480, gifSeconds: 30, image: "balanced" };

// Tune for one conversion: the main settings behind the chosen preset
// (model.presets holds every preset as tuned in Settings).
const GRADE_NAMES = { small: "Smaller", balanced: "Balanced", best: "Best" };
const QUICK_TUNE = {
  image: [
    { k: "look", label: "Quality target (80 = looks the same)", min: 50, max: 95, step: 1 },
    { k: "stripGps", label: "Remove location (GPS)", toggle: true },
  ],
  video: [
    { k: "look", label: "Quality target (93 = looks the same)", min: 80, max: 99, step: 0.5 },
    { k: "encoder", label: "Encoder", options: [["gpu", "Graphics card: fast"], ["cpu", "Processor: smaller files"], ["auto", "Auto: picks per video"]] },
  ],
};

function tuneKindOf(slotId) {
  const keys = OPTIONS[slotId] || [];
  if (keys.includes("video")) return { kind: "video", key: "video" };
  if (keys.includes("image")) return { kind: "image", key: "image" };
  return null;
}

function quickTuneHtml() {
  const tk = optsFor && optsFor.slot && tuneKindOf(optsFor.slot.id);
  if (!tk || !model || !model.presets) return "";
  const grade = optsFor.q[tk.key] === "high" ? "best" : optsFor.q[tk.key];
  if (!GRADE_NAMES[grade]) return "";
  const base = (model.presets[grade] && model.presets[grade][tk.kind]) || {};
  const mine = ((optsFor.q.tune || {})[tk.kind] || {})[grade] || {};
  const val = (k) => (mine[k] !== undefined && mine[k] !== null ? mine[k] : base[k]);
  const rows = QUICK_TUNE[tk.kind].map((f) => {
    const v = val(f.k);
    if (f.toggle) return `<label class="opts-check"><input type="checkbox" data-tune="${f.k}"${v ? " checked" : ""}><span>${esc(f.label)}</span></label>`;
    if (f.options) {
      return `<label class="opts-field"><span>${esc(f.label)}</span><select data-tune="${f.k}">${f.options.map(([o, l]) => `<option value="${o}"${String(o) === String(v) ? " selected" : ""}>${esc(l)}</option>`).join("")}</select></label>`;
    }
    return `<label class="opts-field"><span class="row"><span>${esc(f.label)}</span><span class="val" data-tv="${f.k}">${v}</span></span>` +
      `<input type="range" data-tune="${f.k}" min="${f.min}" max="${f.max}" step="${f.step}" value="${v}"></label>`;
  }).join("");
  return `<details class="opts-tune"${optsFor.tuneOpen ? " open" : ""}><summary>Fine-tune ${GRADE_NAMES[grade]} for this file</summary><div class="opts-fields">${rows}</div></details>`;
}

function renderQuickTune() {
  const old = optsFields.querySelector(".opts-tune");
  if (old) old.remove();
  optsFields.insertAdjacentHTML("beforeend", quickTuneHtml());
}

function setQuickTune(key, value) {
  const tk = tuneKindOf(optsFor.slot.id);
  const grade = optsFor.q[tk.key] === "high" ? "best" : optsFor.q[tk.key];
  const q = optsFor.q;
  q.tune = JSON.parse(JSON.stringify(q.tune || {}));
  q.tune[tk.kind] = q.tune[tk.kind] || {};
  q.tune[tk.kind][grade] = Object.assign({}, q.tune[tk.kind][grade], { [key]: value });
}

const optsEl = document.getElementById("opts");
const optsFields = document.getElementById("opts-fields");
const optsKeep = document.getElementById("opts-keep");
let optsFor = null;   // { slot, q }

function showOptions(s) {
  if (isFolder()) return showFolderOptions(s);
  const q = Object.assign({}, DEFAULT_Q, (model && model.quality) || {});
  optsFor = { slot: s, q };
  document.getElementById("opts-title").textContent = s.label;
  document.getElementById("opts-go").textContent = "Convert";
  document.querySelector(".opts-sub").textContent = "for this conversion";
  optsKeep.nextElementSibling.textContent = "Use these every time";
  optsKeep.checked = false;
  optsFields.innerHTML = OPTIONS[s.id].map((key) => {
    const f = FIELDS[key];
    if (f.options) {
      const opts = f.options.map(([v, label]) => `<option value="${esc(v)}"${String(v) === String(q[key]) ? " selected" : ""}>${esc(label)}</option>`).join("");
      return `<label class="opts-field"><span>${esc(f.label)}</span><select data-k="${key}">${opts}</select></label>`;
    }
    return `<label class="opts-field"><span class="row"><span>${esc(f.label)}</span><span class="val" data-v="${key}">${q[key]}</span></span>` +
      `<input type="range" data-k="${key}" min="${f.min}" max="${f.max}" step="1" value="${q[key]}"></label>`;
  }).join("");
  renderQuickTune();
  optsEl.hidden = false;
  document.body.classList.add("with-opts");
  hintEl.classList.remove("show");
  const first = optsFields.querySelector("select, input");
  if (first) first.focus();
}

function showAskOptions() {
  optsFor = { ask: true, mode: model.ask.default };
  document.getElementById("opts-title").textContent = "Ask Claude";
  document.getElementById("opts-go").textContent = "Open Claude";
  document.querySelector(".opts-sub").textContent = `with ${selectedWords()}`;
  optsKeep.nextElementSibling.textContent = "Use this every time";
  optsKeep.checked = false;
  optsFields.innerHTML = `<fieldset class="opts-modes"><legend>Open in</legend>` + model.ask.modes.map((m) =>
    `<label class="opts-mode"><input type="radio" name="askmode" value="${esc(m.id)}"${m.id === model.ask.default ? " checked" : ""}>` +
    `<span><b>${esc(m.label)}</b><span>${esc(m.does)}</span></span></label>`).join("") + `</fieldset>`;
  optsEl.hidden = false;
  document.body.classList.add("with-opts");
  hintEl.classList.remove("show");
  const first = optsFields.querySelector("input:checked") || optsFields.querySelector("input");
  if (first) first.focus();
}

function hideOptions() {
  optsFor = null;
  const keep = optsKeep && optsKeep.closest("label");
  if (keep) keep.hidden = false;
  if (optsEl) optsEl.hidden = true;
  document.body.classList.remove("with-opts");
}

if (optsEl) {
  optsFields.addEventListener("input", (e) => {
    if (optsFor && optsFor.smart) return e.target.type === "checkbox" || e.target.type === "radio" ? null : smartOptionInput(e);
    if (optsFor && optsFor.folder) return folderOptionInput(e);
    if (optsFor && optsFor.ask && e.target.name === "askmode") { optsFor.mode = e.target.value; return; }
    const tkey = e.target.dataset.tune;
    if (tkey && optsFor) {
      const raw = e.target.type === "checkbox" ? e.target.checked : e.target.type === "range" ? Number(e.target.value) : e.target.value;
      setQuickTune(tkey, raw);
      const tv = optsFields.querySelector(`[data-tv="${tkey}"]`);
      if (tv) tv.textContent = raw;
      return;
    }
    const k = e.target.dataset.k;
    if (!k || !optsFor) return;
    const raw = e.target.value;
    optsFor.q[k] = typeof DEFAULT_Q[k] === "number" ? Number(raw) : raw;
    const v = optsFields.querySelector(`[data-v="${k}"]`);
    if (v) v.textContent = raw;
    // Another preset: its own Tune values.
    if (k === "video" || k === "image") {
      const d = optsFields.querySelector(".opts-tune");
      optsFor.tuneOpen = !!(d && d.open);
      renderQuickTune();
    }
  });
  optsFields.addEventListener("change", (e) => {
    if (optsFor && optsFor.smart) return e.target.tagName === "SELECT" ? null : smartOptionInput(e);
    if (optsFor && optsFor.folder) return folderOptionInput(e);
    if (e.target.type === "checkbox" && e.target.dataset.tune && optsFor) setQuickTune(e.target.dataset.tune, e.target.checked);
  });
  optsEl.addEventListener("submit", (e) => {
    e.preventDefault();
    if (!optsFor) return;
    const remember = optsKeep.checked;
    if (optsFor.ask) {
      const mode = optsFor.mode;
      hideOptions();
      return askGo(mode, remember);
    }
    if (optsFor.folder) {
      const { slot, folder, q, kind } = optsFor;
      hideOptions();
      return folderGo(slot, folder, kind === "compress" ? q : null, remember);
    }
    if (optsFor.smart) {
      const { smart, values } = optsFor;
      hideOptions();
      return startSmart(smart, values, remember);
    }
    const { slot, q } = optsFor;
    hideOptions();
    start(slot, q, remember);
  });
  document.getElementById("opts-cancel").addEventListener("click", (e) => {
    e.stopPropagation();
    hideOptions();
    hintEl.classList.add("show");
  });
  optsEl.addEventListener("click", (e) => {
    e.stopPropagation();
    const pick = e.target.closest && e.target.closest("[data-pick]");
    if (pick && optsFor && optsFor.smart) pickSmartFolder(pick.dataset.pick, null);
  });
  optsEl.addEventListener("keydown", (e) => {
    if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); hideOptions(); hintEl.classList.add("show"); }
  });
}

// ---------- compress to a size ----------
// Compress turns the wheel into a ring of sizes picked for the file(s); each
// says what it will cost. "Custom…", or just typing a number, turns the centre
// into a size box with a live preview under the wheel.

const COMPRESS = new Set(["image.compress", "video.compress", "pdf.compress", "audio.compress"]);
let size = null;   // { slot, together, ring, custom, text, unit, preview, previewFor, loading, error }

function words(bytes) {
  if (bytes >= 1e9) return `${+(bytes / 1e9).toFixed(2)} GB`;
  if (bytes >= 10e6) return `${Math.round(bytes / 1e6)} MB`;
  if (bytes >= 1e6) return `${+(bytes / 1e6).toFixed(1)} MB`;
  return `${Math.max(1, Math.round(bytes / 1e3))} KB`;
}

function enterSize(s) {
  size = { slot: s, together: true, ring: null, custom: false, text: "", unit: "MB", preview: null, previewFor: "", loading: true, error: null,
    folder: s.id === "folder.fit", mode: "folder", measure: null };
  slots = [];
  draw();
  loadPresets();
}

function loadPresets() {
  if (!size) return;
  const mine = size;
  mine.loading = true;
  if (!tauri) return demoPresets(mine);
  const call = mine.folder
    ? tauri.core.invoke("folder_fit_presets", { mode: mine.mode, subfolders: model.folder.defaults.subfolders })
    : tauri.core.invoke("compress_presets", { targetId: mine.slot.id, family: mine.slot.family, together: mine.together });
  call
    .then((r) => { if (size === mine) { mine.ring = r; mine.loading = false; mine.unit = defaultUnit(r); showSizes(); } })
    .catch((e) => { if (size === mine) { mine.loading = false; mine.error = String(e); showSizes(); } });
}

function defaultUnit(r) {
  if (r && r.total >= 5e9) return "GB";
  return r && r.total < 2e6 ? "KB" : "MB";
}
const UNIT = { KB: 1e3, MB: 1e6, GB: 1e9 };
const nextUnit = (u) => (u === "KB" ? "MB" : u === "MB" ? "GB" : "KB");
const FIT_MODES = [["folder", "Folder"], ["zip", "Zip"], ["parts", "Parts"]];
const FIT_SAY = { folder: "A compressed copy of the folder", zip: "One zip, media shrunk just enough", parts: "An untouched zip in pieces" };

function showSizes() {
  const r = size.ring;
  slots = (r ? r.presets : []).map((p) => ({
    kind: "size", id: size.slot.id, family: size.slot.family, icon: null, bytes: p.bytes, preview: p.preview,
    label: p.label, sub: p.name ? `${p.name} · ${p.preview.short}` : p.preview.short, tag: null, count: 1, hint: "",
  }));
  slots.push({ kind: "custom", id: size.slot.id, family: size.slot.family, icon: null, label: "Custom…", sub: "Any size", tag: null, count: 1, hint: "" });
  draw();
}

const multi = () => size && !size.folder && size.ring && size.ring.count > 1;
const scope = () => (size && size.folder ? (size.mode === "parts" ? "Pieces of " : "All under ") : multi() ? (size.together ? "All together under " : "Each under ") : "Under ");

function toggleHtml() {
  if (size && size.folder) {
    return `<div class="seg-toggle" role="group" aria-label="Make">${FIT_MODES.map(([v, l]) => `<button type="button" class="seg-btn${size.mode === v ? " on" : ""}" data-mode="${v}" aria-pressed="${size.mode === v}">${l}</button>`).join("")}</div>`;
  }
  if (!multi()) return "";
  const b = (v, label) => `<button type="button" class="seg-btn${size.together === v ? " on" : ""}" data-together="${v}" aria-pressed="${size.together === v}">${label}</button>`;
  return `<div class="seg-toggle" role="group" aria-label="The size is for">${b(false, "Each")}${b(true, "Together")}</div>`;
}

function sizeHub() {
  if (size.loading) {
    const m = size.measure ? ` ${size.measure.done} of ${size.measure.total}` : "";
    return `<div class="hub size-hub">${tileFor(22)}<div class="hm">Measuring…${esc(m)}</div></div>`;
  }
  const title = (size.ring && size.ring.title) || model.hubTitle;
  if (!size.custom) {
    return `<div class="hub size-hub"><div class="hm">${size.folder ? "Fit" : "Compress to"}</div><div class="hn" title="${esc(title)}">${esc(title)}</div>${toggleHtml()}` +
      `<button type="button" class="hub-btn" data-act="back">Back</button></div>`;
  }
  const p = size.preview && size.previewFor === typedKey() ? size.preview : null;
  const go = !p ? "Compress" : p.fits ? "OK" : p.ok ? "Compress" : `Make ${words(p.min)}`;
  return `<form class="hub size-hub custom" autocomplete="off">` +
    (multi() || size.folder ? toggleHtml() : `<label class="hm" for="size-box">No bigger than</label>`) +
    `<div class="size-row"><input id="size-box" type="text" inputmode="decimal" aria-label="Size" value="${esc(size.text)}" spellcheck="false">` +
    `<button type="button" class="unit" data-act="unit" aria-label="Unit: ${size.unit}, click to switch">${size.unit}</button></div>` +
    `<div class="size-row"><button type="button" class="hub-btn icon" data-act="close" aria-label="Back to the sizes">${icon("back", 14)}</button>` +
    `<button type="submit" class="hub-btn primary"${typedBytes() ? "" : " disabled"}>${go}</button></div></form>`;
}

function wireSizeHub() {
  const hub = wheelEl.querySelector(".hub");
  if (!hub) return;
  hub.addEventListener("click", (e) => {
    const t = e.target.closest("[data-act], [data-together], [data-mode]");
    if (!t) return;
    e.stopPropagation();
    if (t.dataset.mode) {
      if (size.mode !== t.dataset.mode) { size.mode = t.dataset.mode; size.preview = null; size.previewFor = ""; loadPresets(); if (size.custom) { redrawCustom(); refreshPreview(); } }
      return;
    }
    if (t.dataset.together) {
      const v = t.dataset.together === "true";
      if (size.together !== v) { size.together = v; size.preview = null; size.previewFor = ""; loadPresets(); if (size.custom) refreshPreview(); }
      return;
    }
    if (t.dataset.act === "back") return leaveSize();
    if (t.dataset.act === "close") return closeCustom();
    if (t.dataset.act === "unit") { size.unit = size.folder ? nextUnit(size.unit) : size.unit === "MB" ? "KB" : "MB"; redrawCustom(); return refreshPreview(); }
  });
  const form = hub.tagName === "FORM" ? hub : null;
  if (!form) return;
  form.addEventListener("submit", (e) => { e.preventDefault(); submitCustom(); });
  const box = form.querySelector("#size-box");
  box.addEventListener("input", () => {
    const clean = box.value.replace(",", ".").replace(/[^0-9.]/g, "").replace(/(\..*)\./g, "$1");
    if (clean !== box.value) box.value = clean;
    size.text = clean;
    refreshPreview();
  });
  box.addEventListener("keydown", (e) => {
    // "k" and "m" pick the unit while typing.
    const k = e.key.toLowerCase();
    if (k === "k" || k === "m" || (k === "g" && size.folder)) { e.preventDefault(); size.unit = k === "k" ? "KB" : k === "g" ? "GB" : "MB"; redrawCustom(); refreshPreview(); }
  });
  box.focus();
  box.setSelectionRange(box.value.length, box.value.length);
}

/// Redraws just the centre (keeps the ring and the hover).
function redrawCustom() {
  const hub = wheelEl.querySelector(".hub");
  if (!hub) return;
  const tmp = document.createElement("div");
  tmp.innerHTML = sizeHub();
  hub.replaceWith(tmp.firstChild);
  wireSizeHub();
}

function typedBytes() {
  const n = parseFloat(size.text);
  return n > 0 ? Math.round(n * UNIT[size.unit]) : 0;
}
const typedKey = () => `${typedBytes()}|${size.together}|${size.mode}`;

let previewTimer = 0;
function refreshPreview() {
  clearTimeout(previewTimer);
  const bytes = typedBytes();
  if (!bytes) { size.preview = null; size.previewFor = ""; redrawCustomButton(); return sizeHint(null); }
  const key = typedKey();
  const mine = size;
  previewTimer = setTimeout(() => {
    const got = (p) => {
      if (size !== mine || typedKey() !== key) return;
      mine.preview = p;
      mine.previewFor = key;
      redrawCustomButton();
      if (active < 0 || active === slots.length - 1) sizeHint(null);
    };
    if (!tauri) return got(demoPreview(bytes));
    const call = mine.folder
      ? tauri.core.invoke("folder_fit_preview", { bytes, mode: mine.mode, subfolders: model.folder.defaults.subfolders })
      : tauri.core.invoke("compress_preview", { targetId: mine.slot.id, family: mine.slot.family, bytes, together: mine.together, trim: null });
    call.then(got).catch(() => {});
  }, 120);
}

function redrawCustomButton() {
  const btn = wheelEl.querySelector(".size-hub .primary");
  if (!btn) return;
  const p = size.preview && size.previewFor === typedKey() ? size.preview : null;
  btn.textContent = !p ? "Compress" : p.fits ? "OK" : p.ok ? "Compress" : `Make ${words(p.min)}`;
  btn.disabled = !typedBytes();
}

function openCustom(first) {
  if (!size || size.loading) return;
  size.custom = true;
  if (first !== null && first !== undefined) size.text = first;
  draw();
  refreshPreview();
}

function closeCustom() {
  size.custom = false;
  draw();
}

function leaveSize() {
  size = null;
  slots = page === "more" ? model.more : model.main;
  draw();
}

function submitCustom() {
  const bytes = typedBytes();
  if (!bytes) return;
  const p = size.preview && size.previewFor === typedKey() ? size.preview : null;
  if (p && !p.fits && !p.ok) return startSize(p.min, words(p.min), null);
  startSize(bytes, words(bytes), null);
}

function startSize(bytes, label, trim) {
  if (!size || busy) return;
  const s = size.slot;
  if (size.folder) {
    const opts = Object.assign(folderDefaults(), { fit: { bytes, mode: size.mode } });
    return launch({ label }, { cmd: "folder_pick", actionId: s.id, label: `Fit in ${label}`, opts, quality: null, remember: false });
  }
  launch({ label }, {
    targetId: s.id, label, family: s.family, quality: null, remember: false,
    size: { bytes, together: size.together && multi(), trim },
  });
}

/// The hint under the wheel in size mode.
function sizeHint(s) {
  const keys = "Click to compress · type a number for any size · Esc goes back";
  if (s && s.kind === "size") {
    const p = s.preview;
    const each = multi() && !size.together;
    hintEl.innerHTML = `<span class="ht">${esc(scope() + s.label)}</span>` +
      `<span class="hs">${esc(`${p.line1} · ${p.line2}`)} · ≈ ${esc(words(p.est))}${each ? " in all" : ""}</span><span class="hk">${keys}</span>`;
    return;
  }
  if (size.loading) {
    const m = size.measure ? ` (${size.measure.done} of ${size.measure.total} measured)` : "";
    hintEl.innerHTML = `<span class="hs">Working out the sizes for ${esc(model.hubTitle)}…${esc(m)}</span><span class="hk">Esc goes back</span>`;
    return;
  }
  if (s && s.kind === "custom" && !size.custom) {
    hintEl.innerHTML = `<span class="ht">Custom size</span><span class="hs">Type any size in the middle of the wheel</span><span class="hk">Or just start typing a number</span>`;
    return;
  }
  if (size.custom) {
    const bytes = typedBytes();
    const p = size.preview && size.previewFor === typedKey() ? size.preview : null;
    const enter = "Enter to compress · Esc goes back";
    if (!bytes) {
      hintEl.innerHTML = `<span class="ht">Type a size</span><span class="hs">Any number · K or M sets the unit</span><span class="hk">${enter}</span>`;
    } else if (!p) {
      hintEl.innerHTML = `<span class="ht">${esc(scope() + words(bytes))}</span><span class="hs">Working it out…</span><span class="hk">${enter}</span>`;
    } else if (p.fits) {
      hintEl.innerHTML = `<span class="ht">${esc(p.line1)}</span><span class="hs">${esc(p.line2)}</span><span class="hk">${enter}</span>`;
    } else if (p.ok) {
      hintEl.innerHTML = `<span class="ht">${esc(scope() + words(bytes))}</span><span class="hs">${esc(`${p.line1} · ${p.line2}`)} · ≈ ${esc(words(p.est))}</span><span class="hk">${enter}</span>`;
    } else {
      hintEl.innerHTML = `<span class="ht">${esc(p.line1)}</span><span class="hs">${esc(p.line2)}</span>` +
        `<span class="hint-acts"><button type="button" class="hint-btn primary" data-act="min">Make it ${esc(words(p.min))}</button>` +
        (p.alt ? `<button type="button" class="hint-btn" data-act="alt">${esc(p.alt.label)}</button>` : "") + `</span>`;
      const on = (a, f) => { const b = hintEl.querySelector(`[data-act="${a}"]`); if (b) b.addEventListener("click", (e) => { e.stopPropagation(); f(); }); };
      on("min", () => startSize(p.min, words(p.min), null));
      on("alt", () => startSize(p.alt.bytes, words(p.alt.bytes), p.alt.trim));
    }
    return;
  }
  const title = (size.ring && size.ring.title) || model.hubTitle;
  if (size.folder) {
    const none = size.ring && !size.ring.presets.length ? " · no common size fits this folder: type one" : "";
    const what = size.error ? size.error : `${FIT_SAY[size.mode]} · ${words(size.ring ? size.ring.total : 0)} now${none}`;
    hintEl.innerHTML = `<span class="ht">Fit ${esc(title)}</span><span class="hs">${esc(what)}</span><span class="hk">${keys.replace("compress", "start")}</span>`;
    return;
  }
  const what = size.error ? size.error
    : multi() ? (size.together ? `Sizes for all ${size.ring.count} together, or switch to each` : `Sizes for each of the ${size.ring.count}`)
    : `Sizes picked for this ${words(size.ring ? size.ring.total : 0)} file`;
  hintEl.innerHTML = `<span class="ht">Compress ${esc(title)}</span><span class="hs">${esc(what)}</span><span class="hk">${keys}</span>`;
}

/// Keys in size mode; true when handled.
function sizeKey(e) {
  const inBox = e.target && e.target.id === "size-box";
  if (e.key === "Escape") {
    e.preventDefault();
    if (size.custom) closeCustom(); else leaveSize();
    return true;
  }
  if (inBox) return true; // typing in the box
  if (e.key === "Backspace" && !size.custom) { e.preventDefault(); leaveSize(); return true; }
  if (/^[0-9.]$/.test(e.key) && !e.ctrlKey && !e.metaKey && !e.altKey && !size.loading) {
    e.preventDefault();
    if (!size.custom) openCustom(e.key);
    return true;
  }
  return false;
}

// Opened in a browser: sample sizes so the ring can be checked.
function demoPresets(mine) {
  setTimeout(() => {
    const mk = (bytes, name, short, line1) => ({ bytes, label: words(bytes), name, preview: { fits: false, ok: true, short, line1, line2: "Quality lowered to fit", est: bytes * 0.97, min: 4.5e6, alt: null } });
    mine.ring = { title: "Trip.mp4", count: 1, total: 182e6, presets: [mk(60e6, "", "Full quality", "Keeps 1080p"), mk(30e6, "", "720p", "1080p → 720p"), mk(18e6, "Email", "720p", "1080p → 720p"), mk(15e6, "", "480p", "1080p → 480p"), mk(10e6, "Discord", "360p", "1080p → 360p")] };
    mine.loading = false;
    mine.unit = "MB";
    if (size === mine) showSizes();
  }, 400);
}
function demoPreview(bytes) {
  if (bytes < 4.5e6) return { fits: false, ok: false, short: "Too small", line1: `Can't fit Trip.mp4 in ${words(bytes)}`, line2: "The smallest that still plays well is about 4.5 MB", est: bytes, min: 4.5e6, alt: { label: "Keep 720p, first 7 s", bytes, trim: 7 } };
  if (bytes >= 182e6) return { fits: true, ok: true, short: "Already fits", line1: "Trip.mp4 is already 182 MB", line2: "Nothing to do", est: 182e6, min: 4.5e6, alt: null };
  return { fits: false, ok: true, short: "480p", line1: "1080p → 480p", line2: "Quality lowered to fit", est: bytes * 0.97, min: 4.5e6, alt: null };
}

// ---------- progress ring ----------
// After a pick the wheel collapses into a ring at the same spot. A job that
// finishes within HANDOFF_MS ends there (tick, what was saved, Undo); a longer
// one or a failure moves to the HUD's corner card.

let ring = null;            // { id, flip, done, gone, handoff, fade }
const early = new Map();    // job events that arrived before the pick returned its id

const TICK = '<svg class="ic" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-label="Done"><path d="M5 12.5l4.5 4.5L19 7.5"/></svg>';
const UNDONE = '<svg class="undone" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-label="Undone"><path d="M9 14L4 9l5-5M4 9h10a6 6 0 010 12h-3"/></svg>';

function pick(s, cmd, args) {
  busy = true;
  const pickedAt = performance.now();
  hintEl.classList.remove("show");
  wheelEl.classList.add("collapsing");
  const collapse = new Promise((r) => setTimeout(r, 180));
  if (!tauri) return demoRing(s, collapse);
  const picked = tauri.core.invoke(cmd, args);
  const openedAs = openCount;
  Promise.all([picked, collapse]).then(async ([id]) => {
    // A new wheel opened meanwhile: it has handed this job to the corner card.
    if (openCount !== openedAs) return;
    const got = early.get(id) || {};
    early.delete(id);
    // Failed straight away (nothing to convert, say): straight to the corner card.
    if (got.done && (!got.done.ok || got.done.attention)) return tauri.core.invoke("ring_handoff", { id });
    const layout = await tauri.core.invoke("ring_mode");
    if (openCount !== openedAs) return;
    startRing(id, s.label, layout.flip, pickedAt);
    if (got.progress) ringProgress(got.progress);
    if (got.done) ringDone(got.done);
  }).catch((e) => {
    console.error(e);
    picked.then((id) => tauri.core.invoke("ring_end", { id })).catch(() => {});
  });
}

function startRing(id, label, flip, pickedAt) {
  ring = { id, flip, done: false, gone: false, handoff: null, fade: null };
  document.body.classList.add("ringmode");
  ringEl.className = "ring" + (flip ? " flip" : "");
  ringProg.style.strokeDashoffset = RING_C;
  ringIn.innerHTML = `<span class="rl">${esc(label)}</span><span class="rp">0%</span>`;
  ringText.textContent = "";
  ringUndo.hidden = false;
  ringUndo.disabled = false;
  ringCompare.hidden = true;
  requestAnimationFrame(() => requestAnimationFrame(() => ringEl.classList.add("show")));
  const handoffMs = (model && model.handoffMs) || HANDOFF_MS;
  ring.handoff = setTimeout(handOff, Math.max(0, handoffMs - (performance.now() - pickedAt)));
}

function ringProgress(p) {
  if (!ring || ring.done) return;
  ringProg.style.strokeDashoffset = RING_C * (1 - Math.min(1, Math.max(0, p.fraction)));
  const rp = ringIn.querySelector(".rp");
  // "File 2 of 5" is too long for the ring: "2 / 5".
  // A converter downloading on first use: just its percentage (the corner
  // card, after the hand-over, says what's downloading).
  if (rp) rp.textContent = String(p.detail)
    .replace(/^File (\d+) of (\d+)( · .*)?$/, "$1 / $2")
    .replace(/^Getting ready…$/, "…")
    .replace(/^Downloading .* · (\d+%|\d+ MB)$/, "↓ $1")
    .replace(/^(Setting up|Installing) .*$/, "…");
}

function ringDone(d) {
  if (!ring || ring.gone) return;
  if (!d.ok || d.attention) return handOff(); // errors and notes need room to read: the corner card
  clearTimeout(ring.handoff);
  ring.done = true;
  ringProg.style.strokeDashoffset = 0;
  ringIn.innerHTML = TICK;
  // "holiday-goa.jpg · 2.8 MB" -> "Saved holiday-goa.jpg"
  ringText.textContent = d.pill || "Saved " + String(d.body).split(" · ")[0];
  ringUndo.hidden = !d.canUndo;
  // Where it went: "Show in Finder" / "Show in folder" (just "Show" next to Compare) opens it with the file selected.
  ringShow.hidden = !d.canUndo;
  ringShow.textContent = d.compare ? "Show" : isMacUi() ? "Show in Finder" : "Show in folder";
  ringCompare.hidden = !d.compare;
  ringEl.classList.toggle("wide", !!d.compare);
  ringEl.classList.add("done");
  if (tauri) tauri.core.invoke("ring_interactive", { on: true });
  scheduleFade(4000);
}

function scheduleFade(ms) {
  if (!ring) return;
  clearTimeout(ring.fade);
  ring.fade = setTimeout(fadeRing, ms);
}

function fadeRing() {
  if (!ring || ring.gone) return;
  const id = ring.id;
  ring.gone = true;
  ringEl.classList.remove("show");
  ringEl.classList.add("leave");
  setTimeout(() => {
    if (ring && ring.id === id) resetRing();
    if (tauri) tauri.core.invoke("ring_end", { id });
  }, 200);
}

function handOff() {
  if (!ring || ring.gone || ring.done) return;
  const id = ring.id;
  ring.gone = true;
  clearTimeout(ring.handoff);
  ringEl.classList.remove("show");
  ringEl.classList.add("fly");
  setTimeout(() => {
    if (ring && ring.id === id) resetRing();
    if (tauri) tauri.core.invoke("ring_handoff", { id });
  }, 280);
}

function resetRing() {
  if (ring) {
    clearTimeout(ring.handoff);
    clearTimeout(ring.fade);
  }
  ring = null;
  document.body.classList.remove("ringmode");
  ringEl.className = "ring";
  wheelEl.classList.remove("collapsing");
}

ringEl.addEventListener("mouseenter", () => ring && ring.done && clearTimeout(ring.fade));
ringEl.addEventListener("mouseleave", () => ring && ring.done && scheduleFade(900));
ringShow.addEventListener("click", (e) => {
  e.stopPropagation();
  if (!ring || !ring.done) return;
  if (tauri) tauri.core.invoke("job_reveal", { id: ring.id }).catch(() => {});
  fadeRing();
});
ringCompare.addEventListener("click", (e) => {
  e.stopPropagation();
  if (!ring || !ring.done) return;
  if (tauri) tauri.core.invoke("compare_open", { id: ring.id }).catch(() => {});
  fadeRing();
});
ringUndo.addEventListener("click", async (e) => {
  e.stopPropagation();
  if (!ring || !ring.done) return;
  ringUndo.disabled = true;
  try {
    if (tauri) await tauri.core.invoke("job_undo", { id: ring.id });
    ringIn.innerHTML = UNDONE;
    ringText.textContent = isMacUi() ? "Moved to the Trash" : "Moved to the Recycle Bin";
    ringUndo.hidden = true;
    ringShow.hidden = true;
  } catch (err) {
    ringText.textContent = String(err);
    ringUndo.disabled = false;
  }
  scheduleFade(1200);
});

function isMacUi() { return document.documentElement.dataset.os === "mac"; }

// Opened in a browser: fake a job so the ring can be checked.
function demoRing(s, collapse) {
  collapse.then(() => {
    startRing(1, s.label, false, performance.now());
    let f = 0;
    const t = setInterval(() => {
      f = Math.min(1, f + 0.08);
      ringProgress({ fraction: f, detail: Math.round(f * 100) + "%" });
      if (f >= 1) {
        clearInterval(t);
        ringDone({ ok: true, body: "Q3-report – pages · 3 images", canUndo: true, compare: !!s.bytes || s.label.endsWith("B") });
      }
    }, 90);
  });
}

// ---------- input ----------

function indexAt(x, y) {
  const dx = x - CX, dy = y - CY, d = Math.hypot(dx, dy);
  if (d < RI || d > FAR || !slots.length) return -1;
  const a = ((Math.atan2(dy, dx) * 180) / Math.PI + 90 + 360) % 360;
  return Math.round(a / (360 / slots.length)) % slots.length;
}

document.addEventListener("pointermove", (e) => {
  if (!model || busy || ring || optsFor) return;
  if (size && e.target.closest && e.target.closest(".hub, .hint")) return;
  const onAsk = !!(e.target.closest && e.target.closest(".ask"));
  if (onAsk && !askHover) feel("tick");
  if (onAsk || askHover) setAskHover(onAsk);
  if (onAsk) return;
  const i = indexAt(e.clientX, e.clientY);
  if (i !== active) {
    // Only landing on an option ticks; leaving the ring (to the centre or
    // outside) is silent.
    if (i >= 0) feel("tick");
    setActive(i);
  }
});

document.addEventListener("click", (e) => {
  if (!model || busy || ring) return;
  // A click beside the options card closes just the card.
  if (optsFor) { hideOptions(); hintEl.classList.add("show"); return; }
  // The size ring's centre and hint have their own buttons.
  if (size && e.target.closest && e.target.closest(".hub, .hint")) return;
  if (e.target.closest && e.target.closest(".ask")) return ask(e.shiftKey);
  const i = indexAt(e.clientX, e.clientY);
  if (i >= 0) choose(i, e.shiftKey);
  else close(); // the hub or empty space
});

document.addEventListener("contextmenu", (e) => e.preventDefault());

document.addEventListener("keydown", (e) => {
  if (!model || busy || ring || optsFor) return;
  if (size && sizeKey(e)) return;
  const n = slots.length;
  if (e.key === "Escape") {
    e.preventDefault();
    return page === "more" ? showPage("main") : close();
  }
  if (e.key === "ArrowRight" || e.key === "ArrowDown") { e.preventDefault(); if (n) feel("tick"); return setActive((active + 1 + n) % n); }
  if (e.key === "ArrowLeft" || e.key === "ArrowUp") { e.preventDefault(); if (n) feel("tick"); return setActive(active <= 0 ? n - 1 : active - 1); }
  if (e.key === "Enter" && active >= 0) { e.preventDefault(); return choose(active, e.shiftKey); }
  // Nothing pointed at: Enter picks the smart slot.
  if (e.key === "Enter" && slots[0] && slots[0].smart && !slots[0].blocked) { e.preventDefault(); setActive(0); return choose(0, e.shiftKey); }
  if (e.key === "Backspace" && page === "more") { e.preventDefault(); return showPage("main"); }
  if (e.code === "KeyC" && !e.ctrlKey && !e.metaKey && !e.altKey && showAsk()) { e.preventDefault(); return ask(e.shiftKey); }
  // Shift+1 types "!" on most keyboards: read the key's position instead.
  const digit = /^Digit([1-9])$/.exec(e.code);
  const k = digit ? Number(digit[1]) : parseInt(e.key, 10);
  if (k >= 1 && k <= Math.min(9, n)) { e.preventDefault(); setActive(k - 1); choose(k - 1, e.shiftKey); }
});

// ---------- wiring ----------

if (tauri) {
  tauri.event.listen("wheel-open", (ev) => open(ev.payload));
  // A big folder finished reading: the same wheel, now with its slots.
  tauri.event.listen("wheel-update", (ev) => {
    if (!model || !model.folder || busy || ring) return;
    model = ev.payload;
    page = "main";
    size = null;
    slots = model.main;
    draw();
  });
  tauri.event.listen("wheel-scan", (ev) => {
    if (!model || !model.folder || model.folder.scanning === null || model.folder.scanning === undefined) return;
    model.folder.scanning = ev.payload;
    model.hubSubtitle = `Reading… ${Number(ev.payload).toLocaleString()} files`;
    const hm = wheelEl.querySelector(".hub .hm");
    if (hm) hm.textContent = model.hubSubtitle;
    if (active < 0) setActive(-1);
  });
  tauri.event.listen("fit-measure", (ev) => {
    if (!size || !size.folder || !size.loading) return;
    size.measure = ev.payload;
    const hm = wheelEl.querySelector(".size-hub .hm");
    if (hm) hm.textContent = `Measuring… ${ev.payload.done} of ${ev.payload.total}`;
    sizeHint(null);
  });
  const remember = (id, key, value) => early.set(id, Object.assign(early.get(id) || {}, { [key]: value }));
  tauri.event.listen("job-progress", (ev) => {
    const p = ev.payload;
    if (ring && ring.id === p.id) ringProgress(p);
    else if (!ring) remember(p.id, "progress", p);
  });
  tauri.event.listen("job-done", (ev) => {
    const d = ev.payload;
    if (ring && ring.id === d.id) ringDone(d);
    else if (!ring) remember(d.id, "done", d);
  });
  // A click somewhere else while the ring shows (Windows).
  tauri.event.listen("ring-away", () => {
    if (!ring) return;
    if (ring.done) fadeRing();
    else handOff();
  });
  tauri.core.invoke("wheel_model").then((m) => { if (m && !model) open(m); }).catch(() => {});
} else {
  // Opened in a normal browser: show a sample so the design can be checked.
  document.documentElement.style.background = "#6b7a8f";
  const t = (id, label, icon, hint) => ({ id, label, icon, hint, family: "pdf", tag: null, kind: "target", count: 1 });
  // ?folder: a sample folder wheel.
  const fs = (id, label, icon, count, sub, hint, detail, out) => ({ id, label, icon, hint, family: "folder", tag: null, kind: "target", count, sub, detail, out });
  // ?smart: a zip of fonts (a model Rust built), with the smart slot first.
  if (/smart/.test(location.search)) {
    open(Object.assign({"os":"win","accent":null,"files":[{"path":"C:\\Users\\Crofty\\Downloads\\Noto Sans Family.zip","name":"Noto Sans Family.zip","ext":"zip","family":"archive","size":835000}],"skipped":[],"hubTitle":"Noto Sans Family.zip","hubSubtitle":"3 fonts inside","hubFamily":"archive","hubColor":"#9A6B00","main":[{"id":"smart.fonts","label":"Install font","icon":"font","hint":"1 Noto Sans font (TTF), just for you, no admin prompt","family":"smart","tag":null,"kind":"target","count":1,"sub":"1 font","smart":true,"lines":["Straight into Windows' Fonts folder: nothing gets unzipped next to the zip","Left out: 1 duplicate · 1 web font"],"fields":[{"key":"for","kind":"radio","label":"Install for","choices":[{"value":"me","label":"Just me","sub":"No admin prompt. Most apps see these."},{"value":"all","label":"Everyone on this PC","sub":"One admin prompt. For older apps that only see fonts installed for everyone."}],"value":"me"},{"key":"keep","kind":"folder","label":"Also keep the font files in","choices":[],"value":null}],"go":"Install 1 font"},{"id":"archive.extract","label":"Extract","icon":"extract","hint":"Unpack into a folder next to the archive","family":"archive","tag":null,"kind":"target","count":1},{"id":"archive.7z","label":"7Z","icon":"archive","hint":"7-Zip · about 20% smaller","family":"archive","tag":null,"kind":"target","count":1},{"id":"archive.targz","label":"TAR.GZ","icon":"archive","hint":"For Linux and servers","family":"archive","tag":null,"kind":"target","count":1}],"more":[],"ring":true,"handoffMs":2000,"ask":null}, { quality: DEFAULT_Q, presets: {} }));
  } else
  if (/folder/.test(location.search)) {
    open({
      os: /Mac/.test(navigator.platform) ? "mac" : "win", accent: null, files: [], skipped: ["Valorant clips"],
      hubTitle: "Valorant clips", hubSubtitle: "44 files · 14.50 GB", hubFamily: "folder", hubColor: "#E8B84A",
      ring: true, handoffMs: 2000, quality: DEFAULT_Q, presets: {}, ask: null,
      folder: { name: "Valorant clips", roots: ["E:\\Videos\\Valorant clips"], files: 44, inSubfolders: 9, subfolders: 3, bytes: 14.5e9, alone: 2, aloneWords: "1 hidden, 1 system file", blank: false, truncated: false, scanning: null,
        defaults: { into: "new", subfolders: true, skipDone: true }, sequence: null, sound: null, topCounts: { "folder.compress": 35, "folder.zip": 35, "folder.fit": 35 } },
      main: [
        fs("folder.compress", "Compress all", "compress", 44, "44 files", "Smallest that looks the same · 40 videos, 4 pictures", "Leaves 2 hidden or system files alone", "→ new folder Valorant clips (compressed)"),
        fs("folder.to:video.mp4", "All → MP4", "video", 2, "2 videos", "Only 2 videos need it · 38 already MP4 (left alone)", "Leaves 42 files alone", "→ new folder Valorant clips (mp4)"),
        fs("folder.join-video", "Join clips", "join", 40, "40 videos", "One video, in name order", "Leaves 4 files alone", "→ Valorant clips.mp4 next to the folder"),
        fs("folder.audio", "Audio only", "audio", 40, "40 videos", "MP3 from every video · About 245 kbps", "Leaves 4 files alone", "→ new folder Valorant clips (mp3)"),
        fs("folder.fit", "Fit to size", "fit", 44, "14.50 GB →", "Make the folder fit a size: a compressed copy, one zip, or a zip in parts", null, "Pick the size next"),
        fs("folder.zip", "Zip", "archive", 44, "44 files", "Videos and pictures go in as they are (they're compressed already): fast", "Hidden and system files are left out (2)", "→ Valorant clips.zip next to the folder · Shift+click: 7Z, TAR.GZ, password, parts"),
        { id: "more", label: "More", icon: "more", hint: "More: Remove GPS, Watch this folder…", family: "folder", tag: null, kind: "more", count: 2, sub: "2" },
      ],
      more: [
        fs("folder.gps", "Remove GPS", "pin", 4, "4 pictures", "Location removed from every picture that has one · pixels untouched", "Leaves 40 files alone", "→ new folder Valorant clips (no GPS)"),
        fs("folder.watch", "Watch this folder…", "eye", 0, null, "Convert new files as they arrive", null, "Opens Settings › Watched folders"),
        { id: "back", label: "Back", icon: "back", hint: "Back to the folder wheel", family: "folder", tag: null, kind: "back", count: 0 },
      ],
    });
  } else
  open({
    os: /Mac/.test(navigator.platform) ? "mac" : "win",
    accent: null,
    files: [{ path: "C:\\Reports\\Q3-report.pdf", name: "Q3-report.pdf", ext: "pdf", family: "pdf", size: 2500000 }], skipped: [],
    hubTitle: "Q3-report.pdf", hubSubtitle: "PDF · 2.5 MB", hubFamily: "pdf", hubColor: "#C4262E",
    ring: true, handoffMs: 2000, quality: DEFAULT_Q,
    presets: { balanced: { image: { look: 80, floor: 40, png: 1, stripGps: false }, video: { look: 93, encoder: "gpu" } }, small: { image: { look: 70, stripGps: false } }, best: { image: { look: 87, stripGps: false } } },
    ask: { default: "chat", modes: [
      { id: "chat", label: "Chat", does: "Starts a new chat with the file paths written in" },
      { id: "code", label: "Claude Code", does: "Opens a Claude Code session in the files' folder" },
    ] },
    main: [
      t("pdf.jpg", "JPG", "image", "One image per page · 150 DPI"),
      t("pdf.png", "PNG", "image", "Lossless image per page · 150 DPI"),
      t("pdf.webp", "WEBP", "image", "Smaller images, one per page"),
      t("pdf.txt", "TXT", "doc", "All text from every page"),
      t("pdf.split", "Split", "split", "One PDF per page"),
      t("pdf.compress", "Compress", "compress", "Smaller PDF for email"),
      { id: "more", label: "More", icon: "more", hint: "More: TIFF, Grayscale", family: "pdf", tag: null, kind: "more", count: 1 },
    ],
    more: [
      t("pdf.tiff", "TIFF", "image", "One multi-page TIFF · 300 DPI"),
      { id: "back", label: "Back", icon: "back", hint: "Back to the main ring", family: "pdf", tag: null, kind: "back", count: 1 },
    ],
  });
}
