//! Convertino's entry in the shared tools folder, so Crofty's other tools can
//! find it (TOOLS-CONTRACT.md): `%LOCALAPPDATA%\sanyamgoelx\tools\convertino.json`.
//! Written at every start; best effort, never stops Convertino from starting.

use serde_json::json;
use std::path::PathBuf;

/// The shared tools folder.
pub fn dir() -> Option<PathBuf> {
    if cfg!(windows) {
        return std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("sanyamgoelx").join("tools"));
    }
    let home = PathBuf::from(std::env::var_os("HOME")?);
    Some(if cfg!(target_os = "macos") { home.join("Library/Application Support/sanyamgoelx/tools") } else { home.join(".local/share/sanyamgoelx/tools") })
}

pub fn announce() {
    // Development builds stay out of it: other tools should find the installed Convertino.
    if cfg!(debug_assertions) {
        return;
    }
    let Some(dir) = dir() else { return };
    let app = std::env::current_exe().ok().map(|p| crate::cli::strip_verbatim(&p));
    let entry = json!({
        "id": "convertino",
        "name": "Convertino",
        "version": env!("CARGO_PKG_VERSION"),
        "app": app.map(|p| p.to_string_lossy().into_owned()),
        "cli": crate::connect::cli_path().map(|p| p.to_string_lossy().into_owned()),
        "can": ["convert", "compress", "check-copy", "hold", "compare"],
        "repo": "sanyamgoelx/convertino",
        "written": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
    });
    if std::fs::create_dir_all(&dir).is_ok() {
        if let Ok(text) = serde_json::to_string_pretty(&entry) {
            let tmp = dir.join("convertino.json.tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, dir.join("convertino.json"));
            }
        }
    }
}
