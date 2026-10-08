//! Convertino.
//!
//! Runs in the system tray (Windows) or menu bar (Mac). The hotkey or
//! Alt+right-click reads the files selected in Explorer / Finder and opens the
//! radial wheel at the cursor. A pick starts a background job and the wheel
//! shrinks into a progress ring at the same spot. A job that finishes within
//! two seconds ends there with a tick and Undo; a longer one, or a failure,
//! moves to the HUD's corner card (progress, Open folder, Undo).
//! Settings (tray icon) holds the shortcut, the wheel's order, quality and
//! the converters; the "main" window is an activity log for testing.

mod accent;
mod ai;
mod ask;
mod archive;
mod cli;
mod compare;
mod connect;
mod convert;
mod data;
mod engine;
mod image_edit;
mod install;
mod jobs;
mod look;
mod mcp;
mod procs;
mod raw;
mod report;
mod settings;
mod shell;
mod size;
mod tools;
mod update;
#[cfg(windows)]
mod alt_click;
#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
use mac as alt_click;
mod selection;
mod video;
mod wheel;
#[cfg(test)]
mod smoke;

use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use wheel::WheelModel;

/// Hotkeys to try, in order, when none is chosen in Settings. The first is the
/// default: Ctrl+Alt+C on Windows, Control+Option+C (⌃⌥C) on Mac. If another
/// app already owns it, the next free one is used.
const HOTKEY_CANDIDATES: &[&str] = &["ctrl+alt+c", "ctrl+alt+shift+c", "ctrl+alt+k", "ctrl+alt+shift+k"];

/// Wheel window size and where the wheel's centre sits inside it (logical px).
/// Keep in sync with ui/wheel.css.
const WHEEL_WIN_W: f64 = 420.0;
const WHEEL_WIN_H: f64 = 490.0;
const WHEEL_CX: f64 = 210.0;
const WHEEL_CY: f64 = 190.0;

/// The same window while it shows the progress ring: small, so it covers
/// nothing, with the ring's centre at (RING_CX, RING_CY) and its label to the right
/// (or to the left, mirrored, near the right edge of the screen). Keep in sync with ui/wheel.css.
const RING_WIN_W: f64 = 380.0;
const RING_WIN_H: f64 = 120.0;
const RING_CX: f64 = 56.0;
const RING_CY: f64 = 60.0;

/// HUD (progress and done cards) width and margin from the screen edge (logical px).
/// Includes room for the cards' shadow (see hud.css), so the window sits at the screen's edge.
const HUD_W: f64 = 408.0;
const HUD_MARGIN: f64 = 0.0;

/// Which hotkey is active, and which were taken by other apps.
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct HotkeyStatus {
    active: Option<String>,
    taken: Vec<String>,
}

#[derive(Default)]
struct AppState {
    hotkey: Mutex<HotkeyStatus>,
    /// The hotkey is switched off while Settings records a new one.
    hotkey_paused: Mutex<bool>,
    wheel: Mutex<Option<WheelModel>>,
    /// The job the progress ring is showing, while it shows one.
    ring: Mutex<Option<u64>>,
    /// Open PDF editor windows: window label -> the PDF it edits.
    editors: Mutex<HashMap<String, PathBuf>>,
    /// The window that was in front before the wheel opened (Windows), to give focus back.
    #[cfg_attr(not(windows), allow(dead_code))]
    previous_foreground: Mutex<Option<isize>>,
}

/// How the ring is laid out in its window.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RingLayout {
    /// Label to the left of the ring (the ring is near the right edge of the screen).
    flip: bool,
}

/// Sent to the main (debug) window every time the hotkey is pressed.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SelectionEvent {
    source: String,
    paths: Vec<String>,
    error: Option<String>,
    elapsed_ms: u128,
    at_unix_ms: u128,
}

/// Sent to the main window when a wheel slot is picked.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PickEvent {
    target_id: String,
    label: String,
    files: Vec<String>,
    at_unix_ms: u128,
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

// ---------- commands called from the windows ----------

#[tauri::command]
fn hotkey_status(state: tauri::State<'_, AppState>) -> HotkeyStatus {
    state.hotkey.lock().map(|h| h.clone()).unwrap_or_default()
}

/// The wheel window asks for the current model when it loads.
#[tauri::command]
fn wheel_model(state: tauri::State<'_, AppState>) -> Option<WheelModel> {
    state.wheel.lock().ok().and_then(|w| w.clone())
}

/// Starts the picked conversion and returns its job id. The wheel stays up
/// and turns into the progress ring (see ring_mode).
/// `quality`: Shift+click options for this conversion; `remember`: keep them in Settings.
#[tauri::command]
fn wheel_pick(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    target_id: String,
    label: String,
    family: String,
    quality: Option<settings::Quality>,
    remember: Option<bool>,
    size: Option<size::Ask>,
) -> u64 {
    settings::record_pick(&target_id);
    if wheel::opens_window(&target_id) {
        hide_wheel(&app);
        let first = state.wheel.lock().ok().and_then(|w| w.clone()).and_then(|m| m.files.into_iter().find(|f| f.family == family));
        if let Some(f) = first.filter(|f| std::path::Path::new(&f.path).is_file()) {
            if target_id == "pdf.edit" {
                open_editor(&app, PathBuf::from(f.path));
            } else {
                image_edit::open(&app, PathBuf::from(f.path));
            }
        }
        return 0;
    }
    let files: Vec<String> = state
        .wheel
        .lock()
        .ok()
        .and_then(|w| w.clone())
        .map(|m| {
            m.files
                .into_iter()
                .filter(|f| f.family == family && wheel::target_applies(&target_id, &f.ext))
                .map(|f| f.path)
                .collect()
        })
        .unwrap_or_default();
    log::info!("picked {target_id} ({label}) for {} file(s)", files.len());
    let _ = app.emit_to(
        "main",
        "picked",
        PickEvent { target_id: target_id.clone(), label: label.clone(), files: files.clone(), at_unix_ms: now_ms() },
    );
    if let (Some(q), Some(true)) = (&quality, remember) {
        let q = q.clone();
        settings::update(|s| s.quality = q);
        settings_changed(&app);
    }
    let ring = settings::get().progress_ring;
    if !ring {
        hide_wheel(&app);
    }
    let files = files.into_iter().map(std::path::PathBuf::from).collect();
    let pending = state.wheel.lock().ok().and_then(|w| w.as_ref().and_then(|m| m.pending_dialog));
    let id = jobs::start(app, target_id, label, family, files, ring, quality.map(|q| q.clamped()), pending, size);
    if ring {
        if let Ok(mut r) = state.ring.lock() {
            *r = Some(id);
        }
    }
    id
}

