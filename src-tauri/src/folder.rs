//! Folders: Alt+right-click a folder (or empty space inside one) and the wheel
//! offers what makes sense for everything in it: compress all, convert all of
//! one kind, join clips, an image sequence into a video, photos into one PDF,
//! merge PDFs, one spreadsheet from CSV files, extract every archive, zip the
//! folder, fit the folder into a size, remove GPS.
//!
//! The folder is scanned once (`scan`); what's inside decides the slots
//! (`slots`); a pick becomes a `Plan` of ordinary conversion steps (plus a few
//! steps of its own, `FolderOp`) that the engine runs, and `run` reports every
//! file's fate so the corner card can say exactly what happened.
//!
//! Every file is counted: files a slot doesn't touch, hidden and system files,
//! and results of an earlier run are "left alone" with the reason.

use crate::convert::{self, Op, Step};
use crate::engine::{self, file_name, human_size, Event, Sink};
use crate::settings::Quality;
use crate::size::{self, Kind};
use crate::wheel::{self, Slot, SlotKind};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// A folder with more files than this isn't read to the end (the wheel says so).
pub const MAX_FILES: usize = 200_000;

// ---------------------------------------------------------------------------
// Natural order: "scan 2" before "scan 10", case ignored.

fn chunks(s: &str) -> Vec<(bool, String)> {
    let mut out: Vec<(bool, String)> = Vec::new();
    for c in s.chars() {
        let digit = c.is_ascii_digit();
        match out.last_mut() {
            Some((d, text)) if *d == digit => text.push(c),
            _ => out.push((digit, c.to_string())),
        }
    }
    out
}

pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (ca, cb) = (chunks(a), chunks(b));
    for (x, y) in ca.iter().zip(cb.iter()) {
        let o = match (x.0, y.0) {
            (true, true) => {
                let (tx, ty) = (x.1.trim_start_matches('0'), y.1.trim_start_matches('0'));
                tx.len().cmp(&ty.len()).then_with(|| tx.cmp(ty)).then_with(|| x.1.len().cmp(&y.1.len()))
            }
            _ => x.1.to_lowercase().cmp(&y.1.to_lowercase()),
        };
        if o != Ordering::Equal {
            return o;
        }
    }
    ca.len().cmp(&cb.len()).then_with(|| a.cmp(b))
}

/// Paths in natural order, folder by folder.
fn path_cmp(a: &Path, b: &Path) -> Ordering {
    let parts = |p: &Path| p.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>();
    let (pa, pb) = (parts(a), parts(b));
    for (x, y) in pa.iter().zip(pb.iter()) {
        let o = natural_cmp(x, y);
        if o != Ordering::Equal {
            return o;
        }
    }
    pa.len().cmp(&pb.len())
}

// ---------------------------------------------------------------------------
// Scanning

/// A file in the folder that Explorer / Finder shows (hidden ones are in `Scan::alone`).
#[derive(Debug, Clone)]
pub struct Item {
    pub path: PathBuf,
    /// Which selected folder it's in.
    pub root: usize,
    /// Its path inside that folder ("old/clip.mp4").
    pub rel: PathBuf,
    pub ext: String,
    /// The wheel's kind of file ("video", "image", …), None for kinds Convertino doesn't know.
    pub family: Option<String>,
    pub bytes: u64,
    pub modified: Option<SystemTime>,
    /// A result of an earlier Convertino run ("clip (compressed).mp4", or inside
    /// "Clips (compressed)"): left alone by everything but Zip and Fit to size.
    pub earlier: bool,
}

impl Item {
    pub fn in_subfolder(&self) -> bool {
        self.rel.components().count() > 1
    }
    pub fn stem(&self) -> String {
        self.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
    }
    fn is(&self, family: &str) -> bool {
        self.family.as_deref() == Some(family)
    }
}

/// Why a file (or a folder inside) is left out of everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Alone {
    Hidden,
    System,
    Shortcut,
    /// "clip (compressed).mp4" next to "clip.mp4": a result of an earlier run.
    Earlier,
    /// Convertino's own scratch files, Office lock files.
    Temp,
    /// A download still going (.crdownload, .part).
    Partial,
    /// A link to another folder (not followed).
    Link,
    /// Program folders nobody converts (.git, node_modules).
    Tooling,
    /// The Holding Area (`_Quarantine`), or "Convertino originals" from builds before 13 Oct 2026.
    Held,
}

impl Alone {
    pub fn words(self) -> &'static str {
        match self {
            Alone::Hidden => "hidden",
            Alone::System => "system file",
            Alone::Shortcut => "shortcut",
            Alone::Earlier => "made by Convertino earlier",
            Alone::Temp => "temporary file",
            Alone::Partial => "download not finished",
            Alone::Link => "link to another folder",
            Alone::Tooling => "program files",
            Alone::Held => "originals Convertino kept",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Scan {
    /// The selected folders (usually one).
    pub roots: Vec<PathBuf>,
    /// Visible files, in natural order.
    pub items: Vec<Item>,
    /// Files left out, with the reason.
    pub alone: Vec<(PathBuf, Alone)>,
    /// Folders inside that weren't read, with the reason.
    pub skipped_dirs: Vec<(PathBuf, Alone)>,
    /// Folders inside that were read.
    pub subfolders: usize,
    /// More than MAX_FILES: not read to the end.
    pub truncated: bool,
}

/// Suffixes Convertino gives its results ("clip (compressed).mp4") and result folders ("Clips (mp4)").
const RESULT_WORDS: &[&str] = &[
    "compressed", "normalized", "no GPS", "720p", "trimmed", "grayscale", "edited", "extracted", "zip parts",
    "mp4", "mov", "webm", "mkv", "gif", "mp3", "wav", "flac", "aac", "ogg", "opus", "m4a", "aiff",
    "jpg", "png", "webp", "avif", "tiff", "bmp", "ico", "pdf", "docx", "odt", "md", "txt", "html", "csv", "json", "yaml", "xlsx", "tsv",
];

/// "clip (compressed)" -> "clip"; "trip (10 MB, first 7 s)" -> "trip". None when it isn't a result name.
fn result_base(stem: &str) -> Option<&str> {
    let inner_start = stem.rfind(" (")?;
    let inner = stem[inner_start + 2..].strip_suffix(')')?;
    let size_like = {
        let first = inner.split(',').next().unwrap_or("");
        let mut parts = first.split(' ');
        let num = parts.next().unwrap_or("");
        let unit = parts.next().unwrap_or("");
        !num.is_empty() && num.chars().all(|c| c.is_ascii_digit() || c == '.') && matches!(unit, "KB" | "MB" | "GB") && parts.next().is_none()
    };
    (RESULT_WORDS.contains(&inner) || size_like).then(|| &stem[..inner_start])
}

fn hidden_or_system(name: &str, meta: &std::fs::Metadata) -> Option<Alone> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        let a = meta.file_attributes();
        if a & 0x4 != 0 {
            return Some(Alone::System);
        }
        if a & 0x2 != 0 {
            return Some(Alone::Hidden);
        }
    }
    let _ = meta;
    let lower = name.to_lowercase();
    if matches!(lower.as_str(), "desktop.ini" | "thumbs.db" | ".ds_store" | "icon\r" | ".localized") {
        return Some(Alone::System);
    }
    if name.starts_with(".convertino-") || name.starts_with("~$") {
        return Some(Alone::Temp);
    }
    if name.starts_with('.') {
        return Some(Alone::Hidden);
    }
    None
}

/// Where Replace kept the originals before the shared Holding Area (13 Oct 2026); still skipped when scanning.
pub const HOLDING: &str = "Convertino originals";

const TOOLING_DIRS: &[&str] = &[".git", "node_modules", "$recycle.bin", "system volume information", "__pycache__", ".svn", ".hg"];

/// Reads the folders. `cancel` stops early (the wheel was closed); `progress`
/// gets the number of files found so far.
pub fn scan(roots: &[PathBuf], cancel: &AtomicBool, progress: &dyn Fn(usize)) -> Result<Scan, String> {
    scan_with(roots, cancel, progress, false)
}

/// Only the files directly in the folders (watched folders check every few seconds).
pub fn scan_top(roots: &[PathBuf]) -> Result<Scan, String> {
    scan_with(roots, &AtomicBool::new(false), &|_| {}, true)
}

fn scan_with(roots: &[PathBuf], cancel: &AtomicBool, progress: &dyn Fn(usize), top_only: bool) -> Result<Scan, String> {
    let mut out = Scan { roots: roots.to_vec(), ..Scan::default() };
    for (r, root) in roots.iter().enumerate() {
        if !root.is_dir() {
            return Err(format!("{} isn't there any more.", root.display()));
        }
        let mut stack: Vec<(PathBuf, bool)> = vec![(root.clone(), false)];
        while let Some((dir, in_earlier)) = stack.pop() {
            if cancel.load(AtomicOrdering::Relaxed) {
                return Err("Stopped".into());
            }
            let Ok(rd) = std::fs::read_dir(&dir) else {
                if dir == *root {
                    return Err(format!("Convertino can't open {}.", root.display()));
                }
                continue;
            };
            let mut entries: Vec<(String, PathBuf, std::fs::Metadata)> = rd
                .flatten()
                .filter_map(|e| {
                    let meta = std::fs::symlink_metadata(e.path()).ok()?;
                    Some((e.file_name().to_string_lossy().into_owned(), e.path(), meta))
                })
                .collect();
            entries.sort_by(|a, b| natural_cmp(&a.0, &b.0));
            // Stems in this folder, to recognise earlier results.
            let stems: HashSet<String> = entries
                .iter()
                .map(|(n, p, m)| if m.is_dir() { n.clone() } else { p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default() })
                .collect();
            let mut subdirs = Vec::new();
            for (name, path, meta) in entries {
                if meta.file_type().is_symlink() {
                    if std::fs::metadata(&path).map(|m| m.is_dir()).unwrap_or(false) {
                        out.skipped_dirs.push((path, Alone::Link));
                    } else {
                        out.alone.push((path, Alone::Shortcut));
                    }
                    continue;
                }
                if meta.is_dir() {
                    let why = if TOOLING_DIRS.contains(&name.to_lowercase().as_str()) {
                        Some(Alone::Tooling)
                    } else if name == HOLDING || name.eq_ignore_ascii_case(crate::holding::DIR) {
                        Some(Alone::Held)
                    } else if let Some(w) = hidden_or_system(&name, &meta) {
                        Some(w)
                    } else if result_base(&name).is_some_and(|b| stems.contains(b)) {
                        Some(Alone::Earlier)
                    } else {
                        None
                    };
                    match why {
                        // A folder of earlier results is read (Zip takes it) but left alone otherwise.
                        Some(Alone::Earlier) => {
                            out.skipped_dirs.push((path.clone(), Alone::Earlier));
                            subdirs.push((path, true));
                        }
                        Some(w) => out.skipped_dirs.push((path, w)),
                        None => subdirs.push((path, in_earlier)),
                    }
                    continue;
                }
                let lower = name.to_lowercase();
                let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                let why = hidden_or_system(&name, &meta)
                    .or_else(|| matches!(ext.as_str(), "lnk" | "url" | "webloc").then_some(Alone::Shortcut))
                    .or_else(|| matches!(ext.as_str(), "crdownload" | "part" | "partial" | "download" | "opdownload").then_some(Alone::Partial))
                    .or_else(|| (lower.ends_with(".tmp") && lower.contains("convertino")).then_some(Alone::Temp));
                if let Some(w) = why {
                    out.alone.push((path, w));
                    continue;
                }
                let earlier = in_earlier || result_base(&stem).is_some_and(|b| stems.contains(b));
                if out.items.len() >= MAX_FILES {
                    out.truncated = true;
                    break;
                }
                let rel = path.strip_prefix(root).map(Path::to_path_buf).unwrap_or_else(|_| PathBuf::from(&name));
                out.items.push(Item {
                    family: wheel::family_of(&ext).map(|(id, _)| id),
                    ext,
                    rel,
                    root: r,
                    bytes: meta.len(),
                    modified: meta.modified().ok(),
                    earlier,
                    path,
                });
                if out.items.len() % 500 == 0 {
                    progress(out.items.len());
                }
            }
            if out.truncated {
                break;
            }
            if top_only {
                continue;
            }
            out.subfolders += subdirs.iter().filter(|(_, e)| !*e || in_earlier).count();
            // Reversed onto the stack so they're read in order.
            stack.extend(subdirs.into_iter().rev());
        }
    }
    out.items.sort_by(|a, b| a.root.cmp(&b.root).then_with(|| path_cmp(&a.rel, &b.rel)));
    progress(out.items.len());
    Ok(out)
}

impl Scan {
    /// Visible files, with or without subfolders (not earlier results).
    pub fn files(&self, subfolders: bool) -> Vec<&Item> {
        self.items.iter().filter(|i| !i.earlier && (subfolders || !i.in_subfolder())).collect()
    }

    /// Every visible file, earlier results too (what Zip and Fit to size take).
    pub fn everything(&self, subfolders: bool) -> Vec<&Item> {
        self.items.iter().filter(|i| subfolders || !i.in_subfolder()).collect()
    }

    /// Earlier results, as files.
    pub fn earlier(&self) -> usize {
        self.items.iter().filter(|i| i.earlier).count()
    }

    pub fn bytes(&self) -> u64 {
        self.items.iter().filter(|i| !i.earlier).map(|i| i.bytes).sum()
    }

    /// "Valorant clips", or "3 folders".
    pub fn name(&self) -> String {
        match self.roots.as_slice() {
            [one] => folder_name(one),
            many => format!("{} folders", many.len()),
        }
    }

    /// Left-alone files and folders as "2 hidden, 1 shortcut".
    pub fn alone_words(&self) -> String {
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for (_, w) in &self.alone {
            *counts.entry(w.words()).or_default() += 1;
        }
        let mut parts: Vec<String> = counts.into_iter().map(|(w, n)| format!("{n} {w}")).collect();
        let not_read: Vec<&(PathBuf, Alone)> = self.skipped_dirs.iter().filter(|(_, w)| *w != Alone::Earlier).collect();
        if !not_read.is_empty() {
            let n = not_read.len();
            parts.push(format!("{n} folder{} not read ({})", if n == 1 { "" } else { "s" }, {
                let mut ws: Vec<&str> = not_read.iter().map(|(_, w)| w.words()).collect();
                ws.dedup();
                ws.join(", ")
            }));
        }
        parts.join(", ")
    }
}

