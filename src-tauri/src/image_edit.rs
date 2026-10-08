//! Image › Edit: a window (ui/image-editor.html) where crops are drawn on a
//! picture or RAW photo, each saved as its own file next to the original.
//!
//! A crop is a rectangle, or four corners (for something photographed at an
//! angle: a sign, a page, a screen) that is straightened into a flat picture.
//! The window works in the picture's own pixels, after the camera's turn
//! (EXIF orientation) and after any turn or flip made in the window.
//! Saving runs the crops as an ordinary job (corner card, Undo, the same
//! quality search as conversions); the original is never changed.

use crate::convert::{self, s, Op, Step, TempDir};
use crate::look::{Level, Lossy};
use crate::settings::Quality;
use crate::tools::{self, Tool};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// Pictures the window can show straight from the file (the webview reads
/// them and turns them by their EXIF orientation, as ImageMagick's -auto-orient does).
const SHOWN_AS_IS: &[&str] = &["jpg", "jpeg", "jfif", "jpe", "png", "webp", "gif", "bmp", "avif"];
/// Longest side of the picture shown in the window when one has to be made.
const PREVIEW_MAX: u32 = 2400;
/// Largest picture a crop may come out as (each side).
const MAX_SIDE: u32 = 30_000;

/// One open editor window.
struct Session {
    path: PathBuf,
    raw: bool,
    /// What the window shows when it can't show the file itself, and the
    /// picture's full size (RAW: the developed photo).
    preview: Option<(PathBuf, u32, u32)>,
    /// RAW: the photo developed once, used for every crop. Shared with
    /// running jobs so closing the window doesn't delete it under them.
    developed: Option<(Arc<TempDir>, PathBuf)>,
}

fn sessions() -> &'static Mutex<HashMap<String, Session>> {
    static S: OnceLock<Mutex<HashMap<String, Session>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Window labels start with "editor-" so the PDF editor's permissions cover them.
pub fn open(app: &tauri::AppHandle, path: PathBuf) {
    use std::sync::atomic::{AtomicU64, Ordering};
    use tauri::{WebviewUrl, WebviewWindowBuilder};
    static N: AtomicU64 = AtomicU64::new(1);
    let label = format!("editor-img-{}", N.fetch_add(1, Ordering::SeqCst));
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    log::info!("image edit: {}", path.display());
    let raw = crate::raw::is_raw(&path);
    if let Ok(mut s) = sessions().lock() {
        s.insert(label.clone(), Session { path, raw, preview: None, developed: None });
    }
    let app = app.clone();
    // Built off the calling thread: creating a window inside a command can deadlock on Windows.
    std::thread::spawn(move || {
        let built = WebviewWindowBuilder::new(&app, &label, WebviewUrl::App("image-editor.html".into()))
            .title(format!("{name} – Edit"))
            .inner_size(1240.0, 820.0)
            .min_inner_size(820.0, 560.0)
            .center()
            .focused(true)
            .disable_drag_drop_handler()
            .build();
        if let Err(e) = built {
            log::error!("couldn't open the image editor: {e}");
        }
    });
}

// ---------- what to save ----------

/// A format the crops can be saved as.
#[derive(Clone, Serialize)]
pub struct Format {
    pub id: String,
    pub label: String,
}

/// The extension "Same as original" writes (a format ImageMagick can't
/// write well becomes the closest common one).
fn same_ext(src_ext: &str) -> &str {
    match src_ext {
        "jpeg" => "jpeg",
        "jfif" | "jpe" | "jpg" => "jpg",
        "heic" | "heif" => "jpg",
        "svg" => "png",
        "tif" => "tif",
        e => e,
    }
}

