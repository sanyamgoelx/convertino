//! Compress to a size: "make it no bigger than 10 MB".
//!
//! The person picks a size on the wheel's size ring (or types one in its
//! centre); Convertino decides what to give up to get there:
//!
//! - Pictures: the quality first, down to a floor, then the resolution. A PNG
//!   without transparency becomes a JPG (with transparency, a WebP).
//! - Videos: a bitrate from the size and the length, then the resolution
//!   (camera video) or the frame rate first (screen recordings, where sharp
//!   text matters more than smooth motion). A generous size just gets the
//!   usual "smallest that looks the same" Compress.
//! - PDFs: Ghostscript at several picture resolutions; the gentlest that fits.
//! - Audio: a bitrate from the size and the length; mono for very small sizes.
//!
//! The same numbers drive the wheel's preview (what each size will cost) and
//! the conversion itself, so what the ring promises is what the job does.

use crate::convert::{self, s, Op, Step, TempDir};
use crate::look::{self, Lossy};
use crate::tools::{self, Tool};
use crate::video;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// What the wheel asks for: a size in bytes, for each file or for all the
/// selected files together, and optionally only the start of a video.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ask {
    pub bytes: u64,
    #[serde(default)]
    pub together: bool,
    /// Keep only the first this many seconds (a video too long for the size).
    #[serde(default)]
    pub trim: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Image,
    Video,
    Pdf,
    Audio,
}

impl Kind {
    pub fn of_target(target_id: &str) -> Option<Kind> {
        match target_id {
            "image.compress" | "raw.compress" => Some(Kind::Image),
            "video.compress" => Some(Kind::Video),
            "pdf.compress" => Some(Kind::Pdf),
            "audio.compress" => Some(Kind::Audio),
            _ => None,
        }
    }

    fn noun(self, n: usize) -> String {
        let one = match self {
            Kind::Image => "picture",
            Kind::Video => "video",
            Kind::Pdf => "PDF",
            Kind::Audio => "recording",
        };
        if n == 1 { one.to_string() } else { format!("{n} {one}s") }
    }
}

/// What Convertino knows about one file, measured once and cached.
#[derive(Debug, Clone, Default)]
pub struct Info {
    pub path: PathBuf,
    pub bytes: u64,
    pub ext: String,
    // Pictures
    pub width: u32,
    pub height: u32,
    pub alpha: bool,
    // Video and audio
    pub duration: f64,
    pub fps: f64,
    pub has_audio: bool,
    pub audio_codec: Option<String>,
    pub audio_kbps: Option<u32>,
    /// A screen recording: frame rate goes before resolution.
    pub screen: bool,
    /// Not measured (the converter isn't downloaded yet): only the size is known.
    pub rough: bool,
}

impl Info {
    fn pixels(&self) -> f64 {
        (self.width.max(1) as f64) * (self.height.max(1) as f64)
    }
    fn name(&self) -> String {
        self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }
}

fn cache() -> &'static Mutex<HashMap<(PathBuf, u64), Info>> {
    static C: OnceLock<Mutex<HashMap<(PathBuf, u64), Info>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Measures a file (cached by path and size), downloading a missing converter.
pub fn info(kind: Kind, path: &Path) -> Result<Info, String> {
    measure(kind, path, true)
}

/// For the wheel: measures with the converters already there; otherwise
/// only the file's size is known (the job measures it properly).
pub fn quick_info(kind: Kind, path: &Path) -> Result<Info, String> {
    match measure(kind, path, false) {
        Ok(i) => Ok(i),
        Err(e) => {
            log::info!("size: {} not measured ({e})", path.display());
            let bytes = path.metadata().map(|m| m.len()).map_err(|_| format!("{} is no longer there.", path.display()))?;
            let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            Ok(Info { path: path.to_path_buf(), bytes, ext, rough: true, ..Info::default() })
        }
    }
}

fn measure(kind: Kind, path: &Path, download: bool) -> Result<Info, String> {
    let needs = match kind {
        Kind::Image => Some(Tool::Magick),
        Kind::Video | Kind::Audio => Some(Tool::Ffmpeg),
        Kind::Pdf => None,
    };
    if let (Some(t), false) = (needs, download) {
        tools::find(t).ok_or_else(|| format!("{} isn't downloaded yet", t.display_name()))?;
    }
    let bytes = path.metadata().map(|m| m.len()).map_err(|_| format!("{} is no longer there.", path.display()))?;
    let key = (path.to_path_buf(), bytes);
    if let Some(hit) = cache().lock().ok().and_then(|c| c.get(&key).cloned()) {
        return Ok(hit);
    }
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let mut i = Info { path: path.to_path_buf(), bytes, ext, ..Info::default() };
    match kind {
        Kind::Image => {
            let exe = tools::require(Tool::Magick)?;
            let mut src = path.as_os_str().to_os_string();
            src.push("[0]");
            let out = crate::procs::output(
                tools::command(&exe).arg(src).args(["-auto-orient", "-format", "%w %h %[opaque]", "info:"]),
                "ImageMagick",
                None,
            )?;
            let text = String::from_utf8_lossy(&out.stdout).to_string();
            let mut parts = text.split_whitespace();
            i.width = parts.next().and_then(|w| w.parse().ok()).unwrap_or(0);
            i.height = parts.next().and_then(|h| h.parse().ok()).unwrap_or(0);
            // Transparent only if some pixel actually is (many PNGs carry an unused alpha channel).
            i.alpha = parts.next().is_some_and(|o| o.eq_ignore_ascii_case("false"));
            if i.width == 0 || i.height == 0 {
                return Err(format!("ImageMagick couldn't read {}.", i.name()));
            }
        }
        Kind::Video | Kind::Audio => {
            let p = video::probe(path)?;
            i.duration = p.duration.unwrap_or(0.0);
            i.width = p.width;
            i.height = p.height;
            i.fps = p.fps.unwrap_or(30.0);
            i.has_audio = p.audio.is_some();
            i.audio_codec = p.audio.clone();
            i.audio_kbps = p.audio_kbps;
            if kind == Kind::Video {
                i.screen = video::looks_like_screen(path, &p);
            }
        }
        Kind::Pdf => {}
    }
    if let Ok(mut c) = cache().lock() {
        c.insert(key, i.clone());
    }
    Ok(i)
}

// ---------------------------------------------------------------------------
// Sizes in words. Decimal units, like upload limits ("10 MB" = 10,000,000 bytes).

pub const MB: u64 = 1_000_000;
pub const KB: u64 = 1_000;

pub fn words(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{} GB", trim_num(b / 1e9, 2))
    } else if b >= 10e6 {
        format!("{} MB", (b / 1e6).round())
    } else if b >= 1e6 {
        format!("{} MB", trim_num(b / 1e6, 1))
    } else {
        format!("{} KB", ((b / 1e3).round() as u64).max(1))
    }
}

