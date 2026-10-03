//! Camera RAW photos (DNG, CR2/CR3, NEF, ARW, RAF, ORF, RW2 and others).
//!
//! A RAW file is "developed" once into a 16-bit TIFF (ImageMagick's built-in
//! LibRaw: camera white balance, sRGB, turned the right way up), and that
//! picture then goes through the same JPG / PNG / WebP / AVIF / Compress code
//! as any other picture. If ImageMagick can't read the file (very new formats,
//! JPEG XL DNGs from phones), the `rawler` crate develops it instead.
//!
//! **Camera JPG** takes the full-size picture the camera stored inside the
//! RAW (its own look, picture style included), without developing anything.
//! Cameras that only store a small preview get a developed picture instead,
//! with a note.
//!
//! The date, camera, lens, exposure and GPS are copied into JPG, PNG, WebP
//! and TIFF results (EXIF), with the orientation reset since the pixels are
//! already turned.

use crate::convert::{Step, TempDir};
use crate::procs;
use crate::tools::{self, Tool};
use rawler::decoders::{RawDecodeParams, RawMetadata};
use rawler::rawsource::RawSource;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Extensions of camera RAW files Convertino reads.
pub const RAW_EXTS: &[&str] = &[
    "dng", "cr2", "cr3", "crw", "nef", "nrw", "arw", "srf", "sr2", "raf", "orf", "rw2", "pef", "srw", "x3f", "3fr", "iiq", "rwl", "erf", "kdc",
    "dcr", "mos", "mrw",
];

pub fn is_raw(p: &Path) -> bool {
    p.extension().map(|e| RAW_EXTS.contains(&e.to_string_lossy().to_lowercase().as_str())).unwrap_or(false)
}

/// Runs a rawler call, turning a panic on a strange file into an error.
fn guarded<T>(what: &str, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|_| Err(format!("{what} failed on this file")))
}

fn rawler_err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// What the camera recorded.
#[derive(Debug, Clone, Default)]
pub struct Meta {
    pub make: String,
    pub model: String,
    /// Sensor size in pixels.
    pub width: usize,
    pub height: usize,
    exif: Option<rawler::exif::Exif>,
    lens: Option<String>,
}

impl Meta {
    pub fn megapixels(&self) -> f64 {
        (self.width * self.height) as f64 / 1e6
    }
}

/// The camera, size and EXIF of a RAW file (fast: no developing).
pub fn meta(path: &Path) -> Result<Meta, String> {
    guarded("Reading the RAW file", || {
        let src = RawSource::new(path).map_err(rawler_err)?;
        let dec = rawler::get_decoder(&src).map_err(rawler_err)?;
        let params = RawDecodeParams::default();
        let md: Option<RawMetadata> = dec.raw_metadata(&src, &params).ok();
        let raw = dec.raw_image(&src, &params, true).ok();
        let (width, height) = raw.as_ref().map(|r| (r.width, r.height)).unwrap_or((0, 0));
        Ok(Meta {
            make: md.as_ref().map(|m| m.make.clone()).unwrap_or_default(),
            model: md.as_ref().map(|m| m.model.clone()).unwrap_or_default(),
            width,
            height,
            lens: md.as_ref().and_then(|m| m.lens.as_ref().map(|l| l.lens_model.clone())),
            exif: md.map(|m| m.exif),
        })
    })
}

/// "RAW photo · 24 MP" for the wheel's centre.
pub fn describe(path: &Path) -> Option<String> {
    let m = meta(path).ok()?;
    if m.width == 0 {
        return None;
    }
    let mp = m.megapixels();
    Some(if mp >= 10.0 { format!("RAW photo · {:.0} MP", mp) } else { format!("RAW photo · {:.1} MP", mp) })
}

/// Longest wait for one RAW to develop (a 100 MP file on a slow PC).
const DEVELOP_LIMIT: Duration = Duration::from_secs(5 * 60);

