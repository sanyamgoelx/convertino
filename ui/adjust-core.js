// Convertino PDF editor, Adjust tab: the picture maths, no DOM.
//   homography / warp: Corner Pin (four corners onto a rectangle), like After Effects.
//   applyFilter:       Enhance, Grayscale, Black & white, Invert, plus brightness and contrast.
//   findEdges:         finds a sheet of paper in a photo and returns its four corners.
// Pictures are RGBA byte arrays (ImageData.data), w × h.

/** Solves A·x = b (Gaussian elimination with pivoting). */
function solve(A, b) {
  const n = b.length;
  for (let i = 0; i < n; i++) {
    let p = i;
    for (let r = i + 1; r < n; r++) if (Math.abs(A[r][i]) > Math.abs(A[p][i])) p = r;
    [A[i], A[p]] = [A[p], A[i]];
    [b[i], b[p]] = [b[p], b[i]];
    if (Math.abs(A[i][i]) < 1e-12) throw new Error("Those corners don't make a page shape.");
    for (let r = i + 1; r < n; r++) {
      const f = A[r][i] / A[i][i];
      for (let c = i; c < n; c++) A[r][c] -= f * A[i][c];
      b[r] -= f * b[i];
    }
  }
  const x = new Array(n);
  for (let i = n - 1; i >= 0; i--) {
    let s = b[i];
    for (let c = i + 1; c < n; c++) s -= A[i][c] * x[c];
    x[i] = s / A[i][i];
  }
  return x;
}

/** The 3×3 matrix (8 numbers, last one is 1) that maps each src point to its dst point. */
export function homography(src, dst) {
  const A = [], b = [];
  for (let i = 0; i < 4; i++) {
    const [x, y] = src[i], [X, Y] = dst[i];
    A.push([x, y, 1, 0, 0, 0, -x * X, -y * X]);
    b.push(X);
    A.push([0, 0, 0, x, y, 1, -x * Y, -y * Y]);
    b.push(Y);
  }
  return solve(A, b);
}

export function mapPoint(H, x, y) {
  const d = H[6] * x + H[7] * y + 1;
  return [(H[0] * x + H[1] * y + H[2]) / d, (H[3] * x + H[4] * y + H[5]) / d];
}

/**
 * Fills `out` (w × h) by looking up where each pixel comes from in `src` (sw × sh),
 * with H mapping out → src. Bilinear; outside the source is white.
 */
export function warp(src, sw, sh, H, out, w, h) {
  const [a, b, c, d, e, f, g, k] = H;
  for (let y = 0; y < h; y++) {
    const yy = y + 0.5;
    for (let x = 0; x < w; x++) {
      const xx = x + 0.5;
      const den = g * xx + k * yy + 1;
      const u = (a * xx + b * yy + c) / den - 0.5, v = (d * xx + e * yy + f) / den - 0.5;
      const o = (y * w + x) * 4;
      out[o + 3] = 255;
      if (u < -0.5 || v < -0.5 || u > sw - 0.5 || v > sh - 0.5) {
        out[o] = out[o + 1] = out[o + 2] = 255;
        continue;
      }
      const x0 = Math.max(0, Math.min(sw - 1, Math.floor(u))), y0 = Math.max(0, Math.min(sh - 1, Math.floor(v)));
      const x1 = Math.min(sw - 1, x0 + 1), y1 = Math.min(sh - 1, y0 + 1);
      const fx = Math.max(0, Math.min(1, u - x0)), fy = Math.max(0, Math.min(1, v - y0));
      const i00 = (y0 * sw + x0) * 4, i10 = (y0 * sw + x1) * 4, i01 = (y1 * sw + x0) * 4, i11 = (y1 * sw + x1) * 4;
      for (let ch = 0; ch < 3; ch++) {
        const top = src[i00 + ch] + (src[i10 + ch] - src[i00 + ch]) * fx;
        const bot = src[i01 + ch] + (src[i11 + ch] - src[i01 + ch]) * fx;
        out[o + ch] = top + (bot - top) * fy;
      }
    }
  }
}

export const FILTERS = [
  ["original", "Original"],
  ["enhance", "Enhance"],
  ["gray", "Grayscale"],
  ["bw", "Black & white"],
  ["invert", "Invert"],
];

/**
 * Paper white at each point, from lightness only: the brightest value in small blocks,
 * smoothed. Dividing by it evens out shadows and fall-off, so the paper comes out white.
 * Colour is corrected once for the whole page (cast), so text doesn't get coloured halos.
 */