fn trim_num(v: f64, places: usize) -> String {
    let s = format!("{v:.places$}");
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
}

/// A tidy number near `bytes`: whole MB from 10 MB, half MB from 1 MB, 50 KB steps below.
fn tidy(bytes: f64) -> u64 {
    let mb = bytes / MB as f64;
    if mb >= 10.0 {
        (mb.round() as u64) * MB
    } else if mb >= 1.0 {
        ((mb * 2.0).round() / 2.0 * MB as f64) as u64
    } else {
        let kb = ((bytes / KB as f64 / 50.0).round() * 50.0).max(50.0);
        kb as u64 * KB
    }
}

// ---------------------------------------------------------------------------
// Video: what a size buys.

/// How a video is encoded to reach a size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VideoPlan {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub video_kbps: u32,
    pub audio_kbps: u32,
    pub mono: bool,
    /// The size is generous: the usual "looks the same" Compress, capped.
    pub quality_mode: bool,
    /// Seconds encoded (the whole video, or the trimmed start).
    pub seconds: f64,
}

/// Bits per pixel per frame that look fine with H.265, and the least that's
/// still watchable. Screen recordings are mostly still, so need far fewer.
fn bpp(screen: bool) -> (f64, f64) {
    if screen { (0.012, 0.004) } else { (0.035, 0.012) }
}

/// Size of a container's overhead and the encoder's miss, kept in reserve.
const VIDEO_RESERVE: f64 = 0.95;

fn even(v: f64) -> u32 {
    ((v / 2.0).round() as u32).max(1) * 2
}

/// The "looks the same" size, estimated: H.265 at a comfortable bitrate.
fn video_best(i: &Info) -> u64 {
    let px = i.pixels();
    let fps = i.fps.clamp(1.0, 60.0);
    let per = if i.screen { 0.015 } else { 0.05 };
    let audio = if !i.has_audio { 0.0 } else { i.audio_kbps.map(|k| (k as f64).min(128.0)).unwrap_or(128.0) };
    let kbps = per * px * fps / 1000.0 + audio;
    let est = kbps * 1000.0 * i.duration / 8.0;
    (est.min(i.bytes as f64 * 0.95)) as u64
}

fn heights(src: u32) -> Vec<u32> {
    let mut v: Vec<u32> = std::iter::once(src).chain([2160, 1440, 1080, 720, 540, 480, 360, 240]).filter(|h| *h <= src && *h > 0).collect();
    v.dedup();
    v
}

fn frame_rates(i: &Info) -> Vec<f64> {
    let src = i.fps.clamp(1.0, 120.0);
    let mut v = if i.screen {
        vec![src.min(30.0), 15.0, 10.0]
    } else if src > 31.0 {
        vec![src, 30.0]
    } else {
        vec![src]
    };
    v.retain(|f| *f <= src + 0.01);
    v.dedup_by(|a, b| (*a - *b).abs() < 0.5);
    if v.is_empty() {
        v.push(src);
    }
    v
}

fn audio_for(i: &Info, total_kbps: f64) -> (u32, bool) {
    if !i.has_audio {
        return (0, false);
    }
    match total_kbps {
        t if t > 1500.0 => (128, false),
        t if t > 600.0 => (96, false),
        t if t > 250.0 => (64, false),
        _ => (40, true),
    }
}

/// The encode for `target` bytes, or None when even the smallest rung is too little.
pub fn video_plan(i: &Info, target: u64, trim: Option<f64>) -> Option<VideoPlan> {
    let seconds = trim.map(|t| t.min(i.duration)).unwrap_or(i.duration).max(0.5);
    let total_kbps = target as f64 * 8.0 * VIDEO_RESERVE / seconds / 1000.0;
    let (audio_kbps, mono) = audio_for(i, total_kbps);
    let video_kbps = total_kbps - audio_kbps as f64;
    let (src_w, src_h) = (i.width.max(2), i.height.max(2));
    if trim.is_none() && target >= video_best(i) {
        return Some(VideoPlan {
            width: src_w,
            height: src_h,
            fps: i.fps,
            video_kbps: video_kbps.max(100.0) as u32,
            audio_kbps,
            mono,
            quality_mode: true,
            seconds,
        });
    }
    let (good, least) = bpp(i.screen);
    let rungs: Vec<(u32, u32, f64)> = heights(src_h)
        .into_iter()
        .flat_map(|h| {
            let w = if h == src_h { src_w } else { even(src_w as f64 * h as f64 / src_h as f64) };
            frame_rates(i).into_iter().map(move |f| (w, h, f))
        })
        .collect();
    let fits = |&(w, h, f): &(u32, u32, f64), need: f64| video_kbps * 1000.0 / (w as f64 * h as f64 * f) >= need;
    let pick = rungs.iter().find(|r| fits(r, good)).or_else(|| rungs.last().filter(|r| fits(r, least)))?;
    Some(VideoPlan {
        width: pick.0,
        height: pick.1,
        fps: pick.2,
        video_kbps: video_kbps as u32,
        audio_kbps,
        mono,
        quality_mode: false,
        seconds,
    })
}

/// The smallest size a whole video can still be watched at.
fn video_floor(i: &Info) -> u64 {
    let h = heights(i.height.max(2)).last().copied().unwrap_or(240);
    let w = even(i.width.max(2) as f64 * h as f64 / i.height.max(2) as f64);
    let f = frame_rates(i).last().copied().unwrap_or(i.fps);
    let v = bpp(i.screen).1 * w as f64 * h as f64 * f / 1000.0;
    let a = if i.has_audio { 40.0 } else { 0.0 };
    ((v + a) * 1000.0 * i.duration.max(0.5) / 8.0 / VIDEO_RESERVE).ceil() as u64 + 1
}

/// Too small for the whole video: how many seconds fit at a decent picture.
fn video_trim(i: &Info, target: u64) -> Option<Alt> {
    let (good, _) = bpp(i.screen);
    let h = if i.screen { i.height } else { i.height.min(720) };
    let w = even(i.width as f64 * h as f64 / i.height.max(1) as f64);
    let f = if i.screen { i.fps.min(15.0) } else { i.fps.min(30.0) };
    let kbps = good * w as f64 * h as f64 * f / 1000.0 + if i.has_audio { 64.0 } else { 0.0 };
    let secs = (target as f64 * 8.0 * VIDEO_RESERVE / (kbps * 1000.0)).floor();
    if secs < 3.0 || secs >= i.duration {
        return None;
    }
    let label = if i.screen { format!("Keep {h}p, first {} s", secs as u32) } else { format!("Keep {h}p, first {} s", secs as u32) };
    Some(Alt { label, bytes: target, trim: secs })
}

// ---------------------------------------------------------------------------
// Pictures

