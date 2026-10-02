//! Converters are downloaded the first time a conversion needs them, from
//! their official releases, and trimmed to the files Convertino uses.
//!
//! So the installer stays small: someone who only ever converts images never
//! downloads LibreOffice. Each converter gets its own folder under `root()`;
//! the folder only appears once it is complete, so an interrupted download
//! simply starts again next time. A converter that is already installed on
//! the computer (found by `tools::find`) is used as it is and never trimmed.

use crate::procs;
use crate::tools::{self, Tool};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Bumped when the trimming rules change, so older folders are trimmed again.
const TRIM_VERSION: u32 = 1;
const MARKER: &str = ".convertino";
const AGENT: &str = "Convertino";
/// A download that hasn't grown for this long is given up on.
const STALL: Duration = Duration::from_secs(90);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pack {
    SevenZip,
    Ffmpeg,
    Magick,
    Poppler,
    Ghostscript,
    Pandoc,
    LibreOffice,
}

impl Pack {
    pub const ALL: [Pack; 7] =
        [Pack::SevenZip, Pack::Ffmpeg, Pack::Magick, Pack::Poppler, Pack::Ghostscript, Pack::Pandoc, Pack::LibreOffice];

    pub fn of(tool: Tool) -> Pack {
        match tool {
            Tool::SevenZip => Pack::SevenZip,
            Tool::Ffmpeg => Pack::Ffmpeg,
            Tool::Magick => Pack::Magick,
            Tool::Pdftoppm | Tool::Pdftotext | Tool::Pdfseparate | Tool::Pdfunite => Pack::Poppler,
            Tool::Ghostscript => Pack::Ghostscript,
            Tool::Pandoc => Pack::Pandoc,
            Tool::LibreOffice => Pack::LibreOffice,
        }
    }

    pub fn name(self) -> &'static str {
        self.main_tool().display_name()
    }

    /// Folder name under the tools folder (the layouts in tools.rs use these).
    pub fn dir(self) -> &'static str {
        match self {
            Pack::SevenZip => "7zip",
            Pack::Ffmpeg => "ffmpeg",
            Pack::Magick => "imagemagick",
            Pack::Poppler => "poppler",
            Pack::Ghostscript => "ghostscript",
            Pack::Pandoc => "pandoc",
            Pack::LibreOffice => "libreoffice",
        }
    }

    fn main_tool(self) -> Tool {
        match self {
            Pack::SevenZip => Tool::SevenZip,
            Pack::Ffmpeg => Tool::Ffmpeg,
            Pack::Magick => Tool::Magick,
            Pack::Poppler => Tool::Pdftoppm,
            Pack::Ghostscript => Tool::Ghostscript,
            Pack::Pandoc => Tool::Pandoc,
            Pack::LibreOffice => Tool::LibreOffice,
        }
    }

    /// Every program of the pack Convertino runs.
    fn tools(self) -> &'static [Tool] {
        match self {
            Pack::Poppler => &[Tool::Pdftoppm, Tool::Pdftotext, Tool::Pdfseparate, Tool::Pdfunite],
            Pack::SevenZip => &[Tool::SevenZip],
            Pack::Ffmpeg => &[Tool::Ffmpeg],
            Pack::Magick => &[Tool::Magick],
            Pack::Ghostscript => &[Tool::Ghostscript],
            Pack::Pandoc => &[Tool::Pandoc],
            Pack::LibreOffice => &[Tool::LibreOffice],
        }
    }
}

/// Where downloaded converters go. Development builds use the project's
/// src-tauri/tools (filled by fetch-tools.cmd); installed builds a folder in
/// the user's profile, since the app's own folder isn't writable.
pub fn root() -> Option<PathBuf> {
    if cfg!(debug_assertions) {
        return Some(Path::new(env!("CARGO_MANIFEST_DIR")).join("tools"));
    }
    if cfg!(windows) {
        return std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Convertino").join("tools"));
    }
    let home = PathBuf::from(std::env::var_os("HOME")?);
    if cfg!(target_os = "macos") {
        Some(home.join("Library/Application Support/Convertino/tools"))
    } else {
        Some(home.join(".local/share/convertino/tools"))
    }
}

// ---------------------------------------------------------------------------
// Progress, reported to whoever runs the job on this thread.

type Reporter = Box<dyn Fn(f64, &str)>;

thread_local! {
    static REPORTER: RefCell<Option<Reporter>> = const { RefCell::new(None) };
}

/// Download progress on this thread goes to `f` (fraction, text for the card).
pub fn set_reporter(f: impl Fn(f64, &str) + 'static) {
    REPORTER.with(|r| *r.borrow_mut() = Some(Box::new(f)));
}

fn report(fraction: f64, detail: &str) {
    REPORTER.with(|r| {
        if let Some(f) = r.borrow().as_ref() {
            f(fraction.clamp(0.0, 1.0), detail);
        }
    });
}

// ---------------------------------------------------------------------------
// Installing.

/// One lock per pack: two jobs that need LibreOffice download it once.
static LOCKS: [Mutex<()>; 7] = [const { Mutex::new(()) }; 7];

fn installed(pack: Pack) -> bool {
    pack.tools().iter().all(|t| tools::find(*t).is_some())
}

/// Makes sure the pack's programs can be found, downloading them if needed.
pub fn ensure(pack: Pack) -> Result<(), String> {
    if installed(pack) {
        return Ok(());
    }
    if cfg!(target_os = "macos") && pack.bundled_on_mac() {
        return Err(format!(
            "{} comes with Convertino for Mac but is missing here. Download Convertino again, or install it with Homebrew (brew install {}).",
            pack.name(),
            pack.brew_name()
        ));
    }
    let Some(root) = root().filter(|_| downloads_supported()) else {
        return Err(format!("{} isn't installed on this computer yet.", pack.name()));
    };
    let _guard = LOCKS[pack as usize].lock().unwrap_or_else(|e| e.into_inner());
    if installed(pack) {
        return Ok(()); // another job got it while this one waited
    }
    if pack != Pack::SevenZip && cfg!(windows) {
        ensure(Pack::SevenZip)?; // unpacks everything else (a Mac has its own unzip)
    }
    log::info!("installing {} into {}", pack.name(), root.display());
    let _busy = Busy::new(pack);
    let started = Instant::now();
    let work = root.join(".work");
    let result = install(pack, &root, &work);
    let _ = fs::remove_dir_all(work.join(pack.dir()));
    match &result {
        Ok(()) => log::info!("{} ready in {:?}", pack.name(), started.elapsed()),
        Err(e) => log::warn!("{} not installed: {e}", pack.name()),
    }
    result
}

/// Windows and Mac download converters; elsewhere they come from the system.
fn downloads_supported() -> bool {
    cfg!(windows) || cfg!(target_os = "macos")
}

impl Pack {
    /// Shipped inside Convertino for Mac (no official Mac builds to download).
    pub fn bundled_on_mac(self) -> bool {
        matches!(self, Pack::Magick | Pack::Poppler | Pack::Ghostscript)
    }

    fn brew_name(self) -> &'static str {
        match self {
            Pack::Magick => "imagemagick",
            Pack::Poppler => "poppler",
            Pack::Ghostscript => "ghostscript",
            Pack::SevenZip => "sevenzip",
            Pack::Ffmpeg => "ffmpeg",
            Pack::Pandoc => "pandoc",
            Pack::LibreOffice => "--cask libreoffice",
        }
    }
}

fn install(pack: Pack, root: &Path, work: &Path) -> Result<(), String> {
    let source = fetch_and_unpack(pack, work)?;
    let name = pack.name();
    let staging = work.join(pack.dir()).join("ready");
    if cfg!(windows) {
        trim(pack, &staging);
    }
    mark(&staging, &source);
    let dest = root.join(pack.dir());
    if dest.exists() {
        // An older copy (an update), or a broken one find() didn't accept.
        fs::remove_dir_all(&dest).map_err(|e| format!("Couldn't replace the old {name} folder (is it in use?): {e}"))?;
    }
    fs::rename(&staging, &dest).map_err(|e| format!("Couldn't put {name} in place: {e}"))?;
    if pack == Pack::Ffmpeg {
        crate::video::forget_encoders();
    }
    Ok(())
}

