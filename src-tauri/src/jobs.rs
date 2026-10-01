//! Runs conversions in the background and reports them to the HUD window.
//!
//! Each wheel pick becomes a job on its own thread, so several can run at
//! once and the wheel is never blocked. Events go to both the wheel window
//! (its progress ring shows a job that finishes quickly) and the HUD (the
//! corner card, for jobs the ring hands over: long ones and failures).

use crate::convert;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Outputs of finished jobs, for Open folder and Undo.
fn outputs() -> &'static Mutex<HashMap<u64, Vec<PathBuf>>> {
    static OUT: OnceLock<Mutex<HashMap<u64, Vec<PathBuf>>>> = OnceLock::new();
    OUT.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Started {
    id: u64,
    title: String,
    family: String,
    /// The progress ring shows this job; the HUD waits for a hand-over.
    ring: bool,
}

/// Every job event goes to the ring (wheel window) and the corner card (HUD).
fn emit<S: Serialize + Clone>(app: &AppHandle, event: &str, payload: S) {
    let _ = app.emit_to("wheel", event, payload.clone());
    let _ = app.emit_to("hud", event, payload);
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    id: u64,
    fraction: f64,
    detail: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Done {
    id: u64,
    ok: bool,
    title: String,
    body: String,
    can_undo: bool,
}

fn human_size(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.2} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.1} MB", b / 1e6)
    } else {
        format!("{} KB", ((b / 1e3).round() as u64).max(1))
    }
}

fn file_name(p: &std::path::Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// "Converted to MP3", or the action's own wording.
fn done_title(target_id: &str, label: &str) -> String {
    match target_id {
        "image.resize" => "Resized".into(),
        "audio.trim" => "Trimmed silence".into(),
        "audio.normalize" => "Normalized loudness".into(),
        "video.mp3" | "video.wav" => format!("Extracted audio as {label}"),
        _ => format!("Converted to {label}"),
    }
}

/// Starts a job and returns its id. `ring`: the wheel's progress ring shows it
/// first, and the HUD shows a card only if the ring hands it over.
/// `quality`: Shift+click options for this job (else the ones in Settings).
pub fn start(
    app: AppHandle,
    target_id: String,
    label: String,
    family: String,
    files: Vec<PathBuf>,
    ring: bool,
    quality: Option<crate::settings::Quality>,
) -> u64 {
    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    std::thread::spawn(move || {
        crate::procs::set_current_job(id);
        report_downloads(&app, id);
        let title = match files.as_slice() {
            [one] => format!("{} → {label}", file_name(one)),
            many => format!("{} files → {label}", many.len()),
        };
        if !ring {
            crate::show_hud(&app);
        }
        emit(&app, "job-started", Started { id, title, family, ring });

        let quality = quality.unwrap_or_else(|| crate::settings::get().quality);
        let steps = match convert::plan_with(&target_id, &files, &quality) {
            Ok(s) => s,
            Err(reason) => {
                log::info!("job {id}: {target_id} not available: {reason}");
                emit(&app, "job-done", Done { id, ok: false, title: format!("Can't convert to {label} yet"), body: reason, can_undo: false });
                return;
            }
        };

        let total = steps.len();
        let started = Instant::now();
        let (made, errors, cancelled) = run_steps(&app, id, &steps);
        log::info!("job {id}: {} made, {} failed, {:?}", made.len(), errors.len(), started.elapsed());

        let size: u64 = made.iter().map(|p| disk_size(p)).sum();
        // Saved somewhere else because the original's folder is read-only.
        let elsewhere = made
            .iter()
            .find(|p| p.parent() != files.first().and_then(|f| f.parent()))
            .and_then(|p| p.parent())
            .map(|d| d.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| d.display().to_string()));
        let done = if cancelled {
            Done {
                id,
                ok: !made.is_empty(),
                title: "Cancelled".into(),
                body: if made.is_empty() {
                    "Nothing was saved.".into()
                } else {
                    format!("{} of {total} converted before you cancelled.", made.len())
                },
                can_undo: !made.is_empty(),
            }
        } else if made.is_empty() {
            Done {
                id,
                ok: false,
                title: format!("Couldn't convert to {label}"),
                body: errors.first().cloned().unwrap_or_else(|| "Unknown error".into()),
                can_undo: false,
            }
        } else {
            let mut body = match made.as_slice() {
                [one] => format!("{} · {}", file_name(one), human_size(size)),
                many => format!("{} files · {}", many.len(), human_size(size)),
            };
            if let Some(folder) = &elsewhere {
                body.push_str(&format!(" · saved in {folder} (the original folder is read-only)"));
            }
            if !errors.is_empty() {
                body.push_str(&format!(" · {} failed: {}", errors.len(), errors[0]));
            }
            Done { id, ok: true, title: done_title(&target_id, &label), body, can_undo: true }
        };
        if let Ok(mut o) = outputs().lock() {
            o.insert(id, made);
        }
        emit(&app, "job-done", done);
    });
    id
}

/// A file saved by the PDF editor: shown as a done card with Open folder and Undo.
pub fn saved(app: &AppHandle, path: PathBuf) -> u64 {
    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    let body = format!("{} · {}", file_name(&path), human_size(disk_size(&path)));
    if let Ok(mut o) = outputs().lock() {
        o.insert(id, vec![path]);
    }
    crate::show_hud(app);
    emit(app, "job-done", Done { id, ok: true, title: "Saved".into(), body, can_undo: true });
    id
}