/// Ask Claude on the wheel: opens Claude with everything that was selected.
/// `mode`: Shift+click's choice (None: the one in Settings); `remember`: keep it in Settings.
#[tauri::command]
fn ask_claude(app: AppHandle, state: tauri::State<'_, AppState>, mode: Option<String>, remember: Option<bool>) -> Result<(), String> {
    hide_wheel(&app);
    let model = state.wheel.lock().ok().and_then(|w| w.clone()).ok_or("The wheel was closed")?;
    let ask_ui = model.ask.as_ref().ok_or("Convertino isn't connected to Claude")?;
    let available: Vec<ask::Mode> = ask_ui.modes.iter().filter_map(|m| ask::Mode::from_id(m.id)).collect();
    let chosen = ask::pick_mode(mode.as_deref().unwrap_or(ask_ui.default), &available).ok_or("Convertino isn't connected to Claude")?;
    if remember == Some(true) {
        settings::update(|s| s.ask_mode = chosen.id().into());
        settings_changed(&app);
    }
    let items: Vec<PathBuf> = model.selected.iter().map(PathBuf::from).filter(|p| p.exists()).collect();
    if items.is_empty() {
        return Err("The files aren't there any more".into());
    }
    let url = ask::link(chosen, &items);
    log::info!("ask claude: {} with {} item(s), {} chars", chosen.id(), items.len(), url.len());
    ask::open(&url).map_err(|e| {
        log::warn!("ask claude: couldn't open the link: {e}");
        show_hud(&app);
        let _ = app.emit_to("hud", "notice", Notice { title: "Couldn't open Claude".into(), body: format!("Is the Claude app installed? ({e})") });
        e
    })
}

/// The open wheel's files a target works on.
fn wheel_files(app: &AppHandle, target_id: &str, family: &str) -> Vec<PathBuf> {
    app.state::<AppState>()
        .wheel
        .lock()
        .ok()
        .and_then(|w| w.clone())
        .map(|m| {
            m.files
                .into_iter()
                .filter(|f| f.family == family && wheel::target_applies(target_id, &f.ext))
                .map(|f| PathBuf::from(f.path))
                .collect()
        })
        .unwrap_or_default()
}

/// What the size ring shows for the wheel's files.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SizeRing {
    title: String,
    count: usize,
    /// All the files together, in bytes.
    total: u64,
    presets: Vec<size::Preset>,
}

fn size_infos(app: &AppHandle, target_id: &str, family: &str) -> Result<(size::Kind, Vec<size::Info>), String> {
    let kind = size::Kind::of_target(target_id).ok_or("This isn't a Compress slot.")?;
    let files = wheel_files(app, target_id, family);
    if files.is_empty() {
        return Err("There's nothing to compress.".into());
    }
    let infos = files.iter().map(|f| size::quick_info(kind, f)).collect::<Result<Vec<_>, _>>()?;
    Ok((kind, infos))
}