/// Downloads and unpacks the pack into `work/<dir>/ready`; returns what was
/// downloaded (the file name, or LibreOffice's version), for the marker.
fn fetch_and_unpack(pack: Pack, work: &Path) -> Result<String, String> {
    let name = pack.name();
    let w = work.join(pack.dir());
    let _ = fs::remove_dir_all(&w);
    fs::create_dir_all(&w).map_err(|e| format!("Couldn't make a folder for {name}: {e}"))?;
    let staging = w.join("ready");
    let unpacked = w.join("unpacked");
    if cfg!(target_os = "macos") {
        return mac::fetch(pack, &w, &staging, &unpacked);
    }

    let source = match pack {
        Pack::SevenZip => {
            // 7zr.exe (only reads .7z) unpacks the full 7-Zip, whose installer is a .7z.
            let zr = github_download(&w, "ip7z/7zip", |n| n.eq_ignore_ascii_case("7zr.exe"), name)?;
            let setup = github_download(&w, "ip7z/7zip", |n| n.starts_with("7z") && n.ends_with("-x64.exe"), name)?;
            report(1.0, &format!("Setting up {name}…"));
            unpack_with(&zr, &setup, &staging, name)?;
            file_name(&setup)
        }
        Pack::Ffmpeg => {
            // The "shared" build is half the download of the single-file one.
            let build = ffmpeg_build();
            let zip = github_download(&w, "BtbN/FFmpeg-Builds", |n| {
                n.starts_with("ffmpeg-n") && n.contains(&format!("-win64-{build}-shared-")) && n.ends_with(".zip")
            }, name)?;
            report(1.0, &format!("Setting up {name}…"));
            unpack(&zip, &unpacked, name)?;
            promote(&unpacked, "ffmpeg.exe", 0, &staging)?;
            copy_license(&unpacked, &staging);
            file_name(&zip)
        }
        Pack::Magick => {
            let arc = github_download(&w, "ImageMagick/ImageMagick", |n| n.ends_with("-portable-Q16-x64.7z"), name)?;
            report(1.0, &format!("Setting up {name}…"));
            unpack(&arc, &unpacked, name)?;
            promote(&unpacked, "magick.exe", 0, &staging)?;
            file_name(&arc)
        }
        Pack::Poppler => {
            let zip = github_download(&w, "oschwartz10612/poppler-windows", |n| n.starts_with("Release-") && n.ends_with(".zip"), name)?;
            report(1.0, &format!("Setting up {name}…"));
            unpack(&zip, &unpacked, name)?;
            // Library/bin/pdftoppm.exe: keep Library (bin, and share/poppler for
            // the CJK encodings, which Poppler finds at bin/../share/poppler).
            promote(&unpacked, "pdftoppm.exe", 1, &staging)?;
            file_name(&zip)
        }
        Pack::Ghostscript => {
            ensure_vc_runtime()?;
            let exe = github_download(&w, "ArtifexSoftware/ghostpdl-downloads", |n| {
                n.starts_with("gs") && n.ends_with("w64.exe")
            }, name)?;
            report(1.0, &format!("Setting up {name}…"));
            // The installer is an NSIS archive: unpacked, not installed system-wide.
            unpack(&exe, &staging, name)?;
            file_name(&exe)
        }
        Pack::Pandoc => {
            let zip = github_download(&w, "jgm/pandoc", |n| n.ends_with("-windows-x86_64.zip"), name)?;
            report(1.0, &format!("Setting up {name}…"));
            unpack(&zip, &unpacked, name)?;
            promote(&unpacked, "pandoc.exe", 0, &staging)?;
            file_name(&zip)
        }
        Pack::LibreOffice => {
            let version = libreoffice_version()?;
            let msi = w.join("LibreOffice.msi");
            libreoffice_download(&format!("{version}/win/x86_64/LibreOffice_{version}_Win_x86-64.msi"), &msi, name)?;
            report(1.0, &format!("Setting up {name} (this takes a minute)…"));
            msi_unpack(&msi, &staging, name)?;
            format!("LibreOffice {version}")
        }
    };
    Ok(source)
}

/// "lgpl" or "gpl", from Settings.
fn ffmpeg_build() -> String {
    crate::settings::get().ffmpeg_build
}

/// Trims converters in the tools folder that were set up before trimming
/// existed (fetch-tools.cmd from older versions). Returns megabytes freed.
#[cfg_attr(not(test), allow(dead_code))]
pub fn trim_existing() -> u64 {
    if !cfg!(windows) {
        return 0; // the trimming rules are for the Windows downloads
    }
    let Some(root) = root() else { return 0 };
    let mut freed = 0;
    for pack in Pack::ALL {
        let dir = root.join(pack.dir());
        if dir.is_dir() && !trimmed(&dir) {
            let before = dir_size(&dir);
            trim(pack, &dir);
            mark(&dir, &source_of(&dir).unwrap_or_default());
            let mb = before.saturating_sub(dir_size(&dir)) / 1_000_000;
            log::info!("trimmed {}: {mb} MB freed", pack.name());
            freed += mb;
        }
    }
    freed
}

#[cfg_attr(not(test), allow(dead_code))]
fn trimmed(dir: &Path) -> bool {
    fs::read_to_string(dir.join(MARKER)).map(|s| s.contains(&format!("trim={TRIM_VERSION}"))).unwrap_or(false)
}

fn mark(dir: &Path, source: &str) {
    let _ = fs::write(dir.join(MARKER), format!("Set up by Convertino.\ntrim={TRIM_VERSION}\nsource={source}\n"));
}

/// What the folder was set up from (see `mark`), if Convertino set it up.
fn source_of(dir: &Path) -> Option<String> {
    let text = fs::read_to_string(dir.join(MARKER)).ok()?;
    text.lines().find_map(|l| l.strip_prefix("source=")).map(str::to_string).filter(|s| !s.is_empty())
}

/// Convertino's own FFmpeg is a different build from the one Settings asks
/// for (an older copy from before x264/x265 became the default).
pub fn ffmpeg_build_differs() -> bool {
    if cfg!(target_os = "macos") {
        return false; // the Mac builds always include x264/x265
    }
    let Some(dir) = root().map(|r| r.join(Pack::Ffmpeg.dir())).filter(|d| d.is_dir()) else { return false };
    let have = if source_of(&dir).is_some_and(|s| s.contains("-gpl-")) { "gpl" } else { "lgpl" };
    have != ffmpeg_build()
}

/// Swaps FFmpeg for the build Settings asks for, when they differ.
pub fn match_ffmpeg_build() -> Result<bool, String> {
    if !ffmpeg_build_differs() {
        return Ok(false);
    }
    log::info!("FFmpeg: switching to the {} build", ffmpeg_build());
    reinstall(Pack::Ffmpeg)?;
    Ok(true)
}

// ---------------------------------------------------------------------------
// For the Converters page of Settings.

fn busy() -> &'static Mutex<HashSet<Pack>> {
    static B: std::sync::OnceLock<Mutex<HashSet<Pack>>> = std::sync::OnceLock::new();
    B.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Marks the pack as downloading while alive.
struct Busy(Pack);
impl Busy {
    fn new(pack: Pack) -> Self {
        if let Ok(mut b) = busy().lock() {
            b.insert(pack);
        }
        Busy(pack)
    }
}
impl Drop for Busy {
    fn drop(&mut self) {
        if let Ok(mut b) = busy().lock() {
            b.remove(&self.0);
        }
    }
}

impl Pack {
    pub fn from_id(id: &str) -> Option<Pack> {
        Pack::ALL.into_iter().find(|p| p.dir() == id)
    }

