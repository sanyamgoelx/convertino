//! Video conversions with FFmpeg.
//!
//! Decisions are made when the job runs, from what the file contains:
//! streams that already fit the new container are copied (instant, no
//! quality loss), everything else is encoded with the best encoder this
//! machine actually has. The LGPL FFmpeg build has no x264/x265, so the
//! graphics card's encoder (NVIDIA, Intel, AMD, Apple) or Windows' own Media
//! Foundation encoder is used instead; a GPL build with x264/x265 is used
//! as-is if one is installed.

use crate::convert::{last_lines, run_ffmpeg, s};
use crate::tools::{self, Tool};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Job {
    Mp4,
    Mov,
    Webm,
    Mkv,
    Gif,
    Compress,
    To720p,
    /// One PNG per second into a folder.
    Frames,
}

/// What `ffmpeg -i` reports about a file.
#[derive(Debug, Default, Clone)]
pub struct Probe {
    pub video: Option<String>,
    pub audio: Option<String>,
    pub width: u32,
    pub height: u32,
    /// Seconds, when FFmpeg reports it.
    pub duration: Option<f64>,
    /// The audio stream's bitrate, when FFmpeg reports it.
    pub audio_kbps: Option<u32>,
}

/// Choices from Settings (or Shift+click on the wheel) for one video job.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Opts {
    /// MP4, MOV and 720p when the video has to be re-encoded.
    quality: Quality,
    /// Search for the smallest file that still looks the same ("Pictures" in
    /// Settings is not "fixed").
    tune: bool,
    gif_width: u32,
    /// GIFs of long videos get enormous: only the start is used.
    gif_seconds: u32,
}

impl Default for Opts {
    fn default() -> Self {
        Opts::from_quality(&crate::settings::Quality::default())
    }
}

impl Opts {
    pub fn from_quality(q: &crate::settings::Quality) -> Self {
        let quality = match q.video.as_str() {
            "small" => Quality::Small,
            "best" => Quality::Best,
            _ => Quality::High,
        };
        Opts { quality, tune: q.image != "fixed", gif_width: q.gif_width, gif_seconds: q.gif_seconds }
    }
}
/// Frames: one per second, but never more than this many in all.
pub const MAX_FRAMES: f64 = 600.0;

/// Reads the first video and audio stream from FFmpeg's description of the file.
pub fn probe(input: &Path) -> Result<Probe, String> {
    let ff = tools::require(Tool::Ffmpeg)?;
    let out = tools::command(&ff)
        .args(["-hide_banner", "-nostdin", "-i"])
        .arg(input)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("Couldn't start FFmpeg: {e}"))?;
    // With no output file FFmpeg always exits with an error; the description is on stderr.
    let text = String::from_utf8_lossy(&out.stderr);
    let p = parse_probe(&text);
    if p.video.is_none() && p.audio.is_none() {
        return Err(format!("FFmpeg couldn't read this video: {}", last_lines(&text)));
    }
    Ok(p)
}