/// The sizes offered on the ring after Compress is picked (measuring a video takes a moment).
#[tauri::command]
async fn compress_presets(app: AppHandle, target_id: String, family: String, together: bool) -> Result<SizeRing, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (kind, infos) = size_infos(&app, &target_id, &family)?;
        Ok(SizeRing {
            title: size::title(kind, &infos),
            count: infos.len(),
            total: infos.iter().map(|i| i.bytes).sum(),
            presets: size::presets(kind, &infos, together),
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// What a typed size would do (the centre of the size ring).
#[tauri::command]
async fn compress_preview(app: AppHandle, target_id: String, family: String, bytes: u64, together: bool, trim: Option<f64>) -> Result<size::Preview, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (kind, infos) = size_infos(&app, &target_id, &family)?;
        Ok(size::preview(kind, &infos, bytes, together, trim))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Opens the Compare window for a finished Compress job.
#[tauri::command]
fn compare_open(app: AppHandle, id: u64) -> Result<(), String> {
    if jobs::compare_pairs(id).is_empty() {
        return Err("There's nothing to compare.".into());
    }
    // Already open: show this job in it.
    if let Some(w) = app.get_webview_window("compare") {
        let _ = w.eval(&format!("location.hash = '{id}'; location.reload();"));
        let _ = w.unminimize();
        let _ = w.set_focus();
        return Ok(());
    }
    let app = app.clone();
    // Built off the calling thread: creating a window inside a command can deadlock on Windows.
    std::thread::spawn(move || {
        let built = WebviewWindowBuilder::new(&app, "compare", WebviewUrl::App(format!("compare.html#{id}").into()))
            .title("Compare – Convertino")
            .inner_size(1000.0, 760.0)
            .min_inner_size(560.0, 420.0)
            .center()
            .focused(true)
            .build();
        if let Err(e) = built {
            log::error!("couldn't open Compare: {e}");
        }
    });
    Ok(())
}

#[tauri::command]
fn compare_close(app: AppHandle) {
    if let Some(w) = app.get_webview_window("compare") {
        let _ = w.close();
    }
}

/// One original/result pair of a Compress job as pictures the window can show.
#[tauri::command]
async fn compare_data(id: u64, index: usize) -> Result<compare::Pair, String> {
    tauri::async_runtime::spawn_blocking(move || compare::pair(id, index)).await.map_err(|e| e.to_string())?
}

/// After the wheel's collapse animation: shrink the window around the ring,
/// let clicks through, and give focus back to the window the user was in.
#[tauri::command]
fn ring_mode(app: AppHandle) -> RingLayout {
    let mut layout = RingLayout { flip: false };
    let Some(win) = app.get_webview_window("wheel") else { return layout };
    let scale = win.scale_factor().unwrap_or(1.0);
    if let Ok(pos) = win.outer_position() {
        let (cx, cy) = (pos.x as f64 + WHEEL_CX * scale, pos.y as f64 + WHEEL_CY * scale);
        if let Some(m) = win.current_monitor().ok().flatten() {
            let area = m.work_area();
            layout.flip = cx + (RING_WIN_W - RING_CX) * scale > area.position.x as f64 + area.size.width as f64;
        }
        let ring_cx = if layout.flip { RING_WIN_W - RING_CX } else { RING_CX };
        let _ = win.set_size(LogicalSize::new(RING_WIN_W, RING_WIN_H));
        let _ = win.set_position(PhysicalPosition::new((cx - ring_cx * scale).round() as i32, (cy - RING_CY * scale).round() as i32));
    }
    let _ = win.set_ignore_cursor_events(true);
    #[cfg(windows)]
    {
        alt_click::RING_ACTIVE.store(true, std::sync::atomic::Ordering::SeqCst);
        let prev = app.state::<AppState>().previous_foreground.lock().ok().and_then(|p| *p);
        if let Some(h) = prev {
            unsafe {
                use ::windows::Win32::Foundation::HWND;
                use ::windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow;
                let _ = SetForegroundWindow(HWND(h as *mut core::ffi::c_void));
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        alt_click::RING_ACTIVE.store(true, std::sync::atomic::Ordering::SeqCst);
        // The wheel came from Finder (hotkey or Option+right-click): hand the focus back.
        let _ = std::process::Command::new("/usr/bin/osascript")
            .args(["-e", "tell application \"Finder\" to activate"])
            .spawn();
    }
    layout
}

/// The ring finished: its tick and Undo take the pointer for a moment.
#[tauri::command]
fn ring_interactive(app: AppHandle, on: bool) {
    if let Some(win) = app.get_webview_window("wheel") {
        let _ = win.set_ignore_cursor_events(!on);
    }
}

/// A long or failed job moves from the ring to the HUD's corner card.
#[tauri::command]
fn ring_handoff(app: AppHandle, id: u64) {
    if !ring_is(&app, id) {
        return; // a new wheel already took over (and handed this job on)
    }
    hand_off(&app, id);
    end_ring(&app);
}

/// The ring faded out after a quick job.
#[tauri::command]
fn ring_end(app: AppHandle, id: u64) {
    if ring_is(&app, id) {
        end_ring(&app);
    }
}

fn ring_is(app: &AppHandle, id: u64) -> bool {
    app.state::<AppState>().ring.lock().ok().and_then(|r| *r) == Some(id)
}

fn hand_off(app: &AppHandle, id: u64) {
    log::info!("job {id}: moved to the corner card");
    show_hud(app);
    let _ = app.emit_to("hud", "job-handoff", id);
}

/// Hides the ring and puts the window back to its wheel shape for next time.
fn end_ring(app: &AppHandle) {
    if let Ok(mut r) = app.state::<AppState>().ring.lock() {
        *r = None;
    }
    #[cfg(any(windows, target_os = "macos"))]
    alt_click::RING_ACTIVE.store(false, std::sync::atomic::Ordering::SeqCst);
    if let Some(win) = app.get_webview_window("wheel") {
        let _ = win.hide();
        let _ = win.set_ignore_cursor_events(false);
        let _ = win.set_size(LogicalSize::new(WHEEL_WIN_W, WHEEL_WIN_H));
    }
}

/// A click somewhere else while the ring shows (Windows): a running job moves
/// to the corner, a finished one clears away. Clicks on the ring itself don't count.
#[cfg(windows)]
fn on_click_while_ring(app: &AppHandle, x: i32, y: i32) {
    let Some(win) = app.get_webview_window("wheel") else { return };
    if let (Ok(pos), Ok(size)) = (win.outer_position(), win.outer_size()) {
        let inside = x >= pos.x && y >= pos.y && x < pos.x + size.width as i32 && y < pos.y + size.height as i32;
        if inside {
            return;
        }
    }
    let _ = win.emit("ring-away", ());
}

/// Mac: the same, with the click in screen points.
#[cfg(target_os = "macos")]
fn on_click_while_ring(app: &AppHandle, x: f64, y: f64) {
    let Some(win) = app.get_webview_window("wheel") else { return };
    let scale = win.scale_factor().unwrap_or(1.0);
    if let (Ok(pos), Ok(size)) = (win.outer_position(), win.outer_size()) {
        let (px, py) = (x * scale, y * scale);
        let inside = px >= pos.x as f64 && py >= pos.y as f64 && px < (pos.x + size.width as i32) as f64 && py < (pos.y + size.height as i32) as f64;
        if inside {
            return;
        }
    }
    let _ = win.emit("ring-away", ());
}

#[tauri::command]
fn job_reveal(id: u64) -> Result<(), String> {
    jobs::reveal(id)
}

#[tauri::command]
fn job_cancel(id: u64) {
    jobs::cancel(id);
}

#[tauri::command]
fn job_undo(id: u64) -> Result<usize, String> {
    jobs::undo(id)
}

/// "Send report" on a failed job's card.
#[tauri::command]
async fn job_report(app: AppHandle, id: u64, note: Option<String>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || report::send(&app, id, note))
        .await
        .map_err(|e| e.to_string())?
}

/// The card's fix button when the system stopped Convertino saving next to a file.
#[tauri::command]
fn open_fix(kind: String) -> Result<(), String> {
    let target = match kind.as_str() {
        // Windows Security: Virus & threat protection (Ransomware protection is on that page).
        "windows-security" if cfg!(windows) => "windowsdefender://threatsettings",
        "mac-privacy" if cfg!(target_os = "macos") => "x-apple.systempreferences:com.apple.preference.security?Privacy_FilesAndFolders",
        _ => return Err("Nothing to open here.".into()),
    };
    let program = if cfg!(windows) { "explorer.exe" } else { "open" };
    std::process::Command::new(program).arg(target).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// The HUD reports its content height; 0 hides it.
#[tauri::command]
fn hud_resize(app: AppHandle, height: f64) {
    let Some(win) = app.get_webview_window("hud") else { return };
    if height <= 0.0 {
        let _ = win.hide();
        return;
    }
    let _ = win.set_size(LogicalSize::new(HUD_W, height));
    place_hud(&app, height);
}

/// Bottom-right of the screen the pointer is on (top-right on Mac, like its notifications).
fn place_hud(app: &AppHandle, height: f64) {
    let Some(win) = app.get_webview_window("hud") else { return };
    let cursor = app.cursor_position().ok();
    let monitor = cursor
        .and_then(|c| app.monitor_from_point(c.x, c.y).ok().flatten())
        .or_else(|| win.primary_monitor().ok().flatten());
    let Some(m) = monitor else { return };
    let scale = m.scale_factor();
    let area = m.work_area();
    let (w, h, gap) = (HUD_W * scale, height * scale, HUD_MARGIN * scale);
    let x = area.position.x as f64 + area.size.width as f64 - w - gap;
    let y = if cfg!(target_os = "macos") {
        area.position.y as f64 + gap
    } else {
        area.position.y as f64 + area.size.height as f64 - h - gap
    };
    let _ = win.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
}

pub(crate) fn show_hud(app: &AppHandle) {
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(win) = app2.get_webview_window("hud") {
            if !win.is_visible().unwrap_or(false) {
                place_hud(&app2, 120.0);
            }
            let _ = win.show();
        }
    });
}

#[tauri::command]
fn wheel_close(app: AppHandle) {
    hide_wheel(&app);
}

// ---------- Settings ----------

/// Everything the Settings window shows.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SettingsView {
    settings: settings::Settings,
    /// The shortcut that works right now (may differ from the chosen one if it was taken).
    shortcut: Option<String>,
    families: Vec<wheel::FamilyInfo>,
    os: &'static str,
    version: String,
    accent: Option<String>,
    /// Development build: "Remove downloaded converters" and start at sign-in are inert.
    dev: bool,
}

fn settings_view(app: &AppHandle) -> SettingsView {
    let s = settings::get();
    SettingsView {
        families: wheel::families(&s),
        settings: s,
        shortcut: app.state::<AppState>().hotkey.lock().ok().and_then(|h| h.active.clone()),
        os: if cfg!(target_os = "macos") { "mac" } else { "win" },
        version: app.package_info().version.to_string(),
        accent: accent::system_accent(),
        dev: cfg!(debug_assertions),
    }
}

/// Tells the Settings window (if open) to reload.
fn settings_changed(app: &AppHandle) {
    let _ = app.emit_to("settings", "settings-changed", ());
}

#[tauri::command]
fn settings_get(app: AppHandle) -> SettingsView {
    settings_view(&app)
}

/// Merges `patch` (any subset of the settings, camelCase) into the settings.
#[tauri::command]
fn settings_set(app: AppHandle, patch: serde_json::Value) -> Result<SettingsView, String> {
    let before = settings::get();
    let mut merged = serde_json::to_value(&before).map_err(|e| e.to_string())?;
    merge_json(&mut merged, patch);
    let next: settings::Settings = serde_json::from_value(merged).map_err(|e| format!("That setting isn't valid: {e}"))?;
    let after = settings::update(|s| *s = next);
    #[cfg(any(windows, target_os = "macos"))]
    alt_click::ENABLED.store(after.alt_click, Ordering::SeqCst);
    if after.start_at_login != before.start_at_login {
        settings::apply_start_at_login(after.start_at_login);
    }
    log::info!("settings changed");
    Ok(settings_view(&app))
}

/// Objects merge key by key; anything else replaces.
fn merge_json(into: &mut serde_json::Value, patch: serde_json::Value) {
    match (into, patch) {
        (serde_json::Value::Object(a), serde_json::Value::Object(b)) => {
            for (k, v) in b {
                match a.get_mut(&k) {
                    Some(slot) if slot.is_object() && v.is_object() => merge_json(slot, v),
                    _ => {
                        a.insert(k, v);
                    }
                }
            }
        }
        (slot, v) => *slot = v,
    }
}

/// "ctrl+alt+shift+KeyC" -> "Ctrl+Alt+Shift+C" (⌃⌥⇧C on Mac).
fn pretty_shortcut(combo: &str) -> String {
    let mac = cfg!(target_os = "macos");
    let parts: Vec<String> = combo
        .split('+')
        .map(|p| match p.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => if mac { "⌃" } else { "Ctrl" }.to_string(),
            "alt" | "option" => if mac { "⌥" } else { "Alt" }.to_string(),
            "shift" => if mac { "⇧" } else { "Shift" }.to_string(),
            "super" | "cmd" | "command" | "meta" => if mac { "⌘" } else { "Win" }.to_string(),
            _ => {
                let k = p.strip_prefix("Key").or_else(|| p.strip_prefix("Digit")).unwrap_or(p);
                k.to_uppercase()
            }
        })
        .collect();
    parts.join(if mac { "" } else { "+" })
}

