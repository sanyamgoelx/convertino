// Convertino HUD: a progress card per job that turns into a done card.
// Jobs started from the wheel show in its progress ring first; their events are
// kept here and a card appears only if the ring hands the job over ("job-handoff").
const tauri = window.__TAURI__;
const stack = document.getElementById("stack");
const isMac = /Mac/.test(navigator.platform || navigator.userAgent);
if (isMac) document.documentElement.dataset.os = "mac";

const LOGO =
  '<svg class="logo" viewBox="0 0 40 40" aria-hidden="true"><circle cx="20" cy="20" r="18" fill="none" stroke="currentColor" stroke-opacity=".5"/>' +
  '<path d="M20 2A18 18 0 0 1 32.7 7.3L25.6 14.4A8 8 0 0 0 20 12Z" fill="var(--accent)"/><circle cx="20" cy="20" r="6" fill="none" stroke="currentColor" stroke-opacity=".5"/></svg>';
const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
const cards = new Map(); // job id -> { el, timer }

// Keep the window exactly as tall as its cards; 0 hides it.
function fit() {
  const h = stack.children.length ? Math.ceil(stack.getBoundingClientRect().height) : 0;
  if (tauri) tauri.core.invoke("hud_resize", { height: h }).catch(() => {});
}

function card(id) {
  let c = cards.get(id);
  if (!c) {
    const el = document.createElement("div");
    el.className = "card";
    // Mac notifications stack downward from the top; Windows toasts upward from the bottom.
    if (isMac) stack.append(el); else stack.prepend(el);
    c = { el, timer: null };
    cards.set(id, c);
    el.addEventListener("mouseenter", () => clearTimeout(c.timer));
    el.addEventListener("mouseleave", () => { if (c.done) schedule(id, 4000); });
  }
  return c;
}

function remove(id) {
  const c = cards.get(id);
  if (!c) return;
  cards.delete(id);
  clearTimeout(c.timer);
  c.el.classList.add("out");
  setTimeout(() => { c.el.remove(); fit(); }, 200);
}

function schedule(id, ms) {
  const c = cards.get(id);
  if (!c) return;
  clearTimeout(c.timer);
  c.timer = setTimeout(() => remove(id), ms);
}

function head() {
  return `<div class="head">${LOGO}<span>Convertino</span><button class="x" type="button" title="Dismiss" aria-label="Dismiss">✕</button></div>`;
}

function started(job) {
  const c = card(job.id);
  c.el.innerHTML =
    head() +
    `<div class="title" title="${esc(job.title)}">${esc(job.title)}</div>` +
    `<div class="bar indeterminate"><i></i></div>` +
    `<div class="row"><div class="detail">Starting…</div><button class="cancel" type="button">Cancel</button></div>`;
  c.el.querySelector(".x").onclick = () => remove(job.id);
  const cancel = c.el.querySelector(".cancel");
  cancel.onclick = () => {
    cancel.disabled = true;
    cancel.textContent = "Cancelling…";
    if (tauri) tauri.core.invoke("job_cancel", { id: job.id }).catch(() => {});
  };
  fit();
}

function progress(p) {
  const c = cards.get(p.id);
  if (!c || c.done) return;
  const bar = c.el.querySelector(".bar");
  if (!bar) return;
  if (p.fraction > 0) {
    bar.classList.remove("indeterminate");
    bar.querySelector("i").style.width = `${(p.fraction * 100).toFixed(1)}%`;
  }
  c.el.querySelector(".detail").textContent = p.detail;
}

