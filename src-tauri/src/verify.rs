//! Checks before a new copy replaces its original (shared rule of Crofty's tools, TOOLS-CONTRACT.md).
//!
//! Learned the hard way in Drive Cleanup: two Valorant clips came out glitchy,
//! rotated and cropped, yet the right length and playable. So a copy has to
//! *look* like its original, not just play:
//! - the original is checked first (video): NVIDIA DVR recordings can hold broken
//!   sections that players hide, and an encoder bakes them in;
//! - the copy keeps the length, the picture, the sound and the picture's shape;
//! - 8 frames of the copy are compared with the original's (SSIM on small grey frames);
//! - the copy plays cleanly for 4 s at the start, the middle and the end.

use crate::tools::{self, Tool};
use crate::video::{self, Probe};
use std::path::Path;
use std::process::Stdio;

/// Parser complaints before a recording counts as broken (good clips show none; broken ones 4 to 66).
const BROKEN_AT: usize = 3;
/// How alike a frame of the copy must be (good heavy compression scores 0.83 to 0.93, a rotated copy 0.20).
const MIN_MATCH: f64 = 0.6;
/// Frames compared.
const FRAMES: usize = 8;

fn family(p: &Path) -> Option<String> {
    let ext = p.extension()?.to_string_lossy().to_lowercase();
    crate::wheel::family_of(&ext).map(|f| f.0)
}

fn is_video(p: &Path) -> bool {
    family(p).as_deref() == Some("video")
}

/// Containers that normally carry sound (a GIF never does).
fn carries_sound(p: &Path) -> bool {
    let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    matches!(ext.as_str(), "mp4" | "m4v" | "mov" | "mkv" | "webm" | "avi" | "wmv" | "flv" | "ts" | "mts" | "m2ts" | "3gp")
}

/// Lines of FFmpeg's error output that report damage.
fn damage_lines(stderr: &str) -> usize {
    stderr.lines().filter(|l| {
        let l = l.to_lowercase();
        l.contains("error") || l.contains("no frame") || l.contains("non-existing") || l.contains("invalid")
    }).count()
}