fn parse_probe(text: &str) -> Probe {
    let mut p = Probe::default();
    for line in text.lines().map(str::trim) {
        if p.duration.is_none() {
            p.duration = crate::convert::parse_duration(line);
        }
        if !line.starts_with("Stream #") {
            continue;
        }
        if let Some(rest) = line.split(": Video: ").nth(1) {
            // Cover art inside audio files is a "video" stream too.
            if p.video.is_some() || line.contains("(attached pic)") {
                continue;
            }
            p.video = rest.split([' ', ',']).next().map(str::to_string);
            for token in rest.split([' ', ',']) {
                if let Some((w, h)) = token.split_once('x') {
                    if let (Ok(w), Ok(h)) = (w.parse::<u32>(), h.parse::<u32>()) {
                        p.width = w;
                        p.height = h;
                        break;
                    }
                }
            }
        } else if let Some(rest) = line.split(": Audio: ").nth(1) {
            if p.audio.is_none() {
                p.audio = rest.split([' ', ',']).next().map(str::to_string);
                p.audio_kbps = rest
                    .split(',')
                    .map(str::trim)
                    .find_map(|part| part.strip_suffix(" kb/s").and_then(|n| n.trim().parse().ok()));
            }
        }
    }
    p
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Codec {
    H264,
    Hevc,
    Vp9,
}

impl Codec {
    fn key(self) -> &'static str {
        match self {
            Codec::H264 => "h264",
            Codec::Hevc => "hevc",
            Codec::Vp9 => "vp9",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Quality {
    /// As close to the source as practical; bigger files.
    Best,
    /// Visually close to the source ("Balanced").
    High,
    /// Noticeably smaller files.
    Small,
}

impl Quality {
    fn level(self) -> crate::look::Level {
        match self {
            Quality::Best => crate::look::Level::Best,
            Quality::High => crate::look::Level::Balanced,
            Quality::Small => crate::look::Level::Small,
        }
    }
}

fn candidates(codec: Codec) -> &'static [&'static str] {
    match codec {
        Codec::H264 => &["libx264", "h264_nvenc", "h264_qsv", "h264_amf", "h264_videotoolbox", "h264_mf", "libopenh264"],
        Codec::Hevc => &["libx265", "hevc_nvenc", "hevc_qsv", "hevc_amf", "hevc_videotoolbox", "hevc_mf"],
        Codec::Vp9 => &["libvpx-vp9"],
    }
}

fn is_software(enc: &str) -> bool {
    enc.starts_with("lib")
}

/// Hardware encoders are listed even without the hardware, so each one is
/// tried on a tiny clip once; the ones that work are remembered, best first.
fn working_map() -> &'static Mutex<HashMap<Codec, Vec<&'static str>>> {
    static WORKING: OnceLock<Mutex<HashMap<Codec, Vec<&'static str>>>> = OnceLock::new();
    WORKING.get_or_init(|| Mutex::new(HashMap::new()))
}

/// After FFmpeg is replaced (another build), the encoders are tried again.
pub fn forget_encoders() {
    if let Ok(mut m) = working_map().lock() {
        m.clear();
    }
}

/// The H.264 encoder in use, in words, for Settings.
pub fn encoder_in_use() -> String {
    let name = |e: &str| match e {
        "libx264" => "x264 (software, best quality for the size)",
        "libx265" => "x265 (software, best quality for the size)",
        "h264_nvenc" | "hevc_nvenc" => "NVIDIA graphics card",
        "h264_qsv" | "hevc_qsv" => "Intel graphics",
        "h264_amf" | "hevc_amf" => "AMD graphics card",
        "h264_videotoolbox" | "hevc_videotoolbox" => "Apple VideoToolbox",
        "h264_mf" | "hevc_mf" => "Windows Media Foundation",
        "libopenh264" => "OpenH264 (software)",
        other => other,
    }
    .to_string();
    let h264 = working(Codec::H264);
    if h264.is_empty() {
        return if tools::find(Tool::Ffmpeg).is_none() { "Not yet: FFmpeg downloads with your first video" } else { "None found" }.into();
    }
    let hw = h264.iter().find(|e| !is_software(e));
    match (h264.first(), hw) {
        (Some(first), Some(hw)) if first != hw => format!("{}; {} when it's as good and faster", name(first), name(hw)),
        (Some(first), _) => name(first),
        _ => "None found".into(),
    }
}

/// The first working encoder for `codec` (the one used without measuring).
pub fn encoder(codec: Codec) -> Option<&'static str> {
    working(codec).first().copied()
}

/// Every encoder for `codec` that works on this machine, best quality per size first.
pub fn working(codec: Codec) -> Vec<&'static str> {
    let map = working_map();
    if let Some(hit) = map.lock().ok().and_then(|m| m.get(&codec).cloned()) {
        return hit;
    }
    let Some(ff) = tools::find(Tool::Ffmpeg) else { return Vec::new() };
    let listed = tools::command(&ff)
        .args(["-hide_banner", "-encoders"])
        .stdin(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let has = |name: &str| listed.lines().any(|l| l.split_whitespace().nth(1) == Some(name));
    let found: Vec<&'static str> = candidates(codec)
        .iter()
        .copied()
        .filter(|e| has(e))
        .filter(|e| {
            let ok = tools::command(&ff)
                .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-f", "lavfi", "-i"])
                .arg("color=c=gray:s=640x360:r=30:d=0.3")
                .args(encoder_args(e, Quality::High, 640, 360))
                .args(["-f", "null", "-"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            log::info!("video encoder {e}: {}", if ok { "works" } else { "not usable here" });
            ok
        })
        .collect();
    if let Ok(mut m) = map.lock() {
        m.insert(codec, found.clone());
    }
    found
}

/// A bitrate for encoders without a constant-quality mode.
fn bitrate(width: u32, height: u32, q: Quality) -> String {
    let pixels = (width.max(1) * height.max(1)) as f64;
    let full_hd = 1920.0 * 1080.0;
    let mbps = match q {
        Quality::Best => 12.0,
        Quality::High => 8.0,
        Quality::Small => 3.0,
    } * (pixels / full_hd).powf(0.75);
    format!("{}k", ((mbps * 1000.0) as u32).max(400))
}

/// An encoder's constant-quality setting: (lowest = best looking, highest =
/// smallest). None for encoders that only take a bitrate.
fn knob_range(enc: &str) -> Option<(u32, u32)> {
    match enc {
        "libx264" => Some((14, 32)),
        "libx265" => Some((16, 34)),
        "h264_nvenc" => Some((16, 36)),
        "hevc_nvenc" => Some((18, 40)),
        "h264_qsv" | "hevc_qsv" | "h264_amf" | "hevc_amf" => Some((16, 38)),
        "libvpx-vp9" => Some((20, 48)),
        _ => None,
    }
}

/// The value used without measuring, per level: Best, High (Balanced), Small.
fn default_knob(enc: &str, q: Quality) -> u32 {
    let pick = |best: u32, high: u32, small: u32| match q {
        Quality::Best => best,
        Quality::High => high,
        Quality::Small => small,
    };
    match enc {
        "libx264" => pick(18, 20, 26),
        "libx265" => pick(20, 22, 28),
        "h264_nvenc" => pick(19, 21, 28),
        "hevc_nvenc" => pick(22, 24, 31),
        "libvpx-vp9" => pick(28, 32, 38),
        _ => pick(20, 22, 29),
    }
}

fn encoder_args(enc: &str, q: Quality, width: u32, height: u32) -> Vec<String> {
    encoder_args_at(enc, default_knob(enc, q), q, width, height)
}

/// Encoder settings with the constant-quality setting at `v`.
fn encoder_args_at(enc: &str, v: u32, q: Quality, width: u32, height: u32) -> Vec<String> {
    let v = v.to_string();
    let mut a = s(&["-c:v", enc]);
    match enc {
        "libx264" => a.extend(s(&["-preset", "medium", "-crf", &v, "-pix_fmt", "yuv420p"])),
        "libx265" => a.extend(s(&["-preset", "medium", "-crf", &v, "-pix_fmt", "yuv420p", "-x265-params", "log-level=error"])),
        "h264_nvenc" | "hevc_nvenc" => {
            a.extend(s(&["-preset", "p5", "-tune", "hq", "-rc", "vbr", "-cq", &v, "-b:v", "0", "-spatial-aq", "1", "-pix_fmt", "yuv420p"]));
        }
        "h264_qsv" | "hevc_qsv" => a.extend(s(&["-preset", "medium", "-global_quality", &v, "-pix_fmt", "nv12"])),
        "h264_amf" | "hevc_amf" => {
            a.extend(s(&["-quality", "balanced", "-rc", "cqp", "-qp_i", &v, "-qp_p", &v, "-pix_fmt", "yuv420p"]));
            if enc == "h264_amf" {
                a.extend(s(&["-qp_b", &v]));
            }
        }
        "libvpx-vp9" => a.extend(s(&[
            "-crf", &v, "-b:v", "0", "-row-mt", "1", "-deadline", "good", "-cpu-used", "4", "-pix_fmt", "yuv420p",
        ])),
        // VideoToolbox, Media Foundation (built into Windows) and OpenH264: bitrate only.
        _ => a.extend(s(&["-b:v", &bitrate(width, height, q), "-pix_fmt", "yuv420p"])),
    }
    if enc.starts_with("hevc") || enc == "libx265" {
        // Lets QuickTime, iPhones and Macs play H.265 in MP4/MOV.
        a.extend(s(&["-tag:v", "hvc1"]));
    }
    a
}

/// What a job encodes to, and at which encoder and setting.
#[derive(Debug, Clone, PartialEq)]
pub struct Tuned {
    pub encoder: &'static str,
    pub value: u32,
}

fn video_encode(codec: Codec, q: Quality, p: &Probe, tuned: Option<&Tuned>) -> Result<Vec<String>, String> {
    if let Some(t) = tuned {
        return Ok(encoder_args_at(t.encoder, t.value, q, p.width, p.height));
    }
    let enc = encoder(codec)
        .or_else(|| if codec == Codec::Hevc { encoder(Codec::H264) } else { None })
        .ok_or("This FFmpeg has no working H.264 encoder.")?;
    Ok(encoder_args(enc, q, p.width, p.height))
}

/// Audio settings: copy what already fits, otherwise AAC (or Opus for WebM).
fn audio_args(p: &Probe, fits: &[&str], fallback: &[&str]) -> Vec<String> {
    match p.audio.as_deref() {
        None => s(&["-an"]),
        Some(a) if fits.contains(&a) => s(&["-c:a", "copy"]),
        Some(_) => s(fallback),
    }
}

const MP4_VIDEO: &[&str] = &["h264", "hevc", "av1"];
const MP4_AUDIO: &[&str] = &["aac", "mp3", "alac", "opus", "ac3", "eac3"];
const MOV_VIDEO: &[&str] = &["h264", "hevc", "prores", "mjpeg"];
const MOV_AUDIO: &[&str] = &["aac", "mp3", "alac", "pcm_s16le", "pcm_s24le", "pcm_s16be", "pcm_s24be"];
const WEBM_VIDEO: &[&str] = &["vp8", "vp9", "av1"];
const WEBM_AUDIO: &[&str] = &["opus", "vorbis"];

/// FFmpeg arguments (after `-i input`) for a job, or a reason it can't run.
#[cfg_attr(not(test), allow(dead_code))]
pub fn args_for(job: Job, p: &Probe, opts: &Opts) -> Result<Vec<String>, String> {
    args_with(job, p, opts, None)
}

/// The video encode a job needs (codec, and the height it scales to), or None
/// when it copies streams or isn't an ordinary encode (GIF, frames, MKV).
fn encode_of(job: Job, p: &Probe) -> Option<(Codec, Option<u32>)> {
    let v = p.video.as_deref()?;
    match job {
        Job::Mp4 if !MP4_VIDEO.contains(&v) => Some((Codec::H264, None)),
        Job::Mov if !MOV_VIDEO.contains(&v) => Some((Codec::H264, None)),
        Job::Webm if !WEBM_VIDEO.contains(&v) => Some((Codec::Vp9, None)),
        Job::Compress => Some((Codec::Hevc, None)),
        Job::To720p if p.height > 720 => Some((Codec::H264, Some(720))),
        _ => None,
    }
}

fn args_with(job: Job, p: &Probe, opts: &Opts, tuned: Option<&Tuned>) -> Result<Vec<String>, String> {
    let v = p.video.as_deref().ok_or("This file has no video in it.")?;
    let tag_hevc = |mut a: Vec<String>| {
        if v == "hevc" {
            a.extend(s(&["-tag:v", "hvc1"]));
        }
        a
    };
    let aac = |kbps: &str| vec!["-c:a".to_string(), "aac".into(), "-b:a".into(), format!("{kbps}k")];
    let mut a = s(&["-map", "0:v:0", "-map", "0:a:0?", "-map_metadata", "0"]);
    match job {
        Job::Mp4 | Job::Mov => {
            let (vids, auds) = if job == Job::Mp4 { (MP4_VIDEO, MP4_AUDIO) } else { (MOV_VIDEO, MOV_AUDIO) };
            if vids.contains(&v) {
                a = tag_hevc([a, s(&["-c:v", "copy"])].concat());
            } else {
                a.extend(video_encode(Codec::H264, opts.quality, p, tuned)?);
            }
            a.extend(audio_args(p, auds, &["-c:a", "aac", "-b:a", "192k"]));
            a.extend(s(&["-movflags", "+faststart"]));
        }
        Job::Webm => {
            if WEBM_VIDEO.contains(&v) {
                a.extend(s(&["-c:v", "copy"]));
            } else {
                let t = tuned.cloned().unwrap_or(Tuned { encoder: "libvpx-vp9", value: default_knob("libvpx-vp9", opts.quality) });
                a.extend(encoder_args_at(t.encoder, t.value, opts.quality, p.width, p.height));
            }
            a.extend(audio_args(p, WEBM_AUDIO, &["-c:a", "libopus", "-b:a", "128k"]));
        }
        Job::Mkv => {
            // Every stream, untouched: MKV holds almost anything.
            a = s(&["-map", "0", "-c", "copy", "-map_metadata", "0"]);
        }
        Job::Gif => {
            let secs = opts.gif_seconds;
            a = if p.duration.unwrap_or(0.0) > secs as f64 { s(&["-t", &secs.to_string()]) } else { Vec::new() };
            let filter = format!(
                "fps=15,scale='min({},iw)':-1:flags=lanczos,split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=4",
                opts.gif_width
            );
            a.extend(s(&["-vf", &filter, "-loop", "0", "-an"]));
        }
        Job::Compress => {
            a.extend(video_encode(Codec::Hevc, opts.quality, p, tuned)?);
            // Already-small AAC is kept as it is: re-encoding it only loses quality.
            match (p.audio.as_deref(), p.audio_kbps) {
                (None, _) => a.extend(s(&["-an"])),
                (Some("aac"), Some(k)) if k <= 160 => a.extend(s(&["-c:a", "copy"])),
                _ => a.extend(aac("128")),
            }
            a.extend(s(&["-movflags", "+faststart"]));
        }
        Job::To720p => {
            if p.height > 0 && p.height <= 720 {
                return Err("This video is already 720p or smaller.".into());
            }
            a.extend(s(&["-vf", "scale=-2:720:flags=lanczos"]));
            let mut small = p.clone();
            small.width = p.width * 720 / p.height.max(1);
            small.height = 720;
            a.extend(video_encode(Codec::H264, opts.quality, &small, tuned)?);
            a.extend(aac("160"));
            a.extend(s(&["-movflags", "+faststart"]));
        }
        Job::Frames => {
            // A long film would make thousands of pictures: space them out instead.
            let every = (p.duration.unwrap_or(0.0) / MAX_FRAMES).ceil().max(1.0) as u32;
            let fps = if every > 1 { format!("fps=1/{every}") } else { "fps=1".to_string() };
            a = vec!["-map".into(), "0:v:0".into(), "-vf".into(), fps, "-an".into()];
        }
    }
    Ok(a)
}

// ---------------------------------------------------------------------------
// The smallest file that still looks the same.
//
// A few short samples of the video are encoded at different settings and
// compared with the source using VMAF (Netflix's measure of how alike two
// videos look; 93+ is very hard to tell apart). The highest setting (smallest
// file) that still reaches the level from Settings is used for the whole video.
// When both a software encoder (x264/x265) and the graphics card's encoder
// work, both are measured now and then and the better trade-off is kept in
// encoder-choice.json.

/// How alike two videos look.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Metric {
    Vmaf,
    /// FFmpeg builds without libvmaf: SSIM, scaled so the targets line up.
    Ssim,
}

fn metric() -> Metric {
    static M: OnceLock<Mutex<Option<Metric>>> = OnceLock::new();
    let cell = M.get_or_init(|| Mutex::new(None));
    if let Some(m) = cell.lock().ok().and_then(|g| *g) {
        return m;
    }
    let has_vmaf = tools::find(Tool::Ffmpeg)
        .and_then(|ff| tools::command(&ff).args(["-hide_banner", "-filters"]).stdin(Stdio::null()).output().ok())
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().any(|l| l.split_whitespace().nth(1) == Some("libvmaf")))
        .unwrap_or(false);
    let m = if has_vmaf { Metric::Vmaf } else { Metric::Ssim };
    log::info!("video look measure: {m:?}");
    if let Ok(mut g) = cell.lock() {
        *g = Some(m);
    }
    m
}

/// SSIM (0–1) on VMAF's 0–100 scale, roughly: 0.99 → 96, 0.98 → 92, 0.97 → 88.
fn ssim_as_vmaf(ssim: f64) -> f64 {
    (100.0 - (1.0 - ssim) * 400.0).clamp(0.0, 100.0)
}

/// Where the samples come from: (start, length) in seconds.
fn sample_spans(duration: Option<f64>) -> Vec<(f64, f64)> {
    match duration {
        Some(d) if d > 18.0 => [0.15, 0.5, 0.8].iter().map(|f| ((d * f - 1.0).max(0.0), 2.0)).collect(),
        // Short clips: 4 seconds from the middle is plenty.
        Some(d) if d > 4.0 => vec![(d / 2.0 - 2.0, 4.0)],
        Some(d) if d > 0.0 => vec![(0.0, d)],
        _ => vec![(0.0, 4.0)],
    }
}

/// The filter that brings a picture to the size it's compared at: the job's
/// size (720p), then at most 1080 lines high (VMAF's model is for 1080p, and
/// it's much faster).
fn compare_filter(scale_to: Option<u32>, height: u32) -> String {
    let h = scale_to.unwrap_or(height).max(2);
    let mut f = String::new();
    if let Some(h) = scale_to {
        f.push_str(&format!("scale=-2:{h}:flags=lanczos,"));
    }
    if h > 1080 {
        f.push_str("scale=-2:1080:flags=bicubic,");
    }
    f.push_str("format=yuv420p,setpts=PTS-STARTPTS");
    f
}

/// One measurement: the samples encoded with `enc` at `value`.
struct Trial {
    encoder: &'static str,
    value: u32,
    score: f64,
    bytes: u64,
    secs: f64,
}

#[allow(clippy::too_many_arguments)]
fn trial(input: &Path, p: &Probe, enc: &'static str, value: u32, q: Quality, scale_to: Option<u32>, spans: &[(f64, f64)], dir: &Path) -> Result<Trial, String> {
    let ff = tools::require(Tool::Ffmpeg)?;
    let (w, h) = match scale_to {
        Some(sh) => (p.width * sh / p.height.max(1), sh),
        None => (p.width, p.height),
    };
    let mut bytes = 0;
    let mut scores = Vec::new();
    let mut encode_secs = 0.0;
    for (i, (start, len)) in spans.iter().enumerate() {
        if crate::procs::cancelled_here() {
            return Err(crate::procs::CANCELLED.into());
        }
        let out = dir.join(format!("t{i}-{value}.mp4"));
        let _ = std::fs::remove_file(&out);
        let mut cmd = tools::command(&ff);
        cmd.args(["-hide_banner", "-nostdin", "-y", "-loglevel", "error", "-ss", &format!("{start:.3}"), "-t", &format!("{len:.3}"), "-i"])
            .arg(input)
            .args(["-map", "0:v:0", "-an"]);
        if let Some(sh) = scale_to {
            cmd.args(["-vf", &format!("scale=-2:{sh}:flags=lanczos")]);
        }
        cmd.args(encoder_args_at(enc, value, q, w, h)).arg(&out);
        let t0 = std::time::Instant::now();
        crate::convert::exec(Tool::Ffmpeg, cmd)?;
        encode_secs += t0.elapsed().as_secs_f64();
        bytes += out.metadata().map(|m| m.len()).unwrap_or(0);

        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        let norm_d = compare_filter(None, h);
        let norm_r = compare_filter(scale_to, p.height);
        let measure = match metric() {
            Metric::Vmaf => format!("libvmaf=n_threads={threads}:n_subsample=2"),
            Metric::Ssim => "ssim".to_string(),
        };
        let graph = format!("[0:v]{norm_d}[d];[1:v]{norm_r}[r];[d][r]{measure}");
        let mut m = tools::command(&ff);
        m.args(["-hide_banner", "-nostdin", "-i"])
            .arg(&out)
            .args(["-ss", &format!("{start:.3}"), "-t", &format!("{len:.3}"), "-i"])
            .arg(input)
            .args(["-lavfi", &graph, "-f", "null", "-"]);
        let o = crate::procs::output(&mut m, "FFmpeg", None)?;
        let text = String::from_utf8_lossy(&o.stderr);
        let sc = parse_score(&text).ok_or_else(|| format!("couldn't measure the sample: {}", last_lines(&text)))?;
        scores.push(sc);
        let _ = std::fs::remove_file(&out);
    }
    // The average, but one bad sample (a dark or busy scene) pulls it down.
    let mean = scores.iter().sum::<f64>() / scores.len().max(1) as f64;
    let worst = scores.iter().copied().fold(100.0, f64::min);
    let score = mean.min(worst + 3.0);
    log::info!("video look: {enc} at {value}: {score:.2} ({scores:?}), {bytes} bytes, {encode_secs:.1}s");
    Ok(Trial { encoder: enc, value, score, bytes, secs: encode_secs })
}

fn parse_score(text: &str) -> Option<f64> {
    if let Some(rest) = text.split("VMAF score:").nth(1) {
        return rest.split_whitespace().next()?.parse().ok();
    }
    let rest = text.rsplit(" All:").next().filter(|_| text.contains(" All:"))?;
    rest.split_whitespace().next()?.parse().ok().map(ssim_as_vmaf)
}

/// The highest setting of `enc` that still reaches `target`, found with a few
/// trials (a secant search: the look changes about evenly with the setting).
/// Returns the trial at that setting, or the best-looking one tried.
#[allow(clippy::too_many_arguments)]
fn search(
    input: &Path,
    p: &Probe,
    enc: &'static str,
    q: Quality,
    target: f64,
    scale_to: Option<u32>,
    spans: &[(f64, f64)],
    dir: &Path,
    progress: &mut dyn FnMut(f64),
) -> Result<Trial, String> {
    let (lo, hi) = knob_range(enc).ok_or("no quality setting")?;
    let mut tried: Vec<Trial> = Vec::new();
    let mut v = default_knob(enc, q).clamp(lo, hi);
    for round in 0..MAX_TRIALS {
        if tried.iter().any(|t| t.value == v) {
            break;
        }
        tried.push(trial(input, p, enc, v, q, scale_to, spans, dir)?);
        progress((round + 1) as f64 / MAX_TRIALS as f64);
        let pass = tried.iter().filter(|t| t.score >= target).max_by_key(|t| t.value);
        let fail = tried.iter().filter(|t| t.score < target).min_by_key(|t| t.value);
        v = match (pass, fail) {
            (Some(a), Some(b)) => {
                if b.value <= a.value + 1 {
                    break;
                }
                let f = ((a.score - target) / (a.score - b.score).max(0.01)).clamp(0.0, 1.0);
                let guess = a.value as f64 + f * (b.value - a.value) as f64;
                (guess.floor() as u32).clamp(a.value + 1, b.value - 1)
            }
            (Some(a), None) => {
                if a.value >= hi {
                    break;
                }
                let step = ((a.score - target) / 1.2).round().max(1.0) as u32;
                (a.value + step).min(hi)
            }
            (None, Some(b)) => {
                if b.value <= lo {
                    break;
                }
                let step = ((target - b.score) / 1.2).round().max(1.0) as u32;
                b.value.saturating_sub(step).max(lo)
            }
            (None, None) => break,
        };
    }
    let best = match tried.iter().filter(|t| t.score >= target).max_by_key(|t| t.value) {
        Some(t) => t.value,
        None => tried.iter().map(|t| t.value).min().unwrap_or(lo),
    };
    let i = tried.iter().position(|t| t.value == best).unwrap_or(0);
    Ok(tried.swap_remove(i))
}

/// Settings tried per encoder: about one per second of waiting on a fast PC.
const MAX_TRIALS: usize = 3;

/// encoder-choice.json: which encoder won the last measurement, per codec and size.
fn choice_path() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("encoder-choice.json"))
}

