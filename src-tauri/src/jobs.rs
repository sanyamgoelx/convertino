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

/// Compare for something that isn't a job (Settings' "Test on a file"); returns its id.
pub fn register_compare(made_from: Vec<(PathBuf, PathBuf)>) -> u64 {
    let id = crate::engine::new_id();
    if let Ok(mut p) = pairs().lock() {
        p.insert(id, made_from);
    }
    id
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
    /// A folder job: the card offers "Stop after this file" and "Stop now".
    folder: bool,
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
    /// A second line ("Saved so far: 2.9 GB on 12 files").
    #[serde(skip_serializing_if = "Option::is_none")]
    extra: Option<String>,
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
    /// Folder jobs: every file's fate (the card's Details).
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<Vec<crate::folder::Row>>,
    /// Folder jobs stopped with "Stop after this file": how many are left ("Carry on").
    #[serde(skip_serializing_if = "Option::is_none")]
    resume: Option<usize>,
    /// Undo puts the originals back (the results had replaced them).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    replaced: bool,
    /// What the pointer ring's pill says instead of "Saved <file>" ("Installed 8 fonts").
    #[serde(skip_serializing_if = "Option::is_none")]
    pill: Option<String>,
}

impl Done {
    fn plain(id: u64, ok: bool, title: String, body: String, can_undo: bool) -> Done {
        Done { id, ok, title, body, can_undo, attention: false, fix: None, report: false, compare: false, via: None, details: None, resume: None, replaced: false, pill: None }
    }
}

use crate::engine::{disk_size, file_name, human_size};

/// "Converted to MP3", or the action's own wording.
fn done_title(target_id: &str, label: &str) -> String {
    match target_id {
        "image.compress" | "raw.compress" | "video.compress" | "pdf.compress" | "audio.compress" if label == "Compress" => "Compressed".into(),
        "image.compress" | "raw.compress" | "video.compress" | "pdf.compress" | "audio.compress" => format!("Compressed to {label}"),
        "audio.trim" => "Trimmed silence".into(),
        "video.mute" => "Removed the sound".into(),
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
    smart: Option<serde_json::Value>,
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
        emit(&app, "job-started", Started { id, title, family, ring, via: None, folder: false });

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
        let is_smart = crate::smart::is_smart(&target_id);
        let planned = if is_smart { crate::smart::plan(&target_id, &files, smart.as_ref()).map(|s| (s, Vec::new())) } else { convert::plan_sized(&target_id, &files, &quality, size) };
        let (steps, mut notes) = match planned {
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
                let title = if is_smart { format!("Couldn't {}", label.to_lowercase()) } else { format!("Can't convert to {label} yet") };
                emit(&app, "job-done", Done { report, ..Done::plain(id, false, title, reason, false) });
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
        // A smart action that wrote its own card (fonts: "Installed 8 fonts").
        let own = if is_smart && errors.is_empty() && !cancelled { crate::smart::take_done(id) } else { None };
        let done = if let Some((title, body, changed)) = own {
            let body = notes.iter().fold(body, |b, n| format!("{b}\n{n}"));
            Done { pill: Some(title.clone()), ..Done::plain(id, true, title, body, changed) }
        } else if cancelled {
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
                    if is_smart { format!("Couldn't {}", label.to_lowercase()) } else { format!("Couldn't convert to {label}") },
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
            let title = crate::smart::done_title(&target_id, made.len()).unwrap_or_else(|| done_title(&target_id, &label));
            let pill = match made.as_slice() {
                [one] if target_id == crate::smart::FIX_NAME => Some(format!("Renamed to {}", file_name(one))),
                _ => None,
            };
            Done { attention, fix, report, compare, pill, ..Done::plain(id, true, title, body, true) }
        };
        // Fonts, LUTs, presets and renames have their own Undo (smart::undo):
        // their results aren't moved to the bin.
        if let Ok(mut o) = outputs_map().lock() {
            o.insert(id, if crate::smart::own_undo(&target_id) { Vec::new() } else { made });
        }
        emit(&app, "job-done", done);
    });
    id
}

