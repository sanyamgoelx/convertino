// Convertino Settings. Reads everything with settings_get and saves each
// change straight away with settings_set (there's no Save button).

const tauri = window.__TAURI__;
const $ = (id) => document.getElementById(id);
const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

const NAV = [
  { id: "general", label: "General", icon: "M12 15a3 3 0 100-6 3 3 0 000 6z M19.4 15a1.7 1.7 0 00.3 1.8l.1.1a2 2 0 11-2.8 2.8l-.1-.1a1.7 1.7 0 00-1.8-.3 1.7 1.7 0 00-1 1.5V21a2 2 0 11-4 0v-.1a1.7 1.7 0 00-1.1-1.5 1.7 1.7 0 00-1.8.3l-.1.1a2 2 0 11-2.8-2.8l.1-.1a1.7 1.7 0 00.3-1.8 1.7 1.7 0 00-1.5-1H3a2 2 0 110-4h.1a1.7 1.7 0 001.5-1.1 1.7 1.7 0 00-.3-1.8l-.1-.1a2 2 0 112.8-2.8l.1.1a1.7 1.7 0 001.8.3H9a1.7 1.7 0 001-1.5V3a2 2 0 114 0v.1a1.7 1.7 0 001 1.5 1.7 1.7 0 001.8-.3l.1-.1a2 2 0 112.8 2.8l-.1.1a1.7 1.7 0 00-.3 1.8V9a1.7 1.7 0 001.5 1H21a2 2 0 110 4h-.1a1.7 1.7 0 00-1.5 1z" },
  { id: "wheel", label: "Wheel", icon: "M12 3a9 9 0 100 18 9 9 0 000-18z M12 8.5a3.5 3.5 0 100 7 3.5 3.5 0 000-7z M12 3v5.5 M12 15.5V21 M3 12h5.5 M15.5 12H21" },
  { id: "quality", label: "Quality", icon: "M4 21v-7 M4 10V3 M12 21v-9 M12 8V3 M20 21v-5 M20 12V3 M1 14h6 M9 8h6 M17 16h6" },
  { id: "converters", label: "Converters", icon: "M14.7 6.3a1 1 0 000 1.4l1.6 1.6a1 1 0 001.4 0l3.8-3.8a6 6 0 01-7.9 7.9l-6.9 6.9a2.1 2.1 0 01-3-3l6.9-6.9a6 6 0 017.9-7.9z" },
  { id: "ai", label: "AI & CLI", icon: "M4 17l6-5-6-5 M12 19h8" },
  { id: "about", label: "About", icon: "M12 3a9 9 0 100 18 9 9 0 000-18z M12 16v-4 M12 8h.01" },
];
const WARN = "M12 3l10 18H2z M12 10v5 M12 18h.01";
const OK = "M5 12.5l4.5 4.5L19 7.5";
const QDEFAULT = { jpg: 90, webp: 85, resize: 1920, convertMax: 0, dpi: 150, pdfCompress: "balanced", mp3: 320, video: "balanced", gifWidth: 480, gifSeconds: 30, image: "balanced" };
const QGROUPS = [
  { title: "Images", rows: [
    { k: "image", tune: "image", label: "Picture quality", desc: "Convertino tries a few settings and keeps the smallest file that still looks like the original. Also used for PDF pages and Compress", options: [["small", "Smaller: looks very close"], ["balanced", "Balanced: looks the same (recommended)"], ["best", "Best: the same, even side by side"], ["fixed", "Fixed: use the numbers below"]] },
    { k: "jpg", label: "Fixed JPG quality", desc: "Only when Picture quality is Fixed. Higher is sharper and bigger", min: 50, max: 100 },
    { k: "webp", label: "Fixed WebP quality", desc: "Only when Picture quality is Fixed. Higher is sharper and bigger", min: 50, max: 100 },
    { k: "convertMax", label: "Resize large pictures", desc: "When converting: bigger pictures are scaled down, smaller ones are left as they are",
      options: [[0, "Don't resize"], [3840, "Up to 3840 px"], [2560, "Up to 2560 px"], [1920, "Up to 1920 px"], [1280, "Up to 1280 px"]] },
  ] },
  { title: "PDF", rows: [
    { k: "dpi", label: "Page picture resolution", desc: "When turning PDF pages into JPG, PNG or WebP", options: [[72, "Screen (72 DPI)"], [150, "Standard (150 DPI)"], [300, "Print (300 DPI)"]] },
    { k: "pdfCompress", tune: "pdf", label: "PDF compression", desc: "How close a compressed PDF must look to the original; the smallest version that does is kept", options: [["small", "Smaller: looks very close"], ["balanced", "Balanced: looks the same (recommended)"], ["high", "Best: the same, even side by side"]] },
  ] },
  { title: "Audio", rows: [
    { k: "mp3", label: "MP3 quality", desc: "Variable bitrate: quiet and simple parts take less space. Never more than a lossy original had. Also used when pulling audio out of video", options: [[128, "About 130 kbps"], [160, "About 165 kbps"], [192, "About 190 kbps"], [320, "Highest (about 245 kbps)"]] },
  ] },
  { title: "Video", rows: [
    { k: "video", tune: "video", label: "Video quality", desc: "For Compress, and when a video has to be re-encoded (MP4, MOV, WebM, 720p). Short samples are measured first, so the whole video gets the smallest setting that still looks the same", options: [["small", "Smaller: looks very close"], ["balanced", "Balanced: looks the same (recommended)"], ["best", "Best: the same, even side by side"]] },
    { k: "gifWidth", label: "GIF width", desc: "Height follows", options: [[320, "320 px"], [480, "480 px"], [640, "640 px"]] },
    { k: "gifSeconds", label: "GIF length", desc: "From the start; longer videos are cut", options: [[10, "First 10 s"], [30, "First 30 s"], [60, "First 60 s"]] },
  ] },
];
const LICENCES = [
  ["Convertino", "GNU GPL 3.0 or later"],
  ["FFmpeg", "GNU LGPL 2.1 or later (the x264/x265 build: GNU GPL 2.0 or later)"],
  ["ImageMagick", "ImageMagick License (Apache 2.0 style)"],
  ["LibRaw (RAW photos: inside ImageMagick on Windows, dcraw_emu on Mac)", "GNU LGPL 2.1 or CDDL 1.0"],
  ["rawler (RAW photo details and camera pictures)", "GNU LGPL 2.1"],
  ["little_exif, image (EXIF and picture files)", "MIT or Apache 2.0"],
  ["Poppler", "GNU GPL 2.0 or 3.0"],
  ["Ghostscript", "GNU AGPL 3.0"],
  ["Pandoc", "GNU GPL 2.0 or later"],
  ["LibreOffice", "Mozilla Public License 2.0"],
  ["7-Zip", "GNU LGPL 2.1, BSD 3-clause; unRAR restriction"],
  ["PDF.js", "Apache 2.0"],
  ["pdf-lib, fontkit", "MIT"],
  ["Noto Sans, Noto Sans Devanagari, Caveat", "SIL Open Font License 1.1"],
  ["Tauri", "MIT or Apache 2.0"],
  ["Microsoft Visual C++ runtime (only if your PC lacks it)", "Microsoft Software License"],
];

let view = null;          // what settings_get returned
let page = "general";
let fam = null;           // selected family on the Wheel page
let tools = [];           // converters status
const progress = {};      // converter id -> { fraction, detail }
let pollTimer = null;
let encoderLoaded = false;

// ---------- talking to Rust (or a stand-in in a browser) ----------

const invoke = tauri ? (cmd, args) => tauri.core.invoke(cmd, args) : demoInvoke;

async function save(patch) {
  try {
    view = await invoke("settings_set", { patch });
  } catch (e) {
    toast(String(e));
  }
  render();
}

// ---------- shortcut ----------

const isMac = () => view && view.os === "mac";

function keyLabels(combo) {
  if (!combo) return [];
  return combo.split("+").map((p) => {
    const l = p.toLowerCase();
    if (l === "ctrl" || l === "control") return isMac() ? "⌃" : "Ctrl";
    if (l === "alt" || l === "option") return isMac() ? "⌥" : "Alt";
    if (l === "shift") return isMac() ? "⇧" : "Shift";
    if (l === "super" || l === "cmd" || l === "command" || l === "meta") return isMac() ? "⌘" : "Win";
    return p.replace(/^Key/, "").replace(/^Digit/, "").replace(/^Arrow/, "").toUpperCase();
  });
}
const pretty = (combo) => keyLabels(combo).join(isMac() ? "" : "+");

let recording = false;

function showRecMsg(good, text, offerDefault) {
  const m = $("rec-msg");
  m.hidden = false;
  m.className = "msg " + (good ? "good" : "bad");
  $("rec-msg-icon").setAttribute("d", good ? OK : WARN);
  $("rec-msg-text").textContent = text;
  $("rec-default").hidden = !offerDefault;
}

function startRecording() {
  recording = true;
  $("rec-msg").hidden = true;
  invoke("shortcut_pause", { paused: true }).catch(() => {});
  renderShortcut();
  $("recorder").focus();
}

