//! The `convertino` command.
//!
//! ```text
//! convertino <files…> --to <target> [--size 25MB] [--quality small|balanced|best]
//!            [--out <folder>] [--together] [--json] [--progress] [--quiet]
//! convertino formats <files…> [--json]
//! convertino tools [status | install <name> | install --all] [--json]
//! convertino mcp
//! convertino --version | --help
//! ```
//!
//! It runs the same engine as the wheel (engine.rs) with the quality from
//! Settings, and never writes settings.json. Exit codes: 0 done, 1 some
//! files failed, 2 the command was wrong, 130 cancelled with Ctrl+C.

use crate::engine::{self, Event, Request, Sink};
use crate::{settings, size, wheel};
use serde::Serialize;
use std::collections::BTreeMap;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// println! that doesn't panic when the reader has gone (`convertino … | head`).
macro_rules! outln {
    ($($t:tt)*) => {{
        let _ = writeln!(std::io::stdout().lock(), $($t)*);
    }};
}
macro_rules! errln {
    ($($t:tt)*) => {{
        let _ = writeln!(std::io::stderr().lock(), $($t)*);
    }};
}

pub const EXIT_OK: i32 = 0;
pub const EXIT_FAILED: i32 = 1;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_CANCELLED: i32 = 130;

const HELP: &str = "Convertino: convert files from the command line.

Usage:
  convertino <files…> --to <format>      convert (jpg, png, mp3, mp4, pdf, docx, …)
  convertino <files…> --size <size>      compress to a size (25MB, 800KB, 1.5GB)
  convertino formats <files…>            what the files can become
  convertino tools                       which converters are ready
  convertino tools install <name|--all>  download converters now
  convertino mcp                         run as an MCP server (AI apps start this)

Options:
  --to <format>        a format or an action: jpg, webp, mp3, compress, split, merge,
                       720p, frames, extract, … (see `convertino formats <file>`)
  --size <size>        target size for Compress, in decimal units (10MB = 10,000,000 bytes)
  --together           with --size and several files: the size is for all of them together
  --quality <q>        small, balanced or best (default: as in Convertino Settings),
                       as tuned in Settings › Quality
  --look <n>           how close it must look, for this run only (the preset's own
                       score: pictures and PDFs about 70–87, video about 89–96)
  --encoder <e>        video: gpu (graphics card), cpu (smaller files) or auto
  --max-res <p>        video Compress: at most 2160, 1440, 1080 or 720 (or keep)
  --max-fps <n>        video Compress: at most 60 or 30 frames per second (or keep)
  --tune <list>        any preset setting for this run: look=75,floor=50,png=thorough,
                       strip-gps,codec=h264,audio=96,effort=slower,recheck=1,check-dpi=150
  --presets <file>     use presets exported from Settings › Quality instead
  --out <folder>       save there instead of next to the originals
  --json               print the result as JSON
  --progress           progress as JSON lines on stderr, for apps that run Convertino
  --quiet              print nothing but errors
  -h, --help           this help
  -V, --version        the version

Originals are never changed, and existing files are never overwritten:
a new name like \"photo (1).jpg\" is used instead.";

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ConvertArgs {
    pub files: Vec<PathBuf>,
    pub to: Option<String>,
    pub size: Option<u64>,
    pub together: bool,
    pub quality: Option<String>,
    /// One-off preset changes (--look, --encoder, --max-res, --max-fps, --tune).
    pub tune: Vec<(String, String)>,
    /// Exported presets to use instead of the ones in Settings.
    pub presets: Option<PathBuf>,
    pub out: Option<PathBuf>,
    pub json: bool,
    pub progress: bool,
    pub quiet: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Convert(ConvertArgs),
    Formats { files: Vec<PathBuf>, json: bool },
    ToolsStatus { json: bool },
    ToolsInstall { names: Vec<String>, all: bool, json: bool },
    Mcp,
    Setup(Vec<String>),
    Version,
    Help,
}

/// A size typed by a person: "25MB", "25 MB", "800kb", "1.5G", "123456" (bytes).
/// Decimal units, like upload limits.
pub fn parse_size(text: &str) -> Option<u64> {
    let t = text.trim().replace(',', "").to_lowercase();
    let split = t.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(t.len());
    let (num, unit) = t.split_at(split);
    let n: f64 = num.parse().ok()?;
    let mult = match unit.trim() {
        "" | "b" | "bytes" => 1.0,
        "k" | "kb" => 1e3,
        "m" | "mb" => 1e6,
        "g" | "gb" => 1e9,
        _ => return None,
    };
    let bytes = (n * mult).round();
    (bytes >= 1.0 && bytes < 1e15).then_some(bytes as u64)
}

fn usage(msg: impl Into<String>) -> String {
    format!("{}\nRun `convertino --help` for the options.", msg.into())
}

