//! Compare: the original next to its compressed copy, as pictures the
//! Compare window shows under a slider. A video is compared on a frame from
//! its middle, a PDF on its first page.

use crate::convert::{exec, TempDir};
use crate::tools::{self, Tool};
use serde::Serialize;
use std::path::Path;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pair {
    pub count: usize,
    pub index: usize,
    pub title: String,
    /// What is shown: "Frame at 1:15", "Page 1".
    pub what: String,
    pub left: String,
    pub right: String,
    pub left_label: String,
    pub right_label: String,
}

/// Longest side of the pictures shown.
const SIDE: u32 = 2000;

pub fn pair(id: u64, index: usize) -> Result<Pair, String> {
    let pairs = crate::jobs::compare_pairs(id);
    let (original, result) = pairs.get(index).cloned().ok_or("This comparison is no longer available.")?;
    if !result.exists() {
        return Err("The compressed file is no longer there.".into());
    }
    let tmp = TempDir::new("compare")?;
    let ext = |p: &Path| p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let video_exts = ["mp4", "mov", "mkv", "webm", "avi", "wmv", "m4v"];
    let (left, right, what) = if ext(&original) == "pdf" {
        (page(&original, &tmp.0, "a")?, page(&result, &tmp.0, "b")?, "Page 1".to_string())
    } else if video_exts.contains(&ext(&original).as_str()) {
        let d = crate::video::probe(&result).ok().and_then(|p| p.duration).unwrap_or(2.0);
        let at = d / 2.0;
        let what = format!("Frame at {}:{:02}", (at / 60.0) as u32, (at % 60.0) as u32);
        // The result's frame, scaled back up to the original's size, shows what was lost.
        let o = crate::video::probe(&original).ok();
        let (w, h) = o.map(|p| (p.width, p.height)).unwrap_or((0, 0));
        (frame(&original, at, None, &tmp.0, "a")?, frame(&result, at, Some((w, h)), &tmp.0, "b")?, what)
    } else {
        let px = picture_size(&original);
        (picture(&original, None, &tmp.0, "a")?, picture(&result, px, &tmp.0, "b")?, String::new())
    };
    let size = |p: &Path| crate::size::words(p.metadata().map(|m| m.len()).unwrap_or(0));
    Ok(Pair {
        count: pairs.len(),
        index,
        title: result.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        what,
        left,
        right,
        left_label: format!("Original · {}", size(&original)),
        right_label: format!("Compressed · {}", size(&result)),
    })
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
fn picture(p: &Path, like: Option<(u32, u32)>, dir: &Path, tag: &str) -> Result<String, String> {
    let out = dir.join(format!("{tag}.jpg"));
    let mut src = p.as_os_str().to_os_string();
    src.push("[0]");
    let mut cmd = tools::command(&tools::require(Tool::Magick)?);
    cmd.arg(src).arg("-auto-orient").args(["-background", "white", "-alpha", "remove", "-alpha", "off"]);
    if let Some((w, h)) = like.filter(|(w, h)| *w > 0 && *h > 0) {
        cmd.args(["-resize", &format!("{w}x{h}!")]);
    }
    cmd.args(["-resize", &format!("{SIDE}x{SIDE}>"), "-quality", "95"]).arg(&out);
    exec(Tool::Magick, cmd)?;
    data_url(&out)
}

fn frame(p: &Path, at: f64, like: Option<(u32, u32)>, dir: &Path, tag: &str) -> Result<String, String> {
    let out = dir.join(format!("{tag}.png"));
    let mut cmd = tools::command(&tools::require(Tool::Ffmpeg)?);
    cmd.args(["-hide_banner", "-nostdin", "-loglevel", "error", "-y", "-ss", &format!("{at:.2}"), "-i"]).arg(p).args(["-frames:v", "1"]);
    if let Some((w, h)) = like.filter(|(w, h)| *w > 0 && *h > 0) {
        cmd.args(["-vf", &format!("scale={w}:{h}:flags=bicubic")]);
    }
    cmd.arg(&out);
    exec(Tool::Ffmpeg, cmd)?;
    picture(&out, None, dir, &format!("{tag}j"))
}

fn page(p: &Path, dir: &Path, tag: &str) -> Result<String, String> {
    let prefix = dir.join(tag);
    let mut cmd = tools::command(&tools::require(Tool::Pdftoppm)?);
    cmd.args(["-png", "-singlefile", "-r", "120", "-f", "1", "-l", "1"]).arg(p).arg(&prefix);
    exec(Tool::Pdftoppm, cmd)?;
    picture(&prefix.with_extension("png"), None, dir, &format!("{tag}j"))
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
    fn base64_matches_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