/// Develops `input` into a 16-bit picture in `dir`, the right way up.
pub fn develop(input: &Path, dir: &Path) -> Result<PathBuf, String> {
    let out = dir.join("developed.tiff");
    let first = (|| -> Result<(), String> {
        let exe = tools::require(Tool::Magick)?;
        let mut cmd = tools::command(&exe);
        // Camera white balance (what the photographer saw), sRGB output.
        cmd.args(["-define", "dng:use-camera-wb=true", "-define", "dng:output-color=1"]);
        cmd.arg(input);
        cmd.args(["-auto-orient", "-depth", "16", "-compress", "none"]);
        cmd.arg(&out);
        procs::output(&mut cmd, "ImageMagick", Some(DEVELOP_LIMIT))?;
        if out.metadata().map(|m| m.len() > 0).unwrap_or(false) { Ok(()) } else { Err("ImageMagick wrote nothing".into()) }
    })();
    match first {
        Ok(()) => Ok(out),
        Err(e) if e == procs::CANCELLED => Err(e),
        Err(e) => {
            log::warn!("raw: ImageMagick couldn't develop {} ({e}); trying rawler", input.display());
            let png = dir.join("developed.png");
            develop_rawler(input, &png).map_err(|e2| {
                log::warn!("raw: rawler couldn't either: {e2}");
                "Convertino can't develop this RAW file yet. Try Camera JPG for the picture the camera stored inside it.".to_string()
            })?;
            Ok(png)
        }
    }
}

fn develop_rawler(input: &Path, out: &Path) -> Result<(), String> {
    let orientation = meta(input).ok().and_then(|m| m.exif.and_then(|e| e.orientation)).unwrap_or(1);
    let img = guarded("Developing", || rawler::analyze::raw_to_srgb(input, &RawDecodeParams::default()).map_err(rawler_err))?;
    let img = image::DynamicImage::ImageRgb16(img.to_rgb16());
    orient(img, orientation).save_with_format(out, image::ImageFormat::Png).map_err(|e| e.to_string())
}

/// Turns pixels the way an EXIF orientation says.
fn orient(img: image::DynamicImage, orientation: u16) -> image::DynamicImage {
    match orientation {
        2 => img.fliph(),
        3 => img.rotate180(),
        4 => img.flipv(),
        5 => img.rotate90().fliph(),
        6 => img.rotate90(),
        7 => img.rotate270().fliph(),
        8 => img.rotate270(),
        _ => img,
    }
}

/// The picture the camera stored inside the RAW, as a PNG in `dir`, if it
/// is big enough to be worth having (at least half the sensor's width and
/// 1200 px on its long side). None: too small or missing.
pub fn camera_picture(input: &Path, dir: &Path) -> Result<Option<PathBuf>, String> {
    let m = meta(input).unwrap_or_default();
    let img = guarded("Reading the camera's picture", || {
        let src = RawSource::new(input).map_err(rawler_err)?;
        let dec = rawler::get_decoder(&src).map_err(rawler_err)?;
        dec.preview_image(&src, &RawDecodeParams::default()).map_err(rawler_err)
    })
    .unwrap_or(None);
    let Some(img) = img else { return Ok(None) };
    let long = img.width().max(img.height()) as usize;
    let sensor_long = m.width.max(m.height);
    if long < 1200 || (sensor_long > 0 && long * 2 < sensor_long) {
        log::info!("raw: camera picture of {} is only {}x{}", input.display(), img.width(), img.height());
        return Ok(None);
    }
    let orientation = m.exif.as_ref().and_then(|e| e.orientation).unwrap_or(1);
    let out = dir.join("camera.png");
    orient(image::DynamicImage::ImageRgb8(img.to_rgb8()), orientation).save_with_format(&out, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(Some(out))
}

/// Runs a step whose inputs include RAW files: each is developed first, then
/// the step runs on the developed pictures (the output keeps the RAW's name),
/// and the camera's EXIF goes into the result.
pub fn run_developed(
    step: &Step,
    output: &Path,
    progress: &mut dyn FnMut(f64),
    run_op: fn(&Step, &Path, &mut dyn FnMut(f64)) -> Result<PathBuf, String>,
) -> Result<PathBuf, String> {
    let tmp = TempDir::new("raw")?;
    let mut dev = step.clone();
    let n = step.inputs.len().max(1) as f64;
    for (k, input) in step.inputs.iter().enumerate() {
        if !is_raw(input) {
            continue;
        }
        if procs::cancelled_here() {
            return Err(procs::CANCELLED.into());
        }
        let dir = tmp.0.join(k.to_string());
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        dev.inputs[k] = develop(input, &dir)?;
        progress(0.35 * (k + 1) as f64 / n);
    }
    let made = run_op(&dev, output, &mut |p| progress(0.35 + 0.65 * p))?;
    copy_exif(&step.inputs[0], &made);
    Ok(made)
}

/// Camera JPG: the camera's own picture, encoded the way JPGs always are
/// (smallest file that looks the same); developed when there's none.
pub fn camera_jpg(input: &Path, output: &Path, level: Option<crate::look::Level>, fixed: u32, progress: &mut dyn FnMut(f64)) -> Result<PathBuf, String> {
    let tmp = TempDir::new("rawcam")?;
    let picture = match camera_picture(input, &tmp.0)? {
        Some(p) => p,
        None => {
            crate::convert::note(format!(
                "{}: the camera didn't store a full-size picture, so it was developed from the RAW instead.",
                input.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
            ));
            develop(input, &tmp.0)?
        }
    };
    progress(0.4);
    crate::look::picture(&picture, &[], crate::look::Lossy::Jpg, level, fixed, output)?;
    copy_exif(input, output);
    Ok(output.to_path_buf())
}

// ---------- EXIF ----------

/// A WebP with `tiff` (EXIF, starting with its TIFF header) added: a simple
/// WebP (one VP8 / VP8L chunk) becomes an extended one (VP8X) first.
/// See https://developers.google.com/speed/webp/docs/riff_container
pub fn webp_with_exif(bytes: &[u8], tiff: &[u8], (w, h): (u32, u32)) -> Result<Vec<u8>, String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return Err("not a WebP file".into());
    }
    let mut chunks: Vec<([u8; 4], Vec<u8>)> = Vec::new();
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id: [u8; 4] = bytes[at..at + 4].try_into().unwrap();
        let len = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        let end = at + 8 + len;
        if end > bytes.len() {
            return Err("damaged WebP chunk".into());
        }
        chunks.push((id, bytes[at + 8..end].to_vec()));
        at = end + (len & 1);
    }
    chunks.retain(|(id, _)| id != b"EXIF");
    match chunks.first().map(|(id, _)| *id) {
        Some(id) if &id == b"VP8X" => chunks[0].1[0] |= 0x08,
        Some(id) if &id == b"VP8 " || &id == b"VP8L" => {
            // VP8L: bit 28 after the 0x2f signature says whether alpha is used.
            let alpha = &id == b"VP8L" && chunks[0].1.len() >= 5 && (u32::from_le_bytes(chunks[0].1[1..5].try_into().unwrap()) >> 28) & 1 == 1;
            let mut x = vec![0x08 | if alpha { 0x10 } else { 0 }, 0, 0, 0];
            x.extend_from_slice(&(w - 1).to_le_bytes()[..3]);
            x.extend_from_slice(&(h - 1).to_le_bytes()[..3]);
            chunks.insert(0, (*b"VP8X", x));
        }
        _ => return Err("unexpected WebP layout".into()),
    }
    chunks.push((*b"EXIF", tiff.to_vec()));
    let mut body = b"WEBP".to_vec();
    for (id, data) in &chunks {
        body.extend_from_slice(id);
        body.extend_from_slice(&(data.len() as u32).to_le_bytes());
        body.extend_from_slice(data);
        if data.len() & 1 == 1 {
            body.push(0);
        }
    }
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

