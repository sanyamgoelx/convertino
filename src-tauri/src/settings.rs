//! Convertino's settings, kept in settings.json in the app's config folder
//! (%APPDATA%\com.convertino.app on Windows, ~/Library/Application Support on Mac).
//!
//! Everything has a default, so a missing or partly broken file just means
//! defaults for what's missing. Reads go through `get()` (a copy), writes
//! through `update()`, which saves straight away.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Quality {
    /// JPG quality, 50–100 (also JPG page images and resized JPGs).
    pub jpg: u32,
    /// WebP quality, 50–100.
    pub webp: u32,
    /// Resize: longest side in px.
    pub resize: u32,
    /// Longest side when converting images; 0 keeps the size.
    pub convert_max: u32,
    /// PDF page images, DPI.
    pub dpi: u32,
    /// PDF Compress: "small", "balanced" or "high".
    pub pdf_compress: String,
    /// MP3 bitrate, kbps (audio and audio out of video).
    pub mp3: u32,
    /// Re-encoded video: "small", "balanced" or "best".
    pub video: String,
    pub gif_width: u32,
    pub gif_seconds: u32,
    /// Pictures (JPG, WebP, AVIF; also PDF Compress and re-encoded video):
    /// "small", "balanced" or "best" finds the smallest file that still looks
    /// that good; "fixed" uses the JPG/WebP numbers above as they are.
    pub image: String,
}

impl Default for Quality {
    fn default() -> Self {
        Quality {
            jpg: 90,
            webp: 85,
            resize: 1920,
            convert_max: 0,
            dpi: 150,
            pdf_compress: "balanced".into(),
            mp3: 320,
            video: "balanced".into(),
            gif_width: 480,
            gif_seconds: 30,
            image: "balanced".into(),
        }
    }
}

impl Quality {
    /// Out-of-range values (a hand-edited file, an old version) back to something sane.
    pub fn clamped(mut self) -> Self {
        let d = Quality::default();
        self.jpg = self.jpg.clamp(30, 100);
        self.webp = self.webp.clamp(30, 100);
        self.resize = self.resize.clamp(64, 16384);
        if self.convert_max != 0 {
            self.convert_max = self.convert_max.clamp(64, 16384);
        }
        self.dpi = self.dpi.clamp(36, 1200);
        if !["small", "balanced", "high"].contains(&self.pdf_compress.as_str()) {
            self.pdf_compress = d.pdf_compress;
        }
        // 256 kbps used to be a choice; it's the same top VBR level as 320 now.
        self.mp3 = if self.mp3 >= 256 { 320 } else { self.mp3.clamp(64, 320) };
        if !["small", "balanced", "best"].contains(&self.video.as_str()) {
            self.video = d.video;
        }
        self.gif_width = self.gif_width.clamp(120, 1920);
        self.gif_seconds = self.gif_seconds.clamp(1, 600);
        if !["small", "balanced", "best", "fixed"].contains(&self.image.as_str()) {
            self.image = d.image;
        }
        self
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// The wheel's shortcut, as the global-shortcut plugin writes it
    /// ("ctrl+alt+shift+KeyC"); None: the first free default.
    pub shortcut: Option<String>,
    /// Alt+right-click (Option+right-click on Mac) opens the wheel.
    pub alt_click: bool,
    pub start_at_login: bool,
    /// The progress ring at the pointer; off: straight to the corner card.
    pub progress_ring: bool,
    /// "next" (next to the original) or "folder".
    pub save_mode: String,
    pub save_folder: Option<String>,
    /// Seconds before a running conversion moves from the ring to the corner card.
    pub handoff_seconds: u32,
    /// Put the most-picked formats first.
    pub learn: bool,
    /// Per kind of file: target ids in the order chosen in Settings.
    pub order: BTreeMap<String, Vec<String>>,
    /// Target ids hidden from the wheel.
    pub hidden: BTreeSet<String>,
    pub quality: Quality,
    /// "lgpl" (standard) or "gpl" (with x264 and x265).
    pub ffmpeg_build: String,
    /// How often each target was picked (for `learn`).
    pub picks: BTreeMap<String, u32>,
    /// AI apps connected through MCP may convert files.
    pub ai_apps: bool,
    /// A corner card shows while an AI app converts.
    pub ai_card: bool,
    /// "Ask Claude" on the wheel (shown only when Convertino is connected to Claude).
    pub ask_claude: bool,
    /// Where Ask Claude opens: "cowork", "chat" or "code" (see ask.rs).
    pub ask_mode: String,
    /// Settings format. Files without it are from before version 2.
    #[serde(default)]
    pub version: u32,
}

/// 2: FFmpeg defaults to the build with x264/x265 (smaller videos at the same look).
pub const VERSION: u32 = 2;

impl Default for Settings {
    fn default() -> Self {
        Settings {
            shortcut: None,
            alt_click: true,
            start_at_login: true,
            progress_ring: true,
            save_mode: "next".into(),
            save_folder: None,
            handoff_seconds: 2,
            learn: true,
            order: BTreeMap::new(),
            hidden: BTreeSet::new(),
            quality: Quality::default(),
            ffmpeg_build: "gpl".into(),
            picks: BTreeMap::new(),
            ai_apps: true,
            ai_card: true,
            ask_claude: true,
            ask_mode: "cowork".into(),
            version: VERSION,
        }
    }
}

impl Settings {
    fn clamped(mut self) -> Self {
        self.quality = self.quality.clamped();
        self.handoff_seconds = self.handoff_seconds.clamp(1, 10);
        if self.save_mode != "folder" {
            self.save_mode = "next".into();
        }
        if self.ffmpeg_build != "lgpl" {
            self.ffmpeg_build = "gpl".into();
        }
        if crate::ask::Mode::from_id(&self.ask_mode).is_none() {
            self.ask_mode = "cowork".into();
        }
        self
    }