/// What a compressed picture is saved as: formats every site and app takes.
/// JPG, unless the picture is transparent (PNG, with fewer colours when it
/// must shrink), or already a WebP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pic {
    Jpg,
    Png,
    Webp,
}

impl Pic {
    fn lossy(self) -> Option<Lossy> {
        match self {
            Pic::Jpg => Some(Lossy::Jpg),
            Pic::Webp => Some(Lossy::Webp),
            Pic::Png => None,
        }
    }
}

/// Bytes per pixel at the "looks the same" quality, and at the quality floor
/// (PNG: lossless, and at 256 colours).
fn picture_rates(fmt: Pic) -> (f64, f64) {
    match fmt {
        Pic::Webp => (0.12, 0.05),
        Pic::Jpg => (0.18, 0.07),
        Pic::Png => (1.2, 0.35),
    }
}

/// The format a picture is compressed to.
pub fn picture_format(i: &Info) -> Pic {
    if i.ext == "webp" {
        Pic::Webp
    } else if i.alpha {
        Pic::Png
    } else {
        Pic::Jpg
    }
}

fn format_ext(f: Pic) -> &'static str {
    match f {
        Pic::Webp => "webp",
        Pic::Png => "png",
        Pic::Jpg => "jpg",
    }
}

fn picture_best(i: &Info) -> u64 {
    let fmt = picture_format(i);
    let est = i.pixels() * picture_rates(fmt).0;
    let same_kind = matches!((i.ext.as_str(), fmt), ("jpg" | "jpeg", Pic::Jpg) | ("webp", Pic::Webp));
    let cap = match fmt {
        // Lossless PNG: about what oxipng saves.
        Pic::Png => i.bytes as f64 * 0.9,
        _ if same_kind => i.bytes as f64 * 0.8,
        _ => i.bytes as f64 * 0.95,
    };
    est.min(cap) as u64
}

/// Smallest picture still worth having: 320 px on the long side at the floor quality.
const MIN_SIDE: f64 = 320.0;

fn picture_floor(i: &Info) -> u64 {
    let long = i.width.max(i.height).max(1) as f64;
    let scale = (MIN_SIDE / long).min(1.0);
    let rate = picture_rates(picture_format(i)).1;
    (i.pixels() * scale * scale * rate).max(3_000.0) as u64
}

/// Long side, in pixels, that `target` bytes allow at a decent quality.
fn picture_side(i: &Info, target: u64) -> u32 {
    let rate = picture_rates(picture_format(i)).1 * 1.4;
    let scale = (target as f64 / (i.pixels() * rate)).sqrt().min(1.0);
    (i.width.max(i.height) as f64 * scale).round() as u32
}

// ---------------------------------------------------------------------------
// Audio

fn audio_out_ext(i: &Info) -> &'static str {
    match i.ext.as_str() {
        "m4a" | "aac" => "m4a",
        _ => "mp3",
    }
}

fn audio_kbps_for(i: &Info, target: u64) -> u32 {
    (target as f64 * 8.0 * 0.98 / i.duration.max(0.5) / 1000.0).floor() as u32
}

fn audio_best(i: &Info) -> u64 {
    let k = i.audio_kbps.map(|k| k.min(192)).unwrap_or(192) as f64;
    ((k * 1000.0 * i.duration / 8.0) as u64).min((i.bytes as f64 * 0.95) as u64)
}

const AUDIO_MIN_KBPS: u32 = 24;

fn audio_floor(i: &Info) -> u64 {
    ((AUDIO_MIN_KBPS as f64 + 1.0) * 1000.0 * i.duration.max(0.5) / 8.0 / 0.98) as u64
}

// ---------------------------------------------------------------------------
// What a size will cost, in words.

/// Another way to get there when a size is too small (a video's start).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Alt {
    pub label: String,
    pub bytes: u64,
    pub trim: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    /// Already this small: nothing to do.
    pub fits: bool,
    /// Reachable.
    pub ok: bool,
    /// For the slot's tag: "720p", "Full quality", "15 fps".
    pub short: String,
    pub line1: String,
    pub line2: String,
    /// About how big the result will be (all files).
    pub est: u64,
    /// The smallest Convertino can make (all files, together).
    pub min: u64,
    pub alt: Option<Alt>,
}

fn upper(ext: &str) -> String {
    match ext {
        "jpeg" => "JPG".into(),
        e => e.to_uppercase(),
    }
}

/// Already no bigger than `limit`, so left as it is. Never for a RAW photo:
/// it's compressed into a JPG whatever its size.
fn keeps(i: &Info, limit: u64) -> bool {
    i.bytes <= limit && !crate::raw::is_raw(&i.path)
}

pub fn best(kind: Kind, i: &Info) -> u64 {
    if i.rough {
        return (i.bytes as f64 * 0.6) as u64;
    }
    match kind {
        Kind::Image => picture_best(i),
        Kind::Video => video_best(i),
        // Unknown until tried: most PDFs with pictures lose about a third.
        Kind::Pdf => (i.bytes as f64 * 0.65) as u64,
        Kind::Audio => audio_best(i),
    }
}

pub fn floor(kind: Kind, i: &Info) -> u64 {
    if i.rough {
        return (i.bytes as f64 * 0.02) as u64;
    }
    match kind {
        Kind::Image => picture_floor(i),
        Kind::Video => video_floor(i),
        Kind::Pdf => (i.bytes as f64 * 0.05) as u64,
        Kind::Audio => audio_floor(i),
    }
}

