//! Fine-tuning behind the Smaller / Balanced / Best presets (Settings ›
//! Quality › Tune).
//!
//! Each preset keeps only what the person changed (`Tunes`, inside
//! `settings::Quality`, so Shift+click options, the command line and MCP can
//! override it for one job the same way). Everything else follows the
//! defaults here, which are the numbers Convertino has always used. The
//! engines never read `Tunes` directly: they get a resolved `Image`, `Video`
//! or `Pdf` for the preset in use.

use serde::{Deserialize, Serialize};

/// Which preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Grade {
    Small,
    Balanced,
    Best,
}

impl Grade {
    pub const ALL: [Grade; 3] = [Grade::Small, Grade::Balanced, Grade::Best];

    /// "small", "balanced", "best" (PDF Compress says "high" for Best). Anything
    /// else (including "fixed") is Balanced.
    pub fn from_word(w: &str) -> Grade {
        match w {
            "small" | "smaller" => Grade::Small,
            "best" | "high" => Grade::Best,
            _ => Grade::Balanced,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Grade::Small => "small",
            Grade::Balanced => "balanced",
            Grade::Best => "best",
        }
    }

    fn pick<T>(self, small: T, balanced: T, best: T) -> T {
        match self {
            Grade::Small => small,
            Grade::Balanced => balanced,
            Grade::Best => best,
        }
    }
}

// ---------------------------------------------------------------------------
// What is stored (only what was changed)

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Grades<T> {
    pub small: T,
    pub balanced: T,
    pub best: T,
}