    /// Brings settings from an older version up to date. True when something changed.
    fn migrated(&mut self) -> bool {
        if self.version >= VERSION {
            return false;
        }
        if self.version < 2 {
            // "lgpl" was only the old default, never a choice worth keeping.
            self.ffmpeg_build = "gpl".into();
        }
        self.version = VERSION;
        true
    }

    /// The folder all conversions go to, when "Always in one folder" is on.
    pub fn output_folder(&self) -> Option<PathBuf> {
        if self.save_mode != "folder" {
            return None;
        }
        self.save_folder.as_ref().map(PathBuf::from).filter(|p| !p.as_os_str().is_empty())
    }
}

struct Store {
    path: Option<PathBuf>,
    value: Settings,
    /// Loaded by the command line or MCP: never written.
    read_only: bool,
}

fn store() -> &'static RwLock<Store> {
    static S: OnceLock<RwLock<Store>> = OnceLock::new();
    S.get_or_init(|| RwLock::new(Store { path: None, value: Settings::default(), read_only: false }))
}

/// Loads settings.json from `dir`. Returns true when there was no file yet
/// (the first time Convertino runs).
pub fn init(dir: PathBuf) -> bool {
    let path = dir.join("settings.json");
    let mut migrated = false;
    let (value, first) = match std::fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<Settings>(&text) {
            Ok(mut s) => {
                migrated = s.migrated();
                (s.clamped(), false)
            }
            Err(e) => {
                log::warn!("settings.json couldn't be read ({e}); using defaults");
                let _ = std::fs::copy(&path, dir.join("settings.broken.json"));
                (Settings::default(), false)
            }
        },
        Err(_) => (Settings::default(), true),
    };
    if let Ok(mut s) = store().write() {
        s.path = Some(path);
        s.value = value;
    }
    if first || migrated {
        save();
    }
    first
}

/// The app's own config folder, the one Tauri's app_config_dir gives the
/// running app (the command line and the MCP server have no Tauri).
pub fn default_config_dir() -> Option<PathBuf> {
    const ID: &str = "com.crofty.convertino";
    // Tests point the command line at their own settings.
    if let Some(d) = std::env::var_os("CONVERTINO_CONFIG_DIR").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(d));
    }
    if cfg!(windows) {
        return std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join(ID));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        return home.map(|h| h.join("Library/Application Support").join(ID));
    }
    match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        Some(x) => Some(PathBuf::from(x).join(ID)),
        None => home.map(|h| h.join(".config").join(ID)),
    }
}

/// Reads settings.json without ever writing it (command line, MCP server):
/// a first run of the app must still see "no settings yet".
pub fn init_read_only(dir: PathBuf) {
    let path = dir.join("settings.json");
    let value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str::<Settings>(&t).ok())
        .map(|s| s.clamped())
        .unwrap_or_default();
    if let Ok(mut s) = store().write() {
        s.path = Some(path);
        s.value = value;
        s.read_only = true;
    }
}

/// The folder settings.json lives in (None before `init`, e.g. in tests).
pub fn config_dir() -> Option<PathBuf> {
    store().read().ok()?.path.as_ref()?.parent().map(PathBuf::from)
}

/// A copy of the current settings (defaults before `init`, e.g. in tests).
pub fn get() -> Settings {
    store().read().map(|s| s.value.clone()).unwrap_or_default()
}

/// Changes the settings and saves them.
pub fn update(f: impl FnOnce(&mut Settings)) -> Settings {
    let value = match store().write() {
        Ok(mut s) => {
            f(&mut s.value);
            s.value = s.value.clone().clamped();
            s.value.clone()
        }
        Err(_) => return get(),
    };
    save();
    value
}

fn save() {
    let Ok(s) = store().read() else { return };
    let Some(path) = &s.path else { return };
    if s.read_only {
        return;
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let text = serde_json::to_string_pretty(&s.value).unwrap_or_default();
    // Written next to it first, so a crash mid-write can't leave half a file.
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, text).is_ok() {
        if let Err(e) = std::fs::rename(&tmp, path) {
            log::warn!("couldn't save settings: {e}");
        }
    }
}