fn dims_of(bytes: &[u8]) -> Result<(u32, u32), String> {
    image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().map_err(|e| e.to_string())?.into_dimensions().map_err(|e| e.to_string())
}

fn ur(r: &rawler::formats::tiff::Rational) -> little_exif::rational::uR64 {
    little_exif::rational::uR64 { nominator: r.n, denominator: r.d.max(1) }
}

/// Copies what the camera recorded into `output` (JPG, PNG, WebP, TIFF).
/// Best effort: a picture without EXIF is still a good picture.
pub fn copy_exif(raw: &Path, output: &Path) {
    use little_exif::exif_tag::ExifTag;
    use little_exif::filetype::FileExtension;
    let ext = output.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let kind = match ext.as_str() {
        "jpg" | "jpeg" => FileExtension::JPEG,
        "png" => FileExtension::PNG { as_zTXt_chunk: false },
        "webp" => FileExtension::WEBP,
        "tif" | "tiff" => FileExtension::TIFF,
        _ => return,
    };
    let Ok(m) = meta(raw) else { return };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<(), String> {
        let mut bytes = std::fs::read(output).map_err(|e| e.to_string())?;
        // Start from what the file has (a TIFF's own tags must stay).
        let mut md = little_exif::metadata::Metadata::new_from_vec(&bytes, kind).unwrap_or_else(|_| little_exif::metadata::Metadata::new());
        if !m.make.is_empty() {
            md.set_tag(ExifTag::Make(m.make.clone()));
        }
        if !m.model.is_empty() {
            md.set_tag(ExifTag::Model(m.model.clone()));
        }
        md.set_tag(ExifTag::Orientation(vec![1]));
        md.set_tag(ExifTag::Software(format!("Convertino {}", env!("CARGO_PKG_VERSION"))));
        if let Some(lens) = m.lens.clone().filter(|l| l.chars().any(|c| c.is_alphanumeric())) {
            md.set_tag(ExifTag::LensModel(lens));
        }
        if let Some(e) = &m.exif {
            if let Some(d) = e.date_time_original.clone().or_else(|| e.create_date.clone()) {
                md.set_tag(ExifTag::DateTimeOriginal(d.clone()));
                md.set_tag(ExifTag::CreateDate(d));
            }
            if let Some(o) = e.offset_time_original.clone() {
                md.set_tag(ExifTag::OffsetTimeOriginal(o));
            }
            if let Some(r) = &e.exposure_time {
                md.set_tag(ExifTag::ExposureTime(vec![ur(r)]));
            }
            if let Some(r) = &e.fnumber {
                md.set_tag(ExifTag::FNumber(vec![ur(r)]));
            }
            if let Some(iso) = e.iso_speed_ratings {
                md.set_tag(ExifTag::ISO(vec![iso]));
            }
            if let Some(r) = &e.focal_length {
                md.set_tag(ExifTag::FocalLength(vec![ur(r)]));
            }
            if let Some(l) = e.lens_make.clone().filter(|l| l.chars().any(|c| c.is_alphanumeric())) {
                md.set_tag(ExifTag::LensMake(l));
            }
            if let Some(g) = &e.gps {
                if let (Some(lat), Some(lat_ref), Some(lon), Some(lon_ref)) = (&g.gps_latitude, &g.gps_latitude_ref, &g.gps_longitude, &g.gps_longitude_ref) {
                    md.set_tag(ExifTag::GPSLatitudeRef(lat_ref.clone()));
                    md.set_tag(ExifTag::GPSLatitude(lat.iter().map(ur).collect()));
                    md.set_tag(ExifTag::GPSLongitudeRef(lon_ref.clone()));
                    md.set_tag(ExifTag::GPSLongitude(lon.iter().map(ur).collect()));
                    if let (Some(alt), Some(alt_ref)) = (&g.gps_altitude, g.gps_altitude_ref) {
                        md.set_tag(ExifTag::GPSAltitudeRef(vec![alt_ref]));
                        md.set_tag(ExifTag::GPSAltitude(vec![ur(alt)]));
                    }
                }
            }
        }
        // Into a copy first, so a writing problem can never damage the result.
        let before = dims_of(&bytes)?;
        if matches!(kind, FileExtension::WEBP) {
            // little_exif can't turn a simple WebP into an extended one; done here.
            let app1 = md.as_u8_vec(FileExtension::JPEG).map_err(|e| e.to_string())?;
            let at = app1.windows(6).position(|w| w == b"Exif\0\0").ok_or("no EXIF block")?;
            bytes = webp_with_exif(&bytes, &app1[at + 6..], before)?;
        } else {
            md.write_to_vec(&mut bytes, kind).map_err(|e| e.to_string())?;
        }
        // The picture must read back the same (a lossless WebP once came back
        // with width and height swapped): otherwise keep it without EXIF.
        if dims_of(&bytes)? != before {
            return Err("the picture changed while adding EXIF".into());
        }
        let tmp = output.with_extension(format!("{ext}.exif"));
        std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, output).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            e.to_string()
        })
    }));
    match result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => log::warn!("raw: EXIF not copied into {}: {e}", output.display()),
        Err(_) => log::warn!("raw: EXIF writer gave up on {}", output.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_extensions() {
        assert!(is_raw(Path::new("IMG_0412.CR3")));
        assert!(is_raw(Path::new("a/b/PXL_0903.dng")));
        assert!(is_raw(Path::new("x.NEF")));
        assert!(!is_raw(Path::new("x.jpg")));
        assert!(!is_raw(Path::new("dng")));
    }

    #[test]
    fn every_raw_extension_is_a_raw_photo_on_the_wheel() {
        for ext in RAW_EXTS {
            let fam = crate::wheel::family_of(ext).map(|(id, _)| id);
            assert_eq!(fam.as_deref(), Some("raw"), "{ext}");
            let names: Vec<String> = crate::wheel::conversions_for(ext).into_iter().map(|c| c.name).collect();
            for want in ["jpg", "png", "tiff", "avif", "webp", "compress", "pdf", "camera-jpg"] {
                assert!(names.iter().any(|n| n == want), "{ext}: {want} missing from {names:?}");
            }
        }
    }

    /// RAW samples: $CONVERTINO_RAW_SAMPLES, or test-files/raw (fetched by
    /// ci/fetch-raw-samples.sh; public domain, from raw.pixls.us).
    fn samples() -> Vec<PathBuf> {
        let dir = std::env::var_os("CONVERTINO_RAW_SAMPLES")
            .map(PathBuf::from)
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../test-files/raw"));
        let mut v: Vec<PathBuf> = std::fs::read_dir(dir).map(|r| r.flatten().map(|e| e.path()).filter(|p| is_raw(p)).collect()).unwrap_or_default();
        v.sort();
        v
    }

    #[test]
    fn camera_details_are_read() {
        for p in samples() {
            let m = meta(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
            assert!(!m.make.is_empty(), "{}: no make", p.display());
            assert!(m.width > 500 && m.height > 300, "{}: {}x{}", p.display(), m.width, m.height);
            assert!(describe(&p).unwrap().starts_with("RAW photo · "));
        }
    }

    fn dims(p: &Path) -> (u32, u32) {
        image::image_dimensions(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
    }

    /// Mean of each channel: a developed photo is neither black, blown out nor green.
    fn colour_check(p: &Path) {
        let img = image::open(p).unwrap().thumbnail(256, 256).to_rgb8();
        let n = (img.width() * img.height()) as f64;
        let mut sum = [0f64; 3];
        for px in img.pixels() {
            for c in 0..3 {
                sum[c] += px[c] as f64;
            }
        }
        let mean: Vec<f64> = sum.iter().map(|s| s / n).collect();
        let avg = (mean[0] + mean[1] + mean[2]) / 3.0;
        assert!(avg > 20.0 && avg < 240.0, "{}: brightness {avg:.0}", p.display());
        // Green cast (white balance not applied) shows as green far above red and blue.
        assert!(mean[1] < 1.6 * mean[0].max(mean[2]) + 10.0, "{}: green cast {mean:?}", p.display());
    }

    #[test]
    fn real_raws_develop_the_right_way_up() {
        let samples = samples();
        if samples.is_empty() {
            eprintln!("no RAW samples: skipped (run ci/fetch-raw-samples.sh)");
            return;
        }
        if tools::find(Tool::Magick).is_none() {
            eprintln!("no ImageMagick: skipped");
            return;
        }
        for p in samples {
            let tmp = TempDir::new("rawtest").unwrap();
            let dev = develop(&p, &tmp.0).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
            let m = meta(&p).unwrap();
            let (w, h) = dims(&dev);
            // About the sensor's size (cameras crop a few edge pixels).
            let (long, sensor) = (w.max(h) as f64, m.width.max(m.height) as f64);
            assert!(long > sensor * 0.85 && long <= sensor * 1.05, "{}: {w}x{h} from a {}x{} sensor", p.display(), m.width, m.height);
            colour_check(&dev);
        }
    }

    #[test]
    fn exif_never_damages_a_picture() {
        let Some(p) = samples().into_iter().next() else { return };
        let tmp = TempDir::new("rawexif2").unwrap();
        for (w, h) in [(64, 48), (31, 77)] {
            let out = tmp.0.join(format!("lossless-{w}.webp"));
            image::DynamicImage::new_rgb8(w, h).save(&out).unwrap();
            copy_exif(&p, &out);
            assert_eq!(dims(&out), (w, h));
        }
    }

    #[test]
    fn exif_goes_into_jpg_png_webp() {
        let Some(p) = samples().into_iter().find(|p| meta(p).ok().and_then(|m| m.exif).and_then(|e| e.date_time_original).is_some()) else {
            eprintln!("no RAW samples with a date: skipped");
            return;
        };
        let m = meta(&p).unwrap();
        let tmp = TempDir::new("rawexif").unwrap();
        let img = image::DynamicImage::new_rgb8(64, 48);
        let png = tmp.0.join("src.png");
        img.save(&png).unwrap();
        for ext in ["jpg", "png", "webp"] {
            let out = tmp.0.join(format!("x.{ext}"));
            // Made the way Convertino makes them (ImageMagick), else by the image crate.
            let by_magick = tools::find(Tool::Magick)
                .map(|m| tools::command(&m).arg(&png).args(["-quality", "80"]).arg(&out).status().map(|s| s.success()).unwrap_or(false))
                .unwrap_or(false);
            if !by_magick {
                img.save(&out).unwrap();
            }
            copy_exif(&p, &out);
            let md = little_exif::metadata::Metadata::new_from_path(&out).unwrap_or_else(|e| panic!("{ext}: {e}"));
            let make = md.get_tag(&little_exif::exif_tag::ExifTag::Make(String::new())).next().cloned();
            assert!(matches!(make, Some(little_exif::exif_tag::ExifTag::Make(ref s)) if s.trim_end_matches('\0') == m.make), "{ext}: {make:?}");
            // Still a valid picture.
            assert_eq!(dims(&out), (64, 48), "{ext}");
        }
    }
}