function paperWhite(px, w, h, block) {
  const B = Math.max(8, Math.round(block));
  const bw = Math.ceil(w / B), bh = Math.ceil(h / B);
  let m = new Float32Array(bw * bh);
  const step = Math.max(1, Math.floor(B / 12));
  const lum = (i) => 0.299 * px[i] + 0.587 * px[i + 1] + 0.114 * px[i + 2];
  for (let by = 0; by < bh; by++) {
    for (let bx = 0; bx < bw; bx++) {
      let hi = 0;
      const ye = Math.min(h, by * B + B), xe = Math.min(w, bx * B + B);
      for (let y = by * B; y < ye; y += step) {
        for (let x = bx * B; x < xe; x += step) {
          const l = lum((y * w + x) * 4);
          if (l > hi) hi = l;
        }
      }
      m[by * bw + bx] = hi;
    }
  }
  // Blocks full of ink are darker than their paper: lift each block to the brightest
  // of its neighbours, then smooth (3 passes of a 3×3 mean).
  const t0 = m.slice();
  for (let by = 0; by < bh; by++) {
    for (let bx = 0; bx < bw; bx++) {
      let hi = 0;
      for (let dy = -1; dy <= 1; dy++) for (let dx = -1; dx <= 1; dx++) {
        const yy = by + dy, xx = bx + dx;
        if (yy >= 0 && xx >= 0 && yy < bh && xx < bw && t0[yy * bw + xx] > hi) hi = t0[yy * bw + xx];
      }
      m[by * bw + bx] = hi;
    }
  }
  for (let pass = 0; pass < 3; pass++) {
    const t = m.slice();
    for (let by = 0; by < bh; by++) {
      for (let bx = 0; bx < bw; bx++) {
        let s = 0, n = 0;
        for (let dy = -1; dy <= 1; dy++) for (let dx = -1; dx <= 1; dx++) {
          const yy = by + dy, xx = bx + dx;
          if (yy < 0 || xx < 0 || yy >= bh || xx >= bw) continue;
          s += t[yy * bw + xx];
          n++;
        }
        m[by * bw + bx] = Math.max(40, s / n);
      }
    }
  }
  const at = (x, y) => {
    const fx = Math.max(0, Math.min(bw - 1, x / B - 0.5)), fy = Math.max(0, Math.min(bh - 1, y / B - 0.5));
    const x0 = Math.floor(fx), y0 = Math.floor(fy), x1 = Math.min(bw - 1, x0 + 1), y1 = Math.min(bh - 1, y0 + 1);
    const ax = fx - x0, ay = fy - y0;
    return (m[y0 * bw + x0] * (1 - ax) + m[y0 * bw + x1] * ax) * (1 - ay) + (m[y1 * bw + x0] * (1 - ax) + m[y1 * bw + x1] * ax) * ay;
  };
  // The paper's colour cast: average colour of pixels that are paper (close to paper white).
  let cr = 0, cg = 0, cb = 0, n = 0;
  const sstep = Math.max(1, Math.round(Math.sqrt((w * h) / 40000)));
  for (let y = 0; y < h; y += sstep) {
    for (let x = 0; x < w; x += sstep) {
      const i = (y * w + x) * 4, l = lum(i), pw = at(x, y);
      if (l > pw * 0.9 && l > 30) { cr += px[i] / l; cg += px[i + 1] / l; cb += px[i + 2] / l; n++; }
    }
  }
  const cast = n ? [cr / n, cg / n, cb / n] : [1, 1, 1];
  return { at, cast };
}

/**
 * Applies a filter in place. `dpi` sets the block size for Enhance and Black & white.
 * bright and contrast run from -50 to 50 (0 = unchanged).
 */
