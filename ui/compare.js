// Compare: the original and its compressed copy under a slider.
const tauri = window.__TAURI__;
const $ = (id) => document.getElementById(id);
const id = Number(location.hash.slice(1)) || 0;
let index = 0;
let count = 1;
let pos = 50;

if (/Mac/.test(navigator.platform)) document.documentElement.dataset.os = "mac";

function place(v) {
  pos = Math.max(0, Math.min(100, v));
  $("leftwrap").style.clipPath = `inset(0 ${100 - pos}% 0 0)`;
  $("handle").style.left = `${pos}%`;
  $("slider").value = String(Math.round(pos));
}

async function load(i) {
  index = i;
  $("frame").classList.remove("ready");
  $("status").hidden = false;
  $("status").textContent = "Getting the pictures ready…";
  $("stage").setAttribute("aria-busy", "true");
  try {
    const p = tauri ? await tauri.core.invoke("compare_data", { id, index: i }) : demo();
    count = p.count;
    $("title").textContent = p.title;
    $("what").textContent = p.what;
    document.title = `Compare – ${p.title}`;
    $("leftLabel").textContent = p.leftLabel;
    $("rightLabel").textContent = p.rightLabel;
    $("nav").hidden = count < 2;
    $("count").textContent = `${i + 1} of ${count}`;
    $("prev").disabled = i === 0;
    $("next").disabled = i >= count - 1;
    await Promise.all([set($("left"), p.left), set($("right"), p.right)]);
    $("status").hidden = true;
    $("frame").classList.add("ready");
    place(pos);
  } catch (e) {
    $("status").textContent = String(e);
  }
  $("stage").setAttribute("aria-busy", "false");
}

function set(img, src) {
  return new Promise((resolve) => {
    img.onload = resolve;
    img.onerror = resolve;
    img.src = src;
  });
}

// Drag anywhere on the picture.
const frame = $("frame");
let dragging = false;
const fromEvent = (e) => {
  const r = frame.getBoundingClientRect();
  place(((e.clientX - r.left) / r.width) * 100);
};
frame.addEventListener("pointerdown", (e) => { dragging = true; frame.setPointerCapture(e.pointerId); fromEvent(e); });
frame.addEventListener("pointermove", (e) => dragging && fromEvent(e));
frame.addEventListener("pointerup", () => { dragging = false; });
$("slider").addEventListener("input", (e) => place(Number(e.target.value)));
$("prev").addEventListener("click", () => index > 0 && load(index - 1));
$("next").addEventListener("click", () => index < count - 1 && load(index + 1));
$("done").addEventListener("click", () => (tauri ? tauri.core.invoke("compare_close") : window.close()));
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") $("done").click();
  if (e.target === $("slider")) return;
  if (e.key === "ArrowLeft") place(pos - 5);
  if (e.key === "ArrowRight") place(pos + 5);
});

function demo() {
  const svg = (blur) => "data:image/svg+xml," + encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" width="800" height="500"><defs><filter id="b"><feGaussianBlur stdDeviation="${blur}"/></filter></defs><g filter="url(#b)"><rect width="800" height="500" fill="#c9dcea"/><circle cx="620" cy="110" r="50" fill="#f2c46d"/><polygon points="0,330 130,170 250,290 380,140 520,300 640,190 800,320 800,400 0,400" fill="#8aa6bf"/><rect y="390" width="800" height="110" fill="#6f93ad"/></g></svg>`);
  return { count: 1, index: 0, title: "Beach (500 KB).jpg", what: "", left: svg(0), right: svg(2), leftLabel: "Original · 14.2 MB", rightLabel: "Compressed · 498 KB" };
}

load(0);
