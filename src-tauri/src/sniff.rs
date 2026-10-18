//! Looking inside files before the wheel opens: what an archive holds
//! (without unpacking it), what a file really is (its first bytes, not its
//! extension), a video's sound and subtitle tracks, a CSV's text and a PDF's
//! pictures. Everything here only reads; smart.rs decides what to offer.
//!
//! The wheel waits for this, so each look is quick: zips are read in place
//! (zips inside them too, up to a size limit), other archives are listed by
//! 7-Zip when it's installed, videos are described by FFmpeg when it's
//! installed. Nothing is downloaded just to look.

use crate::tools::{self, Tool};
use std::io::{Cursor, Read, Seek};
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// Inner zips bigger than this aren't read into memory to look inside.
const INNER_LIMIT: u64 = 256 * 1024 * 1024;
/// How deep zips inside zips are looked into.
const MAX_DEPTH: usize = 3;

/// Archive extensions (as the Archive family lists them, plus multi-part names).
pub const ARCHIVE_EXTS: [&str; 10] = ["zip", "7z", "rar", "tar", "gz", "tgz", "bz2", "xz", "tbz2", "txz"];

pub fn is_archive_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    ARCHIVE_EXTS.iter().any(|e| lower.ends_with(&format!(".{e}")))
}

/// One file or folder inside an archive.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// Its path inside the (innermost) archive, with `/`.
    pub path: String,
    pub size: u64,
    pub dir: bool,
    /// Inside zips inside the archive: their paths, outermost first.
    pub within: Vec<String>,
}

impl Entry {
    /// The last part of its path ("Satoshi-Bold.otf").
    pub fn name(&self) -> &str {
        self.path.trim_end_matches('/').rsplit('/').next().unwrap_or(&self.path)
    }
    pub fn ext(&self) -> String {
        self.name().rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default()
    }
    /// The path a person reads ("Fonts.zip › Satoshi/Satoshi-Bold.otf").
    pub fn shown(&self) -> String {
        let mut parts = self.within.clone();
        parts.push(self.path.clone());
        parts.join(" › ")
    }
}

/// What an archive holds.
#[derive(Debug, Clone, Default)]
pub struct Listing {
    /// Every file and folder, at every level that could be read.
    pub entries: Vec<Entry>,
    /// Archives inside it (at every level that could be read).
    pub nested: usize,
    /// Some archives inside couldn't be looked into (not a zip, too big).
    pub partial: bool,
}

impl Listing {
    /// Entries at the top level (not inside an inner archive).
    pub fn top(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|e| e.within.is_empty())
    }
}

fn is_zip(bytes_or_path: &Path) -> bool {
    let mut b = [0u8; 4];
    std::fs::File::open(bytes_or_path).and_then(|mut f| f.read_exact(&mut b)).is_ok() && &b == b"PK\x03\x04"
}

/// Lists an archive, or None when it can't be read without unpacking it
/// (7-Zip not installed yet, a damaged or protected archive).
pub fn list(archive: &Path) -> Option<Listing> {
    if is_zip(archive) {
        let f = std::fs::File::open(archive).ok()?;
        let mut zip = zip::ZipArchive::new(std::io::BufReader::new(f)).ok()?;
        let mut out = Listing::default();
        let mut budget = 2 * INNER_LIMIT;
        walk_zip(&mut zip, &[], 0, &mut out, &mut budget);
        return Some(out);
    }
    let seven = tools::find(Tool::SevenZip)?;
    let out = tools::command(&seven)
        .args(["l", "-slt", "-ba", "-p_convertino_", "-sccUTF-8"])
        .arg(archive)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut listing = Listing::default();
    let lower = archive.to_string_lossy().to_lowercase();
    // A .tar.gz lists the .tar inside it: that's one archive, not two.
    let wrapper = [".gz", ".tgz", ".bz2", ".tbz2", ".xz", ".txz"].iter().any(|e| lower.ends_with(e));
    for block in text.replace("\r\n", "\n").split("\n\n") {
        let field = |k: &str| block.lines().find_map(|l| l.strip_prefix(&format!("{k} = ")).map(str::to_string));
        let Some(path) = field("Path") else { continue };
        let dir = field("Folder").as_deref() == Some("+") || field("Attributes").is_some_and(|a| a.starts_with('D'));
        let size = field("Size").and_then(|s| s.parse().ok()).unwrap_or(0);
        let path = path.replace('\\', "/");
        let e = Entry { path, size, dir, within: Vec::new() };
        if !e.dir && is_archive_name(e.name()) && !(wrapper && e.ext() == "tar") {
            listing.nested += 1;
            listing.partial = true;
        }
        listing.entries.push(e);
    }
    Some(listing)
}