function stopRecording() {
  if (!recording) return;
  recording = false;
  invoke("shortcut_pause", { paused: false }).catch(() => {});
  renderShortcut();
}

async function onRecordKey(e) {
  e.preventDefault();
  e.stopPropagation();
  if (["Control", "Alt", "Shift", "Meta", "OS", "AltGraph"].includes(e.key)) return;
  if (e.key === "Escape" && !e.ctrlKey && !e.altKey && !e.metaKey) return stopRecording();
  if (!e.ctrlKey && !e.altKey && !e.metaKey) {
    return showRecMsg(false, isMac()
      ? "Use at least ⌃, ⌥ or ⌘, so normal typing never opens the wheel."
      : "Use at least Ctrl, Alt or the Windows key, so normal typing never opens the wheel.", false);
  }
  const mods = [];
  if (e.ctrlKey) mods.push("ctrl");
  if (e.altKey) mods.push("alt");
  if (e.shiftKey) mods.push("shift");
  if (e.metaKey) mods.push("super");
  const combo = mods.concat([e.code]).join("+");
  recording = false;
  try {
    const active = await invoke("shortcut_set", { shortcut: combo });
    view.shortcut = active;
    view.settings.shortcut = active;
    showRecMsg(true, `Saved. ${pretty(active)} now opens the wheel.`, false);
  } catch (err) {
    invoke("shortcut_pause", { paused: false }).catch(() => {});
    showRecMsg(false, String(err), true);
  }
  renderShortcut();
}

function renderShortcut() {
  const active = view.shortcut;
  $("keys").innerHTML = keyLabels(active).map((k) => `<span>${esc(k)}</span>`).join("");
  $("keys").hidden = recording;
  $("rec-start").hidden = recording;
  $("recorder").hidden = !recording;
  $("rec-cancel").hidden = !recording;
  const chosen = view.settings.shortcut;
  const where = isMac() ? "Finder" : "File Explorer or on the desktop";
  $("shortcut-sub").textContent = !active
    ? "No shortcut works right now: every default is taken by another app. Choose one with Change."
    : chosen && chosen !== active
      ? `${pretty(chosen)} is taken by another app right now, so ${pretty(active)} is in use.`
      : `An extra way to open the wheel for the files selected in ${where}. Change it to any keys you like`;
}

// ---------- rendering ----------

// ---------- Mac permissions ----------

let perms = null;
let permTimer = null;

async function refreshPerms() {
  if (!isMac()) return;
  try { perms = await invoke("permissions_get"); } catch (e) { return; }
  renderPerms();
  // While something is missing, keep checking: System Settings changes it from outside.
  clearTimeout(permTimer);
  if (!perms.accessibility || perms.finder !== "granted") permTimer = setTimeout(refreshPerms, 2000);
}

function renderPerms() {
  const box = $("perms");
  if (!perms || !perms.mac) { box.hidden = true; return; }
  box.hidden = false;
  const state = (el, ok, text) => {
    el.className = "perm-state " + (ok ? "ok" : "todo");
    el.innerHTML = ok ? `<svg class="ic sm" viewBox="0 0 24 24" aria-hidden="true"><path d="${OK}"/></svg>Allowed` : esc(text);
  };
  state($("perm-ax"), perms.accessibility, "Not allowed yet");
  $("perm-ax-btn").hidden = perms.accessibility;
  const f = perms.finder;
  state($("perm-finder"), f === "granted", f === "denied" ? "Turned off" : f === "unknown" ? "Open Finder, then Allow" : "Not allowed yet");
  $("perm-finder-btn").hidden = f === "granted";
  $("perm-finder-btn").textContent = f === "denied" ? "Open System Settings" : "Allow";
  $("perm-note").hidden = perms.accessibility && f === "granted";
}

function setPage(p) {
  // "permissions" lives at the top of General (Mac).
  if (p === "permissions") p = "general";
  if (!NAV.some((n) => n.id === p)) p = "general";
  if (recording) stopRecording();
  page = p;
  history.replaceState(null, "", "#" + p);
  render();
  $("main").scrollTop = 0;
  if (p === "converters") refreshTools();
  if (p === "ai") refreshAi();
  if (p === "quality") loadTuneInfo();
}

function render() {
  if (!view) return;
  const s = view.settings;
  document.documentElement.dataset.os = view.os;
  if (view.accent) applyAccent(view.accent);

  $("nav").innerHTML = NAV.map((n) =>
    `<button type="button" data-go="${n.id}"${page === n.id ? ' aria-current="page"' : ""}>` +
    `<svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><path d="${n.icon}"/></svg><span>${n.label}</span></button>`).join("");
  $("title").textContent = NAV.find((n) => n.id === page).label;
  document.querySelectorAll("section[data-page]").forEach((el) => { el.hidden = el.dataset.page !== page; });

  // Switches.
  document.querySelectorAll(".switch[data-key]").forEach((el) => {
    el.setAttribute("aria-checked", String(!!s[el.dataset.key]));
  });
  document.querySelectorAll("[data-state-for]").forEach((el) => { el.textContent = s[el.dataset.stateFor] ? "On" : "Off"; });
  renderAsk();
  if (isMac()) {
    $("alt-title").textContent = "Option-right-click on files";
    $("hero-key").textContent = "\u2325 Option";
    $("hero-title").textContent = "Hold Option (Alt) and right-click any file";
    if (s.altClick) $("hero-sub").textContent = "The easiest way to convert: the wheel opens right at your pointer in Finder.";
    $("login-title").textContent = "Open Convertino at login";
    $("login-sub").textContent = "Runs quietly in the menu bar";
  }
  if (!s.altClick) {
    $("hero-sub").textContent = "Turned off right now. Switch it back on below: it's the easiest way to convert.";
  } else if (!isMac()) {
    $("hero-sub").textContent = "The easiest way to convert: the wheel opens right at your pointer in File Explorer or on the desktop. Pick a format and you're done.";
  }
  if (view.dev) $("login-sub").textContent += " (not applied while developing)";

  renderShortcut();

  document.querySelectorAll("[data-save]").forEach((el) => el.setAttribute("aria-checked", String(s.saveMode === el.dataset.save)));
  $("folder-row").hidden = s.saveMode !== "folder";
  $("folder-path").textContent = s.saveFolder || "No folder chosen yet";
  $("folder-path").title = s.saveFolder || "";
  $("handoff").value = String(s.handoffSeconds);
  $("handoff").disabled = !s.progressRing;

  renderWheel();
  renderQuality();
  renderConverters();
  $("version").textContent = `Convertino ${view.version}`;
}

function applyAccent(hex) {
  // The Windows accent; lighter in dark mode, as Windows does it.
  const dark = matchMedia("(prefers-color-scheme: dark)").matches;
  document.documentElement.style.setProperty("--accent", dark ? `color-mix(in srgb, ${hex} 55%, white)` : hex);
}

// ---------- Wheel page ----------

function famInfo() {
  const fams = view.families;
  if (!fam || !fams.some((f) => f.id === fam)) fam = fams[0] && fams[0].id;
  return fams.find((f) => f.id === fam);
}

function ringOf(shown) {
  return shown.length > 8 ? shown.slice(0, 7).map((t) => t.label).concat(["More"]) : shown.map((t) => t.label);
}

function pt(c, r, deg) {
  const a = (deg * Math.PI) / 180;
  return (c + r * Math.cos(a)).toFixed(1) + " " + (c + r * Math.sin(a)).toFixed(1);
}

function renderWheel() {
  const f = famInfo();
  if (!f) return;
  $("fams").innerHTML = view.families.map((x) =>
    `<button type="button" role="tab" data-fam="${x.id}" aria-selected="${x.id === fam}">${esc(x.label)}</button>`).join("");

  const shown = f.targets.filter((t) => !t.hidden);
  let n = 0;
  let moreHeader = false;
  $("slots").innerHTML = f.targets.map((t, i) => {
    if (!t.hidden) n++;
    const inMore = !t.hidden && shown.length > 8 && n > 7;
    let head = "";
    if (inMore && !moreHeader) { moreHeader = true; head = `<div class="more-head">In “More”</div>`; }
    return head +
      `<div class="slot${t.hidden ? " hidden-slot" : ""}" data-i="${i}" data-id="${t.id}">` +
      `<span class="grip" tabindex="0" role="button" aria-label="Move ${esc(t.label)} (Alt+arrow keys)">` +
      `<svg class="ic sm" viewBox="0 0 24 24" aria-hidden="true"><path d="M9 6h.01 M15 6h.01 M9 12h.01 M15 12h.01 M9 18h.01 M15 18h.01" style="stroke-width:2.6"/></svg></span>` +
      `<button type="button" class="box" role="checkbox" aria-checked="${!t.hidden}" aria-label="Show ${esc(t.label)} on the wheel">` +
      `<svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><path d="${OK}"/></svg></button>` +
      `<span class="name">${esc(t.label)}</span><span class="hint">${esc(t.hint)}</span>` +
      `<span class="where">${t.hidden ? "Hidden" : inMore ? "More" : "Main ring"}</span></div>`;
  }).join("");

  const labels = ringOf(shown);
  const count = Math.max(1, labels.length), span = 360 / count;
  const segs = labels.map((_, i) => {
    const c = -90 + i * span, a0 = c - span / 2 + 1, a1 = c + span / 2 - 1;
    return `<path d="M ${pt(115, 108, a0)} A 108 108 0 0 1 ${pt(115, 108, a1)} L ${pt(115, 42, a1)} A 42 42 0 0 0 ${pt(115, 42, a0)} Z"/>`;
  }).join("");
  const text = labels.map((label, i) => {
    const rad = ((-90 + i * span) * Math.PI) / 180;
    return `<span class="pl" style="left:${(115 + 76 * Math.cos(rad)).toFixed(1)}px;top:${(115 + 76 * Math.sin(rad)).toFixed(1)}px">${esc(label)}</span>`;
  }).join("");
  $("preview").innerHTML = `<svg width="230" height="230" viewBox="0 0 230 230">${segs}</svg>${text}<div class="hubp">${esc(f.label)}</div>`;
}