/// Runs steps planned elsewhere (the image editor's crops) as a job on the
/// corner card. `done`: the card's title when it worked; `exif_from`: a RAW
/// photo whose EXIF goes into every result; `keep`: held until the job ends
/// (the developed photo the steps read).
pub fn start_steps(
    app: AppHandle,
    title: String,
    family: String,
    done: String,
    files: Vec<PathBuf>,
    steps: Vec<convert::Step>,
    exif_from: Option<PathBuf>,
    keep: Option<std::sync::Arc<convert::TempDir>>,
) -> u64 {
    let id = crate::engine::new_id();
    std::thread::spawn(move || {
        let _keep = keep;
        crate::procs::set_current_job(id);
        report_downloads(&app, id);
        crate::show_hud(&app);
        emit(&app, "job-started", Started { id, title, family, ring: false, via: None, folder: false });
        let total = steps.len();
        let outcome = run_steps(&app, id, &steps);
        let errors = outcome.errors();
        let (made, cancelled) = (outcome.made, outcome.cancelled);
        if let Some(raw) = &exif_from {
            for m in &made {
                crate::raw::copy_exif(raw, m);
            }
        }
        log::info!("job {id}: {} made, {} failed", made.len(), errors.len());
        let size: u64 = made.iter().map(|p| disk_size(p)).sum();
        let report = !errors.is_empty() && crate::report::remember(id, "image.edit", &files, &errors);
        let result = if cancelled {
            Done::plain(id, !made.is_empty(), "Cancelled".into(), format!("{} of {total} saved before you cancelled.", made.len()), !made.is_empty())
        } else if made.is_empty() {
            Done { report, ..Done::plain(id, false, "Couldn't save".into(), errors.first().cloned().unwrap_or_else(|| "Unknown error".into()), false) }
        } else {
            let mut body = match made.as_slice() {
                [one] => format!("{} · {}", file_name(one), human_size(size)),
                many => format!("{} files · {}", many.len(), human_size(size)),
            };
            if !errors.is_empty() {
                body.push_str(&format!("\n{} failed: {}", errors.len(), errors[0]));
            }
            let attention = !errors.is_empty();
            Done { report, attention, ..Done::plain(id, true, done, body, true) }
        };
        if let Ok(mut o) = outputs_map().lock() {
            o.insert(id, made);
        }
        emit(&app, "job-done", result);
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
            emit(app, "job-progress", Progress { id, fraction: 0.0, detail: "Waiting for you to save…".into(), extra: None });
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
        emit(&reporter_app, "job-progress", Progress { id, fraction, detail: detail.to_string(), extra: None });
    });
}

