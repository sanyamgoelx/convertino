//! "Your last 40": how Compress (the "looks the same" kind, no size asked)
//! did with each preset, for the line under each preset in Settings ›
//! Quality. Kept in preset-stats.json; a preset's list starts over when its
//! tuning changes, so the line always describes the settings as they are.

use crate::convert::{Op, Step};
use crate::tune::{Grade, Tunes};
use std::path::{Path, PathBuf};

/// Results kept per preset.
const KEEP: usize = 40;

fn path() -> Option<PathBuf> {
    crate::settings::config_dir().map(|d| d.join("preset-stats.json"))
}

fn load() -> serde_json::Value {
    path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .filter(|v: &serde_json::Value| v.is_object())
        .unwrap_or_else(|| serde_json::json!({}))
}

fn grade_of(word: &str) -> Grade {
    Grade::from_word(word)
}

/// Which preset a step used (kind, preset, fingerprint), when it's one the
/// line counts.
pub fn preset_of(op: &Op) -> Option<(&'static str, Grade, String)> {
    match op {
        Op::Video { job: crate::video::Job::Compress, opts } => {
            let (g, v) = opts.preset();
            Some(("video", g, format!("{v:?}")))
        }
        Op::PdfCompress { level } => Some(("pdf", level.grade, format!("{:?}", crate::tune::Pdf { look: level.target, check_dpi: level.check_dpi }))),
        Op::ToSize { kind, bytes: None, quality, .. } => {
            let (name, g) = match kind {
                crate::size::Kind::Image => ("image", grade_of(&quality.image)),
                crate::size::Kind::Video => ("video", grade_of(&quality.video)),
                crate::size::Kind::Pdf => ("pdf", grade_of(&quality.pdf_compress)),
                crate::size::Kind::Audio => return None,
            };
            Some((name, g, crate::tune::signature(name, &quality.tune, g)))
        }
        _ => None,
    }
}

/// After a step worked: its size next to the original, and the time it took.
pub fn after_step(step: &Step, made: &Path, seconds: f64) {
    let Some((kind, grade, sig)) = preset_of(&step.op) else { return };
    let before = step.inputs.first().and_then(|p| p.metadata().ok()).map(|m| m.len()).unwrap_or(0);
    let after = made.metadata().map(|m| m.len()).unwrap_or(0);
    if before == 0 || after == 0 || !made.is_file() {
        return;
    }
    record(kind, grade, &sig, after as f64 / before as f64, seconds);
}

fn record(kind: &str, grade: Grade, sig: &str, ratio: f64, seconds: f64) {
    let Some(file) = path() else { return };
    let key = format!("{kind}.{}", grade.word());
    let mut all = load();
    let same = all.get(&key).and_then(|e| e.get("sig")).and_then(|s| s.as_str()) == Some(sig);
    let mut runs: Vec<serde_json::Value> = if same { all[&key]["runs"].as_array().cloned().unwrap_or_default() } else { Vec::new() };
    runs.push(serde_json::json!([(ratio * 1000.0).round() / 1000.0, (seconds * 10.0).round() / 10.0]));
    if runs.len() > KEEP {
        runs.drain(..runs.len() - KEEP);
    }
    all[&key] = serde_json::json!({ "sig": sig, "runs": runs });
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&file, serde_json::to_string(&all).unwrap_or_default());
}

/// Per preset ("video.balanced"…): how many results under the current
/// tuning, their average size next to the original (0–1) and seconds each.
pub fn summary(t: &Tunes) -> serde_json::Value {
    let all = load();
    let mut out = serde_json::Map::new();
    for kind in ["image", "video", "pdf"] {
        for g in Grade::ALL {
            let key = format!("{kind}.{}", g.word());
            let e = all.get(&key);
            let current = e.and_then(|e| e.get("sig")).and_then(|s| s.as_str()) == Some(crate::tune::signature(kind, t, g).as_str());
            let runs: Vec<(f64, f64)> = if current {
                e.and_then(|e| e.get("runs")).and_then(|r| r.as_array()).map(|r| r.iter().filter_map(|x| Some((x.get(0)?.as_f64()?, x.get(1)?.as_f64()?))).collect()).unwrap_or_default()
            } else {
                Vec::new()
            };
            let n = runs.len().max(1) as f64;
            out.insert(
                key,
                serde_json::json!({
                    "count": runs.len(),
                    "ratio": runs.iter().map(|r| r.0).sum::<f64>() / n,
                    "seconds": runs.iter().map(|r| r.1).sum::<f64>() / n,
                }),
            );
        }
    }
    serde_json::Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A job's fingerprint matches the one Settings asks about, so results count.
    #[test]
    fn fingerprints_match_settings() {
        let mut q = crate::settings::Quality { video: "best".into(), pdf_compress: "high".into(), ..Default::default() };
        q.tune.apply_list(Grade::Best, "look=97,max-res=1440,check-dpi=200").unwrap();
        let v = Op::Video { job: crate::video::Job::Compress, opts: crate::video::Opts::from_quality(&q) };
        let (k, g, sig) = preset_of(&v).unwrap();
        assert_eq!((k, g), ("video", Grade::Best));
        assert_eq!(sig, crate::tune::signature("video", &q.tune, Grade::Best));
        let p = Op::PdfCompress { level: q.pdf_level() };
        assert_eq!(preset_of(&p).unwrap().2, crate::tune::signature("pdf", &q.tune, Grade::Best));
        let i = Op::ToSize { kind: crate::size::Kind::Image, bytes: None, trim: None, quality: Box::new(q.clone()) };
        assert_eq!(preset_of(&i).unwrap().2, crate::tune::signature("image", &q.tune, Grade::Balanced));
        let sized = Op::ToSize { kind: crate::size::Kind::Image, bytes: Some(1000), trim: None, quality: Box::new(q) };
        assert!(preset_of(&sized).is_none(), "a size asked for isn't the preset's doing");
    }
}