/// Parses the arguments (without the program name).
pub fn parse(args: &[String]) -> Result<Command, String> {
    let first = args.first().map(String::as_str);
    match first {
        None => return Ok(Command::Help),
        Some("-h" | "--help" | "help" | "/?") => return Ok(Command::Help),
        Some("-V" | "--version" | "version") => return Ok(Command::Version),
        Some("mcp") if args.len() == 1 => return Ok(Command::Mcp),
        // Used by the Windows installer and uninstaller.
        Some("setup") => return Ok(Command::Setup(args[1..].to_vec())),
        _ => {}
    }
    let mut c = ConvertArgs::default();
    let mut sub: Option<&str> = None;
    let mut rest: Vec<String> = Vec::new();
    let mut all = false;
    let mut i = 0;
    if matches!(first, Some("formats" | "tools")) && !Path::new(first.unwrap_or_default()).is_file() {
        sub = first;
        i = 1;
    }
    let value = |i: &mut usize, name: &str| -> Result<String, String> {
        *i += 1;
        args.get(*i).cloned().filter(|v| !v.starts_with("--")).ok_or_else(|| usage(format!("{name} needs a value.")))
    };
    let mut only_files = false;
    while i < args.len() {
        let a = &args[i];
        if only_files {
            rest.push(a.clone());
            i += 1;
            continue;
        }
        // --to=jpg works too.
        let (flag, inline) = match a.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
            _ => (a.clone(), None),
        };
        let take = |i: &mut usize, name: &str| -> Result<String, String> {
            match &inline {
                Some(v) => Ok(v.clone()),
                None => value(i, name),
            }
        };
        match flag.as_str() {
            "--" => only_files = true,
            "--to" | "-t" => c.to = Some(take(&mut i, "--to")?),
            "--size" | "-s" => {
                let mut v = take(&mut i, "--size")?;
                // "--size 800 KB"
                if inline.is_none() {
                    if let Some(unit) = args.get(i + 1).filter(|u| matches!(u.to_lowercase().as_str(), "b" | "kb" | "k" | "mb" | "m" | "gb" | "g")) {
                        v.push_str(unit);
                        i += 1;
                    }
                }
                c.size = Some(parse_size(&v).ok_or_else(|| usage(format!("\"{v}\" isn't a size. Try 25MB, 800KB or 1.5GB.")))?);
            }
            "--quality" | "-q" => {
                let q = take(&mut i, "--quality")?.to_lowercase();
                if !matches!(q.as_str(), "small" | "smaller" | "balanced" | "best") {
                    return Err(usage(format!("--quality is small, balanced or best, not \"{q}\".")));
                }
                c.quality = Some(if q == "smaller" { "small".into() } else { q });
            }
            "--look" => c.tune.push(("look".into(), take(&mut i, "--look")?)),
            "--encoder" => c.tune.push(("encoder".into(), take(&mut i, "--encoder")?)),
            "--max-res" => c.tune.push(("max-res".into(), take(&mut i, "--max-res")?)),
            "--max-fps" => c.tune.push(("max-fps".into(), take(&mut i, "--max-fps")?)),
            "--tune" => {
                for part in take(&mut i, "--tune")?.split(',').map(str::trim).filter(|p| !p.is_empty()) {
                    let (k, v) = part.split_once('=').unwrap_or((part, "on"));
                    c.tune.push((k.trim().to_string(), v.trim().to_string()));
                }
            }
            "--presets" => c.presets = Some(PathBuf::from(take(&mut i, "--presets")?)),
            "--out" | "-o" => c.out = Some(PathBuf::from(take(&mut i, "--out")?)),
            "--together" => c.together = true,
            "--json" => c.json = true,
            "--progress" => c.progress = true,
            "--quiet" => c.quiet = true,
            "--all" if sub == Some("tools") => all = true,
            "-h" | "--help" => return Ok(Command::Help),
            f if f.starts_with('-') && f.len() > 1 && !Path::new(f).exists() => {
                return Err(usage(format!("Unknown option {f}.")));
            }
            _ => rest.push(a.clone()),
        }
        i += 1;
    }
    match sub {
        Some("formats") => {
            if rest.is_empty() {
                return Err(usage("Which file? Example: convertino formats photo.jpg"));
            }
            Ok(Command::Formats { files: expand(&rest)?, json: c.json })
        }
        Some("tools") => match rest.first().map(String::as_str) {
            None | Some("status") => Ok(Command::ToolsStatus { json: c.json }),
            Some("install") => {
                let names: Vec<String> = rest[1..].to_vec();
                if names.is_empty() && !all {
                    return Err(usage("Which converter? Example: convertino tools install ffmpeg (or --all)"));
                }
                Ok(Command::ToolsInstall { names, all, json: c.json })
            }
            Some(other) => Err(usage(format!("Unknown tools command \"{other}\". Use status or install."))),
        },
        _ => {
            if rest.is_empty() {
                return Err(usage("Which files? Example: convertino photo.png --to jpg"));
            }
            if c.to.is_none() && c.size.is_none() {
                return Err(usage("Convert to what? Add --to <format> (or --size for Compress)."));
            }
            if c.size.is_some() && c.to.as_deref().is_some_and(|t| !t.eq_ignore_ascii_case("compress") && !t.ends_with(".compress")) {
                return Err(usage("--size works with Compress only. Leave out --to, or use --to compress."));
            }
            if c.together && c.size.is_none() {
                return Err(usage("--together needs --size."));
            }
            // Check the preset changes now, so a typo is a usage error.
            quality_with(None, None, &c.tune).map_err(usage)?;
            c.files = expand(&rest)?;
            Ok(Command::Convert(c))
        }
    }
}

