//! Compare: the original next to its compressed copy, in the Compare window.
//!
//! A session is a list of pairs (pairs.rs): a finished Compress job's results, Settings'
//! "Test on a file", or what the wheel's Compare slot found. The window plays both
//! videos in step (served by the `cvmedia` scheme, which only hands out files of an open
//! session), and falls back to still frames made here when this PC can't play the copy.

use crate::convert::{exec, TempDir};
use crate::pairs::{self, Pair};
use crate::tools::{self, Tool};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Results that are a clip from part of the original (Settings' "Test on a
/// file"): where in the original the clip starts.
fn offsets() -> &'static Mutex<HashMap<PathBuf, f64>> {
    static O: OnceLock<Mutex<HashMap<PathBuf, f64>>> = OnceLock::new();
    O.get_or_init(Default::default)
}

pub fn set_offset(result: &Path, seconds: f64) {
    if let Ok(mut o) = offsets().lock() {
        o.insert(result.to_path_buf(), seconds);
    }
}

fn sessions() -> &'static Mutex<HashMap<u64, Vec<Pair>>> {
    static S: OnceLock<Mutex<HashMap<u64, Vec<Pair>>>> = OnceLock::new();
    S.get_or_init(Default::default)
}

/// A new session; returns its id.
pub fn register(list: Vec<Pair>) -> u64 {
    let id = crate::engine::new_id();
    if let Ok(mut s) = sessions().lock() {
        s.insert(id, list);
    }
    id
}

/// The session `id`: one registered here, or a finished Compress job's results
/// (jobs.rs keeps those as plain original/result pairs).
pub fn session(id: u64) -> Vec<Pair> {
    if let Some(list) = sessions().lock().ok().and_then(|s| s.get(&id).cloned()) {
        return list;
    }
    let list: Vec<Pair> = crate::jobs::compare_pairs(id)
        .into_iter()
        .filter_map(|(original, result)| {
            if original.exists() {
                let offset = offsets().lock().ok().and_then(|o| o.get(&result).copied()).unwrap_or(0.0);
                let made_at = std::fs::metadata(&result).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0);
                Some(Pair { original, copy: result, maker: pairs::JOB.into(), batch_dir: None, from: None, made_at, flag: None, offset })
            } else {
                // Replace moved the original into the Holding Area: its list knows where.
                pairs::for_files(std::slice::from_ref(&result)).into_iter().next()
            }
        })
        .collect();
    if let Ok(mut s) = sessions().lock() {
        s.insert(id, list.clone());
    }
    list
}

pub fn pair_of(id: u64, index: usize) -> Result<Pair, String> {
    session(id).get(index).cloned().ok_or_else(|| "This comparison is no longer available.".into())
}

/// The file the window may stream: side "o" (original) or "c" (copy) of a pair.
pub fn media_path(id: u64, index: usize, side: &str) -> Option<PathBuf> {
    let p = session(id).into_iter().nth(index)?;
    Some(if side == "o" { p.original } else { p.copy })
}

/// Bytes per answer when the window streams a video (it asks for the next part as it plays).
const CHUNK: u64 = 4 * 1024 * 1024;

/// "bytes=100-" / "bytes=100-199" / "bytes=-500" for a file of `len` bytes -> (first, last).
fn parse_range(h: &str, len: u64) -> Option<(u64, u64)> {
    let spec = h.trim().strip_prefix("bytes=")?.split(',').next()?.trim();
    let (a, b) = spec.split_once('-')?;
    if len == 0 {
        return None;
    }
    if a.is_empty() {
        let n: u64 = b.parse().ok()?;
        return Some((len.saturating_sub(n), len - 1));
    }
    let start: u64 = a.parse().ok()?;
    let end = if b.is_empty() { len - 1 } else { b.parse::<u64>().ok()?.min(len - 1) };
    (start <= end).then_some((start, end))
}

fn mime_of(p: &Path) -> &'static str {
    match p.extension().map(|e| e.to_string_lossy().to_lowercase()).as_deref() {
        Some("webm") => "video/webm",
        Some("mov") => "video/quicktime",
        Some("mkv") => "video/x-matroska",
        Some("ogv") => "video/ogg",
        _ => "video/mp4",
    }
}

