//! End-to-end tests of the `convertino` command: the real program, run the
//! way a person (or a script) runs it. Each test works in its own folder,
//! with its own settings folder (CONVERTINO_CONFIG_DIR), so a developer's
//! own Settings never leak in.
//!
//! The ignored `cli_every_conversion` test converts sample files to every
//! format through the command line; CI runs it after downloading the converters.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_convertino-cli");
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

struct Sandbox {
    dir: PathBuf,
    config: PathBuf,
}

impl Sandbox {
    fn new(name: &str) -> Sandbox {
        let dir = std::env::temp_dir().join(format!("convertino-e2e-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let config = dir.join("_config");
        std::fs::create_dir_all(&config).unwrap();
        Sandbox { dir, config }
    }

    /// Copies a fixture in, under `as_name`.
    fn file(&self, fixture: &str, as_name: &str) -> PathBuf {
        let to = self.dir.join(as_name);
        std::fs::copy(Path::new(FIXTURES).join(fixture), &to).unwrap();
        to
    }

    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new(BIN);
        c.args(args).current_dir(&self.dir).env("CONVERTINO_CONFIG_DIR", &self.config).env("NO_COLOR", "1");
        c
    }

    fn run(&self, args: &[&str]) -> Output {
        self.cmd(args).output().expect("convertino-cli runs")
    }

    fn json(&self, args: &[&str]) -> (i32, serde_json::Value) {
        let mut a = args.to_vec();
        a.push("--json");
        let out = self.run(&a);
        let text = String::from_utf8_lossy(&out.stdout);
        let v = serde_json::from_str(text.trim()).unwrap_or_else(|e| panic!("not JSON ({e}): {text}\nstderr: {}", String::from_utf8_lossy(&out.stderr)));
        (out.status.code().unwrap_or(-1), v)
    }

    fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(&self.dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| !n.starts_with('_'))
            .collect();
        v.sort();
        v
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// A second copy of a name, the way the OS names copies.
fn copy_name(stem: &str, ext: &str) -> String {
    if cfg!(target_os = "macos") { format!("{stem} 2.{ext}") } else { format!("{stem} (1).{ext}") }
}

fn png_dims(p: &Path) -> (u32, u32) {
    let b = std::fs::read(p).unwrap();
    assert_eq!(&b[..8], b"\x89PNG\r\n\x1a\n", "{} is a PNG", p.display());
    (u32::from_be_bytes(b[16..20].try_into().unwrap()), u32::from_be_bytes(b[20..24].try_into().unwrap()))
}

fn is_jpeg(p: &Path) -> bool {
    std::fs::read(p).map(|b| b.starts_with(&[0xFF, 0xD8, 0xFF])).unwrap_or(false)
}

#[test]
fn version_and_help() {
    let s = Sandbox::new("version");
    let out = s.run(&["--version"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(text(&out.stdout).trim(), format!("convertino {}", env!("CARGO_PKG_VERSION")));
    let out = s.run(&["--help"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(text(&out.stdout).contains("--to <format>"));
    // No arguments: help, not an error.
    assert_eq!(s.run(&[]).status.code(), Some(0));
}

#[test]
fn usage_mistakes_exit_2_with_a_hint() {
    let s = Sandbox::new("usage");
    s.file("gradient.png", "a.png");
    for (args, hint) in [
        (vec!["a.png"], "--to"),
        (vec!["a.png", "--to", "jpg", "--frobnicate"], "Unknown option"),
        (vec!["missing.png", "--to", "jpg"], "not found"),
        (vec!["a.png", "--size", "huge"], "isn't a size"),
        (vec!["a.png", "--to", "docx"], "can't become docx"),
    ] {
        let out = s.run(&args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        let err = text(&out.stderr);
        assert!(err.contains(hint), "{args:?}: {err}");
    }
    // A wrong target lists what the file can become.
    let err = text(&s.run(&["a.png", "--to", "docx"]).stderr);
    assert!(err.contains("jpg") && err.contains("webp"), "{err}");
    // Nothing was made.
    assert_eq!(s.names(), vec!["a.png"]);
}

#[test]
fn converts_next_to_the_original_and_never_overwrites() {
    let s = Sandbox::new("convert");
    let src = s.file("gradient.png", "a.png");
    let before = std::fs::read(&src).unwrap();

    let (code, v) = s.json(&["a.png", "--to", "jpg"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["schema"], 1);
    assert_eq!(v["ok"], true);
    let out = PathBuf::from(v["outputs"][0]["path"].as_str().unwrap());
    assert_eq!(out.canonicalize().unwrap(), s.dir.join("a.jpg").canonicalize().unwrap());
    assert!(is_jpeg(&out));
    assert!(v["outputs"][0]["bytes"].as_u64().unwrap() > 0);
    assert_eq!(v["outputs"][0]["sourceBytes"].as_u64().unwrap(), before.len() as u64);

    // Again: a new name, the first result untouched.
    let first = std::fs::read(&out).unwrap();
    let (code, v) = s.json(&["a.png", "--to", "jpg"]);
    assert_eq!(code, 0);
    let second = PathBuf::from(v["outputs"][0]["path"].as_str().unwrap());
    assert_eq!(second.file_name().unwrap().to_string_lossy(), copy_name("a", "jpg"));
    assert_eq!(std::fs::read(&out).unwrap(), first);
    // The original is never changed.
    assert_eq!(std::fs::read(&src).unwrap(), before);
}

#[test]
fn plain_output_is_one_line_per_file_and_a_summary() {
    let s = Sandbox::new("plain");
    s.file("gradient.png", "a.png");
    s.file("gradient.png", "b.png");
    let out = s.run(&["a.png", "b.png", "--to", "webp"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let stdout = text(&out.stdout);
    assert!(stdout.contains("a.png → a.webp"), "{stdout}");
    assert!(stdout.contains("b.png → b.webp"), "{stdout}");
    assert!(stdout.contains("✓ 2 converted"), "{stdout}");
    assert!(!stdout.contains('\x1b'), "no colours when NO_COLOR is set or output is piped");
    // --quiet prints nothing when it works.
    let out = s.run(&["a.png", "--to", "avif", "--quiet"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(text(&out.stdout).is_empty());
}

#[test]
fn wildcards_spaces_and_other_alphabets() {
    let s = Sandbox::new("names");
    s.file("gradient.png", "Café – मेनू.png");
    s.file("gradient.png", "two words.png");
    s.file("people.csv", "x.csv");
    let (code, v) = s.json(&["*.png", "--to", "jpg"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["outputs"].as_array().unwrap().len(), 2);
    let names = s.names();
    assert!(names.contains(&"Café – मेनू.jpg".to_string()), "{names:?}");
    assert!(names.contains(&"two words.jpg".to_string()), "{names:?}");
}

#[test]
fn out_folder_and_settings_folder() {
    let s = Sandbox::new("out");
    s.file("gradient.png", "a.png");
    let (code, v) = s.json(&["a.png", "--to", "png", "--out", "results"]);
    // PNG to PNG isn't a conversion.
    assert_eq!(code, 2, "{v}");
    let (code, v) = s.json(&["a.png", "--to", "webp", "--out", "results"]);
    assert_eq!(code, 0, "{v}");
    assert!(s.dir.join("results").join("a.webp").is_file());

    // "Save in one folder" from Settings is followed, and settings.json isn't rewritten.
    let chosen = s.dir.join("chosen");
    let settings = serde_json::json!({ "version": 2, "saveMode": "folder", "saveFolder": chosen });
    let file = s.config.join("settings.json");
    std::fs::write(&file, serde_json::to_string(&settings).unwrap()).unwrap();
    let before = std::fs::read(&file).unwrap();
    let (code, _) = s.json(&["a.png", "--to", "jpg"]);
    assert_eq!(code, 0);
    assert!(chosen.join("a.jpg").is_file());
    assert_eq!(std::fs::read(&file).unwrap(), before);
}

#[test]
fn command_line_never_creates_settings() {
    let s = Sandbox::new("nosettings");
    s.file("gradient.png", "a.png");
    assert_eq!(s.run(&["a.png", "--to", "jpg"]).status.code(), Some(0));
    // The app's first run must still find no settings.json and open Settings.
    assert!(!s.config.join("settings.json").exists());
}

#[test]
fn some_failures_exit_1_and_keep_the_rest() {
    let s = Sandbox::new("partial");
    s.file("gradient.png", "good.png");
    std::fs::write(s.dir.join("broken.png"), b"not a picture").unwrap();
    std::fs::write(s.dir.join("thing.xyz"), b"?").unwrap();
    let (code, v) = s.json(&["good.png", "broken.png", "thing.xyz", "--to", "jpg"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(v["ok"], false);
    assert_eq!(v["outputs"].as_array().unwrap().len(), 1);
    assert_eq!(v["failed"].as_array().unwrap().len(), 2);
    assert!(s.dir.join("good.jpg").is_file());
    assert!(!s.dir.join("broken.jpg").exists(), "no half-written file is left behind");
}

#[test]
fn compress_to_a_size() {
    let s = Sandbox::new("size");
    // A bigger picture, so there is something to compress.
    let (code, v) = s.json(&[&s.file("gradient.png", "g.png").to_string_lossy(), "--to", "tiff"]);
    assert_eq!(code, 0, "{v}");
    let (code, v) = s.json(&["g.tiff", "--size", "4KB"]);
    assert!(code == 0, "{v}");
    if let Some(out) = v["outputs"][0]["path"].as_str() {
        let bytes = std::fs::metadata(out).unwrap().len();
        assert!(bytes <= 4_000, "{bytes} bytes for a 4 KB target");
    } else {
        // Already small enough: a note, nothing made.
        assert!(!v["notes"].as_array().unwrap().is_empty(), "{v}");
    }
}

#[test]
fn data_documents_archives_and_pdf() {
    let s = Sandbox::new("kinds");
    s.file("people.csv", "people.csv");
    s.file("bundle.zip", "bundle.zip");
    s.file("page.pdf", "page.pdf");
    let (code, v) = s.json(&["people.csv", "--to", "json"]);
    assert_eq!(code, 0, "{v}");
    let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(s.dir.join("people.json")).unwrap()).unwrap();
    assert_eq!(json[1]["city"], "Pune");
    let (code, v) = s.json(&["bundle.zip", "--to", "extract"]);
    assert_eq!(code, 0, "{v}");
    let (code, v) = s.json(&["page.pdf", "--to", "png"]);
    assert_eq!(code, 0, "{v}");
    let made = PathBuf::from(v["outputs"][0]["path"].as_str().unwrap());
    let png = if made.is_dir() { std::fs::read_dir(&made).unwrap().flatten().next().unwrap().path() } else { made };
    let (w, h) = png_dims(&png);
    assert!(w > 10 && h > 10);
}

#[test]
fn formats_lists_what_a_file_can_become() {
    let s = Sandbox::new("formats");
    s.file("gradient.png", "a.png");
    let (code, v) = s.json(&["formats", "a.png"]);
    assert_eq!(code, 0);
    let names: Vec<&str> = v[0]["conversions"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"jpg") && names.contains(&"webp") && names.contains(&"compress"), "{names:?}");
    assert!(!names.contains(&"png"), "not its own format");
    let out = s.run(&["formats", "a.png"]);
    assert!(text(&out.stdout).contains("jpg"));
}

#[test]
fn tools_status_is_json() {
    let s = Sandbox::new("tools");
    let (code, v) = s.json(&["tools"]);
    assert_eq!(code, 0);
    let ids: Vec<&str> = v.as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"ffmpeg"), "{ids:?}");
    assert_eq!(s.run(&["tools", "install", "nonsense"]).status.code(), Some(2));
}

/// ffmpeg for making a test video: Convertino's own copy, else one on PATH.
fn ffmpeg() -> Option<PathBuf> {
    let exe = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    let tools = Path::new(env!("CARGO_MANIFEST_DIR")).join("tools").join("ffmpeg");
    for p in [tools.join("bin").join(exe), tools.join(exe)] {
        if p.is_file() {
            return Some(p);
        }
    }
    std::env::var_os("PATH").and_then(|path| std::env::split_paths(&path).map(|d| d.join(exe)).find(|p| p.is_file()))
}

/// Ctrl+C stops the job, its converters and leaves no partial files.
#[cfg(unix)]
#[test]
fn ctrl_c_stops_everything() {
    let Some(ff) = ffmpeg() else {
        eprintln!("no ffmpeg: skipped");
        return;
    };
    let s = Sandbox::new("ctrlc");
    let video = s.dir.join("long.mp4");
    let ok = Command::new(&ff)
        .args(["-v", "error", "-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=30", "-t", "40", "-c:v", "libx264", "-preset", "ultrafast"])
        .arg(&video)
        .status()
        .unwrap()
        .success();
    assert!(ok);
    let mut child = s.cmd(&["long.mp4", "--to", "webm"]).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    std::thread::sleep(std::time::Duration::from_secs(4));
    let pid = child.id();
    Command::new("kill").args(["-INT", &pid.to_string()]).status().unwrap();
    let started = std::time::Instant::now();
    let status = loop {
        if let Some(st) = child.try_wait().unwrap() {
            break st;
        }
        assert!(started.elapsed().as_secs() < 30, "didn't stop after Ctrl+C");
        std::thread::sleep(std::time::Duration::from_millis(100));
    };
    assert_eq!(status.code(), Some(130));
    std::thread::sleep(std::time::Duration::from_millis(500));
    // No converter is still working on our file.
    let left = Command::new("pgrep").args(["-f", &s.dir.to_string_lossy()]).output().unwrap();
    assert!(text(&left.stdout).trim().is_empty(), "left running: {}", text(&left.stdout));
    assert_eq!(s.names(), vec!["long.mp4"], "no half-made output");
}

/// Every sample in test-files (or $CONVERTINO_SAMPLES) to every conversion
/// it offers, through the command line. Needs the converters (fetch-tools).
#[test]
#[ignore]
fn cli_every_conversion() {
    let samples = std::env::var_os("CONVERTINO_SAMPLES").map(PathBuf::from).unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../test-files"));
    let s = Sandbox::new("every");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&samples).map(|r| r.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect()).unwrap_or_default();
    if files.is_empty() {
        for f in ["gradient.png", "people.csv", "note.md", "bundle.zip", "page.pdf"] {
            files.push(Path::new(FIXTURES).join(f));
        }
    }
    let mut failures = Vec::new();
    for f in &files {
        let name = f.file_name().unwrap().to_string_lossy().into_owned();
        let local = s.dir.join(&name);
        std::fs::copy(f, &local).unwrap();
        let (_, list) = s.json(&["formats", &name]);
        for c in list[0]["conversions"].as_array().cloned().unwrap_or_default() {
            if c["multi"] == true {
                continue;
            }
            let target = c["name"].as_str().unwrap().to_string();
            let (code, v) = s.json(&[&name, "--to", &target]);
            let made = v["outputs"].as_array().map(|a| a.len()).unwrap_or(0);
            let nothing_to_do = code == 0 && made == 0 && !v["notes"].as_array().map(|n| n.is_empty()).unwrap_or(true);
            println!("{} {name} --to {target}: exit {code}, {made} made", if code == 0 { "ok  " } else { "FAIL" });
            if code != 0 || (made == 0 && !nothing_to_do) {
                failures.push(format!("{name} --to {target}: {v}"));
            }
        }
    }
    assert!(failures.is_empty(), "{} failed:\n{}", failures.len(), failures.join("\n"));
}

/// RAW photos through the command line (skipped without samples: run
/// ci/fetch-raw-samples.sh, or set CONVERTINO_RAW_SAMPLES).
#[test]
fn raw_photos_convert_with_their_details() {
    let dir = std::env::var_os("CONVERTINO_RAW_SAMPLES").map(PathBuf::from).unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../test-files/raw"));
    let mut raws: Vec<PathBuf> = std::fs::read_dir(&dir).map(|r| r.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect()).unwrap_or_default();
    raws.sort();
    let Some(first) = raws.iter().find(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("cr3"))).or(raws.first()).cloned() else {
        eprintln!("no RAW samples: skipped");
        return;
    };
    let s = Sandbox::new("raw");
    let name = first.file_name().unwrap().to_string_lossy().into_owned();
    std::fs::copy(&first, s.dir.join(&name)).unwrap();
    for target in ["jpg", "camera-jpg", "webp", "png"] {
        let (code, v) = s.json(&[&name, "--to", target]);
        assert_eq!(code, 0, "{target}: {v}");
        let out = PathBuf::from(v["outputs"][0]["path"].as_str().unwrap());
        let bytes = std::fs::read(&out).unwrap();
        // The camera's details came along (EXIF holds the camera make in plain text).
        let has = |m: &[u8]| bytes.windows(m.len()).any(|w| w == m);
        // JPG: "Exif" block; WebP: "EXIF" chunk; PNG: ImageMagick-style "Raw profile type exif".
        assert!(has(b"Exif") || has(b"EXIF") || has(b"Raw profile type exif"), "{target}: no EXIF in {}", out.display());
        if target != "png" {
            assert!(bytes.len() < std::fs::metadata(s.dir.join(&name)).unwrap().len() as usize, "{target} should be smaller than the RAW");
        }
    }
    // Compress to a size: a JPG under the size, even though the RAW itself is "under" it.
    let raw_bytes = std::fs::metadata(s.dir.join(&name)).unwrap().len();
    let target = format!("{}", raw_bytes * 2);
    let (code, v) = s.json(&[&name, "--size", &target]);
    assert_eq!(code, 0, "{v}");
    let out = v["outputs"][0]["path"].as_str().expect("a RAW is always compressed into a JPG");
    assert!(out.ends_with(".jpg"), "{out}");
    assert!(is_jpeg(Path::new(out)));
}