export function applyFilter(px, w, h, filter, bright = 0, contrast = 0, dpi = 150) {
  const even = filter === "enhance" || filter === "bw";
  const pw = even ? paperWhite(px, w, h, dpi * 0.16) : null;
  const k = (100 + contrast * 1.6) / 100, add = bright * 1.6;
  const plain = !bright && !contrast;
  const lut = new Uint8ClampedArray(256);
  for (let v = 0; v < 256; v++) lut[v] = (v - 128) * k + 128 + add;
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const i = (y * w + x) * 4;
      let r = px[i], g = px[i + 1], b = px[i + 2];
      if (pw) {
        const white = pw.at(x, y);
        r = (r / (white * pw.cast[0])) * 255;
        g = (g / (white * pw.cast[1])) * 255;
        b = (b / (white * pw.cast[2])) * 255;
      }
      if (filter === "enhance") {
        // Paper to white, ink a little deeper, colours kept.
        r = (r - 34) * 1.18; g = (g - 34) * 1.18; b = (b - 34) * 1.18;
      } else if (filter === "gray") {
        r = g = b = 0.299 * r + 0.587 * g + 0.114 * b;
      } else if (filter === "bw") {
        r = g = b = 0.299 * r + 0.587 * g + 0.114 * b > 172 ? 255 : 0;
      } else if (filter === "invert") {
        r = 255 - r; g = 255 - g; b = 255 - b;
      }
      r = r < 0 ? 0 : r > 255 ? 255 : r;
      g = g < 0 ? 0 : g > 255 ? 255 : g;
      b = b < 0 ? 0 : b > 255 ? 255 : b;
      if (plain) {
        px[i] = r; px[i + 1] = g; px[i + 2] = b;
      } else {
        px[i] = lut[r | 0]; px[i + 1] = lut[g | 0]; px[i + 2] = lut[b | 0];
      }
    }
  }
}

/**
 * Finds a sheet of paper in a photo (a light shape on a darker background).
 * Returns its corners [TL, TR, BR, BL] as fractions of w and h, or null when the
 * picture is all paper (a normal PDF page) or no clear page shows.
 */
export function findEdges(px, w, h) {
  const n = w * h;
  const gray = new Float32Array(n);
  for (let i = 0; i < n; i++) gray[i] = 0.299 * px[i * 4] + 0.587 * px[i * 4 + 1] + 0.114 * px[i * 4 + 2];
  // Blur (two 5-wide box passes) so text inside the page doesn't break it up.
  const blur = (a) => {
    const t = new Float32Array(n), R = 2;
    for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
      let s = 0, c = 0;
      for (let d = -R; d <= R; d++) { const xx = x + d; if (xx >= 0 && xx < w) { s += a[y * w + xx]; c++; } }
      t[y * w + x] = s / c;
    }
    const o = new Float32Array(n);
    for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
      let s = 0, c = 0;
      for (let d = -R; d <= R; d++) { const yy = y + d; if (yy >= 0 && yy < h) { s += t[yy * w + x]; c++; } }
      o[y * w + x] = s / c;
    }
    return o;
  };
  const g = blur(blur(gray));
  // Otsu's threshold between paper and background.
  const hist = new Float64Array(256);
  for (let i = 0; i < n; i++) hist[Math.max(0, Math.min(255, g[i] | 0))]++;
  let sum = 0;
  for (let t = 0; t < 256; t++) sum += t * hist[t];
  let sumB = 0, wB = 0, best = 0, thr = 128;
  for (let t = 0; t < 256; t++) {
    wB += hist[t];
    if (!wB) continue;
    const wF = n - wB;
    if (!wF) break;
    sumB += t * hist[t];
    const mB = sumB / wB, mF = (sum - sumB) / wF;
    const between = wB * wF * (mB - mF) * (mB - mF);
    if (between > best) { best = between; thr = t; }
  }
  // A flat page (almost all one tone) has no paper edge to find.
  let light = 0;
  for (let i = 0; i < n; i++) if (g[i] > thr) light++;
  const spread = Math.sqrt(best / (n * n)); // ~ half the gap between the two tones
  if (spread < 12) return null;

  // The light region joined to the middle of the picture (or else the biggest one).
  const label = new Int32Array(n).fill(-1);
  const stack = new Int32Array(n);
  let bestLabel = -1, bestSize = 0, centreLabel = -1;
  const centre = ((h / 2) | 0) * w + ((w / 2) | 0);
  let next = 0;
  for (let s0 = 0; s0 < n; s0++) {
    if (label[s0] !== -1 || g[s0] <= thr) continue;
    let top = 0, size = 0;
    stack[top++] = s0;
    label[s0] = next;
    while (top) {
      const p = stack[--top];
      size++;
      const x = p % w, y = (p / w) | 0;
      if (x > 0 && label[p - 1] === -1 && g[p - 1] > thr) { label[p - 1] = next; stack[top++] = p - 1; }
      if (x < w - 1 && label[p + 1] === -1 && g[p + 1] > thr) { label[p + 1] = next; stack[top++] = p + 1; }
      if (y > 0 && label[p - w] === -1 && g[p - w] > thr) { label[p - w] = next; stack[top++] = p - w; }
      if (y < h - 1 && label[p + w] === -1 && g[p + w] > thr) { label[p + w] = next; stack[top++] = p + w; }
    }
    if (size > bestSize) { bestSize = size; bestLabel = next; }
    if (label[centre] === next) centreLabel = next;
    next++;
  }
  const pick = centreLabel >= 0 && countOf(label, centreLabel) > n * 0.08 ? centreLabel : bestLabel;
  const area = countOf(label, pick);
  if (area < n * 0.08 || area > n * 0.97) return null;

  // Corners: the region's furthest points towards each corner of the picture.
  let tl = [0, 0, Infinity], tr = [0, 0, -Infinity], br = [0, 0, -Infinity], bl = [0, 0, Infinity];
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
    if (label[y * w + x] !== pick) continue;
    const s = x + y, d = x - y;
    if (s < tl[2]) tl = [x, y, s];
    if (s > br[2]) br = [x, y, s];
    if (d > tr[2]) tr = [x, y, d];
    if (d < bl[2]) bl = [x, y, d];
  }
  // Pull each corner a little towards the middle, so no background shows along the edges.
  const mx = (tl[0] + tr[0] + br[0] + bl[0]) / 4, my = (tl[1] + tr[1] + br[1] + bl[1]) / 4;
  const q = [tl, tr, br, bl].map(([x, y]) => [(x + 0.5 + (mx - x) * 0.008) / w, (y + 0.5 + (my - y) * 0.008) / h]);
  // Reject shapes that aren't a believable page (a sliver or a twisted quad).
  if (!convex(q) || quadArea(q) < 0.06) return null;
  return q;
}