/// Files from the arguments. Wildcards (`*.wav`, `IMG_04??.CR3`) are expanded
/// here, since PowerShell and Command Prompt don't; a pattern that matches
/// nothing, or a file that doesn't exist, is an error.
pub fn expand(args: &[String]) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    for a in args {
        let p = PathBuf::from(a);
        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if (name.contains('*') || name.contains('?')) && !p.exists() {
            let dir = p.parent().filter(|d| !d.as_os_str().is_empty()).map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
            let mut found: Vec<PathBuf> = std::fs::read_dir(&dir)
                .map_err(|_| usage(format!("Folder not found: {}", dir.display())))?
                .flatten()
                .map(|e| e.path())
                .filter(|f| f.is_file() && f.file_name().map(|n| wildcard(&name, &n.to_string_lossy())).unwrap_or(false))
                .collect();
            if found.is_empty() {
                return Err(usage(format!("No files match {a}")));
            }
            found.sort();
            out.extend(found);
        } else if p.is_dir() {
            return Err(usage(format!("{a} is a folder. Use {a}{}* for the files in it.", std::path::MAIN_SEPARATOR)));
        } else if !p.exists() {
            return Err(usage(format!("File not found: {a}")));
        } else {
            out.push(p);
        }
    }
    // Absolute paths: outputs, messages and MCP all agree on where things are.
    let mut seen = std::collections::HashSet::new();
    Ok(out
        .into_iter()
        .map(|p| std::path::absolute(&p).unwrap_or(p))
        .map(|p| strip_verbatim(&p))
        .filter(|p| seen.insert(p.clone()))
        .collect())
}

/// `\\?\C:\x` → `C:\x` (the converters don't all understand the long form).
pub fn strip_verbatim(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => p.to_path_buf(),
    }
}

