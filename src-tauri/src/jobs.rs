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
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

/// Originals and results of finished Compress jobs, for Compare.
fn pairs() -> &'static Mutex<HashMap<u64, Vec<(PathBuf, PathBuf)>>> {
    static P: OnceLock<Mutex<HashMap<u64, Vec<(PathBuf, PathBuf)>>>> = OnceLock::new();
    P.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The (original, result) pairs of a finished Compress job.
pub fn compare_pairs(id: u64) -> Vec<(PathBuf, PathBuf)> {
    pairs().lock().ok().and_then(|p| p.get(&id).cloned()).unwrap_or_default()
}

/// Outputs of finished jobs, for Open folder and Undo.
fn outputs_map() -> &'static Mutex<HashMap<u64, Vec<PathBuf>>> {
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
    /// The AI app that asked for it (MCP), for the card's badge.
    #[serde(skip_serializing_if = "Option::is_none")]
    via: Option<String>,
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
    /// Something the person should read even though it worked (saved in
    /// another folder, some files failed): the ring hands it to the card.
    attention: bool,
    /// A button that helps: "windows-security" or "mac-privacy".
    fix: Option<&'static str>,
    /// The card offers "Send report" (a failure, and reports are set up).
    report: bool,
    /// Compress: the original and the result can be compared side by side.
    compare: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    via: Option<String>,
}

impl Done {
    fn plain(id: u64, ok: bool, title: String, body: String, can_undo: bool) -> Done {
        Done { id, ok, title, body, can_undo, attention: false, fix: None, report: false, compare: false, via: None }
    }
}

use crate::engine::{disk_size, file_name, human_size};

/// "Converted to MP3", or the action's own wording.
fn done_title(target_id: &str, label: &str) -> String {
    match target_id {
        "image.compress" | "raw.compress" | "video.compress" | "pdf.compress" | "audio.compress" if label == "Compress" => "Compressed".into(),
        "image.compress" | "raw.compress" | "video.compress" | "pdf.compress" | "audio.compress" => format!("Compressed to {label}"),
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
    wait_for_dialog: Option<isize>,
    size: Option<crate::size::Ask>,
) -> u64 {
    let id = crate::engine::new_id();
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
        emit(&app, "job-started", Started { id, title, family, ring, via: None });

        // From a Save dialog: the app hasn't written the file yet.
        if let Some(dialog) = wait_for_dialog {
            if let Err(reason) = wait_for_save(&app, id, dialog, &files) {
                let cancelled = reason == crate::procs::CANCELLED;
                emit(&app, "job-done", Done::plain(
                    id,
                    false,
                    if cancelled { "Cancelled".into() } else { "Nothing was saved".into() },
                    if cancelled { "Nothing was converted.".into() } else { reason },
                    false,
                ));
                return;
            }
        }

        let quality = quality.unwrap_or_else(|| crate::settings::get().quality);
        let (steps, mut notes) = match convert::plan_sized(&target_id, &files, &quality, size) {
            Ok(s) => s,
            // Every file is already small enough: nothing to do, and that's fine.
            Err(reason) if reason.starts_with(crate::size::ALREADY) => {
                let body = reason[crate::size::ALREADY.len()..].to_string();
                emit(&app, "job-done", Done { attention: true, ..Done::plain(id, true, "Nothing to do".into(), body, false) });
                return;
            }
            Err(reason) => {
                log::info!("job {id}: {target_id} not available: {reason}");
                let report = crate::report::remember(id, &target_id, &files, &[reason.clone()]);
                emit(&app, "job-done", Done { report, ..Done::plain(id, false, format!("Can't convert to {label} yet"), reason, false) });
                return;
            }
        };

        let total = steps.len();
        let started = Instant::now();
        let outcome = run_steps(&app, id, &steps);
        let errors = outcome.errors();
        let (made, cancelled) = (outcome.made, outcome.cancelled);
        notes.extend(outcome.notes);
        log::info!("job {id}: {} made, {} failed, {:?}", made.len(), errors.len(), started.elapsed());

        let size: u64 = made.iter().map(|p| disk_size(p)).sum();
        let report = !errors.is_empty() && crate::report::remember(id, &target_id, &files, &errors);
        let done = if cancelled {
            Done::plain(
                id,
                !made.is_empty(),
                "Cancelled".into(),
                if made.is_empty() {
                    "Nothing was saved.".into()
                } else {
                    format!("{} of {total} converted before you cancelled.", made.len())
                },
                !made.is_empty(),
            )
        } else if made.is_empty() {
            Done {
                report,
                ..Done::plain(
                    id,
                    false,
                    format!("Couldn't convert to {label}"),
                    errors.first().cloned().unwrap_or_else(|| "Unknown error".into()),
                    false,
                )
            }
        } else {
            let mut body = match made.as_slice() {
                [one] => format!("{} · {}", file_name(one), human_size(size)),
                many => format!("{} files · {}", many.len(), human_size(size)),
            };
            // Saved somewhere else because the original's folder couldn't be written.
            let mut fix = None;
            if let Some((folder, saved_in, why)) = moved_elsewhere(&files, &made) {
                body = format!("{body}\nSaved in {saved_in}: {}", why_text(why, &folder));
                fix = match why {
                    convert::Blocked::RansomwareProtection => Some("windows-security"),
                    convert::Blocked::Denied if cfg!(windows) => Some("windows-security"),
                    convert::Blocked::MacPrivacy => Some("mac-privacy"),
                    _ => None,
                };
            }
            if !errors.is_empty() {
                body.push_str(&format!("\n{} failed: {}", errors.len(), errors[0]));
            }
            for n in &notes {
                body.push_str(&format!("\n{n}"));
            }
            // Compress: what each result was made from, for Compare (not for sound).
            let compare = crate::size::Kind::of_target(&target_id).is_some_and(|k| k != crate::size::Kind::Audio);
            if compare {
                let made_from: Vec<(PathBuf, PathBuf)> = steps
                    .iter()
                    .filter_map(|st| made.iter().find(|m| m.file_stem().map(|x| x.to_string_lossy().starts_with(&st.stem)).unwrap_or(false)).map(|m| (st.inputs[0].clone(), m.clone())))
                    .collect();
                if let Ok(mut p) = pairs().lock() {
                    p.insert(id, made_from);
                }
            }
            let attention = fix.is_some() || body.contains('\n');
            Done { id, ok: true, title: done_title(&target_id, &label), body, can_undo: true, attention, fix, report, compare, via: None }
        };
        if let Ok(mut o) = outputs_map().lock() {
            o.insert(id, made);
        }
        emit(&app, "job-done", done);
    });
    id
}