/// The save choices for a file: (choices, the one picked at first).
pub fn formats_for(path: &Path) -> (Vec<Format>, String) {
    let f = |id: &str, label: &str| Format { id: id.into(), label: label.into() };
    if crate::raw::is_raw(path) {
        let v = vec![f("jpg", "JPG"), f("png", "PNG"), f("tiff", "TIFF (16-bit)"), f("webp", "WEBP"), f("avif", "AVIF")];
        return (v, "jpg".into());
    }
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let same = same_ext(&ext).to_uppercase();
    let mut v = vec![Format { id: "same".into(), label: format!("Same as original ({same})") }];
    v.extend([f("jpg", "JPG"), f("png", "PNG"), f("webp", "WEBP"), f("avif", "AVIF"), f("tiff", "TIFF")]);
    (v, "same".into())
}

/// One crop as the window sends it, in the turned picture's pixels.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Item {
    Rect { name: String, x: f64, y: f64, w: f64, h: f64 },
    /// Corners in order: top left, top right, bottom right, bottom left; `w`×`h`: the flat size.
    Quad { name: String, pts: [[f64; 2]; 4], w: f64, h: f64 },
    /// The whole picture, with the window's turn and flip.
    Whole { name: String },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    /// Quarter turns clockwise (0–3).
    pub turn: u8,
    /// Mirrored left to right (before the turn, as the window draws it).
    pub flip: bool,
    /// A Format id.
    pub format: String,
    pub items: Vec<Item>,
}