    fn what(self) -> &'static str {
        match self {
            Pack::Ffmpeg => "Audio and video",
            Pack::Magick => "Images",
            Pack::Poppler => "PDF pages and text",
            Pack::Ghostscript => "PDF compress, grayscale",
            Pack::Pandoc => "Markdown and HTML",
            Pack::LibreOffice => "Word, Excel, PowerPoint",
            Pack::SevenZip => "Archives",
        }
    }

    /// Roughly what the first download weighs, in MB.
    fn download_mb(self) -> u32 {
        if cfg!(target_os = "macos") {
            return match self {
                Pack::SevenZip => 2,
                Pack::Ffmpeg => 30,
                Pack::Pandoc => 40,
                Pack::LibreOffice => 300,
                _ => 0, // comes with the app
            };
        }
        match self {
            Pack::SevenZip => 2,
            Pack::Ffmpeg if ffmpeg_build() == "gpl" => 86,
            Pack::Ffmpeg => 77,
            Pack::Magick => 11,
            Pack::Poppler => 42,
            Pack::Ghostscript => 62,
            Pack::Pandoc => 40,
            Pack::LibreOffice => 350,
        }
    }
}

#[derive(serde::Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub id: &'static str,
    pub name: &'static str,
    pub what: &'static str,
    /// "ready" (Convertino's own copy), "system" (installed on the computer),
    /// "downloading" or "missing".
    pub state: &'static str,
    pub version: Option<String>,
    /// On disk, for Convertino's own copy.
    pub size_mb: u64,
    pub download_mb: u32,
    /// FFmpeg only: "lgpl" or "gpl", when known.
    pub build: Option<String>,
}

pub fn status() -> Vec<Status> {
    let root = root();
    let downloading = busy().lock().map(|b| b.clone()).unwrap_or_default();
    Pack::ALL
        .into_iter()
        .map(|pack| {
            let ours = root.as_ref().map(|r| r.join(pack.dir())).filter(|d| d.is_dir() && installed_in(pack, d));
            let source = ours.as_deref().and_then(source_of);
            let state = if downloading.contains(&pack) {
                "downloading"
            } else if cfg!(target_os = "macos") && pack.bundled_on_mac() {
                if installed(pack) { "bundled" } else { "missing" }
            } else if ours.is_some() {
                "ready"
            } else if installed(pack) {
                "system"
            } else {
                "missing"
            };
            Status {
                id: pack.dir(),
                name: pack.name(),
                what: pack.what(),
                state,
                version: source.as_deref().and_then(|s| version_label(pack, s)),
                size_mb: ours.as_deref().map(dir_size).unwrap_or(0) / 1_000_000,
                download_mb: pack.download_mb(),
                build: source.as_deref().filter(|_| pack == Pack::Ffmpeg).map(|s| {
                    if s.contains("-gpl-") { "gpl".to_string() } else { "lgpl".to_string() }
                }),
            }
        })
        .collect()
}

/// The pack's programs are inside `dir` (find() may have found them elsewhere).
fn installed_in(pack: Pack, dir: &Path) -> bool {
    pack.tools().iter().all(|t| tools::find(*t).map(|p| p.starts_with(dir)).unwrap_or(false))
}

/// "8.1" from "ffmpeg-n8.1-latest-win64-lgpl-shared-8.1.zip", and so on.
fn version_label(pack: Pack, source: &str) -> Option<String> {
    let digits = |s: &str| -> Vec<String> {
        s.split(|c: char| !c.is_ascii_digit() && c != '.').filter(|p| p.chars().any(|c| c.is_ascii_digit())).map(|p| p.trim_matches('.').to_string()).collect()
    };
    let parts = digits(source);
    let v = match pack {
        Pack::Ghostscript => {
            // gs10080w64.exe -> 10.08.0
            let n = parts.first()?.trim_start_matches('.');
            let n: String = n.chars().filter(|c| c.is_ascii_digit()).collect();
            if n.len() < 4 {
                return None;
            }
            let (major, rest) = n.split_at(n.len() - 3);
            format!("{major}.{}.{}", &rest[..2], &rest[2..])
        }
        Pack::SevenZip => {
            // 7z2603-x64.exe -> 26.03 (the leading 7 is the name)
            let n: String = source.trim_start_matches("7z").chars().take_while(|c| c.is_ascii_digit()).collect();
            if n.len() < 3 {
                return None;
            }
            let (major, minor) = n.split_at(n.len() - 2);
            format!("{major}.{minor}")
        }
        Pack::Magick => {
            // ImageMagick-7.1.2-32-portable-Q16-x64.7z -> 7.1.2-32
            let rest = source.strip_prefix("ImageMagick-")?;
            rest.split("-portable").next()?.to_string()
        }
        Pack::Ffmpeg => parts.first()?.clone(),
        _ => parts.first()?.clone(),
    };
    Some(v)
}

/// The newest download's name (or LibreOffice's version), as `source_of` records it.
fn latest_source(pack: Pack) -> Result<String, String> {
    let name = pack.name();
    let asset = |repo: &str, pick: &dyn Fn(&str) -> bool| github_asset(repo, pick, name).map(|a| a.0);
    if cfg!(target_os = "macos") {
        return mac::latest_source(pack);
    }
    match pack {
        Pack::SevenZip => asset("ip7z/7zip", &|n| n.starts_with("7z") && n.ends_with("-x64.exe")),
        Pack::Ffmpeg => {
            let build = ffmpeg_build();
            asset("BtbN/FFmpeg-Builds", &|n| n.starts_with("ffmpeg-n") && n.contains(&format!("-win64-{build}-shared-")) && n.ends_with(".zip"))
        }
        Pack::Magick => asset("ImageMagick/ImageMagick", &|n| n.ends_with("-portable-Q16-x64.7z")),
        Pack::Poppler => asset("oschwartz10612/poppler-windows", &|n| n.starts_with("Release-") && n.ends_with(".zip")),
        Pack::Ghostscript => asset("ArtifexSoftware/ghostpdl-downloads", &|n| n.starts_with("gs") && n.ends_with("w64.exe")),
        Pack::Pandoc => asset("jgm/pandoc", &|n| n.ends_with("-windows-x86_64.zip")),
        Pack::LibreOffice => libreoffice_version().map(|v| format!("LibreOffice {v}")),
    }
}

/// Ids of Convertino's own converters that have a newer release.
pub fn check_updates() -> Result<Vec<&'static str>, String> {
    let Some(root) = root() else { return Ok(Vec::new()) };
    let mut newer = Vec::new();
    let mut last_error = None;
    for pack in Pack::ALL {
        let dir = root.join(pack.dir());
        if !dir.is_dir() {
            continue;
        }
        let current = source_of(&dir);
        match latest_source(pack) {
            // Folders from before sources were recorded count as up to date.
            Ok(latest) if current.as_deref().is_some_and(|c| c != latest) => newer.push(pack.dir()),
            Ok(_) => {}
            Err(e) => last_error = Some(e),
        }
    }
    match (newer.is_empty(), last_error) {
        (true, Some(e)) => Err(e),
        _ => Ok(newer),
    }
}

/// Downloads the pack even if a copy is there (an update, or the other FFmpeg build).
pub fn reinstall(pack: Pack) -> Result<(), String> {
    let Some(root) = root().filter(|_| downloads_supported() && !(cfg!(target_os = "macos") && pack.bundled_on_mac())) else {
        return Err(format!("{} can't be downloaded on this computer.", pack.name()));
    };
    let _busy = Busy::new(pack);
    let _guard = LOCKS[pack as usize].lock().unwrap_or_else(|e| e.into_inner());
    if pack != Pack::SevenZip && cfg!(windows) {
        ensure(Pack::SevenZip)?;
    }
    let work = root.join(".work");
    let result = install(pack, &root, &work);
    let _ = fs::remove_dir_all(work.join(pack.dir()));
    result
}

/// Deletes every converter Convertino downloaded (they come back on first use).
/// Returns MB freed. Development builds refuse: their tools folder is the project's.
pub fn remove_all() -> Result<u64, String> {
    if cfg!(debug_assertions) {
        return Err("Turned off in development builds, so the project's converters stay.".into());
    }
    let Some(root) = root() else { return Ok(0) };
    if !busy().lock().map(|b| b.is_empty()).unwrap_or(true) {
        return Err("Wait for the download to finish first.".into());
    }
    let mut freed = 0;
    let mut failed = Vec::new();
    for pack in Pack::ALL {
        let _guard = LOCKS[pack as usize].lock().unwrap_or_else(|e| e.into_inner());
        let dir = root.join(pack.dir());
        if dir.is_dir() {
            let size = dir_size(&dir);
            match fs::remove_dir_all(&dir) {
                Ok(()) => freed += size,
                Err(e) => {
                    log::warn!("couldn't remove {}: {e}", dir.display());
                    failed.push(pack.name());
                }
            }
        }
    }
    let _ = fs::remove_dir_all(root.join(".work"));
    crate::video::forget_encoders();
    if failed.is_empty() {
        Ok(freed / 1_000_000)
    } else {
        Err(format!("{} is in use right now, so it stayed. Try again when nothing is converting.", failed.join(", ")))
    }
}

