//! Finds the converter programs Convertino runs.
//!
//! Looked up, in order: next to the app, in Convertino's own tools folder
//! (where converters are downloaded on first use, see install.rs; in
//! development that's src-tauri/tools), in the usual install locations, then
//! on the PATH.

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Ffmpeg,
    Magick,
    Pdftoppm,
    Pdftotext,
    Pdfseparate,
    Pdfunite,
    Ghostscript,
    LibreOffice,
    Pandoc,
    SevenZip,
}

impl Tool {
    pub fn display_name(self) -> &'static str {
        match self {
            Tool::Ffmpeg => "FFmpeg",
            Tool::Magick => "ImageMagick",
            Tool::Pdftoppm | Tool::Pdftotext | Tool::Pdfseparate | Tool::Pdfunite => "Poppler",
            Tool::Ghostscript => "Ghostscript",
            Tool::LibreOffice => "LibreOffice",
            Tool::Pandoc => "Pandoc",
            Tool::SevenZip => "7-Zip",
        }
    }

    /// Paths inside tools/ (without .exe), then names to try on the PATH.
    fn layout(self) -> (&'static [&'static str], &'static [&'static str]) {
        match self {
            Tool::Ffmpeg => (&["ffmpeg/ffmpeg"], &["ffmpeg"]),
            // ImageMagick 6 on some systems only has `convert`.
            Tool::Magick => (&["imagemagick/magick"], &["magick", "convert"]),
            Tool::Pdftoppm => (&["poppler/bin/pdftoppm", "poppler/pdftoppm"], &["pdftoppm"]),
            Tool::Pdftotext => (&["poppler/bin/pdftotext", "poppler/pdftotext"], &["pdftotext"]),
            Tool::Pdfseparate => (&["poppler/bin/pdfseparate", "poppler/pdfseparate"], &["pdfseparate"]),
            Tool::Pdfunite => (&["poppler/bin/pdfunite", "poppler/pdfunite"], &["pdfunite"]),
            Tool::Ghostscript => (&["ghostscript/bin/gswin64c"], &["gswin64c", "gs"]),
            // soffice.com waits for the conversion to finish; soffice.exe may not.
            Tool::LibreOffice => (
                &[
                    "libreoffice/program/soffice.com",
                    "libreoffice/LibreOffice/program/soffice.com",
                    "libreoffice/program/soffice",
                ],
                &["soffice", "libreoffice"],
            ),
            Tool::Pandoc => (&["pandoc/pandoc"], &["pandoc"]),
            // 7zz is the name of the official macOS and Linux builds.
            Tool::SevenZip => (&["7zip/7z", "7zip/7zz"], &["7zz", "7z", "7za"]),
        }
    }

    /// Standard install locations outside tools/.
    fn installed_locations(self) -> Vec<PathBuf> {
        match self {
            Tool::LibreOffice => {
                let mut v = Vec::new();
                for var in ["ProgramFiles", "ProgramFiles(x86)"] {
                    if let Some(pf) = std::env::var_os(var) {
                        v.push(PathBuf::from(pf).join("LibreOffice/program/soffice.com"));
                    }
                }
                v.push(PathBuf::from("/Applications/LibreOffice.app/Contents/MacOS/soffice"));
                v
            }
            Tool::SevenZip => {
                let mut v = Vec::new();
                for var in ["ProgramFiles", "ProgramFiles(x86)"] {
                    if let Some(pf) = std::env::var_os(var) {
                        v.push(PathBuf::from(pf).join("7-Zip/7z.exe"));
                    }
                }
                v.push(PathBuf::from("/opt/homebrew/bin/7zz"));
                v.push(PathBuf::from("/usr/local/bin/7zz"));
                v
            }
            _ => Vec::new(),
        }
    }
}

fn exe(name: &str) -> String {
    if cfg!(windows) && !name.ends_with(".com") && !name.ends_with(".exe") {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn tool_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(me) = std::env::current_exe() {
        if let Some(dir) = me.parent() {
            dirs.push(dir.join("tools"));
            // macOS app bundle: Contents/MacOS/convertino -> Contents/Resources/tools
            dirs.push(dir.join("../Resources/tools"));
        }
    }
    dirs.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("tools"));
    if let Some(own) = crate::install::root() {
        if !dirs.contains(&own) {
            dirs.push(own);
        }
    }
    dirs
}

/// Full path of the tool, or None when it isn't installed anywhere.
pub fn find(tool: Tool) -> Option<PathBuf> {
    let (inside, on_path) = tool.layout();
    for dir in tool_dirs() {
        for rel in inside {
            let p = dir.join(exe(rel));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    if let Some(p) = tool.installed_locations().into_iter().find(|p| p.is_file()) {
        return Some(p);
    }
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        for name in on_path {
            let p = dir.join(exe(name));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// The tool's path. A missing converter is downloaded first (progress goes
/// to the job's card), or the error says why it can't be.
pub fn require(tool: Tool) -> Result<PathBuf, String> {
    if let Some(p) = find(tool) {
        return Ok(p);
    }
    crate::install::ensure(crate::install::Pack::of(tool))?;
    find(tool).ok_or_else(|| format!("{} was downloaded but couldn't be started.", tool.display_name()))
}

/// A Command for the tool that never flashes a console window on Windows.
pub fn command(path: &Path) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}
