//! Updates from GitHub Releases (Tauri's updater: a signed latest.json next
//! to the installers; the signature is checked with the public key in
//! tauri.conf.json before anything is installed).
//!
//! Installed copies check once shortly after starting and then once a day.
//! A new version is announced (corner card, Settings > About); installing
//! it is one click there, and Convertino restarts on the new version.
//! Development builds never check.

use serde::Serialize;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;

#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UpdateView {
    /// The running version.
    pub current: String,
    /// A newer version, when one is available.
    pub available: Option<String>,
    /// Its release notes (Markdown from the GitHub release).
    pub notes: Option<String>,
    /// Development builds don't update.
    pub dev: bool,
}

/// The last check's result.
#[derive(Default)]
pub struct UpdateState(pub Mutex<Option<(String, Option<String>)>>);

fn view(app: &AppHandle) -> UpdateView {
    let found = app.state::<UpdateState>().0.lock().ok().and_then(|s| s.clone());
    UpdateView {
        current: app.package_info().version.to_string(),
        available: found.as_ref().map(|f| f.0.clone()),
        notes: found.and_then(|f| f.1),
        dev: cfg!(debug_assertions),
    }
}

async fn check(app: &AppHandle) -> Result<UpdateView, String> {
    if cfg!(debug_assertions) {
        return Ok(view(app));
    }
    let updater = app.updater().map_err(|e| e.to_string())?;
    let found = updater.check().await.map_err(|e| format!("Couldn't check for updates: {e}"))?;
    if let Ok(mut s) = app.state::<UpdateState>().0.lock() {
        *s = found.map(|u| (u.version.clone(), u.body.clone()));
    }
    Ok(view(app))
}

/// Settings > About: "Check now".
#[tauri::command]
pub async fn update_check(app: AppHandle) -> Result<UpdateView, String> {
    check(&app).await
}

/// What's known without checking again.
#[tauri::command]
pub fn update_status(app: AppHandle) -> UpdateView {
    view(&app)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    fraction: f64,
    detail: String,
}

/// Settings > About: "Update and restart". Downloads, checks the signature,
/// installs (on Windows the installer runs without questions) and restarts.
#[tauri::command]
pub async fn update_install(app: AppHandle) -> Result<(), String> {
    if cfg!(debug_assertions) {
        return Err("Development builds don't update.".into());
    }
    let updater = app.updater().map_err(|e| e.to_string())?;
    let update = updater
        .check()
        .await
        .map_err(|e| format!("Couldn't check for updates: {e}"))?
        .ok_or("Convertino is up to date.")?;
    log::info!("updating to {}", update.version);
    // Nothing should be converting while files are replaced.
    crate::procs::cancel_all();
    let mut got: u64 = 0;
    let progress_app = app.clone();
    update
        .download_and_install(
            move |chunk, total| {
                got += chunk as u64;
                let fraction = total.map(|t| got as f64 / t.max(1) as f64).unwrap_or(0.0);
                let _ = progress_app.emit_to(
                    "settings",
                    "update-progress",
                    Progress { fraction, detail: format!("Downloading… {}%", (fraction * 100.0).round()) },
                );
            },
            || log::info!("update downloaded; installing"),
        )
        .await
        .map_err(|e| format!("The update couldn't be installed: {e}"))?;
    app.restart();
}

/// Installed copies: check soon after starting, then daily; announce once per version.
pub fn start_background_checks(app: &AppHandle) {
    if cfg!(debug_assertions) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut announced: Option<String> = None;
        tokio_sleep(Duration::from_secs(20)).await;
        loop {
            match check(&app).await {
                Ok(v) => {
                    if let Some(new) = v.available.filter(|n| announced.as_ref() != Some(n)) {
                        log::info!("update available: {new}");
                        crate::show_hud(&app);
                        let _ = app.emit_to(
                            "hud",
                            "notice",
                            crate::Notice {
                                title: format!("Convertino {new} is ready"),
                                body: "Install it from the tray icon: Settings > About > Update and restart.".into(),
                            },
                        );
                        let _ = app.emit_to("settings", "settings-changed", ());
                        announced = Some(new);
                    }
                }
                Err(e) => log::info!("update check: {e}"),
            }
            tokio_sleep(Duration::from_secs(24 * 3600)).await;
        }
    });
}

async fn tokio_sleep(d: Duration) {
    // tauri's async runtime is tokio, but without tokio's timer feature here: sleep on a thread.
    let (tx, rx) = tauri::async_runtime::channel::<()>(1);
    std::thread::spawn(move || {
        std::thread::sleep(d);
        let _ = tx.blocking_send(());
    });
    let mut rx = rx;
    let _ = rx.recv().await;
}