/// Runs the steps through the engine, sending progress and downloads to the ring and the card.
fn run_steps(app: &AppHandle, id: u64, steps: &[convert::Step]) -> crate::engine::Outcome {
    let a = app.clone();
    let sink: crate::engine::Sink = std::sync::Arc::new(move |e| match e {
        crate::engine::Event::Progress { fraction, detail } | crate::engine::Event::Download { fraction, detail } => {
            emit(&a, "job-progress", Progress { id, fraction, detail, extra: None })
        }
        crate::engine::Event::StepDone { .. } | crate::engine::Event::StepStarted { .. } => {}
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
    emit(app, "job-started", Started { id, title, family: "ai".into(), ring: false, via: Some(via), folder: false });
}

pub fn ai_progress(app: &AppHandle, id: u64, fraction: f64, detail: String) {
    emit(app, "job-progress", Progress { id, fraction, detail, extra: None });
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
    select_in_folder(&first)
}

/// Opens the file's folder in Explorer / Finder with the file selected.
pub fn select_in_folder(first: &std::path::Path) -> Result<(), String> {
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
        std::process::Command::new("open").arg("-R").arg(first).spawn().map_err(|e| e.to_string())?;
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = first;
    }
    Ok(())
}

/// Moves a job's outputs to the Recycle Bin / Trash (folder jobs: their new
/// folders whole; after Replace, the originals come back too).
pub fn undo(id: u64) -> Result<usize, String> {
    let own = match crate::smart::undo(id) {
        Some(r) => r?,
        None => 0,
    };
    let files = outputs_map().lock().ok().and_then(|mut o| o.remove(&id)).unwrap_or_default();
    let replaced = folder_jobs().lock().ok().and_then(|mut f| f.get_mut(&id).map(|j| (std::mem::take(&mut j.replaced), j.started)));
    let mut n = own;
    if let Some((list, _)) = replaced.filter(|(l, _)| !l.is_empty()) {
        n += crate::folder::undo_replace(&list)?;
    }
    let existing: Vec<&PathBuf> = files.iter().filter(|p| p.exists()).collect();
    if existing.is_empty() {
        return Ok(n);
    }
    trash::delete_all(&existing).map_err(|e| format!("Couldn't move to the Recycle Bin: {e}"))?;
    log::info!("job {id}: undone, {} file(s) moved to the bin", existing.len());
    Ok(n + existing.len())
}

// ---------- folder jobs (see folder.rs) ----------

/// A folder job, kept for Undo of Replace and for "Carry on".
struct FolderJob {
    plan: crate::folder::Plan,
    /// Files that weren't started (Stop after this file).
    left: Vec<PathBuf>,
    replaced: Vec<(PathBuf, PathBuf, PathBuf)>,
    started: Option<std::time::SystemTime>,
    ring: bool,
}

fn folder_jobs() -> &'static Mutex<HashMap<u64, FolderJob>> {
    static F: OnceLock<Mutex<HashMap<u64, FolderJob>>> = OnceLock::new();
    F.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Starts a folder pick: plans it (measuring files can take a moment) and runs it. Returns its id.
#[allow(clippy::too_many_arguments)]
pub fn start_folder_planned(
    app: AppHandle,
    scan: std::sync::Arc<crate::folder::Scan>,
    action: crate::folder::Action,
    opts: crate::folder::Opts,
    quality: Option<crate::settings::Quality>,
    label: String,
    ring: bool,
) -> u64 {
    let id = crate::engine::new_id();
    std::thread::spawn(move || {
        crate::procs::set_current_job(id);
        report_downloads(&app, id);
        if !ring {
            crate::show_hud(&app);
        }
        emit(&app, "job-started", Started { id, title: format!("{label} · {}", scan.name()), family: "folder".into(), ring, via: None, folder: true });
        emit(&app, "job-progress", Progress { id, fraction: 0.0, detail: "Getting ready…".into(), extra: None });
        let prefs = crate::settings::get();
        let q = quality.unwrap_or_else(|| prefs.quality.clone());
        match crate::folder::plan(&scan, &action, &opts, &q, &prefs) {
            Ok(plan) => run_folder(&app, id, plan, ring),
            Err(e) if e.starts_with(crate::folder::ALREADY_DONE) => {
                let body = e[crate::folder::ALREADY_DONE.len()..].to_string();
                let body = format!("{}{}", body[..1].to_uppercase(), &body[1..]);
                emit(&app, "job-done", Done { attention: true, ..Done::plain(id, true, "Nothing to do".into(), body, false) });
            }
            Err(e) => {
                log::info!("folder job {id}: {e}");
                let cancelled = crate::procs::is_cancelled(id);
                let (title, body) = if cancelled { ("Stopped".to_string(), "Nothing was saved.".to_string()) } else { (format!("Couldn't start: {label}"), e) };
                emit(&app, "job-done", Done::plain(id, false, title, body, false));
            }
        }
    });
    id
}

/// Starts an already planned folder job ("Carry on") and returns its id.
pub fn start_folder(app: AppHandle, plan: crate::folder::Plan, ring: bool) -> u64 {
    let id = crate::engine::new_id();
    std::thread::spawn(move || {
        crate::procs::set_current_job(id);
        report_downloads(&app, id);
        if !ring {
            crate::show_hud(&app);
        }
        emit(&app, "job-started", Started { id, title: plan.title.clone(), family: "folder".into(), ring, via: None, folder: true });
        run_folder(&app, id, plan, ring);
    });
    id
}

/// Runs a folder job on this thread and reports it.
fn run_folder(app: &AppHandle, id: u64, plan: crate::folder::Plan, ring: bool) {
    let (a1, a2) = (app.clone(), app.clone());
    let on: crate::folder::OnLive = std::sync::Arc::new(move |l: crate::folder::Live| emit(&a1, "job-progress", Progress { id, fraction: l.fraction, detail: l.detail, extra: l.extra }));
    let dl: crate::folder::OnDownload = std::sync::Arc::new(move |fraction, detail| emit(&a2, "job-progress", Progress { id, fraction, detail, extra: None }));
    let started = Instant::now();
    let o = crate::folder::run(id, &plan, on, dl);
    log::info!("folder job {id}: {} done, {} failed, {} not started, {:?}", o.done, o.failed, o.not_started, started.elapsed());
    let done = folder_done(id, &plan, &o);
    if let Ok(mut out) = outputs_map().lock() {
        // Replace leaves nothing new to remove (Undo puts the originals back),
        // but Open folder still shows the results.
        out.insert(id, if o.undo.is_empty() { o.made.clone() } else { o.undo.clone() });
    }
    if !o.pairs.is_empty() && plan.action == crate::folder::Action::Compress {
        if let Ok(mut p) = pairs().lock() {
            p.insert(id, o.pairs.clone());
        }
    }
    if let Ok(mut f) = folder_jobs().lock() {
        f.insert(id, FolderJob { plan, left: o.left.clone(), replaced: o.replaced.clone(), started: o.started, ring });
    }
    emit(app, "job-done", done);
}

fn folder_done(id: u64, plan: &crate::folder::Plan, o: &crate::folder::Outcome) -> Done {
    use crate::folder::RowKind;
    let summary = crate::folder::summary(plan, o);
    let sizes = if o.before > 0 && o.after > 0 && o.before != o.after && !plan.is_single() {
        let pct = (o.after as f64 / o.before as f64 * 100.0).round();
        format!("{} → {} ({pct}% of the original)\n", human_size(o.before), human_size(o.after))
    } else if plan.is_single() && o.after > 0 {
        let made = o.made.first().map(|m| file_name(m)).unwrap_or_default();
        format!("{made} · {}\n", human_size(o.after))
    } else {
        String::new()
    };
    let mut body = format!("{sizes}{summary}");
    if let Some(h) = &o.holding {
        body.push_str(&format!("\nThe originals are in {}: delete that folder when you're happy with the results.", crate::holding::describe(h)));
    }
    for n in &o.notes {
        body.push_str(&format!("\n{n}"));
    }
    let made_any = !o.made.is_empty() || !o.replaced.is_empty();
    let compare = plan.action == crate::folder::Action::Compress && !o.pairs.is_empty();
    let attention = o.rows.iter().any(|r| r.kind != RowKind::Done) || !o.notes.is_empty();
    let details = Some(o.rows.clone());
    let replaced = !o.replaced.is_empty();
    if o.cancelled {
        let title = "Stopped".to_string();
        let body = if made_any { format!("{body}\nThe file in progress was removed.") } else { "Nothing was saved.".into() };
        return Done { details, replaced, attention: true, ..Done::plain(id, made_any, title, body, made_any) };
    }
    if o.stopped {
        let total = o.done + o.failed + o.kept + o.not_started;
        let title = format!("Stopped after {} of {total}", total - o.not_started);
        let body = format!("{body}\nThe other {} are untouched.", o.not_started);
        return Done { details, replaced, attention: true, resume: Some(o.not_started), compare, ..Done::plain(id, true, title, body, made_any) };
    }
    if !made_any {
        let first = o.rows.iter().find(|r| r.kind == RowKind::Failed).map(|r| format!("{}: {}", r.name, r.status));
        let kept = o.rows.iter().any(|r| r.kind == RowKind::Kept);
        if kept && o.failed == 0 {
            return Done { details, attention: true, ..Done::plain(id, true, "Nothing to do".into(), body, false) };
        }
        return Done { details, attention: true, ..Done::plain(id, false, format!("Couldn't {}", plan.title.to_lowercase()), first.unwrap_or(body), false) };
    }
    let title = match plan.title.split_once(' ') {
        _ if plan.is_single() => format!("{} · done", plan.title),
        _ => format!("{} · {}", plan.title, if o.failed > 0 { "done, with failures" } else { "done" }),
    };
    Done { details, replaced, attention, compare, ..Done::plain(id, true, title, body, true) }
}

/// "Stop after this file" on a folder job's card.
pub fn stop_after(id: u64) {
    crate::procs::stop_after_current(id);
}

/// "Carry on" on a stopped folder job: the files it didn't start, as a new job.
pub fn resume(app: AppHandle, id: u64) -> Result<u64, String> {
    let (mut plan, left, ring) = {
        let jobs = folder_jobs().lock().map_err(|_| "Busy")?;
        let j = jobs.get(&id).ok_or("That job is gone.")?;
        (j.plan.clone(), j.left.clone(), j.ring)
    };
    if left.is_empty() {
        return Err("Every file was done.".into());
    }
    let keep: std::collections::HashSet<PathBuf> = left.into_iter().collect();
    for g in plan.groups.iter_mut() {
        g.retain(|st| st.inputs.iter().any(|f| keep.contains(f)));
    }
    plan.groups.retain(|g| !g.is_empty());
    // The first job made the new folders; this one only adds to them.
    plan.new_roots.clear();
    plan.rows.retain(|r| r.kind == crate::folder::RowKind::Alone);
    let _ = ring;
    Ok(start_folder(app, plan, false))
}