fn walk_zip<R: Read + Seek>(zip: &mut zip::ZipArchive<R>, within: &[String], depth: usize, out: &mut Listing, budget: &mut u64) {
    let mut inner = Vec::new();
    for i in 0..zip.len() {
        let Ok(f) = zip.by_index_raw(i) else { continue };
        let path = f.name().replace('\\', "/");
        // Mac zips carry a shadow copy of every file's extra data.
        if path.starts_with("__MACOSX/") || path.rsplit('/').next().is_some_and(|n| n.starts_with("._")) {
            continue;
        }
        let e = Entry { path, size: f.size(), dir: f.is_dir(), within: within.to_vec() };
        if !e.dir && is_archive_name(e.name()) {
            out.nested += 1;
            inner.push((i, e.clone()));
        }
        out.entries.push(e);
    }
    for (i, e) in inner {
        let zipped = e.ext() == "zip";
        if !zipped || depth + 1 >= MAX_DEPTH || e.size > INNER_LIMIT || e.size > *budget {
            out.partial = true;
            continue;
        }
        let Ok(mut f) = zip.by_index(i) else { out.partial = true; continue };
        let mut bytes = Vec::with_capacity(e.size as usize);
        if f.read_to_end(&mut bytes).is_err() {
            out.partial = true;
            continue;
        }
        *budget = budget.saturating_sub(e.size);
        match zip::ZipArchive::new(Cursor::new(bytes)) {
            Ok(mut z) => {
                let mut w = within.to_vec();
                w.push(e.path.clone());
                walk_zip(&mut z, &w, depth + 1, out, budget);
            }
            Err(_) => out.partial = true,
        }
    }
}

/// The bytes of one entry (zips: read in place; other archives: 7-Zip writes it to us).
pub fn read(archive: &Path, e: &Entry) -> Result<Vec<u8>, String> {
    let damaged = |x: &dyn std::fmt::Display| format!("Couldn't read {} from {}: {x}", e.shown(), crate::engine::file_name(archive));
    if is_zip(archive) {
        let f = std::fs::File::open(archive).map_err(|x| damaged(&x))?;
        let mut zip = zip::ZipArchive::new(std::io::BufReader::new(f)).map_err(|x| damaged(&x))?;
        if e.within.is_empty() {
            return read_from(&mut zip, &e.path).map_err(|x| damaged(&x));
        }
        let mut bytes = read_from(&mut zip, &e.within[0]).map_err(|x| damaged(&x))?;
        for w in &e.within[1..] {
            let mut z = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|x| damaged(&x))?;
            bytes = read_from(&mut z, w).map_err(|x| damaged(&x))?;
        }
        let mut z = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|x| damaged(&x))?;
        return read_from(&mut z, &e.path).map_err(|x| damaged(&x));
    }
    if !e.within.is_empty() {
        return Err(damaged(&"it's inside another archive"));
    }
    let seven = tools::require(Tool::SevenZip)?;
    let out = tools::command(&seven)
        .args(["e", "-so", "-p_convertino_"])
        .arg(archive)
        .arg("--")
        .arg(&e.path)
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|x| damaged(&x))?;
    if !out.status.success() {
        return Err(damaged(&crate::convert::last_lines(&String::from_utf8_lossy(&out.stderr))));
    }
    Ok(out.stdout)
}

