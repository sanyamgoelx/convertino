//! "The smallest file that still looks the same."
//!
//! Lossy pictures (JPG, WebP, AVIF, PDF page images) are encoded at a few
//! quality values and compared with the original using SSIMULACRA2, a
//! measure of how alike two pictures look to people (100 = identical,
//! 90 = no visible difference at normal size, 70 = high quality). The lowest
//! quality that still reaches the level chosen in Settings wins. The search
//! runs on a crop of the picture, so it stays fast for big photos; the full
//! picture is then encoded once.
//!
//! PNGs are made smaller without changing a pixel (oxipng).
//! PDF Compress compares rendered pages (see `pdf_score`).

use crate::convert::{exec, s, TempDir};
use crate::tools::{self, Tool};
use std::path::{Path, PathBuf};
use std::process::Stdio;
pub use crate::tune::Grade;

/// How close to the original a result must look, and the other choices
/// behind a preset (Settings › Quality, tuned per preset; see tune.rs).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Level {
    pub grade: Grade,
    /// SSIMULACRA2 score a picture (or a PDF page) must reach.
    pub target: f64,
    /// Lowest JPG/WebP quality searched (AVIF: 10 lower).
    pub floor: u32,
    /// oxipng preset.
    pub png: u8,
    /// Remove GPS location from the results' EXIF.
    pub strip_gps: bool,
    /// PDF Compress: pages are compared at this DPI.
    pub check_dpi: u32,
}

#[allow(non_upper_case_globals)]
impl Level {
    /// The presets as Convertino ships them.
    #[cfg(test)]
    pub const Small: Level = Level { grade: Grade::Small, target: 70.0, floor: 40, png: 2, strip_gps: false, check_dpi: 110 };
    pub const Balanced: Level = Level { grade: Grade::Balanced, target: 80.0, floor: 40, png: 2, strip_gps: false, check_dpi: 110 };
    pub const Best: Level = Level { grade: Grade::Best, target: 87.0, floor: 40, png: 4, strip_gps: false, check_dpi: 150 };

    /// A picture preset as tuned in `q`.
    pub fn image(q: &crate::settings::Quality, g: Grade) -> Level {
        let i = q.tune.image(g);
        Level { grade: g, target: i.look, floor: i.floor, png: i.png, strip_gps: i.strip_gps, check_dpi: 110 }
    }

    /// A PDF Compress preset as tuned in `q` (pictures inside keep their GPS).
    pub fn pdf(q: &crate::settings::Quality, g: Grade) -> Level {
        let p = q.tune.pdf(g);
        let png = q.tune.image(g).png;
        Level { grade: g, target: p.look, floor: 40, png, strip_gps: false, check_dpi: p.check_dpi }
    }

    /// SSIMULACRA2 score a picture must reach.
    pub fn picture_target(self) -> f64 {
        self.target
    }

    fn png_preset(self) -> u8 {
        self.png
    }
}

/// The side of the square crop the quality search runs on.
const CROP: u32 = 768;

/// An 8-bit RGB picture.
#[derive(Clone)]
pub struct Pixels {
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<u8>,
}

impl Pixels {
    fn to_ssim(&self) -> Result<ssimulacra2::Rgb, String> {
        let data: Vec<[f32; 3]> = self
            .rgb
            .chunks_exact(3)
            .map(|p| [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0])
            .collect();
        ssimulacra2::Rgb::new(
            data,
            self.width,
            self.height,
            ssimulacra2::TransferCharacteristic::SRGB,
            ssimulacra2::ColorPrimaries::BT709,
        )
        .map_err(|e| format!("{e:?}"))
    }
}

