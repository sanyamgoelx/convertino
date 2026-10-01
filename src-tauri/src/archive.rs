//! Archives with 7-Zip: extract, or repack as ZIP, 7Z or TAR.GZ.
//!
//! Extracting follows the Finder and Explorer habit: an archive holding a
//! single item puts that item next to it; anything else goes into a folder
//! named after the archive.

use crate::convert::{move_file, tool_error, Reserved, TempDir};
use crate::procs;
use crate::tools::{self, Tool};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pack {
    Zip,
    SevenZ,
    TarGz,
}

/// "photos.tar.gz" -> "photos"; "notes.zip" -> "notes".
pub fn archive_stem(path: &Path) -> String {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "archive".into());
    let lower = name.to_lowercase();
    for ext in [".tar.gz", ".tar.bz2", ".tar.xz", ".tar.zst", ".tgz", ".tbz2", ".txz", ".zip", ".7z", ".rar", ".tar", ".gz", ".bz2", ".xz"] {
        if lower.ends_with(ext) && name.len() > ext.len() {
            return name[..name.len() - ext.len()].to_string();
        }
    }
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or(name)
}

/// Runs 7-Zip, reporting its percentage. 7-Zip redraws progress with
/// backspaces rather than new lines, so output is split on those too.
fn seven(args: &[&std::ffi::OsStr], cwd: Option<&Path>, progress: &mut dyn FnMut(f64)) -> Result<(), String> {
    let exe = tools::require(Tool::SevenZip)?;
    let mut cmd = tools::command(&exe);
    // -p with a dummy password: a protected archive fails at once instead of asking and hanging.
    cmd.args(["-y", "-bsp1", "-bso0", "-bse2", "-p_convertino_"]).args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut tracked = procs::spawn(&mut cmd, "7-Zip")?;
    let child = tracked.child();
    let mut stderr = child.stderr.take().expect("piped");
    let err_thread = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });
    let mut stdout = child.stdout.take().expect("piped");
    let mut buf = [0u8; 512];
    let mut token = String::new();
    loop {
        let n = stdout.read(&mut buf).unwrap_or(0);
        if n == 0 {
            break;
        }
        for &b in &buf[..n] {
            if matches!(b, b'\r' | b'\n' | 8) {
                if let Some(pct) = token.trim().split('%').next().and_then(|p| p.trim().parse::<f64>().ok()) {
                    if token.contains('%') {
                        progress((pct / 100.0).clamp(0.0, 1.0));
                    }
                }
                token.clear();
            } else {
                token.push(b as char);
            }
        }
    }
    let status = child.wait().map_err(|e| format!("7-Zip stopped unexpectedly: {e}"))?;
    let errors = err_thread.join().unwrap_or_default();
    if procs::cancelled_here() {
        return Err(procs::CANCELLED.into());
    }
    if status.success() {
        return Ok(());
    }
    let lower = errors.to_lowercase();
    if lower.contains("wrong password") || lower.contains("encrypted") {
        Err("This archive is password-protected. Convertino can't open protected archives yet.".into())
    } else if lower.contains("can not open the file as") || lower.contains("cannot open the file as") {
        Err("This file isn't an archive 7-Zip can open, or it's damaged.".into())
    } else {
        Err(tool_error("7-Zip", &errors))
    }
}

fn entries(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).map(|r| r.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    v.sort();
    v
}

/// Unpacks `archive` into `dest`, then any single .tar that came out of a .gz/.xz/.bz2.
fn unpack(archive: &Path, dest: &Path, progress: &mut dyn FnMut(f64)) -> Result<(), String> {
    let out = format!("-o{}", dest.display());
    seven(&["x".as_ref(), out.as_ref(), archive.as_os_str()], None, &mut |p| progress(p * 0.8))?;
    let inside = entries(dest);
    if inside.len() == 1 && inside[0].extension().map(|e| e.eq_ignore_ascii_case("tar")).unwrap_or(false) {
        let tar = inside[0].clone();
        seven(&["x".as_ref(), out.as_ref(), tar.as_os_str()], None, &mut |p| progress(0.8 + p * 0.2))?;
        let _ = std::fs::remove_file(&tar);
    }
    Ok(())
}

/// Extracts into `dir` (next to the archive, normally) and returns what was made.
pub fn extract(archive: &Path, dir: &Path, progress: &mut dyn FnMut(f64)) -> Result<PathBuf, String> {
    let dir = dir.to_path_buf();
    // Unpack beside the archive so the final move is a rename, not a copy.
    let scratch = dir.join(format!(".convertino-extract-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).map_err(|e| format!("Couldn't create a folder next to the archive: {e}"))?;
    struct Cleanup<'a>(&'a Path);
    impl Drop for Cleanup<'_> {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(self.0);
        }
    }
    let _cleanup = Cleanup(&scratch);

    unpack(archive, &scratch, progress)?;
    let inside = entries(&scratch);
    let result = match inside.as_slice() {
        [] => return Err("The archive is empty.".into()),
        [only] => {
            let name = only.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let (stem, ext) = if only.is_dir() {
                (name, String::new())
            } else {
                match name.rsplit_once('.') {
                    Some((s, e)) if !s.is_empty() => (s.to_string(), e.to_string()),
                    _ => (name, String::new()),
                }
            };
            let name = Reserved::new(&dir, &stem, &ext);
            let target = name.0.clone();
            std::fs::rename(only, &target).map_err(|e| format!("Couldn't move the extracted item: {e}"))?;
            target
        }
        _ => {
            let name = Reserved::new(&dir, &archive_stem(archive), "");
            let target = name.0.clone();
            std::fs::create_dir(&target).map_err(|e| format!("Couldn't create {}: {e}", target.display()))?;
            for item in inside {
                let to = target.join(item.file_name().unwrap_or_default());
                std::fs::rename(&item, &to).map_err(|e| format!("Couldn't move the extracted files: {e}"))?;
            }
            target
        }
    };
    progress(1.0);
    Ok(result)
}