fn read_from<R: Read + Seek>(zip: &mut zip::ZipArchive<R>, name: &str) -> Result<Vec<u8>, String> {
    let mut f = zip.by_name(name).map_err(|e| e.to_string())?;
    let mut v = Vec::with_capacity(f.size() as usize);
    f.read_to_end(&mut v).map_err(|e| e.to_string())?;
    Ok(v)
}

// ---------- what a file really is ----------

fn head(path: &Path, n: usize) -> Vec<u8> {
    let mut v = vec![0u8; n];
    let got = std::fs::File::open(path).and_then(|mut f| {
        let mut total = 0;
        loop {
            let k = f.read(&mut v[total..])?;
            if k == 0 || total + k == n {
                return Ok(total + k);
            }
            total += k;
        }
    });
    v.truncate(got.unwrap_or(0));
    v
}

/// Extensions that name the same format (a .jpeg is a .jpg).
fn same_kind(a: &str, b: &str) -> bool {
    const GROUPS: [&[&str]; 9] = [
        &["jpg", "jpeg", "jfif", "jpe"],
        &["tif", "tiff"],
        &["heic", "heif"],
        &["mp4", "m4v", "mov", "3gp", "3g2", "qt"],
        &["m4a", "m4b", "mp4", "aac"],
        &["aif", "aiff"],
        &["ogg", "oga", "opus", "ogv"],
        &["htm", "html"],
        &["mkv", "mka", "mk3d"],
    ];
    a == b || GROUPS.iter().any(|g| g.contains(&a) && g.contains(&b))
}

/// What the file really is, by its first bytes, when that's surely known.
pub fn real_ext(path: &Path) -> Option<&'static str> {
    let b = head(path, 4096);
    let at = |i: usize, s: &[u8]| b.get(i..i + s.len()) == Some(s);
    if at(0, b"\xFF\xD8\xFF") {
        return Some("jpg");
    }
    if at(0, b"\x89PNG\r\n\x1a\n") {
        return Some("png");
    }
    if at(0, b"GIF87a") || at(0, b"GIF89a") {
        return Some("gif");
    }
    if at(0, b"RIFF") {
        return match b.get(8..12)? {
            b"WEBP" => Some("webp"),
            b"WAVE" => Some("wav"),
            b"AVI " => Some("avi"),
            _ => None,
        };
    }
    if at(4, b"ftyp") {
        let major = b.get(8..12)?;
        let box_len = u32::from_be_bytes(b.get(0..4)?.try_into().ok()?) as usize;
        let compat: Vec<&[u8]> = b.get(16..box_len.min(b.len()))?.chunks(4).collect();
        let has = |x: &[u8]| major == x || compat.contains(&x);
        return if has(b"avif") || has(b"avis") {
            Some("avif")
        } else if has(b"heic") || has(b"heix") || has(b"hevc") || has(b"heim") || has(b"heis") {
            Some("heic")
        } else if major == b"crx " {
            None // Canon CR3: a RAW photo, leave its name alone
        } else if major == b"qt  " {
            Some("mov")
        } else if major == b"M4A " || major == b"M4B " {
            Some("m4a")
        } else if [b"isom", b"iso2", b"iso4", b"iso5", b"iso6", b"mp41", b"mp42", b"avc1", b"dash", b"M4V ", b"mmp4", b"MSNV"].iter().any(|x| major == *x) {
            Some("mp4")
        } else {
            None
        };
    }
    if at(0, b"\x1A\x45\xDF\xA3") {
        let webm = b.windows(4).any(|w| w == b"webm");
        return Some(if webm { "webm" } else { "mkv" });
    }
    if at(0, b"%PDF-") {
        return Some("pdf");
    }
    if at(0, b"PK\x03\x04") {
        return zip_kind(path);
    }
    if at(0, b"Rar!\x1A\x07") {
        return Some("rar");
    }
    if at(0, b"7z\xBC\xAF\x27\x1C") {
        return Some("7z");
    }
    if at(0, b"fLaC") {
        return Some("flac");
    }
    if at(0, b"OggS") {
        return Some(if b.windows(8).any(|w| w == b"OpusHead") { "opus" } else { "ogg" });
    }
    if at(0, b"FORM") && (at(8, b"AIFF") || at(8, b"AIFC")) {
        return Some("aiff");
    }
    if at(0, b"ID3") {
        return Some("mp3");
    }
    if b.len() > 262 && at(257, b"ustar") {
        return Some("tar");
    }
    if at(0, b"BM") {
        let declared = u32::from_le_bytes(b.get(2..6)?.try_into().ok()?) as u64;
        let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        return (declared == len).then_some("bmp");
    }
    None
}