/// The `cvmedia` scheme: `/<session>/<pair>/<o|c>` streams that side of an open
/// session's pair, in parts (Range requests). Nothing else can be read through it.
pub fn media_response(req: &tauri::http::Request<Vec<u8>>) -> tauri::http::Response<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    use tauri::http::{header, Response, StatusCode};
    let empty = |code: StatusCode| Response::builder().status(code).body(Vec::new()).expect("a response");
    let parts: Vec<&str> = req.uri().path().trim_matches('/').split('/').collect();
    let [id, index, side] = parts[..] else { return empty(StatusCode::NOT_FOUND) };
    let (Ok(id), Ok(index)) = (id.parse::<u64>(), index.parse::<usize>()) else { return empty(StatusCode::NOT_FOUND) };
    if side != "o" && side != "c" {
        return empty(StatusCode::NOT_FOUND);
    }
    let Some(path) = media_path(id, index, side) else { return empty(StatusCode::NOT_FOUND) };
    let Ok(mut f) = std::fs::File::open(&path) else { return empty(StatusCode::NOT_FOUND) };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let asked = req.headers().get(header::RANGE).and_then(|v| v.to_str().ok());
    let Some((start, end)) = asked.map_or(if len > 0 { Some((0, len - 1)) } else { None }, |h| parse_range(h, len)) else {
        return Response::builder().status(StatusCode::RANGE_NOT_SATISFIABLE).header(header::CONTENT_RANGE, format!("bytes */{len}")).body(Vec::new()).expect("a response");
    };
    let end = end.min(start + CHUNK - 1);
    let mut buf = vec![0u8; (end - start + 1) as usize];
    if f.seek(SeekFrom::Start(start)).and_then(|_| f.read_exact(&mut buf)).is_err() {
        return empty(StatusCode::INTERNAL_SERVER_ERROR);
    }
    Response::builder()
        .status(StatusCode::PARTIAL_CONTENT)
        .header(header::CONTENT_TYPE, mime_of(&path))
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{len}"))
        .header(header::CONTENT_LENGTH, buf.len().to_string())
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .body(buf)
        .expect("a response")
}

/// One pair as the window lists it.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub name: String,
    pub original: String,
    pub copy: String,
    pub kind: String,
    pub maker: String,
    /// "Compressed by Drive Cleanup on {date} · the original waits in the Holding Area" ({date}: made_at, filled in by the window)
    pub by: String,
    pub made_at: u64,
    pub original_bytes: u64,
    pub copy_bytes: u64,
    pub original_there: bool,
    pub copy_there: bool,
    pub flag: Option<String>,
    pub marked: bool,
    pub held: bool,
    pub offset: f64,
    pub can_put_back: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: u64,
    pub pairs: Vec<View>,
    /// Drive Cleanup is installed (it puts back and deletes the originals of its batches).
    pub drive_cleanup: bool,
}

fn by_words(p: &Pair) -> String {
    // "{date}": the window fills in the day in the PC's own time zone.
    let on = |w: &str| if p.made_at > 0 { format!("{w} on {{date}}") } else { w.to_string() };
    match p.maker.as_str() {
        pairs::DRIVE_CLEANUP => format!("{} · the original waits in the Holding Area", on("Compressed by Drive Cleanup")),
        pairs::CONVERTINO => format!("{} · the original waits in the Holding Area", on("Compressed by Convertino (Replace)")),
        pairs::TRIMOTHY => format!("{} · the original waits in the Holding Area", on("Compressed for Trimothy")),
        pairs::BESIDE => "The original sits right beside its copy".into(),
        _ if p.offset > 0.0 => "A test sample from Settings".into(),
        _ => on("Compressed by Convertino"),
    }
}

pub fn view(p: &Pair) -> View {
    let meta = |q: &Path| std::fs::metadata(q).ok().filter(|m| m.is_file());
    let (mo, mc) = (meta(&p.original), meta(&p.copy));
    View {
        name: p.original.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        original: p.original.to_string_lossy().into_owned(),
        copy: p.copy.to_string_lossy().into_owned(),
        kind: p.kind().unwrap_or("video").into(),
        maker: p.maker.clone(),
        by: by_words(p),
        made_at: p.made_at,
        original_bytes: mo.as_ref().map(|m| m.len()).unwrap_or(0),
        copy_bytes: mc.as_ref().map(|m| m.len()).unwrap_or(0),
        original_there: mo.is_some(),
        copy_there: mc.is_some(),
        flag: p.flag.clone(),
        marked: pairs::marked(&p.original).is_some(),
        held: p.batch_dir.is_some(),
        offset: p.offset,
        can_put_back: p.maker == pairs::CONVERTINO,
    }
}

