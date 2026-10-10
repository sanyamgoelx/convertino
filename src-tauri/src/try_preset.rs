//! "Test on a file" (Settings › Quality › Tune): runs a preset, as it is
//! being tuned, on a file the person picks. Pictures and PDFs are
//! compressed whole into a temporary folder; a video is measured on its
//! samples and a short clip is made. Nothing is saved next to the file, and
//! the result can be opened in the Compare window.

use crate::settings::Quality;
use crate::size::Kind;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub name: String,
    /// Result next to the original (0–1); for a video, estimated from the samples.
    pub ratio: f64,
    /// How alike it looks (the preset's own score), when measured.
    pub score: Option<f64>,
    pub target: f64,
    /// What was chosen, in a few words ("x265 CRF 24", "JPG").
    pub setting: String,
    /// Seconds: the test itself (pictures, PDFs) or the estimate for the whole video.
    pub seconds: f64,
    pub estimate: bool,
    /// Compare window id.
    pub compare: u64,
}

/// The test running now (one at a time), for Cancel.
static CURRENT: AtomicU64 = AtomicU64::new(0);

pub fn cancel() {
    let id = CURRENT.load(Ordering::SeqCst);
    if id != 0 {
        crate::procs::cancel(id);
    }
}

/// A folder for this test; earlier tests' folders go once they're a few minutes old
/// (a Compare window may still be showing the last one).
fn work_dir(id: u64) -> Result<PathBuf, String> {
    let base = std::env::temp_dir().join("convertino-tune-test");
    if let Ok(entries) = std::fs::read_dir(&base) {
        for e in entries.flatten() {
            let old = e.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).is_some_and(|age| age.as_secs() > 600);
            if old {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
    let d = base.join(id.to_string());
    std::fs::create_dir_all(&d).map_err(|e| format!("Couldn't make a folder for the test: {e}"))?;
    Ok(d)
}

/// `kind`: "image", "video" or "pdf"; `grade`: "small", "balanced", "best";
/// `q`: the quality settings with the tuning as it is on screen.
pub fn run(kind: &str, grade: &str, q: &Quality, file: &Path) -> Result<Outcome, String> {
    if !file.is_file() {
        return Err("That file isn't there any more.".into());
    }
    let id = crate::engine::new_id();
    CURRENT.store(id, Ordering::SeqCst);
    crate::procs::set_current_job(id);
    let r = run_inner(id, kind, grade, q, file);
    CURRENT.store(0, Ordering::SeqCst);
    r
}

fn run_inner(id: u64, kind: &str, grade: &str, q: &Quality, file: &Path) -> Result<Outcome, String> {
    let dir = work_dir(id)?;
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let before = file.metadata().map(|m| m.len()).unwrap_or(0).max(1);
    let g = crate::tune::Grade::from_word(grade);
    let mut q = q.clone();
    let started = std::time::Instant::now();
    match kind {
        "video" => {
            q.video = g.word().into();
            let opts = crate::video::Opts::from_quality(&q);
            let t = crate::video::test_compress(file, &opts, &dir, &mut |_| {})?;
            // "H.265 on the graphics card, level 27" (the encoder's quality number; lower looks better).
            let codec = if t.encoder.contains("265") || t.encoder.starts_with("hevc") {
                "H.265"
            } else if t.encoder.contains("av1") {
                "AV1"
            } else if t.encoder.contains("vp9") {
                "VP9"
            } else {
                "H.264"
            };
            let on = if t.encoder.starts_with("lib") { "the processor" } else { "the graphics card" };
            crate::compare::set_offset(&t.clip, t.clip_start);
            let compare = crate::jobs::register_compare(vec![(file.to_path_buf(), t.clip.clone())]);
            Ok(Outcome { name, ratio: t.ratio, score: Some(t.score), target: t.target, setting: format!("{codec} on {on}, level {}", t.value), seconds: t.estimate, estimate: true, compare })
        }
        "image" | "pdf" => {
            let size_kind = if kind == "image" { Kind::Image } else { Kind::Pdf };
            if kind == "image" {
                q.image = g.word().into();
            } else {
                q.pdf_compress = if g == crate::tune::Grade::Best { "high".into() } else { g.word().into() };
            }
            let planned = crate::size::plan(size_kind, &[file.to_path_buf()], None, &q)?;
            let mut step = planned.steps.into_iter().next().ok_or("There's nothing to test on this file.")?;
            step.dir = dir.clone();
            let made = crate::convert::run(&step, &mut |_| {})?;
            let after = made.metadata().map(|m| m.len()).unwrap_or(0);
            let (score, target, setting) = if kind == "image" {
                let level = q.image_level().unwrap_or_else(|| crate::look::Level::image(&q, g));
                let ext = made.extension().map(|e| e.to_string_lossy().to_uppercase()).unwrap_or_default();
                (picture_score(file, &made, &dir), level.target, format!("Saved as {ext}"))
            } else {
                let level = q.pdf_level();
                (None, level.target, format!("Pages compared at {} detail", match level.check_dpi { 0..=120 => "normal", 121..=170 => "fine", _ => "finest" }))
            };
            let compare = crate::jobs::register_compare(vec![(file.to_path_buf(), made)]);
            Ok(Outcome { name, ratio: after as f64 / before as f64, score, target, setting, seconds: started.elapsed().as_secs_f64(), estimate: false, compare })
        }
        other => Err(format!("Can't test \"{other}\".")),
    }
}

/// The look score of the result against the original, on the same centre
/// crop the quality search uses.
fn picture_score(original: &Path, result: &Path, dir: &Path) -> Option<f64> {
    let a = dir.join("score-a");
    let b = dir.join("score-b");
    std::fs::create_dir_all(&a).ok()?;
    std::fs::create_dir_all(&b).ok()?;
    let flat = crate::convert::s(&["-background", "white", "-alpha", "remove", "-alpha", "off"]);
    let ca = crate::look::search_crop(original, &flat, &a).ok()?;
    let cb = crate::look::search_crop(result, &flat, &b).ok()?;
    let (pa, pb) = (crate::look::decode(&ca, &[]).ok()?, crate::look::decode(&cb, &[]).ok()?);
    Some(crate::look::score(&pa, &pb))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{self, Tool};

    #[test]
    fn a_picture_is_tested_without_saving_next_to_it() {
        let Some(im) = tools::find(Tool::Magick) else { return };
        let d = std::env::temp_dir().join(format!("convertino-trypreset-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let src = d.join("photo.jpg");
        assert!(tools::command(&im).args(["-size", "1200x800", "plasma:", "-quality", "98"]).arg(&src).status().unwrap().success());
        let mut q = Quality::default();
        q.tune.apply(crate::tune::Grade::Small, "look", "60").unwrap();
        let o = run("image", "small", &q, &src).unwrap();
        assert!(o.ratio < 0.9, "{o:?}");
        assert_eq!(o.target, 60.0);
        assert!(o.score.unwrap() > 40.0, "{o:?}");
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1, "nothing next to the original");
        assert_eq!(crate::jobs::compare_pairs(o.compare).len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_video_is_measured_and_a_clip_made() {
        let Some(ff) = tools::find(Tool::Ffmpeg) else { return };
        let d = std::env::temp_dir().join(format!("convertino-tryvideo-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let src = d.join("clip.mp4");
        assert!(tools::command(&ff)
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=640x360:rate=30:duration=8"])
            .args(["-vf", "noise=alls=6:allf=t", "-c:v", "libx264", "-preset", "ultrafast", "-crf", "12"])
            .arg(&src)
            .status()
            .unwrap()
            .success());
        let o = run("video", "balanced", &Quality::default(), &src).unwrap();
        assert!(o.estimate && o.seconds > 0.0 && o.ratio > 0.0 && o.ratio < 1.0, "{o:?}");
        let pairs = crate::jobs::compare_pairs(o.compare);
        assert!(pairs[0].1.exists(), "the clip for Compare");
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1, "nothing next to the original");
        let _ = std::fs::remove_dir_all(&d);
    }
}
