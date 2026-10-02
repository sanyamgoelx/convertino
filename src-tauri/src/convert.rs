//! Turns a wheel pick into concrete conversion steps and runs them.
//!
//! Engines: ImageMagick (images), FFmpeg (audio, audio out of video),
//! Poppler (PDF pages, text, split, merge), Ghostscript (PDF compress and
//! grayscale), LibreOffice (Office documents) and Pandoc (Markdown, HTML).
//! Video, data and archives live in their own modules (video, data, archive).
//! Outputs are written next to the source with a name that never
//! overwrites an existing file or folder.

use crate::look::{self, Level, Lossy};
use crate::settings::Quality;
use crate::tools::{self, Tool};
use crate::{archive, data, procs, size, video};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PageFormat {
    Jpg,
    Png,
    Webp,
}

/// What a step does.
#[derive(Debug, Clone)]
pub enum Op {
    /// ImageMagick: inputs, `args`, output. `first_frame` reads only frame 0.
    Magick { args: Vec<String>, first_frame: bool },
    /// A lossy picture (JPG, WebP, AVIF): `prep` (flatten, resize) then the
    /// smallest quality that still looks the same at `level` (None: `fixed`).
    Picture { prep: Vec<String>, format: Lossy, level: Option<Level>, fixed: u32 },
    /// ImageMagick to PNG, then made smaller without changing a pixel.
    Png { args: Vec<String>, level: Option<Level> },
    /// Audio with FFmpeg: `args` (filters), then the codec for `ext`, chosen
    /// when it runs so a low-bitrate source isn't blown up. `mp3`: kbps from Settings.
    Audio { args: Vec<String>, mp3: u32 },
    /// Every PDF page as an image, into a folder (a single page becomes a file).
    /// JPG/WebP pages use the quality search at `level` (None: `quality`).
    PdfPages { format: PageFormat, dpi: u32, quality: u32, level: Option<Level> },
    /// Ghostscript at several strengths; the smallest that still looks like
    /// the original wins.
    PdfCompress { level: Level },
    /// Every PDF page into one multi-page TIFF.
    PdfTiff { dpi: u32 },
    PdfText,
    PdfSplit,
    PdfMerge,
    /// Ghostscript pdfwrite with these settings.
    /// `shrink`: fail instead of keeping a result that isn't smaller than the input.
    Ghostscript { args: Vec<String>, shrink: bool },
    /// LibreOffice `--convert-to` filter, e.g. "pdf" or "docx:MS Word 2007 XML".
    Office { convert_to: String },
    /// Pandoc to the output's format; `to` forces a writer (gfm, plain).
    Pandoc { to: Option<String>, standalone: bool },
    /// LibreOffice to DOCX first, then Pandoc (e.g. .doc to Markdown).
    OfficeThenPandoc { to: String },
    /// Pandoc to DOCX first, then LibreOffice (e.g. Markdown to PDF).
    PandocThenOffice { convert_to: String },
    /// FFmpeg video job; codecs are chosen when it runs.
    Video { job: video::Job, opts: video::Opts },
    /// Data file to another data format, in Rust.
    Data { to: data::Format },
    /// Unpack next to the archive (a single item, or a folder).
    Extract,
    /// Unpack and pack again in another format.
    Repack { to: archive::Pack },
    /// Compress to a size (None: the smallest that still looks the same).
    ToSize { kind: size::Kind, bytes: Option<u64>, trim: Option<f64> },
}

/// One unit of work producing one output file or folder.
#[derive(Debug, Clone)]
pub struct Step {
    pub inputs: Vec<PathBuf>,
    pub dir: PathBuf,
    pub stem: String,
    /// Extension of the output file, or of the files inside an output folder.
    pub ext: String,
    /// The output is a folder named `stem` (PDF pages, split).
    pub folder: bool,
    pub op: Op,
}

pub(crate) fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

pub(crate) fn split(path: &Path) -> (PathBuf, String, String) {
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let stem = path.file_stem().map(|x| x.to_string_lossy().into_owned()).unwrap_or_else(|| "output".into());
    let ext = path.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
    (dir, stem, ext)
}

/// LAME VBR level for about `kbps` on average (V0 ≈ 245, V2 ≈ 190, V4 ≈ 165, V5 ≈ 130, V7 ≈ 100).
fn mp3_vbr(kbps: u32) -> &'static str {
    match kbps {
        256.. => "0",
        192.. => "2",
        160.. => "4",
        128.. => "5",
        _ => "7",
    }
}

/// FFmpeg audio encoder settings for an output extension. `mp3`: kbps asked
/// for in Settings; `cap`: no point going far above a lossy source's bitrate.
fn audio_codec_for(ext: &str, mp3: u32, cap: Option<u32>) -> Vec<String> {
    let at_most = |kbps: u32| cap.map_or(kbps, |c| kbps.min(c.max(64)));
    let kb = |kbps: u32| format!("{}k", at_most(kbps));
    match ext {
        // VBR: the same sound as constant bitrate in a smaller file.
        "mp3" => s(&["-c:a", "libmp3lame", "-q:a", mp3_vbr(at_most(mp3)), "-id3v2_version", "3"]),
        "wav" => s(&["-c:a", "pcm_s16le"]),
        "flac" => s(&["-c:a", "flac", "-compression_level", "8"]),
        "aac" => s(&["-c:a", "aac", "-b:a", &kb(256)]),
        "m4a" => s(&["-c:a", "aac", "-b:a", &kb(256), "-movflags", "+faststart"]),
        "ogg" => s(&["-c:a", "libvorbis", "-q:a", if at_most(192) >= 160 { "6" } else { "4" }]),
        "opus" => s(&["-c:a", "libopus", "-b:a", &kb(96)]),
        "aiff" | "aif" => s(&["-c:a", "pcm_s16be"]),
        "wma" => s(&["-c:a", "wmav2", "-b:a", &kb(192)]),
        _ => Vec::new(),
    }
}

fn audio_codec(ext: &str) -> Vec<String> {
    audio_codec_for(ext, 320, None)
}

/// Lossy audio codecs: converting from these never needs more bits than they had.
const LOSSY_AUDIO: &[&str] = &["mp3", "aac", "opus", "vorbis", "wmav2", "wmav1", "ac3", "eac3"];

const TRIM_FILTER: &str = "silenceremove=start_periods=1:start_threshold=-50dB:start_silence=0.2,areverse,\
silenceremove=start_periods=1:start_threshold=-50dB:start_silence=0.2,areverse";

// Document source groups.
const WORD_LIKE: &[&str] = &["docx", "doc", "odt", "rtf", "txt"];
const OFFICE_ANY: &[&str] = &["docx", "doc", "odt", "rtf", "txt", "pptx", "ppt", "odp", "xlsx", "xls", "ods", "html", "htm"];
const PANDOC_READS: &[&str] = &["docx", "odt", "rtf", "html", "htm", "md"];

/// How to produce `target_ext` from a document with `src`, or None if unsupported.
fn doc_op(target_ext: &str, src: &str) -> Option<Op> {
    let office = |f: &str| Some(Op::Office { convert_to: f.to_string() });
    match (target_ext, src) {
        ("pdf", "md") => Some(Op::PandocThenOffice { convert_to: "pdf".into() }),
        ("pdf", s) if OFFICE_ANY.contains(&s) => office("pdf"),
        ("txt", "md" | "html" | "htm") => Some(Op::Pandoc { to: Some("plain".into()), standalone: false }),
        ("txt", s) if WORD_LIKE.contains(&s) => office("txt:Text (encoded):UTF8"),
        ("html", "md") => Some(Op::Pandoc { to: None, standalone: true }),
        ("html", s) if WORD_LIKE.contains(&s) => office("html"),
        ("md", "doc") => Some(Op::OfficeThenPandoc { to: "gfm".into() }),
        ("md", s) if PANDOC_READS.contains(&s) => Some(Op::Pandoc { to: Some("gfm".into()), standalone: false }),
        ("docx", "md" | "html" | "htm") => Some(Op::Pandoc { to: None, standalone: false }),
        ("docx", s) if WORD_LIKE.contains(&s) => office("docx:MS Word 2007 XML"),
        ("odt", "md" | "html" | "htm") => Some(Op::Pandoc { to: None, standalone: false }),
        ("odt", s) if WORD_LIKE.contains(&s) => office("odt"),
        // First sheet, comma separated, UTF-8.
        ("csv", "xlsx" | "xls" | "ods") => office("csv:Text - txt - csv (StarCalc):44,34,76,1,,0,false,true,true"),
        _ => None,
    }
}

/// The steps for a target, or a user-facing reason it can't run.
#[cfg_attr(not(test), allow(dead_code))] // tests and the smoke run; jobs use plan_with
pub fn plan(target_id: &str, files: &[PathBuf]) -> Result<Vec<Step>, String> {
    plan_with(target_id, files, &crate::settings::get().quality)
}

/// The steps with these quality settings (Settings, or Shift+click options on the wheel).
pub fn plan_with(target_id: &str, files: &[PathBuf], q: &Quality) -> Result<Vec<Step>, String> {
    plan_sized(target_id, files, q, None).map(|(steps, _)| steps)
}