/// Measure again after this many uses of a remembered choice.
const RECHECK_AFTER: u64 = 10;

fn remembered(key: &str) -> Option<(String, u64)> {
    let text = std::fs::read_to_string(choice_path()?).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let e = v.get(key)?;
    Some((e.get("encoder")?.as_str()?.to_string(), e.get("uses")?.as_u64()?))
}

fn remember(key: &str, encoder: &str, uses: u64) {
    let Some(path) = choice_path() else { return };
    let mut v: serde_json::Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .filter(|v: &serde_json::Value| v.is_object())
        .unwrap_or_else(|| serde_json::json!({}));
    v[key] = serde_json::json!({ "encoder": encoder, "uses": uses });
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, serde_json::to_string_pretty(&v).unwrap_or_default());
}

/// Software or graphics card: the software encoder makes smaller files at the
/// same look, the card is much faster. The card wins unless the software
/// result is clearly smaller (and not unbearably slow).
fn prefer_hardware(sw: &Trial, hw: &Trial, duration: f64, sampled: f64, q: Quality) -> bool {
    let margin = if q == Quality::Best { 1.08 } else { 1.15 };
    if hw.bytes as f64 <= sw.bytes as f64 * margin {
        return true;
    }
    // x265 on a long 4K video can take far longer than the video itself.
    let projected = sw.secs / sampled.max(0.1) * duration;
    projected > (duration * 3.0).max(120.0) && hw.bytes as f64 <= sw.bytes as f64 * 1.35
}