/// One more pick of `target_id` (for "Put the formats I pick most first").
pub fn record_pick(target_id: &str) {
    update(|s| *s.picks.entry(target_id.to_string()).or_insert(0) += 1);
}

// ---------------------------------------------------------------------------
// Start at sign-in.

/// Adds or removes Convertino from the programs that start at sign-in.
/// Development builds only log it, so `tauri dev` never lands in the startup list.
pub fn apply_start_at_login(on: bool) {
    if cfg!(debug_assertions) {
        log::info!("start at sign-in: {on} (not applied in development builds)");
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    if let Err(e) = login_item(on, &exe) {
        log::warn!("couldn't change start at sign-in: {e}");
    }
}

#[cfg(windows)]
fn login_item(on: bool, exe: &std::path::Path) -> Result<(), String> {
    use ::windows::core::HSTRING;
    use ::windows::Win32::System::Registry::{RegDeleteKeyValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ};
    let key = HSTRING::from(r"Software\Microsoft\Windows\CurrentVersion\Run");
    let name = HSTRING::from("Convertino");
    unsafe {
        if on {
            let value: Vec<u16> = format!("\"{}\" --background", exe.display()).encode_utf16().chain([0]).collect();
            let bytes = std::slice::from_raw_parts(value.as_ptr() as *const u8, value.len() * 2);
            RegSetKeyValueW(HKEY_CURRENT_USER, &key, &name, REG_SZ.0, Some(bytes.as_ptr() as *const _), bytes.len() as u32)
                .ok()
                .map_err(|e| e.message())
        } else {
            let r = RegDeleteKeyValueW(HKEY_CURRENT_USER, &key, &name);
            // Not there: nothing to remove.
            if r.is_ok() || r.0 == 2 { Ok(()) } else { Err(format!("error {}", r.0)) }
        }
    }
}

#[cfg(target_os = "macos")]
fn login_item(on: bool, exe: &std::path::Path) -> Result<(), String> {
    let home = std::env::var_os("HOME").ok_or("no home folder")?;
    let plist = PathBuf::from(home).join("Library/LaunchAgents/app.convertino.plist");
    if !on {
        let _ = std::fs::remove_file(&plist);
        return Ok(());
    }
    let exe = exe.display().to_string().replace('&', "&amp;").replace('<', "&lt;");
    let text = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>app.convertino</string>
  <key>ProgramArguments</key><array><string>{exe}</string><string>--background</string></array>
  <key>RunAtLoad</key><true/>
</dict>
</plist>
"#
    );
    if let Some(dir) = plist.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(&plist, text).map_err(|e| e.to_string())
}

#[cfg(not(any(windows, target_os = "macos")))]
fn login_item(_on: bool, _exe: &std::path::Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_get_defaults() {
        let s: Settings = serde_json::from_str(r#"{"altClick": false, "quality": {"jpg": 70}}"#).unwrap();
        assert!(!s.alt_click);
        assert_eq!(s.quality.jpg, 70);
        assert_eq!(s.quality.webp, 85);
        assert_eq!(s.handoff_seconds, 2);
        assert!(s.progress_ring);
    }

    #[test]
    fn bad_values_are_clamped() {
        let s: Settings = serde_json::from_str(
            r#"{"handoffSeconds": 99, "saveMode": "cloud", "ffmpegBuild": "x", "quality": {"jpg": 7, "video": "huge", "pdfCompress": "max"}}"#,
        )
        .unwrap();
        let s = s.clamped();
        assert_eq!(s.handoff_seconds, 10);
        assert_eq!(s.save_mode, "next");
        assert_eq!(s.ffmpeg_build, "gpl");
        assert_eq!(s.quality.jpg, 30);
        assert_eq!(s.quality.video, "balanced");
        assert_eq!(s.quality.pdf_compress, "balanced");
    }

    #[test]
    fn old_files_move_to_the_x264_build() {
        let mut old: Settings = serde_json::from_str(r#"{"ffmpegBuild": "lgpl"}"#).unwrap();
        assert_eq!(old.version, 0);
        assert!(old.migrated());
        assert_eq!((old.ffmpeg_build.as_str(), old.version), ("gpl", VERSION));
        // A choice made after the move stays.
        let mut chosen: Settings = serde_json::from_str(r#"{"ffmpegBuild": "lgpl", "version": 2}"#).unwrap();
        assert!(!chosen.migrated());
        assert_eq!(chosen.clamped().ffmpeg_build, "lgpl");
        assert_eq!(Settings::default().quality.image, "balanced");
    }

    #[test]
    fn output_folder_only_when_chosen() {
        let mut s = Settings { save_folder: Some("D:\\Converted".into()), ..Settings::default() };
        assert_eq!(s.output_folder(), None);
        s.save_mode = "folder".into();
        assert_eq!(s.output_folder(), Some(PathBuf::from("D:\\Converted")));
    }
}