/// The steps, with a size from the wheel's size ring for Compress, and notes
/// for the person (files already small enough are left out).
pub fn plan_sized(target_id: &str, files: &[PathBuf], q: &Quality, ask: Option<size::Ask>) -> Result<(Vec<Step>, Vec<String>), String> {
    if files.is_empty() {
        return Err("There was nothing to convert.".into());
    }
    // Compress to a size; without one, pictures and audio still go through it
    // (the smallest that looks the same), video and PDF keep their own Compress.
    if let Some(kind) = size::Kind::of_target(target_id) {
        if ask.is_some() || matches!(kind, size::Kind::Image | size::Kind::Audio) {
            let p = size::plan(kind, files, ask)?;
            return Ok((p.steps, p.notes));
        }
    }
    plan_plain(target_id, files, q).map(|steps| (steps, Vec::new()))
}

fn plan_plain(target_id: &str, files: &[PathBuf], q: &Quality) -> Result<Vec<Step>, String> {
    if files.is_empty() {
        return Err("There was nothing to convert.".into());
    }
    let one = |f: &PathBuf, stem: String, ext: &str, folder: bool, op: Op| {
        let (dir, _, _) = split(f);
        Step { inputs: vec![f.clone()], dir, stem, ext: ext.into(), folder, op }
    };
    let each = |ext: &str, suffix: Option<&str>, folder_suffix: Option<&str>, op: Op| -> Vec<Step> {
        files
            .iter()
            .map(|f| {
                let (_, stem, _) = split(f);
                let stem = match (suffix, folder_suffix) {
                    (_, Some(fs)) => format!("{stem} – {fs}"),
                    (Some(sfx), None) => format!("{stem} ({sfx})"),
                    (None, None) => stem,
                };
                one(f, stem, ext, folder_suffix.is_some(), op.clone())
            })
            .collect()
    };
    let magick = |args: Vec<String>, first_frame: bool| Op::Magick { args, first_frame };
    // "Size when converting": shrink pictures larger than this (never enlarge).
    let fit = || if q.convert_max > 0 { s(&["-resize", &format!("{0}x{0}>", q.convert_max)]) } else { Vec::new() };
    let opts = video::Opts::from_quality(q);
    let level = Level::from_setting(&q.image);
    let picture = |prep: Vec<String>, format: Lossy, fixed: u32| Op::Picture { prep, format, level, fixed };
    let png = |args: Vec<String>| Op::Png { args, level };
    let flatten = || s(&["-background", "white", "-alpha", "remove", "-alpha", "off"]);

    let audio = |out_ext: Option<&str>, suffix: Option<&str>, filter: Option<&str>| -> Vec<Step> {
        files
            .iter()
            .map(|f| {
                let (_, stem, src_ext) = split(f);
                // Trim / Normalize keep the format, unless it's one FFmpeg can't write well.
                let ext = match out_ext {
                    Some(e) => e.to_string(),
                    None if audio_codec(&src_ext).is_empty() => "m4a".to_string(),
                    None => src_ext,
                };
                let mut args = s(&["-vn", "-map_metadata", "0"]);
                if let Some(flt) = filter {
                    args.extend(s(&["-af", flt]));
                }
                let stem = match suffix {
                    Some(sfx) => format!("{stem} ({sfx})"),
                    None => stem,
                };
                one(f, stem, &ext, false, Op::Audio { args, mp3: q.mp3 })
            })
            .collect()
    };

    let steps = match target_id {
        // ---------- images ----------
        "image.jpg" => each("jpg", None, None, picture([flatten(), fit()].concat(), Lossy::Jpg, q.jpg)),
        "image.png" => each("png", None, None, png(fit())),
        "image.webp" => each("webp", None, None, picture(fit(), Lossy::Webp, q.webp)),
        "image.avif" => each("avif", None, None, picture(fit(), Lossy::Avif, 60)),
        "image.tiff" => each("tiff", None, None, magick([fit(), s(&["-compress", "lzw"])].concat(), true)),
        "image.bmp" => each("bmp", None, None, magick(fit(), true)),
        "image.gif" => each("gif", None, None, magick(fit(), true)),
        "image.ico" => each(
            "ico",
            None,
            None,
            magick(s(&["-background", "none", "-define", "icon:auto-resize=256,128,64,48,32,16"]), true),
        ),
        "image.pdf" => {
            let (dir, stem, _) = split(&files[0]);
            let stem = if files.len() == 1 { stem } else { "Combined images".to_string() };
            vec![Step { inputs: files.to_vec(), dir, stem, ext: "pdf".into(), folder: false, op: magick(fit(), false) }]
        }

        // ---------- audio, and audio out of video ----------
        "audio.mp3" | "video.mp3" => audio(Some("mp3"), None, None),
        "audio.wav" | "video.wav" => audio(Some("wav"), None, None),
        "audio.flac" => audio(Some("flac"), None, None),
        "audio.aac" => audio(Some("aac"), None, None),
        "audio.ogg" => audio(Some("ogg"), None, None),
        "audio.opus" => audio(Some("opus"), None, None),
        "audio.m4a" => audio(Some("m4a"), None, None),
        "audio.aiff" => audio(Some("aiff"), None, None),
        "audio.trim" => audio(None, Some("trimmed"), Some(TRIM_FILTER)),
        "audio.normalize" => audio(None, Some("normalized"), Some("loudnorm=I=-14:TP=-1:LRA=11")),

        // ---------- PDF ----------
        "pdf.jpg" => each("jpg", None, Some("pages"), Op::PdfPages { format: PageFormat::Jpg, dpi: q.dpi, quality: q.jpg, level }),
        "pdf.png" => each("png", None, Some("pages"), Op::PdfPages { format: PageFormat::Png, dpi: q.dpi, quality: 0, level }),
        "pdf.webp" => each("webp", None, Some("pages"), Op::PdfPages { format: PageFormat::Webp, dpi: q.dpi, quality: q.webp, level }),
        "pdf.tiff" => each("tiff", None, None, Op::PdfTiff { dpi: 300 }),
        "pdf.txt" => each("txt", None, None, Op::PdfText),
        "pdf.split" => each("pdf", None, Some("split"), Op::PdfSplit),
        "pdf.compress" => each(
            "pdf",
            Some("compressed"),
            None,
            Op::PdfCompress {
                level: match q.pdf_compress.as_str() {
                    "small" => Level::Small,
                    "high" => Level::Best,
                    _ => Level::Balanced,
                },
            },
        ),
        "pdf.grayscale" => each(
            "pdf",
            Some("grayscale"),
            None,
            Op::Ghostscript {
                args: s(&["-sColorConversionStrategy=Gray", "-dProcessColorModel=/DeviceGray", "-dOverrideICC"]),
                shrink: false,
            },
        ),
        "pdf.merge" => {
            let mut sorted = files.to_vec();
            sorted.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default());
            let (dir, _, _) = split(&sorted[0]);
            vec![Step { inputs: sorted, dir, stem: "Merged".into(), ext: "pdf".into(), folder: false, op: Op::PdfMerge }]
        }

        // ---------- video ----------
        "video.mp4" => each("mp4", None, None, Op::Video { job: video::Job::Mp4, opts }),
        "video.mov" => each("mov", None, None, Op::Video { job: video::Job::Mov, opts }),
        "video.webm" => each("webm", None, None, Op::Video { job: video::Job::Webm, opts }),
        "video.mkv" => each("mkv", None, None, Op::Video { job: video::Job::Mkv, opts }),
        "video.gif" => each("gif", None, None, Op::Video { job: video::Job::Gif, opts }),
        "video.compress" => each("mp4", Some("compressed"), None, Op::Video { job: video::Job::Compress, opts }),
        "video.720p" => each("mp4", Some("720p"), None, Op::Video { job: video::Job::To720p, opts }),
        "video.frames" => each("png", None, Some("frames"), Op::Video { job: video::Job::Frames, opts }),

        // ---------- data ----------
        t if t.starts_with("data.") => {
            let to = data::Format::from_ext(&t[5..]).ok_or_else(|| not_yet(t))?;
            let ext = if to == data::Format::Yaml { "yaml" } else { &t[5..] };
            each(ext, None, None, Op::Data { to })
        }

        // ---------- archives ----------
        "archive.extract" => files
            .iter()
            .map(|f| {
                let (dir, _, _) = split(f);
                Step { inputs: vec![f.clone()], dir, stem: archive::archive_stem(f), ext: String::new(), folder: false, op: Op::Extract }
            })
            .collect(),
        "archive.zip" | "archive.7z" | "archive.targz" => {
            let (to, ext) = match target_id {
                "archive.zip" => (archive::Pack::Zip, "zip"),
                "archive.7z" => (archive::Pack::SevenZ, "7z"),
                _ => (archive::Pack::TarGz, "tar.gz"),
            };
            files
                .iter()
                .map(|f| {
                    let (dir, _, _) = split(f);
                    Step { inputs: vec![f.clone()], dir, stem: archive::archive_stem(f), ext: ext.into(), folder: false, op: Op::Repack { to } }
                })
                .collect()
        }

        // ---------- documents ----------
        t if t.starts_with("doc.") => {
            let target_ext = &t[4..];
            let mut steps = Vec::new();
            let mut unsupported = Vec::new();
            for f in files {
                let (_, stem, src) = split(f);
                let src = if src == "htm" { "html".to_string() } else { src };
                match doc_op(target_ext, &src) {
                    Some(op) => steps.push(one(f, stem, target_ext, false, op)),
                    None => unsupported.push(format!(".{src}")),
                }
            }
            if steps.is_empty() {
                return Err(format!(
                    "Convertino can't turn {} files into {} yet.",
                    unsupported.join(", "),
                    target_ext.to_uppercase()
                ));
            }
            steps
        }

        _ => return Err(not_yet(target_id)),
    };
    if steps.is_empty() {
        return Err("There was nothing to convert.".into());
    }
    Ok(steps)
}