/// Repacks `archive` into `output` in another format.
pub fn repack(archive: &Path, to: Pack, output: &Path, progress: &mut dyn FnMut(f64)) -> Result<(), String> {
    let tmp = TempDir::new("archive")?;
    let files = tmp.0.join("files");
    std::fs::create_dir_all(&files).map_err(|e| e.to_string())?;
    unpack(archive, &files, &mut |p| progress(p * 0.5))?;
    if entries(&files).is_empty() {
        return Err("The archive is empty.".into());
    }
    let packed = tmp.0.join("packed");
    let everything: &std::ffi::OsStr = "*".as_ref();
    match to {
        Pack::Zip | Pack::SevenZ => {
            let kind = if to == Pack::Zip { "-tzip" } else { "-t7z" };
            seven(&["a".as_ref(), kind.as_ref(), "-mx=7".as_ref(), packed.as_os_str(), everything], Some(&files), &mut |p| {
                progress(0.5 + p * 0.5)
            })?;
        }
        Pack::TarGz => {
            let tar = tmp.0.join("packed.tar");
            seven(&["a".as_ref(), "-ttar".as_ref(), tar.as_os_str(), everything], Some(&files), &mut |p| progress(0.5 + p * 0.2))?;
            seven(&["a".as_ref(), "-tgzip".as_ref(), "-mx=9".as_ref(), packed.as_os_str(), tar.as_os_str()], None, &mut |p| {
                progress(0.7 + p * 0.3)
            })?;
        }
    }
    // 7-Zip may add its own extension to the name it was given.
    let made = entries(&tmp.0)
        .into_iter()
        .find(|p| p.is_file() && p.file_name().map(|n| n.to_string_lossy().starts_with("packed") && !n.to_string_lossy().ends_with("packed.tar")).unwrap_or(false))
        .ok_or("7-Zip didn't write the new archive.")?;
    move_file(&made, output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stems_drop_double_extensions() {
        assert_eq!(archive_stem(Path::new("/x/photos.tar.gz")), "photos");
        assert_eq!(archive_stem(Path::new("/x/My.Notes.zip")), "My.Notes");
        assert_eq!(archive_stem(Path::new("/x/backup.TGZ")), "backup");
    }

    #[test]
    fn extract_and_repack_when_7zip_exists() {
        if tools::find(Tool::SevenZip).is_none() {
            return;
        }
        let d = std::env::temp_dir().join(format!("convertino-archive-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let src = d.join("src");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("a.txt"), "hello").unwrap();
        std::fs::write(src.join("sub/b.txt"), "world").unwrap();
        let zip = d.join("pack.zip");
        seven(&["a".as_ref(), "-tzip".as_ref(), zip.as_os_str(), "*".as_ref()], Some(&src), &mut |_| {}).unwrap();

        // Two items inside: a folder named after the archive.
        let out = extract(&zip, &d, &mut |_| {}).unwrap();
        assert_eq!(out.file_name().unwrap(), "pack");
        assert_eq!(std::fs::read_to_string(out.join("sub/b.txt")).unwrap(), "world");
        assert!(!d.join(format!(".convertino-extract-{}", std::process::id())).exists());

        for (pack, ext) in [(Pack::SevenZ, "7z"), (Pack::TarGz, "tar.gz"), (Pack::Zip, "zip")] {
            let output = crate::convert::unique_output(&d, "pack", ext);
            repack(&zip, pack, &output, &mut |_| {}).unwrap_or_else(|e| panic!("{ext}: {e}"));
            assert!(output.metadata().unwrap().len() > 50, "{ext}");
            if pack == Pack::TarGz {
                // A .tar.gz extracts in one go, not to a .tar.
                let back = extract(&output, &d, &mut |_| {}).unwrap();
                assert!(back.join("a.txt").exists(), "{}", back.display());
            }
        }

        // One file inside: it lands next to the archive.
        let single = d.join("one.7z");
        seven(&["a".as_ref(), single.as_os_str(), src.join("a.txt").as_os_str()], None, &mut |_| {}).unwrap();
        let out = extract(&single, &d, &mut |_| {}).unwrap();
        assert_eq!(out.file_name().unwrap(), "a.txt");

        // Not an archive: explained.
        let fake = d.join("fake.zip");
        std::fs::write(&fake, "not a zip").unwrap();
        assert!(extract(&fake, &d, &mut |_| {}).unwrap_err().contains("isn't an archive"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