/// The first original whose folder couldn't be written: (that folder's name,
/// where the result went instead, why).
fn moved_elsewhere(files: &[PathBuf], made: &[PathBuf]) -> Option<(String, String, convert::Blocked)> {
    let blocked = convert::blocked().lock().ok()?;
    let name = |d: &std::path::Path| d.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| d.display().to_string());
    files.iter().filter_map(|f| f.parent()).find_map(|dir| {
        let why = *blocked.get(dir)?;
        let saved = made.iter().filter_map(|m| m.parent()).find(|p| *p != dir)?;
        Some((name(dir), name(saved), why))
    })
}

/// Why the result isn't next to the original, in plain words.
fn why_text(why: convert::Blocked, folder: &str) -> String {
    match why {
        convert::Blocked::RansomwareProtection => format!(
            "Windows Security's ransomware protection doesn't let new apps save in {folder}. To save next to your files, allow Convertino there."
        ),
        convert::Blocked::MacPrivacy => format!(
            "macOS didn't let Convertino save in {folder}. Allow it under Privacy & Security › Files and Folders."
        ),
        convert::Blocked::ReadOnly => format!("{folder} is read-only."),
        convert::Blocked::Denied if cfg!(windows) => format!(
            "Windows didn't let Convertino save in {folder}. Security software can do this: in Windows Security, ransomware protection blocks new apps in Documents, Pictures, Videos and Desktop."
        ),
        convert::Blocked::Denied => format!("Convertino isn't allowed to save in {folder}."),
    }
}

/// Longest wait for a Save dialog to be used.
const SAVE_WAIT: Duration = Duration::from_secs(15 * 60);