fn not_yet(target_id: &str) -> String {
    let what = match target_id.split('.').next().unwrap_or("") {
        "pdf" => "This PDF tool isn't",
        "doc" => "This document conversion isn't",
        "video" => "Video conversions aren't",
        "data" => "Data conversions aren't",
        "archive" => "Archive tools aren't",
        _ => "This conversion isn't",
    };
    format!("{what} available yet. It arrives in a later update.")
}

// ---------- output names ----------

/// A path in `dir` that doesn't exist yet: "name.ext", then "name (1).ext"
/// on Windows or "name 2.ext" on Mac, matching each OS's own copies.
/// An empty `ext` names a folder.
#[cfg_attr(not(test), allow(dead_code))]
pub fn unique_output(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    free_name(dir, stem, ext, &HashSet::new())
}

fn free_name(dir: &Path, stem: &str, ext: &str, taken: &HashSet<PathBuf>) -> PathBuf {
    let name = |suffix: &str| {
        if ext.is_empty() { format!("{stem}{suffix}") } else { format!("{stem}{suffix}.{ext}") }
    };
    let free = |p: &PathBuf| !p.exists() && !taken.contains(p);
    let first = dir.join(name(""));
    if free(&first) {
        return first;
    }
    let mac = cfg!(target_os = "macos");
    let mut n = if mac { 2 } else { 1 };
    loop {
        let p = dir.join(name(&if mac { format!(" {n}") } else { format!(" ({n})") }));
        if free(&p) {
            return p;
        }
        n += 1;
    }
}

fn reserved() -> &'static Mutex<HashSet<PathBuf>> {
    static R: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashSet::new()))
}

/// An output name no other running job can take until this is dropped:
/// two jobs finishing "photo.webp" at once get "photo.webp" and "photo (1).webp".
pub(crate) struct Reserved(pub(crate) PathBuf);

impl Reserved {
    pub(crate) fn new(dir: &Path, stem: &str, ext: &str) -> Reserved {
        let mut taken = reserved().lock().unwrap_or_else(|e| e.into_inner());
        let p = free_name(dir, stem, ext, &taken);
        taken.insert(p.clone());
        Reserved(p)
    }
}

impl Drop for Reserved {
    fn drop(&mut self) {
        if let Ok(mut taken) = reserved().lock() {
            taken.remove(&self.0);
        }
    }
}

/// `dir` if Convertino can save there, else the Downloads folder (a read-only
/// folder, a CD, a protected system folder).
/// Where a conversion of a file in `dir` is saved: there, or the one folder
/// chosen in Settings; a read-only place falls back to Downloads.
pub(crate) fn output_dir(dir: &Path) -> PathBuf {
    match crate::settings::get().output_folder() {
        Some(folder) if std::fs::create_dir_all(&folder).is_ok() => writable_dir(&folder),
        Some(folder) => {
            log::warn!("the save folder {} isn't available; saving next to the original", folder.display());
            writable_dir(dir)
        }
        None => writable_dir(dir),
    }
}

pub(crate) fn writable_dir(dir: &Path) -> PathBuf {
    // Two tries, the second with an ordinary name: security software can
    // refuse hidden dot-files on their own.
    let probe = |name: String| -> std::io::Result<()> {
        let p = dir.join(name);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&p) {
            Ok(_) => {
                let _ = std::fs::remove_file(&p);
                Ok(())
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
            Err(e) => Err(e),
        }
    };
    let pid = std::process::id();
    let tried = probe(format!(".convertino-write-test-{pid}")).or_else(|_| probe(format!("convertino-write-test-{pid}.tmp")));
    match tried {
        Ok(()) => dir.to_path_buf(),
        Err(e) => {
            let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"));
            match home.map(|h| PathBuf::from(h).join("Downloads")).filter(|d| d.is_dir() && d != dir) {
                Some(downloads) => {
                    let why = Blocked::of(&e);
                    log::warn!("can't write to {} ({e}, {why:?}); saving to {}", dir.display(), downloads.display());
                    if let Ok(mut m) = blocked().lock() {
                        m.insert(dir.to_path_buf(), why);
                    }
                    downloads
                }
                None => dir.to_path_buf(),
            }
        }
    }
}

/// Why a folder couldn't be written, so the done card can say so (and how to fix it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Blocked {
    /// Windows Security's ransomware protection ("Controlled folder access").
    RansomwareProtection,
    /// macOS privacy settings (Files and Folders / Full Disk Access).
    MacPrivacy,
    /// No permission (another user's folder, a protected system folder).
    Denied,
    /// A read-only drive, disc or share.
    ReadOnly,
}

impl Blocked {
    fn of(e: &std::io::Error) -> Blocked {
        match e.raw_os_error() {
            // ERROR_WRITE_PROTECT
            Some(19) if cfg!(windows) => Blocked::ReadOnly,
            // Ransomware protection refuses with "access denied" or, oddly,
            // "file not found" (seen in Videos on a friend's PC).
            _ if cfg!(windows) && ransomware_protection_on() => Blocked::RansomwareProtection,
            // EROFS
            Some(30) if !cfg!(windows) => Blocked::ReadOnly,
            // EPERM: macOS privacy controls
            Some(1) if cfg!(target_os = "macos") => Blocked::MacPrivacy,
            _ => Blocked::Denied,
        }
    }
}

/// Folders Convertino couldn't write this session, and why.
pub(crate) fn blocked() -> &'static Mutex<HashMap<PathBuf, Blocked>> {
    static B: OnceLock<Mutex<HashMap<PathBuf, Blocked>>> = OnceLock::new();
    B.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The same, asked once per run of Convertino.
fn ransomware_protection_on_cached() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(ransomware_protection_on)
}

/// Moves a finished file or folder into place (copying across drives).
fn move_into(from: &Path, to: &Path) -> Result<(), String> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    fn copy(from: &Path, to: &Path) -> std::io::Result<()> {
        if from.is_dir() {
            std::fs::create_dir_all(to)?;
            for e in std::fs::read_dir(from)? {
                let e = e?;
                copy(&e.path(), &to.join(e.file_name()))?;
            }
            Ok(())
        } else {
            std::fs::copy(from, to).map(|_| ())
        }
    }
    copy(from, to).map_err(|e| {
        if to.is_dir() {
            let _ = std::fs::remove_dir_all(to);
        } else {
            let _ = std::fs::remove_file(to);
        }
        format!("Couldn't save the result in {}: {e}", to.parent().map(|p| p.display().to_string()).unwrap_or_default())
    })
}

/// Windows Security's "Controlled folder access" (blocks unknown apps from
/// writing in Documents, Pictures, Videos, Music and Desktop).
#[cfg(windows)]
fn ransomware_protection_on() -> bool {
    use std::os::windows::process::CommandExt;
    const NO_WINDOW: u32 = 0x0800_0000;
    let key = r"HKLM\SOFTWARE\Microsoft\Windows Defender\Windows Defender Exploit Guard\Controlled Folder Access";
    let policy = r"HKLM\SOFTWARE\Policies\Microsoft\Windows Defender\Windows Defender Exploit Guard\Controlled Folder Access";
    [key, policy].iter().any(|k| {
        std::process::Command::new("reg.exe")
            .args(["query", k, "/v", "EnableControlledFolderAccess"])
            .creation_flags(NO_WINDOW)
            .output()
            .map(|o| {
                let t = String::from_utf8_lossy(&o.stdout);
                // "EnableControlledFolderAccess    REG_DWORD    0x1" (2 = audit only)
                t.lines().any(|l| l.contains("EnableControlledFolderAccess") && l.trim_end().ends_with("0x1"))
            })
            .unwrap_or(false)
    })
}

#[cfg(not(windows))]
fn ransomware_protection_on() -> bool {
    false
}

// ---------- running ----------

static TEMP_N: AtomicU64 = AtomicU64::new(0);