fn same_shortcut(a: &str, b: &str) -> bool {
    use tauri_plugin_global_shortcut::Shortcut;
    match (a.parse::<Shortcut>(), b.parse::<Shortcut>()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a.eq_ignore_ascii_case(b),
    }
}

/// Switches the wheel's shortcut. None: back to the default (the first free one).
#[tauri::command]
fn shortcut_set(app: AppHandle, shortcut: Option<String>) -> Result<String, String> {
    use tauri_plugin_global_shortcut::Shortcut;
    let state = app.state::<AppState>();
    let current = state.hotkey.lock().ok().and_then(|h| h.active.clone());
    let unregister_current = || {
        if let Some(c) = &current {
            let _ = app.global_shortcut().unregister(c.as_str());
        }
    };
    let Some(new) = shortcut else {
        unregister_current();
        settings::update(|s| s.shortcut = None);
        let status = register_hotkey(&app);
        let active = status.active.clone().unwrap_or_default();
        if let Ok(mut h) = state.hotkey.lock() {
            *h = status;
        }
        if let Ok(mut p) = state.hotkey_paused.lock() {
            *p = false;
        }
        return Ok(active);
    };
    if new.parse::<Shortcut>().is_err() {
        return Err("That key can't be part of a shortcut. Try a letter, a number or F1–F12.".into());
    }
    unregister_current();
    if let Err(e) = listen_hotkey(&app, &new) {
        log::info!("shortcut {new} not available: {e}");
        // Put the old one back.
        if let Some(c) = &current {
            let _ = listen_hotkey(&app, c);
        }
        return Err(format!("Another app is already using {}. Try a different one.", pretty_shortcut(&new)));
    }
    if current.as_deref().is_some_and(|c| !same_shortcut(c, &new)) {
        log::info!("hotkey changed to {new}");
    }
    if let Ok(mut h) = state.hotkey.lock() {
        h.active = Some(new.clone());
    }
    if let Ok(mut p) = state.hotkey_paused.lock() {
        *p = false;
    }
    let saved = new.clone();
    settings::update(|s| s.shortcut = Some(saved));
    let _ = app.emit_to("main", "hotkey-status", state.hotkey.lock().map(|h| h.clone()).unwrap_or_default());
    Ok(new)
}

/// While Settings records a new shortcut, the current one is switched off
/// (so pressing it doesn't open the wheel), and switched back on after.
#[tauri::command]
fn shortcut_pause(app: AppHandle, paused: bool) {
    let state = app.state::<AppState>();
    let Some(current) = state.hotkey.lock().ok().and_then(|h| h.active.clone()) else { return };
    let mut p = match state.hotkey_paused.lock() {
        Ok(p) => p,
        Err(_) => return,
    };
    if paused && !*p {
        let _ = app.global_shortcut().unregister(current.as_str());
        *p = true;
    } else if !paused && *p {
        let _ = listen_hotkey(&app, &current);
        *p = false;
    }
}