/// One file to `target` bytes.
pub fn preview_one(kind: Kind, i: &Info, target: u64, trim: Option<f64>) -> Preview {
    let mut p = Preview { fits: false, ok: true, short: String::new(), line1: String::new(), line2: String::new(), est: target, min: floor(kind, i), alt: None };
    if trim.is_none() && keeps(i, target) {
        p.fits = true;
        p.short = "Already fits".into();
        p.line1 = format!("{} is already {}", i.name(), words(i.bytes));
        p.line2 = "Nothing to do".into();
        p.est = i.bytes;
        return p;
    }
    let best = best(kind, i);
    if i.rough {
        let pct = (target as f64 / i.bytes.max(1) as f64 * 100.0).round().max(1.0);
        p.short = format!("{pct}%");
        p.line1 = format!("About {pct}% of the original");
        p.line2 = "Convertino works out how when it starts".into();
        p.ok = target >= p.min;
        return p;
    }
    match kind {
        Kind::Image => {
            let fmt = picture_format(i);
            let out = format_ext(fmt);
            let change = if upper(&i.ext) != upper(out) { format!("{} → {} · ", upper(&i.ext), upper(out)) } else { String::new() };
            let long = i.width.max(i.height);
            if target >= best {
                p.short = "Full quality".into();
                p.line1 = format!("{change}full {long} px");
                p.line2 = if fmt == Pic::Png { "Lossless · stays PNG to keep the transparency".into() } else { "Looks the same".into() };
                p.est = best;
            } else if target < p.min {
                p.ok = false;
            } else {
                let side = picture_side(i, target).min(long);
                if side as f64 >= long as f64 * 0.97 {
                    p.short = "Full size".into();
                    p.line1 = format!("{change}full {long} px");
                    p.line2 = if fmt == Pic::Png { "Fewer colours to fit · stays PNG for the transparency".into() } else { "Quality lowered to fit".into() };
                } else {
                    p.short = format!("{side} px");
                    p.line1 = format!("{change}{long} → {side} px");
                    p.line2 = if fmt == Pic::Png { "Smaller picture · stays PNG for the transparency".into() } else { "Smaller picture, quality kept up".into() };
                }
            }
        }
        Kind::Video => match video_plan(i, target, trim) {
            Some(v) if v.quality_mode => {
                p.short = "Full quality".into();
                p.line1 = format!("Keeps {}p", i.height);
                p.line2 = "Looks the same as the original".into();
                p.est = best;
            }
            Some(v) => {
                let fps_down = v.fps + 0.5 < i.fps.min(120.0);
                let res_down = v.height < i.height;
                let f = v.fps.round() as u32;
                p.short = match (res_down, fps_down) {
                    (true, true) => format!("{}p {f} fps", v.height),
                    (false, true) => format!("{f} fps"),
                    _ => format!("{}p", v.height),
                };
                p.line1 = match (res_down, fps_down) {
                    (true, true) => format!("{}p → {}p at {f} fps", i.height, v.height),
                    (true, false) => format!("{}p → {}p", i.height, v.height),
                    (false, true) => format!("Keeps {}p, {} → {f} fps", i.height, i.fps.round() as u32),
                    (false, false) => format!("Keeps {}p", i.height),
                };
                p.line2 = if i.screen && fps_down {
                    "Screen recording: the frame rate goes first so text stays sharp".into()
                } else if v.mono {
                    "Quality lowered to fit · mono sound".into()
                } else {
                    "Quality lowered to fit".into()
                };
                if let Some(t) = trim {
                    p.line1 = format!("{} · first {} s", p.line1, t.round() as u32);
                }
            }
            None => {
                p.ok = false;
                p.alt = video_trim(i, target);
            }
        },
        Kind::Pdf => {
            let r = target as f64 / i.bytes.max(1) as f64;
            if target >= best {
                p.short = "Light".into();
                p.line1 = "Pictures kept, fonts and structure tidied".into();
                p.line2 = "Looks the same · the gentlest that fits is used".into();
            } else if target < p.min {
                p.ok = false;
            } else {
                let dpi = if r >= 0.4 { 150 } else if r >= 0.2 { 100 } else if r >= 0.1 { 72 } else { 50 };
                p.short = format!("{dpi} DPI");
                p.line1 = format!("Pictures at about {dpi} DPI");
                p.line2 = if dpi >= 100 { "Fine on screen and for most printing".into() } else { "Fine on screen".into() };
            }
        }
        Kind::Audio => {
            if target >= best {
                p.short = "Full quality".into();
                p.line1 = format!("{} at about {} kbps", upper(audio_out_ext(i)), i.audio_kbps.map(|k| k.min(192)).unwrap_or(192));
                p.line2 = "Sounds the same".into();
                p.est = best;
            } else {
                let k = audio_kbps_for(i, target);
                if k < AUDIO_MIN_KBPS {
                    p.ok = false;
                } else {
                    let mono = k < 64;
                    p.short = format!("{k} kbps");
                    p.line1 = format!("{} at {k} kbps{}", upper(audio_out_ext(i)), if mono { ", mono" } else { "" });
                    p.line2 = if k < 40 { "Fine for speech".into() } else if mono { "Fine for speech and podcasts".into() } else { "Quality lowered to fit".into() };
                }
            }
        }
    }
    if !p.ok {
        p.short = "Too small".into();
        p.line1 = format!("Can't fit {} in {}", i.name(), words(target));
        p.line2 = format!(
            "The smallest that still {} is about {}",
            match kind {
                Kind::Video => "plays well",
                Kind::Audio => "sounds right",
                _ => "looks right",
            },
            words(p.min)
        );
    }
    p
}

/// Each file's share of a size meant for all of them together: by how much
/// each needs, and files already small enough are left out (None).
pub fn shares(kind: Kind, infos: &[Info], total: u64) -> Vec<Option<u64>> {
    let mut fitted = vec![false; infos.len()];
    loop {
        let used: u64 = infos.iter().zip(&fitted).filter(|(_, f)| **f).map(|(i, _)| i.bytes).sum();
        let budget = total.saturating_sub(used) as f64;
        let need: f64 = infos.iter().zip(&fitted).filter(|(_, f)| !**f).map(|(i, _)| best(kind, i).max(1) as f64).sum();
        let share = |i: &Info| (budget * best(kind, i).max(1) as f64 / need.max(1.0)) as u64;
        // Tiny files (a fiftieth of the size) aren't worth touching either.
        let newly: Vec<usize> = infos.iter().enumerate().filter(|(n, i)| !fitted[*n] && !crate::raw::is_raw(&i.path) && (i.bytes <= share(i) || i.bytes * 50 <= total)).map(|(n, _)| n).collect();
        if newly.is_empty() {
            return infos.iter().zip(&fitted).map(|(i, f)| if *f { None } else { Some(share(i)) }).collect();
        }
        for n in newly {
            fitted[n] = true;
        }
    }
}

/// What `target` does to the selected files: for each of them (`together`
/// false) or for all of them together.
pub fn preview(kind: Kind, infos: &[Info], target: u64, together: bool, trim: Option<f64>) -> Preview {
    if infos.len() == 1 {
        return preview_one(kind, &infos[0], target, trim);
    }
    let targets: Vec<Option<u64>> = if together {
        shares(kind, infos, target)
    } else {
        infos.iter().map(|i| if keeps(i, target) { None } else { Some(target) }).collect()
    };
    let looks: Vec<(usize, Preview)> = targets.iter().enumerate().filter_map(|(n, t)| t.map(|t| (n, preview_one(kind, &infos[n], t, trim)))).collect();
    let small = targets.iter().filter(|t| t.is_none()).count();
    let small_bytes: u64 = infos.iter().zip(&targets).filter(|(_, t)| t.is_none()).map(|(i, _)| i.bytes).sum();
    let min: u64 = if together {
        infos.iter().map(|i| floor(kind, i).min(i.bytes)).sum()
    } else {
        infos.iter().map(|i| floor(kind, i)).max().unwrap_or(0)
    };
    if looks.is_empty() {
        return Preview {
            fits: true,
            ok: true,
            short: "Already fits".into(),
            line1: format!("All {} are already small enough", kind.noun(infos.len())),
            line2: "Nothing to do".into(),
            est: small_bytes,
            min,
            alt: None,
        };
    }
    // The biggest file speaks for the rest.
    let (_, lead) = looks.iter().max_by_key(|(n, _)| infos[*n].bytes).cloned().expect("not empty");
    let ok = looks.iter().all(|(_, p)| p.ok);
    let est = looks.iter().map(|(_, p)| p.est).sum::<u64>() + if together { small_bytes } else { 0 };
    let mut line2 = lead.line2.clone();
    if small > 0 {
        line2 = format!("{line2} · {small} already small enough");
    }
    if !ok {
        let scope = if together { "together" } else { "each" };
        return Preview {
            fits: false,
            ok: false,
            short: "Too small".into(),
            line1: format!("Can't fit the {} in {} {scope}", kind.noun(infos.len()), words(target)),
            line2: format!("The smallest that still looks right is about {} {scope}", words(min)),
            est,
            min,
            alt: None,
        };
    }
    Preview {
        fits: false,
        ok: true,
        short: lead.short.clone(),
        line1: format!("{} · {}", kind.noun(looks.len()), lead.line1),
        line2,
        est,
        min,
        alt: None,
    }
}