/// `*` and `?` matching; case-insensitive on Windows and Mac, like their file systems.
pub fn wildcard(pattern: &str, name: &str) -> bool {
    let fold = |s: &str| if cfg!(any(windows, target_os = "macos")) { s.to_lowercase() } else { s.to_string() };
    let p: Vec<char> = fold(pattern).chars().collect();
    let n: Vec<char> = fold(name).chars().collect();
    let (mut pi, mut ni, mut star, mut mark) = (0, 0, None, 0);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

fn ext_of(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// One group of files that share a family and so a target id.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub target_id: String,
    pub label: String,
    pub files: Vec<PathBuf>,
}

/// Which target each file goes to. Files whose family has no such target
/// (or that Convertino can't read) are returned with the reason.
pub fn group(files: &[PathBuf], to: &str) -> (Vec<Group>, Vec<(PathBuf, String)>) {
    let mut groups: Vec<Group> = Vec::new();
    let mut skipped = Vec::new();
    for f in files {
        let ext = ext_of(f);
        match wheel::family_of(&ext) {
            None => skipped.push((f.clone(), format!("Convertino can't convert .{ext} files."))),
            Some((_, fam_label)) => match wheel::resolve(to, &ext) {
                Some(c) => match groups.iter_mut().find(|g| g.target_id == c.id) {
                    Some(g) => g.files.push(f.clone()),
                    None => groups.push(Group { target_id: c.id, label: c.label, files: vec![f.clone()] }),
                },
                None => {
                    let can: Vec<String> = wheel::conversions_for(&ext).into_iter().map(|c| c.name).collect();
                    let article = if fam_label.starts_with(['A', 'E', 'I', 'O', 'U']) { "An" } else { "A" };
                    skipped.push((f.clone(), format!("{article} {} file can't become {to}. It can become: {}", fam_label.to_lowercase(), can.join(" "))));
                }
            },
        }
    }
    (groups, skipped)
}

pub(crate) fn quality_for(word: Option<&str>) -> Option<settings::Quality> {
    let w = word?;
    let mut q = settings::get().quality;
    q.image = w.to_string();
    q.video = w.to_string();
    q.pdf_compress = if w == "best" { "high".into() } else { w.to_string() };
    Some(q)
}

/// The quality for a run: the preset `word` (else Settings'), presets from
/// a file instead of Settings' tuning, and one-off changes on top, applied to
/// the preset each kind of file uses. None: Settings as they are.
pub(crate) fn quality_with(word: Option<&str>, presets: Option<&Path>, tune: &[(String, String)]) -> Result<Option<settings::Quality>, String> {
    if word.is_none() && presets.is_none() && tune.is_empty() {
        return Ok(None);
    }
    let mut q = quality_for(word).unwrap_or_else(|| settings::get().quality);
    if let Some(file) = presets {
        let text = std::fs::read_to_string(file).map_err(|e| format!("Couldn't read {}: {e}", file.display()))?;
        q.tune = crate::tune::Tunes::import(&text)?;
    }
    let mut grades = vec![crate::tune::Grade::from_word(&q.image), crate::tune::Grade::from_word(&q.video), crate::tune::Grade::from_word(&q.pdf_compress)];
    grades.dedup();
    for (k, v) in tune {
        for g in &grades {
            q.tune.apply(*g, k, v)?;
        }
    }
    // Ranges as in Settings, but a one-off look may pass the neighbouring preset's.
    let order_free = q.tune.clone();
    q = q.clamped();
    for g in &grades {
        for kind in ["image", "video", "pdf"] {
            let wanted = match kind {
                "image" => order_free.image.get(*g).look,
                "video" => order_free.video.get(*g).look,
                _ => order_free.pdf.get(*g).look,
            };
            if let Some(l) = wanted {
                let (lo, hi) = if kind == "video" { crate::tune::VIDEO_LOOK } else { crate::tune::IMAGE_LOOK };
                let l = Some(l.clamp(lo, hi));
                match kind {
                    "image" => q.tune.image.get_mut(*g).look = l,
                    "video" => q.tune.video.get_mut(*g).look = l,
                    _ => q.tune.pdf.get_mut(*g).look = l,
                }
            }
        }
    }
    Ok(Some(q))
}

// ---------- output ----------

/// Colours on a terminal, unless NO_COLOR is set.
struct Style {
    on: bool,
}

impl Style {
    fn paint(&self, code: &str, text: &str) -> String {
        if self.on { format!("\x1b[{code}m{text}\x1b[0m") } else { text.to_string() }
    }
    fn green(&self, t: &str) -> String {
        self.paint("32", t)
    }
    fn red(&self, t: &str) -> String {
        self.paint("31", t)
    }
    fn dim(&self, t: &str) -> String {
        self.paint("90", t)
    }
}

#[cfg(windows)]
fn enable_ansi() -> bool {
    use windows::Win32::System::Console::{GetConsoleMode, GetStdHandle, SetConsoleMode, CONSOLE_MODE, ENABLE_VIRTUAL_TERMINAL_PROCESSING, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE};
    let mut ok = true;
    for which in [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        unsafe {
            let Ok(h) = GetStdHandle(which) else { return false };
            let mut mode = CONSOLE_MODE(0);
            if GetConsoleMode(h, &mut mode).is_err() || SetConsoleMode(h, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING).is_err() {
                ok = false;
            }
        }
    }
    ok
}

#[cfg(not(windows))]
fn enable_ansi() -> bool {
    true
}

/// Progress for apps (--progress): one JSON object per line on stderr,
/// {"type":"progress"|"download","file":…,"fraction":0.0–1.0,"detail":…},
/// at most each whole percent or once a second, so a reader is never flooded.
struct JsonProgress {
    last: Mutex<(f64, Option<Instant>)>,
}

impl JsonProgress {
    fn new() -> Self {
        JsonProgress { last: Mutex::new((0.0, None)) }
    }
    fn emit(&self, kind: &str, file: &str, fraction: f64, detail: &str) {
        let now = Instant::now();
        let Ok(mut last) = self.last.lock() else { return };
        let since = last.1.map(|t| now.duration_since(t).as_secs_f64());
        if !worth_printing(last.0, since, fraction) {
            return;
        }
        *last = (fraction, Some(now));
        let f = (fraction.clamp(0.0, 1.0) * 1000.0).round() / 1000.0;
        let line = serde_json::json!({ "type": kind, "file": file, "fraction": f, "detail": detail });
        let mut e = std::io::stderr().lock();
        let _ = writeln!(e, "{line}");
        let _ = e.flush();
    }
}

/// Print the first update, then on each whole percent, after a second without one, or on reaching 100 %.
fn worth_printing(last: f64, since: Option<f64>, now: f64) -> bool {
    match since {
        None => true,
        Some(s) => (now - last).abs() >= 0.01 - 1e-9 || s >= 1.0 || (now >= 1.0 && last < 1.0),
    }
}

/// The progress line on stderr (terminal only).
struct Bar {
    on: bool,
    shown: Mutex<bool>,
}

impl Bar {
    fn draw(&self, name: &str, fraction: f64, detail: &str) {
        if !self.on {
            return;
        }
        let width = 28;
        let filled = ((fraction.clamp(0.0, 1.0)) * width as f64).round() as usize;
        let bar = format!("\x1b[34m{}\x1b[90m{}\x1b[0m", "━".repeat(filled), "━".repeat(width - filled));
        let line = format!("\r\x1b[2K  {name}  {bar}  {:>3}%  {detail}", (fraction * 100.0).round() as u32);
        let mut e = std::io::stderr().lock();
        let _ = e.write_all(line.as_bytes());
        let _ = e.flush();
        if let Ok(mut s) = self.shown.lock() {
            *s = true;
        }
    }
    fn clear(&self) {
        let Ok(mut s) = self.shown.lock() else { return };
        if *s {
            let mut e = std::io::stderr().lock();
            let _ = e.write_all(b"\r\x1b[2K");
            let _ = e.flush();
            *s = false;
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonOutput {
    path: PathBuf,
    sources: Vec<PathBuf>,
    bytes: u64,
    source_bytes: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonFailure {
    source: PathBuf,
    error: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonResult {
    schema: u32,
    ok: bool,
    cancelled: bool,
    outputs: Vec<JsonOutput>,
    failed: Vec<JsonFailure>,
    notes: Vec<String>,
}

/// The job running now, so Ctrl+C can stop its converters.
static CURRENT: AtomicU64 = AtomicU64::new(0);
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

fn install_ctrl_c() {
    let _ = ctrlc::set_handler(|| {
        if INTERRUPTED.swap(true, Ordering::SeqCst) {
            // Second Ctrl+C: stop now.
            crate::procs::cancel_all();
            std::process::exit(EXIT_CANCELLED);
        }
        crate::procs::cancel(CURRENT.load(Ordering::SeqCst));
    });
}

fn sources_size(files: &[PathBuf]) -> u64 {
    files.iter().map(|f| engine::disk_size(f)).sum()
}

/// Runs a convert command; returns the exit code.
pub fn run_convert(c: &ConvertArgs) -> i32 {
    let to = c.to.clone().unwrap_or_else(|| "compress".into());
    let (groups, skipped) = group(&c.files, &to);
    let style = Style { on: !c.json && std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none() && enable_ansi() };
    let bar = Arc::new(Bar { on: !c.json && !c.quiet && std::io::stderr().is_terminal() && style.on, shown: Mutex::new(false) });

    if groups.is_empty() {
        for (_, why) in &skipped {
            errln!("{} {why}", style.red("✗"));
        }
        if c.json {
            print_json(&JsonResult {
                schema: 1,
                ok: false,
                cancelled: false,
                outputs: vec![],
                failed: skipped.iter().map(|(p, e)| JsonFailure { source: p.clone(), error: e.clone() }).collect(),
                notes: vec![],
            });
        }
        return EXIT_USAGE;
    }
    if let Some(out) = &c.out {
        if std::fs::create_dir_all(out).is_err() {
            errln!("{} Can't create the folder {}", style.red("✗"), out.display());
            return EXIT_USAGE;
        }
    }

    let quality = match quality_with(c.quality.as_deref(), c.presets.as_deref(), &c.tune) {
        Ok(q) => q,
        Err(e) => {
            errln!("{} {e}", style.red("✗"));
            return EXIT_USAGE;
        }
    };

    let mut outputs = Vec::new();
    let mut failed: Vec<(PathBuf, String)> = skipped.clone();
    let mut notes = Vec::new();
    let mut cancelled = false;
    let mut made_count = 0usize;
    for (p, why) in &skipped {
        if !c.quiet && !c.json {
            outln!("  {} {}: {why}", style.red("✗"), engine::file_name(p));
        }
    }
    for g in &groups {
        if INTERRUPTED.load(Ordering::SeqCst) {
            cancelled = true;
            break;
        }
        let id = engine::new_id();
        CURRENT.store(id, Ordering::SeqCst);
        let req = Request {
            target_id: g.target_id.clone(),
            files: g.files.clone(),
            quality: quality.clone(),
            size: c.size.map(|bytes| size::Ask { bytes, together: c.together, trim: None }),
            out_dir: c.out.clone(),
        };
        let name = match g.files.as_slice() {
            [one] => engine::file_name(one),
            many => format!("{} files", many.len()),
        };
        let (b, quiet, json) = (bar.clone(), c.quiet, c.json);
        let jp = c.progress.then(|| Arc::new(JsonProgress::new()));
        let painted = Style { on: style.on };
        let lines = Arc::new(Mutex::new(()));
        let sink: Sink = Arc::new(move |e| match e {
            Event::Progress { fraction, detail } => {
                if let Some(j) = &jp {
                    j.emit("progress", &name, fraction, &detail);
                }
                b.draw(&name, fraction, &detail)
            }
            Event::Download { fraction, detail } => {
                if let Some(j) = &jp {
                    j.emit("download", &name, fraction, &detail);
                }
                b.draw("Downloading", fraction, &detail)
            }
            Event::StepDone { inputs, result, seconds, .. } => {
                if quiet || json {
                    if let (Err(e), false) = (&result, json) {
                        errln!("{} {}: {e}", painted.red("✗"), engine::file_name(&inputs[0]));
                    }
                    return;
                }
                let _hold = lines.lock();
                b.clear();
                let from = match inputs.as_slice() {
                    [one] => engine::file_name(one),
                    many => format!("{} files", many.len()),
                };
                match result {
                    Ok(out) => outln!(
                        "  {from} → {}    {} → {}    {:.1} s",
                        engine::file_name(&out),
                        engine::human_size(sources_size(&inputs)),
                        engine::human_size(engine::disk_size(&out)),
                        seconds
                    ),
                    Err(e) if e.contains("is already compact") => outln!("  {}", painted.dim(&format!("{from}: {e}"))),
                    Err(e) => outln!("  {} {from}: {e}", painted.red("✗")),
                }
            }
        });
        let started = Instant::now();
        match engine::convert(id, &req, &sink) {
            Ok(o) => {
                bar.clear();
                made_count += o.made.len();
                cancelled |= o.cancelled;
                for (inputs, out) in &o.made_from {
                    outputs.push(JsonOutput { path: out.clone(), sources: inputs.clone(), bytes: engine::disk_size(out), source_bytes: sources_size(inputs) });
                }
                failed.extend(o.failed.iter().cloned());
                notes.extend(o.notes.iter().cloned());
                log::info!("cli: {} → {} in {:?}", g.target_id, o.made.len(), started.elapsed());
            }
            Err(why) => {
                bar.clear();
                if !c.json {
                    errln!("{} {why}", style.red("✗"));
                }
                for f in &g.files {
                    failed.push((f.clone(), why.clone()));
                }
            }
        }
    }
    cancelled |= INTERRUPTED.load(Ordering::SeqCst);

    if c.json {
        print_json(&JsonResult {
            schema: 1,
            ok: failed.is_empty() && !cancelled,
            cancelled,
            outputs,
            failed: failed.iter().map(|(p, e)| JsonFailure { source: p.clone(), error: e.clone() }).collect(),
            notes,
        });
    } else if !c.quiet {
        // ("Already compact" was shown on its file's line.)
        for n in notes.iter().filter(|n| !n.contains("is already compact")) {
            outln!("  {}", style.dim(n));
        }
        let place = match &c.out {
            Some(o) => format!("saved in {}", o.display()),
            None => "saved next to the originals".into(),
        };
        if cancelled {
            outln!("{}", style.red(&format!("Cancelled · {made_count} converted before that")));
        } else if made_count == 0 && failed.is_empty() {
            outln!("{}", style.green("✓ Nothing to do"));
        } else if failed.is_empty() {
            outln!("{}{}", style.green(&format!("✓ {made_count} converted")), style.dim(&format!(" · {place}")));
        } else {
            outln!("{}{}", style.red(&format!("✗ {} failed", failed.len())), style.dim(&format!(" · {made_count} converted, {place}")));
        }
    }
    if cancelled {
        EXIT_CANCELLED
    } else if failed.is_empty() {
        EXIT_OK
    } else {
        EXIT_FAILED
    }
}

fn print_json<T: Serialize>(v: &T) {
    outln!("{}", serde_json::to_string(v).unwrap_or_else(|_| "{}".into()));
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileFormats {
    pub path: PathBuf,
    pub family: Option<String>,
    pub bytes: u64,
    pub conversions: Vec<wheel::Conversion>,
}

pub fn formats_of(files: &[PathBuf]) -> Vec<FileFormats> {
    files
        .iter()
        .map(|f| {
            let ext = ext_of(f);
            FileFormats { path: f.clone(), family: wheel::family_of(&ext).map(|(_, l)| l), bytes: engine::disk_size(f), conversions: wheel::conversions_for(&ext) }
        })
        .collect()
}

fn run_formats(files: &[PathBuf], json: bool) -> i32 {
    let list = formats_of(files);
    if json {
        print_json(&list);
        return EXIT_OK;
    }
    let style = Style { on: std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none() && enable_ansi() };
    for f in &list {
        let name = engine::file_name(&f.path);
        match &f.family {
            None => outln!("  {name} · {}", style.red("Convertino can't convert this kind of file")),
            Some(fam) => {
                outln!("  {name} · {} · {}", fam.to_lowercase(), engine::human_size(f.bytes));
                let names: Vec<String> = f.conversions.iter().map(|c| c.name.clone()).collect();
                outln!("  {}", style.paint("36", &names.join("  ")));
            }
        }
    }
    if list.iter().any(|f| f.family.is_none()) { EXIT_FAILED } else { EXIT_OK }
}

fn run_tools_status(json: bool) -> i32 {
    let list = crate::install::status();
    if json {
        print_json(&list);
        return EXIT_OK;
    }
    for t in &list {
        let state = match t.state {
            "ready" => "ready".to_string(),
            "system" => "installed on this computer".to_string(),
            "downloading" => "downloading".to_string(),
            _ => format!("downloads when first needed (about {} MB)", t.download_mb),
        };
        outln!("  {:<12} {state}", t.name);
    }
    EXIT_OK
}

fn run_tools_install(names: &[String], all: bool, json: bool) -> i32 {
    let status = crate::install::status();
    let wanted: Vec<&'static str> = if all {
        status.iter().filter(|s| s.state == "missing").map(|s| s.id).collect()
    } else {
        let mut v = Vec::new();
        for n in names {
            let n = n.to_lowercase();
            match status.iter().find(|s| s.id == n || s.name.to_lowercase() == n) {
                Some(s) => v.push(s.id),
                None => {
                    let known: Vec<&str> = status.iter().map(|s| s.id).collect();
                    errln!("✗ Unknown converter \"{n}\". Known: {}", known.join(", "));
                    return EXIT_USAGE;
                }
            }
        }
        v
    };
    let tty = std::io::stderr().is_terminal() && enable_ansi();
    let bar = Arc::new(Bar { on: tty && !json, shown: Mutex::new(false) });
    let mut failed = BTreeMap::new();
    for id in wanted {
        let Some(pack) = crate::install::Pack::from_id(id) else { continue };
        let b = bar.clone();
        let name = pack.name().to_string();
        crate::install::set_reporter(move |f, d| b.draw(&name, f, d));
        let r = crate::install::ensure(pack);
        bar.clear();
        match r {
            Ok(()) if !json => outln!("  ✓ {} ready", pack.name()),
            Ok(()) => {}
            Err(e) => {
                if !json {
                    outln!("  ✗ {}: {e}", pack.name());
                }
                failed.insert(id, e);
            }
        }
    }
    if json {
        print_json(&serde_json::json!({ "schema": 1, "ok": failed.is_empty(), "failed": failed }));
    }
    if failed.is_empty() { EXIT_OK } else { EXIT_FAILED }
}

/// Logging for the command line: to the log file only, never the terminal.
fn init_logging() {
    // The app's log goes to its own folder via tauri-plugin-log; the command
    // line keeps quiet unless CONVERTINO_LOG is set.
    if std::env::var_os("CONVERTINO_LOG").is_some() {
        struct Stderr;
        impl log::Log for Stderr {
            fn enabled(&self, _: &log::Metadata) -> bool {
                true
            }
            fn log(&self, r: &log::Record) {
                errln!("[{}] {}", r.level(), r.args());
            }
            fn flush(&self) {}
        }
        static L: Stderr = Stderr;
        let _ = log::set_logger(&L);
        log::set_max_level(log::LevelFilter::Info);
    }
}

/// Entry point of the `convertino` command. Returns the exit code.
pub fn main() -> i32 {
    let args: Vec<String> = std::env::args_os().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
    init_logging();
    let cmd = match parse(&args) {
        Ok(c) => c,
        Err(msg) => {
            errln!("✗ {msg}");
            return EXIT_USAGE;
        }
    };
    if let Some(dir) = settings::default_config_dir() {
        settings::init_read_only(dir);
    }
    match cmd {
        Command::Help => {
            outln!("{HELP}");
            EXIT_OK
        }
        Command::Version => {
            outln!("convertino {}", env!("CARGO_PKG_VERSION"));
            EXIT_OK
        }
        Command::Mcp => crate::mcp::serve(),
        Command::Setup(a) => crate::shell::setup(&a),
        Command::Formats { files, json } => run_formats(&files, json),
        Command::ToolsStatus { json } => run_tools_status(json),
        Command::ToolsInstall { names, all, json } => run_tools_install(&names, all, json),
        Command::Convert(c) => {
            install_ctrl_c();
            run_convert(&c)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    fn tmp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("convertino-cli-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn touch(dir: &Path, name: &str) -> String {
        let p = dir.join(name);
        std::fs::write(&p, b"x").unwrap();
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn sizes() {
        assert_eq!(parse_size("25MB"), Some(25_000_000));
        assert_eq!(parse_size("25 mb"), Some(25_000_000));
        assert_eq!(parse_size("800KB"), Some(800_000));
        assert_eq!(parse_size("800k"), Some(800_000));
        assert_eq!(parse_size("1.5GB"), Some(1_500_000_000));
        assert_eq!(parse_size("1,200,000"), Some(1_200_000));
        assert_eq!(parse_size("123456"), Some(123_456));
        assert_eq!(parse_size("0MB"), None);
        assert_eq!(parse_size("ten MB"), None);
        assert_eq!(parse_size("25 parsecs"), None);
        assert_eq!(parse_size(""), None);
    }

    #[test]
    fn progress_flag() {
        let d = tmp_dir("progress");
        let a = touch(&d, "clip.mp4");
        let Command::Convert(c) = parse(&args(&[&a, "--to", "compress", "--json", "--progress"])).unwrap() else { panic!() };
        assert!(c.progress && c.json);
        let Command::Convert(c) = parse(&args(&[&a, "--to", "compress"])).unwrap() else { panic!() };
        assert!(!c.progress);
    }

    #[test]
    fn preset_flags() {
        let d = tmp_dir("tuneflags");
        let a = touch(&d, "clip.mp4");
        let Command::Convert(c) = parse(&args(&[&a, "--to", "compress", "--quality", "balanced", "--look", "91", "--encoder", "cpu", "--max-res", "1080", "--max-fps=60", "--tune", "strip-gps,effort=slower"])).unwrap() else { panic!() };
        assert_eq!(c.tune.len(), 6);
        let q = quality_with(c.quality.as_deref(), None, &c.tune).unwrap().unwrap();
        let v = q.tune.video(crate::tune::Grade::Balanced);
        assert_eq!((v.look, v.encoder, v.max_res, v.max_fps, v.effort), (91.0, crate::tune::Encoder::Cpu, 1080, 60, 2));
        assert!(q.tune.image(crate::tune::Grade::Balanced).strip_gps);
        // A one-off look may pass the next preset's (Best is 95.5 by default).
        let q = quality_with(Some("balanced"), None, &[("look".into(), "97".into())]).unwrap().unwrap();
        assert_eq!(q.tune.video(crate::tune::Grade::Balanced).look, 97.0);
        assert!(parse(&args(&[&a, "--to", "compress", "--encoder", "quantum"])).is_err());
        assert!(parse(&args(&[&a, "--to", "compress", "--tune", "colour=red"])).is_err());
        assert_eq!(quality_with(None, None, &[]).unwrap(), None, "no flags: Settings as they are");
        // Presets from a file.
        let mut t = crate::tune::Tunes::default();
        t.video.best.look = Some(98.0);
        let f = d.join("p.json");
        std::fs::write(&f, t.export().to_string()).unwrap();
        let q = quality_with(Some("best"), Some(&f), &[]).unwrap().unwrap();
        assert_eq!(q.tune.video(crate::tune::Grade::Best).look, 98.0);
    }

    #[test]
    fn progress_throttle() {
        assert!(worth_printing(0.0, None, 0.0)); // the first update always shows
        assert!(!worth_printing(0.100, Some(0.2), 0.105)); // under a percent, under a second
        assert!(worth_printing(0.100, Some(0.2), 0.110)); // a whole percent
        assert!(worth_printing(0.100, Some(1.5), 0.101)); // a second has passed
        assert!(worth_printing(0.995, Some(0.1), 1.0)); // finishing always shows
        assert!(!worth_printing(1.0, Some(0.1), 1.0));
    }

    #[test]
    fn wildcards() {
        assert!(wildcard("*.wav", "song.wav"));
        assert!(!wildcard("*.wav", "song.wav.bak"));
        assert!(wildcard("IMG_04??.CR3", "IMG_0412.CR3"));
        assert!(!wildcard("IMG_04??.CR3", "IMG_041.CR3"));
        assert!(wildcard("*", "anything"));
        assert!(wildcard("a*b*c", "aXXbYYc"));
        assert!(!wildcard("a*b*c", "aXXbYY"));
        if cfg!(any(windows, target_os = "macos")) {
            assert!(wildcard("*.WAV", "song.wav"));
        }
    }

    #[test]
    fn simple_and_help_commands() {
        assert_eq!(parse(&[]).unwrap(), Command::Help);
        assert_eq!(parse(&args(&["--help"])).unwrap(), Command::Help);
        assert_eq!(parse(&args(&["-V"])).unwrap(), Command::Version);
        assert_eq!(parse(&args(&["mcp"])).unwrap(), Command::Mcp);
        assert_eq!(parse(&args(&["tools"])).unwrap(), Command::ToolsStatus { json: false });
        assert_eq!(parse(&args(&["tools", "install", "--all", "--json"])).unwrap(), Command::ToolsInstall { names: vec![], all: true, json: true });
        assert!(parse(&args(&["tools", "install"])).is_err());
        assert!(parse(&args(&["tools", "frobnicate"])).is_err());
        // (Run from src-tauri, where a "tools" folder exists: still the tools command.)
    }

    #[test]
    fn convert_arguments() {
        let d = tmp_dir("args");
        let a = touch(&d, "a.png");
        let Command::Convert(c) = parse(&args(&[&a, "--to", "jpg", "--json"])).unwrap() else { panic!() };
        assert_eq!(c.to.as_deref(), Some("jpg"));
        assert!(c.json);
        assert_eq!(c.files.len(), 1);
        assert!(c.files[0].is_absolute());
        let Command::Convert(c) = parse(&args(&[&a, "--to=webp"])).unwrap() else { panic!() };
        assert_eq!(c.to.as_deref(), Some("webp"));
        let Command::Convert(c) = parse(&args(&[&a, "--size", "800", "KB", "--together"])).unwrap() else { panic!() };
        assert_eq!(c.size, Some(800_000));
        let Command::Convert(c) = parse(&args(&[&a, "--size", "2MB", "--to", "compress", "--quality", "best", "--out", "x"])).unwrap() else { panic!() };
        assert_eq!((c.size, c.quality.as_deref(), c.out.clone()), (Some(2_000_000), Some("best"), Some(PathBuf::from("x"))));
        // Mistakes are usage errors with a hint.
        assert!(parse(&args(&[&a])).unwrap_err().contains("--to"));
        assert!(parse(&args(&[&a, "--to"])).is_err());
        assert!(parse(&args(&[&a, "--size", "big"])).unwrap_err().contains("isn't a size"));
        assert!(parse(&args(&[&a, "--size", "2MB", "--to", "jpg"])).unwrap_err().contains("Compress"));
        assert!(parse(&args(&[&a, "--to", "jpg", "--together"])).is_err());
        assert!(parse(&args(&[&a, "--to", "jpg", "--quality", "ultra"])).is_err());
        assert!(parse(&args(&[&a, "--to", "jpg", "--frobnicate"])).unwrap_err().contains("Unknown option"));
        assert!(parse(&args(&[d.join("missing.png").to_str().unwrap(), "--to", "jpg"])).unwrap_err().contains("not found"));
        assert!(parse(&args(&[d.to_str().unwrap(), "--to", "jpg"])).unwrap_err().contains("folder"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn wildcards_expand_and_dedupe() {
        let d = tmp_dir("glob");
        touch(&d, "b.wav");
        touch(&d, "a.wav");
        touch(&d, "c.mp3");
        let pat = d.join("*.wav").to_string_lossy().into_owned();
        let Command::Convert(c) = parse(&args(&[&pat, &pat, "--to", "mp3"])).unwrap() else { panic!() };
        let names: Vec<String> = c.files.iter().map(|f| engine::file_name(f)).collect();
        assert_eq!(names, vec!["a.wav", "b.wav"]);
        assert!(parse(&args(&[d.join("*.flac").to_str().unwrap(), "--to", "mp3"])).unwrap_err().contains("No files match"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn names_resolve_to_slots() {
        let r = |w: &str, ext: &str| wheel::resolve(w, ext).map(|c| c.id);
        assert_eq!(r("jpg", "png").as_deref(), Some("image.jpg"));
        assert_eq!(r("JPEG", "png").as_deref(), Some("image.jpg"));
        assert_eq!(r(".webp", "png").as_deref(), Some("image.webp"));
        assert_eq!(r("image.avif", "png").as_deref(), Some("image.avif"));
        assert_eq!(r("compress", "mp4").as_deref(), Some("video.compress"));
        assert_eq!(r("mp3", "mp4").as_deref(), Some("video.mp3"));
        assert_eq!(r("mp3", "wav").as_deref(), Some("audio.mp3"));
        assert_eq!(r("png", "pdf").as_deref(), Some("pdf.png"));
        assert_eq!(r("pdf", "docx").as_deref(), Some("doc.pdf"));
        assert_eq!(r("xlsx", "csv").as_deref(), Some("data.xlsx"));
        assert_eq!(r("extract", "zip").as_deref(), Some("archive.extract"));
        assert_eq!(r("png", "png"), None, "same format is not a conversion");
        assert_eq!(r("docx", "pdf"), None);
        assert_eq!(r("edit", "pdf"), None, "the PDF editor needs the app");
        assert_eq!(r("edit", "png"), None, "the image editor needs the app");
        assert_eq!(r("edit", "cr3"), None);
        assert_eq!(r("jpg", "xyz"), None);
    }

    #[test]
    fn every_listed_name_resolves_back() {
        for ext in ["png", "jpg", "wav", "mp4", "pdf", "docx", "md", "csv", "json", "zip", "svg", "heic", "gif", "xlsx", "pptx"] {
            for c in wheel::conversions_for(ext) {
                let back = wheel::resolve(&c.name, ext).map(|x| x.id);
                assert_eq!(back.as_deref(), Some(c.id.as_str()), "{ext}: {} should mean {}", c.name, c.id);
            }
        }
    }

    #[test]
    fn mixed_files_group_by_target() {
        let d = tmp_dir("group");
        let files: Vec<PathBuf> = ["a.png", "b.jpg", "c.wav", "d.xyz"].iter().map(|n| PathBuf::from(touch(&d, n))).collect();
        let (groups, skipped) = group(&files, "webp");
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].target_id, "image.webp");
        assert_eq!(groups[0].files.len(), 2);
        assert_eq!(skipped.len(), 2);
        assert!(skipped.iter().any(|(_, w)| w.contains("can't become webp") && w.contains("mp3")));
        assert!(skipped.iter().any(|(_, w)| w.contains(".xyz")));
        let (groups, _) = group(&files, "compress");
        let ids: Vec<&str> = groups.iter().map(|g| g.target_id.as_str()).collect();
        assert!(ids.contains(&"image.compress"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn json_result_shape_is_stable() {
        let r = JsonResult {
            schema: 1,
            ok: true,
            cancelled: false,
            outputs: vec![JsonOutput { path: "o.jpg".into(), sources: vec!["i.png".into()], bytes: 2, source_bytes: 3 }],
            failed: vec![JsonFailure { source: "x.png".into(), error: "bad".into() }],
            notes: vec!["n".into()],
        };
        assert_eq!(
            serde_json::to_string(&r).unwrap(),
            r#"{"schema":1,"ok":true,"cancelled":false,"outputs":[{"path":"o.jpg","sources":["i.png"],"bytes":2,"sourceBytes":3}],"failed":[{"source":"x.png","error":"bad"}],"notes":["n"]}"#
        );
    }
}