/// A folder picker for "Always in one folder".
#[tauri::command]
async fn choose_folder() -> Option<String> {
    tauri::async_runtime::spawn_blocking(pick_folder).await.ok().flatten()
}

#[cfg(windows)]
fn pick_folder() -> Option<String> {
    use ::windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
    use ::windows::Win32::UI::Shell::{FileOpenDialog, IFileOpenDialog, FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, SIGDN_FILESYSPATH};
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let result = (|| -> ::windows::core::Result<String> {
            let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
            dialog.SetOptions(dialog.GetOptions()? | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM)?;
            dialog.SetTitle(&::windows::core::HSTRING::from("Save converted files in"))?;
            dialog.Show(None)?;
            let item = dialog.GetResult()?;
            let path = item.GetDisplayName(SIGDN_FILESYSPATH)?;
            let text = path.to_string().unwrap_or_default();
            ::windows::Win32::System::Com::CoTaskMemFree(Some(path.0 as *const _));
            Ok(text)
        })();
        CoUninitialize();
        result.ok().filter(|p| !p.is_empty())
    }
}

#[cfg(target_os = "macos")]
fn pick_folder() -> Option<String> {
    let out = std::process::Command::new("osascript")
        .args(["-e", "POSIX path of (choose folder with prompt \"Save converted files in\")"])
        .output()
        .ok()?;
    let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !p.is_empty()).then_some(p)
}

#[cfg(not(any(windows, target_os = "macos")))]
fn pick_folder() -> Option<String> {
    None
}

/// Opens the folder with Convertino's logs (for reporting a problem).
#[tauri::command]
fn open_logs(app: AppHandle) -> Result<(), String> {
    let dir = app.path().app_log_dir().map_err(|e| e.to_string())?;
    let _ = std::fs::create_dir_all(&dir);
    open_in_file_manager(&dir)
}

pub(crate) fn open_in_file_manager(dir: &std::path::Path) -> Result<(), String> {
    let program = if cfg!(windows) { "explorer.exe" } else if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    std::process::Command::new(program).arg(dir).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Links Settings may open in the browser (nothing else, whatever a page asks).
const LINKS: &[&str] = &[
    "https://github.com/sponsors/sanyamgoelx",
    "https://github.com/sanyamgoelx/convertino",
    "https://github.com/sanyamgoelx/convertino/releases",
    "https://github.com/sanyamgoelx/convertino#command-line",
];

/// Opens one of `LINKS` in the default browser.
#[tauri::command]
fn open_link(url: String) -> Result<(), String> {
    if !LINKS.contains(&url.as_str()) {
        return Err("That link isn't one Convertino opens.".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // rundll32's URL handler opens the default browser without a console window.
        std::process::Command::new("rundll32.exe")
            .args(["url.dll,FileProtocolHandler", &url])
            .creation_flags(0x0800_0000)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        let program = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
        std::process::Command::new(program).arg(&url).spawn().map(|_| ()).map_err(|e| e.to_string())
    }
}

#[tauri::command]
fn show_activity(app: AppHandle) {
    show_main(&app);
}

#[tauri::command]
async fn converters_status() -> Vec<install::Status> {
    tauri::async_runtime::spawn_blocking(install::status).await.unwrap_or_default()
}

/// The video encoder in use (tried on a tiny clip the first time).
#[tauri::command]
async fn video_encoder() -> String {
    tauri::async_runtime::spawn_blocking(video::encoder_in_use).await.unwrap_or_default()
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConverterProgress {
    id: String,
    fraction: f64,
    detail: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConverterDone {
    id: String,
    ok: bool,
    message: String,
}

/// Downloads a converter now ("Download now", "Update", the other FFmpeg build).
/// Progress arrives as "converter-progress", the end as "converter-done".
#[tauri::command]
fn converter_download(app: AppHandle, id: String, replace: bool) -> Result<(), String> {
    let pack = install::Pack::from_id(&id).ok_or("Unknown converter.")?;
    std::thread::spawn(move || {
        let progress_app = app.clone();
        let progress_id = id.clone();
        install::set_reporter(move |fraction, detail| {
            let _ = progress_app.emit_to(
                "settings",
                "converter-progress",
                ConverterProgress { id: progress_id.clone(), fraction, detail: detail.to_string() },
            );
        });
        let result = if replace { install::reinstall(pack) } else { install::ensure(pack) };
        let done = match result {
            Ok(()) => ConverterDone { id, ok: true, message: format!("{} is ready.", pack.name()) },
            Err(e) => ConverterDone { id, ok: false, message: e },
        };
        let _ = app.emit_to("settings", "converter-done", done);
    });
    Ok(())
}

#[tauri::command]
async fn converters_remove() -> Result<u64, String> {
    tauri::async_runtime::spawn_blocking(install::remove_all).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn converters_check() -> Result<Vec<&'static str>, String> {
    tauri::async_runtime::spawn_blocking(install::check_updates).await.map_err(|e| e.to_string())?
}

/// Opens Settings (or brings it forward), optionally on a page.
fn show_settings(app: &AppHandle, page: Option<&str>) {
    if let Some(win) = app.get_webview_window("settings") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
        // It may have been hidden a while: show what's current.
        let _ = win.emit("settings-changed", ());
        if let Some(p) = page {
            let _ = win.emit("settings-page", p);
        }
        return;
    }
    let app = app.clone();
    let url = match page {
        Some(p) => format!("settings.html#{p}"),
        None => "settings.html".to_string(),
    };
    // Built off the calling thread: creating a window inside a command can deadlock on Windows.
    std::thread::spawn(move || {
        let built = WebviewWindowBuilder::new(&app, "settings", WebviewUrl::App(url.into()))
            .title("Convertino Settings")
            .inner_size(1000.0, 720.0)
            .min_inner_size(820.0, 560.0)
            .center()
            .focused(true)
            .build();
        if let Err(e) = built {
            log::error!("couldn't open Settings: {e}");
        }
    });
}

#[tauri::command]
fn open_settings(app: AppHandle, page: Option<String>) {
    show_settings(&app, page.as_deref());
}

// ---------- PDF editor ----------

/// Opens an editor window for one PDF (ui/editor.html).
fn open_editor(app: &AppHandle, path: PathBuf) {
    static N: AtomicU64 = AtomicU64::new(1);
    let label = format!("editor-{}", N.fetch_add(1, Ordering::SeqCst));
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    log::info!("editing {}", path.display());
    if let Ok(mut e) = app.state::<AppState>().editors.lock() {
        e.insert(label.clone(), path);
    }
    let app = app.clone();
    // Built off the calling thread: creating a window inside a command can deadlock on Windows.
    std::thread::spawn(move || {
        let built = WebviewWindowBuilder::new(&app, &label, WebviewUrl::App("editor.html".into()))
            .title(format!("{name} – Edit"))
            .inner_size(1240.0, 820.0)
            .min_inner_size(900.0, 600.0)
            .center()
            .focused(true)
            // Tauri's own file-drop handling swallows the page's drag and drop
            // on Windows, which the page grid uses to reorder pages.
            .disable_drag_drop_handler()
            .build();
        if let Err(e) = built {
            log::error!("couldn't open the PDF editor: {e}");
        }
    });
}

fn editor_path(app: &AppHandle, label: &str) -> Result<PathBuf, String> {
    app.state::<AppState>()
        .editors
        .lock()
        .ok()
        .and_then(|e| e.get(label).cloned())
        .ok_or_else(|| "This editor window has no PDF.".to_string())
}

#[derive(Clone, Serialize)]
struct EditorFile {
    name: String,
    /// The system accent colour (Windows), like the wheel's.
    accent: Option<String>,
}

#[tauri::command]
fn editor_file(app: AppHandle, window: tauri::WebviewWindow) -> Result<EditorFile, String> {
    let path = editor_path(&app, window.label())?;
    Ok(EditorFile {
        name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        accent: accent::system_accent(),
    })
}

/// The PDF's bytes, sent raw (no JSON) so large files load quickly.
#[tauri::command]
fn editor_bytes(app: AppHandle, window: tauri::WebviewWindow) -> Result<tauri::ipc::Response, String> {
    let path = editor_path(&app, window.label())?;
    let bytes = std::fs::read(&path).map_err(|e| format!("Couldn't read {}: {e}", path.display()))?;
    Ok(tauri::ipc::Response::new(bytes))
}

#[derive(Clone, Serialize)]
struct SavedFile {
    name: String,
}

/// Saves the edited PDF next to the original as "<name> (<suffix>).pdf";
/// the original is never touched. The body is the raw PDF.
#[tauri::command]
fn editor_save(app: AppHandle, window: tauri::WebviewWindow, request: tauri::ipc::Request<'_>) -> Result<SavedFile, String> {
    let original = editor_path(&app, window.label())?;
    // The PDF normally arrives raw. If the window's fast IPC channel ever
    // failed (Tauri then switches that window to postMessage for good), the
    // same bytes arrive as a JSON array of numbers instead: accept both.
    let from_json: Vec<u8>;
    let bytes: &[u8] = match request.body() {
        tauri::ipc::InvokeBody::Raw(raw) => raw,
        tauri::ipc::InvokeBody::Json(serde_json::Value::Array(items)) => {
            log::warn!("editor save arrived as JSON ({} bytes); the custom IPC protocol had failed", items.len());
            from_json = items
                .iter()
                .map(|v| v.as_u64().filter(|n| *n <= 255).map(|n| n as u8))
                .collect::<Option<Vec<u8>>>()
                .ok_or_else(|| "The edited PDF didn't arrive intact.".to_string())?;
            &from_json
        }
        _ => return Err("Nothing to save.".into()),
    };
    if !bytes.starts_with(b"%PDF") {
        return Err("The edited PDF came out empty.".into());
    }
    let suffix = request
        .headers()
        .get("x-convertino-suffix")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.chars().filter(|c| c.is_ascii_alphanumeric() || " ,-".contains(*c)).take(60).collect::<String>())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "edited".into());
    let dir = convert::output_dir(original.parent().unwrap_or(std::path::Path::new(".")));
    let stem = original.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "PDF".into());
    let name = convert::Reserved::new(&dir, &format!("{stem} ({})", suffix.trim()), "pdf");
    std::fs::write(&name.0, bytes).map_err(|e| format!("Couldn't save {}: {e}", name.0.display()))?;
    log::info!("editor saved {}", name.0.display());
    let file = name.0.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    jobs::saved(&app, name.0.clone());
    Ok(SavedFile { name: file })
}