function countOf(label, v) {
  let c = 0;
  for (let i = 0; i < label.length; i++) if (label[i] === v) c++;
  return c;
}
function quadArea(q) {
  let s = 0;
  for (let i = 0; i < 4; i++) { const [x1, y1] = q[i], [x2, y2] = q[(i + 1) % 4]; s += x1 * y2 - x2 * y1; }
  return Math.abs(s) / 2;
}
/** True when the four corners (in order) make a convex shape, so the pin can't fold over. */
export function convex(q) {
  let sign = 0;
  for (let i = 0; i < 4; i++) {
    const [ax, ay] = q[i], [bx, by] = q[(i + 1) % 4], [cx, cy] = q[(i + 2) % 4];
    const z = (bx - ax) * (cy - by) - (by - ay) * (cx - bx);
    if (Math.abs(z) < 1e-9) return false;
    const s = Math.sign(z);
    if (sign && s !== sign) return false;
    sign = s;
  }
  return true;
}

// ---------- corners and page turns ----------
// Corners are stored on the page as it is in the PDF (before any turn added in the
// editor), as fractions 0–1, in the order top-left, top-right, bottom-right, bottom-left.

/** A point on the page → the same point on the page turned clockwise by `deg`. */
export function turnPoint([u, v], deg) {
  switch (((deg % 360) + 360) % 360) {
    case 90: return [1 - v, u];
    case 180: return [1 - u, 1 - v];
    case 270: return [v, 1 - u];
    default: return [u, v];
  }
}
/** The reverse of turnPoint. */
export function unturnPoint(p, deg) {
  return turnPoint(p, 360 - (((deg % 360) + 360) % 360));
}

export const IDENTITY = [[0, 0], [1, 0], [1, 1], [0, 1]];
export function isIdentity(q) {
  return q.every(([u, v], i) => Math.abs(u - IDENTITY[i][0]) < 1e-4 && Math.abs(v - IDENTITY[i][1]) < 1e-4);
}

/** Paper sizes in points (portrait). */
export const PAPER = { a4: [595.28, 841.89], letter: [612, 792] };

/**
 * The adjusted page's size in points. base = the page's own size (w, h) in points.
 * a = { on, q, size }.
 */
export function outputSize(a, [bw, bh]) {
  if (!a.on) return [bw, bh];
  const q = a.q.map(([u, v]) => [u * bw, v * bh]);
  const len = (p, r) => Math.hypot(p[0] - r[0], p[1] - r[1]);
  const cw = (len(q[0], q[1]) + len(q[3], q[2])) / 2;
  const ch = (len(q[0], q[3]) + len(q[1], q[2])) / 2;
  if (a.size === "original") return [bw, bh];
  if (a.size === "a4" || a.size === "letter") {
    const [pw, ph] = PAPER[a.size];
    return cw > ch ? [ph, pw] : [pw, ph];
  }
  return [Math.max(36, cw), Math.max(36, ch)];
}