/// Waits until the Save dialog has closed and its file exists and has stopped
/// growing (the app may still be writing it).
fn wait_for_save(app: &AppHandle, id: u64, dialog: isize, files: &[PathBuf]) -> Result<(), String> {
    let started = Instant::now();
    let mut closed_at: Option<Instant> = None;
    let mut last: Option<(u64, Instant)> = None;
    let mut told = false;
    loop {
        if crate::procs::is_cancelled(id) {
            return Err(crate::procs::CANCELLED.into());
        }
        if !told {
            emit(app, "job-progress", Progress { id, fraction: 0.0, detail: "Waiting for you to save…".into() });
            told = true;
        }
        let open = crate::selection::window_open(dialog);
        if !open && closed_at.is_none() {
            closed_at = Some(Instant::now());
        }
        let size: Option<u64> = files.iter().map(|f| f.metadata().ok().map(|m| m.len())).sum();
        match (size, last) {
            // Same size for a second after the dialog closed: done writing.
            (Some(s), Some((before, since))) if s == before && s > 0 && !open && since.elapsed() >= Duration::from_secs(1) => {
                return Ok(());
            }
            (Some(s), Some((before, _))) if s == before => {}
            (Some(s), _) => last = Some((s, Instant::now())),
            (None, _) => last = None,
        }
        // Closed without the file appearing within a minute: Save was cancelled.
        if size.is_none() && closed_at.is_some_and(|t| t.elapsed() > Duration::from_secs(60)) {
            return Err("The file wasn't saved, so there was nothing to convert.".into());
        }
        if started.elapsed() > SAVE_WAIT {
            return Err("Convertino stopped waiting for the file to be saved.".into());
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

/// A file saved by the PDF editor: shown as a done card with Open folder and Undo.
pub fn saved(app: &AppHandle, path: PathBuf) -> u64 {
    let id = crate::engine::new_id();
    let body = format!("{} · {}", file_name(&path), human_size(disk_size(&path)));
    if let Ok(mut o) = outputs_map().lock() {
        o.insert(id, vec![path]);
    }
    crate::show_hud(app);
    emit(app, "job-done", Done::plain(id, true, "Saved".into(), body, true));
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

/// Runs the steps through the engine, sending progress and downloads to the ring and the card.
fn run_steps(app: &AppHandle, id: u64, steps: &[convert::Step]) -> crate::engine::Outcome {
    let a = app.clone();
    let sink: crate::engine::Sink = std::sync::Arc::new(move |e| match e {
        crate::engine::Event::Progress { fraction, detail } | crate::engine::Event::Download { fraction, detail } => {
            emit(&a, "job-progress", Progress { id, fraction, detail })
        }
        crate::engine::Event::StepDone { .. } => {}
    });
    crate::engine::run_steps(id, steps, None, &sink)
}

/// Stops a running job (the corner card's Cancel). An AI app's job runs in
/// the MCP server's process: it's asked through a file it watches.
pub fn cancel(id: u64) {
    if id >= 1 << 40 {
        if let Some(dir) = crate::mcp::ai_dir().map(|d| d.join("cancel")) {
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(dir.join(id.to_string()), b"");
        }
        return;
    }
    crate::procs::cancel(id);
}

// ---------- jobs an AI app runs through MCP (see ai.rs) ----------

pub fn ai_started(app: &AppHandle, id: u64, title: String, via: String) {
    crate::show_hud(app);
    emit(app, "job-started", Started { id, title, family: "ai".into(), ring: false, via: Some(via) });
}

pub fn ai_progress(app: &AppHandle, id: u64, fraction: f64, detail: String) {
    emit(app, "job-progress", Progress { id, fraction, detail });
}

pub fn ai_done(app: &AppHandle, id: u64, ok: bool, title: String, body: String, outputs: Vec<PathBuf>, via: Option<String>) {
    let can_undo = ok && !outputs.is_empty();
    if let Ok(mut o) = outputs_map().lock() {
        o.insert(id, outputs);
    }
    crate::show_hud(app);
    emit(app, "job-done", Done { via, ..Done::plain(id, ok, title, body, can_undo) });
}

/// Shows the first output of a job selected in Explorer / Finder.
pub fn reveal(id: u64) -> Result<(), String> {
    let first = outputs_map()
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
    let files = outputs_map().lock().ok().and_then(|mut o| o.remove(&id)).unwrap_or_default();
    let existing: Vec<&PathBuf> = files.iter().filter(|p| p.exists()).collect();
    if existing.is_empty() {
        return Ok(0);
    }
    trash::delete_all(&existing).map_err(|e| format!("Couldn't move to the Recycle Bin: {e}"))?;
    log::info!("job {id}: undone, {} file(s) moved to the bin", existing.len());
    Ok(existing.len())
}