/// The editor asked to close (after checking for unsaved changes).
#[tauri::command]
fn editor_close(app: AppHandle, window: tauri::WebviewWindow) {
    if let Ok(mut e) = app.state::<AppState>().editors.lock() {
        e.remove(window.label());
    }
    let _ = window.destroy();
}

// ---------- hotkey ----------

fn listen_hotkey(app: &AppHandle, combo: &str) -> Result<(), String> {
    app.global_shortcut()
        .on_shortcut(combo, |app, _shortcut, event| {
            if event.state == ShortcutState::Pressed {
                on_hotkey(app);
            }
        })
        .map_err(|e| e.to_string())
}

/// Registers the shortcut chosen in Settings, or else the first free one
/// from HOTKEY_CANDIDATES.
fn register_hotkey(app: &AppHandle) -> HotkeyStatus {
    let mut status = HotkeyStatus::default();
    if let Some(chosen) = settings::get().shortcut {
        match listen_hotkey(app, &chosen) {
            Ok(()) => {
                log::info!("hotkey registered: {chosen}");
                status.active = Some(chosen);
                return status;
            }
            Err(e) => {
                log::warn!("the chosen hotkey {chosen} is taken by another app ({e}); using a default");
                status.taken.push(chosen);
            }
        }
    }
    for &candidate in HOTKEY_CANDIDATES {
        let result = listen_hotkey(app, candidate);
        match result {
            Ok(()) => {
                log::info!("hotkey registered: {candidate}");
                status.active = Some(candidate.to_string());
                break;
            }
            Err(e) => {
                log::warn!("hotkey {candidate} is taken by another app: {e}");
                status.taken.push(candidate.to_string());
            }
        }
    }
    if status.active.is_none() {
        log::error!("no hotkey could be registered; all candidates are taken");
    }
    status
}

fn on_hotkey(app: &AppHandle) {
    let app = app.clone();
    // Where the pointer is when the key is pressed: the wheel opens there.
    let cursor = app.cursor_position().ok();
    // Read the selection on its own thread: COM (Windows) and osascript (Mac)
    // can take a moment, and the hotkey callback must return quickly.
    std::thread::spawn(move || {
        let started = Instant::now();
        let selection = selection::current_selection();
        open_for_selection(&app, selection, started, cursor, "hotkey");
    });
}