function saveOrder(ids) {
  const f = famInfo();
  const byId = Object.fromEntries(f.targets.map((t) => [t.id, t]));
  f.targets = ids.map((id) => byId[id]);
  renderWheel();
  save({ order: { [fam]: ids } });
}

function moveSlot(from, to) {
  const ids = famInfo().targets.map((t) => t.id);
  if (from === to || to < 0 || to >= ids.length) return;
  const [m] = ids.splice(from, 1);
  ids.splice(to, 0, m);
  saveOrder(ids);
}

// Dragging by the grip (pointer events: HTML drag and drop isn't reliable in WebView2).
let drag = null;
$("slots").addEventListener("pointerdown", (e) => {
  const grip = e.target.closest(".grip");
  if (!grip) return;
  const row = grip.closest(".slot");
  drag = { from: Number(row.dataset.i), to: null, row };
  row.classList.add("dragging");
  grip.setPointerCapture(e.pointerId);
  e.preventDefault();
});
$("slots").addEventListener("pointermove", (e) => {
  if (!drag) return;
  const rows = [...$("slots").querySelectorAll(".slot")];
  rows.forEach((r) => r.classList.remove("drop-before", "drop-after"));
  let target = rows.length - 1, after = true;
  for (let i = 0; i < rows.length; i++) {
    const r = rows[i].getBoundingClientRect();
    if (e.clientY < r.top + r.height / 2) { target = i; after = false; break; }
  }
  rows[target].classList.add(after ? "drop-after" : "drop-before");
  let to = after ? target + 1 : target;
  if (to > drag.from) to--;
  drag.to = to;
});
const endDrag = () => {
  if (!drag) return;
  const { from, to } = drag;
  drag = null;
  if (to === null || to === from) return renderWheel();
  moveSlot(from, to);
};
$("slots").addEventListener("pointerup", endDrag);
$("slots").addEventListener("pointercancel", () => { drag = null; renderWheel(); });
$("slots").addEventListener("keydown", (e) => {
  const grip = e.target.closest(".grip");
  if (!grip || !e.altKey || (e.key !== "ArrowUp" && e.key !== "ArrowDown")) return;
  e.preventDefault();
  const from = Number(grip.closest(".slot").dataset.i);
  const to = from + (e.key === "ArrowUp" ? -1 : 1);
  moveSlot(from, to);
  const g = $("slots").querySelector(`.slot[data-i="${Math.max(0, Math.min(to, famInfo().targets.length - 1))}"] .grip`);
  if (g) g.focus();
});
$("slots").addEventListener("click", (e) => {
  const box = e.target.closest(".box");
  if (!box) return;
  const id = box.closest(".slot").dataset.id;
  const t = famInfo().targets.find((x) => x.id === id);
  t.hidden = !t.hidden;
  const hidden = new Set(view.settings.hidden);
  if (t.hidden) hidden.add(id); else hidden.delete(id);
  renderWheel();
  save({ hidden: [...hidden] });
});
$("fams").addEventListener("click", (e) => {
  const b = e.target.closest("[data-fam]");
  if (b) { fam = b.dataset.fam; renderWheel(); }
});
$("reset-fam").addEventListener("click", () => {
  const f = famInfo();
  const ids = new Set(f.targets.map((t) => t.id));
  save({ order: { [fam]: [] }, hidden: view.settings.hidden.filter((h) => !ids.has(h)) });
});

// ---------- Quality page ----------

function renderQuality() {
  const q = Object.assign({}, QDEFAULT, view.settings.quality);
  $("qgroups").innerHTML = QGROUPS.map((g) =>
    `<h2>${esc(g.title)}</h2><div class="card list">` + g.rows.map((r) => {
      const id = "q-" + r.k;
      const kind = r.tune;
      const grade = kind ? GRADE_OF[q[r.k]] : null;
      const control = r.options
        ? `<select id="${id}" data-q="${r.k}">${r.options.map(([v, l]) => `<option value="${esc(v)}"${String(v) === String(q[r.k]) ? " selected" : ""}>${esc(l)}${kind && GRADE_OF[v] && tuneEdited(kind, GRADE_OF[v]) ? " ●" : ""}</option>`).join("")}</select>`
        : `<input id="${id}" type="range" data-q="${r.k}" min="${r.min}" max="${r.max}" step="1" value="${q[r.k]}"><span class="val" data-v="${r.k}">${q[r.k]}</span>`;
      if (!kind) return `<div class="qrow"><label for="${id}" class="grow"><div>${esc(r.label)}</div><div class="sub">${esc(r.desc)}</div></label>${control}</div>`;
      const edited = grade && tuneEdited(kind, grade);
      const open = !!(grade && tuneOpen[kind]);
      const button = `<button type="button" class="btn tune-btn" data-tune="${kind}" aria-expanded="${open}"${grade ? "" : ' disabled title="Fine-tune works with Smaller, Balanced or Best"'}>Fine-tune ${CHEVRON}</button>`;
      return `<div class="qrow"><label for="${id}" class="grow"><div>${esc(r.label)}${edited ? '<span class="edited">● Customised</span>' : ""}</div><div class="sub">${esc(r.desc)}</div></label>${control}${button}</div>` +
        (grade ? `<div class="tune-stats">${tuneStatsLine(kind, grade)}</div>` : "") +
        (open ? tunePanel(kind, grade) : "");
    }).join("") + `</div>`).join("");
  renderImport();
}

$("qgroups").addEventListener("input", (e) => {
  const k = e.target.dataset.q;
  const v = $("qgroups").querySelector(`[data-v="${k}"]`);
  if (v) v.textContent = e.target.value;
});
$("qgroups").addEventListener("change", (e) => {
  const k = e.target.dataset.q;
  if (!k) return;
  const raw = e.target.value;
  save({ quality: { [k]: typeof QDEFAULT[k] === "number" ? Number(raw) : raw } });
});
$("reset-quality").addEventListener("click", async () => {
  try { await invoke("presets_apply", { tune: {} }); } catch (e) { /* the rest still resets */ }
  await save({ quality: Object.assign({}, QDEFAULT) });
  loadTuneInfo();
});

// ---------- Tune: the settings behind each preset ----------
// Each preset keeps only what was changed (settings.quality.tune.<kind>.<grade>);
// the rest is Convertino's default (tune_info). Rust keeps the values in range and
// the look scores in order (Smaller ≤ Balanced ≤ Best).