/// A converter downloaded for this job (on this thread) reports on the job's card.
fn report_downloads(app: &AppHandle, id: u64) {
    let reporter_app = app.clone();
    let last = std::cell::Cell::new(Instant::now() - Duration::from_secs(1));
    crate::install::set_reporter(move |fraction, detail| {
        // Status lines ("Setting up …") always show; percentages 5 times a second.
        if last.get().elapsed() < Duration::from_millis(200) && !detail.ends_with('…') {
            return;
        }
        last.set(Instant::now());
        emit(&reporter_app, "job-progress", Progress { id, fraction, detail: detail.to_string() });
    });
}

/// How many files of one pick convert at the same time. Videos and Office
/// documents go one by one (each already uses the whole machine, or can't
/// share LibreOffice); pictures, audio and PDFs use about half the cores.
fn parallel_for(steps: &[convert::Step]) -> usize {
    let one_at_a_time = steps.iter().any(|s| {
        matches!(s.op, convert::Op::Video { .. } | convert::Op::Office { .. } | convert::Op::OfficeThenPandoc { .. } | convert::Op::PandocThenOffice { .. })
    });
    if one_at_a_time {
        return 1;
    }
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
    (cores / 2).clamp(1, 4).min(steps.len().max(1))
}

/// Runs the steps (several at once when they allow it) and returns what was
/// made, in the order of the steps, the errors, and whether it was cancelled.
fn run_steps(app: &AppHandle, id: u64, steps: &[convert::Step]) -> (Vec<PathBuf>, Vec<String>, bool) {
    let total = steps.len();
    let workers = parallel_for(steps);
    let next = AtomicU64::new(0);
    // Per step: progress 0–1, and the outcome once done.
    let fractions = Mutex::new(vec![0.0f64; total]);
    let results: Mutex<Vec<Option<Result<PathBuf, String>>>> = Mutex::new(vec![None; total]);
    let last_emit = Mutex::new(Instant::now() - Duration::from_secs(1));
    let report_all = |i: usize, p: f64| {
        let overall = {
            let Ok(mut f) = fractions.lock() else { return };
            f[i] = p;
            f.iter().sum::<f64>() / total as f64
        };
        let Ok(mut last) = last_emit.lock() else { return };
        if last.elapsed() < Duration::from_millis(80) && p < 1.0 {
            return;
        }
        *last = Instant::now();
        let detail = if total > 1 {
            let done = results.lock().map(|r| r.iter().filter(|x| x.is_some()).count()).unwrap_or(0);
            format!("File {} of {total}", (done + 1).min(total))
        } else {
            format!("{}%", (p * 100.0).round() as u32)
        };
        emit(app, "job-progress", Progress { id, fraction: overall, detail });
    };
    let work = || loop {
        if crate::procs::is_cancelled(id) {
            break;
        }
        let i = next.fetch_add(1, Ordering::SeqCst) as usize;
        let Some(step) = steps.get(i) else { break };
        log::info!("job {id}: {} ({} input(s))", step.inputs[0].display(), step.inputs.len());
        report_all(i, 0.0);
        let r = convert::run(step, &mut |p| report_all(i, p));
        if let Ok(mut all) = results.lock() {
            all[i] = Some(r);
        }
    };
    if workers <= 1 {
        work();
    } else {
        std::thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|| {
                    crate::procs::set_current_job(id);
                    report_downloads(app, id);
                    work();
                });
            }
        });
    }

    let mut made = Vec::new();
    let mut errors = Vec::new();
    let mut cancelled = crate::procs::is_cancelled(id);
    for (step, r) in steps.iter().zip(results.into_inner().unwrap_or_default()) {
        match r {
            Some(Ok(output)) => {
                log::info!("job {id}: made {}", output.display());
                made.push(output);
            }
            Some(Err(e)) if e == crate::procs::CANCELLED => cancelled = true,
            Some(Err(e)) => {
                log::warn!("job {id}: failed on {}: {e}", step.inputs[0].display());
                errors.push(format!("{}: {e}", file_name(&step.inputs[0])));
            }
            None => {}
        }
    }
    (made, errors, cancelled)
}

/// Stops a running job (the corner card's Cancel).
pub fn cancel(id: u64) {
    crate::procs::cancel(id);
}

/// Shows the first output of a job selected in Explorer / Finder.
pub fn reveal(id: u64) -> Result<(), String> {
    let first = outputs()
        .lock()
        .ok()
        .and_then(|o| o.get(&id).and_then(|v| v.first().cloned()))
        .ok_or("Nothing to show")?;
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("explorer.exe")
            .raw_arg(format!("/select,\"{}\"", first.display()))
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg("-R").arg(&first).spawn().map_err(|e| e.to_string())?;
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = first;
    }
    Ok(())
}

/// Moves a job's outputs to the Recycle Bin / Trash.
pub fn undo(id: u64) -> Result<usize, String> {
    let files = outputs().lock().ok().and_then(|mut o| o.remove(&id)).unwrap_or_default();
    let existing: Vec<&PathBuf> = files.iter().filter(|p| p.exists()).collect();
    if existing.is_empty() {
        return Ok(0);
    }
    trash::delete_all(&existing).map_err(|e| format!("Couldn't move to the Recycle Bin: {e}"))?;
    log::info!("job {id}: undone, {} file(s) moved to the bin", existing.len());
    Ok(existing.len())
}

/// Size of a file, or of everything inside a folder.
fn disk_size(p: &std::path::Path) -> u64 {
    match p.metadata() {
        Ok(m) if m.is_dir() => std::fs::read_dir(p)
            .map(|rd| rd.flatten().map(|e| disk_size(&e.path())).sum())
            .unwrap_or(0),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}