/// A name that is safe as a file name ("photo - 1"); empty means `fallback`.
fn clean_name(name: &str, fallback: &str) -> String {
    let n: String = name
        .chars()
        .map(|c| if "\\/:*?\"<>|".contains(c) || c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches('.')
        .chars()
        .take(120)
        .collect();
    if n.trim().is_empty() { fallback.to_string() } else { n.trim().to_string() }
}

fn px(v: f64) -> i64 {
    v.round() as i64
}
fn side(v: f64) -> Result<u32, String> {
    let n = v.round();
    if !(1.0..=MAX_SIDE as f64).contains(&n) {
        return Err(format!("A crop can be 1 to {MAX_SIDE} pixels on each side."));
    }
    Ok(n as u32)
}

/// ImageMagick arguments that turn the picture as the window shows it, then cut out one item.
pub fn geometry(turn: u8, flip: bool, item: &Item) -> Result<Vec<String>, String> {
    let mut a = s(&["-auto-orient"]);
    if flip {
        a.push("-flop".into());
    }
    if turn % 4 != 0 {
        a.extend(s(&["-rotate", &(90 * (turn % 4) as u32).to_string()]));
    }
    a.push("+repage".into());
    match item {
        Item::Whole { .. } => {}
        Item::Rect { x, y, w, h, .. } => {
            let (w, h) = (side(*w)?, side(*h)?);
            a.extend(s(&["-crop", &format!("{w}x{h}+{}+{}", px(*x).max(0), px(*y).max(0)), "+repage"]));
        }
        Item::Quad { pts, w, h, .. } => {
            let (w, h) = (side(*w)?, side(*h)?);
            let dst = [[0.0, 0.0], [w as f64, 0.0], [w as f64, h as f64], [0.0, h as f64]];
            let ctrl = pts
                .iter()
                .zip(dst.iter())
                .map(|(p, d)| format!("{:.2},{:.2} {},{}", p[0], p[1], d[0], d[1]))
                .collect::<Vec<_>>()
                .join("  ");
            a.extend(s(&[
                "-virtual-pixel",
                "edge",
                "-define",
                &format!("distort:viewport={w}x{h}+0+0"),
                "-distort",
                "Perspective",
                &ctrl,
                "+repage",
            ]));
        }
    }
    Ok(a)
}

/// The steps that save every item. `input`: the picture to cut from (RAW:
/// the developed photo); `original`: the file the names and folder come from.
pub fn steps(original: &Path, input: &Path, req: &Request, q: &Quality) -> Result<Vec<Step>, String> {
    if req.items.is_empty() {
        return Err("There are no crops to save.".into());
    }
    let (dir, stem, src_ext) = convert::split(original);
    let raw = crate::raw::is_raw(original);
    let ext = match req.format.as_str() {
        "same" if !raw => same_ext(&src_ext).to_string(),
        "jpg" | "png" | "webp" | "avif" | "tiff" => req.format.clone(),
        _ => "jpg".to_string(),
    };
    let level = Level::from_setting(&q.image);
    let flatten = s(&["-background", "white", "-alpha", "remove", "-alpha", "off"]);
    let mut out = Vec::new();
    for (i, item) in req.items.iter().enumerate() {
        let geo = geometry(req.turn, req.flip, item)?;
        let fallback = match item {
            Item::Whole { .. } => format!("{stem} (edited)"),
            _ => format!("{stem} - {}", i + 1),
        };
        let name = match item {
            Item::Rect { name, .. } | Item::Quad { name, .. } | Item::Whole { name } => clean_name(name, &fallback),
        };
        let depth8 = if raw { s(&["-depth", "8"]) } else { Vec::new() };
        let picture = |prep: Vec<String>, format: Lossy, fixed: u32| Op::Picture { prep, format, level, fixed };
        let op = match ext.as_str() {
            "jpg" | "jpeg" => picture([geo, flatten.clone()].concat(), Lossy::Jpg, q.jpg),
            "webp" => picture(geo, Lossy::Webp, q.webp),
            "avif" => picture([geo, depth8].concat(), Lossy::Avif, 60),
            "png" => Op::Png { args: [geo, depth8].concat(), level },
            "tiff" | "tif" if raw => Op::Magick { args: [geo, s(&["-depth", "16", "-compress", "zip"])].concat(), first_frame: true },
            "tiff" | "tif" => Op::Magick { args: [geo, s(&["-compress", "lzw"])].concat(), first_frame: true },
            "ico" => Op::Magick {
                args: [geo, s(&["-background", "none", "-define", "icon:auto-resize=256,128,64,48,32,16"])].concat(),
                first_frame: true,
            },
            _ => Op::Magick { args: geo, first_frame: true },
        };
        out.push(Step { inputs: vec![input.to_path_buf()], dir: dir.clone(), stem: name, ext: ext.clone(), folder: false, op });
    }
    Ok(out)
}

// ---------- the picture the window shows ----------

/// The picture's size after the camera's turn.
fn size_of(path: &Path) -> Result<(u32, u32), String> {
    let exe = tools::require(Tool::Magick)?;
    let mut src = path.as_os_str().to_os_string();
    src.push("[0]");
    let out = tools::command(&exe)
        .arg(src)
        .args(["-auto-orient", "-format", "%w %h", "info:"])
        .output()
        .map_err(|e| format!("Couldn't read the picture: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut it = text.split_whitespace().filter_map(|v| v.parse::<u32>().ok());
    match (it.next(), it.next()) {
        (Some(w), Some(h)) if w > 0 && h > 0 => Ok((w, h)),
        _ => Err("Couldn't read the picture.".into()),
    }
}

/// Makes the picture shown in the window (a JPG at most PREVIEW_MAX on its
/// long side) and returns it with the full size.
fn make_preview(src: &Path, dir: &Path) -> Result<(PathBuf, u32, u32), String> {
    let (w, h) = size_of(src)?;
    let out = dir.join("preview.jpg");
    let exe = tools::require(Tool::Magick)?;
    let mut input = src.as_os_str().to_os_string();
    input.push("[0]");
    let mut cmd = tools::command(&exe);
    cmd.arg(input).args(["-auto-orient", "-background", "white", "-alpha", "remove", "-alpha", "off", "-resize"]);
    cmd.arg(format!("{PREVIEW_MAX}x{PREVIEW_MAX}>")).args(["-depth", "8", "-quality", "88"]).arg(&out);
    convert::exec(Tool::Magick, cmd)?;
    Ok((out, w, h))
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Opened {
    name: String,
    accent: Option<String>,
    raw: bool,
    /// The window can show the file as it is (else it asks for a preview).
    as_is: bool,
    formats: Vec<Format>,
    format: String,
}

fn label_path(label: &str) -> Result<(PathBuf, bool), String> {
    sessions()
        .lock()
        .ok()
        .and_then(|s| s.get(label).map(|x| (x.path.clone(), x.raw)))
        .ok_or_else(|| "This window has no picture.".to_string())
}

#[tauri::command]
pub fn imgedit_file(window: tauri::WebviewWindow) -> Result<Opened, String> {
    let (path, raw) = label_path(window.label())?;
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let (formats, format) = formats_for(&path);
    Ok(Opened {
        name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        accent: crate::accent::system_accent(),
        raw,
        as_is: !raw && SHOWN_AS_IS.contains(&ext.as_str()),
        formats,
        format,
    })
}

/// The original file's bytes, for pictures the window shows as they are.
#[tauri::command]
pub fn imgedit_bytes(window: tauri::WebviewWindow) -> Result<tauri::ipc::Response, String> {
    let (path, _) = label_path(window.label())?;
    let bytes = std::fs::read(&path).map_err(|e| format!("Couldn't read {}: {e}", path.display()))?;
    Ok(tauri::ipc::Response::new(bytes))
}

#[derive(Clone, Serialize)]
pub struct Preview {
    width: u32,
    height: u32,
}

/// Makes the picture shown in the window (RAW: develops the photo first).
/// Returns the full size; the picture itself comes from imgedit_preview_bytes.
#[tauri::command]
pub async fn imgedit_prepare(window: tauri::WebviewWindow) -> Result<Preview, String> {
    let label = window.label().to_string();
    tauri::async_runtime::spawn_blocking(move || prepare(&label)).await.map_err(|e| e.to_string())?
}

fn prepare(label: &str) -> Result<Preview, String> {
    if let Some((_, w, h)) = sessions().lock().ok().and_then(|s| s.get(label).and_then(|x| x.preview.clone())) {
        return Ok(Preview { width: w, height: h });
    }
    let (path, raw) = label_path(label)?;
    let tmp = Arc::new(TempDir::new("imgedit")?);
    let source = if raw {
        let dev = crate::raw::develop(&path, &tmp.0)?;
        log::info!("image edit: developed {}", path.display());
        dev
    } else {
        path.clone()
    };
    let (prev, w, h) = make_preview(&source, &tmp.0)?;
    if let Ok(mut s) = sessions().lock() {
        if let Some(x) = s.get_mut(label) {
            x.preview = Some((prev, w, h));
            x.developed = Some((tmp, source));
        }
    }
    Ok(Preview { width: w, height: h })
}

#[tauri::command]
pub fn imgedit_preview_bytes(window: tauri::WebviewWindow) -> Result<tauri::ipc::Response, String> {
    let p = sessions()
        .lock()
        .ok()
        .and_then(|s| s.get(window.label()).and_then(|x| x.preview.as_ref().map(|p| p.0.clone())))
        .ok_or("The picture isn't ready yet.")?;
    let bytes = std::fs::read(&p).map_err(|e| e.to_string())?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// Saves the crops as a job (the corner card shows it). Returns the job id.
#[tauri::command]
pub fn imgedit_save(app: tauri::AppHandle, window: tauri::WebviewWindow, request: Request) -> Result<u64, String> {
    let (path, raw, keep) = {
        let s = sessions().lock().map_err(|_| "busy")?;
        let x = s.get(window.label()).ok_or("This window has no picture.")?;
        (x.path.clone(), x.raw, x.developed.clone())
    };
    // RAW crops come from the photo developed for the window; other pictures from the file.
    let (input, keep) = match (raw, keep) {
        (true, Some((tmp, dev))) => (dev, Some(tmp)),
        (true, None) => return Err("The photo is still being developed.".into()),
        (false, _) => (path.clone(), None),
    };
    if !path.exists() {
        return Err("The original isn't there any more.".into());
    }
    let q = crate::settings::get().quality;
    let steps = steps(&path, &input, &request, &q)?;
    let n = steps.len();
    log::info!("image edit: saving {n} item(s) from {}", path.display());
    let exif_from = if raw { Some(path.clone()) } else { None };
    let title = format!("{} → {n} crop{}", crate::engine::file_name(&path), if n == 1 { "" } else { "s" });
    let done = if n == 1 { "Saved 1 crop".to_string() } else { format!("Saved {n} crops") };
    Ok(crate::jobs::start_steps(app, title, "image".into(), done, vec![path], steps, exif_from, keep))
}

#[tauri::command]
pub fn imgedit_close(window: tauri::WebviewWindow) {
    if let Ok(mut s) = sessions().lock() {
        s.remove(window.label());
    }
    let _ = window.destroy();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> Item {
        Item::Rect { name: String::new(), x, y, w, h }
    }

    #[test]
    fn turn_and_flip_come_before_the_crop() {
        let g = geometry(1, true, &rect(10.0, 20.4, 300.0, 200.0)).unwrap();
        assert_eq!(g, s(&["-auto-orient", "-flop", "-rotate", "90", "+repage", "-crop", "300x200+10+20", "+repage"]));
        let g = geometry(0, false, &Item::Whole { name: String::new() }).unwrap();
        assert_eq!(g, s(&["-auto-orient", "+repage"]));
    }

    #[test]
    fn four_corners_flatten_to_the_size_asked() {
        let q = Item::Quad { name: String::new(), pts: [[10.0, 10.0], [110.0, 20.0], [100.0, 120.0], [5.0, 100.0]], w: 400.0, h: 300.0 };
        let g = geometry(0, false, &q).unwrap();
        assert!(g.contains(&"distort:viewport=400x300+0+0".to_string()));
        assert!(g.contains(&"10.00,10.00 0,0  110.00,20.00 400,0  100.00,120.00 400,300  5.00,100.00 0,300".to_string()));
    }

    #[test]
    fn rejects_empty_and_huge_crops() {
        assert!(geometry(0, false, &rect(0.0, 0.0, 0.2, 10.0)).is_err());
        assert!(geometry(0, false, &rect(0.0, 0.0, 40_000.0, 10.0)).is_err());
    }

    #[test]
    fn names_and_formats() {
        let req = Request {
            turn: 0,
            flip: false,
            format: "same".into(),
            items: vec![rect(0.0, 0.0, 10.0, 10.0), Item::Rect { name: "Sign / board?".into(), x: 0.0, y: 0.0, w: 5.0, h: 5.0 }, Item::Whole { name: String::new() }],
        };
        let q = Quality::default();
        let st = steps(Path::new("/p/beach.jpeg"), Path::new("/p/beach.jpeg"), &req, &q).unwrap();
        assert_eq!(st.iter().map(|s| s.stem.as_str()).collect::<Vec<_>>(), ["beach - 1", "Sign board", "beach (edited)"]);
        assert!(st.iter().all(|s| s.ext == "jpeg"));
        assert!(matches!(st[0].op, Op::Picture { format: Lossy::Jpg, .. }));
        // HEIC and SVG can't be written back: the closest common format.
        let heic = steps(Path::new("/p/a.heic"), Path::new("/p/a.heic"), &req, &q).unwrap();
        assert_eq!(heic[0].ext, "jpg");
        let svg = steps(Path::new("/p/a.svg"), Path::new("/p/a.svg"), &req, &q).unwrap();
        assert_eq!(svg[0].ext, "png");
        // RAW: no "same", JPG by default.
        let raw = steps(Path::new("/p/a.cr3"), Path::new("/t/developed.tiff"), &req, &q).unwrap();
        assert_eq!(raw[0].ext, "jpg");
        assert_eq!(raw[0].inputs[0], PathBuf::from("/t/developed.tiff"));
        assert_eq!(formats_for(Path::new("/p/a.cr3")).1, "jpg");
        assert_eq!(formats_for(Path::new("/p/a.png")).0[0].label, "Same as original (PNG)");
    }

    /// Real crops with ImageMagick: a turned and flipped picture, a rectangle,
    /// four corners straightened, the whole picture; sizes and colours checked.
    #[test]
    fn real_crops() {
        let Some(exe) = tools::find(Tool::Magick) else { return };
        let tmp = TempDir::new("imgedit-test").unwrap();
        let src = tmp.0.join("scene.png");
        // 400×300 grey with a red block at 40..140 × 30..90 and a blue tilted quad.
        let st = tools::command(&exe)
            .args(["-size", "400x300", "xc:#808080", "-fill", "#ff0000", "-draw", "rectangle 40,30 139,89"])
            .args(["-fill", "#0000ff", "-draw", "polygon 220,120 360,140 350,260 210,240"])
            .arg(&src)
            .status()
            .unwrap();
        assert!(st.success());
        let size = |p: &Path| size_of(p).unwrap();
        let px = |p: &Path, x: u32, y: u32| -> String {
            let out = tools::command(&exe).arg(p).args(["-format", &format!("%[pixel:p{{{x},{y}}}]"), "info:"]).output().unwrap();
            String::from_utf8_lossy(&out.stdout).to_string()
        };
        let q = Quality { image: "fixed".into(), ..Quality::default() };
        let run = |turn: u8, flip: bool, format: &str, items: Vec<Item>| -> Vec<PathBuf> {
            let req = Request { turn, flip, format: format.into(), items };
            steps(&src, &src, &req, &q).unwrap().iter().map(|s| convert::run(s, &mut |_| {}).unwrap()).collect()
        };
        // The red block cut out exactly, named "scene - 1.png".
        let a = run(0, false, "same", vec![Item::Rect { name: String::new(), x: 40.0, y: 30.0, w: 100.0, h: 60.0 }]);
        assert_eq!(a[0].file_name().unwrap().to_string_lossy(), "scene - 1.png");
        assert_eq!(size(&a[0]), (100, 60));
        assert!(px(&a[0], 50, 30).contains("255,0,0"), "{}", px(&a[0], 50, 30));
        // Four corners on the blue quad: flattened to 200×150 and blue all over.
        let b = run(0, false, "png", vec![Item::Quad { name: "sign".into(), pts: [[222.0, 123.0], [356.0, 142.0], [347.0, 256.0], [213.0, 238.0]], w: 200.0, h: 150.0 }]);
        assert_eq!(b[0].file_name().unwrap().to_string_lossy(), "sign.png");
        assert_eq!(size(&b[0]), (200, 150));
        for (x, y) in [(3, 3), (196, 3), (196, 146), (3, 146), (100, 75)] {
            assert!(px(&b[0], x, y).contains("0,0,255"), "corner {x},{y}: {}", px(&b[0], x, y));
        }
        // Flipped then turned right (as the window draws it): the picture is 300×400;
        // the red block flips to x 260..360, then turns to x 300-90..300-30 = 210..270, y 260..360.
        let c = run(1, true, "jpg", vec![
            Item::Rect { name: String::new(), x: 210.0, y: 260.0, w: 60.0, h: 100.0 },
            Item::Whole { name: String::new() },
        ]);
        assert_eq!(size(&c[0]), (60, 100));
        assert!(px(&c[0], 30, 50).starts_with("srgb(25") || px(&c[0], 30, 50).contains("(254,") || px(&c[0], 30, 50).contains("(255,"), "{}", px(&c[0], 30, 50));
        assert_eq!(size(&c[1]), (300, 400));
        assert_eq!(c[1].file_name().unwrap().to_string_lossy(), "scene (edited).jpg");
    }
}