const GRADES = ["small", "balanced", "best"];
const GRADE_OF = { small: "small", balanced: "balanced", best: "best", high: "best" };
const GRADE_NAME = { small: "Smaller", balanced: "Balanced", best: "Best" };
const CHEVRON = '<svg class="chev" viewBox="0 0 16 16" aria-hidden="true"><path d="M4 6l4 4 4-4"/></svg>';
const LOOK_WORDS = [[0, "visibly softer"], [70, "looks very close"], [80, "looks the same"], [87, "the same side by side"], [95, "near identical"]];
const VLOOK_WORDS = [[0, "visibly softer"], [89, "looks very close"], [93, "looks the same"], [95.5, "the same side by side"], [99, "near identical"]];
const lookWord = (words, v) => words.reduce((w, [n, s]) => (v >= n ? s : w), words[0][1]);
const nth = (n) => n + (n % 100 >= 11 && n % 100 <= 13 ? "th" : ["th", "st", "nd", "rd"][n % 10] || "th");
const TUNE = {
  image: [
    { g: "Quality" },
    { k: "look", label: "Quality target", tech: "SSIMULACRA2 score", min: 50, max: 95, step: 1, ends: ["Smaller file", "More detail"], words: LOOK_WORDS, order: true },
    { k: "floor", label: "Lowest quality allowed", tech: "Stops busy photos going blurry · JPG and WebP (AVIF 10 lower)", min: 30, max: 90, step: 1, ends: ["30", "90"] },
    { g: "Speed and privacy" },
    { k: "png", label: "PNG compression effort", tech: "Same picture, just more time for a smaller file", seg: [[0, "Quick"], [1, "Normal"], [2, "Thorough"]] },
    { k: "stripGps", label: "Remove location (GPS)", tech: "Keeps the camera, date and other photo details", toggle: true },
  ],
  video: [
    { g: "Quality" },
    { k: "look", label: "Quality target", tech: "VMAF score", min: 80, max: 99, step: 0.5, ends: ["Smaller file", "More detail"], words: VLOOK_WORDS, order: true },
    { k: "codec", label: "Video format", tech: "For Compress; converting to MP4 stays H.264", seg: [["h265", "H.265: smaller"], ["h264", "H.264: plays everywhere"], ["av1", "AV1: smallest, newer devices"]] },
    { g: "Size limits (Compress only)" },
    { k: "maxRes", label: "Maximum resolution", tech: "Bigger videos are scaled down; smaller ones are left alone", seg: [[0, "Original"], [2160, "4K"], [1440, "1440p"], [1080, "1080p"], [720, "720p"]] },
    { k: "maxFps", label: "Maximum frame rate", tech: "Higher rates are reduced evenly: 144 → 48, 120 → 60", seg: [[0, "Original"], [60, "60 fps"], [30, "30 fps"]] },
    { k: "audio", label: "Audio quality", tech: "AAC; audio that's already this small is kept as it is", seg: [[96, "96 kbps"], [128, "128 kbps"], [192, "192 kbps"]] },
    { g: "Speed" },
    { k: "encoder", label: "Encoder", tech: "Processor means x265/x264 · Auto measures both now and then", seg: [["gpu", "Graphics card: fast"], ["cpu", "Processor: smaller files"], ["auto", "Auto: picks per video"]] },
    { k: "effort", label: "Encoding speed", tech: "x265 fast / medium / slow · NVIDIA p4 / p5 / p7", seg: [[0, "Fast: bigger file"], [1, "Balanced"], [2, "Thorough: smaller file"]] },
    { k: "recheck", label: "Re-check quality every", tech: "Once a setting has worked for a kind of video, the check is skipped in between", min: 1, max: 20, step: 1, ends: ["Every video", "Every 20th"], fmt: (v) => (v <= 1 ? "video" : nth(v)) },
  ],
  pdf: [
    { g: "Quality" },
    { k: "look", label: "Quality target", tech: "SSIMULACRA2 on rendered pages", min: 50, max: 95, step: 1, ends: ["Smaller file", "More detail"], words: LOOK_WORDS, order: true },
    { k: "checkDpi", label: "Comparison detail", tech: "Finer catches small text damage, and takes longer", seg: [[110, "Normal"], [150, "Fine"], [200, "Finest"]] },
  ],
};

let tuneInfo = null;                                  // { defaults, stats, av1 }
const tuneOpen = { image: false, video: false, pdf: false };
const tuneTests = {};                                 // kind -> { state, name, result, error }
let importPending = null;                             // { path, tune, changes }

async function loadTuneInfo() {
  try { tuneInfo = await invoke("tune_info"); } catch (e) { tuneInfo = null; }
  if (page === "quality") renderQuality();
}

function tuneStored(kind, grade) {
  const t = (view.settings.quality && view.settings.quality.tune) || {};
  return (t[kind] && t[kind][grade]) || {};
}

function tuneEdited(kind, grade) {
  const s = tuneStored(kind, grade);
  return Object.keys(s).some((k) => s[k] !== null && s[k] !== undefined);
}

function tuneValues(kind, grade) {
  const d = (tuneInfo && tuneInfo.defaults && tuneInfo.defaults[grade] && tuneInfo.defaults[grade][kind]) || {};
  const s = tuneStored(kind, grade);
  const out = Object.assign({}, d);
  for (const k in s) if (s[k] !== null && s[k] !== undefined) out[k] = s[k];
  return out;
}

/// A look score can't pass the neighbouring presets'.
function lookBounds(kind, grade, knob) {
  const i = GRADES.indexOf(grade);
  const lo = i > 0 ? tuneValues(kind, GRADES[i - 1]).look : knob.min;
  const hi = i < 2 ? tuneValues(kind, GRADES[i + 1]).look : knob.max;
  return { lo: Math.max(knob.min, lo ?? knob.min), hi: Math.min(knob.max, hi ?? knob.max), below: i > 0 ? GRADES[i - 1] : null, above: i < 2 ? GRADES[i + 1] : null };
}

function duration(sec) {
  if (sec >= 90) return `${Math.round(sec / 60)} min`;
  if (sec >= 10) return `${Math.round(sec)} s`;
  return `${(Math.round(sec * 10) / 10)} s`;
}

function tuneStatsLine(kind, grade) {
  if (!tuneInfo) return "";
  const s = tuneInfo.stats && tuneInfo.stats[`${kind}.${grade}`];
  if (!s || !s.count) return esc(tuneEdited(kind, grade) ? "No compressions since you customised it yet." : "No compressions with this preset yet.");
  const smaller = Math.round((1 - s.ratio) * 100);
  const what = s.count === 1 ? "Last compression" : `Last ${s.count} compressions`;
  return `${what}: <b>${smaller}% smaller</b>${s.count === 1 ? "" : " on average"}, <b>${duration(s.seconds)}</b> each`;
}

function tunePanel(kind, grade) {
  const v = tuneValues(kind, grade);
  const knobs = TUNE[kind].map((k) => {
    if (k.g) return `<div class="tune-group">${esc(k.g)}</div>`;
    const val = v[k.k];
    const head = `<div class="knob-text"><div>${esc(k.label)}</div><div class="sub">${esc(k.tech)}${k.words ? ` · this level: <span data-word="${k.k}">${esc(lookWord(k.words, val))}</span>` : ""}</div></div>`;
    if (k.toggle) {
      return `<div class="knob">${head}<button type="button" class="switch" role="switch" data-tswitch="${k.k}" aria-checked="${!!val}"><span></span></button></div>`;
    }
    if (k.seg) {
      return `<div class="knob">${head}<div class="seg" role="radiogroup" aria-label="${esc(k.label)}">` + k.seg.map(([sv, label]) => {
        const off = k.k === "codec" && sv === "av1" && tuneInfo && !tuneInfo.av1;
        return `<button type="button" role="radio" data-tseg="${k.k}" data-v="${esc(sv)}" aria-checked="${String(sv) === String(val)}"${off ? ' disabled title="There\'s no AV1 encoder on this computer"' : ""}>${esc(label)}</button>`;
      }).join("") + `</div></div>`;
    }
    let hint = "";
    if (k.order) {
      const b = lookBounds(kind, grade, k);
      if (b.below && val <= b.lo) hint = `Can't go below ${GRADE_NAME[b.below]} (${b.lo}).`;
      else if (b.above && val >= b.hi) hint = `Can't go above ${GRADE_NAME[b.above]} (${b.hi}).`;
    }
    const shown = k.fmt ? k.fmt(val) : val;
    return `<div class="knob">${head}<div class="knob-range"><input type="range" data-trange="${k.k}" min="${k.min}" max="${k.max}" step="${k.step}" value="${val}" aria-label="${esc(k.label)}">` +
      `<span class="ends"><span>${esc(k.ends[0])}</span><span>${esc(k.ends[1])}</span></span></div><span class="val" data-tval="${k.k}">${esc(shown)}</span>` +
      `<div class="knob-hint" data-thint="${k.k}">${esc(hint)}</div></div>`;
  }).join("");
  return `<div class="tune-panel" data-tkind="${kind}" data-tgrade="${grade}">${knobs}${testBox(kind)}` +
    `<div class="tune-foot"><span class="sub">Changes save straight away and apply everywhere: the wheel, Compress, the command line and AI apps.</span>` +
    `<button type="button" class="link" data-treset${tuneEdited(kind, grade) ? "" : " disabled"}>Restore ${GRADE_NAME[grade]} defaults</button></div></div>`;
}

function panelGrade(kind) {
  const key = { image: "image", video: "video", pdf: "pdfCompress" }[kind];
  return GRADE_OF[(view.settings.quality || {})[key]] || "balanced";
}