/// A zip that's really a Word, Excel or PowerPoint file (or just a zip).
fn zip_kind(path: &Path) -> Option<&'static str> {
    let f = std::fs::File::open(path).ok()?;
    let zip = zip::ZipArchive::new(std::io::BufReader::new(f)).ok()?;
    let names: Vec<&str> = zip.file_names().collect();
    let has = |p: &str| names.iter().any(|n| n.starts_with(p));
    if names.contains(&"[Content_Types].xml") {
        if has("word/") {
            return Some("docx");
        }
        if has("xl/") {
            return Some("xlsx");
        }
        if has("ppt/") {
            return Some("pptx");
        }
        return None;
    }
    if names.contains(&"mimetype") {
        return None; // OpenDocument and EPUB: their own extensions, leave them
    }
    Some("zip")
}

/// The extension this file should have, when its name says something else.
/// Only for files Convertino knows (or with no extension), never camera RAW
/// files (most are TIFFs inside) or fonts.
pub fn wrong_ext(path: &Path) -> Option<&'static str> {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if crate::raw::is_raw(path) || crate::fonts::is_font_name(&path.to_string_lossy()) {
        return None;
    }
    let known = ext.is_empty() || crate::wheel::family_of(&ext).is_some();
    if !known || !path.is_file() {
        return None;
    }
    let real = real_ext(path)?;
    // A .webm with something other than WebM inside: MKV; the reverse is fine (WebM is MKV).
    if ext == "mkv" && real == "webm" {
        return None;
    }
    // Office files and other zip-based formats with their own names are zips inside.
    if real == "zip" && !ext.is_empty() && ext != "zip" {
        return None;
    }
    (!same_kind(&ext, real)).then_some(real)
}

// ---------- a video's tracks ----------

#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    /// Its number among tracks of its kind (0 for the first sound track).
    pub index: usize,
    pub codec: String,
    pub language: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Tracks {
    pub audio: Vec<Track>,
    pub subtitles: Vec<Track>,
}

/// Handler names that say nothing ("SoundHandler").
fn generic_title(t: &str) -> bool {
    let l = t.to_lowercase();
    l.is_empty() || l.contains("handler") || l.contains("media file produced") || l == "stereo" || l == "mono" || l.starts_with("core media") || l == "und"
}

/// The sound and subtitle tracks, read from FFmpeg's description (None when
/// FFmpeg isn't installed yet: nothing is downloaded just to look).
pub fn tracks(path: &Path) -> Option<Tracks> {
    let ff = tools::find(Tool::Ffmpeg)?;
    let out = tools::command(&ff).args(["-hide_banner", "-nostdin", "-i"]).arg(path).stdin(Stdio::null()).stdout(Stdio::null()).output().ok()?;
    Some(parse_tracks(&String::from_utf8_lossy(&out.stderr)))
}

