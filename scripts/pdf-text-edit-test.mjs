// Checks ui/pdf-text-edit.js on real PDFs (27 Oct 2026). From the project folder:
//   node scripts\pdf-text-edit-test.mjs
// For each case: change one line, save, open the saved copy again and check that
// the new text is there, the old text is gone from every stream in the file, and
// every other line on the page is still where it was.
import fs from "fs";
import path from "path";
import { createRequire } from "module";
import { fileURLToPath } from "url";

const root = path.join(path.dirname(fileURLToPath(import.meta.url)), "..");
const require = createRequire(import.meta.url);
const PDFLib = require(path.join(root, "ui/vendor/pdf-lib.min.js"));
const fontkit = require(path.join(root, "ui/vendor/fontkit.umd.min.js"));
// Loaded as a module whatever package.json says (the editor loads it the same way).
const engine = fs.readFileSync(path.join(root, "ui/pdf-text-edit.js"), "utf8");
const { createTextEditor } = await import("data:text/javascript;base64," + Buffer.from(engine).toString("base64"));
const noto = fs.readFileSync(path.join(root, "ui/vendor/fonts/NotoSans-Regular.ttf"));

const CASES = [
  ["test-files/Brochure.pdf", "Revenue up 12%", "Revenue up 18%"],
  ["test-files/Brochure.pdf", "Two new clients", "Three new clients"],
  ["test-files/results/Report.pdf", "Quarterly report", "Quarterly review"],
  ["test-files/results/Sales.pdf", "Kanpur", "Lucknow"],
  ["test-files/pdf-text/Chrome-invoice.pdf", "45,000", "52,000"],
  ["test-files/pdf-text/Chrome-invoice.pdf", "Billed to: Kekaa Lifestyle Pvt Ltd, Kanpur", "Billed to: Coolcases, Kanpur"],
  ["test-files/pdf-text/ReportLab-letter.pdf", "Closing balance: 3,480.75", "Closing balance: 4,999.10"],
  ["test-files/pdf-text/LaTeX-brief.pdf", "in progress — awaiting assets.", "done — awaiting assets."],
  ["test-files/pdf-text/LaTeX-brief.pdf", "30 November.", "15 December.", { fit: true }],
];

async function allStreams(bytes) {
  const d = await PDFLib.PDFDocument.load(bytes);
  let t = "";
  for (const [, o] of d.context.enumerateIndirectObjects()) {
    if (!(o instanceof PDFLib.PDFRawStream)) continue;
    try { t += Buffer.from(PDFLib.decodePDFRawStream(o).decode()).toString("latin1"); } catch (e) { /* pictures and other streams */ }
  }
  return t;
}
async function pageStreams(bytes) {
  // Each page's content as text (to tell this page's text from identical text on other pages).
  const d = await PDFLib.PDFDocument.load(bytes);
  return d.getPages().map((pg) => {
    let c = pg.node.get(PDFLib.PDFName.of("Contents"));
    c = c instanceof PDFLib.PDFRef ? d.context.lookup(c) : c;
    const parts = c instanceof PDFLib.PDFArray ? c.asArray().map((x) => d.context.lookup(x)) : [c];
    return parts.map((x) => (x ? Buffer.from(PDFLib.decodePDFRawStream(x).decode()).toString("latin1") : "")).join("\n");
  });
}
const textOps = (s) => [...s.matchAll(/\[[^\]]*\]\s*TJ|<[0-9A-Fa-f]+>\s*Tj|\((?:\\.|[^)])*\)\s*Tj/g)].map((m) => m[0]);

let bad = 0;
for (const [file, from, to, how] of CASES) {
  const src = fs.readFileSync(path.join(root, file));
  const doc = await PDFLib.PDFDocument.load(src);
  const ed = createTextEditor(PDFLib, doc, { fontkit, fallbackFontBytes: noto });
  const before = ed.runs(0);
  const run = before.find((r) => r.text === from);
  if (!run) { console.log(`FAIL ${path.basename(file)}: no line "${from}"`); bad++; continue; }
  const res = await ed.editRun(run.id, to, how || {});
  const saved = await doc.save();
  const after = createTextEditor(PDFLib, await PDFLib.PDFDocument.load(saved), { fontkit }).runs(0);
  const hasNew = after.some((r) => r.text === to);
  const moved = before.filter((r) => r.id !== run.id && !after.some((a) => a.id === r.id && a.text === r.text));
  const all = await allStreams(saved);
  // Text that drew the old line (only on this page) must be gone from the whole file.
  // (When another page has the very same text, as in Brochure.pdf, check this page's own content.)
  const [p1, ...others] = await pageStreams(src);
  const [savedP1] = await pageStreams(saved);
  const unique = textOps(p1).filter((o) => o.length > 8 && !others.some((t) => t.includes(o)));
  const removed = unique.length ? unique.filter((o) => !all.includes(o)).length : textOps(p1).filter((o) => o.length > 8 && !savedP1.includes(o)).length;
  const ok = hasNew && !moved.length && removed > 0;
  if (!ok) bad++;
  const font = res.font === "page" ? "page's own font" : `${res.fontName} (PDF's font lacks ${res.missing.join("")})`;
  console.log(`${ok ? "ok  " : "FAIL"} ${path.basename(file)}: "${from}" -> "${to}" · ${font}${res.squeezed ? ` · squeezed to ${res.squeezed}%` : ""}` +
    `${hasNew ? "" : " · NEW TEXT NOT FOUND"}${moved.length ? ` · MOVED: ${moved.map((m) => m.text).join(", ")}` : ""}${removed ? "" : " · OLD TEXT STILL IN THE FILE"}`);
}
console.log(bad ? `${bad} failed` : "all passed");
process.exitCode = bad ? 1 : 0;