/// Encoder and setting for a job, measured on samples. None: nothing to
/// measure with (bitrate-only encoders), use the defaults.
fn tune(input: &Path, p: &Probe, codec: Codec, q: Quality, scale_to: Option<u32>, progress: &mut dyn FnMut(f64)) -> Result<Option<(Tuned, f64)>, String> {
    let mut list = working(codec);
    if list.is_empty() && codec == Codec::Hevc {
        list = working(Codec::H264);
    }
    let list: Vec<&'static str> = list.into_iter().filter(|e| knob_range(e).is_some()).collect();
    let Some(&first) = list.first() else { return Ok(None) };
    let hardware = list.iter().copied().find(|e| !is_software(e));
    let target = q.level().vmaf_target();
    let spans = sample_spans(p.duration);
    let sampled: f64 = spans.iter().map(|s| s.1).sum();
    let duration = p.duration.unwrap_or(sampled);
    let tmp = crate::convert::TempDir::new("vlook")?;
    let size_key = if scale_to.unwrap_or(p.height) > 1080 { "large" } else { "hd" };
    let key = format!("{}-{size_key}-{:?}", codec.key(), q).to_lowercase();

    let chosen = match hardware {
        // "Balanced" and "Smaller": the graphics card, which is many times faster;
        // its setting is still chosen by how the result looks.
        Some(hw) if q != Quality::Best => search(input, p, hw, q, target, scale_to, &spans, &tmp.0, progress)?,
        // "Best": software and card are both measured (now and then) and the better one kept.
        Some(hw) if hw != first && is_software(first) => match remembered(&key) {
            Some((enc, uses)) if uses < RECHECK_AFTER && list.iter().any(|e| *e == enc) => {
                let enc = list.iter().copied().find(|e| *e == enc).unwrap_or(first);
                remember(&key, enc, uses + 1);
                search(input, p, enc, q, target, scale_to, &spans, &tmp.0, progress)?
            }
            _ => {
                let sw = search(input, p, first, q, target, scale_to, &spans, &tmp.0, &mut |f| progress(f * 0.5))?;
                let hwt = search(input, p, hw, q, target, scale_to, &spans, &tmp.0, &mut |f| progress(0.5 + f * 0.5))?;
                let use_hw = prefer_hardware(&sw, &hwt, duration, sampled, q);
                log::info!(
                    "video encoder choice: {first} {} bytes in {:.1}s vs {hw} {} bytes in {:.1}s -> {}",
                    sw.bytes, sw.secs, hwt.bytes, hwt.secs, if use_hw { hw } else { first }
                );
                let (enc, t) = if use_hw { (hw, hwt) } else { (first, sw) };
                remember(&key, enc, 1);
                t
            }
        },
        _ => search(input, p, first, q, target, scale_to, &spans, &tmp.0, progress)?,
    };
    progress(1.0);
    // How big the result will be next to the source, from the samples.
    let source_bytes = input.metadata().map(|m| m.len()).unwrap_or(0) as f64;
    let ratio = if duration > 0.0 && source_bytes > 0.0 { chosen.bytes as f64 / (source_bytes * sampled.min(duration) / duration) } else { 0.0 };
    Ok(Some((Tuned { encoder: chosen.encoder, value: chosen.value }, ratio)))
}