pub fn parse_tracks(text: &str) -> Tracks {
    let mut t = Tracks::default();
    // Which list the metadata lines below a stream belong to.
    let mut current: Option<(bool, usize)> = None;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with("Stream #") {
            current = None;
            let lang = l.split_once('(').and_then(|(_, r)| r.split_once(')')).map(|(x, _)| x.to_string()).filter(|x| x.len() == 3 && x != "und");
            let codec = |kind: &str| l.split_once(kind).map(|(_, r)| r.split([',', ' ']).next().unwrap_or("").to_string()).unwrap_or_default();
            if l.contains(": Audio: ") {
                t.audio.push(Track { index: t.audio.len(), codec: codec(": Audio: "), language: lang, title: None });
                current = Some((true, t.audio.len() - 1));
            } else if l.contains(": Subtitle: ") {
                t.subtitles.push(Track { index: t.subtitles.len(), codec: codec(": Subtitle: "), language: lang, title: None });
                current = Some((false, t.subtitles.len() - 1));
            }
            continue;
        }
        // A stream's own lines are indented further ("    Metadata:", "      title : …").
        if !line.starts_with("    ") {
            current = None;
        }
        let Some((audio, i)) = current else { continue };
        if let Some((k, v)) = l.split_once(':') {
            let (k, v) = (k.trim(), v.trim());
            if (k == "title" || k == "handler_name") && !generic_title(v) {
                let list = if audio { &mut t.audio } else { &mut t.subtitles };
                if k == "title" || list[i].title.is_none() {
                    list[i].title = Some(v.to_string());
                }
            }
        }
    }
    t
}

/// Subtitles stored as text can become SRT; picture ones (Blu-ray, DVD) can't.
pub fn text_subtitle(codec: &str) -> bool {
    matches!(codec, "subrip" | "srt" | "ass" | "ssa" | "mov_text" | "webvtt" | "text" | "microdvd" | "mpl2" | "jacosub" | "realtext" | "sami" | "stl" | "subviewer" | "subviewer1" | "vplayer")
}

// ---------- a CSV that Excel would garble ----------

#[derive(Debug, Clone, PartialEq)]
pub struct CsvLook {
    /// Characters outside plain English that Excel shows wrong without a BOM ("₹ é ह").
    pub sample: Vec<char>,
    /// Separated by semicolons (Excel in most places expects commas).
    pub semicolons: bool,
}

pub fn csv_look(path: &Path) -> Option<CsvLook> {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if ext != "csv" {
        return None;
    }
    let b = head(path, 1 << 20);
    if b.starts_with(b"\xEF\xBB\xBF") || b.starts_with(b"\xFF\xFE") || b.starts_with(b"\xFE\xFF") {
        return None; // already marked: Excel reads it right
    }
    // A cut in the middle of a character at the 1 MB mark is fine.
    let text = match std::str::from_utf8(&b) {
        Ok(t) => t,
        Err(e) if e.valid_up_to() + 4 >= b.len() => std::str::from_utf8(&b[..e.valid_up_to()]).ok()?,
        Err(_) => return None,
    };
    let mut sample = Vec::new();
    for c in text.chars().filter(|c| !c.is_ascii() && !c.is_whitespace()) {
        if !sample.contains(&c) {
            sample.push(c);
        }
        if sample.len() == 3 {
            break;
        }
    }
    let first = text.lines().next().unwrap_or("");
    let semicolons = first.contains(';') && !first.contains(',') && !first.contains('\t');
    (!sample.is_empty() || semicolons).then_some(CsvLook { sample, semicolons })
}

// ---------- a PDF's pictures ----------

/// Pictures smaller than this (icons, bullets, bits of a logo) are left out.
pub const MIN_PICTURE: u32 = 64;

/// One row of `pdfimages -list`.
#[derive(Debug, Clone, PartialEq)]
pub struct PdfImage {
    pub num: usize,
    pub kind: String,
    pub width: u32,
    pub height: u32,
}

impl PdfImage {
    pub fn kept(&self) -> bool {
        self.kind == "image" && self.width >= MIN_PICTURE && self.height >= MIN_PICTURE
    }
}