/// Alt+right-click (Windows): select the file under the pointer like a normal
/// right-click would, then open the wheel where the button was released.
#[cfg(windows)]
fn on_alt_click(app: &AppHandle, x: i32, y: i32, root: isize) {
    let started = Instant::now();
    match selection::select_item_at(x, y) {
        Ok(true) => {}
        Ok(false) => log::info!("alt-click: no file under the pointer, using the current selection"),
        Err(e) => log::warn!("alt-click: couldn't select the file under the pointer: {e}"),
    }
    let selection = selection::selection_for(Some(root));
    open_for_selection(app, selection, started, Some(PhysicalPosition::new(x as f64, y as f64)), "alt-click");
}

/// Option+right-click (Mac): the file under the pointer, or Finder's selection
/// when the pointer is on one of the selected files (like a normal right-click).
#[cfg(target_os = "macos")]
fn on_option_click(app: &AppHandle, x: f64, y: f64) {
    let started = Instant::now();
    let selection = selection::current_selection();
    let under = mac::item_at(x, y).map(|p| p.to_string_lossy().into_owned());
    let selection = match (selection, under) {
        (Ok(sel), Some(item)) if sel.paths.iter().any(|p| p.trim_end_matches('/') == item.trim_end_matches('/')) => Ok(sel),
        (_, Some(item)) => Ok(selection::Selection { source: "Finder".into(), paths: vec![item], ..Default::default() }),
        (sel, None) => {
            log::info!("option-click: no item found under the pointer; using Finder's selection");
            sel
        }
    };
    let cursor = app.cursor_position().ok();
    open_for_selection(app, selection, started, cursor, "option-click");
}

/// Mac permissions, for the Permissions page of Settings.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Permissions {
    mac: bool,
    /// Option+right-click (Accessibility).
    accessibility: bool,
    /// The tap is running (it starts within two seconds of the permission).
    alt_click_on: bool,
    /// Reading Finder's selection: "granted", "denied", "not-asked" or "unknown".
    finder: String,
}

