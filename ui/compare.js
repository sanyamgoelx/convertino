// Compare: each compressed copy next to its original (compare.rs, pairs.rs).
// Modelled on Drive Cleanup's "Copy and original": the facts of both files with
// the differences marked; side by side, wipe or flicker; zoom with panning; sound
// from either one; frame stepping; the 8 checkpoints of the copy check, scored.
// Videos play in step, streamed by Convertino; when this PC can't play the copy
// (some PCs have no H.265 support), still frames are shown instead and say so.
// "Looks good" marks the original for deletion; Drive Cleanup sees the same mark.
const tauri = window.__TAURI__;
const $ = (id) => document.getElementById(id);
const invoke = (cmd, args) => (tauri ? tauri.core.invoke(cmd, args) : demo(cmd, args));
const esc = (s) => String(s ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
if (/Mac/.test(navigator.platform)) document.documentElement.dataset.os = "mac";

const MEDIA = /Windows/.test(navigator.userAgent) ? "http://cvmedia.localhost/" : "cvmedia://localhost/";
const S = {
  id: Number(location.hash.slice(1)) || 0, pairs: [], dc: false, i: 0, tab: "all",
  mode: "side", z: 1, px: 0, py: 0, wipe: 50, sound: "off", showCopy: true, flick: null,
  facts: null, points: null, still: false, t: 0, page: 1, gen: 0, stillTimer: 0, sure: "",
};
const pair = () => S.pairs[S.i];

// ---------- words ----------
function size(b) {
  if (!b) return "0 B";
  const u = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  while (b >= 1024 && i < u.length - 1) { b /= 1024; i++; }
  return `${b >= 100 || i === 0 ? Math.round(b) : b.toFixed(b >= 10 ? 1 : 2)} ${u[i]}`;
}
const tc = (s) => { if (!isFinite(s) || s < 0) s = 0; const m = Math.floor(s / 60), x = s - m * 60; return `${m}:${x.toFixed(2).padStart(5, "0")}`; };
const kbps = (b) => (b ? (b >= 1e6 ? `${(b / 1e6).toFixed(1)} Mbit/s` : `${Math.round(b / 1e3)} kbit/s`) : "–");
const simCls = (s) => (s == null ? "" : s >= 0.85 ? "good" : s >= 0.7 ? "mid" : "bad");
const shortDate = (ms) => (ms ? new Date(ms).toLocaleDateString(undefined, { day: "numeric", month: "short" }) : "");
const isVideo = () => pair() && pair().kind === "video";

// ---------- loading ----------
async function boot() {
  try {
    const s = await invoke("compare_session", { id: S.id });
    S.pairs = s.pairs;
    S.dc = s.driveCleanup;
  } catch (e) {
    return fail(String(e));
  }
  if (!S.pairs.length) return fail("There's nothing to compare any more.");
  $("list").hidden = S.pairs.length < 2;
  load(0);
}

function fail(msg) {
  $("status").hidden = false;
  $("status").textContent = msg;
}

function load(i) {
  if (i < 0 || i >= S.pairs.length) return;
  release();
  S.i = i; S.gen++; S.facts = null; S.points = null; S.still = false; S.t = 0; S.page = 1; S.sure = "";
  const g = S.gen, p = pair();
  renderList(); renderHead(); renderActions();
  $("facts").innerHTML = `<tbody><tr><td class="by">Reading both files…</td></tr></tbody>`;
  $("still-note").hidden = true;
  $("play").disabled = false;
  $("play").textContent = "Play";
  $("trans").hidden = !isVideo(); $("tl").hidden = !isVideo(); $("g-sound").hidden = !isVideo();
  $("page-box").hidden = p.kind !== "pdf";
  $("page").value = "1";
  $("marks").innerHTML = "";
  keysLine();
  if (p.done) return fail(p.done);
  if (!p.originalThere || !p.copyThere) return fail(p.originalThere ? "The compressed copy isn't there any more, so there's nothing to compare." : "The original isn't there any more, so there's nothing to compare.");
  $("status").hidden = false;
  $("status").textContent = "Getting both files ready…";
  invoke("compare_facts", { id: S.id, index: i }).then((f) => { if (g === S.gen) { S.facts = f; renderFacts(); } }).catch((e) => { if (g === S.gen) $("facts").innerHTML = `<tbody><tr><td class="by">${esc(e)}</td></tr></tbody>`; });
  if (p.kind === "video") startVideo(g);
  else loadStills(g);
  if (p.kind === "video" || p.kind === "image") {
    if (p.kind === "video") $("marks").innerHTML = `<span class="wait">Scoring the 8 checkpoints…</span>`;
    invoke("compare_points", { id: S.id, index: i }).then((pts) => { if (g === S.gen) { S.points = pts; renderPoints(); } }).catch((e) => { if (g === S.gen) $("marks").innerHTML = `<span class="wait">Couldn't score the checkpoints: ${esc(e)}</span>`; });
  }
  applyMode();
}

/// Lets go of both files (Windows keeps a playing file locked).
function release() {
  for (const v of document.querySelectorAll(".view video")) { v.pause(); v.removeAttribute("src"); v.load(); }
  $("view-o").innerHTML = ""; $("view-c").innerHTML = "";
  clearInterval(S.flick); S.flick = null; $("auto").checked = false;
}

// ---------- video ----------
const vO = () => $("view-o").querySelector("video");
const vC = () => $("view-c").querySelector("video");
const dur = () => { const c = vC(); return (c && isFinite(c.duration) && c.duration) || (S.facts && (S.facts.copy.seconds || S.facts.original.seconds)) || 0; };
const fps = () => (S.facts && (S.facts.copy.fps || S.facts.original.fps)) || 30;

function startVideo(g) {
  const make = (side) => {
    const v = document.createElement("video");
    v.muted = true; v.preload = "auto"; v.playsInline = true;
    v.src = `${MEDIA}${S.id}/${S.i}/${side}`;
    return v;
  };
  const o = make("o"), c = make("c");
  $("view-o").appendChild(o); $("view-c").appendChild(c);
  let ready = 0;
  const giveUp = setTimeout(() => { if (g === S.gen && ready < 2) toStills(g, "The videos didn't start playing here"); }, 10000);
  const onReady = (v) => () => {
    if (g !== S.gen) return;
    if (!v.videoWidth) return toStills(g, "This PC can't show this video's picture in Convertino's window");
    if (++ready === 2) { clearTimeout(giveUp); $("status").hidden = true; setT(0); }
  };
  for (const v of [o, c]) {
    v.addEventListener("loadeddata", onReady(v), { once: true });
    v.addEventListener("error", () => { if (g === S.gen) { clearTimeout(giveUp); toStills(g, "This PC can't play this video in Convertino's window"); } }, { once: true });
  }
  c.addEventListener("play", () => { $("play").textContent = "Pause"; });
  c.addEventListener("pause", () => { $("play").textContent = "Play"; });
  c.addEventListener("ended", () => { const b = vO(); if (b) b.pause(); });
  c.addEventListener("seeked", () => { const b = vO(); if (b && Math.abs(b.currentTime - (c.currentTime + pair().offset)) > 0.02) b.currentTime = c.currentTime + pair().offset; });
  applySound();
  (function loop() {
    if (g !== S.gen) return;
    const a = vC(), b = vO();
    if (a && b && !S.still) {
      if (!a.paused && Math.abs(b.currentTime - (a.currentTime + pair().offset)) > 0.12) b.currentTime = a.currentTime + pair().offset;
      S.t = a.currentTime;
      showTime();
    }
    requestAnimationFrame(loop);
  })();
}

/// Still frames instead of playback (the copy's codec can't play in this window).
function toStills(g, why) {
  if (g !== S.gen || S.still) return;
  const codec = S.facts && S.facts.copy.video ? ` (${S.facts.copy.video.toUpperCase()})` : "";
  for (const v of document.querySelectorAll(".view video")) { v.pause(); v.removeAttribute("src"); v.load(); v.remove(); }
  S.still = true;
  note(`${why}${codec}, so you see still frames. Seeking, frame steps and the checkpoints work; Play doesn't.`, "warn");
  $("play").disabled = true;
  loadStills(g);
}

function setT(t) {
  const d = dur();
  S.t = Math.max(0, d ? Math.min(d - 0.001, t) : t);
  if (S.still) { showTime(); clearTimeout(S.stillTimer); S.stillTimer = setTimeout(() => loadStills(S.gen), 120); return; }
  const a = vC(), b = vO();
  if (a) a.currentTime = S.t;
  if (b) b.currentTime = S.t + pair().offset;
  showTime();
}

function playing() { const a = vC(); return !!a && !a.paused; }
function play(on) {
  const a = vC(), b = vO();
  if (!a || !b || S.still) return;
  if (on) { b.currentTime = a.currentTime + pair().offset; a.play().catch(() => {}); b.play().catch(() => {}); }
  else { a.pause(); b.pause(); b.currentTime = a.currentTime + pair().offset; }
}
function step(n) { play(false); setT(S.t + n / fps()); }

function showTime() {
  const d = dur();
  $("time").textContent = `${tc(S.t)} / ${tc(d)}`;
  if (d && document.activeElement !== $("seek")) $("seek").value = String(Math.round((S.t / d) * 1000));
}

// ---------- stills (pictures, PDF pages, videos this PC can't play) ----------
async function loadStills(g) {
  const p = pair(), at = p.kind === "pdf" ? S.page : p.kind === "video" ? S.t : 0;
  try {
    const [o, c] = await Promise.all([invoke("compare_still", { id: S.id, index: S.i, side: "o", at }), invoke("compare_still", { id: S.id, index: S.i, side: "c", at })]);
    if (g !== S.gen) return;
    for (const [view, url, alt] of [["view-o", o, "Original"], ["view-c", c, "Compressed copy"]]) {
      let img = $(view).querySelector("img");
      if (!img) { img = document.createElement("img"); img.alt = alt; $(view).appendChild(img); }
      img.src = url;
    }
    $("status").hidden = true;
    applyZoom();
  } catch (e) {
    if (g === S.gen) fail(String(e));
  }
}

// ---------- header, facts, list ----------
function renderHead() {
  const p = pair();
  $("name").textContent = p.name;
  $("name").title = p.original;
  $("by").textContent = p.by.replace("{date}", p.madeAt ? new Date(p.madeAt).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" }) : "");
  document.title = `Compare – ${p.name}`;
  $("badge").hidden = !p.marked;
  $("badge").textContent = "✓ Checked: marked for deletion";
  $("nav").hidden = S.pairs.length < 2;
  $("count").textContent = `${S.i + 1} of ${S.pairs.length}`;
  $("prev").disabled = S.i === 0;
  $("next").disabled = S.i >= S.pairs.length - 1;
  const save = p.originalBytes - p.copyBytes;
  $("save").innerHTML = p.originalBytes && p.copyBytes
    ? save > 0 ? `The copy saves <b>${size(save)}</b> (${Math.round((save / p.originalBytes) * 100)}% smaller · ${size(p.originalBytes)} → ${size(p.copyBytes)})` : `The copy isn't smaller (${size(p.copyBytes)}, the original ${size(p.originalBytes)})`
    : "";
  $("flag").hidden = !p.flag;
  $("flag").textContent = p.flag ? `This copy didn't pass the automatic check: ${p.flag}. Both files are kept; look closely at the orange and red checkpoints before you mark the original for deletion.` : "";
}

function renderFacts() {
  const f = S.facts, p = pair();
  if (!f) return;
  const o = f.original, c = f.copy;
  const pic = (x) => (x.width ? `${x.width}×${x.height}` : "–");
  const cols = p.kind === "video"
    ? [["Size", (x) => size(x.bytes)], ["Picture", pic, true], ["Frame rate", (x) => (x.fps ? `${+x.fps.toFixed(2)} fps` : "–"), true], ["Bitrate", (x) => kbps(x.bitrate)],
       ["Video codec", (x) => esc(x.video || "–"), true], ["Sound", (x) => (x.audio ? `${esc(x.audio)}${x.audioKbps ? `, ${x.audioKbps} kbit/s` : ""}` : "none"), true],
       ["Length", (x) => (x.seconds ? tc(x.seconds) : "–"), () => Math.abs((o.seconds || 0) - (c.seconds || 0)) > Math.max(1, (o.seconds || 0) * 0.002) && !p.offset]]
    : p.kind === "image"
      ? [["Size", (x) => size(x.bytes)], ["Picture", pic, true], ["Format", (x) => esc(x.format), true]]
      : [["Size", (x) => size(x.bytes)], ["Format", (x) => esc(x.format), true]];
  $("facts").innerHTML = `<thead><tr><th></th>${cols.map((k) => `<th>${k[0]}</th>`).join("")}</tr></thead><tbody>` +
    `<tr><th>Original</th>${cols.map((k) => `<td>${k[1](o)}</td>`).join("")}</tr>` +
    `<tr><th>Compressed copy</th>${cols.map((k) => { const d = typeof k[2] === "function" ? k[2]() : k[2] && k[1](o) !== k[1](c); return `<td class="${d ? "diff" : ""}">${k[1](c)}</td>`; }).join("")}</tr></tbody>` +
    (o.error || c.error ? `<tbody><tr><td colspan="${cols.length + 1}" class="by">${esc(o.error || c.error)}: some details are missing.</td></tr></tbody>` : "");
  const shape = c.width ? c : o;
  if (shape.width && shape.height) { $("stage").style.setProperty("--arw", shape.width); $("stage").style.setProperty("--arh", shape.height); }
  showTime();
}

function renderPoints() {
  const p = pair(), pts = S.points || [];
  if (p.kind === "image") {
    const s = pts[0] && pts[0].s;
    if (s != null) $("save").insertAdjacentHTML("beforeend", ` · <span class="${simCls(s)}" style="color:var(--${simCls(s) || "ink-2"})">looks ${Math.round(s * 100)}% alike</span>`);
    return;
  }
  const d = dur();
  if (!d) return;
  $("marks").innerHTML = pts.map((x, k) => `<button type="button" class="mark ${simCls(x.s)}" style="left:${(x.t / d) * 100}%" data-pt="${k}" title="Checkpoint ${k + 1} at ${tc(x.t)}${x.s != null ? `: ${Math.round(x.s * 100)}% alike` : ": couldn't be compared"}">${x.s != null ? `${Math.round(x.s * 100)}%` : "?"}</button>`).join("");
}

const counts = () => ({
  check: S.pairs.filter((p) => !p.marked && !p.flag && !p.done).length,
  flag: S.pairs.filter((p) => p.flag && !p.marked && !p.done).length,
  done: S.pairs.filter((p) => p.marked).length,
  all: S.pairs.length,
});
const inTab = (p) => S.tab === "all" || (S.tab === "check" && !p.marked && !p.flag && !p.done) || (S.tab === "flag" && p.flag && !p.marked && !p.done) || (S.tab === "done" && p.marked);

function renderList() {
  if (S.pairs.length < 2) return;
  const n = counts();
  $("tabs").innerHTML = [["check", "To check"], ["flag", "Didn't pass"], ["done", "Checked"], ["all", "All"]]
    .map(([k, l]) => `<button type="button" data-tab="${k}" aria-pressed="${S.tab === k}">${l} <b>${n[k]}</b></button>`).join("");
  const rows = S.pairs.map((p, i) => [p, i]).filter(([p]) => inTab(p));
  $("rows").innerHTML = rows.length
    ? rows.map(([p, i]) => `<button type="button" class="row" data-p="${i}" aria-current="${i === S.i}" title="${esc(p.original)}"><span class="dot ${p.marked ? "d-ok" : p.flag ? "d-flag" : ""}"></span><span class="t"><b>${esc(p.name)}</b><span>${size(p.originalBytes)} → ${size(p.copyBytes)}${p.madeAt ? ` · ${shortDate(p.madeAt)}` : ""}</span></span></button>`).join("")
    : `<p class="empty">Nothing here.</p>`;
  const cur = $("rows").querySelector('[aria-current="true"]');
  if (cur) cur.scrollIntoView({ block: "nearest" });
}

// ---------- what you can do with the pair ----------
function renderActions() {
  const p = pair();
  const mark = p.done ? "" : `<button type="button" class="${p.marked ? "" : "primary"}" id="mark">${p.marked ? "Marked for deletion (click to undo)" : "✓ Looks good: mark the original for deletion"}</button>`;
  let more = "", why = "";
  if (p.done) {
    why = "";
  } else if (p.held && p.maker !== "convertino") {
    more = S.dc ? `<button type="button" id="dc">Put back or delete in Drive Cleanup ↗</button>` : "";
    why = S.dc
      ? (p.marked ? "Drive Cleanup lists it under Compressed Videos › Checked, ready to delete. Until then the original waits in the Holding Area." : "Drive Cleanup made this batch, so it puts the original back or deletes it. Until then the original waits in the Holding Area.")
      : "The original waits in the Holding Area until it's emptied. Drive Cleanup deletes marked originals; it isn't installed on this PC.";
  } else if (p.canPutBack) {
    more = `<button type="button" id="putback">${S.sure === "putback" ? "Click again to put it back" : "Put the original back"}</button>`;
    why = p.marked
      ? (S.dc ? "Drive Cleanup lists it under Compressed Videos › Checked, ready to delete." : "The original waits in the Holding Area until you empty it.")
      : "The original waits in the Holding Area until you empty it (in Drive Cleanup, or by hand).";
  } else {
    why = p.marked
      ? (S.dc ? "Drive Cleanup lists it under Compressed Videos › Checked, ready to delete." : "Marked. Delete the original yourself when you're ready (Show original).")
      : "Both files are in the same folder.";
  }
  $("act").innerHTML = `${mark}${more}<button type="button" id="show-o">Show original</button><button type="button" id="show-c">Show copy</button><span class="why">${esc(why)}</span>`;
}

async function toggleMark() {
  const p = pair(), i = S.i;
  try {
    p.marked = await invoke("compare_mark", { id: S.id, index: i, on: !p.marked });
  } catch (e) {
    return note(String(e), "warn");
  }
  renderHead(); renderList(); renderActions();
  // Marked: on to the next one still to check.
  if (p.marked && S.pairs.length > 1) {
    const next = S.pairs.findIndex((x, k) => k > i && !x.marked && !x.done);
    if (next >= 0) load(next);
  }
}

async function putBack() {
  if (S.sure !== "putback") { S.sure = "putback"; renderActions(); setTimeout(() => { if (S.sure === "putback") { S.sure = ""; renderActions(); } }, 4000); return; }
  S.sure = "";
  play(false);
  release(); // let go of the files before they move
  const p = pair();
  try {
    const said = await invoke("compare_put_back", { id: S.id, index: S.i });
    p.done = said; p.marked = false;
    fail(said);
    renderHead(); renderList(); renderActions();
  } catch (e) {
    note(String(e), "warn");
    load(S.i);
  }
}

function note(msg, kind) {
  $("still-note").hidden = false;
  $("still-note").className = `note ${kind}`;
  $("still-note").textContent = msg;
}

// ---------- view: side by side, wipe, flicker; zoom; sound ----------
function seg(id, opts, val, label) {
  $(id).innerHTML = (label ? `<span class="sl">${label}</span>` : "") + opts.map(([k, l]) => `<button type="button" data-${id}="${k}" aria-pressed="${String(k) === String(val)}">${l}</button>`).join("");
}
function applyMode() {
  const st = $("stage");
  st.className = `stage m-${S.mode}${S.mode === "flick" ? (S.showCopy ? " show-c" : " show-o") : ""}${S.z > 1 ? " zoomed" : ""}`;
  $("flip").hidden = S.mode !== "flick"; $("auto-box").hidden = S.mode !== "flick";
  $("flip").textContent = `Showing: ${S.showCopy ? "copy" : "original"} · flip`;
  if (S.mode !== "flick") { clearInterval(S.flick); S.flick = null; $("auto").checked = false; }
  seg("g-mode", [["side", "Side by side"], ["wipe", "Wipe"], ["flick", "Flicker"]], S.mode);
  seg("g-zoom", [[1, "1×"], [2, "2×"], [4, "4×"]], S.z);
  seg("g-sound", [["off", "Off"], ["o", "Original"], ["c", "Copy"]], S.sound, "Sound");
  applyWipe(); applyZoom();
}
function applyWipe() {
  const pc = document.querySelector(".pc"), on = S.mode === "wipe";
  pc.style.clipPath = on ? `inset(0 0 0 ${S.wipe}%)` : "";
  if (on) { const r = $("stage").getBoundingClientRect(), q = pc.getBoundingClientRect(); $("handle").style.left = `${q.left - r.left + (q.width * S.wipe) / 100}px`; }
}
function clampPan() {
  const v = $("view-c"), mx = ((S.z - 1) * v.clientWidth) / 2, my = ((S.z - 1) * v.clientHeight) / 2;
  S.px = Math.max(-mx, Math.min(mx, S.px)); S.py = Math.max(-my, Math.min(my, S.py));
}
function applyZoom() {
  clampPan();
  for (const el of document.querySelectorAll(".view video, .view img")) el.style.transform = S.z > 1 ? `translate(${S.px}px, ${S.py}px) scale(${S.z})` : "";
}
function setZoom(z) { if (z === 1) { S.px = 0; S.py = 0; } S.z = z; applyMode(); }
function applySound() {
  const a = vC(), b = vO();
  if (b) b.muted = S.sound !== "o";
  if (a) a.muted = S.sound !== "c";
}
function flip() { S.showCopy = !S.showCopy; applyMode(); }

// ---------- events ----------
document.addEventListener("click", (e) => {
  const b = e.target.closest("button");
  if (!b) return;
  const d = b.dataset;
  if (d.tab) { S.tab = d.tab; return renderList(); }
  if (d.p !== undefined) return load(Number(d.p));
  if (d["gMode"]) { S.mode = d["gMode"]; return applyMode(); }
  if (d["gZoom"]) return setZoom(Number(d["gZoom"]));
  if (d["gSound"]) { S.sound = d["gSound"]; applyMode(); return applySound(); }
  if (d.pt !== undefined && S.points) { play(false); return setT(S.points[Number(d.pt)].t); }
  switch (b.id) {
    case "prev": return load(S.i - 1);
    case "next": return load(S.i + 1);
    case "play": return play(!playing());
    case "back1": return step(-1);
    case "fwd1": return step(1);
    case "flip": return flip();
    case "mark": return toggleMark();
    case "putback": return putBack();
    case "dc": return invoke("compare_drive_cleanup", { id: S.id, index: S.i }).catch((err) => note(String(err), "warn"));
    case "show-o": return invoke("compare_show", { id: S.id, index: S.i, side: "o" }).catch((err) => note(String(err), "warn"));
    case "show-c": return invoke("compare_show", { id: S.id, index: S.i, side: "c" }).catch((err) => note(String(err), "warn"));
  }
});
$("auto").addEventListener("change", (e) => {
  clearInterval(S.flick);
  S.flick = e.target.checked ? setInterval(flip, 600) : null;
});
$("seek").addEventListener("input", (e) => { play(false); setT((Number(e.target.value) / 1000) * dur()); });
$("page").addEventListener("change", (e) => { S.page = Math.max(1, Number(e.target.value) || 1); loadStills(S.gen); });
window.addEventListener("resize", () => { applyWipe(); applyZoom(); });

// Dragging: the wipe line, or looking around when zoomed in.
const stage = $("stage");
stage.addEventListener("pointerdown", (e) => {
  const onHandle = e.target.closest("#handle");
  const wipeDrag = S.mode === "wipe" && (onHandle || S.z === 1);
  if (!wipeDrag && S.z === 1) return;
  e.preventDefault();
  stage.setPointerCapture(e.pointerId);
  const r = document.querySelector(".pc").getBoundingClientRect(), sx = e.clientX, sy = e.clientY, px0 = S.px, py0 = S.py;
  const move = (ev) => {
    if (wipeDrag) { S.wipe = Math.max(0, Math.min(100, ((ev.clientX - r.left) / r.width) * 100)); applyWipe(); }
    else { S.px = px0 + ev.clientX - sx; S.py = py0 + ev.clientY - sy; applyZoom(); }
  };
  if (wipeDrag) move(e);
  stage.onpointermove = move;
  stage.onpointerup = stage.onpointercancel = () => { stage.onpointermove = null; };
});

function keysLine() {
  const pairs = S.pairs.length > 1 ? " · <kbd>J</kbd><kbd>K</kbd> next / previous pair" : "";
  $("keys").innerHTML = isVideo()
    ? `<kbd>Space</kbd> play · <kbd>←</kbd><kbd>→</kbd> one frame · <kbd>Shift</kbd>+arrows 5 s · <kbd>N</kbd> next checkpoint · <kbd>F</kbd> flip · <kbd>Z</kbd> zoom · drag the picture to look around when zoomed${pairs}`
    : `<kbd>F</kbd> flip · <kbd>Z</kbd> zoom · drag the picture to look around when zoomed${pairs}`;
}

document.addEventListener("keydown", (e) => {
  if (/input|select|textarea/i.test(e.target.tagName) || e.ctrlKey || e.metaKey || e.altKey) return;
  const k = e.key;
  if (k === "Escape") return invoke("compare_close").catch(() => window.close());
  if (k === "j" || k === "J") return load(S.i + 1);
  if (k === "k" || k === "K") return load(S.i - 1);
  if (k === "f" || k === "F") { if (S.mode !== "flick") S.mode = "flick"; return flip(); }
  if (k === "z" || k === "Z") return setZoom(S.z === 1 ? 2 : S.z === 2 ? 4 : 1);
  if (!isVideo()) return;
  if (k === " ") { e.preventDefault(); return play(!playing()); }
  if (k === "ArrowRight" || k === "ArrowLeft") {
    e.preventDefault();
    const dir = k === "ArrowRight" ? 1 : -1;
    if (e.shiftKey) { play(false); return setT(S.t + 5 * dir); }
    return step(dir);
  }
  if ((k === "n" || k === "N") && S.points && S.points.length) {
    const next = S.points.find((x) => x.t > S.t + 0.05) || S.points[0];
    play(false);
    return setT(next.t);
  }
});

// ---------- opened in a normal browser: a sample, so the design can be checked ----------
function demo(cmd, args) {
  const svg = (blur, hue) => "data:image/svg+xml," + encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" width="1280" height="720"><defs><filter id="b"><feGaussianBlur stdDeviation="${blur}"/></filter></defs><g filter="url(#b)"><rect width="1280" height="720" fill="hsl(${hue},45%,62%)"/><circle cx="980" cy="170" r="70" fill="#f2c46d"/><polygon points="0,470 200,250 400,420 600,210 830,430 1020,280 1280,460 1280,720 0,720" fill="#8aa6bf"/><rect y="580" width="1280" height="140" fill="#6f93ad"/></g></svg>`);
  const P = [
    { name: "Valorant 2026.06.10 - 21.14.33.02.DVR.mp4", kind: "image", maker: "drive-cleanup", held: true, by: "Compressed by Drive Cleanup on 11 Oct 2026 · the original waits in the Holding Area", madeAt: 1791700000000, originalBytes: 2190000000, copyBytes: 103000000, originalThere: true, copyThere: true, flag: null, marked: false, offset: 0, canPutBack: false, original: "C:\\Users\\crofty\\_Quarantine\\shrink-2026-10-11-09-58-57\\Videos\\a.mp4", copy: "C:\\Users\\crofty\\Videos\\a.mp4" },
    { name: "Desktop 2026.06.19 - 01.12.40.DVR.mp4", kind: "image", maker: "drive-cleanup", held: true, by: "Compressed by Drive Cleanup on 12 Oct 2026 · the original waits in the Holding Area", madeAt: 1791790000000, originalBytes: 407000000, copyBytes: 43000000, originalThere: true, copyThere: true, flag: "2 of 8 frames didn't match the original", marked: false, offset: 0, canPutBack: false, original: "b", copy: "b" },
    { name: "hero-loop.mp4", kind: "image", maker: "beside", held: false, by: "The original sits right beside its copy", madeAt: 1791600000000, originalBytes: 432000000, copyBytes: 39800000, originalThere: true, copyThere: true, flag: null, marked: true, offset: 0, canPutBack: false, original: "c", copy: "c" },
  ];
  const r = {
    compare_session: () => ({ id: 1, pairs: P, driveCleanup: true }),
    compare_facts: () => ({ original: { bytes: P[args.index].originalBytes, width: 2560, height: 1440, format: "PNG" }, copy: { bytes: P[args.index].copyBytes, width: 2560, height: 1440, format: "WEBP" } }),
    compare_points: () => [{ t: 0, s: args.index === 1 ? 0.64 : 0.93 }],
    compare_still: () => svg(args.side === "c" ? 1.4 : 0, 205 + args.index * 40),
    compare_mark: () => args.on,
  }[cmd];
  return r ? Promise.resolve(r()) : Promise.reject(new Error("Not in the browser sample"));
}

boot();