/// A folder's name; a drive ("E:\") is "Drive E".
pub fn folder_name(p: &Path) -> String {
    match p.file_name() {
        Some(n) => n.to_string_lossy().into_owned(),
        None => {
            let s = p.to_string_lossy();
            let letter = s.chars().next().filter(|c| c.is_ascii_alphabetic());
            match letter {
                Some(l) if s.len() <= 3 => format!("Drive {}", l.to_ascii_uppercase()),
                _ => "Folder".into(),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Image sequences: render_0001.png … render_0240.png

#[derive(Debug, Clone, PartialEq)]
pub struct Sequence {
    pub dir: PathBuf,
    pub prefix: String,
    pub ext: String,
    /// (frame number, file), in order.
    pub frames: Vec<(u64, PathBuf)>,
    /// Frame numbers missing between the first and the last.
    pub gaps: Vec<u64>,
    pub width: u32,
    pub height: u32,
}

const SEQ_EXT: &[&str] = &["png", "jpg", "jpeg", "tif", "tiff", "webp", "bmp", "exr", "dpx", "tga"];

/// The longest run of numbered frames in one folder (at least 10, and most of the pictures there).
pub fn find_sequence(items: &[&Item]) -> Option<Sequence> {
    let mut groups: HashMap<(PathBuf, String, String), Vec<(u64, PathBuf)>> = HashMap::new();
    let mut pictures_in: HashMap<PathBuf, usize> = HashMap::new();
    for i in items {
        if !SEQ_EXT.contains(&i.ext.as_str()) {
            continue;
        }
        let dir = i.path.parent().map(Path::to_path_buf).unwrap_or_default();
        *pictures_in.entry(dir.clone()).or_default() += 1;
        let stem = i.stem();
        let digits: String = stem.chars().rev().take_while(|c| c.is_ascii_digit()).collect::<Vec<_>>().into_iter().rev().collect();
        if digits.is_empty() || digits.len() > 9 {
            continue;
        }
        let prefix = stem[..stem.len() - digits.len()].to_string();
        let n: u64 = digits.parse().ok()?;
        groups.entry((dir, prefix, i.ext.clone())).or_default().push((n, i.path.clone()));
    }
    let ((dir, prefix, ext), mut frames) = groups.into_iter().filter(|(_, f)| f.len() >= 10).max_by_key(|(_, f)| f.len())?;
    if frames.len() * 2 < pictures_in.get(&dir).copied().unwrap_or(0) {
        return None;
    }
    frames.sort_by_key(|(n, _)| *n);
    frames.dedup_by_key(|(n, _)| *n);
    let (first, last) = (frames[0].0, frames[frames.len() - 1].0);
    let have: HashSet<u64> = frames.iter().map(|(n, _)| *n).collect();
    let gaps: Vec<u64> = if last - first < 1_000_000 { (first..=last).filter(|n| !have.contains(n)).collect() } else { Vec::new() };
    let (width, height) = image::image_dimensions(&frames[0].1).unwrap_or((0, 0));
    Some(Sequence { dir, prefix, ext, frames, gaps, width, height })
}

// ---------------------------------------------------------------------------
// Choices for a folder job (Shift+click, the command line, MCP)

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Into {
    /// A new folder next to the original, same subfolders inside.
    #[default]
    New,
    /// Next to each original ("clip (compressed).mp4").
    Beside,
    /// The new file takes the original's place once it checks out; the original goes to the Holding Area.
    Replace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Order {
    #[default]
    Name,
    Date,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SeqFormat {
    #[default]
    Mp4,
    H265,
    Prores,
    Webm,
    Gif,
}

impl SeqFormat {
    fn ext(self) -> &'static str {
        match self {
            SeqFormat::Mp4 | SeqFormat::H265 => "mp4",
            SeqFormat::Prores => "mov",
            SeqFormat::Webm => "webm",
            SeqFormat::Gif => "gif",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Gaps {
    /// A missing frame shows the one before it for longer.
    #[default]
    Hold,
    /// A missing frame stops the job before it starts.
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PackFormat {
    #[default]
    Zip,
    #[serde(rename = "7z")]
    SevenZ,
    #[serde(rename = "targz")]
    TarGz,
}

impl PackFormat {
    fn ext(self) -> &'static str {
        match self {
            PackFormat::Zip => "zip",
            PackFormat::SevenZ => "7z",
            PackFormat::TarGz => "tar.gz",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum FitMode {
    /// A compressed copy of the folder that fits.
    #[default]
    Folder,
    /// One zip that fits (media shrunk just enough).
    Zip,
    /// An untouched zip in pieces of the size.
    Parts,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fit {
    pub bytes: u64,
    #[serde(default)]
    pub mode: FitMode,
}

/// The choices kept in Settings ("Use these every time").
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Defaults {
    pub into: Into,
    pub subfolders: bool,
    pub skip_done: bool,
}

impl Default for Defaults {
    fn default() -> Self {
        Defaults { into: Into::New, subfolders: true, skip_done: true }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Opts {
    pub into: Into,
    pub subfolders: bool,
    pub skip_done: bool,
    pub order: Order,
    /// Image sequence: frames per second, what to make, a sound file, missing frames.
    pub fps: f64,
    pub format: SeqFormat,
    pub sound: bool,
    pub gaps: Gaps,
    /// Zip: the format, a password, pieces of this many bytes.
    pub pack: PackFormat,
    pub password: Option<String>,
    pub parts: Option<u64>,
    /// Fit to a size.
    pub fit: Option<Fit>,
}

impl Default for Opts {
    fn default() -> Self {
        Opts::from_defaults(Defaults::default())
    }
}

impl Opts {
    pub fn from_defaults(d: Defaults) -> Self {
        Opts {
            into: d.into,
            subfolders: d.subfolders,
            skip_done: d.skip_done,
            order: Order::Name,
            fps: 24.0,
            format: SeqFormat::Mp4,
            sound: false,
            gaps: Gaps::Hold,
            pack: PackFormat::Zip,
            password: None,
            parts: None,
            fit: None,
        }
    }

    pub fn defaults(&self) -> Defaults {
        Defaults { into: self.into, subfolders: self.subfolders, skip_done: self.skip_done }
    }
}

// ---------------------------------------------------------------------------
// The folder wheel

/// What a folder slot does.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Compress,
    /// Every file of a kind to one format: the target id ("video.mp4").
    ConvertAll(String),
    AudioOnly,
    Normalize,
    RemoveGps,
    PhotosPdf,
    MergePdf,
    JoinVideo,
    JoinAudio,
    Sequence,
    SequenceGif,
    Sheets,
    ExtractAll,
    Zip,
    Fit,
    Watch,
}

impl Action {
    pub fn id(&self) -> String {
        match self {
            Action::ConvertAll(t) => format!("folder.to:{t}"),
            other => format!(
                "folder.{}",
                match other {
                    Action::Compress => "compress",
                    Action::AudioOnly => "audio",
                    Action::Normalize => "normalize",
                    Action::RemoveGps => "gps",
                    Action::PhotosPdf => "pdf",
                    Action::MergePdf => "merge",
                    Action::JoinVideo => "join-video",
                    Action::JoinAudio => "join-audio",
                    Action::Sequence => "sequence",
                    Action::SequenceGif => "sequence-gif",
                    Action::Sheets => "sheets",
                    Action::ExtractAll => "extract",
                    Action::Zip => "zip",
                    Action::Fit => "fit",
                    Action::Watch => "watch",
                    Action::ConvertAll(_) => unreachable!(),
                }
            ),
        }
    }

    pub fn from_id(id: &str) -> Option<Action> {
        let rest = id.strip_prefix("folder.")?;
        if let Some(t) = rest.strip_prefix("to:") {
            return wheel::is_target(t).then(|| Action::ConvertAll(t.to_string()));
        }
        Some(match rest {
            "compress" => Action::Compress,
            "audio" => Action::AudioOnly,
            "normalize" => Action::Normalize,
            "gps" => Action::RemoveGps,
            "pdf" => Action::PhotosPdf,
            "merge" => Action::MergePdf,
            "join-video" => Action::JoinVideo,
            "join-audio" => Action::JoinAudio,
            "sequence" => Action::Sequence,
            "sequence-gif" => Action::SequenceGif,
            "sheets" => Action::Sheets,
            "extract" => Action::ExtractAll,
            "zip" => Action::Zip,
            "fit" => Action::Fit,
            "watch" => Action::Watch,
            _ => return None,
        })
    }

    /// Batch actions make one result per file (and take "Put the results").
    pub fn is_batch(&self) -> bool {
        matches!(self, Action::Compress | Action::ConvertAll(_) | Action::AudioOnly | Action::Normalize | Action::RemoveGps)
    }
}

/// Kinds Compress works on, and the Compress target for each.
const COMPRESS_TARGETS: &[(&str, &str)] = &[("image", "image.compress"), ("raw", "raw.compress"), ("video", "video.compress"), ("pdf", "pdf.compress"), ("audio", "audio.compress")];
/// Pictures whose GPS location can be removed in place.
const GPS_EXT: &[&str] = &["jpg", "jpeg", "png", "webp", "tif", "tiff"];

fn noun(family: &str, n: usize) -> String {
    let (one, many) = match family {
        "image" => ("picture", "pictures"),
        "raw" => ("RAW photo", "RAW photos"),
        "video" => ("video", "videos"),
        "audio" => ("audio file", "audio files"),
        "pdf" => ("PDF", "PDFs"),
        "doc" => ("document", "documents"),
        "data" => ("data file", "data files"),
        "archive" => ("archive", "archives"),
        "frame" => ("frame", "frames"),
        "page" => ("page", "pages"),
        _ => ("file", "files"),
    };
    format!("{n} {}", if n == 1 { one } else { many })
}

fn files_word(n: usize) -> String {
    noun("", n)
}

/// What each slot would touch in this folder.
pub struct Picks<'a> {
    pub files: Vec<&'a Item>,
    pub label: String,
    pub icon: &'static str,
    /// The slot's count line ("40 videos").
    pub sub: String,
    /// What it does, in a line.
    pub what: String,
    /// Where the result goes.
    pub out: String,
    pub score: u32,
}

fn share(n: usize, of: usize) -> f64 {
    if of == 0 { 0.0 } else { n as f64 / of as f64 }
}

/// Where a batch's results go, in words, for the hint.
fn into_words(into: Into, scan: &Scan, suffix: &str) -> String {
    match into {
        Into::New => {
            let exists = scan.roots.iter().any(|r| r.parent().is_some_and(|p| p.join(format!("{} ({suffix})", folder_name(r))).is_dir()));
            if exists {
                format!("→ {} ({suffix}), already there: files done in it before are skipped", scan.name())
            } else {
                format!("→ new folder {} ({suffix})", scan.name())
            }
        }
        Into::Beside => "→ next to each original".into(),
        Into::Replace => "→ replaces each original once its copy checks out (the originals move to the Holding Area)".to_string(),
    }
}


/// The name of a one-file result next to the folder ("Valorant clips.zip").
fn one_file_words(scan: &Scan, ext: &str) -> String {
    format!("→ {}.{ext} next to the folder", single_stem(scan))
}

fn single_stem(scan: &Scan) -> String {
    match scan.roots.as_slice() {
        [one] => folder_name(one),
        many => many[0].parent().map(folder_name).unwrap_or_else(|| "Folders".into()),
    }
}

/// Every slot that applies to this folder, best first (Fit, Zip and Watch are added by `slots`).
pub fn candidates<'a>(scan: &'a Scan, d: &Defaults, prefs: &crate::settings::Settings) -> Vec<(Action, Picks<'a>)> {
    let files = scan.files(d.subfolders);
    let total = files.len();
    let of = |fam: &str| files.iter().copied().filter(|i| i.is(fam)).collect::<Vec<_>>();
    let mut out: Vec<(Action, Picks)> = Vec::new();
    let seq = find_sequence(&files);

    // Image sequence.
    if let Some(s) = &seq {
        let frames: Vec<&Item> = files.iter().copied().filter(|i| s.frames.iter().any(|(_, p)| *p == i.path)).collect();
        let secs = s.frames.len() as f64 / 24.0;
        let gaps = if s.gaps.is_empty() { String::new() } else { format!(" · {} missing frame{} (the one before is held)", s.gaps.len(), if s.gaps.len() == 1 { "" } else { "s" }) };
        let range = format!("{}{} → {}", s.prefix, s.frames[0].0, s.frames[s.frames.len() - 1].0);
        let dims = if s.width > 0 { format!(" · {} × {}", s.width, s.height) } else { String::new() };
        out.push((
            Action::Sequence,
            Picks {
                files: frames.clone(),
                label: "Sequence → video".into(),
                icon: "frames",
                sub: noun("frame", frames.len()),
                what: format!("{range} · {secs:.1} s at 24 fps{dims}{gaps}"),
                out: one_file_words(scan, "mp4"),
                score: 100,
            },
        ));
        out.push((
            Action::SequenceGif,
            Picks { files: frames.clone(), label: "Sequence → GIF".into(), icon: "gif", sub: noun("frame", frames.len()), what: "480 px wide, 15 fps".into(), out: one_file_words(scan, "gif"), score: 85 },
        ));
    }

    // Compress all.
    let compressible: Vec<&Item> = files.iter().copied().filter(|i| COMPRESS_TARGETS.iter().any(|(f, _)| i.is(f))).collect();
    if !compressible.is_empty() {
        let mut kinds: Vec<String> = Vec::new();
        for (fam, _) in COMPRESS_TARGETS {
            let n = compressible.iter().filter(|i| i.is(fam)).count();
            if n > 0 {
                kinds.push(noun(fam, n));
            }
        }
        out.push((
            Action::Compress,
            Picks {
                sub: files_word(compressible.len()),
                label: "Compress all".into(),
                icon: "compress",
                what: format!("Smallest that looks the same · {}", kinds.join(", ")),
                out: into_words(d.into, scan, "compressed"),
                score: 80,
                files: compressible,
            },
        ));
    }

    // Photos → PDF (not when they're a video sequence).
    let pictures = of("image");
    if pictures.len() >= 2 && seq.is_none() {
        out.push((
            Action::PhotosPdf,
            Picks {
                sub: noun("page", pictures.len()),
                label: "Photos → PDF".into(),
                icon: "pdf",
                what: "One PDF, one page per picture, in name order".into(),
                out: one_file_words(scan, "pdf"),
                score: if share(pictures.len(), total) >= 0.8 { 90 } else { 40 },
                files: pictures.clone(),
            },
        ));
    }

    // Merge PDFs.
    let pdfs = of("pdf");
    if pdfs.len() >= 2 {
        let docs = of("doc").len();
        let what = if docs > 0 {
            format!("One PDF, in name order · the {} aren't included", noun("doc", docs))
        } else {
            "One PDF, in name order".into()
        };
        out.push((
            Action::MergePdf,
            Picks { sub: noun("pdf", pdfs.len()), label: "Merge PDFs".into(), icon: "merge", what, out: one_file_words(scan, "pdf"), score: if share(pdfs.len(), total) >= 0.8 { 90 } else { 45 }, files: pdfs.clone() },
        ));
    }

    // All → <format>, for each kind found (most files first).
    let mut fams: Vec<(String, usize)> = Vec::new();
    for i in &files {
        if let Some(f) = &i.family {
            match fams.iter_mut().find(|(x, _)| x == f) {
                Some((_, n)) => *n += 1,
                None => fams.push((f.clone(), 1)),
            }
        }
    }
    fams.sort_by(|a, b| b.1.cmp(&a.1));
    let mut labels_seen: HashMap<String, usize> = HashMap::new();
    for (fam, n_fam) in &fams {
        if matches!(fam.as_str(), "pdf" | "archive") {
            continue;
        }
        let these = of(fam);
        let mut offered = 0;
        for (tid, label) in wheel::format_targets(fam, prefs) {
            // Pictures into PDF is one PDF of them all: that's "Photos → PDF".
            if matches!(tid.as_str(), "image.pdf" | "raw.pdf") {
                continue;
            }
            let picked: Vec<&Item> = these.iter().copied().filter(|i| wheel::target_applies(&tid, &i.ext)).collect();
            if picked.is_empty() {
                continue;
            }
            let target_ext = wheel::target_ext(&tid);
            let already = these.iter().filter(|i| target_ext.as_deref().is_some_and(|e| wheel::same_format(&i.ext, e))).count();
            let cant = these.len() - picked.len() - already;
            let what = if already + cant > 0 {
                let mut w = format!("Only {} need{} it", noun(fam, picked.len()), if picked.len() == 1 { "s" } else { "" });
                if already > 0 {
                    w.push_str(&format!(" · {already} already {label}"));
                }
                if cant > 0 {
                    w.push_str(&format!(" · {cant} can't become {label}"));
                }
                w.push_str(" (left alone)");
                w
            } else {
                format!("Every {} in the folder", noun(fam, 1).trim_start_matches("1 "))
            };
            let suffix = label.to_lowercase();
            *labels_seen.entry(label.clone()).or_default() += 1;
            out.push((
                Action::ConvertAll(tid.clone()),
                Picks {
                    sub: noun(fam, picked.len()),
                    label: format!("All → {label}"),
                    icon: wheel::target_icon(&tid),
                    what,
                    out: into_words(d.into, scan, &suffix),
                    score: if offered == 0 { 60 + (10.0 * share(*n_fam, total)) as u32 } else { 20 },
                    files: picked,
                },
            ));
            offered += 1;
            if offered >= 3 {
                break;
            }
        }
    }

    // Join clips / audio.
    let videos = of("video");
    if videos.len() >= 2 {
        out.push((
            Action::JoinVideo,
            Picks {
                sub: noun("video", videos.len()),
                label: "Join clips".into(),
                icon: "join",
                what: "One video, in name order · without re-encoding when every clip matches, else matched to the first".into(),
                out: one_file_words(scan, "mp4"),
                score: 50,
                files: videos.clone(),
            },
        ));
    }
    let audio = of("audio");
    if audio.len() >= 2 {
        out.push((
            Action::JoinAudio,
            Picks {
                sub: noun("audio", audio.len()),
                label: "Join audio".into(),
                icon: "join",
                what: "One recording, in name order".into(),
                out: one_file_words(scan, &join_audio_ext(&audio)),
                score: if share(audio.len(), total) >= 0.8 { 66 } else { 48 },
                files: audio.clone(),
            },
        ));
    }
    if !videos.is_empty() {
        out.push((
            Action::AudioOnly,
            Picks {
                sub: noun("video", videos.len()),
                label: "Audio only".into(),
                icon: "audio",
                what: format!("MP3 from every video · {}", wheel::mp3_words(prefs.quality.mp3)),
                out: into_words(d.into, scan, "mp3"),
                score: 42,
                files: videos.clone(),
            },
        ));
    }
    if !audio.is_empty() {
        out.push((
            Action::Normalize,
            Picks {
                sub: noun("audio", audio.len()),
                label: "Normalize all".into(),
                icon: "level",
                what: "The same loudness on every file (-14 LUFS)".into(),
                out: into_words(d.into, scan, "normalized"),
                score: if share(audio.len(), total) >= 0.8 { 72 } else { 45 },
                files: audio.clone(),
            },
        ));
    }

    // One spreadsheet from CSV / TSV files.
    let tables: Vec<&Item> = files.iter().copied().filter(|i| matches!(i.ext.as_str(), "csv" | "tsv" | "tab")).collect();
    if tables.len() >= 2 {
        out.push((
            Action::Sheets,
            Picks { sub: files_word(tables.len()), label: "One spreadsheet".into(), icon: "table", what: "One XLSX, a sheet per file, in name order".into(), out: one_file_words(scan, "xlsx"), score: 55, files: tables },
        ));
    }

    // Extract every archive.
    let archives = of("archive");
    if !archives.is_empty() {
        out.push((
            Action::ExtractAll,
            Picks { sub: noun("archive", archives.len()), label: "Extract all".into(), icon: "extract", what: "Each archive into its own folder".into(), out: "→ next to each archive".into(), score: 58, files: archives },
        ));
    }

    // Remove GPS.
    let gps: Vec<&Item> = files.iter().copied().filter(|i| GPS_EXT.contains(&i.ext.as_str())).collect();
    if !gps.is_empty() {
        out.push((
            Action::RemoveGps,
            Picks {
                sub: noun("image", gps.len()),
                label: "Remove GPS".into(),
                icon: "pin",
                what: "Location removed from every picture that has one · pixels untouched".into(),
                out: into_words(d.into, scan, "no GPS"),
                score: 30,
                files: gps,
            },
        ));
    }

    // Two slots with the same label (pictures and RAW photos → JPG): say which kind.
    for (a, p) in out.iter_mut() {
        if let Action::ConvertAll(t) = a {
            let short = p.label.trim_start_matches("All → ").to_string();
            if labels_seen.get(&short).copied().unwrap_or(0) > 1 {
                let fam = t.split('.').next().unwrap_or("");
                p.label = format!("{} → {short}", if fam == "raw" { "RAW" } else { "All" });
            }
        }
    }
    out.sort_by(|a, b| b.1.score.cmp(&a.1.score));
    out
}

fn slot_of(action: &Action, p: &Picks, scan: &Scan, d: &Defaults) -> Slot {
    let total = scan.files(d.subfolders).len();
    let left = total.saturating_sub(p.files.len());
    let hidden = scan.alone.len();
    let leaves = match (left, hidden) {
        (0, 0) => String::new(),
        (0, h) => format!("Leaves {h} hidden or system file{} alone", if h == 1 { "" } else { "s" }),
        (l, 0) => format!("Leaves {} alone", files_word(l)),
        (l, h) => format!("Leaves {} alone (and {h} hidden)", files_word(l)),
    };
    Slot {
        id: action.id(),
        label: p.label.clone(),
        icon: p.icon.into(),
        hint: p.what.clone(),
        family: "folder".into(),
        tag: None,
        kind: SlotKind::Target,
        count: p.files.len(),
        sub: Some(p.sub.clone()),
        detail: (!leaves.is_empty()).then_some(leaves),
        out: (!p.out.is_empty()).then(|| p.out.clone()),
        ..Slot::default()
    }
}

/// The folder wheel: up to five best slots, then Fit to size and Zip, and
/// More (the rest, plus Watch this folder).
pub fn slots(scan: &Scan, d: &Defaults, prefs: &crate::settings::Settings) -> (Vec<Slot>, Vec<Slot>) {
    let all: Vec<Slot> = candidates(scan, d, prefs).iter().map(|(a, p)| slot_of(a, p, scan, d)).collect();
    // Zip and Fit take every visible file, earlier results too.
    let everything = scan.everything(d.subfolders);
    let n_all = everything.len();
    let media = everything.iter().filter(|i| is_packed_already(&i.ext)).map(|i| i.bytes).sum::<u64>();
    let bytes = everything.iter().map(|i| i.bytes).sum::<u64>();
    let fit = Slot {
        id: Action::Fit.id(),
        label: "Fit to size".into(),
        icon: "fit".into(),
        hint: "Make the folder fit a size: a compressed copy, one zip, or a zip in parts".into(),
        family: "folder".into(),
        kind: SlotKind::Target,
        count: n_all,
        sub: Some(format!("{} →", human_size(bytes))),
        out: Some("Pick the size next".into()),
        ..Slot::default()
    };
    let left_out = scan.alone_words();
    let zip = Slot {
        id: Action::Zip.id(),
        label: "Zip".into(),
        icon: "archive".into(),
        hint: if media * 5 >= bytes * 4 {
            "Videos and pictures go in as they are (they're compressed already): fast".into()
        } else {
            "Everything in the folder, in one zip".into()
        },
        family: "folder".into(),
        kind: SlotKind::Target,
        count: n_all,
        sub: Some(files_word(n_all)),
        detail: (!left_out.is_empty()).then(|| format!("Left out: {left_out}")),
        out: Some(format!("{} · Shift+click: 7Z, TAR.GZ, password, parts", one_file_words(scan, "zip"))),
        ..Slot::default()
    };
    let watch = Slot {
        id: Action::Watch.id(),
        label: "Watch this folder…".into(),
        icon: "eye".into(),
        hint: "Convert new files as they arrive".into(),
        family: "folder".into(),
        kind: SlotKind::Target,
        count: 0,
        out: Some("Opens Settings › Watched folders".into()),
        ..Slot::default()
    };
    let mut main: Vec<Slot> = all.iter().take(5).cloned().collect();
    let mut more: Vec<Slot> = all.iter().skip(5).cloned().collect();
    if n_all > 0 {
        main.push(fit);
        main.push(zip);
    }
    more.push(watch);
    let more_hint = format!("More: {}", more.iter().map(|s| s.label.as_str()).collect::<Vec<_>>().join(", "));
    main.push(Slot { id: "more".into(), label: "More".into(), icon: "more".into(), hint: more_hint, family: "folder".into(), kind: SlotKind::More, count: more.len(), sub: Some(more.len().to_string()), ..Slot::default() });
    more.push(Slot { id: "back".into(), label: "Back".into(), icon: "back".into(), hint: "Back to the folder wheel".into(), family: "folder".into(), kind: SlotKind::Back, ..Slot::default() });
    (main, more)
}

/// Files that zipping can't make smaller (already compressed).
fn is_packed_already(ext: &str) -> bool {
    matches!(
        ext,
        "mp4" | "mov" | "mkv" | "webm" | "avi" | "m4v" | "mp3" | "m4a" | "aac" | "ogg" | "opus" | "flac" | "jpg" | "jpeg" | "png" | "webp" | "avif" | "heic" | "heif" | "gif" | "zip" | "7z" | "rar" | "gz" | "pdf" | "docx" | "xlsx" | "pptx"
    )
}

/// The join of these recordings: their own format when they all share one FFmpeg writes, else WAV.
fn join_audio_ext(files: &[&Item]) -> String {
    let first = files.first().map(|i| i.ext.clone()).unwrap_or_else(|| "wav".into());
    let same = files.iter().all(|i| i.ext == first);
    if same && !convert::audio_codec_for(&first, 320, None).is_empty() { first } else { "wav".into() }
}

/// The names the command line and AI apps use for folder actions.
pub const WORDS: &str = "compress, fit (with a size), zip, 7z, tar.gz, merge-pdf, photos-pdf, join, join-audio, sequence, sequence-gif, audio, normalize, no-gps, extract, sheets, or a format (mp4, jpg, webp, mp3, pdf, …)";

/// What a word means for a folder: the actions to run (a format can mean one
/// per kind of file: "jpg" for pictures and for RAW photos), and the zip format.
pub fn actions_for_word(scan: &Scan, word: &str, sized: bool, subfolders: bool) -> Result<(Vec<Action>, Option<PackFormat>), String> {
    let w = word.trim().trim_start_matches('.').to_lowercase();
    if sized {
        return match w.as_str() {
            "" | "compress" | "fit" => Ok((vec![Action::Fit], None)),
            _ => Err("--size works with compress (or fit) on a folder.".into()),
        };
    }
    let one = |a: Action| Ok((vec![a], None));
    match w.as_str() {
        "compress" => one(Action::Compress),
        "fit" => Err("fit needs a size (--size 4GB).".into()),
        "zip" => Ok((vec![Action::Zip], Some(PackFormat::Zip))),
        "7z" => Ok((vec![Action::Zip], Some(PackFormat::SevenZ))),
        "tar.gz" | "targz" | "tgz" => Ok((vec![Action::Zip], Some(PackFormat::TarGz))),
        "merge" | "merge-pdf" | "merge-pdfs" => one(Action::MergePdf),
        "photos-pdf" | "pictures-pdf" => one(Action::PhotosPdf),
        "join" | "join-video" | "join-clips" => one(Action::JoinVideo),
        "join-audio" => one(Action::JoinAudio),
        "sequence" | "sequence-video" => one(Action::Sequence),
        "sequence-gif" => one(Action::SequenceGif),
        "audio" | "audio-only" => one(Action::AudioOnly),
        "normalize" => one(Action::Normalize),
        "no-gps" | "gps" | "remove-gps" => one(Action::RemoveGps),
        "extract" | "extract-all" | "unzip" => one(Action::ExtractAll),
        "sheets" | "spreadsheet" | "one-spreadsheet" => one(Action::Sheets),
        "watch" => Err("Watching a folder is set up in Convertino's Settings › Watched folders.".into()),
        _ => {
            let mut targets: Vec<String> = Vec::new();
            for i in scan.files(subfolders) {
                if let Some(c) = wheel::resolve(&w, &i.ext) {
                    let multi_pdf = matches!(c.id.as_str(), "image.pdf" | "raw.pdf");
                    if !c.multi && !multi_pdf && !targets.contains(&c.id) {
                        targets.push(c.id);
                    }
                }
            }
            if targets.is_empty() {
                Err(format!("Nothing in {} can become \"{word}\". For a folder: {WORDS}.", scan.name()))
            } else {
                Ok((targets.into_iter().map(Action::ConvertAll).collect(), None))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Planning

/// One file's fate, for the Details list and the command line.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    /// Its path inside the folder.
    pub name: String,
    pub status: String,
    pub kind: RowKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RowKind {
    Done,
    /// Already small enough / already compact: the original was kept, nothing made.
    Kept,
    Failed,
    /// Not something this slot works on, or hidden.
    Alone,
    /// Done in an earlier run (its result is still there).
    DoneBefore,
    /// The job stopped before it got to this file.
    NotStarted,
}

/// What a folder pick will do.
#[derive(Debug, Clone)]
pub struct Plan {
    pub action: Action,
    /// "Compress Valorant clips" (the card's title while it runs).
    pub title: String,
    /// "compressed", "converted" (the done card's counts).
    pub verb: &'static str,
    /// Steps, in groups that run one after another (pictures 4 at a time, then videos one by one).
    pub groups: Vec<Vec<Step>>,
    /// Files whose fate is known before it runs.
    pub rows: Vec<Row>,
    /// Folders this job creates for its results (Undo removes them whole).
    pub new_roots: Vec<PathBuf>,
    /// New results take the originals' places; the originals move to the Holding Area.
    pub replace: bool,
    /// Replace: the Holding Area batch the originals go in ("2026-10-13-09-41-07 convertino").
    pub holding: Option<String>,
    /// Scratch folders to remove when the job ends.
    pub scratch: Vec<PathBuf>,
    pub roots: Vec<PathBuf>,
    pub notes: Vec<String>,
}

impl Plan {
    /// One result made from many files (zip, merge, join, sequence, …).
    pub fn is_single(&self) -> bool {
        !self.action.is_batch() && self.action != Action::ExtractAll && !matches!(self.action, Action::Fit)
    }
    pub fn step_count(&self) -> usize {
        self.groups.iter().map(Vec::len).sum()
    }
}

/// A row name: "old/clip.mp4", prefixed with the folder's name when several were picked.
fn row_name(scan: &Scan, path: &Path) -> String {
    for (r, root) in scan.roots.iter().enumerate() {
        if let Ok(rel) = path.strip_prefix(root) {
            let rel = rel.to_string_lossy().replace('\\', "/");
            return if scan.roots.len() > 1 { format!("{}/{rel}", folder_name(&scan.roots[r])) } else { rel };
        }
    }
    file_name(path)
}

/// Where Convertino may write next to `dir`: there, the Settings folder, or Downloads.
fn base_dir(dir: &Path) -> PathBuf {
    match crate::settings::get().output_folder() {
        Some(folder) if std::fs::create_dir_all(&folder).is_ok() => convert::writable_dir(&folder),
        _ => convert::writable_dir(dir),
    }
}

/// The parent of the selected folders (where one-file results go).
fn parent_of(scan: &Scan) -> PathBuf {
    scan.roots[0].parent().map(Path::to_path_buf).unwrap_or_else(|| scan.roots[0].clone())
}

fn newer_or_same(result: &Path, original: &Path) -> bool {
    let (Ok(r), Ok(o)) = (result.metadata(), original.metadata()) else { return false };
    if r.len() == 0 {
        return false;
    }
    match (r.modified(), o.modified()) {
        (Ok(a), Ok(b)) => a >= b,
        _ => true,
    }
}

fn order_items(files: &mut [&Item], order: Order) {
    match order {
        Order::Name => files.sort_by(|a, b| a.root.cmp(&b.root).then_with(|| path_cmp(&a.rel, &b.rel))),
        Order::Date => files.sort_by(|a, b| a.modified.cmp(&b.modified).then_with(|| path_cmp(&a.rel, &b.rel))),
    }
}

/// Plans a pick. `fit_data`: measured files for Fit to size (see `fit_measure`).
pub fn plan(scan: &Scan, action: &Action, opts: &Opts, q: &Quality, prefs: &crate::settings::Settings) -> Result<Plan, String> {
    let d = opts.defaults();
    let all_files = scan.files(opts.subfolders);
    let mut cands = candidates(scan, &d, prefs);
    let mut rows: Vec<Row> = scan.alone.iter().map(|(p, w)| Row { name: row_name(scan, p), status: format!("Left alone · {}", w.words()), kind: RowKind::Alone }).collect();
    for (p, w) in scan.skipped_dirs.iter().filter(|(_, w)| *w != Alone::Earlier) {
        rows.push(Row { name: format!("{}/", row_name(scan, p)), status: format!("Folder not read · {}", w.words()), kind: RowKind::Alone });
    }
    if !opts.subfolders {
        for i in scan.items.iter().filter(|i| i.in_subfolder()) {
            rows.push(Row { name: row_name(scan, &i.path), status: "Left alone · in a subfolder".into(), kind: RowKind::Alone });
        }
    }
    if !matches!(action, Action::Zip | Action::Fit) {
        for i in scan.items.iter().filter(|i| i.earlier && (opts.subfolders || !i.in_subfolder())) {
            rows.push(Row { name: row_name(scan, &i.path), status: format!("Left alone · {}", Alone::Earlier.words()), kind: RowKind::Alone });
        }
    }
    let name = scan.name();
    let mut plan = Plan {
        action: action.clone(),
        title: String::new(),
        verb: "converted",
        groups: Vec::new(),
        rows: Vec::new(),
        new_roots: Vec::new(),
        replace: false,
        holding: None,
        scratch: Vec::new(),
        roots: scan.roots.clone(),
        notes: Vec::new(),
    };
    let everything = scan.everything(opts.subfolders);
    let picked: Vec<&Item> = match action {
        Action::Zip | Action::Fit => everything.clone(),
        // Any format, not only the ones on the wheel (the command line asks for any).
        Action::ConvertAll(t) => {
            let fam = wheel::target_family(t).ok_or_else(|| format!("Convertino doesn't know {t}."))?;
            let v: Vec<&Item> = all_files.iter().copied().filter(|i| i.is(&fam) && wheel::target_applies(t, &i.ext)).collect();
            if v.is_empty() {
                return Err(format!("{name} has nothing for this."));
            }
            v
        }
        Action::Watch => return Err("Watch this folder opens Settings.".into()),
        _ => {
            let at = cands.iter().position(|(a, _)| a == action).ok_or_else(|| format!("{} has nothing for this.", name))?;
            cands.swap_remove(at).1.files
        }
    };
    let leave_rest = |rows: &mut Vec<Row>, picked: &[&Item], why: &str| {
        let set: HashSet<&Path> = picked.iter().map(|i| i.path.as_path()).collect();
        for i in &all_files {
            if !set.contains(i.path.as_path()) {
                rows.push(Row { name: row_name(scan, &i.path), status: format!("Left alone · {why}"), kind: RowKind::Alone });
            }
        }
    };

    match action {
        Action::Compress | Action::ConvertAll(_) | Action::AudioOnly | Action::Normalize | Action::RemoveGps => {
            let (suffix, why): (String, &str) = match action {
                Action::Compress => ("compressed".into(), "not something Compress works on"),
                Action::ConvertAll(t) => (wheel::target_label(t).map(|l| l.to_lowercase()).unwrap_or_default(), "not this kind of file, or already in that format"),
                Action::AudioOnly => ("mp3".into(), "not a video"),
                Action::Normalize => ("normalized".into(), "not an audio file"),
                _ => ("no GPS".into(), "not a picture Convertino can remove GPS from"),
            };
            plan.verb = match action {
                Action::Compress => "compressed",
                Action::Normalize => "normalized",
                Action::RemoveGps => "cleaned",
                Action::AudioOnly => "extracted",
                _ => "converted",
            };
            plan.title = match action {
                Action::Compress => format!("Compress {name}"),
                Action::ConvertAll(t) => format!("{name} → {}", wheel::target_label(t).unwrap_or_default()),
                Action::AudioOnly => format!("Audio from {name}"),
                Action::Normalize => format!("Normalize {name}"),
                _ => format!("Remove GPS · {name}"),
            };
            leave_rest(&mut rows, &picked, why);
            // A plan per kind of file (each has its own target).
            let mut by_target: Vec<(String, Vec<PathBuf>)> = Vec::new();
            for i in &picked {
                let t = match action {
                    Action::Compress => COMPRESS_TARGETS.iter().find(|(f, _)| i.is(f)).map(|(_, t)| t.to_string()),
                    Action::ConvertAll(t) => Some(t.clone()),
                    Action::AudioOnly => Some("video.mp3".into()),
                    Action::Normalize => Some("audio.normalize".into()),
                    _ => Some("folder.gps".into()),
                };
                let Some(t) = t else { continue };
                match by_target.iter_mut().find(|(x, _)| *x == t) {
                    Some((_, v)) => v.push(i.path.clone()),
                    None => by_target.push((t, vec![i.path.clone()])),
                }
            }
            // Pictures, PDFs and audio first (several at a time), video last (one by one).
            by_target.sort_by_key(|(t, _)| t.starts_with("video."));
            // Where results go.
            let mut new_root_for: Vec<Option<PathBuf>> = vec![None; scan.roots.len()];
            if opts.into == Into::New {
                for (r, root) in scan.roots.iter().enumerate() {
                    if !picked.iter().any(|i| i.root == r) {
                        continue;
                    }
                    let parent = root.parent().map(Path::to_path_buf).unwrap_or_else(|| root.clone());
                    let base = base_dir(&parent);
                    let stem = format!("{} ({suffix})", folder_name(root));
                    let first = base.join(&stem);
                    let dest = if first.exists() && !opts.skip_done { convert::unique_output(&base, &stem, "") } else { first };
                    if !dest.exists() {
                        plan.new_roots.push(dest.clone());
                    }
                    new_root_for[r] = Some(dest);
                }
            }
            plan.replace = opts.into == Into::Replace;
            if plan.replace {
                // Not the Recycle Bin: Windows deletes files too big for it without asking.
                plan.holding = Some(crate::holding::batch_name());
            }
            let job_tag = std::process::id();
            for (target, files) in by_target {
                let mut steps = if target == "folder.gps" {
                    files
                        .iter()
                        .map(|f| {
                            let (dir, stem, ext) = convert::split(f);
                            Step { inputs: vec![f.clone()], dir, stem: format!("{stem} (no GPS)"), ext, folder: false, op: Op::Folder(Box::new(FolderOp::StripGps)) }
                        })
                        .collect()
                } else {
                    match convert::plan_sized(&target, &files, q, None) {
                        Ok((steps, _notes)) => steps,
                        Err(e) if e.starts_with(size::ALREADY) => Vec::new(),
                        Err(e) => {
                            for f in &files {
                                rows.push(Row { name: row_name(scan, f), status: format!("Couldn't plan: {e}"), kind: RowKind::Failed });
                            }
                            continue;
                        }
                    }
                };
                // Files the plan left out are already small enough.
                let planned: HashSet<PathBuf> = steps.iter().map(|s| s.inputs[0].clone()).collect();
                for f in &files {
                    if !planned.contains(f) {
                        rows.push(Row { name: row_name(scan, f), status: "Already small enough · left as it is".into(), kind: RowKind::Kept });
                    }
                }
                for st in steps.iter_mut() {
                    let input = st.inputs[0].clone();
                    let item = picked.iter().find(|i| i.path == input);
                    let (src_dir, src_stem, _) = convert::split(&input);
                    match opts.into {
                        Into::New => {
                            if let (Some(item), Some(root)) = (item, item.and_then(|i| new_root_for[i.root].clone())) {
                                st.dir = item.rel.parent().map(|p| root.join(p)).unwrap_or(root);
                                st.stem = src_stem.clone();
                            }
                        }
                        Into::Beside => {}
                        Into::Replace => {
                            let staging = src_dir.join(format!(".convertino-new-{job_tag}"));
                            if !plan.scratch.contains(&staging) {
                                plan.scratch.push(staging.clone());
                            }
                            st.dir = staging;
                            st.stem = src_stem.clone();
                        }
                    }
                }
                // Done in an earlier run: its result is still there.
                if opts.skip_done && opts.into != Into::Replace {
                    steps.retain(|st| {
                        if st.folder {
                            return true;
                        }
                        let expected = st.dir.join(format!("{}.{}", st.stem, st.ext));
                        if expected != st.inputs[0] && expected.exists() && newer_or_same(&expected, &st.inputs[0]) {
                            rows.push(Row { name: row_name(scan, &st.inputs[0]), status: format!("Done in an earlier run · {}", file_name(&expected)), kind: RowKind::DoneBefore });
                            false
                        } else {
                            true
                        }
                    });
                }
                if !steps.is_empty() {
                    plan.groups.push(steps);
                }
            }
        }
        Action::PhotosPdf | Action::MergePdf | Action::Sheets | Action::JoinVideo | Action::JoinAudio | Action::Sequence | Action::SequenceGif => {
            leave_rest(
                &mut rows,
                &picked,
                match action {
                    Action::PhotosPdf => "not a picture",
                    Action::MergePdf => "not a PDF",
                    Action::Sheets => "not a CSV file",
                    Action::JoinVideo => "not a video",
                    Action::JoinAudio => "not an audio file",
                    _ => "not a frame of the sequence",
                },
            );
            let mut ordered = picked.clone();
            if !matches!(action, Action::Sequence | Action::SequenceGif) {
                order_items(&mut ordered, opts.order);
            }
            let inputs: Vec<PathBuf> = ordered.iter().map(|i| i.path.clone()).collect();
            let dir = base_dir(&parent_of(scan));
            let stem = single_stem(scan);
            let (ext, op, title, verb) = match action {
                Action::PhotosPdf => ("pdf".to_string(), Op::Magick { args: Vec::new(), first_frame: false }, format!("{name} → PDF"), "added"),
                Action::MergePdf => ("pdf".to_string(), Op::PdfMerge, format!("Merge {name}"), "merged"),
                Action::Sheets => ("xlsx".to_string(), Op::Folder(Box::new(FolderOp::Sheets)), format!("{name} → one spreadsheet"), "added"),
                Action::JoinVideo => (join_video_ext(&ordered), Op::Folder(Box::new(FolderOp::JoinVideo)), format!("Join {name}"), "joined"),
                Action::JoinAudio => (join_audio_ext(&ordered), Op::Folder(Box::new(FolderOp::JoinAudio)), format!("Join {name}"), "joined"),
                _ => {
                    let seq = find_sequence(&all_files).ok_or("The numbered frames aren't there any more.")?;
                    if opts.gaps == Gaps::Stop && !seq.gaps.is_empty() {
                        return Err(format!("{} frame{} missing ({}). Choose \"Hold the previous frame\" to make it anyway.", seq.gaps.len(), if seq.gaps.len() == 1 { " is" } else { "s are" }, gap_words(&seq.gaps)));
                    }
                    let gif = *action == Action::SequenceGif || opts.format == SeqFormat::Gif;
                    let format = if gif { SeqFormat::Gif } else { opts.format };
                    let sound = if opts.sound && !gif { all_files.iter().find(|i| i.is("audio")).map(|i| i.path.clone()) } else { None };
                    if !seq.gaps.is_empty() {
                        plan.notes.push(format!("{} missing frame{} ({}): the frame before {} held longer.", seq.gaps.len(), if seq.gaps.len() == 1 { "" } else { "s" }, gap_words(&seq.gaps), if seq.gaps.len() == 1 { "it was" } else { "each was" }));
                    }
                    let op = Op::Folder(Box::new(FolderOp::Sequence { fps: opts.fps.clamp(1.0, 240.0), format, sound, numbers: seq.frames.iter().map(|(n, _)| *n).collect() }));
                    (format.ext().to_string(), op, format!("{name} → {}", if gif { "GIF" } else { "video" }), "used")
                }
            };
            plan.title = title;
            plan.verb = verb;
            if inputs.len() < 2 && !matches!(action, Action::Sequence | Action::SequenceGif) {
                return Err("This needs at least two files.".into());
            }
            plan.groups.push(vec![Step { inputs, dir, stem, ext, folder: false, op }]);
        }
        Action::ExtractAll => {
            leave_rest(&mut rows, &picked, "not an archive");
            plan.title = format!("Extract all · {name}");
            plan.verb = "extracted";
            let files: Vec<PathBuf> = picked.iter().map(|i| i.path.clone()).collect();
            plan.groups.push(convert::plan_with("archive.extract", &files, q)?);
        }
        Action::Zip => {
            plan.title = format!("Zip {name}");
            plan.verb = "added";
            let parent = parent_of(scan);
            let list: Vec<PathBuf> = picked.iter().filter_map(|i| i.path.strip_prefix(&parent).ok().map(Path::to_path_buf)).collect();
            let packed = picked.iter().filter(|i| is_packed_already(&i.ext)).map(|i| i.bytes).sum::<u64>();
            let bytes: u64 = picked.iter().map(|i| i.bytes).sum();
            let store = packed * 5 >= bytes * 4;
            let parts = opts.parts.filter(|p| *p > 0 && *p < bytes);
            let op = Op::Folder(Box::new(FolderOp::Pack { base: parent.clone(), list, format: opts.pack, password: opts.password.clone().filter(|p| !p.is_empty()), parts, store }));
            let stem = single_stem(scan);
            let (stem, ext, folder) = if parts.is_some() { (format!("{stem} (zip parts)"), opts.pack.ext().to_string(), true) } else { (stem, opts.pack.ext().to_string(), false) };
            let inputs: Vec<PathBuf> = picked.iter().map(|i| i.path.clone()).collect();
            if inputs.is_empty() {
                return Err(format!("{name} has no files to zip."));
            }
            plan.groups.push(vec![Step { inputs, dir: base_dir(&parent), stem, ext, folder, op }]);
        }
        Action::Fit => {
            let fit = opts.fit.ok_or("Pick a size first.")?;
            plan.title = format!("Fit {name} in {}", size::words(fit.bytes));
            plan.verb = "made to fit";
            fit_plan(scan, &everything, fit, q, &mut plan, &mut rows)?;
        }
        Action::Watch => unreachable!(),
    }
    plan.rows = rows;
    check_space(&plan, scan, opts)?;
    if plan.groups.iter().all(Vec::is_empty) {
        let nothing = plan.rows.iter().any(|r| matches!(r.kind, RowKind::DoneBefore | RowKind::Kept));
        return Err(if nothing { format!("{ALREADY_DONE}every file in {name} is already done or small enough.") } else { format!("{name} has nothing for this.") });
    }
    Ok(plan)
}

/// Refuses a job that can't fit on its drive (a 14 GB zip on a drive with 9 GB free).
fn check_space(plan: &Plan, scan: &Scan, opts: &Opts) -> Result<(), String> {
    let inputs: u64 = plan.groups.iter().flatten().flat_map(|s| s.inputs.iter()).filter(|p| p.is_file()).map(|p| p.metadata().map(|m| m.len()).unwrap_or(0)).sum();
    let need = match &plan.action {
        Action::Fit => opts.fit.map(|f| if f.mode == FitMode::Parts { inputs } else { f.bytes }).unwrap_or(0),
        // Compressed results are smaller than the originals; this is a rough floor.
        Action::Compress => inputs / 2,
        Action::Sequence | Action::SequenceGif | Action::ExtractAll | Action::Watch => 0,
        _ => inputs,
    };
    if need == 0 {
        return Ok(());
    }
    // Where it writes: next to the folder (new folder, one file), or among the files.
    let at = match (plan.new_roots.first(), plan.groups.first().and_then(|g| g.first())) {
        (Some(r), _) => r.parent().map(Path::to_path_buf).unwrap_or_else(|| r.clone()),
        (None, Some(st)) => st.dir.ancestors().find(|a| a.is_dir()).map(Path::to_path_buf).unwrap_or_else(|| parent_of(scan)),
        _ => parent_of(scan),
    };
    let Ok(free) = fs4::available_space(&at) else { return Ok(()) };
    let margin = 200 * MB;
    if free < need + margin {
        return Err(format!(
            "Not enough free space: this needs about {}, and {} has {} free. Free some space, or pick Replace the originals (Shift+click).",
            human_size(need),
            at.display(),
            human_size(free)
        ));
    }
    Ok(())
}

/// Starts the message when there's nothing left to do (not a failure).
pub const ALREADY_DONE: &str = "Nothing to do: ";

fn gap_words(gaps: &[u64]) -> String {
    let shown: Vec<String> = gaps.iter().take(3).map(|g| g.to_string()).collect();
    if gaps.len() > 3 { format!("{} and {} more", shown.join(", "), gaps.len() - 3) } else { shown.join(", ") }
}

/// The joined video's container: the clips' own when they all share one, else MP4.
fn join_video_ext(files: &[&Item]) -> String {
    let first = files.first().map(|i| i.ext.clone()).unwrap_or_default();
    if files.iter().all(|i| i.ext == first) && matches!(first.as_str(), "mp4" | "mov" | "mkv" | "webm" | "m4v") { first } else { "mp4".into() }
}

// ---------------------------------------------------------------------------
// Fit to a size

/// Measured files of a folder, for Fit to size.
#[derive(Debug, Clone, Default)]
pub struct FitData {
    pub kinds: Vec<(Kind, Vec<size::Info>)>,
    /// Files that are copied as they are (not pictures, video, PDF or audio).
    pub other: u64,
    pub total: u64,
    pub files: usize,
}

fn kind_of(i: &Item) -> Option<Kind> {
    match i.family.as_deref()? {
        "image" | "raw" => Some(Kind::Image),
        "video" => Some(Kind::Video),
        "pdf" => Some(Kind::Pdf),
        "audio" => Some(Kind::Audio),
        _ => None,
    }
}

/// Measures every picture, video, PDF and audio file (four at a time; cached).
/// `download`: fetch a missing converter (the job does; the wheel doesn't).
pub fn fit_measure(files: &[&Item], download: bool, progress: &(dyn Fn(usize, usize) + Sync)) -> Result<FitData, String> {
    let mut data = FitData { total: files.iter().map(|i| i.bytes).sum(), files: files.len(), ..FitData::default() };
    let media: Vec<(Kind, PathBuf)> = files.iter().filter_map(|i| kind_of(i).map(|k| (k, i.path.clone()))).collect();
    data.other = files.iter().filter(|i| kind_of(i).is_none()).map(|i| i.bytes).sum();
    let results: Mutex<Vec<Option<(Kind, size::Info)>>> = Mutex::new(vec![None; media.len()]);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let done = std::sync::atomic::AtomicUsize::new(0);
    let first_err: Mutex<Option<String>> = Mutex::new(None);
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| loop {
                let n = next.fetch_add(1, AtomicOrdering::SeqCst);
                let Some((k, p)) = media.get(n) else { break };
                let r = if download { size::info(*k, p) } else { size::quick_info(*k, p) };
                match r {
                    Ok(i) => {
                        if let Ok(mut v) = results.lock() {
                            v[n] = Some((*k, i));
                        }
                    }
                    Err(e) => {
                        if let Ok(mut f) = first_err.lock() {
                            f.get_or_insert(e);
                        }
                    }
                }
                progress(done.fetch_add(1, AtomicOrdering::SeqCst) + 1, media.len());
            });
        }
    });
    if let Some(e) = first_err.into_inner().unwrap_or(None) {
        return Err(e);
    }
    for (k, i) in results.into_inner().unwrap_or_default().into_iter().flatten() {
        match data.kinds.iter_mut().find(|(x, _)| *x == k) {
            Some((_, v)) => v.push(i),
            None => data.kinds.push((k, vec![i])),
        }
    }
    Ok(data)
}

fn kind_noun(k: Kind, n: usize) -> String {
    let fam = match k {
        Kind::Image => "image",
        Kind::Video => "video",
        Kind::Pdf => "pdf",
        Kind::Audio => "audio",
    };
    noun(fam, n)
}

/// Each kind's share of the room left for media: by how much each needs.
fn kind_budgets(data: &FitData, media: u64) -> Vec<u64> {
    let need: Vec<u64> = data.kinds.iter().map(|(k, infos)| infos.iter().map(|i| size::best(*k, i).max(1)).sum()).collect();
    let all: u64 = need.iter().sum::<u64>().max(1);
    need.iter().map(|n| (media as f64 * *n as f64 / all as f64) as u64).collect()
}

/// Room kept for the zip's own bookkeeping and rounding.
fn slack(bytes: u64, files: usize) -> u64 {
    bytes / 100 + files as u64 * 200
}

/// What `bytes` does to the folder in `mode`.
pub fn fit_preview(data: &FitData, bytes: u64, mode: FitMode) -> size::Preview {
    let mut p = size::Preview { fits: false, ok: true, short: String::new(), line1: String::new(), line2: String::new(), est: data.total, min: 0, alt: None };
    if mode == FitMode::Parts {
        let parts = data.total.div_ceil(bytes.max(1)).max(1);
        p.min = 1;
        p.fits = parts <= 1;
        p.short = if p.fits { "One part".into() } else { format!("{parts} parts") };
        p.line1 = if p.fits { "The zip already fits in one piece".into() } else { format!("{parts} pieces of up to {}", size::words(bytes)) };
        p.line2 = "Nothing is re-encoded · open the first piece with 7-Zip to put it back together".into();
        return p;
    }
    let floor: u64 = data.kinds.iter().map(|(k, infos)| infos.iter().map(|i| size::floor(*k, i).min(i.bytes)).sum::<u64>()).sum::<u64>() + data.other;
    p.min = floor + slack(floor, data.files);
    if data.total + slack(data.total, data.files) <= bytes {
        p.fits = true;
        p.short = "Already fits".into();
        p.line1 = format!("The folder is already {}", size::words(data.total));
        p.line2 = if mode == FitMode::Zip { "Zipped as it is".into() } else { "Copied as it is".into() };
        return p;
    }
    let room = bytes.saturating_sub(data.other + slack(bytes, data.files));
    if room == 0 || p.min > bytes {
        p.ok = false;
        p.short = "Too small".into();
        p.line1 = format!("The folder can't fit in {}", size::words(bytes));
        p.line2 = format!("The smallest that still looks right is about {}", size::words(p.min));
        return p;
    }
    let budgets = kind_budgets(data, room);
    let mut est = data.other;
    let mut lead: Option<(u64, size::Preview, Kind, usize)> = None;
    let mut ok = true;
    for ((k, infos), b) in data.kinds.iter().zip(budgets) {
        let pv = size::preview(*k, infos, b, true, None);
        ok &= pv.ok || pv.fits;
        est += pv.est.min(b.max(pv.est));
        let biggest: u64 = infos.iter().map(|i| i.bytes).sum();
        if lead.as_ref().is_none_or(|(b0, ..)| biggest > *b0) {
            lead = Some((biggest, pv, *k, infos.len()));
        }
    }
    p.est = est;
    if !ok {
        p.ok = false;
        p.short = "Too small".into();
        p.line1 = format!("The folder can't fit in {}", size::words(bytes));
        p.line2 = format!("The smallest that still looks right is about {}", size::words(p.min));
        return p;
    }
    if let Some((_, lp, k, n)) = lead {
        p.short = lp.short.clone();
        p.line1 = format!("{} · {}", kind_noun(k, n), if lp.fits { "already small enough".to_string() } else { lp.short.clone() });
        p.line2 = if data.other > 0 { format!("Other files copied as they are ({})", size::words(data.other)) } else { lp.line2.clone() };
    }
    p
}

/// A size offered on the folder's size ring.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FitPreset {
    pub bytes: u64,
    pub label: String,
    pub name: String,
    pub preview: size::Preview,
}

const MB: u64 = size::MB;
const GB: u64 = 1_000 * MB;

/// Up to five sizes worth offering: common limits, then steps down from "looks the same".
pub fn fit_presets(data: &FitData, mode: FitMode) -> Vec<FitPreset> {
    let mut out: Vec<FitPreset> = Vec::new();
    let near = |a: u64, b: u64| (a as f64 - b as f64).abs() / (a.max(b).max(1) as f64) < 0.2;
    let mut add = |bytes: u64, name: &str| {
        if out.len() >= 5 || bytes == 0 || bytes >= data.total || out.iter().any(|p| near(p.bytes, bytes)) {
            return;
        }
        let pv = fit_preview(data, bytes, mode);
        if !pv.ok || pv.fits {
            return;
        }
        if mode == FitMode::Parts && data.total / bytes.max(1) > 999 {
            return;
        }
        out.push(FitPreset { bytes, label: size::words(bytes), name: name.into(), preview: pv });
    };
    if mode == FitMode::Parts {
        for (b, n) in [(18 * MB, "Email"), (100 * MB, ""), (500 * MB, ""), (2 * GB, "WhatsApp"), (4 * GB, "USB stick")] {
            add(b, n);
        }
    } else {
        let best: u64 = data.kinds.iter().map(|(k, infos)| infos.iter().map(|i| size::best(*k, i).min(i.bytes)).sum::<u64>()).sum::<u64>() + data.other;
        for (b, n) in [(10 * MB, "Discord"), (18 * MB, "Email"), (2 * GB, "WhatsApp"), (4 * GB, "USB stick")] {
            add(b, n);
        }
        for f in [1.0, 0.5, 0.25, 0.1] {
            add(size::tidy(best as f64 * f), "");
        }
    }
    out.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    out
}

fn fit_plan(scan: &Scan, files: &[&Item], fit: Fit, q: &Quality, plan: &mut Plan, rows: &mut Vec<Row>) -> Result<(), String> {
    let parent = parent_of(scan);
    let stem = single_stem(scan);
    if fit.mode == FitMode::Parts {
        let list: Vec<PathBuf> = files.iter().filter_map(|i| i.path.strip_prefix(&parent).ok().map(Path::to_path_buf)).collect();
        let op = Op::Folder(Box::new(FolderOp::Pack { base: parent.clone(), list, format: PackFormat::Zip, password: None, parts: Some(fit.bytes), store: true }));
        plan.groups.push(vec![Step { inputs: files.iter().map(|i| i.path.clone()).collect(), dir: base_dir(&parent), stem: format!("{stem} (zip parts)"), ext: "zip".into(), folder: true, op }]);
        plan.verb = "added";
        return Ok(());
    }
    let data = fit_measure(files, true, &|_, _| {})?;
    let pv = fit_preview(&data, fit.bytes, fit.mode);
    if !pv.ok {
        return Err(format!("{}. {}.", pv.line1, pv.line2));
    }
    // Where the copy is made: the result folder, or a scratch folder that becomes the zip.
    let base = base_dir(&parent);
    let label = size::words(fit.bytes);
    let dest_root = if fit.mode == FitMode::Folder {
        let d = convert::unique_output(&base, &format!("{stem} ({label})"), "");
        plan.new_roots.push(d.clone());
        d
    } else {
        let d = base.join(format!(".convertino-fit-{}-{}", std::process::id(), engine::new_id()));
        plan.scratch.push(d.clone());
        d
    };
    let into_dir = |i: &Item| -> PathBuf {
        let r = if scan.roots.len() > 1 { PathBuf::from(folder_name(&scan.roots[i.root])).join(&i.rel) } else { i.rel.clone() };
        r.parent().map(|p| dest_root.join(p)).unwrap_or_else(|| dest_root.clone())
    };
    let room = fit.bytes.saturating_sub(data.other + slack(fit.bytes, data.files));
    let budgets = if pv.fits { vec![u64::MAX; data.kinds.len()] } else { kind_budgets(&data, room) };
    let mut shrink: Vec<Step> = Vec::new();
    let mut copies: Vec<Step> = Vec::new();
    let mut targets: HashMap<PathBuf, Option<u64>> = HashMap::new();
    for ((k, infos), b) in data.kinds.iter().zip(budgets) {
        let s = if b == u64::MAX { vec![None; infos.len()] } else { size::shares(*k, infos, b) };
        for (i, t) in infos.iter().zip(s) {
            targets.insert(i.path.clone(), t.map(|x| (k, x)).map(|(_, x)| x));
        }
    }
    for i in files {
        let dir = into_dir(i);
        let (_, st, ext) = convert::split(&i.path);
        match (kind_of(i), targets.get(&i.path).copied().flatten()) {
            (Some(k), Some(bytes)) => {
                let out_ext = match k {
                    Kind::Video => "mp4".to_string(),
                    Kind::Pdf => "pdf".to_string(),
                    _ => {
                        // The size plan picks the format (a PNG photo becomes a JPG, say).
                        match size::plan(k, std::slice::from_ref(&i.path), Some(size::Ask { bytes, together: false, trim: None }), q) {
                            Ok(p) if !p.steps.is_empty() => p.steps[0].ext.clone(),
                            _ => ext.clone(),
                        }
                    }
                };
                shrink.push(Step { inputs: vec![i.path.clone()], dir, stem: st, ext: out_ext, folder: false, op: Op::ToSize { kind: k, bytes: Some(bytes), trim: None, quality: Box::new(q.clone()) } });
            }
            _ => {
                if kind_of(i).is_some() {
                    rows.push(Row { name: row_name(scan, &i.path), status: "Already small enough · copied as it is".into(), kind: RowKind::Kept });
                }
                copies.push(Step { inputs: vec![i.path.clone()], dir, stem: st, ext, folder: false, op: Op::Folder(Box::new(FolderOp::Copy)) });
            }
        }
    }
    shrink.sort_by_key(|s| matches!(s.op, Op::ToSize { kind: Kind::Video, .. }));
    let (videos, others): (Vec<Step>, Vec<Step>) = shrink.into_iter().partition(|s| matches!(s.op, Op::ToSize { kind: Kind::Video, .. }));
    for g in [copies, others, videos] {
        if !g.is_empty() {
            plan.groups.push(g);
        }
    }
    if fit.mode == FitMode::Zip {
        let list: Vec<PathBuf> = vec![PathBuf::from(".")];
        let op = Op::Folder(Box::new(FolderOp::Pack { base: dest_root.clone(), list, format: PackFormat::Zip, password: None, parts: None, store: true }));
        plan.groups.push(vec![Step { inputs: vec![dest_root.clone()], dir: base, stem: stem.clone(), ext: "zip".into(), folder: false, op }]);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Steps of folder jobs

#[derive(Debug, Clone, PartialEq)]
pub enum FolderOp {
    /// A copy of the file as it is (Fit to size: files that already fit).
    Copy,
    /// A copy without the GPS location.
    StripGps,
    /// The clips, in order, as one video.
    JoinVideo,
    /// The recordings, in order, as one.
    JoinAudio,
    /// Numbered pictures (`numbers`, one per input, in order) as a video or GIF.
    Sequence { fps: f64, format: SeqFormat, sound: Option<PathBuf>, numbers: Vec<u64> },
    /// CSV / TSV files as one workbook, a sheet each.
    Sheets,
    /// `list` (paths relative to `base`) into one archive; `parts`: pieces of this
    /// many bytes into the output folder; `store`: no compression (media).
    Pack { base: PathBuf, list: Vec<PathBuf>, format: PackFormat, password: Option<String>, parts: Option<u64>, store: bool },
}

pub fn run_op(op: &FolderOp, step: &Step, output: &Path, progress: &mut dyn FnMut(f64)) -> Result<PathBuf, String> {
    let input = &step.inputs[0];
    match op {
        FolderOp::Copy => {
            std::fs::copy(input, output).map_err(|e| format!("Couldn't copy {}: {e}", file_name(input)))?;
            Ok(output.to_path_buf())
        }
        FolderOp::StripGps => {
            std::fs::copy(input, output).map_err(|e| format!("Couldn't copy {}: {e}", file_name(input)))?;
            crate::look::strip_gps(output);
            Ok(output.to_path_buf())
        }
        FolderOp::JoinVideo => join_video(&step.inputs, output, progress),
        FolderOp::JoinAudio => join_audio(&step.inputs, output, progress),
        FolderOp::Sequence { fps, format, sound, numbers } => sequence(&step.inputs, numbers, *fps, *format, sound.as_deref(), output, progress),
        FolderOp::Sheets => {
            let mut sheets = Vec::new();
            for (n, f) in step.inputs.iter().enumerate() {
                let ext = f.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                let from = crate::data::Format::from_ext(if ext == "tab" { "tsv" } else { &ext }).ok_or_else(|| format!("{} isn't a table.", file_name(f)))?;
                let bytes = std::fs::read(f).map_err(|e| format!("Couldn't read {}: {e}", file_name(f)))?;
                let v = crate::data::read(&crate::data::decode(&bytes), from).map_err(|e| format!("{}: {e}", file_name(f)))?;
                let stem = f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                sheets.push((stem, v));
                progress((n + 1) as f64 / step.inputs.len() as f64 * 0.8);
            }
            crate::data::write_sheets(&sheets, output)?;
            Ok(output.to_path_buf())
        }
        FolderOp::Pack { base, list, format, password, parts, store } => pack(base, list, *format, password.as_deref(), *parts, *store, output, progress),
    }
}

/// A list file for FFmpeg's concat reader.
fn concat_list(dir: &Path, lines: &[String]) -> Result<PathBuf, String> {
    let p = dir.join("list.txt");
    std::fs::write(&p, format!("ffconcat version 1.0\n{}\n", lines.join("\n"))).map_err(|e| format!("Couldn't write a list for FFmpeg: {e}"))?;
    Ok(p)
}

fn concat_file(p: &Path) -> String {
    // Single quotes inside a quoted path: close, escaped quote, reopen.
    format!("file '{}'", p.to_string_lossy().replace('\\', "/").replace('\'', "'\\''"))
}

/// Whether the clips can be joined without re-encoding, and why not.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinCheck {
    pub copy: bool,
    pub words: String,
}

fn same_clip(a: &crate::video::Probe, b: &crate::video::Probe) -> bool {
    let fps = |p: &crate::video::Probe| p.fps.map(|f| (f * 100.0).round() as i64);
    a.video == b.video && a.width == b.width && a.height == b.height && fps(a) == fps(b) && a.audio == b.audio
}

/// Probes the clips (four at a time) and says how they'll be joined.
pub fn join_check(files: &[PathBuf]) -> Result<JoinCheck, String> {
    let probes = probe_all(files)?;
    let first = &probes[0];
    let same_ext = files.iter().all(|f| f.extension() == files[0].extension());
    if same_ext && probes.iter().all(|p| same_clip(first, p)) {
        return Ok(JoinCheck { copy: true, words: format!("Every clip matches ({} × {}): joined without re-encoding", first.width, first.height) });
    }
    // Say what differs, by how many.
    let mut sizes: BTreeMap<String, usize> = BTreeMap::new();
    let mut rates: BTreeMap<String, usize> = BTreeMap::new();
    for p in &probes {
        *sizes.entry(format!("{} × {}", p.width, p.height)).or_default() += 1;
        *rates.entry(p.fps.map(|f| format!("{} fps", (f * 100.0).round() / 100.0)).unwrap_or_else(|| "unknown fps".into())).or_default() += 1;
    }
    let target = format!("{} × {} at {}", first.width, first.height, first.fps.map(|f| format!("{} fps", (f * 100.0).round() / 100.0)).unwrap_or_else(|| "its frame rate".into()));
    let describe = |m: &BTreeMap<String, usize>| m.iter().map(|(k, n)| format!("{n} at {k}")).collect::<Vec<_>>().join(", ");
    let why = if rates.len() > 1 {
        format!("Mixed frame rates: {}", describe(&rates))
    } else if sizes.len() > 1 {
        format!("Mixed sizes: {}", describe(&sizes))
    } else {
        "The clips are encoded differently".into()
    };
    Ok(JoinCheck { copy: false, words: format!("{why} → re-encoded to match the first ({target})") })
}

fn probe_all(files: &[PathBuf]) -> Result<Vec<crate::video::Probe>, String> {
    if files.is_empty() {
        return Err("There are no clips.".into());
    }
    let results: Mutex<Vec<Option<Result<crate::video::Probe, String>>>> = Mutex::new(vec![None; files.len()]);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let job = crate::procs::current_job();
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| {
                crate::procs::set_current_job(job);
                loop {
                    let n = next.fetch_add(1, AtomicOrdering::SeqCst);
                    let Some(f) = files.get(n) else { break };
                    let r = crate::video::probe(f).map_err(|e| format!("{}: {e}", file_name(f)));
                    if let Ok(mut v) = results.lock() {
                        v[n] = Some(r);
                    }
                }
            });
        }
    });
    results.into_inner().unwrap_or_default().into_iter().map(|r| r.unwrap_or_else(|| Err("Not checked".into()))).collect()
}

/// FFmpeg reading a concat list; `total`: seconds, for progress (the list may not say).
fn ffmpeg_concat(list: &Path, args: &[String], output: &Path, total: Option<f64>, progress: &mut dyn FnMut(f64)) -> Result<(), String> {
    let before = convert::s(&["-f", "concat", "-safe", "0"]);
    convert::run_ffmpeg_pre(&before, list, args, output, total, progress)
}

/// MP4 and MOV start playing before they've fully downloaded.
fn faststart(output: &Path) -> Vec<String> {
    let ext = output.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if matches!(ext.as_str(), "mp4" | "mov" | "m4v") { convert::s(&["-movflags", "+faststart"]) } else { Vec::new() }
}

fn join_video(files: &[PathBuf], output: &Path, progress: &mut dyn FnMut(f64)) -> Result<PathBuf, String> {
    let probes = probe_all(files)?;
    let total: f64 = probes.iter().map(|p| p.duration.unwrap_or(0.0)).sum();
    let tmp = convert::TempDir::new("join")?;
    let same_ext = files.iter().all(|f| f.extension() == files[0].extension());
    if same_ext && probes.iter().all(|p| same_clip(&probes[0], p)) {
        let lines: Vec<String> = files.iter().map(|f| concat_file(f)).collect();
        let list = concat_list(&tmp.0, &lines)?;
        let mut args = convert::s(&["-map", "0:v:0", "-map", "0:a?", "-c", "copy"]);
        args.extend(faststart(output));
        match ffmpeg_concat(&list, &args, output, Some(total), progress) {
            Ok(()) => return Ok(output.to_path_buf()),
            Err(e) if e == crate::procs::CANCELLED => return Err(e),
            // The container can't hold these streams as they are: re-encode below.
            Err(e) => {
                log::info!("join: copying didn't work ({e}); re-encoding");
                let _ = std::fs::remove_file(output);
            }
        }
    }
    // Each clip to the first one's size, frame rate and sound, then joined as is.
    let first = &probes[0];
    let (w, h) = ((first.width.max(2) / 2) * 2, (first.height.max(2) / 2) * 2);
    let fps = first.fps.filter(|f| *f > 1.0 && *f <= 240.0).unwrap_or(30.0);
    let enc = crate::video::encode_args(crate::video::Codec::H264, false, w, h)?;
    let mut parts = Vec::new();
    let count = files.len() as f64;
    for (n, (f, p)) in files.iter().zip(&probes).enumerate() {
        if crate::procs::cancelled_here() {
            return Err(crate::procs::CANCELLED.into());
        }
        let part = tmp.0.join(format!("part{n:05}.mp4"));
        let vf = format!("scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1,fps={fps}");
        let mut args: Vec<String> = Vec::new();
        if p.audio.is_none() {
            // A clip without sound gets silence, so the sound stays in step after it.
            args.extend(convert::s(&["-f", "lavfi", "-i", "anullsrc=r=48000:cl=stereo", "-map", "0:v:0", "-map", "1:a:0", "-shortest"]));
        } else {
            args.extend(convert::s(&["-map", "0:v:0", "-map", "0:a:0"]));
        }
        args.extend(convert::s(&["-vf", &vf]));
        args.extend(enc.iter().cloned());
        args.extend(convert::s(&["-c:a", "aac", "-b:a", "192k", "-ar", "48000", "-ac", "2"]));
        convert::run_ffmpeg(f, &args, &part, &mut |x| progress((n as f64 + x) / count * 0.95))?;
        parts.push(part);
    }
    let lines: Vec<String> = parts.iter().map(|f| concat_file(f)).collect();
    let list = concat_list(&tmp.0, &lines)?;
    let mut args = convert::s(&["-map", "0:v:0", "-map", "0:a?", "-c", "copy"]);
    args.extend(faststart(output));
    ffmpeg_concat(&list, &args, output, Some(total), &mut |x| progress(0.95 + x * 0.05))?;
    Ok(output.to_path_buf())
}

fn join_audio(files: &[PathBuf], output: &Path, progress: &mut dyn FnMut(f64)) -> Result<PathBuf, String> {
    let probes = probe_all(files)?;
    let total: f64 = probes.iter().map(|p| p.duration.unwrap_or(0.0)).sum();
    let tmp = convert::TempDir::new("join")?;
    let lines: Vec<String> = files.iter().map(|f| concat_file(f)).collect();
    let list = concat_list(&tmp.0, &lines)?;
    let ext = output.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let same = files.iter().all(|f| f.extension().map(|e| e.to_string_lossy().to_lowercase()) == Some(ext.clone())) && probes.iter().all(|p| p.audio == probes[0].audio);
    // The same format throughout: joined as it is (nothing lost; 24-bit stays 24-bit).
    if same {
        if ffmpeg_concat(&list, &convert::s(&["-vn", "-c:a", "copy"]), output, Some(total), progress).is_ok() {
            return Ok(output.to_path_buf());
        }
        if crate::procs::cancelled_here() {
            return Err(crate::procs::CANCELLED.into());
        }
        let _ = std::fs::remove_file(output);
    }
    let mut args = convert::s(&["-vn"]);
    args.extend(convert::audio_codec_for(&ext, 320, None));
    ffmpeg_concat(&list, &args, output, Some(total), progress)?;
    Ok(output.to_path_buf())
}

#[allow(clippy::too_many_arguments)]
fn sequence(frames: &[PathBuf], numbers: &[u64], fps: f64, format: SeqFormat, sound: Option<&Path>, output: &Path, progress: &mut dyn FnMut(f64)) -> Result<PathBuf, String> {
    if frames.is_empty() {
        return Err("There are no frames.".into());
    }
    let tmp = convert::TempDir::new("seq")?;
    // Each frame shows for 1/fps; a missing number makes the frame before it last longer.
    let mut lines = Vec::new();
    for (n, f) in frames.iter().enumerate() {
        let gap = numbers.get(n + 1).zip(numbers.get(n)).map(|(b, a)| b.saturating_sub(*a).max(1)).unwrap_or(1);
        lines.push(concat_file(f));
        lines.push(format!("duration {:.6}", gap as f64 / fps));
    }
    // The concat reader ignores the last duration unless the last file is repeated.
    lines.push(concat_file(&frames[frames.len() - 1]));
    let list = concat_list(&tmp.0, &lines)?;
    let (w, h) = image::image_dimensions(&frames[0]).unwrap_or((1920, 1080));
    let even = "scale=trunc(iw/2)*2:trunc(ih/2)*2";
    let mut after: Vec<String> = Vec::new();
    if let Some(snd) = sound {
        after.extend(convert::s(&["-i"]));
        after.push(snd.to_string_lossy().into_owned());
        after.extend(convert::s(&["-map", "0:v:0", "-map", "1:a:0", "-shortest"]));
    }
    let fps_s = format!("{fps}");
    match format {
        SeqFormat::Mp4 | SeqFormat::H265 => {
            after.extend(convert::s(&["-vf", &format!("fps={fps_s},{even}"), "-fps_mode", "cfr"]));
            let codec = if format == SeqFormat::H265 { crate::video::Codec::Hevc } else { crate::video::Codec::H264 };
            after.extend(crate::video::encode_args(codec, true, w, h)?);
            if sound.is_some() {
                after.extend(convert::s(&["-c:a", "aac", "-b:a", "192k"]));
            }
            after.extend(convert::s(&["-movflags", "+faststart"]));
        }
        SeqFormat::Prores => {
            after.extend(convert::s(&["-vf", &format!("fps={fps_s},{even}"), "-fps_mode", "cfr", "-c:v", "prores_ks", "-profile:v", "3", "-pix_fmt", "yuv422p10le"]));
            if sound.is_some() {
                after.extend(convert::s(&["-c:a", "pcm_s16le"]));
            }
        }
        SeqFormat::Webm => {
            after.extend(convert::s(&["-vf", &format!("fps={fps_s},{even}"), "-fps_mode", "cfr", "-c:v", "libvpx-vp9", "-crf", "30", "-b:v", "0", "-row-mt", "1", "-pix_fmt", "yuva420p", "-auto-alt-ref", "0"]));
            if sound.is_some() {
                after.extend(convert::s(&["-c:a", "libopus", "-b:a", "128k"]));
            }
        }
        SeqFormat::Gif => {
            let g = fps.min(15.0);
            after.extend(convert::s(&["-vf", &format!("fps={g},scale=480:-1:flags=lanczos,split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=4"), "-loop", "0"]));
        }
    }
    let secs = numbers.last().zip(numbers.first()).map(|(b, a)| (b - a + 1) as f64 / fps).unwrap_or(frames.len() as f64 / fps);
    ffmpeg_concat(&list, &after, output, Some(secs), progress)?;
    Ok(output.to_path_buf())
}

#[allow(clippy::too_many_arguments)]
fn pack(base: &Path, list: &[PathBuf], format: PackFormat, password: Option<&str>, parts: Option<u64>, store: bool, output: &Path, progress: &mut dyn FnMut(f64)) -> Result<PathBuf, String> {
    let tmp = convert::TempDir::new("pack")?;
    let listfile = tmp.0.join("files.txt");
    let body: String = list.iter().map(|p| format!("{}\n", p.to_string_lossy())).collect();
    std::fs::write(&listfile, body).map_err(|e| format!("Couldn't write the list of files: {e}"))?;
    let at = format!("@{}", listfile.display());
    let level = if store { "-mx=0" } else { "-mx=5" };
    // Parts go into the output folder, named like the folder.
    let target = match parts {
        Some(_) => {
            let name = output.file_name().map(|n| n.to_string_lossy().replace(" (zip parts)", "")).unwrap_or_else(|| "parts".into());
            output.join(format!("{name}.{}", format.ext()))
        }
        None => output.to_path_buf(),
    };
    let os = |s: &str| std::ffi::OsString::from(s);
    match format {
        PackFormat::Zip | PackFormat::SevenZ => {
            let mut args: Vec<std::ffi::OsString> = vec![os("a"), os(if format == PackFormat::Zip { "-tzip" } else { "-t7z" }), os(level), os("-scsUTF-8"), os("-spf2")];
            if let Some(pw) = password {
                args.push(os(&format!("-p{pw}")));
                args.push(os(if format == PackFormat::Zip { "-mem=AES256" } else { "-mhe=on" }));
            }
            if let Some(b) = parts {
                args.push(os(&format!("-v{b}b")));
            }
            args.push(target.as_os_str().to_os_string());
            args.push(os(&at));
            let refs: Vec<&std::ffi::OsStr> = args.iter().map(|a| a.as_os_str()).collect();
            crate::archive::seven(&refs, Some(base), progress)?;
        }
        PackFormat::TarGz => {
            let tar = tmp.0.join("packed.tar");
            let args: Vec<std::ffi::OsString> = vec![os("a"), os("-ttar"), os("-scsUTF-8"), os("-spf2"), tar.as_os_str().to_os_string(), os(&at)];
            let refs: Vec<&std::ffi::OsStr> = args.iter().map(|a| a.as_os_str()).collect();
            crate::archive::seven(&refs, Some(base), &mut |p| progress(p * 0.5))?;
            let args: Vec<std::ffi::OsString> = vec![os("a"), os("-tgzip"), os("-mx=9"), target.as_os_str().to_os_string(), tar.as_os_str().to_os_string()];
            let refs: Vec<&std::ffi::OsStr> = args.iter().map(|a| a.as_os_str()).collect();
            crate::archive::seven(&refs, None, &mut |p| progress(0.5 + p * 0.5))?;
        }
    }
    if parts.is_none() && !target.exists() {
        return Err("7-Zip didn't write the archive.".into());
    }
    Ok(output.to_path_buf())
}

// ---------------------------------------------------------------------------
// Running a plan

/// What a folder job did.
#[derive(Debug, Default)]
pub struct Outcome {
    /// What to remove on Undo: whole new folders, and files made elsewhere.
    pub undo: Vec<PathBuf>,
    /// Everything made (files and folders), in order.
    pub made: Vec<PathBuf>,
    /// (originals, result) for everything made, in order.
    pub made_from: Vec<(Vec<PathBuf>, PathBuf)>,
    /// (original, why) for each file that failed.
    pub failures: Vec<(PathBuf, String)>,
    /// (original, result) for Compare.
    pub pairs: Vec<(PathBuf, PathBuf)>,
    /// Replace: (new file in the original's place, the original's path, where the original is kept).
    pub replaced: Vec<(PathBuf, PathBuf, PathBuf)>,
    /// Replace: the Holding Area batch folder the originals were moved to (the first one, when several drives).
    pub holding: Option<PathBuf>,
    pub rows: Vec<Row>,
    pub done: usize,
    pub kept: usize,
    pub failed: usize,
    pub not_started: usize,
    /// The files that weren't started ("Carry on" does them).
    pub left: Vec<PathBuf>,
    pub before: u64,
    pub after: u64,
    pub stopped: bool,
    pub cancelled: bool,
    pub notes: Vec<String>,
    /// When the job started (Undo of Replace finds the originals in the bin by time).
    pub started: Option<SystemTime>,
}

impl Outcome {
    pub fn left_alone(&self) -> usize {
        self.rows.iter().filter(|r| r.kind == RowKind::Alone).count()
    }
    pub fn done_before(&self) -> usize {
        self.rows.iter().filter(|r| r.kind == RowKind::DoneBefore).count()
    }
}

/// Live progress of a folder job, for the ring and the corner card.
#[derive(Debug, Clone)]
pub struct Live {
    pub fraction: f64,
    /// "File 13 of 44 · clip_0413.mp4 · 58%"
    pub detail: String,
    /// "Saved so far: 2.9 GB on 12 files"
    pub extra: Option<String>,
}

/// Callbacks for a running folder job: progress, and a converter downloading.
pub type OnLive = Arc<dyn Fn(Live) + Send + Sync>;
pub type OnDownload = Arc<dyn Fn(f64, String) + Send + Sync>;

#[derive(Default)]
struct Tally {
    /// Step index -> file name, and the step in progress.
    names: HashMap<usize, String>,
    current: usize,
    /// Bytes before and after, and how many files, for "Saved so far".
    before: u64,
    after: u64,
    files: usize,
}

/// Runs a plan: its groups one after another. "Stop after this file" and
/// Cancel are checked between files (procs::stop_after_current / cancel).
pub fn run(id: u64, plan: &Plan, on: OnLive, download: OnDownload) -> Outcome {
    let mut out = Outcome { rows: plan.rows.clone(), started: Some(SystemTime::now()), notes: plan.notes.clone(), ..Outcome::default() };
    let total_steps = plan.step_count();
    let single = plan.is_single();
    let scan_like = Scan { roots: plan.roots.clone(), ..Scan::default() };
    let saved = Arc::new(Mutex::new(Tally::default()));
    let mut offset = 0usize;
    let mut ran: HashSet<usize> = HashSet::new();
    for (g, group) in plan.groups.iter().enumerate() {
        if crate::procs::is_stopping(id) {
            break;
        }
        ran.insert(g);
        // Replace: a video original must be sound before a copy may take its place.
        let checked: Vec<Step>;
        let group: &Vec<Step> = if plan.replace {
            let mut keep = Vec::with_capacity(group.len());
            for st in group {
                if !st.folder && st.inputs.len() == 1 && !crate::procs::is_stopping(id) {
                    on(Live { fraction: (offset as f64 / total_steps.max(1) as f64).clamp(0.0, 1.0), detail: format!("Checking {} before replacing it", file_name(&st.inputs[0])), extra: None });
                    if let Err(e) = crate::verify::source_ok(&st.inputs[0]) {
                        out.kept += 1;
                        out.rows.push(Row { name: row_name(&scan_like, &st.inputs[0]), status: e, kind: RowKind::Kept });
                        continue;
                    }
                }
                keep.push(st.clone());
            }
            checked = keep;
            &checked
        } else {
            group
        };
        let group_len = group.len();
        // Files left alone by the check count as finished in "File N of M".
        let off = offset + (plan.groups[g].len() - group_len);
        let sink: Sink = {
            let (tally, on, dl) = (saved.clone(), on.clone(), download.clone());
            Arc::new(move |e: Event| match e {
                Event::StepStarted { index, input } => {
                    if let Ok(mut t) = tally.lock() {
                        t.names.insert(off + index, file_name(&input));
                        t.current = off + index;
                    }
                }
                Event::Progress { fraction, .. } => {
                    let Ok(t) = tally.lock() else { return };
                    let name = t.names.get(&t.current).cloned().unwrap_or_default();
                    let overall = ((off as f64 + fraction * group_len as f64) / total_steps.max(1) as f64).clamp(0.0, 1.0);
                    let detail = if single || total_steps == 1 {
                        format!("{}%", (fraction * 100.0).round() as u32)
                    } else {
                        let finished = (fraction * group_len as f64).floor() as usize;
                        format!("File {} of {total_steps} · {name}", (off + finished + 1).min(total_steps))
                    };
                    let extra = (t.files > 0 && t.before > t.after).then(|| format!("Saved so far: {} on {}", human_size(t.before - t.after), files_word(t.files)));
                    drop(t);
                    on(Live { fraction: overall, detail, extra });
                }
                Event::Download { fraction, detail } => dl(fraction, detail),
                Event::StepDone { inputs, result: Ok(made), .. } => {
                    if let Ok(mut t) = tally.lock() {
                        if inputs.len() == 1 && made.is_file() && inputs[0].is_file() {
                            t.before += inputs[0].metadata().map(|m| m.len()).unwrap_or(0);
                            t.after += made.metadata().map(|m| m.len()).unwrap_or(0);
                            t.files += 1;
                        }
                    }
                }
                Event::StepDone { .. } => {}
            })
        };
        let o = engine::run_steps_exact(id, group, &sink);
        offset += plan.groups[g].len();
        out.cancelled |= o.cancelled;
        // What happened to each file of this group.
        let mut seen: HashSet<PathBuf> = HashSet::new();
        for (inputs, made) in &o.made_from {
            seen.insert(inputs[0].clone());
            // Fit to one zip: the zip made from the scratch copy.
            if inputs[0].is_dir() {
                out.made.push(made.clone());
                continue;
            }
            if plan.replace && plan.scratch.iter().any(|s| made.starts_with(s)) {
                let held = plan.holding.as_ref().map(|batch| crate::holding::held_path(&inputs[0], batch));
                let bytes = inputs[0].metadata().map(|m| m.len()).unwrap_or(0);
                match held.ok_or_else(|| "No place to keep the original".to_string()).and_then(|held| replace_original(&inputs[0], made, &held).map(|f| (f, held))) {
                    Ok((final_path, held)) => {
                        if let Err(e) = crate::holding::record(&held, &inputs[0], &final_path, bytes) {
                            out.notes.push(format!("{}: {e}. The original is at {}", file_name(&inputs[0]), held.display()));
                        }
                        let batch_dir = crate::holding::batch_dir_of(&held);
                        if out.holding.is_none() {
                            out.holding = batch_dir;
                        } else if batch_dir.is_some() && batch_dir != out.holding && !out.notes.iter().any(|n| n.starts_with("Originals from another drive")) {
                            out.notes.push("Originals from another drive are in that drive's Holding Area.".into());
                        }
                        let (b, a) = (bytes, engine::disk_size(&final_path));
                        out.before += b;
                        out.after += a;
                        out.done += 1;
                        out.rows.push(Row { name: row_name(&scan_like, &inputs[0]), status: format!("Replaced · {} → {}", human_size(b), human_size(a)), kind: RowKind::Done });
                        out.replaced.push((final_path.clone(), inputs[0].clone(), held));
                        out.made_from.push((inputs.clone(), final_path.clone()));
                        out.made.push(final_path);
                    }
                    Err(e) => {
                        out.failed += 1;
                        out.failures.push((inputs[0].clone(), e.clone()));
                        out.rows.push(Row { name: row_name(&scan_like, &inputs[0]), status: e, kind: RowKind::Failed });
                        let _ = std::fs::remove_file(made);
                    }
                }
                continue;
            }
            out.made.push(made.clone());
            out.made_from.push((inputs.clone(), made.clone()));
            let name = row_name(&scan_like, &inputs[0]);
            let copy = group.iter().any(|st| st.inputs[0] == inputs[0] && matches!(&st.op, Op::Folder(f) if **f == FolderOp::Copy));
            if copy {
                if !out.rows.iter().any(|r| r.name == name) {
                    out.rows.push(Row { name, status: "Copied as it is".into(), kind: RowKind::Kept });
                }
                continue;
            }
            if single {
                for f in inputs {
                    out.rows.push(Row { name: row_name(&scan_like, f), status: format!("In {}", file_name(made)), kind: RowKind::Done });
                }
                out.done += inputs.len();
                out.before += inputs.iter().map(|f| f.metadata().map(|m| m.len()).unwrap_or(0)).sum::<u64>();
                out.after += engine::disk_size(made);
            } else {
                let (b, a) = sizes(&inputs[0], made);
                out.before += b;
                out.after += a;
                out.done += 1;
                let status = if made.is_dir() { format!("→ {}/", file_name(made)) } else { format!("{} → {}", human_size(b), human_size(a)) };
                out.rows.push(Row { name, status, kind: RowKind::Done });
                if made.is_file() {
                    out.pairs.push((inputs[0].clone(), made.clone()));
                }
            }
        }
        for (input, e) in &o.failed {
            seen.insert(input.clone());
            let n = if single { group.first().map(|s| s.inputs.len()).unwrap_or(1) } else { 1 };
            if e.contains("is already compact") {
                out.kept += 1;
                out.rows.push(Row { name: row_name(&scan_like, input), status: "Already as small as it looks · kept".into(), kind: RowKind::Kept });
            } else {
                out.failed += n;
                out.failures.push((input.clone(), e.clone()));
                out.rows.push(Row { name: row_name(&scan_like, input), status: e.clone(), kind: RowKind::Failed });
            }
        }
        out.notes.extend(o.notes);
        for st in group.iter().filter(|st| !seen.contains(&st.inputs[0]) && !st.inputs[0].is_dir()) {
            for f in st.inputs.iter().take(if single { usize::MAX } else { 1 }) {
                out.not_started += 1;
                out.left.push(f.clone());
                out.rows.push(Row { name: row_name(&scan_like, f), status: "Not started (stopped)".into(), kind: RowKind::NotStarted });
            }
        }
    }
    // Groups that never started.
    for (g, group) in plan.groups.iter().enumerate() {
        if ran.contains(&g) {
            continue;
        }
        for st in group.iter().filter(|st| !st.inputs[0].is_dir()) {
            for f in st.inputs.iter().take(if single { usize::MAX } else { 1 }) {
                out.not_started += 1;
                out.left.push(f.clone());
                out.rows.push(Row { name: row_name(&scan_like, f), status: "Not started (stopped)".into(), kind: RowKind::NotStarted });
            }
        }
    }
    out.stopped = !out.cancelled && crate::procs::is_stopping(id) && out.not_started > 0;
    // Scratch folders go (Replace's are empty by now unless something failed).
    for s in &plan.scratch {
        if plan.replace {
            let _ = std::fs::remove_dir(s);
        } else {
            let _ = std::fs::remove_dir_all(s);
        }
    }
    if plan.action == Action::Fit {
        out.made.retain(|m| !plan.scratch.iter().any(|s| m.starts_with(s)));
        out.made_from.retain(|(_, m)| !plan.scratch.iter().any(|s| m.starts_with(s)));
        out.pairs.clear();
        out.before = plan.groups.iter().flatten().filter(|s| s.inputs[0].is_file()).map(|s| s.inputs[0].metadata().map(|m| m.len()).unwrap_or(0)).sum();
        out.after = match plan.new_roots.first() {
            Some(r) if r.exists() => engine::disk_size(r),
            _ => out.made.iter().map(|m| engine::disk_size(m)).sum(),
        };
    }
    // Undo: new folders whole; everything else file by file (replaced ones have their own undo).
    out.undo = plan.new_roots.iter().filter(|r| r.exists()).cloned().collect();
    for m in &out.made {
        if !plan.new_roots.iter().any(|r| m.starts_with(r)) && !out.replaced.iter().any(|(p, _, _)| p == m) {
            out.undo.push(m.clone());
        }
    }
    out
}

fn sizes(original: &Path, made: &Path) -> (u64, u64) {
    (original.metadata().map(|m| m.len()).unwrap_or(0), engine::disk_size(made))
}

/// The new copy checks out: it looks, sounds and plays like the original (`verify::copy_ok`).
fn checks_out(original: &Path, made: &Path) -> Result<(), String> {
    crate::verify::copy_ok(original, made)
}

/// Moves a file (renamed on the same drive; copied, checked and removed across drives).
fn move_checked(from: &Path, to: &Path) -> Result<(), String> {
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
    }
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    let len = from.metadata().map(|m| m.len()).map_err(|e| e.to_string())?;
    std::fs::copy(from, to).map_err(|e| format!("Couldn't copy {}: {e}", from.display()))?;
    if to.metadata().map(|m| m.len()).unwrap_or(0) != len {
        let _ = std::fs::remove_file(to);
        return Err(format!("The copy of {} came out the wrong size", from.display()));
    }
    std::fs::remove_file(from).map_err(|e| format!("Couldn't remove {} after copying it: {e}", from.display()))
}

/// Puts `made` in `original`'s place (same name, the new extension) and moves
/// the original to `held`. Returns where the new file is.
fn replace_original(original: &Path, made: &Path, held: &Path) -> Result<PathBuf, String> {
    checks_out(original, made).map_err(|e| format!("{e}; the original was kept"))?;
    let (dir, stem, _) = convert::split(original);
    let ext = made.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    let held = if held.exists() { convert::unique_output(held.parent().unwrap_or(Path::new(".")), &held.file_stem().unwrap_or_default().to_string_lossy(), &held.extension().unwrap_or_default().to_string_lossy()) } else { held.to_path_buf() };
    move_checked(original, &held).map_err(|e| format!("{e}; the original was kept"))?;
    let wanted = dir.join(format!("{stem}.{ext}"));
    let dest = if wanted.exists() { convert::unique_output(&dir, &stem, &ext) } else { wanted };
    if let Err(e) = move_checked(made, &dest) {
        // Put the original back where it was.
        let back = move_checked(&held, original);
        return Err(match back {
            Ok(()) => format!("{e}; the original was kept"),
            Err(_) => format!("{e}. The original is in {}", held.display()),
        });
    }
    Ok(dest)
}

/// Undo of a Replace: the new files go to the Recycle Bin / Trash and the
/// originals move back from the Holding Area. Returns how many came back.
pub fn undo_replace(replaced: &[(PathBuf, PathBuf, PathBuf)]) -> Result<usize, String> {
    let mut back = 0;
    let mut problems = Vec::new();
    for (new, original, held) in replaced {
        if !held.exists() {
            problems.push(format!("{} isn't in the Holding Area any more", file_name(original)));
            continue;
        }
        if new.exists() {
            if let Err(e) = trash::delete(new) {
                problems.push(format!("{}: {e}", file_name(new)));
                continue;
            }
        }
        if original.exists() {
            problems.push(format!("{} already exists; its original stays in the Holding Area", file_name(original)));
            continue;
        }
        match move_checked(held, original) {
            Ok(()) => {
                back += 1;
                crate::holding::forget(held);
            }
            Err(e) => problems.push(e),
        }
    }
    if problems.is_empty() { Ok(back) } else { Err(format!("{back} put back. {}", problems.join("; "))) }
}

/// The done card's summary: "42 compressed · 1 already small, kept · 1 couldn't be read · 2 left alone".
pub fn summary(plan: &Plan, o: &Outcome) -> String {
    let mut parts = Vec::new();
    if o.done > 0 {
        parts.push(if plan.is_single() { format!("{} {}", files_word(o.done), plan.verb) } else { format!("{} {}", o.done, plan.verb) });
    }
    let kept = o.rows.iter().filter(|r| r.kind == RowKind::Kept).count();
    if kept > 0 {
        parts.push(if plan.action == Action::Fit { format!("{kept} copied as they are") } else { format!("{kept} already small, kept") });
    }
    if o.done_before() > 0 {
        parts.push(format!("{} done in an earlier run", o.done_before()));
    }
    if o.failed > 0 {
        parts.push(format!("{} failed", o.failed));
    }
    if o.not_started > 0 {
        parts.push(format!("{} not started", o.not_started));
    }
    if o.left_alone() > 0 {
        parts.push(format!("{} left alone", o.left_alone()));
    }
    parts.join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("convertino-folder-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn touch(d: &Path, rel: &str, bytes: usize) -> PathBuf {
        let p = d.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, vec![b'x'; bytes]).unwrap();
        p
    }

    fn no_cancel() -> AtomicBool {
        AtomicBool::new(false)
    }

    #[test]
    fn natural_order() {
        let mut v = vec!["scan 10", "scan 2", "Scan 1", "scan 02b", "a"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["a", "Scan 1", "scan 2", "scan 02b", "scan 10"]);
        assert_eq!(natural_cmp("clip_0009", "clip_0010"), Ordering::Less);
    }

    #[test]
    fn result_names() {
        assert_eq!(result_base("clip (compressed)"), Some("clip"));
        assert_eq!(result_base("trip (10 MB, first 7 s)"), Some("trip"));
        assert_eq!(result_base("Valorant clips (mp4)"), Some("Valorant clips"));
        assert_eq!(result_base("Holiday (Goa)"), None);
        assert_eq!(result_base("plain"), None);
    }

    #[test]
    fn scan_counts_everything() {
        let d = dir("scan");
        touch(&d, "clip_2.mp4", 10);
        touch(&d, "clip_10.mp4", 10);
        touch(&d, "clip_2 (compressed).mp4", 5);
        touch(&d, ".hidden.mp4", 5);
        touch(&d, "notes.qqq", 3);
        touch(&d, "old/clip_1.mov", 7);
        touch(&d, ".git/config", 1);
        touch(&d, "movie.mp4.crdownload", 1);
        let s = scan(&[d.clone()], &no_cancel(), &|_| {}).unwrap();
        let names: Vec<String> = s.files(true).iter().map(|i| i.rel.to_string_lossy().replace('\\', "/")).collect();
        assert_eq!(names, ["clip_2.mp4", "clip_10.mp4", "notes.qqq", "old/clip_1.mov"]);
        // The earlier result is read (Zip takes it) but not offered to anything else.
        assert_eq!(s.everything(true).len(), 5);
        assert_eq!(s.earlier(), 1);
        assert_eq!(s.subfolders, 1);
        let alone: Vec<Alone> = s.alone.iter().map(|(_, w)| *w).collect();
        assert!(alone.contains(&Alone::Hidden) && alone.contains(&Alone::Partial));
        assert_eq!(s.skipped_dirs.len(), 1);
        assert_eq!(s.files(false).len(), 3);
        let files = s.files(true);
        assert_eq!(files[3].family.as_deref(), Some("video"));
        assert_eq!(files[2].family, None);
    }

    #[test]
    fn sequences_are_found_with_gaps() {
        let d = dir("seq");
        for n in 1..=12 {
            if n != 5 {
                touch(&d, &format!("render_{n:04}.png"), 4);
            }
        }
        touch(&d, "cover.png", 4);
        let s = scan(&[d.clone()], &no_cancel(), &|_| {}).unwrap();
        let seq = find_sequence(&s.files(true)).unwrap();
        assert_eq!(seq.frames.len(), 11);
        assert_eq!(seq.gaps, vec![5]);
        assert_eq!(seq.prefix, "render_");
        // Too few numbered pictures: not a sequence.
        let d2 = dir("noseq");
        for n in 1..=4 {
            touch(&d2, &format!("IMG_{n}.jpg"), 4);
        }
        let s2 = scan(&[d2], &no_cancel(), &|_| {}).unwrap();
        assert!(find_sequence(&s2.files(true)).is_none());
    }

    #[test]
    fn wheel_offers_what_fits_and_counts_honestly() {
        let d = dir("wheel");
        for n in 1..=3 {
            touch(&d, &format!("clip {n}.mp4"), 100);
        }
        touch(&d, "clip 4.mkv", 100);
        touch(&d, "thumb.jpg", 10);
        touch(&d, "scene.json", 10);
        let s = scan(&[d.clone()], &no_cancel(), &|_| {}).unwrap();
        let prefs = crate::settings::Settings::default();
        let (main, more) = slots(&s, &Defaults::default(), &prefs);
        let all: Vec<&Slot> = main.iter().chain(more.iter()).collect();
        let get = |id: &str| all.iter().find(|x| x.id == id).copied();
        let compress = get("folder.compress").unwrap();
        assert_eq!(compress.count, 5);
        assert_eq!(compress.detail.as_deref(), Some("Leaves 1 file alone"));
        let mp4 = get("folder.to:video.mp4").unwrap();
        assert_eq!(mp4.count, 1, "only the MKV needs MP4");
        assert!(get("folder.join-video").is_some());
        assert!(get("folder.zip").is_some() && get("folder.fit").is_some() && get("folder.watch").is_some());
        assert!(main.len() <= 8);
        assert_eq!(main.last().unwrap().kind, SlotKind::More);
        // Picking: the plan touches exactly what the slot said.
        let q = Quality::default();
        let p = plan(&s, &Action::ConvertAll("video.mp4".into()), &Opts::default(), &q, &prefs).unwrap();
        assert_eq!(p.step_count(), 1);
        assert_eq!(p.rows.iter().filter(|r| r.kind == RowKind::Alone).count(), 5);
        let root = &p.new_roots[0];
        assert_eq!(root.file_name().unwrap().to_string_lossy(), format!("{} (mp4)", d.file_name().unwrap().to_string_lossy()));
        assert_eq!(p.groups[0][0].dir, *root);
        assert_eq!(p.groups[0][0].stem, "clip 4");
    }

    #[test]
    fn done_earlier_is_skipped() {
        let d = dir("redo");
        let src = touch(&d, "a.mkv", 100);
        let s = scan(&[d.clone()], &no_cancel(), &|_| {}).unwrap();
        let prefs = crate::settings::Settings::default();
        let q = Quality::default();
        let opts = Opts { into: Into::Beside, ..Opts::default() };
        let p = plan(&s, &Action::ConvertAll("video.mp4".into()), &opts, &q, &prefs).unwrap();
        assert_eq!(p.step_count(), 1);
        // A result from before, newer than the original.
        std::thread::sleep(std::time::Duration::from_millis(20));
        touch(&d, "a.mp4", 50);
        let _ = src;
        let s = scan(&[d.clone()], &no_cancel(), &|_| {}).unwrap();
        // a.mp4 now counts as a video of its own; the MKV's result is a.mp4 (done before).
        let err = plan(&s, &Action::ConvertAll("video.mp4".into()), &opts, &q, &prefs).unwrap_err();
        assert!(err.starts_with(ALREADY_DONE), "{err}");
        let redo = Opts { skip_done: false, ..opts };
        assert_eq!(plan(&s, &Action::ConvertAll("video.mp4".into()), &redo, &q, &prefs).unwrap().step_count(), 1);
    }

    #[test]
    fn zip_lists_visible_files_with_the_folder() {
        let d = dir("zip");
        touch(&d, "a.txt", 3);
        touch(&d, "sub/b.txt", 3);
        touch(&d, ".secret", 3);
        let s = scan(&[d.clone()], &no_cancel(), &|_| {}).unwrap();
        let p = plan(&s, &Action::Zip, &Opts::default(), &Quality::default(), &crate::settings::Settings::default()).unwrap();
        let st = &p.groups[0][0];
        let Op::Folder(op) = &st.op else { panic!() };
        let FolderOp::Pack { list, .. } = &**op else { panic!() };
        let name = d.file_name().unwrap().to_string_lossy().into_owned();
        let listed: Vec<String> = list.iter().map(|p| p.to_string_lossy().replace('\\', "/")).collect();
        assert_eq!(listed, [format!("{name}/a.txt"), format!("{name}/sub/b.txt")]);
        assert_eq!(st.ext, "zip");
        assert_eq!(st.stem, name);
        assert!(p.rows.iter().any(|r| r.name == ".secret" && r.kind == RowKind::Alone));
    }

    #[test]
    fn parts_preview_counts_pieces() {
        let data = FitData { total: 95 * MB, other: 95 * MB, files: 3, ..FitData::default() };
        let p = fit_preview(&data, 18 * MB, FitMode::Parts);
        assert_eq!(p.short, "6 parts");
        assert!(!p.fits);
        let presets = fit_presets(&data, FitMode::Parts);
        assert_eq!(presets.iter().map(|p| p.bytes).collect::<Vec<_>>(), [18 * MB]);
        // Folder mode: files that can't shrink don't fit below their size.
        let p = fit_preview(&data, 50 * MB, FitMode::Folder);
        assert!(!p.ok);
    }
}

/// Real folder jobs with the converters (skipped when one isn't installed).
#[cfg(test)]
mod real_tests {
    use super::*;
    use crate::tools::{self, Tool};

    fn have(t: &[Tool]) -> bool {
        let ok = t.iter().all(|t| tools::find(*t).is_some());
        if !ok {
            eprintln!("skipped: a converter isn't installed");
        }
        ok
    }

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("convertino-folder-real-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(d.with_file_name(format!("{} (compressed)", d.file_name().unwrap().to_string_lossy())));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn ffmpeg(args: &[&str]) {
        let ff = tools::find(Tool::Ffmpeg).unwrap();
        let st = std::process::Command::new(ff).args(["-hide_banner", "-loglevel", "error", "-y"]).args(args).status().unwrap();
        assert!(st.success(), "ffmpeg {args:?}");
    }

    fn clip(path: &Path, size: &str, rate: u32, secs: u32) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        ffmpeg(&["-f", "lavfi", "-i", &format!("testsrc2=size={size}:rate={rate}:duration={secs}"), "-f", "lavfi", "-i", &format!("sine=frequency=440:duration={secs}"), "-shortest", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", path.to_str().unwrap()]);
    }

    fn picture(path: &Path, color: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        ffmpeg(&["-f", "lavfi", "-i", &format!("color=c={color}:size=320x240,format=rgb24"), "-frames:v", "1", path.to_str().unwrap()]);
    }

    fn run_it(d: &Path, action: Action, opts: Opts) -> (Plan, Outcome) {
        let never = AtomicBool::new(false);
        let s = scan(&[d.to_path_buf()], &never, &|_| {}).unwrap();
        let prefs = crate::settings::Settings::default();
        let p = plan(&s, &action, &opts, &Quality::default(), &prefs).unwrap_or_else(|e| panic!("{action:?}: {e}"));
        let id = engine::new_id();
        let o = run(id, &p, Arc::new(|_| {}), Arc::new(|_, _| {}));
        (p, o)
    }

    fn probe_len(p: &Path) -> f64 {
        crate::video::probe(p).unwrap().duration.unwrap_or(0.0)
    }

    #[test]
    fn join_clips_copy_and_reencode() {
        if !have(&[Tool::Ffmpeg]) {
            return;
        }
        let d = dir("join");
        clip(&d.join("b 10.mp4"), "320x240", 30, 1);
        clip(&d.join("b 2.mp4"), "320x240", 30, 2);
        let (_, o) = run_it(&d, Action::JoinVideo, Opts::default());
        assert_eq!(o.failed, 0, "{:?}", o.rows);
        let out = d.parent().unwrap().join(format!("{}.mp4", d.file_name().unwrap().to_string_lossy()));
        assert_eq!(o.made, vec![out.clone()]);
        assert!((probe_len(&out) - 3.0).abs() < 0.3, "{}", probe_len(&out));
        std::fs::remove_file(&out).unwrap();
        // Different size and frame rate: re-encoded to match the first.
        clip(&d.join("b 3.mp4"), "640x360", 60, 1);
        let paths: Vec<PathBuf> = ["b 2.mp4", "b 3.mp4"].iter().map(|n| d.join(n)).collect();
        let check = join_check(&paths).unwrap();
        assert!(!check.copy && check.words.contains("frame rates"), "{}", check.words);
        let (_, o) = run_it(&d, Action::JoinVideo, Opts::default());
        assert_eq!(o.failed, 0, "{:?}", o.rows);
        let p = crate::video::probe(&out).unwrap();
        assert_eq!((p.width, p.height), (320, 240));
        assert!((p.duration.unwrap() - 4.0).abs() < 0.4, "{:?}", p.duration);
        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn sequence_to_video_and_gif_holds_gaps() {
        if !have(&[Tool::Ffmpeg]) {
            return;
        }
        let d = dir("seq");
        for n in 1..=24u32 {
            if n == 7 {
                continue;
            }
            picture(&d.join(format!("render_{n:04}.png")), if n % 2 == 0 { "red" } else { "blue" });
        }
        let opts = Opts { fps: 12.0, ..Opts::default() };
        let (p, o) = run_it(&d, Action::Sequence, opts.clone());
        assert_eq!(o.failed, 0, "{:?}", o.rows);
        assert!(p.notes[0].contains("1 missing frame (7)"), "{:?}", p.notes);
        let out = &o.made[0];
        assert_eq!(out.extension().unwrap(), "mp4");
        // 24 frame numbers at 12 fps: 2 s, the gap held.
        assert!((probe_len(out) - 2.0).abs() < 0.2, "{}", probe_len(out));
        let _ = std::fs::remove_file(out);
        let stop = Opts { gaps: Gaps::Stop, ..opts.clone() };
        let never = AtomicBool::new(false);
        let s = scan(&[d.clone()], &never, &|_| {}).unwrap();
        let err = plan(&s, &Action::Sequence, &stop, &Quality::default(), &crate::settings::Settings::default()).unwrap_err();
        assert!(err.contains("1 frame is missing (7)"), "{err}");
        let (_, o) = run_it(&d, Action::SequenceGif, opts);
        assert_eq!(o.made[0].extension().unwrap(), "gif");
        let _ = std::fs::remove_file(&o.made[0]);
    }

    #[test]
    fn convert_all_into_a_new_folder_then_skip_done_and_undo() {
        if !have(&[Tool::Ffmpeg]) {
            return;
        }
        let d = dir("toall");
        clip(&d.join("a.mp4"), "320x240", 30, 1);
        clip(&d.join("sub/b.mp4"), "320x240", 30, 1);
        std::fs::rename(d.join("sub/b.mp4"), d.join("sub/b.mov")).unwrap();
        std::fs::write(d.join("notes.txt"), "x").unwrap();
        let (p, o) = run_it(&d, Action::ConvertAll("video.mp4".into()), Opts::default());
        assert_eq!((o.done, o.failed), (1, 0), "{:?}", o.rows);
        let root = d.with_file_name(format!("{} (mp4)", d.file_name().unwrap().to_string_lossy()));
        assert_eq!(p.new_roots, vec![root.clone()]);
        assert!(root.join("sub/b.mp4").is_file(), "{:?}", o.made);
        assert_eq!(o.undo, vec![root.clone()]);
        // Every file accounted for: a.mp4 and notes.txt left alone, b converted.
        let alone: Vec<&str> = o.rows.iter().filter(|r| r.kind == RowKind::Alone).map(|r| r.name.as_str()).collect();
        assert!(alone.contains(&"a.mp4") && alone.contains(&"notes.txt"), "{alone:?}");
        // Again: done before, nothing to do.
        let never = AtomicBool::new(false);
        let s = scan(&[d.clone()], &never, &|_| {}).unwrap();
        let err = plan(&s, &Action::ConvertAll("video.mp4".into()), &Opts::default(), &Quality::default(), &crate::settings::Settings::default()).unwrap_err();
        assert!(err.starts_with(ALREADY_DONE), "{err}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn replace_puts_results_in_place_and_undo_brings_originals_back() {
        if !have(&[Tool::Ffmpeg]) {
            return;
        }
        let base = crate::holding::test_base();
        let d = dir("replace");
        clip(&d.join("c.mp4"), "320x240", 30, 1);
        std::fs::rename(d.join("c.mp4"), d.join("c.mkv")).unwrap();
        let original = d.join("c.mkv");
        let opts = Opts { into: Into::Replace, ..Opts::default() };
        let (p, o) = run_it(&d, Action::ConvertAll("video.mp4".into()), opts);
        assert_eq!(o.done, 1, "{:?}", o.rows);
        assert!(p.new_roots.is_empty());
        assert!(!original.exists());
        assert!(d.join("c.mp4").is_file());
        let held = o.replaced[0].2.clone();
        assert_eq!(o.replaced, vec![(d.join("c.mp4"), original.clone(), held.clone())]);
        assert!(held.starts_with(base.join(crate::holding::DIR)), "{}", held.display());
        assert!(held.is_file(), "the original is kept, not deleted");
        // It is listed in its batch's manifest, with the file now in its place.
        let batch = crate::holding::batch_dir_of(&held).unwrap();
        let m: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(batch.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(m["entries"][0]["from"], original.to_string_lossy().as_ref());
        assert_eq!(m["entries"][0]["replaced"], d.join("c.mp4").to_string_lossy().as_ref());
        assert_eq!(o.holding.as_deref(), Some(batch.as_path()));
        // No scratch folder left behind.
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1);
        assert_eq!(undo_replace(&o.replaced).unwrap(), 1);
        assert!(original.is_file() && !d.join("c.mp4").exists() && !held.exists());
        assert!(!batch.exists(), "the emptied batch goes");
    }

    #[test]
    fn replace_leaves_a_broken_recording_alone() {
        if !have(&[Tool::Ffmpeg]) {
            return;
        }
        let _ = crate::holding::test_base();
        let d = dir("replace-broken");
        // A recording whose slice headers are garbled (what broken NVIDIA DVR clips look like to the parser).
        let src = d.join("clip.mts");
        ffmpeg(&["-f", "lavfi", "-i", "testsrc2=size=320x240:rate=30:duration=4", "-c:v", "libx264", "-g", "15", src.to_str().unwrap()]);
        let mut bytes = std::fs::read(&src).unwrap();
        let (mut i, mut n) = (0, 0);
        while i + 8 < bytes.len() {
            if bytes[i..i + 3] == [0, 0, 1] && matches!(bytes[i + 3] & 0x1f, 1 | 5) {
                if n % 2 == 0 {
                    bytes[i + 4] ^= 0xff;
                    bytes[i + 5] ^= 0xa5;
                }
                n += 1;
            }
            i += 1;
        }
        std::fs::write(&src, &bytes).unwrap();
        let before = std::fs::read(&src).unwrap();
        let opts = Opts { into: Into::Replace, ..Opts::default() };
        let (_, o) = run_it(&d, Action::ConvertAll("video.mp4".into()), opts);
        assert_eq!((o.done, o.kept, o.failed), (0, 1, 0), "{:?}", o.rows);
        assert!(o.rows.iter().any(|r| r.kind == RowKind::Kept && r.status.contains("broken sections")), "{:?}", o.rows);
        assert_eq!(std::fs::read(&src).unwrap(), before, "the original is untouched");
        assert!(o.replaced.is_empty() && o.holding.is_none());
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1, "no copy or scratch folder left behind");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn pdfs_photos_sheets_zip_extract() {
        if !have(&[Tool::Ffmpeg, Tool::Magick, Tool::SevenZip, Tool::Pdfunite]) {
            return;
        }
        let d = dir("docs");
        picture(&d.join("p 2.png"), "red");
        picture(&d.join("p 10.png"), "green");
        let parent = d.parent().unwrap().to_path_buf();
        let name = d.file_name().unwrap().to_string_lossy().into_owned();
        let (_, o) = run_it(&d, Action::PhotosPdf, Opts::default());
        let pdf = parent.join(format!("{name}.pdf"));
        assert_eq!(o.made, vec![pdf.clone()], "{:?}", o.rows);
        assert_eq!(crate::look::pdf_page_count(&pdf), Some(2));
        // Two PDFs in the folder now: merge (name order) them.
        std::fs::copy(&pdf, d.join("one.pdf")).unwrap();
        std::fs::rename(&pdf, d.join("two.pdf")).unwrap();
        let (_, o) = run_it(&d, Action::MergePdf, Opts::default());
        assert_eq!(crate::look::pdf_page_count(&o.made[0]), Some(4), "{:?}", o.rows);
        let _ = std::fs::remove_file(&o.made[0]);
        // CSV files into one workbook.
        std::fs::write(d.join("sales.csv"), "a,b\n1,2\n").unwrap();
        std::fs::write(d.join("costs.csv"), "a,b\n3,4\n").unwrap();
        let (_, o) = run_it(&d, Action::Sheets, Opts::default());
        assert!(o.made[0].metadata().unwrap().len() > 1000, "{:?}", o.rows);
        let _ = std::fs::remove_file(&o.made[0]);
        // Zip with a password: the files are inside under the folder's name, encrypted.
        let opts = Opts { password: Some("s3cret".into()), ..Opts::default() };
        let (_, o) = run_it(&d, Action::Zip, opts);
        let zip = parent.join(format!("{name}.zip"));
        assert_eq!(o.made, vec![zip.clone()], "{:?}", o.rows);
        let seven = tools::find(Tool::SevenZip).unwrap();
        let listing = std::process::Command::new(&seven).args(["l", "-slt", "-p"]).arg(&zip).output().unwrap();
        let text = String::from_utf8_lossy(&listing.stdout);
        assert!(text.contains(&format!("Path = {name}")) && text.contains("Encrypted = +"), "{text}");
        // Extract all: the zip, moved into the folder, comes out next to itself.
        std::fs::rename(&zip, d.join("packed.zip")).unwrap();
        let (_, o) = run_it(&d, Action::ExtractAll, Opts::default());
        assert!(o.failed == 1 && o.rows.iter().any(|r| r.status.contains("password")), "{:?}", o.rows);
        std::fs::remove_file(d.join("packed.zip")).unwrap();
        // A zip without a password extracts.
        let (_, o) = run_it(&d, Action::Zip, Opts::default());
        let text = String::from_utf8_lossy(&std::process::Command::new(&seven).args(["l", "-slt"]).arg(&o.made[0]).output().unwrap().stdout).into_owned();
        assert!(!text.contains("Encrypted = +"), "an ordinary zip must not be encrypted");
        std::fs::rename(&o.made[0], d.join("plain.zip")).unwrap();
        let (_, o) = run_it(&d, Action::ExtractAll, Opts::default());
        assert_eq!(o.failed, 0, "{:?}", o.rows);
        // The zip holds one folder (the folder itself): that folder comes out.
        assert!(o.made[0].join("p 2.png").is_file(), "{:?}", o.made);
    }

    #[test]
    fn zip_in_parts_and_fit_to_size() {
        if !have(&[Tool::Ffmpeg, Tool::SevenZip]) {
            return;
        }
        let d = dir("fit");
        clip(&d.join("long.mp4"), "640x360", 30, 6);
        // Incompressible filler so parts are several.
        let noise: Vec<u8> = (0..3_000_000u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8).collect();
        std::fs::write(d.join("noise.bin"), &noise).unwrap();
        let total: u64 = std::fs::read_dir(&d).unwrap().flatten().map(|e| e.metadata().unwrap().len()).sum();
        let fit = Fit { bytes: 1_000_000, mode: FitMode::Parts };
        let (_, o) = run_it(&d, Action::Fit, Opts { fit: Some(fit), ..Opts::default() });
        let parts_dir = &o.made[0];
        let pieces = std::fs::read_dir(parts_dir).unwrap().count() as u64;
        assert_eq!(pieces, total.div_ceil(1_000_000), "{:?}", o.rows);
        std::fs::remove_dir_all(parts_dir).unwrap();
        // Fit the folder (copy) in a size smaller than the video: the video shrinks, the rest is copied.
        let video = d.join("long.mp4").metadata().unwrap().len();
        let target = 3_000_000 + video / 2 + 200_000;
        let (p, o) = run_it(&d, Action::Fit, Opts { fit: Some(Fit { bytes: target, mode: FitMode::Folder }), ..Opts::default() });
        assert_eq!(o.failed, 0, "{:?}", o.rows);
        let root = &p.new_roots[0];
        assert!(engine::disk_size(root) <= target, "{} > {target}", engine::disk_size(root));
        assert!(root.join("noise.bin").is_file() && root.join("long.mp4").is_file());
        std::fs::remove_dir_all(root).unwrap();
        // One zip in that size.
        let (p, o) = run_it(&d, Action::Fit, Opts { fit: Some(Fit { bytes: target, mode: FitMode::Zip }), ..Opts::default() });
        assert_eq!(o.made.len(), 1, "{:?}", o.rows);
        assert!(o.made[0].metadata().unwrap().len() <= target);
        assert!(p.scratch.iter().all(|s| !s.exists()), "the scratch copy is removed");
        std::fs::remove_file(&o.made[0]).unwrap();
    }

    #[test]
    fn compress_all_and_stop_after_the_current_file() {
        if !have(&[Tool::Ffmpeg, Tool::Magick]) {
            return;
        }
        let d = dir("compress");
        for n in 1..=3 {
            clip(&d.join(format!("v{n}.mp4")), "320x240", 30, 2);
        }
        picture(&d.join("pic.png"), "orange");
        let never = AtomicBool::new(false);
        let s = scan(&[d.clone()], &never, &|_| {}).unwrap();
        let prefs = crate::settings::Settings::default();
        let p = plan(&s, &Action::Compress, &Opts::default(), &Quality::default(), &prefs).unwrap();
        // Pictures first (several at a time), then the videos one by one.
        assert_eq!(p.groups.len(), 2);
        assert_eq!(p.groups[1].len(), 3);
        let id = engine::new_id();
        // Stop after the first video starts.
        let stopper: OnLive = Arc::new(move |l: Live| {
            if l.detail.contains("v1.mp4") {
                crate::procs::stop_after_current(id);
            }
        });
        let o = run(id, &p, stopper, Arc::new(|_, _| {}));
        assert!(o.stopped, "{:?}", o.rows);
        assert_eq!(o.not_started, 2, "{:?}", o.rows);
        assert_eq!(o.left.len(), 2);
        // Every file has exactly one row.
        let names: HashSet<&str> = o.rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names.len(), o.rows.len(), "{:?}", o.rows);
        assert_eq!(names.len(), 4);
        let root = &p.new_roots[0];
        let _ = std::fs::remove_dir_all(root);
    }
}