/// A size offered on the ring.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    pub bytes: u64,
    pub label: String,
    /// "Discord", "Email", or empty.
    pub name: String,
    pub preview: Preview,
}

/// Common upload limits. Gmail's 25 MB counts the encoded attachment, so a
/// file must stay under about 18 MB.
const NAMED: [(u64, &str); 2] = [(10 * MB, "Discord"), (18 * MB, "Email")];

/// Up to five sizes worth offering for these files: the common limits, then
/// steps down from the "looks the same" size. Sizes the files already fit,
/// that can't be reached, or are within a fifth of another are left out.
pub fn presets(kind: Kind, infos: &[Info], together: bool) -> Vec<Preset> {
    if infos.is_empty() {
        return Vec::new();
    }
    let (base, biggest) = if together {
        (infos.iter().map(|i| best(kind, i)).sum::<u64>(), infos.iter().map(|i| i.bytes).sum::<u64>())
    } else {
        (infos.iter().map(|i| best(kind, i)).max().unwrap_or(0), infos.iter().map(|i| i.bytes).max().unwrap_or(0))
    };
    let mut out: Vec<Preset> = Vec::new();
    let near = |a: u64, b: u64| (a as f64 - b as f64).abs() / (a.max(b).max(1) as f64) < 0.2;
    let add = |bytes: u64, name: &str, out: &mut Vec<Preset>| {
        if out.len() >= 5 || bytes == 0 || bytes as f64 > base as f64 * 1.1 || bytes >= biggest {
            return;
        }
        if out.iter().any(|p| near(p.bytes, bytes)) {
            return;
        }
        let pv = preview(kind, infos, bytes, together, None);
        if !pv.ok || pv.fits {
            return;
        }
        out.push(Preset { bytes, label: words(bytes), name: name.to_string(), preview: pv });
    };
    for (b, n) in NAMED {
        add(b, n, &mut out);
    }
    for f in [1.0, 0.5, 0.25, 0.1, 1.0 / 30.0, 1.0 / 3.0, 1.0 / 6.0, 0.01] {
        add(tidy(base as f64 * f), "", &mut out);
    }
    out.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    out
}

/// The ring's title for these files: the file's name, or "3 videos".
pub fn title(kind: Kind, infos: &[Info]) -> String {
    match infos {
        [one] => one.name(),
        many => kind.noun(many.len()),
    }
}

// ---------------------------------------------------------------------------
// Planning the job

/// Notes from planning ("Beach.png is already under 2 MB").
pub struct Planned {
    pub steps: Vec<Step>,
    pub notes: Vec<String>,
}

/// Steps for a size job. Files already small enough get a note instead of a step.
pub fn plan(kind: Kind, files: &[PathBuf], ask: Option<Ask>) -> Result<Planned, String> {
    let infos: Vec<Info> = files.iter().map(|f| info(kind, f)).collect::<Result<_, _>>()?;
    let targets: Vec<Option<u64>> = match ask {
        None => infos.iter().map(|_| Some(u64::MAX)).collect(),
        Some(a) if a.together && infos.len() > 1 => shares(kind, &infos, a.bytes),
        Some(a) => infos.iter().map(|i| if a.trim.is_none() && keeps(i, a.bytes) { None } else { Some(a.bytes) }).collect(),
    };
    let label = ask.map(|a| words(a.bytes));
    let mut steps = Vec::new();
    let mut notes = Vec::new();
    for (i, t) in infos.iter().zip(targets) {
        let (dir, stem, _) = convert::split(&i.path);
        let Some(t) = t else {
            notes.push(format!("{} is already {}, so it was left as it is.", i.name(), words(i.bytes)));
            continue;
        };
        let ext = match kind {
            Kind::Image => format_ext(picture_format(i)).to_string(),
            Kind::Video => "mp4".into(),
            Kind::Pdf => "pdf".into(),
            Kind::Audio => audio_out_ext(i).to_string(),
        };
        let stem = match (&label, ask.and_then(|a| a.trim)) {
            (Some(l), Some(secs)) => format!("{stem} ({l}, first {} s)", secs.round() as u32),
            (Some(l), None) => format!("{stem} ({l})"),
            (None, _) => format!("{stem} (compressed)"),
        };
        let bytes = if t == u64::MAX { None } else { Some(t) };
        steps.push(Step { inputs: vec![i.path.clone()], dir, stem, ext, folder: false, op: Op::ToSize { kind, bytes, trim: ask.and_then(|a| a.trim) } });
    }
    if steps.is_empty() {
        let what = if infos.len() == 1 { format!("{} is already {}", infos[0].name(), words(infos[0].bytes)) } else { format!("All {} are already small enough", kind.noun(infos.len())) };
        return Err(format!("{ALREADY}{what}, so there's nothing to do."));
    }
    Ok(Planned { steps, notes })
}

/// Starts the message when every file already fits.
pub const ALREADY: &str = "Already small enough: ";

// ---------------------------------------------------------------------------
// Running

pub fn run(kind: Kind, input: &Path, bytes: Option<u64>, trim: Option<f64>, output: &Path, progress: &mut dyn FnMut(f64)) -> Result<PathBuf, String> {
    match kind {
        Kind::Image => picture_to(input, bytes, output, progress).map(|_| output.to_path_buf()),
        Kind::Video => {
            let i = info(kind, input)?;
            match bytes {
                None => video::run(input, video::Job::Compress, &video::Opts::from_quality(&crate::settings::get().quality), output, progress),
                Some(b) => {
                    let plan = video_plan(&i, b, trim).ok_or_else(|| too_small(kind, &i, b))?;
                    video::run_to_size(input, &i, &plan, b, trim, output, progress)?;
                    Ok(output.to_path_buf())
                }
            }
        }
        Kind::Pdf => convert::pdf_to_size(input, bytes, output, progress).map(|_| output.to_path_buf()),
        Kind::Audio => audio_to(input, bytes, output, progress).map(|_| output.to_path_buf()),
    }
}

