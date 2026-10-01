// Convertino spike window: shows every selection the hotkey reads.
const isMac = /Mac/.test(navigator.platform || navigator.userAgent);
if (isMac) document.documentElement.classList.add("mac");

// "ctrl+alt+shift+c" -> Ctrl Alt Shift C (Windows) or ⌃ ⌥ ⇧ C (Mac)
const KEY_NAMES = isMac
  ? { ctrl: "⌃", alt: "⌥", shift: "⇧", cmd: "⌘", super: "⌘" }
  : { ctrl: "Ctrl", alt: "Alt", shift: "Shift", super: "Win" };
const keyLabel = (combo) => combo.split("+").map((k) => KEY_NAMES[k] || k.toUpperCase());
const kbd = (combo) => keyLabel(combo).map((k) => `<kbd>${k}</kbd>`).join(" ");

function showHotkey(status) {
  const el = document.getElementById("keys");
  const note = document.getElementById("hotkey-note");
  if (!status || !status.active) {
    el.innerHTML = "the hotkey";
    note.textContent = "Every shortcut Convertino tried is taken by other apps. A shortcut picker is coming in Settings.";
    note.hidden = false;
    return;
  }
  el.innerHTML = kbd(status.active);
  if (status.taken.length) {
    note.innerHTML = `${status.taken.map((c) => keyLabel(c).join("+")).join(", ")} is already used by another app on this computer, so Convertino is using ${keyLabel(status.active).join("+")} instead.`;
    note.hidden = false;
  }
}
showHotkey({ active: "ctrl+alt+c", taken: [] });
document.querySelectorAll("[data-fm]").forEach((el) => (el.textContent = isMac ? "Finder" : "File Explorer"));
document.querySelectorAll("[data-alt]").forEach((el) => (el.textContent = isMac ? "⌥ Option" : "Alt"));
document.querySelectorAll("[data-tray]").forEach((el) => (el.textContent = isMac ? "menu bar" : "system tray"));

const list = document.getElementById("events");
const empty = document.getElementById("empty");

const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

function addEvent(e) {
  const time = new Date(Number(e.atUnixMs)).toLocaleTimeString();
  const li = document.createElement("li");
  li.className = "event";
  if (e.error) {
    li.innerHTML = `<div class="meta"><b>${time}</b><span>${e.elapsedMs} ms</span></div><div class="err">${esc(e.error)}</div>`;
  } else {
    const n = e.paths.length;
    li.innerHTML =
      `<div class="meta"><b>${time}</b><span>${esc(e.source)}</span><span>${n} item${n === 1 ? "" : "s"}</span><span>${e.elapsedMs} ms</span></div>` +
      (n ? `<ul>${e.paths.map((p) => `<li>${esc(p)}</li>`).join("")}</ul>` : `<div class="none">Nothing selected.</div>`);
  }
  list.prepend(li);
  while (list.children.length > 20) list.lastElementChild.remove();
  list.hidden = false;
  empty.hidden = true;
}

function addPick(p) {
  const time = new Date(Number(p.atUnixMs)).toLocaleTimeString();
  const n = p.files.length;
  const li = document.createElement("li");
  li.className = "event pick";
  li.innerHTML =
    `<div class="meta"><b>${time}</b><span>Picked <b>${esc(p.label)}</b></span><span>${n} file${n === 1 ? "" : "s"}</span><span>${esc(p.targetId)}</span></div>` +
    `<div class="none">Converting; progress shows in the corner of the screen.</div>`;
  list.prepend(li);
  list.hidden = false;
  empty.hidden = true;
}

if (window.__TAURI__) {
  window.__TAURI__.event.listen("selection", (ev) => addEvent(ev.payload));
  window.__TAURI__.event.listen("picked", (ev) => addPick(ev.payload));
  window.__TAURI__.event.listen("hotkey-status", (ev) => showHotkey(ev.payload));
  window.__TAURI__.core.invoke("hotkey_status").then(showHotkey).catch(() => {});
} else {
  empty.querySelector("strong").textContent = "Open this through the Convertino app, not a browser.";
}