function testBox(kind) {
  const t = tuneTests[kind] || { state: "idle" };
  const what = kind === "video" ? "measures a few seconds of a video and makes a short sample" : "compresses a copy";
  if (t.state === "running") {
    return `<div class="test-box" data-test-kind="${kind}"><div class="grow"><b>${esc(t.name)}</b><div class="sub">Testing…</div><div class="test-bar"><i></i></div></div><button type="button" class="btn" data-tcancel>Cancel</button></div>`;
  }
  if (t.state === "error") {
    return `<div class="test-box" data-test-kind="${kind}"><div class="grow"><b>${esc(t.name)}</b><div class="sub bad">${esc(t.error)}</div></div><button type="button" class="btn" data-ttest>Choose file…</button></div>`;
  }
  if (t.state === "done") {
    const r = t.result;
    const ok = r.score === null || r.score === undefined || r.score >= r.target - 0.05;
    const look = r.score === null || r.score === undefined ? "Checked" : `${Math.round(r.score * 10) / 10} <span class="sub">target ${r.target}</span>`;
    const time = r.estimate ? `about ${duration(r.seconds)}` : duration(r.seconds);
    return `<div class="test-box done" data-test-kind="${kind}"><div class="grow"><b>${esc(r.name)}</b> <span class="${ok ? "good" : "warnc"}">${ok ? (tuneEdited(kind, panelGrade(kind)) ? "✓ Meets your quality target" : "✓ Looks the same as the original") : "Couldn't reach the quality target; the closest setting is used"}</span>` +
      `<div class="test-grid"><div><span>Size</span><b>${Math.round((1 - r.ratio) * 100)}% smaller</b> <span class="sub">${Math.max(1, Math.round(r.ratio * 100))}% of the original</span></div><div><span>Quality</span><b>${look}</b></div>` +
      `<div><span>${r.estimate ? "Time for the whole video" : "Took"}</span><b>${esc(time)}</b></div><div><span>Used</span><b>${esc(r.setting)}</b></div></div></div>` +
      `<div class="test-actions"><button type="button" class="btn primary" data-tcompare="${r.compare}">Compare</button><button type="button" class="link" data-ttest>Try another file</button></div></div>`;
  }
  return `<div class="test-box" data-test-kind="${kind}"><div class="grow"><b>Try these settings on a file</b><div class="sub">Pick a file (or drop one here): ${what}. Nothing is saved next to it.</div></div><button type="button" class="btn" data-ttest>Choose file…</button></div>`;
}

function saveTune(kind, grade, key, value) {
  delete tuneTests[kind];
  return save({ quality: { tune: { [kind]: { [grade]: { [key]: value } } } } }).then(loadTuneInfo);
}

async function runTest(kind, grade, path) {
  const name = path.split(/[\\/]/).pop();
  tuneTests[kind] = { state: "running", name };
  renderQuality();
  try {
    const result = await invoke("tune_test", { kind, grade, quality: view.settings.quality, path });
    tuneTests[kind] = { state: "done", result };
  } catch (e) {
    const msg = String(e);
    tuneTests[kind] = /cancel/i.test(msg) ? undefined : { state: "error", name, error: msg };
    if (!tuneTests[kind]) delete tuneTests[kind];
  }
  renderQuality();
}

$("qgroups").addEventListener("click", async (e) => {
  const tb = e.target.closest("[data-tune]");
  if (tb) { tuneOpen[tb.dataset.tune] = !tuneOpen[tb.dataset.tune]; renderQuality(); return; }
  const panel = e.target.closest(".tune-panel");
  if (!panel) return;
  const kind = panel.dataset.tkind, grade = panel.dataset.tgrade;
  const seg = e.target.closest("[data-tseg]");
  if (seg && !seg.disabled) {
    const raw = seg.dataset.v;
    return saveTune(kind, grade, seg.dataset.tseg, /^-?\d+(\.\d+)?$/.test(raw) ? Number(raw) : raw);
  }
  const sw = e.target.closest("[data-tswitch]");
  if (sw) return saveTune(kind, grade, sw.dataset.tswitch, sw.getAttribute("aria-checked") !== "true");
  if (e.target.closest("[data-treset]")) {
    const keys = Object.keys(tuneStored(kind, grade));
    delete tuneTests[kind];
    return save({ quality: { tune: { [kind]: { [grade]: Object.fromEntries(keys.map((k) => [k, null])) } } } }).then(loadTuneInfo);
  }
  if (e.target.closest("[data-ttest]")) {
    const path = await invoke("tune_pick_file").catch(() => null);
    if (path) runTest(kind, grade, path);
    return;
  }
  if (e.target.closest("[data-tcancel]")) { invoke("tune_test_cancel").catch(() => {}); return; }
  const cmp = e.target.closest("[data-tcompare]");
  if (cmp) invoke("compare_open", { id: Number(cmp.dataset.tcompare) }).catch((err) => toast(String(err)));
});

$("qgroups").addEventListener("input", (e) => {
  const key = e.target.dataset.trange;
  if (!key) return;
  const panel = e.target.closest(".tune-panel");
  const kind = panel.dataset.tkind, grade = panel.dataset.tgrade;
  const knob = TUNE[kind].find((k) => k.k === key);
  let v = Number(e.target.value);
  let hint = "";
  if (knob.order) {
    const b = lookBounds(kind, grade, knob);
    if (b.below && v <= b.lo) { v = b.lo; hint = `Can't go below ${GRADE_NAME[b.below]} (${b.lo}).`; }
    if (b.above && v >= b.hi) { v = b.hi; hint = `Can't go above ${GRADE_NAME[b.above]} (${b.hi}).`; }
    e.target.value = v;
  }
  panel.querySelector(`[data-tval="${key}"]`).textContent = knob.fmt ? knob.fmt(v) : v;
  const w = panel.querySelector(`[data-word="${key}"]`);
  if (w && knob.words) w.textContent = lookWord(knob.words, v);
  const h = panel.querySelector(`[data-thint="${key}"]`);
  if (h) h.textContent = hint;
});

$("qgroups").addEventListener("change", (e) => {
  const key = e.target.dataset.trange;
  if (!key) return;
  const panel = e.target.closest(".tune-panel");
  saveTune(panel.dataset.tkind, panel.dataset.tgrade, key, Number(e.target.value));
});

// A file dropped on a Test box is tested there.
if (tauri && tauri.webview && tauri.webview.getCurrentWebview) {
  try {
    tauri.webview.getCurrentWebview().onDragDropEvent((ev) => {
      const p = ev.payload;
      if (!p || p.type !== "drop" || !p.paths || !p.paths.length || page !== "quality") return;
      const scale = window.devicePixelRatio || 1;
      const el = document.elementFromPoint(p.position.x / scale, p.position.y / scale);
      const box = el && el.closest("[data-test-kind]");
      const panel = box && box.closest(".tune-panel");
      if (panel) runTest(panel.dataset.tkind, panel.dataset.tgrade, p.paths[0]);
    });
  } catch (e) { /* no drag and drop here */ }
}

// ---------- export and import presets ----------

$("presets-export").addEventListener("click", async () => {
  try {
    const path = await invoke("presets_export");
    if (path) toast(`Saved ${path.split(/[\\/]/).pop()}`);
  } catch (e) { toast(String(e)); }
});

$("presets-import").addEventListener("click", async () => {
  try {
    importPending = await invoke("presets_import");
  } catch (e) { toast(String(e)); importPending = null; }
  renderImport();
});

function renderImport() {
  const box = $("import-box");
  if (!importPending) { box.hidden = true; box.innerHTML = ""; return; }
  const name = importPending.path.split(/[\\/]/).pop();
  const list = importPending.changes.length
    ? `<ul>${importPending.changes.map((c) => `<li>${esc(c)}</li>`).join("")}</ul>`
    : `<div class="sub">It's the same as what you have now.</div>`;
  box.innerHTML = `<div><b>${esc(name)}</b> would change:</div>${list}` +
    `<div class="buttons end"><button type="button" class="btn" data-icancel>Cancel</button><button type="button" class="btn primary" data-iapply${importPending.changes.length ? "" : " disabled"}>Use these presets</button></div>`;
  box.hidden = false;
}

$("import-box").addEventListener("click", async (e) => {
  if (e.target.closest("[data-icancel]")) { importPending = null; renderImport(); return; }
  if (e.target.closest("[data-iapply]") && importPending) {
    try { view = await invoke("presets_apply", { tune: importPending.tune }); toast("Presets imported."); } catch (err) { toast(String(err)); }
    importPending = null;
    render();
    loadTuneInfo();
  }
});

// ---------- Converters page ----------

async function refreshTools() {
  try {
    tools = await invoke("converters_status");
  } catch (e) {
    tools = [];
  }
  renderConverters();
  const busy = tools.some((t) => t.state === "downloading");
  clearTimeout(pollTimer);
  if (busy && page === "converters") pollTimer = setTimeout(refreshTools, 1500);
  if (!encoderLoaded && page === "converters") {
    encoderLoaded = true;
    invoke("video_encoder").then((name) => { $("encoder").textContent = name; }).catch(() => { $("encoder").textContent = "Unknown"; });
  }
}

const updates = new Set();