fn too_small(kind: Kind, i: &Info, target: u64) -> String {
    let p = preview_one(kind, i, target, None);
    format!("{}. {}.", p.line1, p.line2)
}

/// Picture quality floors: below these, a smaller picture looks better than a worse one.
fn quality_floor(f: Lossy) -> u32 {
    match f {
        Lossy::Webp => 50,
        _ => 55,
    }
}

/// A transparent picture stays PNG: lossless and smaller first, then 256
/// colours (dithered, like pngquant), then smaller pictures at 256 colours.
fn png_to(input: &Path, i: &Info, target: Option<u64>, output: &Path, progress: &mut dyn FnMut(f64)) -> Result<(), String> {
    let tmp = TempDir::new("sizepng")?;
    let make = |side: Option<u32>, colours: bool, tag: &str| -> Result<(PathBuf, u64), String> {
        let out = tmp.0.join(format!("{tag}.png"));
        let mut src = input.as_os_str().to_os_string();
        src.push("[0]");
        let mut cmd = tools::command(&tools::require(Tool::Magick)?);
        cmd.arg(src).arg("-auto-orient");
        if let Some(px) = side {
            cmd.args(["-resize", &format!("{px}x{px}>")]);
        }
        if colours {
            cmd.args(["-dither", "FloydSteinberg", "-colors", "256"]);
        }
        cmd.args(["-define", "png:compression-level=9"]).arg(&out);
        convert::exec(Tool::Magick, cmd)?;
        look::shrink_png(&out, Some(look::Level::Best));
        let size = out.metadata().map(|m| m.len()).unwrap_or(u64::MAX);
        Ok((out, size))
    };
    let (lossless, size) = make(None, false, "full")?;
    progress(0.3);
    let target = match target {
        Some(t) if size > t => t,
        _ => return convert::move_file(&lossless, output),
    };
    let (fewer, size) = make(None, true, "colours")?;
    progress(0.5);
    if size <= target {
        log::info!("size: {} as a 256-colour PNG, {size} bytes", input.display());
        return convert::move_file(&fewer, output);
    }
    let long = i.width.max(i.height) as f64;
    let mut side = (picture_side(i, target) as f64 * 1.2).min(long * 0.9);
    let mut tries = 0;
    while side >= 64.0 {
        if crate::procs::cancelled_here() {
            return Err(crate::procs::CANCELLED.into());
        }
        tries += 1;
        progress((0.5 + tries as f64 * 0.07).min(0.95));
        let (p, size) = make(Some(side.round() as u32), true, &format!("s{tries}"))?;
        if size <= target {
            log::info!("size: {} as a 256-colour PNG at {} px", input.display(), side.round());
            return convert::move_file(&p, output);
        }
        side *= 0.85;
    }
    Err(too_small(Kind::Image, i, target))
}
const QUALITY_LOWEST: u32 = 30;

fn picture_to(input: &Path, target: Option<u64>, output: &Path, progress: &mut dyn FnMut(f64)) -> Result<(), String> {
    let i = info(Kind::Image, input)?;
    let pic = picture_format(&i);
    let Some(fmt) = pic.lossy() else { return png_to(input, &i, target, output, progress) };
    let flatten = || if fmt == Lossy::Jpg { s(&["-background", "white", "-alpha", "remove", "-alpha", "off"]) } else { Vec::new() };
    let tmp = TempDir::new("size")?;
    // 1. The smallest file that still looks the same, at full size.
    let crop = look::search_crop(input, &flatten(), &tmp.0)?;
    let same_q = look::search_quality(&crop, fmt, look::Level::Balanced.picture_target())?;
    progress(0.25);
    let full = prepare(input, &flatten(), None, &tmp.0, "full")?;
    let at = |src: &Path, q: u32, tag: &str| -> Result<(PathBuf, u64), String> {
        let out = tmp.0.join(format!("{tag}-q{q}.{}", format_ext(pic)));
        let mut cmd = tools::command(&tools::require(Tool::Magick)?);
        cmd.arg(src).args(fmt.args(q)).arg(&out);
        convert::exec(Tool::Magick, cmd)?;
        let size = out.metadata().map(|m| m.len()).unwrap_or(u64::MAX);
        Ok((out, size))
    };
    let (same, same_size) = at(&full, same_q, "full")?;
    let target = match target {
        Some(t) if same_size > t => t,
        _ => {
            log::info!("size: {} looks the same at quality {same_q}, {same_size} bytes", input.display());
            return convert::move_file(&same, output);
        }
    };
    // 2. Lower quality, then smaller pictures.
    let long = i.width.max(i.height) as f64;
    let mut side = (picture_side(&i, target) as f64 * 1.25).min(long);
    let floor_q = quality_floor(fmt).min(same_q);
    let mut fallback: Option<PathBuf> = None;
    let mut tries = 0;
    while side >= 64.0 {
        if crate::procs::cancelled_here() {
            return Err(crate::procs::CANCELLED.into());
        }
        tries += 1;
        progress((0.25 + tries as f64 * 0.08).min(0.95));
        let src = if side >= long * 0.995 { full.clone() } else { prepare(input, &flatten(), Some(side.round() as u32), &tmp.0, &format!("s{tries}"))? };
        let tag = format!("t{tries}");
        let (low, low_size) = at(&src, QUALITY_LOWEST, &tag)?;
        if low_size <= target {
            // The highest quality that still fits, by bisection.
            let (mut lo, mut hi) = (QUALITY_LOWEST, same_q);
            let mut best = (low, QUALITY_LOWEST);
            while hi > lo + 2 {
                let mid = (lo + hi) / 2;
                let (p, sz) = at(&src, mid, &tag)?;
                if sz <= target {
                    lo = mid;
                    best = (p, mid);
                } else {
                    hi = mid;
                }
            }
            log::info!("size: {} at {} px, quality {}", input.display(), side.round(), best.1);
            if best.1 >= floor_q || side < MIN_SIDE {
                return convert::move_file(&best.0, output);
            }
            if fallback.is_none() {
                fallback = Some(best.0);
            }
        }
        side *= 0.85;
    }
    match fallback {
        Some(p) => convert::move_file(&p, output),
        None => Err(too_small(Kind::Image, &i, target)),
    }
}