#[cfg(target_os = "macos")]
#[tauri::command]
fn permissions_get() -> Permissions {
    Permissions {
        mac: true,
        accessibility: mac::accessibility_granted(),
        alt_click_on: mac::alt_click_running(),
        finder: mac::finder_automation(false).to_string(),
    }
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
fn permissions_get() -> Permissions {
    Permissions { mac: false, accessibility: true, alt_click_on: true, finder: "granted".into() }
}

/// Asks for a permission: "accessibility" (prompt + System Settings),
/// "finder" (macOS's own prompt; if it was refused before, System Settings).
#[cfg(target_os = "macos")]
#[tauri::command]
fn permissions_request(app: AppHandle, kind: String) {
    if kind == "accessibility" {
        mac::ask_accessibility();
    } else if kind == "finder" {
        std::thread::spawn(move || {
            let state = mac::ask_finder_once();
            if state == "denied" {
                mac::open_settings("Privacy_Automation");
            }
            let _ = app.emit_to("settings", "settings-changed", ());
        });
    }
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
fn permissions_request(_app: AppHandle, _kind: String) {}

/// Shared by the hotkey and Alt+right-click: log the selection, then open the
/// wheel, or explain in the main window why there's nothing to show.
fn open_for_selection(
    app: &AppHandle,
    selection: Result<selection::Selection, String>,
    started: Instant,
    cursor: Option<PhysicalPosition<f64>>,
    trigger: &str,
) {
    {
        let app = app.clone();
        let elapsed_ms = started.elapsed().as_millis();

        let pending_dialog;
        let paths = match selection {
            Ok(sel) => {
                pending_dialog = sel.pending_dialog;
                log::info!("{trigger}: {} item(s) from {} in {elapsed_ms} ms", sel.paths.len(), sel.source);
                for p in &sel.paths {
                    log::info!("  {p}");
                }
                let _ = app.emit_to(
                    "main",
                    "selection",
                    SelectionEvent {
                        source: sel.source,
                        paths: sel.paths.clone(),
                        error: None,
                        elapsed_ms,
                        at_unix_ms: now_ms(),
                    },
                );
                sel.paths
            }
            Err(err) => {
                log::warn!("{trigger}: could not read the selection: {err}");
                report_error(&app, err, elapsed_ms);
                return;
            }
        };

        match wheel::build(&paths, accent::system_accent()) {
            Ok(mut model) => {
                if pending_dialog.is_some() {
                    // Converts the file the app is about to save.
                    model.pending_dialog = pending_dialog;
                    model.hub_subtitle = "Converts after you click Save".into();
                }
                // Ask Claude needs files that exist (not one a Save dialog is about to write).
                if pending_dialog.is_none() {
                    model.ask = ask::for_wheel(&settings::get(), ask::connected());
                }
                let app2 = app.clone();
                // Window work must happen on the main thread.
                let _ = app.run_on_main_thread(move || open_wheel(&app2, model, cursor));
            }
            Err(msg) => {
                log::info!("{trigger}: nothing to convert: {msg}");
                report_error(&app, msg, elapsed_ms);
            }
        }
    }
}

/// Nothing to show on the wheel: say why in a corner card (and log it in the main window).
fn report_error(app: &AppHandle, error: String, elapsed_ms: u128) {
    let _ = app.emit_to(
        "main",
        "selection",
        SelectionEvent {
            source: String::new(),
            paths: Vec::new(),
            error: Some(error.clone()),
            elapsed_ms,
            at_unix_ms: now_ms(),
        },
    );
    show_hud(app);
    let _ = app.emit_to("hud", "notice", Notice { title: "Nothing to convert".into(), body: error });
}

#[derive(Clone, Serialize)]
pub(crate) struct Notice {
    pub(crate) title: String,
    pub(crate) body: String,
}

// ---------- windows ----------

fn open_wheel(app: &AppHandle, model: WheelModel, cursor: Option<PhysicalPosition<f64>>) {
    let Some(win) = app.get_webview_window("wheel") else {
        log::error!("wheel window is missing");
        return;
    };
    if let Ok(mut w) = app.state::<AppState>().wheel.lock() {
        *w = Some(model.clone());
    }
    // A ring still showing gives way: its job continues in the corner card.
    let ring = app.state::<AppState>().ring.lock().ok().and_then(|r| *r);
    if let Some(id) = ring {
        hand_off(app, id);
    }
    end_ring(app);
    #[cfg(windows)]
    if let Ok(mut p) = app.state::<AppState>().previous_foreground.lock() {
        let fg = unsafe { ::windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
        *p = (!fg.is_invalid()).then_some(fg.0 as isize);
    }

    // Centre the wheel on the cursor, kept inside the screen's work area.
    let cursor = cursor.or_else(|| app.cursor_position().ok());
    if let Some(c) = cursor {
        let monitor = app.monitor_from_point(c.x, c.y).ok().flatten();
        let scale = monitor.as_ref().map(|m| m.scale_factor()).unwrap_or(1.0);
        let (w, h) = (WHEEL_WIN_W * scale, WHEEL_WIN_H * scale);
        let mut x = c.x - WHEEL_CX * scale;
        let mut y = c.y - WHEEL_CY * scale;
        if let Some(m) = monitor {
            let area = m.work_area();
            let (ax, ay) = (area.position.x as f64, area.position.y as f64);
            let (aw, ah) = (area.size.width as f64, area.size.height as f64);
            x = x.clamp(ax, (ax + aw - w).max(ax));
            y = y.clamp(ay, (ay + ah - h).max(ay));
        }
        let _ = win.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
    }

    let _ = win.emit("wheel-open", model);
    let _ = win.show();
    let _ = win.set_focus();
}

fn hide_wheel(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("wheel") {
        let _ = win.hide();
    }
}

fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
/// The `convertino` command (src/bin/convertino-cli.rs). Returns the exit code.
pub fn cli_main() -> i32 {
    cli::main()
}

pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .build(),
        )
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(AppState::default())
        .manage(update::UpdateState::default())
        .invoke_handler(tauri::generate_handler![
            hotkey_status,
            open_link,
            job_report,
            open_fix,
            update::update_check,
            update::update_status,
            update::update_install,
            permissions_get,
            permissions_request,
            settings_get,
            settings_set,
            shortcut_set,
            shortcut_pause,
            choose_folder,
            open_logs,
            show_activity,
            open_settings,
            converters_status,
            video_encoder,
            converter_download,
            converters_remove,
            converters_check,
            wheel_model,
            wheel_pick,
            wheel_close,
            compress_presets,
            compress_preview,
            compare_open,
            compare_data,
            compare_close,
            ring_mode,
            ring_interactive,
            ring_handoff,
            ring_end,
            editor_file,
            editor_bytes,
            editor_save,
            editor_close,
            image_edit::imgedit_file,
            image_edit::imgedit_bytes,
            image_edit::imgedit_prepare,
            image_edit::imgedit_preview_bytes,
            image_edit::imgedit_save,
            image_edit::imgedit_close,
            job_reveal,
            job_cancel,
            ask_claude,
            ai::ai_status,
            ai::ai_connect,
            ai::ai_setup_snippet,
            ai::command_install,
            ai::ai_jobs_clear,
            ai::ai_open_folder,
            job_undo,
            hud_resize
        ])
        .setup(|app| {
            // Menu-bar app on Mac: no Dock icon.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let config = app.path().app_config_dir().unwrap_or_else(|_| std::env::temp_dir().join("Convertino"));
            let first_run = settings::init(config);
            let prefs = settings::get();
            #[cfg(any(windows, target_os = "macos"))]
            alt_click::ENABLED.store(prefs.alt_click, Ordering::SeqCst);
            // Keeps the startup entry pointing at this copy of Convertino (it may have moved).
            settings::apply_start_at_login(prefs.start_at_login);
            // Corner cards for AI apps' jobs; connected AI apps follow this copy if it moved.
            ai::start(app.handle().clone());
            // Settings moved to the FFmpeg build with x264/x265: swap an older copy in the background.
            if install::ffmpeg_build_differs() {
                std::thread::spawn(|| match install::match_ffmpeg_build() {
                    Ok(_) => {}
                    Err(e) => log::warn!("FFmpeg build switch: {e}"),
                });
            }

            let settings_item = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
            let activity = MenuItem::with_id(app, "show", "Activity log", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Convertino", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&settings_item, &activity, &quit])?;

            let mut tray = TrayIconBuilder::with_id("main-tray")
                .tooltip("Convertino")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "settings" => show_settings(app, None),
                    "show" => show_main(app),
                    "quit" => {
                        // Nothing keeps converting after Convertino quits.
                        procs::cancel_all();
                        app.exit(0)
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_settings(tray.app_handle(), None);
                    }
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;

            let status = register_hotkey(app.handle());
            if let Ok(mut h) = app.state::<AppState>().hotkey.lock() {
                *h = status.clone();
            }
            let _ = app.emit_to("main", "hotkey-status", status);

            #[cfg(windows)]
            {
                let handle = app.handle().clone();
                alt_click::install(move |x, y, root| on_alt_click(&handle, x, y, root));
                let handle = app.handle().clone();
                alt_click::on_click_while_ring(move |x, y| on_click_while_ring(&handle, x, y));
            }
            #[cfg(target_os = "macos")]
            {
                let handle = app.handle().clone();
                mac::install(move |x, y| on_option_click(&handle, x, y));
                let handle = app.handle().clone();
                mac::on_click_while_ring(move |x, y| on_click_while_ring(&handle, x, y));
            }
            // The first time: Settings says hello (and shows the shortcut).
            let background = std::env::args().any(|a| a == "--background");
            if first_run && !background {
                show_settings(app.handle(), None);
            }
            // Mac: anything not allowed yet opens Settings at the Permissions page.
            #[cfg(target_os = "macos")]
            {
                if !background && (!mac::accessibility_granted() || mac::finder_automation(false) != "granted") {
                    show_settings(app.handle(), Some("permissions"));
                }
            }
            update::start_background_checks(app.handle());
            log::info!("Convertino started");
            Ok(())
        })
        .on_window_event(|window, event| match (window.label(), event) {
            // The wheel closes as soon as you click anywhere else.
            // (Not while it shows the progress ring, which gives focus back on purpose.)
            ("wheel", WindowEvent::Focused(false)) => {
                let ring = window.app_handle().state::<AppState>().ring.lock().ok().and_then(|r| *r);
                if ring.is_none() {
                    let _ = window.hide();
                }
            }
            // An editor asks first if there are unsaved changes (it then calls editor_close).
            (label, WindowEvent::CloseRequested { api, .. }) if label.starts_with("editor-") => {
                api.prevent_close();
                let _ = window.emit_to(label, "editor-close-requested", ());
            }
            // Closing a window only hides it; the app keeps running in the tray.
            (_, WindowEvent::CloseRequested { api, .. }) => {
                api.prevent_close();
                let _ = window.hide();
            }
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("error while running Convertino");
}