/// A fresh scratch folder, removed when dropped.
pub(crate) struct TempDir(pub(crate) PathBuf);
impl TempDir {
    pub(crate) fn new(tag: &str) -> Result<Self, String> {
        let n = TEMP_N.fetch_add(1, Ordering::SeqCst);
        let p = std::env::temp_dir().join(format!("convertino-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).map_err(|e| format!("Couldn't create a temporary folder: {e}"))?;
        Ok(TempDir(p))
    }
}
impl TempDir {
    /// A scratch folder inside `dir` (hidden by its leading dot).
    pub(crate) fn inside(dir: &Path, tag: &str) -> Result<Self, String> {
        let n = TEMP_N.fetch_add(1, Ordering::SeqCst);
        let p = dir.join(format!(".convertino-{tag}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&p).map_err(|e| format!("Couldn't create a temporary folder: {e}"))?;
        Ok(TempDir(p))
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn plain_name(p: &Path) -> bool {
    p.to_str().is_some_and(|s| s.is_ascii())
}

/// Ghostscript (with -dSAFER) and pdfunite on Windows can't open paths with
/// characters outside plain ASCII ("Café – मेनू.pdf"). Such a job works on
/// copies under plain temporary names, and the result is moved into place.
struct Plain {
    tmp: Option<TempDir>,
}

impl Plain {
    fn new<'a>(paths: impl IntoIterator<Item = &'a Path>) -> Result<Self, String> {
        Self::when(cfg!(windows), paths)
    }

    fn when<'a>(needed_here: bool, paths: impl IntoIterator<Item = &'a Path>) -> Result<Self, String> {
        let paths: Vec<&Path> = paths.into_iter().collect();
        if !needed_here || paths.iter().all(|p| plain_name(p)) {
            return Ok(Plain { tmp: None });
        }
        // The temporary folder itself needs a plain path (a user name with accents won't do).
        let base = std::env::temp_dir();
        let tmp = if plain_name(&base) {
            TempDir::new("plain")?
        } else {
            let near = paths.iter().filter_map(|p| p.parent()).find(|d| plain_name(d)).map(Path::to_path_buf);
            match near {
                Some(dir) => TempDir::inside(&dir, "plain")?,
                None => TempDir::new("plain")?,
            }
        };
        Ok(Plain { tmp: Some(tmp) })
    }

    fn input(&self, p: &Path, n: usize) -> Result<PathBuf, String> {
        match &self.tmp {
            Some(t) if !plain_name(p) => {
                let ext = p.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
                let copy = t.0.join(format!("in{n}.{ext}"));
                std::fs::copy(p, &copy).map_err(|e| format!("Couldn't read {}: {e}", p.display()))?;
                Ok(copy)
            }
            _ => Ok(p.to_path_buf()),
        }
    }

    fn output(&self, p: &Path) -> PathBuf {
        match &self.tmp {
            Some(t) => t.0.join(format!("out.{}", p.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default())),
            None => p.to_path_buf(),
        }
    }

    fn finish(&self, made: &Path, output: &Path) -> Result<(), String> {
        if made != output {
            move_file(made, output)?;
        }
        Ok(())
    }
}

/// Moves a file, copying when the destination is on another drive.
pub(crate) fn move_file(from: &Path, to: &Path) -> Result<(), String> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    std::fs::copy(from, to).map_err(|e| format!("Couldn't save {}: {e}", to.display()))?;
    let _ = std::fs::remove_file(from);
    Ok(())
}

pub(crate) fn last_lines(text: &str) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let tail = &lines[lines.len().saturating_sub(2)..];
    let msg = tail.join(" ").trim().to_string();
    if msg.is_empty() { "unknown error".into() } else { msg }
}

/// Longest a converter that reports no progress may run before it's stopped.
const NO_PROGRESS_LIMIT: Duration = Duration::from_secs(15 * 60);

/// Runs a command to completion (cancellable, with a time limit); the error
/// explains the tool's last words in plain language.
pub(crate) fn exec(tool: Tool, mut cmd: Command) -> Result<(), String> {
    let out = procs::output(&mut cmd, tool.display_name(), Some(NO_PROGRESS_LIMIT))?;
    if out.status.success() {
        Ok(())
    } else {
        let mut text = String::from_utf8_lossy(&out.stderr).into_owned();
        if text.trim().is_empty() {
            text = String::from_utf8_lossy(&out.stdout).into_owned();
        }
        Err(tool_error(tool.display_name(), &text))
    }
}

/// Logs a converter's raw error and returns the plain-language version.
pub(crate) fn tool_error(tool: &str, raw: &str) -> String {
    log::warn!("{tool} failed: {}", last_lines(raw));
    procs::friendly(tool, raw)
}

/// Runs a step and returns where its result was written.
/// `progress` gets 0.0–1.0 when the tool reports it.
pub fn run(step: &Step, progress: &mut dyn FnMut(f64)) -> Result<PathBuf, String> {
    if let Some(missing) = step.inputs.iter().find(|p| !p.exists()) {
        return Err(format!("{} is no longer there.", missing.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
    }
    let mut step = step.clone();
    step.dir = output_dir(&step.dir);
    let step = &step;
    let name = Reserved::new(&step.dir, &step.stem, if step.folder { "" } else { &step.ext });
    let reserved = name.0.clone();
    // With Windows' ransomware protection on, only Convertino itself may be
    // allowed into Documents, Videos and so on, not each converter: the
    // converter writes into a scratch folder and Convertino moves the result.
    let staging = if ransomware_protection_on_cached() { Some(TempDir::new("out")?) } else { None };
    let output = match &staging {
        Some(t) => t.0.join(reserved.file_name().unwrap_or_default()),
        None => reserved.clone(),
    };
    if step.folder {
        std::fs::create_dir_all(&output).map_err(|e| format!("Couldn't create {}: {e}", output.display()))?;
    }
    let result = run_op(step, &output, progress);
    // Cancelled while the tool was finishing: treat it as cancelled all the same.
    let result = if result.is_ok() && procs::cancelled_here() { Err(procs::CANCELLED.to_string()) } else { result };
    let result = match (result, &staging) {
        (Ok(made), Some(_)) if made.exists() => {
            let name = made.file_name().unwrap_or_default().to_string_lossy().into_owned();
            let dest = if made == output {
                reserved.clone()
            } else {
                let p = Path::new(&name);
                let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or(name.clone());
                let ext = p.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
                let r = Reserved::new(&step.dir, &stem, &ext);
                r.0.clone()
            };
            move_into(&made, &dest).map(|_| dest)
        }
        (r, _) => r,
    };
    let output = if staging.is_some() { reserved.clone() } else { output };
    match result {
        Ok(final_path) => {
            if final_path != output && output.exists() && step.folder {
                // The folder was replaced by a single file (a one-page PDF); drop the empty folder.
                let _ = std::fs::remove_dir(&output);
            }
            if !final_path.exists() {
                return Err("The converter finished but didn't write anything.".into());
            }
            progress(1.0);
            Ok(final_path)
        }
        Err(e) => {
            // Remove what we half-wrote; it's ours, never the user's file.
            if step.folder {
                let _ = std::fs::remove_dir_all(&output);
            } else {
                let _ = std::fs::remove_file(&output);
            }
            Err(e)
        }
    }
}

fn run_op(step: &Step, output: &Path, progress: &mut dyn FnMut(f64)) -> Result<PathBuf, String> {
    let input = &step.inputs[0];
    match &step.op {
        Op::Magick { args, first_frame } => {
            magick(&step.inputs, *first_frame, args, output)?;
            Ok(output.to_path_buf())
        }
        Op::Audio { args, mp3 } => {
            let src = video::probe(input).ok();
            let cap = src
                .as_ref()
                .filter(|p| p.audio.as_deref().is_some_and(|a| LOSSY_AUDIO.contains(&a)))
                .and_then(|p| p.audio_kbps)
                .map(|k| k + k / 10);
            let ext = output.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            let all = [args.clone(), audio_codec_for(&ext, *mp3, cap)].concat();
            run_ffmpeg(input, &all, output, progress)?;
            Ok(output.to_path_buf())
        }
        Op::Picture { prep, format, level, fixed } => {
            progress(0.1);
            let q = look::picture(input, prep, *format, *level, *fixed, output)?;
            log::info!("picture {format:?} at quality {q}");
            Ok(output.to_path_buf())
        }
        Op::Png { args, level } => {
            magick(std::slice::from_ref(input), true, &[s(&["-auto-orient"]), args.clone()].concat(), output)?;
            progress(0.5);
            look::shrink_png(output, *level);
            Ok(output.to_path_buf())
        }
        Op::PdfCompress { level } => {
            pdf_compress(input, *level, output, progress)?;
            Ok(output.to_path_buf())
        }
        Op::PdfPages { format, dpi, quality, level } => pdf_pages(step, *format, *dpi, *quality, *level, output, progress),
        Op::PdfTiff { dpi } => {
            let tmp = TempDir::new("tiff")?;
            pdftoppm(input, &["-tiff", "-tiffcompression", "lzw"], *dpi, &tmp.0.join("p"), &mut |p| progress(p * 0.8))?;
            let pages = files_in(&tmp.0)?;
            magick(&pages, false, &s(&["-compress", "lzw"]), output)?;
            Ok(output.to_path_buf())
        }
        Op::PdfText => {
            let mut cmd = tools::command(&tools::require(Tool::Pdftotext)?);
            cmd.args(["-layout", "-enc", "UTF-8"]).arg(input).arg(output);
            exec(Tool::Pdftotext, cmd)?;
            let text = std::fs::read_to_string(output).unwrap_or_default();
            if text.trim().is_empty() {
                return Err("This PDF has no selectable text; it's probably a scan.".into());
            }
            Ok(output.to_path_buf())
        }
        Op::PdfSplit => {
            let mut cmd = tools::command(&tools::require(Tool::Pdfseparate)?);
            // pdfseparate treats % as a placeholder, so escape it in the name.
            let (_, stem, _) = split(input);
            let pattern = output.join(format!("{}-%d.pdf", stem.replace('%', "%%")));
            cmd.arg(input).arg(pattern);
            exec(Tool::Pdfseparate, cmd)?;
            Ok(output.to_path_buf())
        }
        Op::PdfMerge => {
            let exe = tools::require(Tool::Pdfunite)?;
            let plain = Plain::new(step.inputs.iter().map(PathBuf::as_path).chain([output]))?;
            let mut cmd = tools::command(&exe);
            for (i, f) in step.inputs.iter().enumerate() {
                cmd.arg(plain.input(f, i)?);
            }
            let out = plain.output(output);
            cmd.arg(&out);
            exec(Tool::Pdfunite, cmd)?;
            plain.finish(&out, output)?;
            Ok(output.to_path_buf())
        }
        Op::Ghostscript { args, shrink } => {
            let exe = tools::require(Tool::Ghostscript)?;
            let plain = Plain::new([input.as_path(), output])?;
            let (src, out) = (plain.input(input, 0)?, plain.output(output));
            let mut cmd = tools::command(&exe);
            cmd.args(["-sDEVICE=pdfwrite", "-dNOPAUSE", "-dBATCH", "-dQUIET", "-dSAFER"])
                .args(args)
                .arg(format!("-sOutputFile={}", out.display()))
                .arg(&src);
            exec(Tool::Ghostscript, cmd)?;
            plain.finish(&out, output)?;
            if *shrink {
                let before = input.metadata().map(|m| m.len()).unwrap_or(0);
                let after = output.metadata().map(|m| m.len()).unwrap_or(0);
                // Under 5% smaller isn't worth a second copy.
                if after as f64 >= before as f64 * 0.95 {
                    return Err("This PDF is already compact; compressing it wouldn't make it smaller.".into());
                }
            }
            Ok(output.to_path_buf())
        }
        Op::Office { convert_to } => {
            office(input, convert_to, output)?;
            if convert_to.starts_with("csv") {
                // Same as Convertino's own CSVs: a byte-order mark so Excel reads accents correctly.
                if let Ok(body) = std::fs::read(output) {
                    if !body.starts_with(b"\xEF\xBB\xBF") {
                        let _ = std::fs::write(output, [b"\xEF\xBB\xBF".as_slice(), &body].concat());
                    }
                }
            }
            Ok(output.to_path_buf())
        }
        Op::Pandoc { to, standalone } => {
            pandoc(input, to.as_deref(), *standalone, output)?;
            Ok(output.to_path_buf())
        }
        Op::OfficeThenPandoc { to } => {
            let tmp = TempDir::new("doc")?;
            let (_, stem, _) = split(input);
            let docx = tmp.0.join(format!("{stem}.docx"));
            office(input, "docx:MS Word 2007 XML", &docx)?;
            progress(0.5);
            pandoc(&docx, Some(to), false, output)?;
            Ok(output.to_path_buf())
        }
        Op::Video { job, opts } => video::run(input, *job, opts, output, progress),
        Op::ToSize { kind, bytes, trim } => size::run(*kind, input, *bytes, *trim, output, progress),
        Op::Data { to } => {
            data::convert(input, *to, output)?;
            Ok(output.to_path_buf())
        }
        Op::Extract => archive::extract(input, &step.dir, progress),
        Op::Repack { to } => {
            archive::repack(input, *to, output, progress)?;
            Ok(output.to_path_buf())
        }
        Op::PandocThenOffice { convert_to } => {
            let tmp = TempDir::new("doc")?;
            let (_, stem, _) = split(input);
            let docx = tmp.0.join(format!("{stem}.docx"));
            pandoc(input, None, false, &docx)?;
            progress(0.5);
            office(&docx, convert_to, output)?;
            Ok(output.to_path_buf())
        }
    }
}

pub(crate) fn files_in(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file())
        .collect();
    v.sort();
    Ok(v)
}

fn magick(inputs: &[PathBuf], first_frame: bool, args: &[String], output: &Path) -> Result<(), String> {
    let mut cmd = tools::command(&tools::require(Tool::Magick)?);
    for input in inputs {
        let mut arg = input.as_os_str().to_os_string();
        if first_frame {
            arg.push("[0]");
        }
        cmd.arg(arg);
    }
    cmd.args(args).arg(output);
    exec(Tool::Magick, cmd)
}

/// pdftoppm with -progress: stderr lines are "<page> <total> <file>".
fn pdftoppm(input: &Path, fmt: &[&str], dpi: u32, prefix: &Path, progress: &mut dyn FnMut(f64)) -> Result<(), String> {
    let mut cmd = tools::command(&tools::require(Tool::Pdftoppm)?);
    cmd.args(fmt)
        .args(["-r", &dpi.to_string(), "-progress"])
        .arg(input)
        .arg(prefix)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut tracked = procs::spawn(&mut cmd, "Poppler")?;
    let child = tracked.child();
    let mut errors = String::new();
    for line in BufReader::new(child.stderr.take().expect("piped")).lines().map_while(Result::ok) {
        let mut parts = line.split_whitespace();
        match (parts.next().and_then(|a| a.parse::<f64>().ok()), parts.next().and_then(|b| b.parse::<f64>().ok())) {
            (Some(page), Some(total)) if total > 0.0 => progress((page / total).clamp(0.0, 1.0)),
            _ => {
                errors.push_str(&line);
                errors.push('\n');
            }
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if procs::cancelled_here() {
        Err(procs::CANCELLED.into())
    } else if status.success() {
        Ok(())
    } else {
        Err(tool_error("Poppler", &errors))
    }
}

#[allow(clippy::too_many_arguments)]
fn pdf_pages(
    step: &Step,
    format: PageFormat,
    dpi: u32,
    quality: u32,
    level: Option<Level>,
    folder: &Path,
    progress: &mut dyn FnMut(f64),
) -> Result<PathBuf, String> {
    let input = &step.inputs[0];
    let (_, stem, _) = split(input);
    let prefix = folder.join(&stem);
    // Pages are rendered losslessly; JPG/WebP are encoded afterwards, at the
    // quality the look search picks on the first page.
    let share = if format == PageFormat::Png { 0.8 } else { 0.6 };
    pdftoppm(input, &["-png"], dpi, &prefix, &mut |p| progress(p * share))?;
    let pngs = files_in(folder)?;
    if pngs.is_empty() {
        return Err("The PDF has no pages.".into());
    }
    let n = pngs.len() as f64;
    match format {
        PageFormat::Png => {
            for (i, png) in pngs.iter().enumerate() {
                look::shrink_png(png, level);
                progress(share + (1.0 - share) * (i + 1) as f64 / n);
            }
        }
        PageFormat::Jpg | PageFormat::Webp => {
            let lossy = if format == PageFormat::Jpg { Lossy::Jpg } else { Lossy::Webp };
            let q = match level {
                Some(level) => {
                    let tmp = TempDir::new("pages")?;
                    let crop = look::search_crop(&pngs[0], &[], &tmp.0)?;
                    look::search_quality(&crop, lossy, level.picture_target())?
                }
                None => quality,
            };
            log::info!("PDF pages as {lossy:?} at quality {q}");
            let ext = if format == PageFormat::Jpg { "jpg" } else { "webp" };
            for (i, png) in pngs.iter().enumerate() {
                magick(std::slice::from_ref(png), false, &lossy.args(q), &png.with_extension(ext))?;
                let _ = std::fs::remove_file(png);
                progress(share + (1.0 - share) * (i + 1) as f64 / n);
            }
        }
    }

    // A one-page PDF becomes a single file next to it, not a folder of one.
    let made = files_in(folder)?;
    if made.len() == 1 {
        let file = Reserved::new(&step.dir, &stem, &step.ext);
        let file = file.0.clone();
        move_file(&made[0], &file)?;
        let _ = std::fs::remove_dir(folder);
        return Ok(file);
    }
    Ok(folder.to_path_buf())
}

/// Ghostscript settings for one PDF Compress strength: None keeps every
/// picture as it is (only fonts, duplicates and structure get smaller);
/// Some(dpi) also shrinks pictures above that resolution.
fn pdf_compress_args(dpi: Option<u32>) -> Vec<String> {
    let mut a = s(&[
        "-dCompatibilityLevel=1.6",
        "-dDetectDuplicateImages=true",
        "-dCompressFonts=true",
        "-dSubsetFonts=true",
        "-dCompressPages=true",
    ]);
    match dpi {
        None => a.extend(s(&[
            "-dPassThroughJPEGImages=true",
            "-dPassThroughJPXImages=true",
            "-dDownsampleColorImages=false",
            "-dDownsampleGrayImages=false",
            "-dDownsampleMonoImages=false",
            "-dAutoFilterColorImages=false",
            "-dAutoFilterGrayImages=false",
            "-dColorImageFilter=/FlateEncode",
            "-dGrayImageFilter=/FlateEncode",
        ])),
        Some(d) => {
            let preset = match d {
                0..=96 => "/screen",
                97..=200 => "/ebook",
                _ => "/printer",
            };
            a.extend(s(&[&format!("-dPDFSETTINGS={preset}"), "-dPassThroughJPEGImages=false"]));
            for kind in ["Color", "Gray"] {
                a.extend(s(&[
                    &format!("-dDownsample{kind}Images=true"),
                    &format!("-d{kind}ImageDownsampleType=/Bicubic"),
                    &format!("-d{kind}ImageResolution={d}"),
                    &format!("-d{kind}ImageDownsampleThreshold=1.5"),
                ]));
            }
            a.extend(s(&[
                "-dDownsampleMonoImages=true",
                &format!("-dMonoImageResolution={}", (d * 2).max(300)),
                "-dMonoImageDownsampleThreshold=1.5",
            ]));
        }
    }
    a
}

/// The strengths PDF Compress tries, from gentlest to strongest.
const PDF_STRENGTHS: [Option<u32>; 4] = [None, Some(300), Some(150), Some(72)];

/// PDF Compress: every strength is made (in parallel), sample pages of each
/// are compared with the original, and the smallest that still looks like it
/// wins. The bar is relative to the gentlest version (fonts can render a hair
/// differently after any rewrite, which isn't a loss of quality).
fn pdf_compress(input: &Path, level: Level, output: &Path, progress: &mut dyn FnMut(f64)) -> Result<(), String> {
    let exe = tools::require(Tool::Ghostscript)?;
    let plain = Plain::new([input])?;
    let src = plain.input(input, 0)?;
    let tmp = TempDir::new("pdfc")?;
    let job = procs::current_job();
    let versions: Vec<(Option<u32>, Result<PathBuf, String>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = PDF_STRENGTHS
            .iter()
            .map(|dpi| {
                let (exe, src, dir) = (&exe, &src, &tmp.0);
                let dpi = *dpi;
                scope.spawn(move || {
                    procs::set_current_job(job);
                    let out = dir.join(format!("v{}.pdf", dpi.unwrap_or(0)));
                    let mut cmd = tools::command(exe);
                    cmd.args(["-sDEVICE=pdfwrite", "-dNOPAUSE", "-dBATCH", "-dQUIET", "-dSAFER"])
                        .args(pdf_compress_args(dpi))
                        .arg(format!("-sOutputFile={}", out.display()))
                        .arg(src);
                    (dpi, exec(Tool::Ghostscript, cmd).map(|_| out))
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap_or((None, Err("Ghostscript stopped".into())))).collect()
    });
    if procs::cancelled_here() {
        return Err(procs::CANCELLED.into());
    }
    progress(0.5);
    let before = input.metadata().map(|m| m.len()).unwrap_or(0);
    let mut made: Vec<(Option<u32>, PathBuf, u64)> = Vec::new();
    for (dpi, r) in versions {
        match r {
            Ok(p) => {
                let size = p.metadata().map(|m| m.len()).unwrap_or(u64::MAX);
                made.push((dpi, p, size));
            }
            Err(e) if dpi.is_none() => return Err(e),
            Err(e) => log::info!("PDF Compress at {dpi:?} DPI failed: {e}"),
        }
    }

    // Only versions smaller than the gentlest one need a look check.
    let gentlest = made.iter().find(|m| m.0.is_none()).map(|m| m.2).unwrap_or(u64::MAX);
    let mut candidates: Vec<&(Option<u32>, PathBuf, u64)> = made.iter().filter(|m| m.0.is_none() || m.2 < gentlest).collect();
    candidates.sort_by_key(|m| m.2);
    let render_dpi = if level == Level::Best { 150 } else { 110 };
    let pages = look::pdf_page_count(&src).unwrap_or(1);
    let mut original_pages = Vec::new();
    for page in look::sample_pages(pages) {
        original_pages.push((page, look::render_page(&src, page, render_dpi, &tmp.0, "orig")?));
    }
    let floor = made
        .iter()
        .find(|m| m.0.is_none())
        .map(|m| look::pdf_score(&original_pages, &m.1, render_dpi, &tmp.0, "v0"))
        .unwrap_or(100.0);
    let bar = level.picture_target().min(floor - 3.0);
    let mut chosen = None;
    for (i, (dpi, path, size)) in candidates.iter().enumerate() {
        if procs::cancelled_here() {
            return Err(procs::CANCELLED.into());
        }
        let sc = if dpi.is_none() { floor } else { look::pdf_score(&original_pages, path, render_dpi, &tmp.0, "v") };
        log::info!("PDF Compress at {dpi:?} DPI: {size} bytes, looks {sc:.1} (bar {bar:.1})");
        progress(0.5 + 0.5 * (i + 1) as f64 / candidates.len() as f64);
        if sc >= bar {
            chosen = Some((path.clone(), *size));
            break;
        }
    }
    let (path, size) = chosen.ok_or("Ghostscript couldn't compress this PDF.")?;
    // Under 5% smaller isn't worth a second copy.
    if size as f64 >= before as f64 * 0.95 {
        return Err("This PDF is already compact; compressing it wouldn't make it smaller.".into());
    }
    move_file(&path, output)
}

/// Strengths tried for a PDF size, gentlest first.
const PDF_SIZE_STRENGTHS: [Option<u32>; 8] = [None, Some(300), Some(200), Some(150), Some(110), Some(90), Some(72), Some(50)];

/// PDF Compress to a size: every strength is made (four at a time) and the
/// gentlest one that fits wins. When none fits, the smallest is kept and
/// the card says so. Without a size: the usual Compress.
pub(crate) fn pdf_to_size(input: &Path, target: Option<u64>, output: &Path, progress: &mut dyn FnMut(f64)) -> Result<(), String> {
    let Some(target) = target else {
        let level = match crate::settings::get().quality.pdf_compress.as_str() {
            "small" => Level::Small,
            "high" => Level::Best,
            _ => Level::Balanced,
        };
        return pdf_compress(input, level, output, progress);
    };
    let exe = tools::require(Tool::Ghostscript)?;
    let plain = Plain::new([input])?;
    let src = plain.input(input, 0)?;
    let tmp = TempDir::new("pdfs")?;
    let job = procs::current_job();
    let mut made: Vec<(Option<u32>, PathBuf, u64)> = Vec::new();
    for (round, batch) in PDF_SIZE_STRENGTHS.chunks(4).enumerate() {
        let versions: Vec<(Option<u32>, Result<PathBuf, String>)> = std::thread::scope(|scope| {
            let handles: Vec<_> = batch
                .iter()
                .map(|dpi| {
                    let (exe, src, dir) = (&exe, &src, &tmp.0);
                    let dpi = *dpi;
                    scope.spawn(move || {
                        procs::set_current_job(job);
                        let out = dir.join(format!("s{}.pdf", dpi.unwrap_or(0)));
                        let mut cmd = tools::command(exe);
                        cmd.args(["-sDEVICE=pdfwrite", "-dNOPAUSE", "-dBATCH", "-dQUIET", "-dSAFER"])
                            .args(pdf_compress_args(dpi))
                            .arg(format!("-sOutputFile={}", out.display()))
                            .arg(src);
                        (dpi, exec(Tool::Ghostscript, cmd).map(|_| out))
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap_or((None, Err("Ghostscript stopped".into())))).collect()
        });
        if procs::cancelled_here() {
            return Err(procs::CANCELLED.into());
        }
        for (dpi, r) in versions {
            match r {
                Ok(p) => {
                    let size = p.metadata().map(|m| m.len()).unwrap_or(u64::MAX);
                    log::info!("PDF size: at {dpi:?} DPI {size} bytes (target {target})");
                    made.push((dpi, p, size));
                }
                Err(e) => log::info!("PDF size at {dpi:?} DPI failed: {e}"),
            }
        }
        progress(0.5 * (round + 1) as f64);
        // The gentlest that fits, in the order tried.
        if let Some(fit) = PDF_SIZE_STRENGTHS.iter().find_map(|d| made.iter().find(|m| m.0 == *d && m.2 <= target)) {
            log::info!("PDF size: {:?} DPI fits", fit.0);
            return move_file(&fit.1, output);
        }
    }
    let before = input.metadata().map(|m| m.len()).unwrap_or(0);
    let smallest = made.iter().min_by_key(|m| m.2).ok_or("Ghostscript couldn't compress this PDF.")?;
    if smallest.2 as f64 >= before as f64 * 0.95 {
        return Err("This PDF is already compact; compressing it wouldn't make it smaller.".into());
    }
    note(format!(
        "{} couldn't get under {}; this is the smallest it can be ({}). Its text and drawings take the room, not pictures.",
        input.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        size::words(target),
        size::words(smallest.2)
    ));
    move_file(&smallest.1, output)
}

/// Notes for the person, per job ("couldn't get under 2 MB"), shown on the card.
fn notes() -> &'static Mutex<HashMap<u64, Vec<String>>> {
    static N: OnceLock<Mutex<HashMap<u64, Vec<String>>>> = OnceLock::new();
    N.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn note(text: String) {
    log::info!("note: {text}");
    if let Ok(mut n) = notes().lock() {
        n.entry(procs::current_job()).or_default().push(text);
    }
}

/// The notes a job left, once.
pub(crate) fn take_notes(job: u64) -> Vec<String> {
    notes().lock().ok().and_then(|mut n| n.remove(&job)).unwrap_or_default()
}

/// A LibreOffice profile of our own, so a running LibreOffice doesn't block conversions.
fn office_profile_url() -> String {
    let dir = std::env::temp_dir().join("convertino-libreoffice-profile");
    let p = dir.to_string_lossy().replace('\\', "/").replace(' ', "%20");
    if p.starts_with('/') { format!("file://{p}") } else { format!("file:///{p}") }
}

/// LibreOffice headless conversion of `input` to exactly `output`.
fn office(input: &Path, convert_to: &str, output: &Path) -> Result<(), String> {
    let exe = tools::require(Tool::LibreOffice)?;
    // One LibreOffice at a time: two sharing Convertino's profile would clash.
    static OFFICE: Mutex<()> = Mutex::new(());
    let _one_at_a_time = OFFICE.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new("office")?;
    let mut cmd = tools::command(&exe);
    cmd.args(["--headless", "--norestore", "--nolockcheck", "--nodefault"])
        .arg(format!("-env:UserInstallation={}", office_profile_url()))
        .args(["--convert-to", convert_to, "--outdir"])
        .arg(&tmp.0)
        .arg(input);
    exec(Tool::LibreOffice, cmd)?;
    // LibreOffice names the result after the input, with the filter's extension.
    let made = files_in(&tmp.0)?;
    let first = made.first().ok_or("LibreOffice couldn't convert this file. It may be damaged or password-protected.")?;
    move_file(first, output)
}

fn pandoc(input: &Path, to: Option<&str>, standalone: bool, output: &Path) -> Result<(), String> {
    let mut cmd = tools::command(&tools::require(Tool::Pandoc)?);
    if let Some(dir) = input.parent() {
        // Relative images in Markdown and HTML resolve from the file's own folder.
        cmd.arg(format!("--resource-path={}", dir.display()));
    }
    if input.extension().map(|e| e.eq_ignore_ascii_case("htm")).unwrap_or(false) {
        cmd.args(["-f", "html"]);
    }
    if let Some(t) = to {
        cmd.args(["-t", t]);
    }
    if standalone {
        cmd.arg("-s");
    }
    cmd.arg(input).arg("-o").arg(output);
    exec(Tool::Pandoc, cmd)
}

/// "Duration: 00:03:36.12," -> seconds
pub(crate) fn parse_duration(line: &str) -> Option<f64> {
    let rest = line.split("Duration: ").nth(1)?;
    let hms = rest.split(',').next()?.trim();
    let mut parts = hms.split(':');
    let h: f64 = parts.next()?.parse().ok()?;
    let m: f64 = parts.next()?.parse().ok()?;
    let sec: f64 = parts.next()?.parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + sec)
}

pub(crate) fn run_ffmpeg(input: &Path, args: &[String], output: &Path, progress: &mut dyn FnMut(f64)) -> Result<(), String> {
    run_ffmpeg_in(None, input, args, output, progress)
}

/// FFmpeg working in `dir` (two-pass encoding writes its log there).
pub(crate) fn run_ffmpeg_in(dir: Option<&Path>, input: &Path, args: &[String], output: &Path, progress: &mut dyn FnMut(f64)) -> Result<(), String> {
    let mut cmd = tools::command(&tools::require(Tool::Ffmpeg)?);
    if let Some(d) = dir {
        cmd.current_dir(d);
    }
    cmd.args(["-hide_banner", "-nostdin", "-n", "-i"])
        .arg(input)
        .args(args)
        .args(["-progress", "pipe:1", "-nostats"])
        .arg(output)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut tracked = procs::spawn(&mut cmd, "FFmpeg")?;
    let child = tracked.child();

    // FFmpeg prints the input's duration on stderr and progress on stdout.
    let duration = Arc::new(Mutex::new(None::<f64>));
    let stderr_text = Arc::new(Mutex::new(String::new()));
    let stderr = child.stderr.take().expect("piped");
    let (d2, t2) = (duration.clone(), stderr_text.clone());
    let err_thread = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if let Some(d) = parse_duration(&line) {
                if let Ok(mut g) = d2.lock() {
                    g.get_or_insert(d);
                }
            }
            if let Ok(mut t) = t2.lock() {
                t.push_str(&line);
                t.push('\n');
            }
        }
    });

    let stdout = child.stdout.take().expect("piped");
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        if let Some(us) = line.strip_prefix("out_time_us=").or_else(|| line.strip_prefix("out_time_ms=")) {
            if let (Ok(us), Some(total)) = (us.trim().parse::<f64>(), duration.lock().ok().and_then(|d| *d)) {
                if total > 0.0 {
                    progress((us / 1e6 / total).clamp(0.0, 1.0));
                }
            }
        }
    }
    let status = child.wait().map_err(|e| format!("FFmpeg stopped unexpectedly: {e}"))?;
    let _ = err_thread.join();
    if procs::cancelled_here() {
        Err(procs::CANCELLED.into())
    } else if status.success() {
        Ok(())
    } else {
        let text = stderr_text.lock().map(|t| t.clone()).unwrap_or_default();
        Err(tool_error("FFmpeg", &text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("convertino-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn run_target(target: &str, inputs: &[PathBuf]) -> Vec<PathBuf> {
        plan(target, inputs)
            .unwrap_or_else(|e| panic!("{target}: {e}"))
            .iter()
            .map(|step| {
                let mut last = 0.0;
                let out = run(step, &mut |p| last = p).unwrap_or_else(|e| panic!("{target}: {e}"));
                assert!((last - 1.0).abs() < 1e-9, "{target} progress ended at {last}");
                out
            })
            .collect()
    }

    #[test]
    fn plain_names_for_picky_tools() {
        let dir = std::env::temp_dir().join(format!("convertino-plain-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("Café – मेनू.pdf");
        std::fs::write(&input, b"%PDF-1.4 test").unwrap();
        let output = dir.join("Café – मेनू (compressed).pdf");
        let plain = Plain::when(true, [input.as_path(), output.as_path()]).unwrap();
        let src = plain.input(&input, 0).unwrap();
        let out = plain.output(&output);
        assert!(plain_name(&src) && plain_name(&out), "{src:?} {out:?}");
        assert_eq!(std::fs::read(&src).unwrap(), b"%PDF-1.4 test");
        std::fs::write(&out, b"result").unwrap();
        plain.finish(&out, &output).unwrap();
        assert_eq!(std::fs::read(&output).unwrap(), b"result");
        // Plain names are used as they are.
        let ascii = dir.join("plain.pdf");
        let none = Plain::when(true, [ascii.as_path()]).unwrap();
        assert_eq!(none.input(&ascii, 0).unwrap(), ascii);
        drop(plain);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn quality_settings_reach_the_converters() {
        let q = Quality { jpg: 72, resize: 1280, mp3: 192, dpi: 300, pdf_compress: "small".into(), convert_max: 2560, ..Quality::default() };
        let f = |name: &str| vec![PathBuf::from(format!("/tmp/{name}"))];
        let args = |steps: Vec<Step>| match &steps[0].op {
            Op::Magick { args, .. } | Op::Png { args, .. } | Op::Ghostscript { args, .. } => args.join(" "),
            Op::Audio { args, mp3 } => format!("{} mp3={mp3}", args.join(" ")),
            Op::Picture { prep, format, level, fixed } => format!("{} {format:?} {level:?} fixed={fixed}", prep.join(" ")),
            Op::PdfCompress { level } => format!("pdfcompress {level:?}"),
            other => format!("{other:?}"),
        };
        let jpg = args(plan_with("image.jpg", &f("a.png"), &q).unwrap());
        assert!(jpg.contains("fixed=72") && jpg.contains("-resize 2560x2560>"), "{jpg}");
        // Compress to a size: the size names the file, the step carries it.
        let ask = crate::size::Ask { bytes: 2 * crate::size::MB, together: false, trim: None };
        let (sized, _) = plan_sized("video.compress", &f("a.mp4"), &q, Some(ask)).unwrap_or_default();
        assert!(sized.is_empty(), "missing files can't be measured");
        assert!(args(plan_with("audio.mp3", &f("a.wav"), &q).unwrap()).contains("mp3=192"));
        assert!(args(plan_with("pdf.compress", &f("a.pdf"), &q).unwrap()).contains("Small"));
        match &plan_with("pdf.jpg", &f("a.pdf"), &q).unwrap()[0].op {
            Op::PdfPages { dpi, quality, .. } => assert_eq!((*dpi, *quality), (300, 72)),
            other => panic!("{other:?}"),
        }
        // Defaults are what Convertino always did.
        let d = Quality::default();
        let jpg = args(plan_with("image.jpg", &f("a.png"), &d).unwrap());
        assert!(jpg.contains("Some(Balanced)") && !jpg.contains("-resize"), "{jpg}");
        assert!(args(plan_with("audio.mp3", &f("a.wav"), &d).unwrap()).contains("mp3=320"));
        let fixed = Quality { image: "fixed".into(), ..Quality::default() };
        assert!(args(plan_with("image.webp", &f("a.png"), &fixed).unwrap()).contains("None fixed=85"));
    }

    #[test]
    fn audio_never_needs_more_bits_than_the_source() {
        let mp3 = |kbps, cap| audio_codec_for("mp3", kbps, cap).join(" ");
        assert!(mp3(320, None).contains("-q:a 0"));
        assert!(mp3(192, None).contains("-q:a 2"));
        // A 128 kbps source: V5 (about 130), not V0.
        assert!(mp3(320, Some(140)).contains("-q:a 5"), "{}", mp3(320, Some(140)));
        assert!(audio_codec_for("m4a", 320, Some(110)).contains(&"110k".to_string()));
        assert!(audio_codec_for("flac", 320, None).contains(&"8".to_string()));
    }

    #[test]
    fn unique_names_never_overwrite() {
        let d = tmpdir("unique");
        let a = unique_output(&d, "song", "mp3");
        assert_eq!(a.file_name().unwrap(), "song.mp3");
        std::fs::write(&a, b"x").unwrap();
        let b = unique_output(&d, "song", "mp3");
        let expected = if cfg!(target_os = "macos") { "song 2.mp3" } else { "song (1).mp3" };
        assert_eq!(b.file_name().unwrap(), expected);
        std::fs::create_dir(d.join("pages")).unwrap();
        let c = unique_output(&d, "pages", "");
        let expected = if cfg!(target_os = "macos") { "pages 2" } else { "pages (1)" };
        assert_eq!(c.file_name().unwrap(), expected);
    }

    #[test]
    fn running_jobs_never_share_an_output_name() {
        let d = tmpdir("reserve");
        let a = Reserved::new(&d, "photo", "webp");
        let b = Reserved::new(&d, "photo", "webp");
        assert_ne!(a.0, b.0);
        drop(a);
        assert_eq!(Reserved::new(&d, "photo", "webp").0.file_name().unwrap(), "photo.webp");
    }

    #[test]
    fn a_deleted_source_is_explained() {
        let gone = tmpdir("gone").join("vanished.png");
        let step = plan("image.jpg", &[gone]).unwrap().remove(0);
        assert!(run(&step, &mut |_| {}).unwrap_err().contains("no longer there"));
    }

    #[test]
    fn duration_is_parsed() {
        assert_eq!(parse_duration("  Duration: 00:03:36.50, start: 0.0"), Some(216.5));
        assert_eq!(parse_duration("nothing"), None);
    }

    #[test]
    fn unsupported_targets_explain_themselves() {
        assert!(plan("video.nope", &[PathBuf::from("a.mov")]).unwrap_err().contains("later update"));
        assert!(plan("doc.md", &[PathBuf::from("a.pptx")]).unwrap_err().contains(".pptx"));
    }


    #[test]
    fn document_routes() {
        assert!(matches!(doc_op("pdf", "docx"), Some(Op::Office { .. })));
        assert!(matches!(doc_op("pdf", "md"), Some(Op::PandocThenOffice { .. })));
        assert!(matches!(doc_op("md", "doc"), Some(Op::OfficeThenPandoc { .. })));
        assert!(matches!(doc_op("md", "docx"), Some(Op::Pandoc { .. })));
        assert!(doc_op("md", "pptx").is_none());
    }

    #[test]
    fn merge_sorts_by_name_and_makes_one_step() {
        let steps = plan("pdf.merge", &[PathBuf::from("/x/b.pdf"), PathBuf::from("/x/A.pdf")]).unwrap();
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].inputs[0], PathBuf::from("/x/A.pdf"));
        assert_eq!(steps[0].stem, "Merged");
    }

    /// Real conversions, run for whichever tools are installed on this machine.
    #[test]
    fn real_conversions_when_tools_exist() {
        let d = tmpdir("real");
        if let Some(ff) = tools::find(Tool::Ffmpeg) {
            let wav = d.join("tone.wav");
            assert!(tools::command(&ff)
                .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "sine=frequency=440:duration=2"])
                .arg(&wav)
                .status()
                .unwrap()
                .success());
            for target in ["audio.mp3", "audio.flac", "audio.opus", "audio.m4a", "audio.trim"] {
                let out = run_target(target, &[wav.clone()]);
                assert!(out[0].metadata().unwrap().len() > 0, "{target} wrote nothing");
            }
        }
        if let Some(im) = tools::find(Tool::Magick) {
            let png = d.join("pic.png");
            assert!(tools::command(&im).args(["-size", "64x48", "gradient:red-blue"]).arg(&png).status().unwrap().success());
            for target in ["image.jpg", "image.webp", "image.ico", "image.compress", "image.gif"] {
                run_target(target, &[png.clone()]);
            }
        }
    }

    /// Every video target, when FFmpeg is installed.
    #[test]
    fn real_video_conversions() {
        let Some(ff) = tools::find(Tool::Ffmpeg) else { return };
        let d = tmpdir("video");
        // An old-style AVI (MPEG-4 Part 2 + MP3) forces real encoding; a sharp,
        // high-bitrate picture makes sure Compress has something to save.
        let avi = d.join("clip.avi");
        assert!(tools::command(&ff)
            .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc2=size=1920x1080:rate=30:duration=2"])
            .args(["-f", "lavfi", "-i", "sine=frequency=330:duration=2"])
            .args(["-c:v", "mpeg4", "-q:v", "1", "-c:a", "libmp3lame", "-shortest"])
            .arg(&avi)
            .status()
            .unwrap()
            .success());
        for target in ["video.mp4", "video.webm", "video.gif", "video.compress", "video.720p", "video.mkv", "video.mov", "video.mp3"] {
            let out = run_target(target, &[avi.clone()]).remove(0);
            assert!(out.metadata().unwrap().len() > 1000, "{target} wrote almost nothing");
        }
        let frames = run_target("video.frames", &[avi.clone()]).remove(0);
        assert!(files_in(&frames).unwrap().len() >= 2);

        // An H.264 MP4 goes to MOV and MKV without re-encoding.
        let mp4 = d.join("clip.mp4");
        let mov = run_target("video.mov", &[mp4.clone()]).remove(0);
        let p = video::probe(&mov).unwrap();
        assert_eq!((p.width, p.height), (1920, 1080));
        let small = run_target("video.720p", &[mp4.clone()]).remove(0);
        assert_eq!(video::probe(&small).unwrap().height, 720);
        let step = plan("video.720p", &[small.clone()]).unwrap().remove(0);
        assert!(run(&step, &mut |_| {}).unwrap_err().contains("720p or smaller"));
    }

    /// Data targets through plan() and run(), as the wheel runs them (archives: archive.rs).
    #[test]
    fn real_data_conversions() {
        let d = tmpdir("data");
        let csv = d.join("orders.csv");
        std::fs::write(&csv, "id,item,price\n1,Case,499\n2,Strap,199.5\n").unwrap();
        let json = run_target("data.json", &[csv.clone()]).remove(0);
        assert!(std::fs::read_to_string(&json).unwrap().contains("\"price\": 199.5"));
        let yaml = run_target("data.yaml", &[json.clone()]).remove(0);
        assert_eq!(yaml.extension().unwrap(), "yaml");
        run_target("data.xlsx", &[yaml.clone()]);
        let back = run_target("data.csv", &[yaml]).remove(0);
        assert!(std::fs::read_to_string(&back).unwrap().contains("2,Strap,199.5"));

    }

    /// PDF and document conversions, when Poppler / Ghostscript / LibreOffice / Pandoc are installed.
    #[test]
    fn real_pdf_and_document_conversions() {
        let d = tmpdir("docs");
        let md = d.join("notes.md");
        std::fs::write(&md, "# Title\n\nSome *text*.\n\n- one\n- two\n\n\\newpage\n\nPage two.\n").unwrap();

        if tools::find(Tool::Pandoc).is_some() {
            let docx = run_target("doc.docx", &[md.clone()]).remove(0);
            assert_eq!(docx.extension().unwrap(), "docx");
            let md_back = run_target("doc.md", &[docx.clone()]).remove(0);
            assert!(std::fs::read_to_string(&md_back).unwrap().contains("Title"));
            run_target("doc.html", &[md.clone()]);
            run_target("doc.odt", &[md.clone()]);
        }
        let office = tools::find(Tool::Pandoc).is_some() && tools::find(Tool::LibreOffice).is_some();
        let pdf = if office {
            let docx = run_target("doc.docx", &[md.clone()]).remove(0);
            run_target("doc.odt", &[docx.clone()]);
            run_target("doc.txt", &[docx.clone()]);
            run_target("doc.pdf", &[md.clone()]);
            run_target("doc.pdf", &[docx]).remove(0)
        } else if let Some(im) = tools::find(Tool::Magick) {
            // No LibreOffice: make a two-page PDF from images so the PDF tools still get tested.
            let (a, b) = (d.join("p1.png"), d.join("p2.png"));
            for (p, c) in [(&a, "gradient:red-blue"), (&b, "gradient:green-white")] {
                assert!(tools::command(&im).args(["-size", "200x280", c]).arg(p).status().unwrap().success());
            }
            run_target("image.pdf", &[a, b]).remove(0)
        } else {
            return;
        };
        assert_eq!(pdf.extension().unwrap(), "pdf");

        if tools::find(Tool::Pdftoppm).is_some() {
            let pages = run_target("pdf.jpg", &[pdf.clone()]).remove(0);
            assert!(pages.exists());
            if office {
                let txt = run_target("pdf.txt", &[pdf.clone()]).remove(0);
                assert!(std::fs::read_to_string(&txt).unwrap().contains("Title"));
            } else {
                // Pages made from images have no text to extract.
                let step = plan("pdf.txt", &[pdf.clone()]).unwrap().remove(0);
                assert!(run(&step, &mut |_| {}).unwrap_err().contains("no selectable text"));
            }
            run_target("pdf.split", &[pdf.clone()]);
            let pdf2 = d.join("second.pdf");
            std::fs::copy(&pdf, &pdf2).unwrap();
            let merged = run_target("pdf.merge", &[pdf.clone(), pdf2]).remove(0);
            assert_eq!(merged.file_name().unwrap(), "Merged.pdf");
            if tools::find(Tool::Magick).is_some() {
                run_target("pdf.webp", &[pdf.clone()]);
                run_target("pdf.tiff", &[pdf.clone()]);
            }
        }
        if tools::find(Tool::Ghostscript).is_some() {
            run_target("pdf.grayscale", &[pdf.clone()]);
            if let Some(im) = tools::find(Tool::Magick) {
                // A photo stored at 600 DPI shrinks a lot at the /ebook setting.
                let big = d.join("scan.pdf");
                assert!(tools::command(&im)
                    .args(["-size", "2400x3000", "plasma:", "-quality", "95", "-density", "600"])
                    .arg(&big)
                    .status()
                    .unwrap()
                    .success());
                let small = run_target("pdf.compress", &[big.clone()]).remove(0);
                assert!(small.metadata().unwrap().len() < big.metadata().unwrap().len() / 2);
            }
            // Re-compressing a compressed PDF gains nothing: explained, and no file left behind.
            let again = plan("pdf.compress", &[pdf.clone()]).unwrap();
            if let Err(e) = run(&again[0], &mut |_| {}) {
                assert!(e.contains("already compact"), "{e}");
                assert!(!again[0].dir.join(format!("{}.pdf", again[0].stem)).exists());
            }
        }
    }
}