/// The picture, upright (and flattened for JPG), at `side` px on its long
/// side, as a lossless file to encode from.
fn prepare(input: &Path, extra: &[String], side: Option<u32>, dir: &Path, tag: &str) -> Result<PathBuf, String> {
    let out = dir.join(format!("{tag}.miff"));
    let mut src = input.as_os_str().to_os_string();
    src.push("[0]");
    let mut cmd = tools::command(&tools::require(Tool::Magick)?);
    cmd.arg(src).arg("-auto-orient").args(extra);
    if let Some(px) = side {
        cmd.args(["-resize", &format!("{px}x{px}>")]);
    }
    cmd.arg(&out);
    convert::exec(Tool::Magick, cmd)?;
    Ok(out)
}

/// MP3 bitrates LAME can write.
const MP3_RATES: [u32; 14] = [32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320];

fn audio_to(input: &Path, target: Option<u64>, output: &Path, progress: &mut dyn FnMut(f64)) -> Result<(), String> {
    let i = info(Kind::Audio, input)?;
    let ext = output.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let best_k = i.audio_kbps.map(|k| k.min(192)).unwrap_or(192);
    let k = match target {
        Some(t) => {
            let k = audio_kbps_for(&i, t);
            if k < AUDIO_MIN_KBPS {
                return Err(too_small(Kind::Audio, &i, t));
            }
            k.min(best_k)
        }
        None => best_k.min(160),
    };
    let mono = k < 64;
    let mut args = s(&["-vn", "-map", "0:a:0", "-map_metadata", "0"]);
    if ext == "m4a" {
        args.extend(s(&["-c:a", "aac", "-b:a", &format!("{k}k"), "-movflags", "+faststart"]));
    } else {
        // Constant bitrate keeps the size exact; LAME only writes these rates.
        let k = MP3_RATES.iter().copied().filter(|r| *r <= k.max(32)).last().unwrap_or(32);
        args.extend(s(&["-c:a", "libmp3lame", "-b:a", &format!("{k}k"), "-id3v2_version", "3"]));
        if k < 48 {
            args.extend(s(&["-ar", "22050"]));
        } else if k < 64 {
            args.extend(s(&["-ar", "32000"]));
        }
    }
    if mono {
        args.extend(s(&["-ac", "1"]));
    }
    log::info!("size: audio {} at {k} kbps{}", input.display(), if mono { " mono" } else { "" });
    convert::run_ffmpeg(input, &args, output, progress)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(w: u32, h: u32, fps: f64, secs: f64, bytes: u64, screen: bool) -> Info {
        Info { path: "Trip.mp4".into(), bytes, ext: "mp4".into(), width: w, height: h, fps, duration: secs, has_audio: true, audio_codec: Some("aac".into()), audio_kbps: Some(192), screen, ..Info::default() }
    }

    fn photo(w: u32, h: u32, bytes: u64, alpha: bool) -> Info {
        Info { path: "Beach.png".into(), bytes, ext: "png".into(), width: w, height: h, alpha, ..Info::default() }
    }

    #[test]
    fn words_and_tidy_numbers() {
        assert_eq!(words(10 * MB), "10 MB");
        assert_eq!(words(4_500_000), "4.5 MB");
        assert_eq!(words(1_000_000), "1 MB");
        assert_eq!(words(250_000), "250 KB");
        assert_eq!(tidy(45_600_000.0), 46 * MB);
        assert_eq!(tidy(3_300_000.0), 3_500_000);
        assert_eq!(tidy(712_000.0), 700_000);
        assert_eq!(tidy(10_000.0), 50_000);
    }

    #[test]
    fn camera_video_drops_resolution() {
        let v = video(1920, 1080, 30.0, 150.0, 182 * MB, false);
        // Generous: the usual look-the-same compress.
        assert!(video_plan(&v, 70 * MB, None).unwrap().quality_mode);
        let p = video_plan(&v, 25 * MB, None).unwrap();
        assert!(!p.quality_mode);
        assert_eq!((p.height, p.fps as u32), (720, 30));
        let p = video_plan(&v, 10 * MB, None).unwrap();
        assert!(p.height < 720 && p.height >= 360, "{p:?}");
        // The bitrate leaves room for the container.
        let bytes = (p.video_kbps + p.audio_kbps) as f64 * 1000.0 * 150.0 / 8.0;
        assert!(bytes <= 10.0 * MB as f64 * 0.96);
        assert!(video_plan(&v, MB, None).is_none());
        let pv = preview_one(Kind::Video, &v, MB, None);
        assert!(!pv.ok);
        let alt = pv.alt.expect("the start of the video fits");
        assert!(alt.trim >= 3.0 && alt.trim < 20.0, "{alt:?}");
        assert!(video_plan(&v, MB, Some(alt.trim)).is_some());
    }

    #[test]
    fn screen_recordings_drop_frame_rate_first() {
        let v = video(1920, 1080, 30.0, 150.0, 96 * MB, true);
        let p = video_plan(&v, 10 * MB, None).unwrap();
        assert_eq!(p.height, 1080, "{p:?}");
        assert!(p.fps <= 15.0);
        let pv = preview_one(Kind::Video, &v, 10 * MB, None);
        assert!(pv.line2.contains("text stays sharp"), "{pv:?}");
        let tighter = video_plan(&v, 3 * MB, None).unwrap();
        assert!(tighter.height < 1080);
    }

    #[test]
    fn pictures_lower_quality_then_size() {
        let p = photo(4032, 3024, 14_200_000, false);
        assert_eq!(picture_format(&p), Pic::Jpg);
        let full = preview_one(Kind::Image, &p, 5 * MB, None);
        assert_eq!(full.short, "Full quality");
        assert!(full.line1.starts_with("PNG → JPG"));
        let small = preview_one(Kind::Image, &p, 300 * KB, None);
        assert!(small.ok && small.short.ends_with(" px"), "{small:?}");
        assert!(!preview_one(Kind::Image, &p, 2 * KB, None).ok);
        assert!(preview_one(Kind::Image, &p, 20 * MB, None).fits);
        // Transparent pictures stay PNG: WebP isn't taken everywhere.
        let clear = photo(800, 800, MB, true);
        assert_eq!(picture_format(&clear), Pic::Png);
        assert!(preview_one(Kind::Image, &clear, 300 * KB, None).line2.contains("PNG"));
    }

    #[test]
    fn together_shares_by_need_and_skips_small_files() {
        let infos = vec![photo(4000, 3000, 14 * MB, false), photo(4000, 3000, 13 * MB, false), photo(300, 200, 40 * KB, false)];
        let sh = shares(Kind::Image, &infos, 6 * MB);
        assert_eq!(sh[2], None, "the small one already fits");
        let total: u64 = sh.iter().flatten().sum::<u64>() + 40 * KB;
        assert!(total <= 6 * MB);
        let pv = preview(Kind::Image, &infos, 6 * MB, true, None);
        assert!(pv.ok && pv.line2.contains("1 already small enough"), "{pv:?}");
    }

    #[test]
    fn presets_fit_the_file() {
        let v = video(1920, 1080, 30.0, 150.0, 182 * MB, false);
        let ps = presets(Kind::Video, std::slice::from_ref(&v), false);
        assert_eq!(ps.len(), 5, "{ps:?}");
        assert!(ps.windows(2).all(|w| w[0].bytes > w[1].bytes));
        assert!(ps.iter().any(|p| p.name == "Discord"));
        assert!(ps.iter().all(|p| p.preview.ok && !p.preview.fits));
        let small = photo(1200, 900, 300 * KB, false);
        let ps = presets(Kind::Image, std::slice::from_ref(&small), false);
        assert!(ps.iter().all(|p| p.bytes < 300 * KB), "{ps:?}");
    }

    #[test]
    fn audio_bitrate_from_size() {
        let a = Info { path: "Talk.wav".into(), bytes: 50 * MB, ext: "wav".into(), duration: 300.0, has_audio: true, ..Info::default() };
        let p = preview_one(Kind::Audio, &a, 2 * MB, None);
        assert!(p.ok && p.line1.contains("mono"), "{p:?}");
        assert!(!preview_one(Kind::Audio, &a, 500 * KB, None).ok);
        assert_eq!(audio_out_ext(&a), "mp3");
    }

    // ---------- real conversions, when the converters are installed ----------

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("convertino-size-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn run_plan(kind: Kind, file: &Path, ask: Ask) -> PathBuf {
        let planned = plan(kind, &[file.to_path_buf()], Some(ask)).unwrap();
        convert::run(&planned.steps[0], &mut |_| {}).unwrap()
    }

    #[test]
    fn real_picture_to_size() {
        let Some(im) = tools::find(Tool::Magick) else { return };
        let d = tmpdir("pic");
        let src = d.join("photo.png");
        assert!(tools::command(&im).args(["-size", "1600x1200", "plasma:", "-blur", "0x0.6"]).arg(&src).status().unwrap().success());
        let before = src.metadata().unwrap().len();
        for target in [150 * KB, 40 * KB] {
            let out = run_plan(Kind::Image, &src, Ask { bytes: target, together: false, trim: None });
            let size = out.metadata().unwrap().len();
            assert!(size <= target, "{size} > {target}");
            assert!(size < before);
            assert_eq!(out.extension().unwrap(), "jpg");
        }
        // Generous: just the look-the-same file.
        let out = run_plan(Kind::Image, &src, Ask { bytes: before - 1, together: false, trim: None });
        assert!(out.metadata().unwrap().len() < before / 2);

        // An alpha channel nobody uses still becomes a JPG.
        let opaque = d.join("opaque.png");
        assert!(tools::command(&im).arg(&src).args(["-alpha", "set"]).arg(&opaque).status().unwrap().success());
        let out = run_plan(Kind::Image, &opaque, Ask { bytes: 150 * KB, together: false, trim: None });
        assert_eq!(out.extension().unwrap(), "jpg");
        // Real transparency stays PNG, never WebP.
        let clear = d.join("cutout.png");
        assert!(tools::command(&im)
            .args(["-size", "1200x900", "plasma:", "(", "-size", "1200x900", "xc:black", "-fill", "white", "-draw", "circle 600,450 600,60", ")", "-alpha", "off", "-compose", "CopyOpacity", "-composite"])
            .arg(&clear)
            .status()
            .unwrap()
            .success());
        let big = clear.metadata().unwrap().len();
        let target = big / 4;
        let out = run_plan(Kind::Image, &clear, Ask { bytes: target, together: false, trim: None });
        assert_eq!(out.extension().unwrap(), "png");
        assert!(out.metadata().unwrap().len() <= target, "{} > {target}", out.metadata().unwrap().len());
    }

    #[test]
    fn real_video_and_audio_to_size() {
        let Some(ff) = tools::find(Tool::Ffmpeg) else { return };
        if video::working(video::Codec::Hevc).is_empty() && video::working(video::Codec::H264).is_empty() {
            return;
        }
        let d = tmpdir("vid");
        let src = d.join("clip.mp4");
        assert!(tools::command(&ff)
            .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "mandelbrot=s=1280x720:r=30", "-f", "lavfi", "-i", "sine=f=440:r=48000", "-t", "8", "-c:v", "libx264", "-crf", "12", "-c:a", "aac", "-b:a", "192k", "-shortest"])
            .arg(&src)
            .status()
            .unwrap()
            .success());
        let before = src.metadata().unwrap().len();
        let target = before / 6;
        let out = run_plan(Kind::Video, &src, Ask { bytes: target, together: false, trim: None });
        let size = out.metadata().unwrap().len();
        assert!(size <= target, "{size} > {target} (source {before})");
        assert!(size as f64 > target as f64 * 0.5, "{size} is far under {target}");

        // A moving picture isn't a screen recording; a still one is.
        assert!(!video::looks_like_screen(&src, &video::probe(&src).unwrap()));
        let still = d.join("still.mp4");
        assert!(tools::command(&ff)
            .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "smptebars=s=1280x720:r=30", "-t", "6", "-c:v", "libx264"])
            .arg(&still)
            .status()
            .unwrap()
            .success());
        assert!(video::looks_like_screen(&still, &video::probe(&still).unwrap()));

        let wav = d.join("tone.wav");
        assert!(tools::command(&ff).args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "sine=f=330:r=44100", "-t", "20"]).arg(&wav).status().unwrap().success());
        let target = 120 * KB;
        let out = run_plan(Kind::Audio, &wav, Ask { bytes: target, together: false, trim: None });
        assert!(out.metadata().unwrap().len() <= target);
        assert_eq!(out.extension().unwrap(), "mp3");
    }

    #[test]
    fn real_pdf_to_size() {
        let (Some(im), Some(_)) = (tools::find(Tool::Magick), tools::find(Tool::Ghostscript)) else { return };
        let d = tmpdir("pdf");
        let pic = d.join("p.png");
        assert!(tools::command(&im).args(["-size", "2400x3200", "plasma:"]).arg(&pic).status().unwrap().success());
        let pdf = d.join("scan.pdf");
        assert!(tools::command(&im).arg(&pic).args(["-quality", "95", "-density", "300"]).arg(&pdf).status().unwrap().success());
        let before = pdf.metadata().unwrap().len();
        let target = before / 4;
        let out = run_plan(Kind::Pdf, &pdf, Ask { bytes: target, together: false, trim: None });
        assert!(out.metadata().unwrap().len() <= target, "{} > {target}", out.metadata().unwrap().len());
    }

    #[test]
    fn small_files_are_left_alone() {
        let d = tmpdir("fits");
        let f = d.join("tiny.pdf");
        std::fs::write(&f, b"%PDF-1.4 tiny").unwrap();
        let e = plan(Kind::Pdf, &[f], Some(Ask { bytes: MB, together: false, trim: None })).err().unwrap();
        assert!(e.starts_with(ALREADY));
    }
}
