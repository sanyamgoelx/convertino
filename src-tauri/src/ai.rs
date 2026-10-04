//! The app's side of AI apps and the command line (Settings › AI & CLI).
//!
//! - Shows the corner card for jobs an AI app runs through `convertino mcp`:
//!   the MCP server writes progress lines to `<config>/ai/live/<pid>.jsonl`
//!   (see mcp.rs) and this watcher turns them into the same card a wheel
//!   job gets, with a "via Claude Desktop" badge.
//! - Commands for the Settings tab: status, Connect / Disconnect, the setup
//!   to copy, Install command (Mac), recent AI jobs.

use crate::{connect, jobs, mcp, settings, shell};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};
use tauri::AppHandle;

/// Starts watching for AI jobs, and points connected AI apps at this copy of Convertino.
pub fn start(app: AppHandle) {
    std::thread::spawn(|| {
        connect::repair();
        crate::ask::refresh();
    });
    std::thread::spawn(move || watch(app));
}

fn watch(app: AppHandle) {
    let Some(dir) = mcp::ai_dir().map(|d| d.join("live")) else { return };
    let _ = std::fs::create_dir_all(&dir);
    // What's there already happened before the app started: skip it, and
    // drop files left by servers that ended without tidying up.
    let mut offsets: HashMap<PathBuf, u64> = HashMap::new();
    for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let p = e.path();
        let old = e.metadata().ok().and_then(|m| m.modified().ok()).and_then(|t| SystemTime::now().duration_since(t).ok()).is_some_and(|age| age > Duration::from_secs(2 * 86400));
        if old {
            let _ = std::fs::remove_file(&p);
        } else {
            offsets.insert(p, e.metadata().map(|m| m.len()).unwrap_or(0));
        }
    }
    let mut via: HashMap<u64, String> = HashMap::new();
    loop {
        std::thread::sleep(Duration::from_millis(300));
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let files: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "jsonl")).collect();
        offsets.retain(|p, _| files.contains(p));
        for f in files {
            let pos = offsets.entry(f.clone()).or_insert(0);
            let Ok(mut file) = std::fs::File::open(&f) else { continue };
            let len = file.metadata().map(|m| m.len()).unwrap_or(0);
            if len < *pos {
                *pos = 0; // a new server took the same name
            }
            if len == *pos || file.seek(SeekFrom::Start(*pos)).is_err() {
                continue;
            }
            let mut text = String::new();
            if file.read_to_string(&mut text).is_err() {
                continue;
            }
            // Only whole lines; a line still being written waits for the next look.
            let Some(end) = text.rfind('\n') else { continue };
            *pos += (end + 1) as u64;
            for line in text[..end].lines() {
                if let Ok(v) = serde_json::from_str::<Value>(line) {
                    event(&app, &v, &mut via);
                }
            }
        }
    }
}

fn event(app: &AppHandle, v: &Value, via: &mut HashMap<u64, String>) {
    if !settings::get().ai_card {
        return;
    }
    let id = v["id"].as_u64().unwrap_or(0);
    let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
    match v["type"].as_str() {
        Some("started") => {
            via.insert(id, s("client"));
            jobs::ai_started(app, id, s("title"), s("client"));
        }
        Some("progress") => jobs::ai_progress(app, id, v["fraction"].as_f64().unwrap_or(0.0), s("detail")),
        Some("done") => {
            let outputs: Vec<PathBuf> = v["outputs"].as_array().map(|a| a.iter().filter_map(|p| p.as_str().map(PathBuf::from)).collect()).unwrap_or_default();
            jobs::ai_done(app, id, v["ok"].as_bool().unwrap_or(false), s("title"), s("body"), outputs, via.remove(&id));
        }
        _ => {}
    }
}

// ---------- Settings › AI & CLI ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiStatus {
    command: shell::CommandStatus,
    apps: Vec<connect::AppStatus>,
    jobs: Vec<Value>,
}

fn recent_jobs() -> Vec<Value> {
    let Some(path) = mcp::ai_dir().map(|d| d.join("jobs.jsonl")) else { return Vec::new() };
    let text = std::fs::read_to_string(path).unwrap_or_default();
    text.lines().rev().filter_map(|l| serde_json::from_str(l).ok()).take(12).collect()
}

fn status() -> AiStatus {
    AiStatus { command: shell::status(), apps: connect::status(), jobs: recent_jobs() }
}

#[tauri::command]
pub async fn ai_status() -> AiStatus {
    tauri::async_runtime::spawn_blocking(status).await.unwrap_or_else(|_| AiStatus { command: shell::status(), apps: Vec::new(), jobs: Vec::new() })
}

#[tauri::command]
pub async fn ai_connect(id: String, on: bool) -> Result<AiStatus, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let app = connect::App::from_id(&id).ok_or("Unknown app")?;
        if on { connect::connect(app) } else { connect::disconnect(app) }?;
        log::info!("ai: {} {}", if on { "connected" } else { "disconnected" }, app.name());
        crate::ask::refresh();
        Ok(status())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn ai_setup_snippet() -> String {
    connect::setup_snippet()
}

#[tauri::command]
pub async fn command_install() -> Result<AiStatus, String> {
    tauri::async_runtime::spawn_blocking(|| {
        shell::install_command()?;
        Ok(status())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn ai_jobs_clear() {
    if let Some(p) = mcp::ai_dir().map(|d| d.join("jobs.jsonl")) {
        let _ = std::fs::remove_file(p);
    }
}

/// Opens the folder of a recent AI job (only those: Settings can't open just anything).
#[tauri::command]
pub fn ai_open_folder(path: String) -> Result<(), String> {
    let known = recent_jobs().iter().any(|j| j["folder"].as_str() == Some(path.as_str()));
    if !known {
        return Err("That folder isn't one of the recent AI jobs.".into());
    }
    crate::open_in_file_manager(std::path::Path::new(&path))
}