impl<T> Grades<T> {
    pub fn get(&self, g: Grade) -> &T {
        match g {
            Grade::Small => &self.small,
            Grade::Balanced => &self.balanced,
            Grade::Best => &self.best,
        }
    }
    pub fn get_mut(&mut self, g: Grade) -> &mut T {
        match g {
            Grade::Small => &mut self.small,
            Grade::Balanced => &mut self.balanced,
            Grade::Best => &mut self.best,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct ImageTune {
    /// How close it must look (SSIMULACRA2 score).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub look: Option<f64>,
    /// Never below this JPG/WebP quality (AVIF: 10 lower).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub floor: Option<u32>,
    /// PNG effort: 0 quick, 1 normal, 2 thorough.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub png: Option<u8>,
    /// Remove GPS location from the EXIF of the results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strip_gps: Option<bool>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct VideoTune {
    /// How close it must look (VMAF score).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub look: Option<f64>,
    /// Compress as "h265", "h264" or "av1".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codec: Option<String>,
    /// Compress: the short side at most (0: keep). 2160, 1440, 1080, 720.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_res: Option<u32>,
    /// Compress: frames per second at most (0: keep). 60, 30.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_fps: Option<u32>,
    /// Compress: AAC kbps. 96, 128, 192.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio: Option<u32>,
    /// "gpu" (graphics card), "cpu" (x264/x265) or "auto" (measure both).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoder: Option<String>,
    /// Encoder effort: 0 faster, 1 medium, 2 slower.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<u8>,
    /// A setting trusted for a kind of video still gets a full check every Nth video (1: always).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recheck: Option<u32>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct PdfTune {
    /// How close it must look (SSIMULACRA2 on rendered pages).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub look: Option<f64>,
    /// Pages are rendered at this DPI for the comparison. 110, 150, 200.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check_dpi: Option<u32>,
}

/// Every preset's changes.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Tunes {
    pub image: Grades<ImageTune>,
    pub video: Grades<VideoTune>,
    pub pdf: Grades<PdfTune>,
}

// ---------------------------------------------------------------------------
// What the engines get

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Image {
    pub look: f64,
    pub floor: u32,
    /// oxipng preset.
    pub png: u8,
    pub strip_gps: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    H265,
    H264,
    Av1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoder {
    Gpu,
    Cpu,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Video {
    pub look: f64,
    pub codec: Codec,
    pub max_res: u32,
    pub max_fps: u32,
    pub audio: u32,
    pub encoder: Encoder,
    pub effort: u8,
    pub recheck: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pdf {
    pub look: f64,
    pub check_dpi: u32,
}

// Ranges (the sliders in Settings use the same).
pub const IMAGE_LOOK: (f64, f64) = (50.0, 95.0);
pub const VIDEO_LOOK: (f64, f64) = (80.0, 99.0);
pub const FLOOR: (u32, u32) = (30, 90);
pub const RECHECK: (u32, u32) = (1, 20);
const RES: [u32; 5] = [0, 2160, 1440, 1080, 720];
const FPS: [u32; 3] = [0, 60, 30];
const AUDIO: [u32; 3] = [96, 128, 192];
const CHECK_DPI: [u32; 3] = [110, 150, 200];
const PNG_PRESET: [u8; 3] = [1, 2, 4];

/// Convertino's own numbers for each preset.
pub fn default_image(g: Grade) -> Image {
    Image { look: g.pick(70.0, 80.0, 87.0), floor: 40, png: g.pick(2, 2, 4), strip_gps: false }
}

pub fn default_video(g: Grade) -> Video {
    Video {
        look: g.pick(89.0, 93.0, 95.5),
        codec: Codec::H265,
        // Smaller: game recordings at 1440p/144 shrink most from these two.
        max_res: g.pick(1080, 0, 0),
        max_fps: g.pick(60, 0, 0),
        audio: 128,
        encoder: g.pick(Encoder::Gpu, Encoder::Gpu, Encoder::Auto),
        effort: 1,
        recheck: 10,
    }
}

pub fn default_pdf(g: Grade) -> Pdf {
    Pdf { look: g.pick(70.0, 80.0, 87.0), check_dpi: g.pick(110, 110, 150) }
}

fn png_effort(preset: u8) -> u8 {
    PNG_PRESET.iter().position(|p| *p == preset).unwrap_or(1) as u8
}

fn nearest(list: &[u32], v: u32) -> u32 {
    *list.iter().min_by_key(|x| (**x as i64 - v as i64).abs()).unwrap_or(&list[0])
}

fn codec_of(w: &str) -> Option<Codec> {
    match w.to_ascii_lowercase().as_str() {
        "h265" | "hevc" | "x265" => Some(Codec::H265),
        "h264" | "avc" | "x264" => Some(Codec::H264),
        "av1" => Some(Codec::Av1),
        _ => None,
    }
}

fn encoder_of(w: &str) -> Option<Encoder> {
    match w.to_ascii_lowercase().as_str() {
        "gpu" | "card" | "graphics" | "hardware" => Some(Encoder::Gpu),
        "cpu" | "processor" | "software" => Some(Encoder::Cpu),
        "auto" => Some(Encoder::Auto),
        _ => None,
    }
}

pub fn codec_word(c: Codec) -> &'static str {
    match c {
        Codec::H265 => "h265",
        Codec::H264 => "h264",
        Codec::Av1 => "av1",
    }
}

pub fn encoder_word(e: Encoder) -> &'static str {
    match e {
        Encoder::Gpu => "gpu",
        Encoder::Cpu => "cpu",
        Encoder::Auto => "auto",
    }
}

impl Tunes {
    pub fn image(&self, g: Grade) -> Image {
        let d = default_image(g);
        let t = self.image.get(g);
        Image {
            look: t.look.unwrap_or(d.look).clamp(IMAGE_LOOK.0, IMAGE_LOOK.1),
            floor: t.floor.unwrap_or(d.floor).clamp(FLOOR.0, FLOOR.1),
            png: t.png.map(|e| PNG_PRESET[e.min(2) as usize]).unwrap_or(d.png),
            strip_gps: t.strip_gps.unwrap_or(d.strip_gps),
        }
    }

    pub fn video(&self, g: Grade) -> Video {
        let d = default_video(g);
        let t = self.video.get(g);
        Video {
            look: t.look.unwrap_or(d.look).clamp(VIDEO_LOOK.0, VIDEO_LOOK.1),
            codec: t.codec.as_deref().and_then(codec_of).unwrap_or(d.codec),
            max_res: t.max_res.map(|v| nearest(&RES, v)).unwrap_or(d.max_res),
            max_fps: t.max_fps.map(|v| nearest(&FPS, v)).unwrap_or(d.max_fps),
            audio: t.audio.map(|v| nearest(&AUDIO, v)).unwrap_or(d.audio),
            encoder: t.encoder.as_deref().and_then(encoder_of).unwrap_or(d.encoder),
            effort: t.effort.map(|e| e.min(2)).unwrap_or(d.effort),
            recheck: t.recheck.unwrap_or(d.recheck).clamp(RECHECK.0, RECHECK.1),
        }
    }

    pub fn pdf(&self, g: Grade) -> Pdf {
        let d = default_pdf(g);
        let t = self.pdf.get(g);
        Pdf {
            look: t.look.unwrap_or(d.look).clamp(IMAGE_LOOK.0, IMAGE_LOOK.1),
            check_dpi: t.check_dpi.map(|v| nearest(&CHECK_DPI, v)).unwrap_or(d.check_dpi),
        }
    }

    /// Out-of-range values back in range, values equal to the default
    /// forgotten (so a later change of default still applies), and the look
    /// scores kept in order: Smaller ≤ Balanced ≤ Best.
    pub fn clamped(mut self) -> Self {
        for g in Grade::ALL {
            let (r, d) = (self.image(g), default_image(g));
            let t = self.image.get_mut(g);
            t.look = t.look.map(|_| r.look).filter(|v| *v != d.look);
            t.floor = t.floor.map(|_| r.floor).filter(|v| *v != d.floor);
            t.png = t.png.map(|_| png_effort(r.png)).filter(|v| *v != png_effort(d.png));
            t.strip_gps = t.strip_gps.filter(|v| *v != d.strip_gps);

            let (r, d) = (self.video(g), default_video(g));
            let t = self.video.get_mut(g);
            t.look = t.look.map(|_| r.look).filter(|v| *v != d.look);
            t.codec = t.codec.as_ref().map(|_| codec_word(r.codec).to_string()).filter(|v| *v != codec_word(d.codec));
            t.max_res = t.max_res.map(|_| r.max_res).filter(|v| *v != d.max_res);
            t.max_fps = t.max_fps.map(|_| r.max_fps).filter(|v| *v != d.max_fps);
            t.audio = t.audio.map(|_| r.audio).filter(|v| *v != d.audio);
            t.encoder = t.encoder.as_ref().map(|_| encoder_word(r.encoder).to_string()).filter(|v| *v != encoder_word(d.encoder));
            t.effort = t.effort.map(|_| r.effort).filter(|v| *v != d.effort);
            t.recheck = t.recheck.map(|_| r.recheck).filter(|v| *v != d.recheck);

            let (r, d) = (self.pdf(g), default_pdf(g));
            let t = self.pdf.get_mut(g);
            t.look = t.look.map(|_| r.look).filter(|v| *v != d.look);
            t.check_dpi = t.check_dpi.map(|_| r.check_dpi).filter(|v| *v != d.check_dpi);
        }
        self.keep_order();
        self
    }

    /// A preset's look score can't pass its neighbour's: Smaller is moved
    /// down to Balanced, Best up to Balanced (Balanced is the anchor).
    fn keep_order(&mut self) {
        let (s, b, x) = (self.image(Grade::Small).look, self.image(Grade::Balanced).look, self.image(Grade::Best).look);
        if s > b {
            self.image.small.look = Some(b);
        }
        if x < b {
            self.image.best.look = Some(b);
        }
        let (s, b, x) = (self.video(Grade::Small).look, self.video(Grade::Balanced).look, self.video(Grade::Best).look);
        if s > b {
            self.video.small.look = Some(b);
        }
        if x < b {
            self.video.best.look = Some(b);
        }
        let (s, b, x) = (self.pdf(Grade::Small).look, self.pdf(Grade::Balanced).look, self.pdf(Grade::Best).look);
        if s > b {
            self.pdf.small.look = Some(b);
        }
        if x < b {
            self.pdf.best.look = Some(b);
        }
    }

    /// One-off changes for a job (command line `--tune`, `--look`…, MCP
    /// `tune`), applied to the preset `g` of every kind of file that has
    /// the setting. Keys: look, floor, png, strip-gps, codec, max-res,
    /// max-fps, audio, encoder, effort, recheck, check-dpi.
    pub fn apply(&mut self, g: Grade, key: &str, value: &str) -> Result<(), String> {
        let key = key.trim().to_ascii_lowercase().replace('_', "-");
        let v = value.trim();
        let num = || v.trim_end_matches(['p', 'P']).trim_end_matches("fps").trim_end_matches("kbps").trim().parse::<f64>().map_err(|_| format!("{key} needs a number, not \"{v}\"."));
        let word3 = |words: [&str; 3]| -> Result<u8, String> {
            if let Some(i) = words.iter().position(|w| w.eq_ignore_ascii_case(v)) {
                return Ok(i as u8);
            }
            match v.parse::<u8>() {
                Ok(n) if n <= 2 => Ok(n),
                _ => Err(format!("{key} is {}, {} or {}, not \"{v}\".", words[0], words[1], words[2])),
            }
        };
        match key.as_str() {
            "look" => {
                let n = num()?;
                self.image.get_mut(g).look = Some(n);
                self.video.get_mut(g).look = Some(n);
                self.pdf.get_mut(g).look = Some(n);
            }
            "floor" => self.image.get_mut(g).floor = Some(num()? as u32),
            "png" => self.image.get_mut(g).png = Some(word3(["quick", "normal", "thorough"])?),
            "strip-gps" | "gps" | "remove-gps" => {
                let on = !matches!(v.to_ascii_lowercase().as_str(), "false" | "no" | "off" | "0" | "keep");
                self.image.get_mut(g).strip_gps = Some(on);
            }
            "codec" => {
                codec_of(v).ok_or_else(|| format!("codec is h265, h264 or av1, not \"{v}\"."))?;
                self.video.get_mut(g).codec = Some(v.to_ascii_lowercase());
            }
            "max-res" | "res" => {
                let n = if v.eq_ignore_ascii_case("keep") { 0.0 } else if v.eq_ignore_ascii_case("4k") { 2160.0 } else { num()? };
                self.video.get_mut(g).max_res = Some(n as u32);
            }
            "max-fps" | "fps" => {
                let n = if v.eq_ignore_ascii_case("keep") { 0.0 } else { num()? };
                self.video.get_mut(g).max_fps = Some(n as u32);
            }
            "audio" => self.video.get_mut(g).audio = Some(num()? as u32),
            "encoder" | "encode-on" => {
                encoder_of(v).ok_or_else(|| format!("encoder is gpu, cpu or auto, not \"{v}\"."))?;
                self.video.get_mut(g).encoder = Some(v.to_ascii_lowercase());
            }
            "effort" => self.video.get_mut(g).effort = Some(word3(["faster", "medium", "slower"])?),
            "recheck" => self.video.get_mut(g).recheck = Some(num()? as u32),
            "check-dpi" | "dpi" => self.pdf.get_mut(g).check_dpi = Some(num()? as u32),
            other => return Err(format!("Unknown tune setting \"{other}\". Use look, floor, png, strip-gps, codec, max-res, max-fps, audio, encoder, effort, recheck or check-dpi.")),
        }
        Ok(())
    }

    /// `look=91,encoder=cpu,strip-gps` (a key alone means on).
    #[cfg(test)]
    pub fn apply_list(&mut self, g: Grade, list: &str) -> Result<(), String> {
        for part in list.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let (k, v) = part.split_once('=').unwrap_or((part, "on"));
            self.apply(g, k, v)?;
        }
        Ok(())
    }

    /// Exported presets (Settings › Quality › Export presets…).
    pub fn export(&self) -> serde_json::Value {
        serde_json::json!({ "convertino": "presets", "version": 1, "tune": self })
    }

    /// An exported file, or a bare `tune` object.
    pub fn import(text: &str) -> Result<Tunes, String> {
        let v: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("That isn't a presets file ({e})."))?;
        let inner = v.get("tune").cloned().unwrap_or(v);
        let t: Tunes = serde_json::from_value(inner).map_err(|e| format!("That presets file isn't valid: {e}"))?;
        Ok(t.clamped())
    }
}

/// Convertino's defaults for every preset, for Settings (what Reset goes back to).
pub fn defaults_json() -> serde_json::Value {
    resolved_json(&Tunes::default())
}

/// Every preset as `t` resolves it: {grade: {image, video, pdf}}, with the
/// same keys (and choice indexes) Settings uses.
pub fn resolved_json(t: &Tunes) -> serde_json::Value {
    let mut out = serde_json::Map::new();
    for g in Grade::ALL {
        let (i, v, p) = (t.image(g), t.video(g), t.pdf(g));
        out.insert(
            g.word().into(),
            serde_json::json!({
                "image": { "look": i.look, "floor": i.floor, "png": png_effort(i.png), "stripGps": i.strip_gps },
                "video": { "look": v.look, "codec": codec_word(v.codec), "maxRes": v.max_res, "maxFps": v.max_fps, "audio": v.audio,
                           "encoder": encoder_word(v.encoder), "effort": v.effort, "recheck": v.recheck },
                "pdf": { "look": p.look, "checkDpi": p.check_dpi },
            }),
        );
    }
    serde_json::Value::Object(out)
}

/// What importing `to` over `from` would change, in words ("Video › Balanced: look 93 → 91").
pub fn changes(from: &Tunes, to: &Tunes) -> Vec<String> {
    let (a, b) = (resolved_json(from), resolved_json(to));
    let mut out = Vec::new();
    for g in Grade::ALL {
        for (kind, name) in [("image", "Images"), ("video", "Video"), ("pdf", "PDF")] {
            let (Some(x), Some(y)) = (a[g.word()][kind].as_object(), b[g.word()][kind].as_object()) else { continue };
            for (k, v) in y {
                let was = &x[k];
                if was != v {
                    let preset = match g {
                        Grade::Small => "Smaller",
                        Grade::Balanced => "Balanced",
                        Grade::Best => "Best",
                    };
                    out.push(format!("{name} › {preset}: {k} {was} → {v}").replace('"', ""));
                }
            }
        }
    }
    out
}

/// A short fingerprint of a preset as resolved, so results recorded under
/// other settings aren't counted (stats.rs).
pub fn signature(kind: &str, t: &Tunes, g: Grade) -> String {
    match kind {
        "image" => format!("{:?}", t.image(g)),
        "video" => format!("{:?}", t.video(g)),
        _ => format!("{:?}", t.pdf(g)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_the_old_numbers() {
        let t = Tunes::default();
        assert_eq!(t.image(Grade::Balanced).look, 80.0);
        assert_eq!(t.image(Grade::Best).png, 4);
        assert_eq!(t.video(Grade::Small).look, 89.0);
        assert_eq!(t.video(Grade::Best).encoder, Encoder::Auto);
        assert_eq!(t.video(Grade::Balanced).max_res, 0);
        assert_eq!(t.video(Grade::Small).max_res, 1080);
        assert_eq!(t.pdf(Grade::Best).check_dpi, 150);
    }

    #[test]
    fn order_is_kept_and_defaults_forgotten() {
        let mut t = Tunes::default();
        t.video.small.look = Some(97.0);
        t.image.best.look = Some(60.0);
        t.image.balanced.floor = Some(40); // the default: not kept
        t.video.balanced.max_res = Some(1000); // nearest choice: 1080
        let c = t.clamped();
        assert_eq!(c.video(Grade::Small).look, 93.0);
        assert_eq!(c.image(Grade::Best).look, 80.0);
        assert_eq!(c.image.balanced.floor, None);
        assert_eq!(c.video.balanced.max_res, Some(1080));
        // Stored compactly.
        let json = serde_json::to_string(&Tunes::default()).unwrap();
        assert!(!json.contains("look"), "{json}");
    }

    #[test]
    fn one_off_changes() {
        let mut t = Tunes::default();
        t.apply_list(Grade::Balanced, "look=91, encoder=cpu, max-res=1080p, max-fps=60, strip-gps, effort=slower, png=thorough").unwrap();
        let v = t.video(Grade::Balanced);
        assert_eq!((v.look, v.encoder, v.max_res, v.max_fps, v.effort), (91.0, Encoder::Cpu, 1080, 60, 2));
        assert!(t.image(Grade::Balanced).strip_gps);
        assert_eq!(t.image(Grade::Balanced).png, 4);
        assert_eq!(t.image(Grade::Balanced).look, 91.0);
        assert!(t.apply(Grade::Balanced, "codec", "mpeg2").is_err());
        assert!(t.apply(Grade::Balanced, "colour", "red").is_err());
    }

    #[test]
    fn export_and_import() {
        let mut t = Tunes::default();
        t.video.balanced.look = Some(91.5);
        let text = t.export().to_string();
        assert_eq!(Tunes::import(&text).unwrap(), t);
        assert_eq!(Tunes::import(&serde_json::to_string(&t).unwrap()).unwrap(), t);
        assert!(Tunes::import("not json").is_err());
        assert_eq!(changes(&Tunes::default(), &t), vec!["Video › Balanced: look 93.0 → 91.5".to_string()]);
    }
}