/// Runs a video job from `input` to `output` (for Frames, a folder).
/// Runs a video job from `input` to `output` (for Frames, a folder) and
/// returns what was made (a long video's GIF is renamed "… (first 30 s).gif").
pub fn run(input: &Path, job: Job, opts: &Opts, output: &Path, progress: &mut dyn FnMut(f64)) -> Result<PathBuf, String> {
    let p = probe(input)?;
    // Measure first (about a fifth of the time), then encode the whole video.
    let mut tuned = None;
    let mut share = 0.0;
    if let (true, Some((codec, scale_to))) = (opts.tune, encode_of(job, &p)) {
        share = 0.2;
        match tune(input, &p, codec, opts.quality, scale_to, &mut |f| progress(f * 0.2)) {
            Ok(Some((t, ratio))) => {
                log::info!("video {job:?}: {} at {}, about {:.0}% of the source", t.encoder, t.value, ratio * 100.0);
                if job == Job::Compress && ratio > 0.9 {
                    return Err("This video is already compact; compressing it wouldn't make it smaller.".into());
                }
                tuned = Some(t);
            }
            Ok(None) => {}
            Err(e) if e == crate::procs::CANCELLED => return Err(e),
            Err(e) => log::warn!("video look search failed ({e}); using the usual settings"),
        }
    }
    let args = args_with(job, &p, opts, tuned.as_ref())?;
    log::info!("video {job:?}: {:?} -> {}", p, args.join(" "));
    let mut inner = |f: f64| progress(share + (1.0 - share) * f);
    let progress: &mut dyn FnMut(f64) = &mut inner;
    match job {
        Job::Frames => {
            let (_, stem, _) = crate::convert::split(input);
            // FFmpeg reads % anywhere in the path as part of its numbering pattern.
            let escaped = PathBuf::from(output.to_string_lossy().replace('%', "%%"));
            let pattern = escaped.join(format!("{}-%04d.png", stem.replace('%', "%%")));
            run_ffmpeg(input, &args, &pattern, progress)?;
        }
        Job::Mkv => {
            if let Err(e) = run_ffmpeg(input, &args, output, progress) {
                if e == crate::procs::CANCELLED {
                    return Err(e);
                }
                // Some subtitle or data streams can't go into MKV; keep just picture and sound.
                log::info!("mkv with every stream failed ({e}); retrying with video and audio only");
                let _ = std::fs::remove_file(output);
                run_ffmpeg(input, &s(&["-map", "0:v", "-map", "0:a?", "-c", "copy", "-map_metadata", "0"]), output, progress)?;
            }
        }
        Job::Compress => {
            run_ffmpeg(input, &args, output, progress)?;
            let before = input.metadata().map(|m| m.len()).unwrap_or(0);
            let after = output.metadata().map(|m| m.len()).unwrap_or(0);
            if after as f64 >= before as f64 * 0.95 {
                return Err("This video is already compact; compressing it wouldn't make it smaller.".into());
            }
        }
        Job::Gif if p.duration.unwrap_or(0.0) > opts.gif_seconds as f64 => {
            run_ffmpeg(input, &args, output, progress)?;
            let dir = output.parent().map(Path::to_path_buf).unwrap_or_default();
            let stem = output.file_stem().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();
            let name = crate::convert::Reserved::new(&dir, &format!("{stem} (first {} s)", opts.gif_seconds), "gif");
            let renamed = name.0.clone();
            std::fs::rename(output, &renamed).map_err(|e| format!("Couldn't name the GIF: {e}"))?;
            return Ok(renamed);
        }
        _ => run_ffmpeg(input, &args, output, progress)?,
    }
    Ok(output.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'a.mp4':
  Duration: 00:00:10.00, start: 0.000000, bitrate: 1000 kb/s
  Stream #0:0[0x1](und): Video: h264 (High) (avc1 / 0x31637661), yuv420p(tv, bt709, progressive), 1920x1080 [SAR 1:1 DAR 16:9], 900 kb/s, 30 fps
  Stream #0:1[0x2](und): Audio: aac (LC) (mp4a / 0x6134706D), 48000 Hz, stereo, fltp, 128 kb/s";

    #[test]
    fn probe_reads_streams() {
        let p = parse_probe(SAMPLE);
        assert_eq!(p.video.as_deref(), Some("h264"));
        assert_eq!(p.audio.as_deref(), Some("aac"));
        assert_eq!((p.width, p.height), (1920, 1080));
        let art = parse_probe("  Stream #0:1: Video: mjpeg (Baseline), yuvj420p, 600x600, 90k tbr (attached pic)\n  Stream #0:0: Audio: mp3, 44100 Hz");
        assert!(art.video.is_none());
    }

    #[test]
    fn long_videos_are_limited() {
        let mut p = parse_probe(SAMPLE);
        assert_eq!(p.duration, Some(10.0));
        assert!(!args_for(Job::Gif, &p, &Opts::default()).unwrap().contains(&"-t".to_string()));
        p.duration = Some(95.0);
        let gif = args_for(Job::Gif, &p, &Opts::default()).unwrap();
        assert_eq!(&gif[..2], ["-t", "30"]);
        p.duration = Some(2.0 * 3600.0);
        let frames = args_for(Job::Frames, &p, &Opts::default()).unwrap();
        assert!(frames.contains(&"fps=1/12".to_string()), "{frames:?}");
    }

    #[test]
    fn settings_shape_gifs_and_quality() {
        let mut p = parse_probe(SAMPLE);
        p.duration = Some(40.0);
        let q = crate::settings::Quality { gif_width: 320, gif_seconds: 10, video: "best".into(), ..Default::default() };
        let o = Opts::from_quality(&q);
        let gif = args_for(Job::Gif, &p, &o).unwrap();
        assert_eq!(&gif[..2], ["-t", "10"]);
        assert!(gif.iter().any(|a| a.contains("min(320,iw)")));
        assert_eq!(o.quality, Quality::Best);
        assert!(encoder_args("libx264", Quality::Best, 1920, 1080).contains(&"18".to_string()));
        assert!(encoder_args("libx264", Quality::High, 1920, 1080).contains(&"20".to_string()));
    }

    #[test]
    fn fitting_streams_are_copied() {
        let p = parse_probe(SAMPLE);
        let a = args_for(Job::Mp4, &p, &Opts::default()).unwrap();
        assert!(a.windows(2).any(|w| w == ["-c:v", "copy"]));
        assert!(a.windows(2).any(|w| w == ["-c:a", "copy"]));
        let small = Probe { height: 480, ..p };
        assert!(args_for(Job::To720p, &small, &Opts::default()).unwrap_err().contains("720p"));
    }

    #[test]
    fn bitrate_scales_with_size() {
        assert_eq!(bitrate(1920, 1080, Quality::High), "8000k");
        let kbps = |b: String| b.trim_end_matches('k').parse::<u32>().unwrap();
        assert!(kbps(bitrate(3840, 2160, Quality::High)) > kbps(bitrate(1280, 720, Quality::High)));
        assert_eq!(bitrate(160, 120, Quality::Small), "400k");
    }
}

#[cfg(test)]
mod look_tests {
    use super::*;

    /// The search ends at a setting that reaches the target, and a 1080p
    /// camera-like clip gets far smaller than its high-bitrate source.
    #[test]
    fn search_reaches_the_target() {
        let Some(ff) = tools::find(Tool::Ffmpeg) else { return };
        if metric() != Metric::Vmaf {
            return;
        }
        let d = std::env::temp_dir().join(format!("convertino-vlook-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let src = d.join("cam.mp4");
        assert!(tools::command(&ff)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=30:duration=6"])
            .args(["-vf", "noise=alls=8:allf=t", "-c:v", "libx264", "-preset", "ultrafast", "-crf", "12"])
            .arg(&src)
            .status()
            .unwrap()
            .success());
        let p = probe(&src).unwrap();
        let (t, ratio) = tune(&src, &p, Codec::Hevc, Quality::High, None, &mut |_| {}).unwrap().unwrap();
        println!("tuned: {t:?}, about {:.0}% of the source", ratio * 100.0);
        let tmp = crate::convert::TempDir::new("vlook-check").unwrap();
        let check = trial(&src, &p, t.encoder, t.value, Quality::High, None, &sample_spans(p.duration), &tmp.0).unwrap();
        assert!(check.score >= Quality::High.level().vmaf_target(), "{}", check.score);
        assert!(ratio < 0.6, "{ratio}");
        assert!(t.value > default_knob(t.encoder, Quality::Small) - 8);
    }

    #[test]
    fn scores_are_read() {
        assert_eq!(parse_score("[Parsed_libvmaf_6 @ 0x1] VMAF score: 94.512345\n"), Some(94.512345));
        let ssim = parse_score("[Parsed_ssim_4 @ 0x1] SSIM Y:0.99 (20) U:0.99 V:0.99 All:0.980000 (17.0)").unwrap();
        assert!((ssim - 92.0).abs() < 0.01);
        assert_eq!(sample_spans(Some(10.0)), vec![(3.0, 4.0)]);
        assert_eq!(sample_spans(Some(100.0)).len(), 3);
    }
}