function renderConverters() {
  if (!view) return;
  const s = view.settings;
  document.querySelectorAll("[data-ff]").forEach((el) => el.setAttribute("aria-checked", String(s.ffmpegBuild === el.dataset.ff)));
  if (!tools.length) return;
  $("tools").innerHTML = tools.map((t) => {
    const p = progress[t.id];
    const busy = t.state === "downloading" || p;
    let status;
    if (busy) {
      const pct = p ? Math.round(p.fraction * 100) : 0;
      status = `<span class="bar"><i style="width:${pct}%"></i></span><span class="pct">${pct ? pct + "%" : "…"}</span>`;
    } else if (t.state === "ready" && updates.has(t.id)) {
      status = `<button type="button" class="btn" data-get="${t.id}" data-replace="1">Update</button>`;
    } else if (t.state === "ready" || t.state === "system" || t.state === "bundled") {
      status = `<span class="ready"><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><path d="${OK}"/></svg>Ready</span>`;
    } else {
      status = `<button type="button" class="btn" data-get="${t.id}">Download now</button>`;
    }
    const size = t.state === "ready"
      ? `${t.version ? esc(t.version) + " · " : ""}${t.sizeMb} MB`
      : t.state === "system" ? (isMac() ? "Installed on this Mac" : "Installed on this PC")
      : t.state === "bundled" ? "Comes with Convertino" : `about ${t.downloadMb} MB to download`;
    const detail = p && p.detail ? `<span class="detail">${esc(p.detail)}</span>` : "";
    return `<div class="tool"><span class="name">${esc(t.name)}</span><span class="what">${esc(t.what)}${detail}</span>` +
      `<span class="size">${size}</span><span class="status">${status}</span></div>`;
  }).join("");

  const used = tools.filter((t) => t.state === "ready").reduce((a, t) => a + t.sizeMb, 0);
  $("used").textContent = `Using ${used} MB now.`;
  const missing = tools.filter((t) => t.state === "missing");
  const mb = missing.reduce((a, t) => a + t.downloadMb, 0);
  $("get-all").textContent = missing.length ? `Download all now (about ${mb} MB)` : "All downloaded";
  $("get-all").disabled = !missing.length;
  $("remove-all").disabled = view.dev || !tools.some((t) => t.state === "ready");
  $("remove-all").title = view.dev ? "Turned off in development builds" : "";
}

function toolsNote(text, bad) {
  const n = $("tools-msg");
  n.hidden = !text;
  n.textContent = text || "";
  n.classList.toggle("bad", !!bad);
}

function download(id, replace) {
  progress[id] = { fraction: 0, detail: "Starting…" };
  renderConverters();
  invoke("converter_download", { id, replace: !!replace }).catch((e) => {
    delete progress[id];
    toolsNote(String(e), true);
    renderConverters();
  });
}

$("tools").addEventListener("click", (e) => {
  const b = e.target.closest("[data-get]");
  if (b) download(b.dataset.get, b.dataset.replace === "1");
});
$("get-all").addEventListener("click", () => {
  // One at a time is kinder to a slow connection; 7-Zip goes first anyway.
  const queue = tools.filter((t) => t.state === "missing").map((t) => t.id);
  const next = () => {
    const id = queue.shift();
    if (!id) return;
    download(id, false);
    const wait = setInterval(() => {
      if (!progress[id]) { clearInterval(wait); next(); }
    }, 500);
  };
  next();
});
$("check-updates").addEventListener("click", async () => {
  const b = $("check-updates");
  b.disabled = true;
  b.textContent = "Checking…";
  try {
    const newer = await invoke("converters_check");
    updates.clear();
    newer.forEach((id) => updates.add(id));
    b.textContent = newer.length ? `${newer.length} update${newer.length > 1 ? "s" : ""} available` : "Everything is up to date";
  } catch (e) {
    b.textContent = "Check for updates";
    toolsNote(String(e), true);
  }
  b.disabled = false;
  renderConverters();
});
$("remove-all").addEventListener("click", async () => {
  try {
    const mb = await invoke("converters_remove");
    toolsNote(`Removed. ${mb} MB freed; each converter downloads again the next time it's needed.`);
  } catch (e) {
    toolsNote(String(e), true);
  }
  refreshTools();
});
document.querySelectorAll("[data-ff]").forEach((el) => el.addEventListener("click", async () => {
  const build = el.dataset.ff;
  if (view.settings.ffmpegBuild === build) return;
  await save({ ffmpegBuild: build });
  const ff = tools.find((t) => t.id === "ffmpeg");
  // Swap the FFmpeg that's there; if there's none yet, the right one comes on first use.
  if (ff && ff.state === "ready" && ff.build !== build) {
    toolsNote(`Getting the ${build === "gpl" ? "x264/x265" : "standard"} FFmpeg…`);
    download("ffmpeg", true);
    encoderLoaded = false;
  }
}));

// ---------- General page wiring ----------

$("nav").addEventListener("click", (e) => {
  const b = e.target.closest("[data-go]");
  if (b) setPage(b.dataset.go);
});
document.querySelectorAll(".switch[data-key]").forEach((el) => el.addEventListener("click", () => {
  const k = el.dataset.key;
  save({ [k]: !view.settings[k] });
}));
$("rec-start").addEventListener("click", startRecording);
$("perm-ax-btn").addEventListener("click", () => invoke("permissions_request", { kind: "accessibility" }).then(refreshPerms));
$("perm-finder-btn").addEventListener("click", () => invoke("permissions_request", { kind: "finder" }).then(() => setTimeout(refreshPerms, 500)));
$("rec-cancel").addEventListener("pointerdown", (e) => { e.preventDefault(); stopRecording(); });
$("recorder").addEventListener("keydown", onRecordKey);
$("recorder").addEventListener("blur", () => setTimeout(() => recording && stopRecording(), 0));
$("rec-default").addEventListener("click", async () => {
  try {
    const active = await invoke("shortcut_set", { shortcut: null });
    view.shortcut = active;
    view.settings.shortcut = null;
    showRecMsg(true, `Saved. ${pretty(active)} now opens the wheel.`, false);
  } catch (e) {
    showRecMsg(false, String(e), false);
  }
  renderShortcut();
});

document.querySelectorAll("[data-save]").forEach((el) => el.addEventListener("click", async () => {
  if (el.dataset.save === "folder" && !view.settings.saveFolder) return chooseFolder();
  save({ saveMode: el.dataset.save });
}));
async function chooseFolder() {
  const folder = await invoke("choose_folder").catch(() => null);
  if (folder) save({ saveMode: "folder", saveFolder: folder });
}
$("folder-choose").addEventListener("click", chooseFolder);
$("handoff").addEventListener("change", (e) => save({ handoffSeconds: Number(e.target.value) }));

// ---------- About ----------

$("licence-rows").innerHTML = LICENCES.map(([a, b]) => `<tr><td>${esc(a)}</td><td>${esc(b)}</td></tr>`).join("");
$("show-licences").addEventListener("click", () => $("licences").showModal());
$("licences-close").addEventListener("click", () => $("licences").close());
$("open-logs").addEventListener("click", () => invoke("open_logs").catch((e) => toast(String(e))));
$("show-activity").addEventListener("click", () => invoke("show_activity").catch(() => {}));
const openLink = (url) => invoke("open_link", { url }).catch((e) => toast(String(e)));
$("sponsor").addEventListener("click", () => openLink("https://github.com/sponsors/sanyamgoelx"));
$("open-repo").addEventListener("click", () => openLink("https://github.com/sanyamgoelx/convertino"));
$("upi").addEventListener("click", () => $("upi-dialog").showModal());
$("upi-close").addEventListener("click", () => $("upi-dialog").close());
$("upi-copy").addEventListener("click", async () => {
  try {
    await navigator.clipboard.writeText($("upi-id").textContent);
    $("upi-copy").textContent = "Copied";
    setTimeout(() => { $("upi-copy").textContent = "Copy"; }, 1500);
  } catch (e) { toast("Couldn't copy; select the ID and copy it instead."); }
});

// Updates (Settings > About).
let update = null;
let updating = false;
function renderUpdate() {
  const sub = $("update-sub"), btn = $("update-btn");
  if (!update) return;
  if (updating) return;
  if (update.dev) {
    sub.textContent = `Version ${update.current} · development builds don't update`;
    btn.disabled = true;
  } else if (update.available) {
    sub.textContent = `Convertino ${update.available} is ready (you have ${update.current}). Convertino restarts on the new version.`;
    btn.textContent = "Update and restart";
    btn.classList.add("primary");
    btn.disabled = false;
  } else {
    sub.textContent = `You have the latest version (${update.current}). Checked automatically once a day.`;
    btn.textContent = "Check now";
    btn.classList.remove("primary");
    btn.disabled = false;
  }
}
async function loadUpdate() {
  try { update = await invoke("update_status"); } catch (e) { return; }
  renderUpdate();
}
$("update-btn").addEventListener("click", async () => {
  const btn = $("update-btn"), sub = $("update-sub");
  if (update && update.available) {
    updating = true;
    btn.disabled = true;
    sub.textContent = "Downloading the update…";
    try { await invoke("update_install"); } catch (e) { updating = false; toast(String(e)); loadUpdate(); }
    return;
  }
  btn.disabled = true;
  sub.textContent = "Checking…";
  try { update = await invoke("update_check"); } catch (e) { toast(String(e)); }
  btn.disabled = false;
  renderUpdate();
});
if (tauri) {
  tauri.event.listen("update-progress", (e) => { $("update-sub").textContent = e.payload.detail; });
}