pub fn parse_pdfimages(text: &str) -> Vec<PdfImage> {
    text.lines()
        .filter_map(|l| {
            let c: Vec<&str> = l.split_whitespace().collect();
            // page num type width height color comp bpc enc interp object ID x-ppi y-ppi size ratio
            if c.len() < 5 {
                return None;
            }
            Some(PdfImage { num: c[1].parse().ok()?, kind: c[2].to_string(), width: c[3].parse().ok()?, height: c[4].parse().ok()? })
        })
        .collect()
}

pub fn pdfimages_exe() -> Option<PathBuf> {
    tools::find(Tool::Pdfimages)
}

/// The PDF's pictures worth saving (None: Poppler isn't installed yet, or
/// the PDF is too big to list while the wheel waits).
pub fn pdf_pictures(path: &Path) -> Option<Vec<PdfImage>> {
    let exe = pdfimages_exe()?;
    if std::fs::metadata(path).map(|m| m.len()).unwrap_or(0) > 80 * 1024 * 1024 {
        return None;
    }
    let out = tools::command(&exe).arg("-list").arg(path).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| parse_pdfimages(&String::from_utf8_lossy(&out.stdout)))
}

// ---------- archives in several parts ----------

/// "Footage.zip.003" -> ("Footage.zip", 3); "Footage.part2.rar" -> ("Footage", 2).
pub fn part_of(path: &Path) -> Option<(String, usize, PartStyle)> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    let lower = name.to_lowercase();
    if let Some((base, num)) = lower.rsplit_once('.') {
        if num.len() == 3 && num.chars().all(|c| c.is_ascii_digit()) && base.contains('.') {
            let n: usize = num.parse().ok()?;
            return (n >= 1).then(|| (name[..base.len()].to_string(), n, PartStyle::Numbered));
        }
    }
    if lower.ends_with(".rar") {
        let stem = &lower[..lower.len() - 4];
        if let Some((base, num)) = stem.rsplit_once(".part") {
            if !num.is_empty() && num.chars().all(|c| c.is_ascii_digit()) {
                return Some((name[..base.len()].to_string(), num.parse().ok()?, PartStyle::Rar(num.len())));
            }
        }
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PartStyle {
    /// name.zip.001, .002 …
    Numbered,
    /// name.part1.rar (the digits are padded to this width).
    Rar(usize),
}

/// The parts found next to one of them: the paths in order, and the first
/// missing number when there's a gap (the parts after it can't be joined).
pub fn parts(path: &Path) -> Option<(String, Vec<PathBuf>, Option<usize>)> {
    let (base, _, style) = part_of(path)?;
    let dir = path.parent()?.to_path_buf();
    let name_of = |n: usize| match style {
        PartStyle::Numbered => format!("{base}.{n:03}"),
        PartStyle::Rar(w) => format!("{base}.part{n:0w$}.rar"),
    };
    // The highest number present (any part), then which below it are missing.
    let mut highest = 0;
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            if let Some((b, n, s)) = part_of(&e.path()) {
                if b.eq_ignore_ascii_case(&base) && s == style {
                    highest = highest.max(n);
                }
            }
        }
    }
    let mut found = Vec::new();
    let mut missing = None;
    for n in 1..=highest {
        let p = dir.join(name_of(n));
        if p.is_file() {
            found.push(p);
        } else if missing.is_none() {
            missing = Some(n);
        }
    }
    let shown = match style {
        PartStyle::Numbered => base.rsplit_once('.').map(|(b, _)| b.to_string()).unwrap_or(base.clone()),
        PartStyle::Rar(_) => base.clone(),
    };
    Some((shown, found, missing))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("convertino-sniff-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    pub(crate) fn make_zip(path: &Path, files: &[(&str, &[u8])]) {
        let f = std::fs::File::create(path).unwrap();
        let mut z = zip::ZipWriter::new(f);
        for (name, data) in files {
            if name.ends_with('/') {
                z.add_directory(name.trim_end_matches('/'), zip::write::SimpleFileOptions::default()).unwrap();
            } else {
                z.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
                std::io::Write::write_all(&mut z, data).unwrap();
            }
        }
        z.finish().unwrap();
    }

    #[test]
    fn lists_zips_inside_zips_without_unpacking() {
        let d = dir("nested");
        let inner = d.join("inner.zip");
        make_zip(&inner, &[("deep/a.txt", b"a")]);
        let mid = d.join("mid.zip");
        make_zip(&mid, &[("inner.zip", &std::fs::read(&inner).unwrap()), ("b.txt", b"b")]);
        let outer = d.join("Pack.zip");
        make_zip(&outer, &[("mid.zip", &std::fs::read(&mid).unwrap()), ("readme.txt", b"r"), ("__MACOSX/._readme.txt", b"x")]);
        let l = list(&outer).unwrap();
        assert_eq!(l.nested, 2);
        assert!(!l.partial);
        let deep = l.entries.iter().find(|e| e.path == "deep/a.txt").unwrap();
        assert_eq!(deep.within, vec!["mid.zip".to_string(), "inner.zip".to_string()]);
        assert_eq!(read(&outer, deep).unwrap(), b"a");
        assert!(l.entries.iter().all(|e| !e.path.starts_with("__MACOSX")));
        assert_eq!(deep.shown(), "mid.zip › inner.zip › deep/a.txt");
    }

    #[test]
    fn knows_what_a_file_really_is() {
        let d = dir("real");
        let webp = d.join("hero-banner.jpg");
        std::fs::write(&webp, b"RIFF\x10\x00\x00\x00WEBPVP8 ").unwrap();
        assert_eq!(wrong_ext(&webp), Some("webp"));
        let jpg = d.join("photo.jpeg");
        std::fs::write(&jpg, b"\xFF\xD8\xFF\xE0rest").unwrap();
        assert_eq!(wrong_ext(&jpg), None, "a .jpeg is a JPG");
        let bare = d.join("download");
        std::fs::write(&bare, b"\x89PNG\r\n\x1a\nxxxx").unwrap();
        assert_eq!(wrong_ext(&bare), Some("png"));
        let unknown = d.join("notes.xyz");
        std::fs::write(&unknown, b"\x89PNG\r\n\x1a\nxxxx").unwrap();
        assert_eq!(wrong_ext(&unknown), None, "only files Convertino knows");
        let docx = d.join("Letter.docx");
        make_zip(&docx, &[("[Content_Types].xml", b"x"), ("word/document.xml", b"x")]);
        assert_eq!(real_ext(&docx), Some("docx"));
        assert_eq!(wrong_ext(&docx), None);
        let zip_as_docx = d.join("Report.zip");
        make_zip(&zip_as_docx, &[("[Content_Types].xml", b"x"), ("word/document.xml", b"x")]);
        assert_eq!(wrong_ext(&zip_as_docx), Some("docx"));
        let mut mkv = b"\x1A\x45\xDF\xA3".to_vec();
        mkv.extend_from_slice(b"\x42\x82\x88matroska");
        let fake = d.join("clip.webm");
        std::fs::write(&fake, &mkv).unwrap();
        assert_eq!(wrong_ext(&fake), Some("mkv"));
        let mut webm = b"\x1A\x45\xDF\xA3".to_vec();
        webm.extend_from_slice(b"\x42\x82\x84webm");
        let ok = d.join("clip.mkv");
        std::fs::write(&ok, &webm).unwrap();
        assert_eq!(wrong_ext(&ok), None, "WebM is a kind of MKV");
    }

    #[test]
    fn reads_tracks_from_ffmpegs_description() {
        let text = "Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'Valorant.mp4':
  Duration: 00:01:00.00, start: 0.000000, bitrate: 50000 kb/s
  Stream #0:0[0x1](und): Video: h264 (High), yuv420p, 2560x1440, 60 fps
    Metadata:
      handler_name    : VideoHandler
  Stream #0:1[0x2](und): Audio: aac (LC) (mp4a / 0x6134706D), 48000 Hz, stereo, fltp, 160 kb/s
    Metadata:
      handler_name    : Game
  Stream #0:2[0x3](und): Audio: aac (LC), 48000 Hz, mono, fltp, 96 kb/s
    Metadata:
      handler_name    : SoundHandler
  Stream #0:3(eng): Subtitle: subrip (srt)
    Metadata:
      title           : English
  Stream #0:4(hin): Subtitle: hdmv_pgs_subtitle
At least one output file must be specified
";
        let t = parse_tracks(text);
        assert_eq!(t.audio.len(), 2);
        assert_eq!(t.audio[0].title.as_deref(), Some("Game"));
        assert_eq!(t.audio[1].title, None);
        assert_eq!(t.subtitles.len(), 2);
        assert_eq!((t.subtitles[0].language.as_deref(), t.subtitles[0].codec.as_str()), (Some("eng"), "subrip"));
        assert!(text_subtitle(&t.subtitles[0].codec) && !text_subtitle(&t.subtitles[1].codec));
    }

    #[test]
    fn spots_csvs_excel_would_garble() {
        let d = dir("csv");
        let p = d.join("orders.csv");
        std::fs::write(&p, "name,city\nRavi,कानपुर\nAnna,Zürich ₹\n").unwrap();
        let l = csv_look(&p).unwrap();
        assert_eq!(l.sample, vec!['क', 'ा', 'न']);
        assert!(!l.semicolons);
        std::fs::write(&p, "a;b\n1;2\n").unwrap();
        assert!(csv_look(&p).unwrap().semicolons);
        std::fs::write(&p, "a,b\n1,2\n").unwrap();
        assert_eq!(csv_look(&p), None, "plain English CSVs are fine");
        std::fs::write(&p, "\u{FEFF}a,b\nZürich,2\n").unwrap();
        assert_eq!(csv_look(&p), None, "already has the mark");
    }

    #[test]
    fn finds_every_part() {
        let d = dir("parts");
        for n in [1, 2, 4] {
            std::fs::write(d.join(format!("Footage.zip.{n:03}")), b"x").unwrap();
        }
        let (name, found, missing) = parts(&d.join("Footage.zip.002")).unwrap();
        assert_eq!((name.as_str(), found.len(), missing), ("Footage", 3, Some(3)));
        std::fs::write(d.join("Footage.zip.003"), b"x").unwrap();
        let (_, found, missing) = parts(&d.join("Footage.zip.004")).unwrap();
        assert_eq!((found.len(), missing), (4, None));
        assert_eq!(found[0], d.join("Footage.zip.001"));
        std::fs::write(d.join("Movie.part01.rar"), b"x").unwrap();
        std::fs::write(d.join("Movie.part02.rar"), b"x").unwrap();
        let (name, found, missing) = parts(&d.join("Movie.part02.rar")).unwrap();
        assert_eq!((name.as_str(), found.len(), missing), ("Movie", 2, None));
        assert_eq!(part_of(Path::new("notes.txt")), None);
        assert_eq!(part_of(Path::new("photo.001")), None, "a number alone isn't a part");
    }

    #[test]
    fn reads_pdfimages_lists() {
        let text = "page   num  type   width height color comp bpc  enc interp  object ID x-ppi y-ppi size ratio
--------------------------------------------------------------------------------------------
   1     0 image    1200   800  rgb     3   8  jpeg   no        12  0   150   150  200K 7.1%
   1     1 smask    1200   800  gray    1   8  image  no        12  0   150   150  10K 1.0%
   2     2 image      32    32  rgb     3   8  image  no        14  0    72    72  1K 10%
";
        let rows = parse_pdfimages(text);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows.iter().filter(|r| r.kept()).count(), 1);
    }
}