pub fn session_view(id: u64) -> Session {
    Session { id, pairs: session(id).iter().map(view).collect(), drive_cleanup: pairs::drive_cleanup_app().is_some() }
}

/// What the facts table shows about one file.
#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Facts {
    pub bytes: u64,
    pub width: u32,
    pub height: u32,
    pub fps: Option<f64>,
    pub seconds: Option<f64>,
    /// Bits per second of the whole file.
    pub bitrate: Option<u64>,
    pub video: Option<String>,
    pub audio: Option<String>,
    pub audio_kbps: Option<u32>,
    pub format: String,
    pub error: Option<String>,
}

fn facts_of(p: &Path, kind: &str) -> Facts {
    let bytes = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    let format = p.extension().map(|e| e.to_string_lossy().to_uppercase()).unwrap_or_default();
    let mut f = Facts { bytes, format, ..Facts::default() };
    match kind {
        "video" => match crate::video::probe(p) {
            Ok(v) => {
                f.width = v.width;
                f.height = v.height;
                f.fps = v.fps;
                f.seconds = v.duration;
                f.bitrate = v.duration.filter(|d| *d > 0.0).map(|d| (bytes as f64 * 8.0 / d) as u64);
                f.video = v.video;
                f.audio = v.audio;
                f.audio_kbps = v.audio_kbps;
            }
            Err(e) => f.error = Some(e),
        },
        "image" => {
            if let Some((w, h)) = picture_size(p) {
                f.width = w;
                f.height = h;
            }
        }
        _ => {}
    }
    f
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairFacts {
    pub original: Facts,
    pub copy: Facts,
}

pub fn facts(id: u64, index: usize) -> Result<PairFacts, String> {
    let p = pair_of(id, index)?;
    let kind = p.kind().unwrap_or("video");
    Ok(PairFacts { original: facts_of(&p.original, kind), copy: facts_of(&p.copy, kind) })
}

pub fn points(id: u64, index: usize) -> Result<Vec<crate::verify::Point>, String> {
    let p = pair_of(id, index)?;
    match p.kind() {
        Some("video") | Some("image") => crate::verify::checkpoints(&p.original, &p.copy, p.offset),
        _ => Ok(Vec::new()),
    }
}

/// Longest side of the pictures shown.
const SIDE: u32 = 2000;
/// Still frames of a video this PC can't play: smaller, so stepping stays quick.
const FRAME_SIDE: u32 = 1280;

/// A still of one side: a picture, a PDF page (`at` = page number), or a video frame
/// at `at` seconds into the copy (the original's frame is taken `offset` later).
/// The copy is scaled to the original's size, so it shows what was lost.
pub fn still(id: u64, index: usize, side: &str, at: f64) -> Result<String, String> {
    let p = pair_of(id, index)?;
    let tmp = TempDir::new("compare")?;
    let (file, other) = if side == "o" { (&p.original, None) } else { (&p.copy, Some(&p.original)) };
    if !file.exists() {
        return Err(if side == "o" { "The original is no longer there.".into() } else { "The compressed copy is no longer there.".into() });
    }
    match p.kind() {
        Some("pdf") => page(file, (at.max(1.0)) as u32, &tmp.0, side),
        Some("video") => {
            let like = other.and_then(|o| crate::video::probe(o).ok()).map(|v| (v.width, v.height));
            let t = if side == "o" { at + p.offset } else { at };
            frame(file, t, like, &tmp.0, side, FRAME_SIDE)
        }
        _ => picture(file, other.and_then(|o| picture_size(o)), &tmp.0, side, SIDE),
    }
}

fn picture_size(p: &Path) -> Option<(u32, u32)> {
    let exe = tools::find(Tool::Magick)?;
    let mut src = p.as_os_str().to_os_string();
    src.push("[0]");
    let out = tools::command(&exe).arg(src).args(["-auto-orient", "-format", "%w %h", "info:"]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let mut it = text.split_whitespace().filter_map(|n| n.parse().ok());
    Some((it.next()?, it.next()?))
}

/// A picture as a JPEG data URL, at most SIDE px; `like`: first scaled to
/// the original's size, so a smaller result is shown as it would look.
fn picture(p: &Path, like: Option<(u32, u32)>, dir: &Path, tag: &str, side: u32) -> Result<String, String> {
    let out = dir.join(format!("{tag}.jpg"));
    let mut src = p.as_os_str().to_os_string();
    src.push("[0]");
    let mut cmd = tools::command(&tools::require(Tool::Magick)?);
    cmd.arg(src).arg("-auto-orient").args(["-background", "white", "-alpha", "remove", "-alpha", "off"]);
    if let Some((w, h)) = like.filter(|(w, h)| *w > 0 && *h > 0) {
        cmd.args(["-resize", &format!("{w}x{h}!")]);
    }
    cmd.args(["-resize", &format!("{side}x{side}>"), "-quality", "92"]).arg(&out);
    exec(Tool::Magick, cmd)?;
    data_url(&out)
}

fn frame(p: &Path, at: f64, like: Option<(u32, u32)>, dir: &Path, tag: &str, side: u32) -> Result<String, String> {
    let out = dir.join(format!("{tag}.png"));
    let mut cmd = tools::command(&tools::require(Tool::Ffmpeg)?);
    cmd.args(["-hide_banner", "-nostdin", "-loglevel", "error", "-y", "-ss", &format!("{at:.2}"), "-i"]).arg(p).args(["-frames:v", "1"]);
    if let Some((w, h)) = like.filter(|(w, h)| *w > 0 && *h > 0) {
        cmd.args(["-vf", &format!("scale={w}:{h}:flags=bicubic")]);
    }
    cmd.arg(&out);
    exec(Tool::Ffmpeg, cmd)?;
    picture(&out, None, dir, &format!("{tag}j"), side)
}

fn page(p: &Path, n: u32, dir: &Path, tag: &str) -> Result<String, String> {
    let prefix = dir.join(tag);
    let n = n.to_string();
    let mut cmd = tools::command(&tools::require(Tool::Pdftoppm)?);
    cmd.args(["-png", "-singlefile", "-r", "120", "-f", &n, "-l", &n]).arg(p).arg(&prefix);
    exec(Tool::Pdftoppm, cmd)?;
    picture(&prefix.with_extension("png"), None, dir, &format!("{tag}j"), SIDE)
}

fn data_url(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    Ok(format!("data:image/jpeg;base64,{}", base64(&bytes)))
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-", 10), Some((0, 9)));
        assert_eq!(parse_range("bytes=2-4", 10), Some((2, 4)));
        assert_eq!(parse_range("bytes=5-99", 10), Some((5, 9)));
        assert_eq!(parse_range("bytes=-3", 10), Some((7, 9)));
        assert_eq!(parse_range("bytes=11-", 10), None);
        assert_eq!(parse_range("bytes=0-", 0), None);
        assert_eq!(parse_range("items=0-1", 10), None);
    }

    #[test]
    fn media_is_served_in_parts_and_only_for_a_session() {
        let d = std::env::temp_dir().join(format!("convertino-cmpmedia-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let (o, c) = (d.join("a.mp4"), d.join("a (compressed).mp4"));
        std::fs::write(&o, b"0123456789").unwrap();
        std::fs::write(&c, b"abcdef").unwrap();
        let id = register(vec![Pair { original: o.clone(), copy: c.clone(), maker: pairs::BESIDE.into(), batch_dir: None, from: None, made_at: 0, flag: None, offset: 0.0 }]);
        let req = |path: &str, range: Option<&str>| {
            let mut b = tauri::http::Request::builder().uri(format!("cvmedia://localhost{path}"));
            if let Some(r) = range {
                b = b.header("Range", r);
            }
            media_response(&b.body(Vec::new()).unwrap())
        };
        let r = req(&format!("/{id}/0/o"), Some("bytes=2-5"));
        assert_eq!(r.status(), 206);
        assert_eq!(r.body(), b"2345");
        assert_eq!(r.headers()["content-range"], "bytes 2-5/10");
        let r = req(&format!("/{id}/0/c"), None);
        assert_eq!(r.body(), b"abcdef");
        assert_eq!(req(&format!("/{id}/1/c"), None).status(), 404);
        assert_eq!(req("/999999/0/c", None).status(), 404);
        assert_eq!(req(&format!("/{id}/0/x"), None).status(), 404);
        assert_eq!(req(&format!("/{id}/0/o"), Some("bytes=20-")).status(), 416);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn base64_matches_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