/// Reads a binary PPM (P6, 8-bit), as `magick … ppm:-` writes it.
fn parse_ppm(data: &[u8]) -> Result<Pixels, String> {
    let mut fields = Vec::new();
    let mut i = 0;
    while fields.len() < 4 {
        while i < data.len() && data[i].is_ascii_whitespace() {
            i += 1;
        }
        if i < data.len() && data[i] == b'#' {
            while i < data.len() && data[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        let start = i;
        while i < data.len() && !data[i].is_ascii_whitespace() {
            i += 1;
        }
        if start == i {
            return Err("not a PPM picture".into());
        }
        fields.push(String::from_utf8_lossy(&data[start..i]).into_owned());
    }
    i += 1; // the single whitespace after maxval
    if fields[0] != "P6" || fields[3] != "255" {
        return Err(format!("unexpected PPM header {fields:?}"));
    }
    let width: usize = fields[1].parse().map_err(|_| "bad PPM width")?;
    let height: usize = fields[2].parse().map_err(|_| "bad PPM height")?;
    let rgb = data.get(i..i + width * height * 3).ok_or("PPM picture is cut short")?.to_vec();
    Ok(Pixels { width, height, rgb })
}

/// Decodes any picture ImageMagick reads (first frame) to RGB on white.
pub fn decode(path: &Path, extra: &[String]) -> Result<Pixels, String> {
    let exe = tools::require(Tool::Magick)?;
    let mut src = path.as_os_str().to_os_string();
    src.push("[0]");
    let out = crate::procs::output(
        tools::command(&exe)
            .arg(src)
            .args(extra)
            .args(["-background", "white", "-alpha", "remove", "-alpha", "off", "-colorspace", "sRGB", "-depth", "8", "ppm:-"])
            .stdin(Stdio::null()),
        "ImageMagick",
        None,
    )?;
    if !out.status.success() {
        return Err(crate::convert::tool_error("ImageMagick", &String::from_utf8_lossy(&out.stderr)));
    }
    parse_ppm(&out.stdout)
}

/// How alike two pictures look, 0–100 (SSIMULACRA2; can dip below 0 for very
/// different pictures). Pictures of different sizes score 0.
pub fn score(original: &Pixels, candidate: &Pixels) -> f64 {
    if original.width != candidate.width || original.height != candidate.height {
        return 0.0;
    }
    if original.width < 8 || original.height < 8 {
        // Too small to measure: only identical counts.
        return if original.rgb == candidate.rgb { 100.0 } else { 0.0 };
    }
    let (Ok(a), Ok(b)) = (original.to_ssim(), candidate.to_ssim()) else { return 0.0 };
    ssimulacra2::compute_frame_ssimulacra2(a, b).unwrap_or(0.0)
}

/// Which lossy format a picture is being written as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lossy {
    Jpg,
    Webp,
    Avif,
}

impl Lossy {
    fn ext(self) -> &'static str {
        match self {
            Lossy::Jpg => "jpg",
            Lossy::Webp => "webp",
            Lossy::Avif => "avif",
        }
    }

    /// Lowest and highest quality searched.
    fn range(self) -> (u32, u32) {
        self.range_from(40)
    }

    /// The range with the lowest quality from a preset (AVIF's scale sits 10 lower).
    fn range_from(self, floor: u32) -> (u32, u32) {
        match self {
            Lossy::Jpg | Lossy::Webp => (floor.clamp(10, 90), 95),
            Lossy::Avif => (floor.saturating_sub(10).clamp(10, 85), 90),
        }
    }

    /// Encoder settings that give smaller files at the same look.
    pub fn args(self, quality: u32) -> Vec<String> {
        let mut a = s(&["-quality", &quality.to_string()]);
        match self {
            // 4:2:0 colour is invisible in photos above quality 90 but
            // noticeable on sharp coloured text below it.
            Lossy::Jpg if quality >= 90 => a.extend(s(&["-sampling-factor", "4:2:0"])),
            Lossy::Jpg => a.extend(s(&["-sampling-factor", "4:2:0", "-define", "jpeg:optimize-coding=true"])),
            Lossy::Webp => a.extend(s(&["-define", "webp:method=5", "-define", "webp:use-sharp-yuv=true"])),
            Lossy::Avif => a.extend(s(&["-define", "heic:speed=6"])),
        }
        a
    }
}

/// Encodes `src` (already prepared, e.g. a PNG crop) at `quality` and decodes it again.
fn trial(src: &Path, format: Lossy, quality: u32, dir: &Path) -> Result<(Pixels, u64), String> {
    let out = dir.join(format!("q{quality}.{}", format.ext()));
    let mut cmd = tools::command(&tools::require(Tool::Magick)?);
    cmd.arg(src).args(format.args(quality)).arg(&out);
    exec(Tool::Magick, cmd)?;
    let size = out.metadata().map(|m| m.len()).unwrap_or(0);
    let px = decode(&out, &[])?;
    let _ = std::fs::remove_file(&out);
    Ok((px, size))
}