function toast(text) {
  const t = document.createElement("div");
  t.className = "toast";
  t.setAttribute("role", "status");
  t.textContent = text;
  document.body.appendChild(t);
  setTimeout(() => t.remove(), 3500);
}

// ---------- AI & CLI ----------

let ai = null;
const APP_ICON = { "claude-desktop": "M4 5h16v14H4z M4 9h16", "claude-code": "M8 8l-4 4 4 4 M16 8l4 4-4 4", cursor: "M8 8l-4 4 4 4 M16 8l4 4-4 4" };

function ago(ms) {
  const s = Math.max(0, (Date.now() - ms) / 1000);
  if (s < 90) return "just now";
  if (s < 3600) return `${Math.round(s / 60)} min ago`;
  if (s < 86400) return `${Math.round(s / 3600)} h ago`;
  if (s < 2 * 86400) return "yesterday";
  return new Date(ms).toLocaleDateString();
}

async function refreshAi() {
  try { ai = await invoke("ai_status"); } catch (e) { ai = null; }
  renderAi();
}

function renderAi() {
  if (!ai) return;
  const mac = isMac();
  const c = ai.command;
  const st = $("cli-state");
  st.className = "perm-state " + (c.installed ? "ok" : "todo");
  st.innerHTML = c.installed
    ? `<svg class="ic sm" viewBox="0 0 24 24" aria-hidden="true"><path d="${OK}"/></svg>Ready`
    : (c.path ? "Not installed" : "Not in this copy");
  $("cli-install").hidden = !(mac && c.canInstall && !c.installed);
  if (mac) {
    $("cli-sub").textContent = c.installed ? "Works in Terminal. Same converters and quality settings as the wheel." : "Adds it to /usr/local/bin so Terminal finds it. macOS asks for your password once.";
    $("cli-note").textContent = c.installed ? "Linked to the command inside Convertino" : "";
  } else if (!c.installed) {
    $("cli-note").textContent = view && view.dev ? "Development build: the installer adds the command" : "Reinstall Convertino to add the command";
  }

  $("ai-apps").innerHTML = ai.apps.map((a) => {
    const state = a.connected && !a.outdated ? `<span class="perm-state ok"><svg class="ic sm" viewBox="0 0 24 24" aria-hidden="true"><path d="${OK}"/></svg>Connected</span>` : "";
    const sub = a.connected ? (a.outdated ? "Points at an older copy of Convertino" : (a.id === "claude-desktop" ? "Also used by Cowork on this computer" : "Restart it if it was open")) : (a.found ? `Found on this ${mac ? "Mac" : "PC"}` : "Not installed");
    const btn = a.connected && !a.outdated
      ? `<button type="button" class="btn wide" data-app="${a.id}" data-on="0">Disconnect</button>`
      : `<button type="button" class="btn wide${a.found ? " primary" : ""}" data-app="${a.id}" data-on="1"${a.found ? "" : " disabled"}>${a.outdated ? "Update" : "Connect"}</button>`;
    return `<div class="item${a.found ? "" : " off"}"><span class="app-ic"><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><path d="${APP_ICON[a.id] || APP_ICON["claude-desktop"]}"/></svg></span>` +
      `<div class="grow"><div class="name">${esc(a.name)}</div><div class="sub">${esc(sub)}</div></div>${state}${btn}</div>`;
  }).join("") +
    `<div class="item"><span class="app-ic"><svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><path d="M9 9h10v10H9z M5 15V5h10"/></svg></span>` +
    `<div class="grow"><div class="name">Another app</div><div class="sub">Paste the setup into its MCP settings</div></div>` +
    `<button type="button" class="btn wide" id="ai-copy">Copy setup</button></div>`;
  if (mac) {
    $("ai-note").textContent = "Connect adds Convertino to that app's settings. Quit and reopen the app afterwards.";
    $("aicard-sub").textContent = "The top-right card says which app asked, with Show in Finder and Undo";
    $("ai-sub").textContent = $("ai-sub").textContent.replace("this computer", "your Mac");
  }

  renderAsk();

  const jobs = ai.jobs || [];
  $("ai-jobs").innerHTML = jobs.length
    ? jobs.map((j, i) => `<div class="item"><div class="grow"><div>${esc(j.what)}</div><div class="sub">${esc(j.client)} · ${ago(j.at)}${j.folder ? " · " + esc(j.folder.split(/[\\/]/).pop()) : ""}</div></div>` +
        `<span class="perm-state ${j.ok ? "ok" : "todo"}">${esc(j.state)}</span>` +
        (j.folder ? `<button type="button" class="link" data-job="${i}">Show</button>` : "") + `</div>`).join("") +
      `<div class="item"><span class="grow sub">Kept on this computer only</span><button type="button" class="btn" id="ai-clear">Clear</button></div>`
    : `<div class="sub ai-empty">Nothing yet. Jobs an AI app asks for show here.</div>`;
}

// Ask Claude: shown on the wheel only when Convertino is connected to Claude
// (Claude Desktop for Chat, Claude Code for Code); see ask.rs.
const ASK_MODES = [
  { id: "chat", app: "claude-desktop", name: "Chat", sub: "Starts a new chat in Claude Desktop with the file paths written in. Claude converts on this computer." },
  { id: "code", app: "claude-code", name: "Claude Code", sub: "Opens a Claude Code session in the files' folder" },
];

function renderAsk() {
  if (!ai || !view) return;
  const s = view.settings;
  const on = (id) => ai.apps.some((a) => a.id === id && a.connected);
  const modes = ASK_MODES.filter((m) => on(m.app));
  const usable = modes.length > 0 && s.aiApps;
  $("ask-switch").hidden = !usable;
  $("ask-state").hidden = !usable;
  $("ask-modes").hidden = !usable || !s.askClaude || modes.length < 2;
  $("ask-card").classList.toggle("off", !usable);
  $("ask-sub").textContent = !s.aiApps
    ? "Turned off with “Let AI apps convert files” above"
    : !modes.length
      ? "Connect Convertino to Claude Desktop or Claude Code above, and an Ask Claude button appears on the wheel: it opens Claude with the files you right-clicked."
      : modes.length > 1
        ? "Opens Claude with the files you right-clicked; you say what to do with them. Shift+click it to choose where."
        : `Starts a new ${modes[0].id === "code" ? "Claude Code session" : "Claude chat"} with the files you right-clicked; you say what to do with them.`;
  const current = (modes.find((m) => m.id === s.askMode) || modes[0] || {}).id;
  $("ask-modes").innerHTML = modes.map((m) =>
    `<button type="button" class="radio top" role="radio" data-ask="${m.id}" aria-checked="${m.id === current}"><span class="dot"></span>` +
    `<span><span class="blk">${esc(m.name)}</span><span class="blk sub">${esc(m.sub)}</span></span></button>`).join("");
}

$("ask-modes").addEventListener("click", (e) => {
  const b = e.target.closest("[data-ask]");
  if (b) save({ askMode: b.dataset.ask });
});

$("ai-apps").addEventListener("click", async (e) => {
  const b = e.target.closest("[data-app]");
  if (b) {
    b.disabled = true;
    b.textContent = b.dataset.on === "1" ? "Connecting…" : "Disconnecting…";
    try {
      ai = await invoke("ai_connect", { id: b.dataset.app, on: b.dataset.on === "1" });
      renderAi();
      toast(b.dataset.on === "1" ? "Connected. Restart the app to use Convertino there." : "Disconnected.");
    } catch (err) {
      toast(String(err));
      refreshAi();
    }
    return;
  }
  if (e.target.closest("#ai-copy")) {
    const text = await invoke("ai_setup_snippet");
    try { await navigator.clipboard.writeText(text); toast("Setup copied."); } catch (_) { toast(text); }
  }
});
$("ai-jobs").addEventListener("click", async (e) => {
  const s = e.target.closest("[data-job]");
  if (s) { const j = ai.jobs[Number(s.dataset.job)]; if (j && j.folder) invoke("ai_open_folder", { path: j.folder }).catch((err) => toast(String(err))); }
  if (e.target.closest("#ai-clear")) { await invoke("ai_jobs_clear"); refreshAi(); }
});
$("cli-copy").addEventListener("click", async () => {
  try { await navigator.clipboard.writeText($("cli-examples").textContent); toast("Copied."); } catch (_) {}
});
$("cli-help").addEventListener("click", () => invoke("open_link", { url: "https://github.com/sanyamgoelx/convertino#command-line" }).catch(() => {}));
$("cli-install").addEventListener("click", async () => {
  const b = $("cli-install");
  b.disabled = true;
  try { ai = await invoke("command_install"); renderAi(); toast("Installed. Open a new Terminal window to use it."); }
  catch (err) { if (String(err) !== "Cancelled.") toast(String(err)); }
  b.disabled = false;
});

// ---------- start ----------

async function load() {
  view = await invoke("settings_get");
  render();
  refreshPerms();
  loadUpdate();
}