/// Is a video original sound enough to replace? Reads every video packet
/// without decoding pictures (roughly the time it takes to read the file).
/// Not a video: nothing to check.
pub fn source_ok(original: &Path) -> Result<(), String> {
    if !is_video(original) {
        return Ok(());
    }
    let ff = tools::require(Tool::Ffmpeg)?;
    let out = tools::command(&ff)
        .args(["-hide_banner", "-nostdin", "-v", "error", "-i"])
        .arg(original)
        .args(["-map", "0:v:0", "-c", "copy", "-f", "null", "-"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("Couldn't start FFmpeg: {e}"))?;
    let err = String::from_utf8_lossy(&out.stderr);
    let damage = damage_lines(&err);
    if damage >= BROKEN_AT {
        return Err(format!("The video itself has broken sections ({damage} damaged spots); a compressed copy would bake in glitches, so it was left as it is"));
    }
    if !out.status.success() {
        return Err("The video couldn't be read, so it was left as it is".into());
    }
    Ok(())
}

/// Size of the grey frames compared.
const FW: usize = 320;
const FH: usize = 180;

/// One frame of `p` at `t` seconds as small grey pixels; None when FFmpeg gave no frame.
/// `seek`: jump straight to `t` (videos). Animated pictures (GIF) don't seek reliably in
/// every FFmpeg build, so they are decoded from the start and trimmed to `t` instead.
///
/// Each file is read by its own FFmpeg run and the two frames are compared here: a single
/// FFmpeg run with both files and its `ssim` filter pairs frames by timing, and on a busy
/// Windows PC it now and then paired the wrong ones (a good copy scored 0%).
fn frame_at(ff: &Path, p: &Path, t: f64, seek: bool) -> Result<Option<Vec<u8>>, String> {
    let ts = format!("{t:.3}");
    let mut cmd = tools::command(ff);
    cmd.args(["-hide_banner", "-nostdin", "-nostats", "-v", "error"]);
    let filter = if seek {
        cmd.args(["-ss", &ts]);
        format!("scale={FW}:{FH},format=gray")
    } else {
        format!("trim=start={ts},setpts=PTS-STARTPTS,scale={FW}:{FH},format=gray")
    };
    cmd.arg("-i").arg(p);
    let out = cmd
        .args(["-map", "0:v:0", "-frames:v", "1", "-vf", &filter, "-f", "rawvideo", "-pix_fmt", "gray", "-"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("Couldn't start FFmpeg: {e}"))?;
    let mut px = out.stdout;
    if px.len() < FW * FH {
        return Ok(None);
    }
    px.truncate(FW * FH);
    Ok(Some(px))
}

/// SSIM of two grey frames (0..1), the way FFmpeg's `ssim` filter scores them:
/// 8×8 windows stepping 4 pixels.
fn ssim(a: &[u8], b: &[u8], w: usize, h: usize) -> f64 {
    let (bw, bh) = (w / 4, h / 4);
    // Sums per 4×4 block: a, b, a²+b², a·b.
    let mut sums = vec![[0f64; 4]; bw * bh];
    for by in 0..bh {
        for bx in 0..bw {
            let s = &mut sums[by * bw + bx];
            for y in by * 4..by * 4 + 4 {
                for x in bx * 4..bx * 4 + 4 {
                    let (p, q) = (a[y * w + x] as f64, b[y * w + x] as f64);
                    s[0] += p;
                    s[1] += q;
                    s[2] += p * p + q * q;
                    s[3] += p * q;
                }
            }
        }
    }
    let c1 = (0.01f64 * 0.01 * 255.0 * 255.0 * 64.0 + 0.5).floor();
    let c2 = (0.03f64 * 0.03 * 255.0 * 255.0 * 64.0 * 63.0 + 0.5).floor();
    let (mut total, mut n) = (0.0, 0usize);
    for y in 0..bh.saturating_sub(1) {
        for x in 0..bw.saturating_sub(1) {
            let mut s = [0f64; 4];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let k = &sums[(y + dy) * bw + x + dx];
                for i in 0..4 {
                    s[i] += k[i];
                }
            }
            let [s1, s2, ss, s12] = s;
            let vars = ss * 64.0 - s1 * s1 - s2 * s2;
            let covar = s12 * 64.0 - s1 * s2;
            total += (2.0 * s1 * s2 + c1) * (2.0 * covar + c2) / ((s1 * s1 + s2 * s2 + c1) * (vars + c2));
            n += 1;
        }
    }
    if n == 0 { 0.0 } else { total / n as f64 }
}

/// SSIM of one frame of `a` and `b` at `t`; Err when no comparison was possible at all.
fn similarity(ff: &Path, a: &Path, b: &Path, t: f64, picture: bool) -> Result<f64, String> {
    similarity_at(ff, a, t, b, t, picture)
}

/// SSIM of `a` at `ta` and `b` at `tb` (a test sample starts later in its original).
fn similarity_at(ff: &Path, a: &Path, ta: f64, b: &Path, t: f64, picture: bool) -> Result<f64, String> {
    let fail = || format!("The new copy couldn't be compared with the original around {t:.0} s, so the original was kept");
    let fa = frame_at(ff, a, ta, true)?.ok_or_else(fail)?;
    let mut best = None;
    if !picture {
        if let Some(fb) = frame_at(ff, b, t, true)? {
            let s = ssim(&fa, &fb, FW, FH);
            if s >= MIN_MATCH {
                return Ok(s);
            }
            best = Some(s);
        }
    }
    // A low score from the seek is checked again by decoding the copy from the start
    // (a seek can land a frame or two off). A copy that really differs scores low both ways.
    match frame_at(ff, b, t, false)? {
        Some(fb) => {
            let s = ssim(&fa, &fb, FW, FH);
            Some(best.map_or(s, |f: f64| f.max(s)))
        }
        None => best,
    }
    .ok_or_else(fail)
}

/// Does `p` decode for 4 s from `at` with no complaints?
fn plays(ff: &Path, p: &Path, at: f64, video: bool) -> Result<bool, String> {
    let mut cmd = tools::command(ff);
    cmd.args(["-hide_banner", "-nostdin", "-v", "error", "-ss", &format!("{:.3}", at.max(0.0)), "-i"]).arg(p).args(["-t", "4"]);
    if !video {
        cmd.arg("-vn");
    }
    let out = cmd.args(["-f", "null", "-"]).stdin(Stdio::null()).output().map_err(|e| format!("Couldn't start FFmpeg: {e}"))?;
    Ok(out.status.success() && String::from_utf8_lossy(&out.stderr).trim().is_empty())
}

fn probe(p: &Path) -> Result<Probe, String> {
    video::probe(p).map_err(|_| "The new copy couldn't be read".to_string())
}

/// One of Compare's checkpoints: where in the copy, and how alike (0..1; None: no frame there).
#[derive(Debug, Clone, serde::Serialize)]
pub struct Point {
    pub t: f64,
    pub s: Option<f64>,
}

/// Where the copy check looks: 8 points spread over the copy (same as `copy_ok`).
pub fn checkpoint_times(duration: f64) -> Vec<f64> {
    (0..FRAMES).map(|i| (duration * (0.03 + 0.94 * i as f64 / (FRAMES - 1) as f64)).min((duration - 1.0).max(0.0)).max(0.0)).collect()
}

/// Compare's checkpoints: the same frames the copy check compares, scored the same way.
/// `offset`: the copy starts this many seconds into the original. A picture gets one point.
pub fn checkpoints(original: &Path, copy: &Path, offset: f64) -> Result<Vec<Point>, String> {
    let ff = tools::require(Tool::Ffmpeg)?;
    if family(copy).as_deref() == Some("image") || family(original).as_deref() == Some("image") {
        let s = similarity_at(&ff, original, 0.0, copy, 0.0, true).ok();
        return Ok(vec![Point { t: 0.0, s }]);
    }
    let d = probe(copy)?.duration.ok_or("The copy's length couldn't be read")?;
    Ok(checkpoint_times(d)
        .into_iter()
        .map(|t| Point { t, s: similarity_at(&ff, original, t + offset, copy, t, false).ok() })
        .collect())
}

/// Is `made` a sound replacement for `original`? Err says why not.
pub fn copy_ok(original: &Path, made: &Path) -> Result<(), String> {
    let len = made.metadata().map(|m| m.len()).unwrap_or(0);
    if len == 0 {
        return Err("The new copy is empty".into());
    }
    let (fo, fm) = (family(original), family(made));
    let av = |f: &Option<String>| matches!(f.as_deref(), Some("video" | "audio"));
    if !av(&fo) {
        return Ok(());
    }
    let ff = tools::require(Tool::Ffmpeg)?;
    let a = video::probe(original).map_err(|_| "The original couldn't be read to compare it".to_string())?;
    let b = probe(made)?;
    let (Some(da), Some(db)) = (a.duration, b.duration) else {
        return Err("The lengths couldn't be read to compare them".into());
    };
    if (da - db).abs() > (da * 0.002).max(1.0) {
        return Err(format!("The new copy is {db:.0} s long, the original {da:.0} s"));
    }
    // A picture result (a video, or an animated GIF), or a sound-only one (MP3 from a video).
    let made_video = fm.as_deref() != Some("audio") && b.video.is_some();
    if a.video.is_some() && made_video {
        if b.video.is_none() {
            return Err("The new copy has no picture".into());
        }
        if a.audio.is_some() && b.audio.is_none() && carries_sound(made) {
            return Err("The new copy lost its sound".into());
        }
        if a.width == 0 || b.width == 0 || a.height == 0 || b.height == 0 {
            return Err("The picture size couldn't be read to compare it".into());
        }
        let (ra, rb) = (a.width as f64 / a.height as f64, b.width as f64 / b.height as f64);
        if (ra - rb).abs() > 0.01 * ra {
            return Err(format!("The picture's shape changed ({}×{} → {}×{})", a.width, a.height, b.width, b.height));
        }
        for t in checkpoint_times(db) {
            let s = similarity(&ff, original, made, t, fm.as_deref() == Some("image"))?;
            if s < MIN_MATCH {
                return Err(format!("The new copy doesn't look like the original around {t:.0} s (match {:.0}%)", s * 100.0));
            }
        }
    } else if a.audio.is_some() && b.audio.is_none() {
        return Err("The new copy has no sound".into());
    }
    for t in [0.0, db / 2.0, (db - 5.0).max(0.0)] {
        if !plays(&ff, made, t, made_video)? {
            return Err(format!("The new copy doesn't play cleanly around {t:.0} s"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn ff() -> Option<PathBuf> {
        tools::find(Tool::Ffmpeg)
    }

    fn run(ff: &Path, args: &[&str]) {
        let ok = std::process::Command::new(ff).args(["-y", "-loglevel", "error"]).args(args).status().unwrap().success();
        assert!(ok, "ffmpeg {args:?}");
    }

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("convertino-verify-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn counts_damage_lines() {
        let s = "[h264 @ 0x1] decode_slice_header error\n[h264 @ 0x1] no frame!\n[h264 @ 0x1] non-existing PPS 0 referenced\nfine line\n";
        assert_eq!(damage_lines(s), 3);
        assert_eq!(damage_lines(""), 0);
    }

    #[test]
    fn ssim_scores_like_ffmpegs_filter() {
        let flat = vec![128u8; FW * FH];
        assert!((ssim(&flat, &flat, FW, FH) - 1.0).abs() < 1e-9);
        let Some(ff) = ff() else { return };
        let d = dir("ssim");
        let src = d.join("src.mp4");
        let s = src.to_string_lossy().into_owned();
        run(&ff, &["-f", "lavfi", "-i", "testsrc2=size=640x360:rate=30:duration=3", "-c:v", "mpeg4", "-q:v", "2", &s]);
        let small = d.join("small.mp4");
        run(&ff, &["-i", &s, "-c:v", "mpeg4", "-q:v", "15", "-vf", "scale=320:180", &small.to_string_lossy()]);
        let a = frame_at(&ff, &src, 1.5, true).unwrap().unwrap();
        let b = frame_at(&ff, &small, 1.5, true).unwrap().unwrap();
        let ours = ssim(&a, &b, FW, FH);
        // FFmpeg's own score of the same two grey frames.
        let (pa, pb) = (d.join("a.gray"), d.join("b.gray"));
        std::fs::write(&pa, &a).unwrap();
        std::fs::write(&pb, &b).unwrap();
        let size = format!("{FW}x{FH}");
        let out = std::process::Command::new(&ff)
            .args(["-hide_banner", "-nostdin", "-f", "rawvideo", "-pix_fmt", "gray", "-s", &size, "-i"])
            .arg(&pa)
            .args(["-f", "rawvideo", "-pix_fmt", "gray", "-s", &size, "-i"])
            .arg(&pb)
            .args(["-lavfi", "[0:v][1:v]ssim", "-f", "null", "-"])
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stderr);
        let theirs: f64 = text.lines().rev().find_map(|l| l.split("All:").nth(1)).and_then(|r| r.split_whitespace().next()?.parse().ok()).unwrap();
        assert!((ours - theirs).abs() < 0.001, "ours {ours}, FFmpeg {theirs}");
        assert!(ours > 0.8 && ours < 1.0, "{ours}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn checkpoints_score_a_good_copy_high_and_a_flipped_one_low() {
        let Some(ff) = ff() else { return };
        let d = dir("points");
        let src = d.join("src.mov");
        let s = src.to_string_lossy().into_owned();
        run(&ff, &["-f", "lavfi", "-i", "testsrc2=size=640x360:rate=30:duration=8", "-c:v", "mpeg4", "-q:v", "3", &s]);
        let good = d.join("good.mp4");
        run(&ff, &["-i", &s, "-c:v", "mpeg4", "-q:v", "12", &good.to_string_lossy()]);
        let pts = checkpoints(&src, &good, 0.0).unwrap();
        assert_eq!(pts.len(), 8);
        assert!(pts.iter().all(|p| p.s.is_some_and(|s| s > 0.8)), "{pts:?}");
        let flipped = d.join("flipped.mp4");
        run(&ff, &["-i", &s, "-c:v", "mpeg4", "-vf", "vflip,hflip", &flipped.to_string_lossy()]);
        assert!(checkpoints(&src, &flipped, 0.0).unwrap().iter().all(|p| p.s.is_some_and(|s| s < 0.6)));
        // A test sample: 3 s from 4 s into the original, lined up by the offset.
        let clip = d.join("clip.mp4");
        run(&ff, &["-ss", "4", "-i", &s, "-t", "3", "-c:v", "mpeg4", "-q:v", "6", &clip.to_string_lossy()]);
        assert!(checkpoints(&src, &clip, 4.0).unwrap().iter().all(|p| p.s.is_some_and(|s| s > 0.8)));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_faithful_copy_passes_and_a_rotated_or_short_one_fails() {
        let Some(ff) = ff() else { return };
        let d = dir("copies");
        let src = d.join("src.mov");
        let s = src.to_string_lossy().into_owned();
        run(&ff, &["-f", "lavfi", "-i", "testsrc2=size=640x360:rate=30:duration=8", "-f", "lavfi", "-i", "sine=f=440:duration=8", "-c:v", "mpeg4", "-q:v", "3", "-c:a", "aac", "-shortest", &s]);
        assert!(source_ok(&src).is_ok());
        // a heavier, smaller copy: still the same video
        let good = d.join("good.mp4");
        run(&ff, &["-i", &s, "-c:v", "mpeg4", "-q:v", "12", "-vf", "scale=320:180", "-c:a", "aac", &good.to_string_lossy()]);
        assert_eq!(copy_ok(&src, &good), Ok(()));
        // flipped upside down: right length and shape, wrong picture
        let flipped = d.join("flipped.mp4");
        run(&ff, &["-i", &s, "-c:v", "mpeg4", "-vf", "vflip,hflip", "-c:a", "aac", &flipped.to_string_lossy()]);
        assert!(copy_ok(&src, &flipped).unwrap_err().contains("doesn't look like"));
        // turned sideways: the shape changed
        let rotated = d.join("rotated.mp4");
        run(&ff, &["-i", &s, "-c:v", "mpeg4", "-vf", "transpose=1", "-c:a", "aac", &rotated.to_string_lossy()]);
        assert!(copy_ok(&src, &rotated).unwrap_err().contains("shape"));
        // cut short
        let short = d.join("short.mp4");
        run(&ff, &["-i", &s, "-t", "5", "-c:v", "mpeg4", "-c:a", "aac", &short.to_string_lossy()]);
        assert!(copy_ok(&src, &short).unwrap_err().contains("long"));
        // lost its sound
        let mute = d.join("mute.mp4");
        run(&ff, &["-i", &s, "-c:v", "mpeg4", "-an", &mute.to_string_lossy()]);
        assert!(copy_ok(&src, &mute).unwrap_err().contains("sound"));
        // a GIF never has sound: that's not a fault
        let gif = d.join("anim.gif");
        run(&ff, &["-i", &s, "-vf", "fps=10,scale=160:-1", &gif.to_string_lossy()]);
        assert_eq!(copy_ok(&src, &gif), Ok(()));
        // the soundtrack as MP3: picture checks don't apply
        let mp3 = d.join("sound.mp3");
        run(&ff, &["-i", &s, "-vn", &mp3.to_string_lossy()]);
        assert_eq!(copy_ok(&src, &mp3), Ok(()));
        // empty
        let empty = d.join("empty.mp4");
        std::fs::write(&empty, b"").unwrap();
        assert!(copy_ok(&src, &empty).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_damaged_recording_is_left_alone() {
        let Some(ff) = ff() else { return };
        let d = dir("damaged");
        let src = d.join("clip.mts");
        let s = src.to_string_lossy().into_owned();
        // H.264 in MPEG-TS (.mts); then every other slice header is garbled, the damage the
        // parser reports in broken NVIDIA DVR clips ("non-existing PPS", slice header errors).
        let enc = if std::process::Command::new(&ff).args(["-hide_banner", "-encoders"]).output().map(|o| String::from_utf8_lossy(&o.stdout).contains("libx264")).unwrap_or(false) { "libx264" } else { return };
        run(&ff, &["-f", "lavfi", "-i", "testsrc2=size=640x360:rate=30:duration=6", "-c:v", enc, "-g", "15", &s]);
        assert!(source_ok(&src).is_ok(), "the clean recording passes");
        let mut bytes = std::fs::read(&src).unwrap();
        let (mut i, mut n) = (0, 0);
        while i + 8 < bytes.len() {
            if bytes[i..i + 3] == [0, 0, 1] && matches!(bytes[i + 3] & 0x1f, 1 | 5) {
                if n % 2 == 0 {
                    bytes[i + 4] ^= 0xff;
                    bytes[i + 5] ^= 0xa5;
                }
                n += 1;
            }
            i += 1;
        }
        std::fs::write(&src, &bytes).unwrap();
        let e = source_ok(&src).unwrap_err();
        assert!(e.contains("broken sections"), "{e}");
        let _ = std::fs::remove_dir_all(&d);
    }
}