// ---------------------------------------------------------------------------
// Trimming: only what Convertino runs stays.

fn trim(pack: Pack, dir: &Path) {
    match pack {
        Pack::SevenZip => keep_only(dir, &["7z.exe", "7z.dll", "License.txt", MARKER]),
        Pack::Ffmpeg => {
            remove(dir, &["doc", "include", "lib", "presets"]);
            keep_programs(dir, &["ffmpeg.exe"]);
        }
        Pack::Magick => {
            // The portable build has eight identical copies of the program.
            remove(dir, &["ChangeLog.md", "www", "images"]);
            keep_programs(dir, &["magick.exe"]);
        }
        Pack::Poppler => {
            let bin = if dir.join("bin").is_dir() { dir.join("bin") } else { dir.to_path_buf() };
            keep_programs(&bin, &["pdftoppm.exe", "pdftotext.exe", "pdfseparate.exe", "pdfunite.exe"]);
            if bin != dir {
                keep_only(dir, &["bin", "share", MARKER]);
                keep_only(&dir.join("share"), &["poppler"]);
            }
        }
        Pack::Ghostscript => remove(
            dir,
            &[
                "$PLUGINSDIR",
                "doc",
                "examples",
                "vcredist_x64.exe",
                "uninstgs.exe.nsis",
                "bin/gswin64.exe",
                "bin/gsdll64.lib",
            ],
        ),
        Pack::Pandoc => remove(dir, &["MANUAL.html"]),
        Pack::LibreOffice => trim_libreoffice(dir),
    }
}

/// LibreOffice loads its parts at run time, so this removes known extras
/// instead of following imports: the interface in other languages, spelling
/// dictionaries and thesauri, help, extra icon themes, PDF import.
/// Hyphenation patterns stay, since they can change how a document lays out.
fn trim_libreoffice(dir: &Path) {
    // An administrative install puts the Visual C++ runtime in System64
    // (normally installed system-wide). Next to soffice it's found without
    // installing anything.
    let program = dir.join("program");
    if let Ok(rd) = fs::read_dir(dir.join("System64")) {
        for e in rd.flatten() {
            let to = program.join(e.file_name());
            if !to.exists() {
                let _ = fs::copy(e.path(), to);
            }
        }
    }
    remove(
        dir,
        &[
            "help",
            "readmes",
            "Fonts", // goes to C:\Windows\Fonts in a normal install; unused here
            "System",
            "System64",
            "CREDITS.fodt",
            "program/resource",
            "program/xpdfimport.exe",
            "share/xpdfimport",
            "share/tipoftheday",
            "share/extensions/nlpsolver",
            "share/extensions/wiki-publisher",
        ],
    );
    for e in list(dir) {
        if file_name(&e).to_ascii_lowercase().ends_with(".msi") {
            let _ = fs::remove_file(e);
        }
    }
    let keep_lang = |name: &str| name.ends_with("en-US.xcd");
    for e in list(&dir.join("share/registry")) {
        let n = file_name(&e);
        if n.starts_with("Langpack-") && !keep_lang(&n) {
            let _ = fs::remove_file(&e);
        }
    }
    for e in list(&dir.join("share/registry/res")) {
        let n = file_name(&e);
        if (n.starts_with("registry_") || n.starts_with("fcfg_langpack_")) && !keep_lang(&n) {
            let _ = fs::remove_file(&e);
        }
    }
    for e in list(&dir.join("share/config")) {
        let n = file_name(&e);
        if n.starts_with("images_") && n.ends_with(".zip") && n != "images_colibre.zip" {
            let _ = fs::remove_file(&e);
        }
    }
    for d in list(&dir.join("share/extensions")) {
        if !file_name(&d).starts_with("dict-") {
            continue;
        }
        for e in list(&d) {
            let n = file_name(&e);
            let spelling = (n.ends_with(".dic") && !n.starts_with("hyph_")) || n.ends_with(".aff");
            if spelling || n.starts_with("th_") {
                let _ = fs::remove_file(&e);
            }
        }
    }
}

fn list(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir).map(|rd| rd.flatten().map(|e| e.path()).collect()).unwrap_or_default()
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn remove_path(p: &Path) {
    let r = if p.is_dir() { fs::remove_dir_all(p) } else { fs::remove_file(p) };
    if let Err(e) = r {
        if p.exists() {
            log::warn!("couldn't remove {}: {e}", p.display());
        }
    }
}

fn remove(dir: &Path, rels: &[&str]) {
    for rel in rels {
        let p = dir.join(rel);
        if p.exists() {
            remove_path(&p);
        }
    }
}

/// Removes everything in `dir` except the named entries (case-insensitive).
fn keep_only(dir: &Path, keep: &[&str]) {
    for e in list(dir) {
        let n = file_name(&e);
        if !keep.iter().any(|k| k.eq_ignore_ascii_case(&n)) {
            remove_path(&e);
        }
    }
}

/// Keeps `programs` and the DLLs they load (following PE imports, also from
/// DLL to DLL); every other .exe and .dll in `dir` goes.
fn keep_programs(dir: &Path, programs: &[&str]) {
    let files: HashMap<String, PathBuf> = list(dir)
        .into_iter()
        .filter(|p| p.is_file())
        .map(|p| (file_name(&p).to_ascii_lowercase(), p))
        .collect();
    let needed = closure(&files, programs);
    if needed.is_empty() {
        return; // the programs aren't here: leave the folder alone
    }
    for (name, path) in &files {
        if (name.ends_with(".exe") || name.ends_with(".dll")) && !needed.contains(name) {
            remove_path(path);
        }
    }
}

fn closure(files: &HashMap<String, PathBuf>, programs: &[&str]) -> HashSet<String> {
    let mut needed = HashSet::new();
    let mut queue: VecDeque<String> = programs.iter().map(|p| p.to_ascii_lowercase()).collect();
    while let Some(name) = queue.pop_front() {
        let Some(path) = files.get(&name) else { continue };
        if !needed.insert(name) {
            continue;
        }
        for dll in pe_imports(path) {
            let dll = dll.to_ascii_lowercase();
            if files.contains_key(&dll) && !needed.contains(&dll) {
                queue.push_back(dll);
            }
        }
    }
    needed
}

/// Names of the DLLs a Windows program or DLL imports (normal and delay-loaded).
/// Anything that isn't a readable PE file gives an empty list.
fn pe_imports(path: &Path) -> Vec<String> {
    fs::read(path).map(|b| pe_imports_of(&b)).unwrap_or_default()
}

fn pe_imports_of(b: &[u8]) -> Vec<String> {
    let u16_at = |o: usize| b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]]) as usize);
    let u32_at = |o: usize| b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize);
    let mut out = Vec::new();
    let Some(pe) = u32_at(0x3c) else { return out };
    if b.get(pe..pe + 4) != Some(b"PE\0\0") {
        return out;
    }
    let (Some(sections), Some(opt_size)) = (u16_at(pe + 6), u16_at(pe + 20)) else { return out };
    let opt = pe + 24;
    let dirs = match u16_at(opt) {
        Some(0x10b) => opt + 96,  // PE32
        Some(0x20b) => opt + 112, // PE32+
        _ => return out,
    };
    let table = opt + opt_size;
    let to_offset = |rva: usize| -> Option<usize> {
        (0..sections).find_map(|i| {
            let s = table + i * 40;
            let (vsize, va, raw_size, raw) = (u32_at(s + 8)?, u32_at(s + 12)?, u32_at(s + 16)?, u32_at(s + 20)?);
            (rva >= va && rva < va + vsize.max(raw_size)).then(|| rva - va + raw)
        })
    };
    let c_str = |o: usize| -> Option<String> {
        let end = b.get(o..)?.iter().position(|&c| c == 0)?;
        Some(String::from_utf8_lossy(&b[o..o + end]).into_owned())
    };
    // Data directory 1: imports (20-byte entries, name at +12).
    // Data directory 13: delay-loaded imports (32-byte entries, name at +4).
    for (index, entry, name_at) in [(1usize, 20usize, 12usize), (13, 32, 4)] {
        let Some(rva) = u32_at(dirs + index * 8) else { continue };
        if rva == 0 {
            continue;
        }
        let Some(mut o) = to_offset(rva) else { continue };
        for _ in 0..4096 {
            let Some(name_rva) = u32_at(o + name_at) else { break };
            if name_rva == 0 {
                break;
            }
            if let Some(name) = to_offset(name_rva).and_then(c_str) {
                out.push(name);
            }
            o += entry;
        }
    }
    out
}