if (tauri) {
  tauri.event.listen("settings-changed", load);
  tauri.event.listen("settings-page", (e) => setPage(e.payload));
  tauri.event.listen("converter-progress", (e) => {
    const p = e.payload;
    progress[p.id] = p;
    if (page === "converters") renderConverters();
  });
  tauri.event.listen("converter-done", (e) => {
    const d = e.payload;
    delete progress[d.id];
    updates.delete(d.id);
    toolsNote(d.message, !d.ok);
    encoderLoaded = d.id === "ffmpeg" ? false : encoderLoaded;
    refreshTools();
  });
}
page = (location.hash || "#general").slice(1);
if (!NAV.some((n) => n.id === page)) page = "general";
load().then(() => setPage(page));

// ---------- in a plain browser: sample data, for checking the design ----------

function demoInvoke(cmd, args) {
  if (cmd === "update_status" || cmd === "update_check") return Promise.resolve({ current: "0.1.0", available: new URLSearchParams(location.search).get("update"), notes: null, dev: false });
  if (cmd === "permissions_get") return Promise.resolve({ mac: true, accessibility: true, altClickOn: true, finder: "not-asked" });
  if (cmd === "permissions_request") return Promise.resolve(null);
  const d = (window.__demo = window.__demo || {
    settings: {
      shortcut: "ctrl+alt+shift+KeyC", altClick: true, startAtLogin: true, progressRing: true, saveMode: "next", saveFolder: null,
      handoffSeconds: 2, learn: true, order: {}, hidden: [], quality: Object.assign({}, QDEFAULT), ffmpegBuild: "lgpl", picks: {}, aiApps: true, aiCard: true, askClaude: true, askMode: "chat",
    },
    shortcut: "ctrl+alt+shift+KeyC",
    families: [
      { id: "image", label: "Images", targets: [["image.jpg", "JPG", "Quality 90 · keeps EXIF"], ["image.png", "PNG", "Lossless"], ["image.webp", "WEBP", "Quality 85"], ["image.avif", "AVIF", "Smallest"], ["image.pdf", "PDF", "One page per picture"], ["image.ico", "ICO", "16 to 256 px"], ["image.compress", "Compress", "Pick a file size"], ["image.tiff", "TIFF", "Lossless"], ["image.bmp", "BMP", "Uncompressed"], ["image.gif", "GIF", "256 colours"]] },
      { id: "doc", label: "Documents", targets: [["doc.pdf", "PDF", "Keeps the layout"], ["doc.txt", "TXT", "Plain text"], ["doc.md", "Markdown", "Headings, lists"]] },
    ].map((f) => ({ id: f.id, label: f.label, targets: f.targets.map(([id, label, hint]) => ({ id, label, hint, hidden: false })) })),
    tools: [
      ["7zip", "7-Zip", "Archives", "ready", "26.03", 2, 2], ["ffmpeg", "FFmpeg", "Audio and video", "ready", "8.1", 128, 77],
      ["imagemagick", "ImageMagick", "Images", "ready", "7.1.2-32", 33, 11], ["poppler", "Poppler", "PDF pages and text", "missing", null, 0, 42],
      ["ghostscript", "Ghostscript", "PDF compress, grayscale", "missing", null, 0, 62], ["pandoc", "Pandoc", "Markdown and HTML", "missing", null, 0, 40],
      ["libreoffice", "LibreOffice", "Word, Excel, PowerPoint", "system", null, 0, 350],
    ].map(([id, name, what, state, version, sizeMb, downloadMb]) => ({ id, name, what, state, version, sizeMb, downloadMb, build: id === "ffmpeg" ? "lgpl" : null })),
  });
  const viewOf = () => ({ settings: JSON.parse(JSON.stringify(d.settings)), shortcut: d.shortcut, families: JSON.parse(JSON.stringify(d.families)), os: new URLSearchParams(location.search).get("os") || "win", version: "0.1.0", accent: null, dev: false });
  const merge = (a, b) => { for (const k in b) { if (b[k] && typeof b[k] === "object" && !Array.isArray(b[k]) && a[k] && typeof a[k] === "object") merge(a[k], b[k]); else a[k] = b[k]; } };
  switch (cmd) {
    case "settings_get": return Promise.resolve(viewOf());
    case "settings_set": {
      merge(d.settings, args.patch);
      d.families.forEach((f) => {
        const order = d.settings.order[f.id];
        if (order && order.length) f.targets.sort((x, y) => order.indexOf(x.id) - order.indexOf(y.id));
        f.targets.forEach((t) => { t.hidden = d.settings.hidden.includes(t.id); });
      });
      return Promise.resolve(viewOf());
    }
    case "shortcut_set":
      if (args.shortcut === "ctrl+alt+KeyC") return Promise.reject("Another app is already using Ctrl+Alt+C. Try a different one.");
      d.shortcut = args.shortcut || "ctrl+alt+shift+KeyC";
      return Promise.resolve(d.shortcut);
    case "converters_status": return Promise.resolve(JSON.parse(JSON.stringify(d.tools)));
    case "video_encoder": return Promise.resolve("NVIDIA graphics card");
    case "converters_check": return Promise.resolve(["ffmpeg"]);
    case "converters_remove": d.tools.forEach((t) => { if (t.state === "ready") { t.state = "missing"; t.sizeMb = 0; } }); return Promise.resolve(163);
    case "choose_folder": return Promise.resolve("D:\\Converted");
    case "tune_info": {
      const dv = (g) => ({
        image: { look: [70, 80, 87][g], floor: 40, png: g === 2 ? 2 : 1, stripGps: false },
        video: { look: [89, 93, 95.5][g], codec: "h265", maxRes: g === 0 ? 1080 : 0, maxFps: g === 0 ? 60 : 0, audio: 128, encoder: g === 2 ? "auto" : "gpu", effort: 1, recheck: 10 },
        pdf: { look: [70, 80, 87][g], checkDpi: g === 2 ? 150 : 110 },
      });
      return Promise.resolve({
        defaults: { small: dv(0), balanced: dv(1), best: dv(2) },
        stats: { "video.balanced": { count: 40, ratio: 0.31, seconds: 72 }, "image.balanced": { count: 12, ratio: 0.34, seconds: 0.6 } },
        av1: false,
      });
    }
    case "tune_pick_file": return Promise.resolve("E:\\Videos\\Valorant 2026-10-09.mp4");
    case "tune_test": return new Promise((res) => setTimeout(() => res({ name: "Valorant 2026-10-09.mp4", ratio: 0.27, score: 94.3, target: 93, setting: "H.265 on the graphics card, level 27", seconds: 95, estimate: true, compare: 7 }), 1500));
    case "presets_export": return Promise.resolve("C:\\Users\\you\\Documents\\convertino-presets.json");
    case "presets_import": return Promise.resolve({ path: "C:\\Users\\you\\Downloads\\crofty-presets.json", tune: { video: { balanced: { look: 91 } } }, changes: ["Video › Balanced: look 93.0 → 91.0", "Video › Balanced: encoder gpu → cpu"] });
    case "presets_apply": d.settings.quality.tune = args.tune; return Promise.resolve(viewOf());
    case "ai_status": return Promise.resolve(d.ai || (d.ai = {
      command: { installed: !new URLSearchParams(location.search).get("os"), path: "C:\\Users\\you\\AppData\\Local\\Convertino\\bin\\convertino.exe", canInstall: true },
      apps: [
        { id: "claude-desktop", name: "Claude Desktop", found: true, connected: true, outdated: false },
        { id: "claude-code", name: "Claude Code", found: true, connected: false, outdated: false },
        { id: "cursor", name: "Cursor", found: false, connected: false, outdated: false },
      ],
      jobs: [
        { at: Date.now() - 180000, client: "Claude Desktop", what: "12 files → jpg", ok: true, state: "Done", folder: "C:\\Users\\you\\Pictures\\Goa" },
        { at: Date.now() - 3600000, client: "Claude Code", what: "Contract.pdf → txt", ok: false, state: "This PDF is password-protected.", folder: null },
      ],
    }));
    case "ai_connect": { const a = d.ai.apps.find((x) => x.id === args.id); a.connected = args.on; return Promise.resolve(d.ai); }
    case "ai_setup_snippet": return Promise.resolve('{ "mcpServers": { "convertino": { "command": "convertino.exe", "args": ["mcp"] } } }');
    case "command_install": d.ai.command.installed = true; return Promise.resolve(d.ai);
    case "ai_jobs_clear": d.ai.jobs = []; return Promise.resolve(null);
    case "converter_download": {
      const t = d.tools.find((x) => x.id === args.id);
      let f = 0;
      const timer = setInterval(() => {
        f = Math.min(1, f + 0.1);
        progress[args.id] = { id: args.id, fraction: f, detail: `Downloading ${t.name} · ${Math.round(f * 100)}%` };
        renderConverters();
        if (f >= 1) {
          clearInterval(timer);
          delete progress[args.id];
          t.state = "ready"; t.sizeMb = t.downloadMb; t.version = "1.0";
          refreshTools();
          toolsNote(`${t.name} is ready.`);
        }
      }, 80);
      return Promise.resolve();
    }
    default: return Promise.resolve(null);
  }
}