function done(d) {
  const c = card(d.id);
  c.done = true;
  c.el.classList.toggle("bad", !d.ok);
  const fixLabel = { "windows-security": "Open Windows Security", "mac-privacy": "Open Privacy settings" }[d.fix];
  const buttons = [];
  if (d.ok) buttons.push(`<button type="button" data-a="open">${isMac ? "Show in Finder" : "Open folder"}</button>`, `<button type="button" data-a="undo">Undo</button>`);
  if (fixLabel) buttons.push(`<button type="button" data-a="fix">${fixLabel}</button>`);
  if (d.report) buttons.push(`<button type="button" data-a="report">Send report</button>`);
  c.el.innerHTML =
    head() +
    `<div class="title" title="${esc(d.title)}">${esc(d.title)}</div>` +
    `<div class="body">${esc(d.body)}</div>` +
    (buttons.length ? `<div class="buttons">${buttons.join("")}</div>` : "");
  c.el.querySelector(".x").onclick = () => remove(d.id);
  const on = (a, f) => { const b = c.el.querySelector(`[data-a="${a}"]`); if (b) b.onclick = () => f(b); };
  on("open", () => {
    tauri.core.invoke("job_reveal", { id: d.id }).catch(() => {});
    remove(d.id);
  });
  on("undo", async (undo) => {
    undo.disabled = true;
    try {
      await tauri.core.invoke("job_undo", { id: d.id });
      c.el.querySelector(".title").textContent = isMac ? "Moved to the Trash" : "Moved to the Recycle Bin";
      c.el.querySelector(".body").textContent = "The original files are untouched.";
      c.el.querySelector(".buttons").remove();
      fit();
      schedule(d.id, 2000);
    } catch (e) {
      c.el.querySelector(".body").textContent = String(e);
      undo.disabled = false;
    }
  });
  on("fix", () => tauri.core.invoke("open_fix", { kind: d.fix }).catch(() => {}));
  on("report", async (b) => {
    clearTimeout(c.timer);
    b.disabled = true;
    b.textContent = "Sending…";
    try {
      await tauri.core.invoke("job_report", { id: d.id, note: null });
      b.textContent = "Report sent, thanks";
      schedule(d.id, 3000);
    } catch (e) {
      b.textContent = "Send report";
      b.disabled = false;
      c.el.querySelector(".body").textContent = String(e);
      fit();
    }
  });
  fit();
  // Cards that need reading or a decision stay longer.
  schedule(d.id, !d.ok || d.attention ? 20000 : 8000);
}

// A message that isn't about a job ("Nothing to convert").
let noticeN = 0;
function notice(n) {
  const id = `notice-${++noticeN}`;
  const c = card(id);
  c.done = true;
  c.el.innerHTML = head() + `<div class="title">${esc(n.title)}</div><div class="body">${esc(n.body)}</div>`;
  c.el.querySelector(".x").onclick = () => remove(id);
  fit();
  schedule(id, 6000);
}

// id -> { started, progress, done } for jobs the ring is showing.
const inRing = new Map();
const handedOff = new Set();

function onStarted(job) {
  if (job.ring && !handedOff.has(job.id)) {
    inRing.set(job.id, { started: job });
    return;
  }
  started(job);
}
function onProgress(p) {
  const r = inRing.get(p.id);
  if (r) r.progress = p;
  else progress(p);
}
function onDone(d) {
  const r = inRing.get(d.id);
  if (!r) return done(d);
  r.done = d;
  // Finished in the ring: forget it once a hand-over can no longer come.
  setTimeout(() => inRing.delete(d.id), 30000);
}
function onHandoff(id) {
  handedOff.add(id);
  const r = inRing.get(id);
  inRing.delete(id);
  if (!r) return; // its started event hasn't arrived yet; it will show normally
  started(r.started);
  if (r.progress) progress(r.progress);
  if (r.done) done(r.done);
}

if (tauri) {
  tauri.event.listen("job-started", (e) => onStarted(e.payload));
  tauri.event.listen("job-progress", (e) => onProgress(e.payload));
  tauri.event.listen("job-done", (e) => onDone(e.payload));
  tauri.event.listen("job-handoff", (e) => onHandoff(e.payload));
  tauri.event.listen("notice", (e) => notice(e.payload));
} else {
  // Opened in a browser: show sample cards for design checks.
  document.documentElement.style.background = "#6b7a8f";
  started({ id: 1, title: "voice-memo.wav → MP3" });
  progress({ id: 1, fraction: 0.42, detail: "42%" });
  done({ id: 2, ok: true, title: "Converted to JPG", body: "holiday-goa.jpg · 2.8 MB" });
  done({ id: 3, ok: false, title: "Couldn't convert to MP4", body: "Clip.mov: This file seems to be damaged, or isn't really the type its name says.", report: true });
  done({ id: 4, ok: true, attention: true, fix: "windows-security", title: "Extracted audio as MP3", body: "Clip.mp3 · 3.1 MB\nSaved in Downloads: Windows Security's ransomware protection doesn't let new apps save in Videos. To save next to your files, allow Convertino there." });
  notice({ title: "Nothing to convert", body: "Select files in File Explorer or on the desktop first, then press the shortcut." });
}