fn dir_size(p: &Path) -> u64 {
    match p.symlink_metadata() {
        Ok(m) if m.is_dir() => list(p).iter().map(|c| dir_size(c)).sum(),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

// ---------------------------------------------------------------------------
// Unpacking.

/// Moves the folder holding `marker` (or the folder `up` levels above it) to `to`.
fn promote(from: &Path, marker: &str, up: usize, to: &Path) -> Result<(), String> {
    let found = find_file(from, marker, 6).ok_or_else(|| format!("{marker} wasn't in the download."))?;
    let mut dir = found.parent().map(Path::to_path_buf).unwrap_or_default();
    for _ in 0..up {
        dir = dir.parent().map(Path::to_path_buf).unwrap_or(dir);
    }
    fs::rename(&dir, to).map_err(|e| format!("Couldn't set up the download: {e}"))
}

fn find_file(dir: &Path, name: &str, depth: usize) -> Option<PathBuf> {
    let entries = list(dir);
    if let Some(f) = entries.iter().find(|p| p.is_file() && file_name(p).eq_ignore_ascii_case(name)) {
        return Some(f.clone());
    }
    if depth == 0 {
        return None;
    }
    entries.iter().filter(|p| p.is_dir()).find_map(|d| find_file(d, name, depth - 1))
}

/// FFmpeg's licence sits at the top of its download, outside bin.
fn copy_license(from: &Path, to: &Path) {
    if let Some(l) = find_file(from, "LICENSE.txt", 2) {
        let _ = fs::copy(l, to.join("LICENSE.txt"));
    }
}

fn unpack(archive: &Path, to: &Path, name: &str) -> Result<(), String> {
    let seven = tools::find(Tool::SevenZip).ok_or("7-Zip is missing, so the download can't be unpacked.")?;
    unpack_with(&seven, archive, to, name)
}

fn unpack_with(seven: &Path, archive: &Path, to: &Path, name: &str) -> Result<(), String> {
    let mut cmd = tools::command(seven);
    cmd.arg("x").arg(archive).arg(format!("-o{}", to.display())).args(["-y", "-bso0", "-bsp0"]);
    let out = procs::output(&mut cmd, "7-Zip", Some(Duration::from_secs(15 * 60)))?;
    if !out.status.success() {
        log::warn!("unpacking {name}: {}", String::from_utf8_lossy(&out.stderr));
        return Err(format!("Couldn't unpack the {name} download. Try again; it'll download afresh."));
    }
    Ok(())
}

/// An administrative install of an MSI just copies its files into a folder:
/// nothing is registered and no admin rights are needed.
#[cfg(windows)]
fn msi_unpack(msi: &Path, to: &Path, name: &str) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let msiexec = system32("msiexec.exe");
    let mut cmd = tools::command(&msiexec);
    // msiexec wants PROPERTY="value", which normal argument quoting can't produce.
    cmd.raw_arg(format!("/a \"{}\" /qn TARGETDIR=\"{}\"", msi.display(), to.display()));
    let out = procs::output(&mut cmd, name, Some(Duration::from_secs(20 * 60)))?;
    match out.status.code() {
        Some(0) => Ok(()),
        Some(1618) => Err(format!("Windows is busy installing something else. Try {name} again in a minute.")),
        code => Err(format!("Couldn't unpack {name} (Windows Installer error {}).", code.unwrap_or(-1))),
    }
}

#[cfg(not(windows))]
fn msi_unpack(_msi: &Path, _to: &Path, name: &str) -> Result<(), String> {
    Err(format!("{name} can only be unpacked on Windows."))
}

#[cfg(windows)]
fn system32(exe: &str) -> PathBuf {
    let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    root.join("System32").join(exe)
}

/// Ghostscript and ImageMagick need Microsoft's Visual C++ runtime, which
/// most PCs already have (games and many apps install it). If it's missing,
/// Microsoft's own installer adds it; Windows asks for permission once.
#[cfg(windows)]
fn ensure_vc_runtime() -> Result<(), String> {
    let needed = ["vcruntime140.dll", "vcruntime140_1.dll", "msvcp140.dll", "vcomp140.dll"];
    if needed.iter().all(|d| system32(d).is_file()) {
        return Ok(());
    }
    let name = "the Microsoft Visual C++ runtime";
    let Some(root) = root() else { return Ok(()) };
    let w = root.join(".work").join("vcredist");
    let _ = fs::create_dir_all(&w);
    let exe = w.join("vc_redist.x64.exe");
    download("https://aka.ms/vs/17/release/vc_redist.x64.exe", None, &exe, name)?;
    report(1.0, "Installing the Microsoft Visual C++ runtime (Windows will ask first)…");
    let mut cmd = tools::command(&exe);
    cmd.args(["/install", "/quiet", "/norestart"]);
    let out = procs::output(&mut cmd, name, Some(Duration::from_secs(15 * 60)));
    let _ = fs::remove_dir_all(&w);
    match out?.status.code() {
        // 3010: done, a restart finishes it; 1638: a newer one is installed.
        Some(0 | 3010 | 1638) => Ok(()),
        Some(1602) => Err("The Microsoft Visual C++ runtime is needed for this, and installing it was cancelled.".into()),
        code => Err(format!("Couldn't install the Microsoft Visual C++ runtime (error {}).", code.unwrap_or(-1))),
    }
}

#[cfg(not(windows))]
fn ensure_vc_runtime() -> Result<(), String> {
    Ok(())
}

// ---------------------------------------------------------------------------
// Downloading, with the curl that comes with Windows 10+ and macOS.

fn curl() -> Command {
    #[cfg(windows)]
    let mut cmd = tools::command(&system32("curl.exe"));
    #[cfg(not(windows))]
    let mut cmd = tools::command(Path::new("/usr/bin/curl"));
    cmd.args(["-fL", "-sS", "--retry", "3", "--retry-delay", "2", "--connect-timeout", "30", "-A", AGENT]);
    cmd
}

/// A plain message for curl's exit code; the raw error goes to the log.
fn curl_error(code: Option<i32>, what: &str, raw: &str) -> String {
    log::warn!("download of {what} failed ({code:?}): {}", raw.trim());
    match code {
        Some(6 | 7 | 28 | 35 | 52 | 56) => {
            format!("Couldn't connect to download {what}. Check the internet connection and try again.")
        }
        Some(22) => format!("The download of {what} isn't available right now. Try again later."),
        Some(23) => format!("Couldn't save the download of {what}: the drive may be full."),
        _ => format!("The download of {what} failed. Try again in a bit."),
    }
}

fn fetch_text(url: &str, what: &str) -> Result<String, String> {
    let mut cmd = curl();
    cmd.args(["-H", "Accept: application/vnd.github+json"]).arg(url);
    let out = procs::output(&mut cmd, "curl", Some(Duration::from_secs(90)))?;
    if !out.status.success() {
        return Err(curl_error(out.status.code(), what, &String::from_utf8_lossy(&out.stderr)));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Size the server reports for `url`, following redirects.
fn remote_size(url: &str) -> Option<u64> {
    let mut cmd = curl();
    cmd.arg("-I").arg(url);
    let out = procs::output(&mut cmd, "curl", Some(Duration::from_secs(60))).ok()?;
    let text = String::from_utf8_lossy(&out.stdout).to_ascii_lowercase();
    text.lines().filter_map(|l| l.strip_prefix("content-length:")).filter_map(|v| v.trim().parse().ok()).next_back()
}

/// Numbers in a name, for picking the newest: "n8.1" > "n7.1", "26.2.1" > "25.8.4".
fn version_key(s: &str) -> Vec<u64> {
    s.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty()).filter_map(|p| p.parse().ok()).collect()
}

/// The newest matching file of a GitHub project's latest release: (url, size).
fn github_asset(repo: &str, pick: impl Fn(&str) -> bool, what: &str) -> Result<(String, String, u64), String> {
    let text = fetch_text(&format!("https://api.github.com/repos/{repo}/releases/latest"), what)?;
    let json: serde_json::Value = serde_json::from_str(&text).map_err(|_| format!("Couldn't read the {what} release list."))?;
    pick_asset(&json, pick).ok_or_else(|| {
        log::warn!("{repo}: no matching download in {}", json["tag_name"]);
        format!("The {what} download wasn't where Convertino expected it. An update of Convertino will fix this.")
    })
}

fn pick_asset(json: &serde_json::Value, pick: impl Fn(&str) -> bool) -> Option<(String, String, u64)> {
    json["assets"]
        .as_array()?
        .iter()
        .filter_map(|a| Some((a["name"].as_str()?.to_string(), a["browser_download_url"].as_str()?.to_string(), a["size"].as_u64().unwrap_or(0))))
        .filter(|(n, _, _)| pick(n))
        .max_by_key(|(n, _, _)| version_key(n))
}

fn github_download(dir: &Path, repo: &str, pick: impl Fn(&str) -> bool, what: &str) -> Result<PathBuf, String> {
    let (name, url, size) = github_asset(repo, pick, what)?;
    log::info!("{what}: {name} ({} MB)", size / 1_000_000);
    let dest = dir.join(&name);
    download(&url, Some(size).filter(|s| *s > 0), &dest, what)?;
    Ok(dest)
}

/// Where LibreOffice comes from. The first is The Document Foundation's own
/// server, which hands each download to a nearby mirror; that mirror is
/// sometimes unreachable, so well-known mirrors with the same layout follow.
const LIBREOFFICE_MIRRORS: &[&str] = &[
    "https://download.documentfoundation.org/libreoffice/stable/",
    "https://ftp.fau.de/tdf/libreoffice/stable/",
    "https://mirror.netcologne.de/tdf/libreoffice/stable/",
    "https://ftp.halifax.rwth-aachen.de/tdf/libreoffice/stable/",
];

fn libreoffice_version() -> Result<String, String> {
    let mut last = String::new();
    for base in LIBREOFFICE_MIRRORS {
        match fetch_text(base, "LibreOffice") {
            Ok(page) => match newest_listed_version(&page) {
                Some(v) => return Ok(v),
                None => {
                    log::warn!("no LibreOffice version listed at {base}");
                    last = "Couldn't find the current LibreOffice version.".into();
                }
            },
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Downloads `path` (relative to the "stable" folder) from the first mirror that works.
fn libreoffice_download(path: &str, dest: &Path, what: &str) -> Result<(), String> {
    let mut last = String::new();
    for base in LIBREOFFICE_MIRRORS {
        let url = format!("{base}{path}");
        match download(&url, remote_size(&url), dest, what) {
            Ok(()) => return Ok(()),
            Err(e) if e == procs::CANCELLED => return Err(e),
            Err(e) => {
                log::warn!("LibreOffice from {base} failed: {e}");
                last = e;
            }
        }
    }
    Err(last)
}

/// The highest `href="X.Y.Z/"` in a directory listing.
fn newest_listed_version(page: &str) -> Option<String> {
    page.split("href=\"")
        .skip(1)
        .filter_map(|s| s.split('"').next()?.strip_suffix('/'))
        .filter(|v| v.split('.').count() == 3 && v.split('.').all(|p| !p.is_empty() && p.bytes().all(|c| c.is_ascii_digit())))
        .max_by_key(|v| version_key(v))
        .map(str::to_string)
}

fn download(url: &str, expected: Option<u64>, dest: &Path, what: &str) -> Result<(), String> {
    let part = dest.with_file_name(format!("{}.part", file_name(dest)));
    let _ = fs::remove_file(&part);
    let mut cmd = curl();
    cmd.arg("-o").arg(&part).arg(url).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    let mut tracked = procs::spawn(&mut cmd, "curl")?;
    let (mut last_size, mut last_change) = (0u64, Instant::now());
    let mb = |b: u64| (b as f64 / 1e6).round() as u64;
    let status = loop {
        match tracked.child().try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(e) => return Err(format!("The download of {what} stopped: {e}")),
        }
        let size = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        if size != last_size {
            (last_size, last_change) = (size, Instant::now());
        } else if last_change.elapsed() > STALL {
            procs::kill_tree(tracked.child().id());
            let _ = tracked.child().wait();
            let _ = fs::remove_file(&part);
            return Err(format!("The download of {what} stalled. Try again in a bit."));
        }
        match expected {
            Some(total) => report(
                size as f64 / total as f64,
                &format!("Downloading {what} (first time only) · {}%", (size * 100 / total).min(100)),
            ),
            None => report(0.0, &format!("Downloading {what} (first time only) · {} MB", mb(size))),
        }
        std::thread::sleep(Duration::from_millis(250));
    };
    let mut err = String::new();
    if let Some(mut e) = tracked.child().stderr.take() {
        let _ = e.read_to_string(&mut err);
    }
    drop(tracked);
    if procs::cancelled_here() {
        let _ = fs::remove_file(&part);
        return Err(procs::CANCELLED.into());
    }
    if !status.success() {
        let _ = fs::remove_file(&part);
        return Err(curl_error(status.code(), what, &err));
    }
    fs::rename(&part, dest).map_err(|e| format!("Couldn't save the download of {what}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal PE32+ file: one section holding an import table for
    /// KERNEL32.dll and avcodec-62.dll, and a delay-load entry for zlib.dll.
    fn tiny_pe() -> Vec<u8> {
        let mut b = vec![0u8; 0x400];
        let put32 = |b: &mut Vec<u8>, o: usize, v: u32| b[o..o + 4].copy_from_slice(&v.to_le_bytes());
        let put16 = |b: &mut Vec<u8>, o: usize, v: u16| b[o..o + 2].copy_from_slice(&v.to_le_bytes());
        b[0] = b'M';
        b[1] = b'Z';
        put32(&mut b, 0x3c, 0x80);
        b[0x80..0x84].copy_from_slice(b"PE\0\0");
        put16(&mut b, 0x80 + 6, 1); // one section
        put16(&mut b, 0x80 + 20, 240); // optional header size (PE32+)
        let opt = 0x80 + 24;
        put16(&mut b, opt, 0x20b);
        let dirs = opt + 112;
        put32(&mut b, dirs + 8, 0x1000); // imports at RVA 0x1000
        put32(&mut b, dirs + 13 * 8, 0x1100); // delay imports at RVA 0x1100
        let sec = opt + 240;
        put32(&mut b, sec + 8, 0x200); // virtual size
        put32(&mut b, sec + 12, 0x1000); // virtual address
        put32(&mut b, sec + 16, 0x200); // raw size
        put32(&mut b, sec + 20, 0x200); // raw offset
        // Import descriptors at file 0x200 (RVA 0x1000); names at RVA 0x1180.
        put32(&mut b, 0x200 + 12, 0x1180);
        put32(&mut b, 0x200 + 20 + 12, 0x1190);
        // Delay descriptor at file 0x300 (RVA 0x1100).
        put32(&mut b, 0x300 + 4, 0x11a0);
        let name = |b: &mut Vec<u8>, rva: usize, s: &str| {
            let o = rva - 0x1000 + 0x200;
            b[o..o + s.len()].copy_from_slice(s.as_bytes());
        };
        name(&mut b, 0x1180, "KERNEL32.dll");
        name(&mut b, 0x1190, "avcodec-62.dll");
        name(&mut b, 0x11a0, "zlib.dll");
        b
    }

    #[test]
    fn reads_pe_imports() {
        assert_eq!(pe_imports_of(&tiny_pe()), ["KERNEL32.dll", "avcodec-62.dll", "zlib.dll"]);
        assert!(pe_imports_of(b"not a program").is_empty());
        assert!(pe_imports_of(&[]).is_empty());
    }

    #[test]
    fn keeps_programs_and_what_they_load() {
        let dir = std::env::temp_dir().join(format!("convertino-trim-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("ffmpeg.exe"), tiny_pe()).unwrap();
        fs::write(dir.join("avcodec-62.dll"), b"no imports").unwrap();
        fs::write(dir.join("zlib.dll"), b"").unwrap();
        fs::write(dir.join("ffprobe.exe"), tiny_pe()).unwrap();
        fs::write(dir.join("ffplay.exe"), tiny_pe()).unwrap();
        fs::write(dir.join("SDL2.dll"), b"").unwrap();
        fs::write(dir.join("LICENSE.txt"), b"GPL").unwrap();
        keep_programs(&dir, &["ffmpeg.exe"]);
        let mut left: Vec<String> = list(&dir).iter().map(|p| file_name(p)).collect();
        left.sort();
        assert_eq!(left, ["LICENSE.txt", "avcodec-62.dll", "ffmpeg.exe", "zlib.dll"]);
        // A folder without the program is left alone.
        keep_programs(&dir, &["magick.exe"]);
        assert_eq!(list(&dir).len(), 4);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn trims_libreoffice_extras() {
        let dir = std::env::temp_dir().join(format!("convertino-lo-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        for d in ["program/resource/de/LC_MESSAGES", "help/en-US", "System64", "Fonts", "share/registry/res", "share/config", "share/extensions/dict-de/META-INF", "share/extensions/nlpsolver"] {
            fs::create_dir_all(dir.join(d)).unwrap();
        }
        for f in [
            "program/soffice.com",
            "System64/msvcp140.dll",
            "Fonts/Carlito-Regular.ttf",
            "LibreOffice_26.2.msi",
            "share/registry/main.xcd",
            "share/registry/Langpack-en-US.xcd",
            "share/registry/Langpack-de.xcd",
            "share/registry/res/registry_en-US.xcd",
            "share/registry/res/registry_de.xcd",
            "share/registry/res/fcfg_langpack_de.xcd",
            "share/config/images_colibre.zip",
            "share/config/images_elementary.zip",
            "share/extensions/dict-de/de_DE_frami.dic",
            "share/extensions/dict-de/de_DE_frami.aff",
            "share/extensions/dict-de/hyph_de_DE.dic",
            "share/extensions/dict-de/th_de_DE_v2.dat",
            "share/extensions/dict-de/dictionaries.xcu",
        ] {
            fs::write(dir.join(f), b"x").unwrap();
        }
        trim(Pack::LibreOffice, &dir);
        let has = |p: &str| dir.join(p).exists();
        for kept in [
            "program/soffice.com",
            "program/msvcp140.dll",
            "share/registry/main.xcd",
            "share/registry/Langpack-en-US.xcd",
            "share/registry/res/registry_en-US.xcd",
            "share/config/images_colibre.zip",
            "share/extensions/dict-de/hyph_de_DE.dic",
            "share/extensions/dict-de/dictionaries.xcu",
        ] {
            assert!(has(kept), "{kept} should stay");
        }
        for gone in [
            "program/resource",
            "help",
            "System64",
            "Fonts",
            "LibreOffice_26.2.msi",
            "share/registry/Langpack-de.xcd",
            "share/registry/res/registry_de.xcd",
            "share/registry/res/fcfg_langpack_de.xcd",
            "share/config/images_elementary.zip",
            "share/extensions/dict-de/de_DE_frami.dic",
            "share/extensions/dict-de/de_DE_frami.aff",
            "share/extensions/dict-de/th_de_DE_v2.dat",
            "share/extensions/nlpsolver",
        ] {
            assert!(!has(gone), "{gone} should go");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn version_labels() {
        assert_eq!(version_label(Pack::Ffmpeg, "ffmpeg-n8.1-latest-win64-lgpl-shared-8.1.zip").as_deref(), Some("8.1"));
        assert_eq!(version_label(Pack::Magick, "ImageMagick-7.1.2-32-portable-Q16-x64.7z").as_deref(), Some("7.1.2-32"));
        assert_eq!(version_label(Pack::Poppler, "Release-26.09.0-0.zip").as_deref(), Some("26.09.0"));
        assert_eq!(version_label(Pack::Ghostscript, "gs10080w64.exe").as_deref(), Some("10.08.0"));
        assert_eq!(version_label(Pack::Pandoc, "pandoc-3.12-windows-x86_64.zip").as_deref(), Some("3.12"));
        assert_eq!(version_label(Pack::SevenZip, "7z2603-x64.exe").as_deref(), Some("26.03"));
        assert_eq!(version_label(Pack::LibreOffice, "LibreOffice 26.8.0").as_deref(), Some("26.8.0"));
    }

    #[test]
    fn picks_the_right_downloads() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{"tag_name":"latest","assets":[
                {"name":"ffmpeg-master-latest-win64-lgpl-shared.zip","browser_download_url":"u0","size":1},
                {"name":"ffmpeg-n7.1-latest-win64-lgpl-shared-7.1.zip","browser_download_url":"u1","size":2},
                {"name":"ffmpeg-n8.1-latest-win64-lgpl-shared-8.1.zip","browser_download_url":"u2","size":3},
                {"name":"ffmpeg-n8.1-latest-win64-lgpl-8.1.zip","browser_download_url":"u3","size":4}]}"#,
        )
        .unwrap();
        let pick = |n: &str| n.starts_with("ffmpeg-n") && n.contains("-win64-lgpl-shared-") && n.ends_with(".zip");
        assert_eq!(pick_asset(&json, pick).map(|a| a.1), Some("u2".to_string()));
        assert_eq!(pick_asset(&json, |n| n.ends_with(".7z")), None);

        let listing = r#"<a href="../">Up</a> <a href="25.8.5/">25.8.5/</a> <a href="26.2.1/">26.2.1/</a> <a href="26.2.10/">x</a> <a href="?C=M">sort</a>"#;
        assert_eq!(newest_listed_version(listing).as_deref(), Some("26.2.10"));
    }

    /// fetch-tools.cmd: downloads whatever converter is missing into
    /// src-tauri/tools and trims the ones already there.
    #[test]
    #[ignore]
    fn fetch_tools() {
        let last = std::cell::Cell::new(String::new());
        set_reporter(move |_, detail| {
            if detail != last.take() {
                println!("    {detail}");
            }
            last.set(detail.to_string());
        });
        let root = root().unwrap();
        let freed = trim_existing();
        if freed > 0 {
            println!("Trimmed the converters already there: {freed} MB freed");
        }
        let mut failed = Vec::new();
        match match_ffmpeg_build() {
            Ok(true) => println!("FFmpeg: switched to the build with x264/x265"),
            Ok(false) => {}
            Err(e) => {
                println!("FFmpeg: couldn't switch builds: {e}");
                failed.push("FFmpeg (build switch)");
            }
        }
        for pack in Pack::ALL {
            println!("==> {}", pack.name());
            match ensure(pack) {
                Ok(()) => println!("    ready: {}", tools::find(pack.main_tool()).map(|p| p.display().to_string()).unwrap_or_default()),
                Err(e) => {
                    println!("    FAILED: {e}");
                    failed.push(pack.name());
                }
            }
        }
        println!();
        let mut total = 0;
        for pack in Pack::ALL {
            let dir = root.join(pack.dir());
            if dir.is_dir() {
                let mb = dir_size(&dir) / 1_000_000;
                total += mb;
                println!("{:>8} MB  {}", mb, pack.name());
            }
        }
        println!("{total:>8} MB  in {}", root.display());
        assert!(failed.is_empty(), "not ready: {failed:?}");
        println!("TOOLS READY");
    }
}

// ---------------------------------------------------------------------------
// macOS: FFmpeg, Pandoc, 7-Zip and LibreOffice download on first use;
// ImageMagick, Poppler and Ghostscript come inside the app.

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod mac {
    use super::*;

    fn arm() -> bool {
        cfg!(target_arch = "aarch64")
    }

    pub fn ffmpeg_url() -> String {
        format!("https://ffmpeg.martin-riedl.de/redirect/latest/macos/{}/release/ffmpeg.zip", if arm() { "arm64" } else { "amd64" })
    }

    fn pandoc_pick(n: &str) -> bool {
        n.ends_with(if arm() { "-arm64-macOS.zip" } else { "-x86_64-macOS.zip" })
    }

    fn seven_pick(n: &str) -> bool {
        n.starts_with("7z") && n.ends_with("-mac.tar.xz")
    }

    /// The .dmg's path inside LibreOffice's "stable" folder (any mirror).
    pub fn libreoffice_path(version: &str) -> String {
        let (dir, file) = if arm() { ("aarch64", "aarch64") } else { ("x86_64", "x86-64") };
        format!("{version}/mac/{dir}/LibreOffice_{version}_MacOS_{file}.dmg")
    }

    fn run(cmd: &mut Command, what: &str, minutes: u64) -> Result<(), String> {
        let out = procs::output(cmd, what, Some(Duration::from_secs(minutes * 60)))?;
        if out.status.success() {
            Ok(())
        } else {
            log::warn!("{what}: {}", String::from_utf8_lossy(&out.stderr));
            Err(format!("Couldn't set up {what}. Try again; it'll download afresh."))
        }
    }

    /// Unzips or untars with the Mac's own tools.
    fn unpack(archive: &Path, to: &Path, what: &str) -> Result<(), String> {
        fs::create_dir_all(to).map_err(|e| e.to_string())?;
        let name = file_name(archive);
        if name.ends_with(".zip") {
            run(Command::new("/usr/bin/ditto").args(["-x", "-k"]).arg(archive).arg(to), what, 10)
        } else {
            run(Command::new("/usr/bin/tar").arg("-xf").arg(archive).arg("-C").arg(to), what, 10)
        }
    }

    /// Programs downloaded with curl aren't quarantined, but Apple silicon
    /// only runs code with at least an ad-hoc signature.
    fn sign(path: &Path) {
        let _ = Command::new("/usr/bin/codesign").args(["--force", "--sign", "-"]).arg(path).output();
        let _ = Command::new("/bin/chmod").arg("+x").arg(path).output();
    }

    /// Final URL after redirects (the FFmpeg link points at the current version).
    fn resolved(url: &str) -> Option<String> {
        let mut cmd = super::curl();
        cmd.args(["-sIL", "-o", "/dev/null", "-w", "%{url_effective}"]).arg(url);
        let out = procs::output(&mut cmd, "curl", Some(Duration::from_secs(30))).ok()?;
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (!s.is_empty()).then_some(s)
    }

    pub fn fetch(pack: Pack, w: &Path, staging: &Path, unpacked: &Path) -> Result<String, String> {
        let name = pack.name();
        match pack {
            Pack::SevenZip => {
                let arc = github_download(w, "ip7z/7zip", seven_pick, name)?;
                report(1.0, &format!("Setting up {name}…"));
                unpack(&arc, staging, name)?;
                let zz = staging.join("7zz");
                if !zz.is_file() {
                    return Err("7zz wasn't in the 7-Zip download.".into());
                }
                sign(&zz);
                Ok(file_name(&arc))
            }
            Pack::Ffmpeg => {
                let zip = w.join("ffmpeg.zip");
                let url = ffmpeg_url();
                let source = resolved(&url).unwrap_or_else(|| url.clone());
                fs::create_dir_all(staging).map_err(|e| e.to_string())?;
                match download(&url, None, &zip, name).and_then(|_| {
                    report(1.0, &format!("Setting up {name}…"));
                    unpack(&zip, unpacked, name)?;
                    let exe = find_file(unpacked, "ffmpeg", 3).ok_or("ffmpeg wasn't in the download.")?;
                    fs::rename(&exe, staging.join("ffmpeg")).map_err(|e| e.to_string())
                }) {
                    Ok(()) => {}
                    Err(e) => {
                        // Second source: a static build on GitHub.
                        log::warn!("FFmpeg from {url} failed ({e}); trying the GitHub build");
                        let asset = if arm() { "ffmpeg-darwin-arm64" } else { "ffmpeg-darwin-x64" };
                        let bin = github_download(w, "eugeneware/ffmpeg-static", |n| n == asset, name)?;
                        fs::rename(&bin, staging.join("ffmpeg")).map_err(|e| e.to_string())?;
                    }
                }
                sign(&staging.join("ffmpeg"));
                Ok(source.rsplit('/').next().unwrap_or("ffmpeg").to_string())
            }
            Pack::Pandoc => {
                let zip = github_download(w, "jgm/pandoc", pandoc_pick, name)?;
                report(1.0, &format!("Setting up {name}…"));
                unpack(&zip, unpacked, name)?;
                promote(unpacked, "pandoc", 0, staging)?;
                Ok(file_name(&zip))
            }
            Pack::LibreOffice => {
                let version = libreoffice_version()?;
                let dmg = w.join("LibreOffice.dmg");
                libreoffice_download(&libreoffice_path(&version), &dmg, name)?;
                report(1.0, &format!("Setting up {name} (this takes a minute)…"));
                let mnt = w.join("mnt");
                fs::create_dir_all(&mnt).map_err(|e| e.to_string())?;
                run(
                    Command::new("/usr/bin/hdiutil").args(["attach", "-nobrowse", "-readonly", "-noautoopen", "-mountpoint"]).arg(&mnt).arg(&dmg),
                    name,
                    5,
                )?;
                fs::create_dir_all(staging).map_err(|e| e.to_string())?;
                let copied = run(
                    Command::new("/usr/bin/ditto").arg(mnt.join("LibreOffice.app")).arg(staging.join("LibreOffice.app")),
                    name,
                    15,
                );
                let _ = Command::new("/usr/bin/hdiutil").args(["detach", "-quiet"]).arg(&mnt).output();
                copied?;
                Ok(format!("LibreOffice {version}"))
            }
            Pack::Magick | Pack::Poppler | Pack::Ghostscript => Err(format!("{name} comes with Convertino for Mac.")),
        }
    }

    pub fn latest_source(pack: Pack) -> Result<String, String> {
        let name = pack.name();
        let asset = |repo: &str, pick: &dyn Fn(&str) -> bool| github_asset(repo, pick, name).map(|a| a.0);
        match pack {
            Pack::SevenZip => asset("ip7z/7zip", &seven_pick),
            Pack::Pandoc => asset("jgm/pandoc", &pandoc_pick),
            Pack::Ffmpeg => resolved(&ffmpeg_url())
                .map(|u| u.rsplit('/').next().unwrap_or_default().to_string())
                .ok_or_else(|| "Couldn't check FFmpeg for updates.".into()),
            Pack::LibreOffice => libreoffice_version().map(|v| format!("LibreOffice {v}")),
            _ => Err(format!("{name} updates with Convertino.")),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn mac_download_names() {
            assert!(ffmpeg_url().contains("/macos/") && ffmpeg_url().ends_with("ffmpeg.zip"));
            assert!(seven_pick("7z2603-mac.tar.xz") && !seven_pick("7z2603-linux-x64.tar.xz"));
            assert!(pandoc_pick("pandoc-3.12-arm64-macOS.zip") || pandoc_pick("pandoc-3.12-x86_64-macOS.zip"));
            assert!(!pandoc_pick("pandoc-3.12-arm64-macOS.pkg"));
            let lo = libreoffice_path("26.2.1");
            assert!(lo.starts_with("26.2.1/mac/") && lo.ends_with(".dmg"), "{lo}");
            assert!(LIBREOFFICE_MIRRORS.iter().all(|m| m.starts_with("https://") && m.ends_with("/libreoffice/stable/")));
            assert!(Pack::Magick.bundled_on_mac() && !Pack::Ffmpeg.bundled_on_mac());
        }
    }
}