/// The lowest quality whose result scores at least `target` against `reference`
/// (a lossless PNG), found by bisection. Falls back to the top of the range.
pub fn search_quality(reference: &Path, format: Lossy, target: f64, floor: u32) -> Result<u32, String> {
    let original = decode(reference, &[])?;
    if original.width < 8 || original.height < 8 {
        return Ok(format.range().1);
    }
    let tmp = TempDir::new("look")?;
    let (mut lo, mut hi) = format.range_from(floor);
    // `hi` is known good only once tested.
    let (top, _) = trial(reference, format, hi, &tmp.0)?;
    let top_score = score(&original, &top);
    if top_score < target {
        log::info!("look {format:?}: even quality {hi} scores {top_score:.1} < {target}; using {hi}");
        return Ok(hi);
    }
    while hi - lo > 3 {
        if crate::procs::cancelled_here() {
            return Err(crate::procs::CANCELLED.into());
        }
        let mid = (lo + hi) / 2;
        let (px, _) = trial(reference, format, mid, &tmp.0)?;
        let sc = score(&original, &px);
        log::debug!("look {format:?}: quality {mid} scores {sc:.1}");
        if sc >= target {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    log::info!("look {format:?}: quality {hi} reaches {target}");
    Ok(hi)
}

/// Writes a crop of the prepared picture (`prep`: flatten, resize…) as a lossless PNG
/// for the quality search.
pub fn search_crop(input: &Path, prep: &[String], dir: &Path) -> Result<PathBuf, String> {
    let out = dir.join("crop.png");
    let mut src = input.as_os_str().to_os_string();
    src.push("[0]");
    let mut cmd = tools::command(&tools::require(Tool::Magick)?);
    cmd.arg(src)
        .arg("-auto-orient")
        .args(prep)
        .args(["-gravity", "center", "-crop", &format!("{CROP}x{CROP}+0+0"), "+repage", "-define", "png:compression-level=1"])
        .arg(&out);
    exec(Tool::Magick, cmd)?;
    Ok(out)
}

/// Encodes `input` (first frame) to `output` as `format`: the smallest quality that
/// still looks like the original at `level`, or `fixed` when there's no level.
pub fn picture(input: &Path, prep: &[String], format: Lossy, level: Option<Level>, fixed: u32, output: &Path) -> Result<u32, String> {
    let quality = match level {
        Some(level) => {
            let tmp = TempDir::new("pic")?;
            let crop = search_crop(input, prep, &tmp.0)?;
            search_quality(&crop, format, level.picture_target(), level.floor)?
        }
        None => fixed,
    };
    let mut src = input.as_os_str().to_os_string();
    src.push("[0]");
    let mut cmd = tools::command(&tools::require(Tool::Magick)?);
    cmd.arg(src).arg("-auto-orient").args(prep).args(format.args(quality)).arg(output);
    exec(Tool::Magick, cmd)?;
    Ok(quality)
}

/// Removes the GPS location from a picture's EXIF (JPG, WebP, PNG, TIFF…);
/// the rest (camera, date, orientation) stays. Pictures without EXIF, or
/// that little_exif can't read, are left as they are.
pub fn strip_gps(path: &Path) {
    use little_exif::ifd::ExifTagGroup;
    let Ok(mut md) = little_exif::metadata::Metadata::new_from_path(path) else { return };
    let mut removed = 0;
    // GPS tags are numbered 0x00–0x1f in their own directory.
    for hex in 0u16..=0x1f {
        removed += md.remove_tag_by_hex_group(hex, ExifTagGroup::GPS);
    }
    if removed == 0 {
        return;
    }
    match md.write_to_file(path) {
        Ok(()) => log::info!("removed the GPS location from {}", path.display()),
        Err(e) => log::warn!("couldn't remove the GPS location from {}: {e}", path.display()),
    }
}

/// Makes a PNG smaller without changing any pixel. Failures keep the PNG as it is.
pub fn shrink_png(path: &Path, level: Option<Level>) {
    let preset = level.unwrap_or(Level::Balanced).png_preset();
    let Ok(data) = std::fs::read(path) else { return };
    let opts = oxipng::Options::from_preset(preset);
    match oxipng::optimize_from_memory(&data, &opts) {
        Ok(smaller) if smaller.len() < data.len() => {
            let tmp = path.with_extension("png.tmp");
            if std::fs::write(&tmp, &smaller).is_ok() && std::fs::rename(&tmp, path).is_err() {
                let _ = std::fs::remove_file(&tmp);
            }
        }
        Ok(_) => {}
        Err(e) => log::info!("oxipng left {} as it was: {e}", path.display()),
    }
}

// ---------------------------------------------------------------------------
// PDFs

/// Pages compared for PDF Compress: first, middle and last.
pub fn sample_pages(count: u32) -> Vec<u32> {
    let mut v = vec![1, count.div_ceil(2), count];
    v.retain(|p| *p >= 1);
    v.dedup();
    v
}

/// Number of pages. Poppler's pdfinfo isn't shipped, but pdftoppm names the
/// last page when asked for one far past it.
pub fn pdf_page_count(pdf: &Path) -> Option<u32> {
    let exe = tools::require(Tool::Pdftoppm).ok()?;
    let out = tools::command(&exe)
        .args(["-png", "-f", "1000000", "-l", "1000000"])
        .arg(pdf)
        .arg(std::env::temp_dir().join("convertino-count"))
        .stdin(Stdio::null())
        .output()
        .ok()?;
    parse_last_page(&String::from_utf8_lossy(&out.stderr))
}

fn parse_last_page(text: &str) -> Option<u32> {
    let rest = text.split("the last page (").nth(1)?;
    rest.split(')').next()?.trim().parse().ok()
}

/// One PDF page rendered at `dpi`.
pub fn render_page(pdf: &Path, page: u32, dpi: u32, dir: &Path, tag: &str) -> Result<Pixels, String> {
    let prefix = dir.join(format!("{tag}-{page}"));
    let mut cmd = tools::command(&tools::require(Tool::Pdftoppm)?);
    cmd.args(["-png", "-singlefile", "-r", &dpi.to_string(), "-f", &page.to_string(), "-l", &page.to_string()])
        .arg(pdf)
        .arg(&prefix);
    exec(Tool::Pdftoppm, cmd)?;
    let png = prefix.with_extension("png");
    let px = decode(&png, &[]);
    let _ = std::fs::remove_file(&png);
    px
}

/// The worst score over the sample pages of `candidate` against `original_pages`.
pub fn pdf_score(original_pages: &[(u32, Pixels)], candidate: &Path, dpi: u32, dir: &Path, tag: &str) -> f64 {
    let mut worst = 100.0f64;
    for (page, original) in original_pages {
        let sc = match render_page(candidate, *page, dpi, dir, tag) {
            Ok(px) => score(original, &px),
            Err(_) => 0.0,
        };
        worst = worst.min(sc);
    }
    worst
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("convertino-look-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn ppm_is_read() {
        let mut data = b"P6\n# made by a test\n2 1\n255\n".to_vec();
        data.extend([255, 0, 0, 0, 0, 255]);
        let p = parse_ppm(&data).unwrap();
        assert_eq!((p.width, p.height), (2, 1));
        assert_eq!(p.rgb, vec![255, 0, 0, 0, 0, 255]);
        assert!(parse_ppm(b"P3\n1 1\n255\n0 0 0").is_err());
    }

    #[test]
    fn identical_pictures_score_high_and_noise_low() {
        let (w, h) = (64usize, 64usize);
        let rgb: Vec<u8> = (0..w * h * 3).map(|i| ((i * 7) % 251) as u8).collect();
        let a = Pixels { width: w, height: h, rgb: rgb.clone() };
        assert!(score(&a, &a) > 99.0);
        let noisy = Pixels { width: w, height: h, rgb: rgb.iter().enumerate().map(|(i, v)| v.wrapping_add((i % 97) as u8)).collect() };
        assert!(score(&a, &noisy) < 60.0);
        let other_size = Pixels { width: 32, height: 128, rgb };
        assert_eq!(score(&a, &other_size), 0.0);
    }

    #[test]
    fn levels() {
        let mut q = crate::settings::Quality { image: "fixed".into(), ..Default::default() };
        assert_eq!(q.image_level(), None);
        q.image = "nonsense".into();
        assert_eq!(q.image_level(), Some(Level::Balanced));
        assert!(Level::Best.picture_target() > Level::Small.picture_target());
        assert_eq!(sample_pages(1), vec![1]);
        assert_eq!(sample_pages(2), vec![1, 2]);
        assert_eq!(sample_pages(9), vec![1, 5, 9]);
        assert_eq!(
            parse_last_page("Wrong page range given: the first page (1000000) can not be after the last page (3)."),
            Some(3)
        );
    }

    /// A photo-like picture: the search finds a quality below the fixed default,
    /// the result is smaller than the fixed one, and still scores at the target.
    #[test]
    fn search_finds_a_smaller_file_that_looks_the_same() {
        let Some(im) = tools::find(Tool::Magick) else { return };
        let d = tmpdir("search");
        let src = d.join("photo.png");
        assert!(tools::command(&im)
            .args(["-size", "1400x900", "plasma:", "-blur", "0x1"])
            .arg(&src)
            .status()
            .unwrap()
            .success());
        let fixed = d.join("fixed.jpg");
        picture(&src, &[], Lossy::Jpg, None, 90, &fixed).unwrap();
        let tuned = d.join("tuned.jpg");
        let q = picture(&src, &[], Lossy::Jpg, Some(Level::Balanced), 90, &tuned).unwrap();
        let (a, b) = (fixed.metadata().unwrap().len(), tuned.metadata().unwrap().len());
        assert!(q < 90 && b < a, "quality {q}: {b} bytes vs {a} at 90");
        let sc = score(&decode(&src, &[]).unwrap(), &decode(&tuned, &[]).unwrap());
        assert!(sc >= Level::Balanced.picture_target() - 4.0, "full picture scores {sc:.1}");
    }

    #[test]
    fn png_shrinks_losslessly() {
        let Some(im) = tools::find(Tool::Magick) else { return };
        let d = tmpdir("png");
        let png = d.join("flat.png");
        assert!(tools::command(&im)
            .args(["-size", "600x400", "xc:skyblue", "-fill", "black", "-draw", "rectangle 50,50 300,200", "-define", "png:compression-level=0"])
            .arg(&png)
            .status()
            .unwrap()
            .success());
        let before_px = decode(&png, &[]).unwrap();
        let before = png.metadata().unwrap().len();
        shrink_png(&png, Some(Level::Balanced));
        assert!(png.metadata().unwrap().len() < before);
        assert_eq!(decode(&png, &[]).unwrap().rgb, before_px.rgb);
    }

    /// "Remove location (GPS)": the GPS tags go, the camera details stay.
    #[test]
    fn gps_is_removed_and_the_rest_kept() {
        use little_exif::exif_tag::ExifTag;
        use little_exif::rational::uR64;
        let Some(im) = tools::find(Tool::Magick) else { return };
        let d = tmpdir("gps");
        let jpg = d.join("trip.jpg");
        assert!(tools::command(&im).args(["-size", "300x200", "plasma:"]).arg(&jpg).status().unwrap().success());
        let mut md = little_exif::metadata::Metadata::new();
        md.set_tag(ExifTag::Make("Convertino Cam".into()));
        md.set_tag(ExifTag::GPSLatitudeRef("N".into()));
        md.set_tag(ExifTag::GPSLatitude(vec![uR64 { nominator: 26, denominator: 1 }, uR64 { nominator: 27, denominator: 1 }, uR64 { nominator: 0, denominator: 1 }]));
        md.write_to_file(&jpg).unwrap();
        let has_gps = |p: &Path| {
            let md = little_exif::metadata::Metadata::new_from_path(p).unwrap();
            let gps = md.get_tag(&ExifTag::GPSLatitude(Vec::new())).next().is_some();
            let make = md.get_tag(&ExifTag::Make(String::new())).next().is_some();
            (gps, make)
        };
        assert_eq!(has_gps(&jpg), (true, true));
        strip_gps(&jpg);
        assert_eq!(has_gps(&jpg), (false, true));
        // A picture without EXIF is left alone.
        let plain = d.join("plain.png");
        assert!(tools::command(&im).args(["-size", "20x20", "xc:red"]).arg(&plain).status().unwrap().success());
        let before = std::fs::read(&plain).unwrap();
        strip_gps(&plain);
        assert_eq!(std::fs::read(&plain).unwrap(), before);
    }
}
