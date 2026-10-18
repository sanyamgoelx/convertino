//! Fonts: what a font file is called, whether it's installed already,
//! installing and uninstalling it (Windows, Mac), and web fonts in and out
//! (WOFF, WOFF2).
//!
//! Installing writes each font straight into the fonts folder from wherever
//! its bytes came from (a zip, another archive, a loose file): nothing is
//! unpacked next to the archive. "Just me" needs no admin prompt: Windows'
//! per-user fonts folder (%LOCALAPPDATA%\Microsoft\Windows\Fonts, registered
//! under HKCU) or ~/Library/Fonts on a Mac. "Everyone" asks once for admin
//! (C:\Windows\Fonts + HKLM, or /Library/Fonts).

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Ttf,
    Otf,
    Ttc,
    Woff,
    Woff2,
}

impl Kind {
    pub fn is_web(self) -> bool {
        matches!(self, Kind::Woff | Kind::Woff2)
    }
}

/// Font extensions Convertino reads.
pub const EXTS: [&str; 5] = ["ttf", "otf", "ttc", "woff", "woff2"];

pub fn is_font_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    EXTS.iter().any(|e| lower.ends_with(&format!(".{e}")))
}

/// The kind of font in these bytes, from its first four bytes.
pub fn kind_of(bytes: &[u8]) -> Option<Kind> {
    match bytes.get(..4)? {
        [0, 1, 0, 0] | b"true" => Some(Kind::Ttf),
        b"OTTO" => Some(Kind::Otf),
        b"ttcf" => Some(Kind::Ttc),
        b"wOFF" => Some(Kind::Woff),
        b"wOF2" => Some(Kind::Woff2),
        _ => None,
    }
}

/// What a font is called.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Meta {
    /// "Satoshi Bold" (each face of a collection, joined with " & ").
    pub full: String,
    pub family: String,
    pub style: String,
    /// PostScript (CFF) outlines, as in most .otf files; else TrueType.
    pub cff: bool,
    pub kind: Kind,
    /// Every face's full name (one, except in a .ttc).
    pub faces: Vec<String>,
}

/// The desktop font inside a web font (WOFF/WOFF2), or the bytes as they are.
pub fn desktop_bytes(bytes: &[u8]) -> Result<Vec<u8>, String> {
    match kind_of(bytes) {
        Some(Kind::Woff) => wuff::decompress_woff1(bytes).map_err(|e| format!("This WOFF font is damaged: {e:?}")),
        Some(Kind::Woff2) => wuff::decompress_woff2(bytes).map_err(|e| format!("This WOFF2 font is damaged: {e:?}")),
        Some(_) => Ok(bytes.to_vec()),
        None => Err("This isn't a font file.".into()),
    }
}

fn name_of(face: &ttf_parser::Face, ids: &[u16]) -> Option<String> {
    for id in ids {
        // English first (Windows 0x409 or Mac 0), then any readable one.
        let mut any = None;
        for n in face.names() {
            if n.name_id != *id {
                continue;
            }
            let Some(s) = n.to_string().filter(|s| !s.trim().is_empty()) else { continue };
            let english = n.language_id == 0x409 || (n.platform_id == ttf_parser::PlatformId::Macintosh && n.language_id == 0);
            if english {
                return Some(s.trim().to_string());
            }
            any.get_or_insert(s.trim().to_string());
        }
        if any.is_some() {
            return any;
        }
    }
    None
}

/// Names and outline type of a font (web fonts are read through their desktop font).
pub fn meta(bytes: &[u8]) -> Option<Meta> {
    let kind = kind_of(bytes)?;
    let data = if kind.is_web() { desktop_bytes(bytes).ok()? } else { bytes.to_vec() };
    let count = ttf_parser::fonts_in_collection(&data).unwrap_or(1).max(1);
    let mut faces = Vec::new();
    let (mut family, mut style, mut cff) = (String::new(), String::new(), false);
    for i in 0..count.min(64) {
        let Ok(face) = ttf_parser::Face::parse(&data, i) else { continue };
        let fam = name_of(&face, &[16, 1]).unwrap_or_default();
        let sty = name_of(&face, &[17, 2]).unwrap_or_else(|| "Regular".into());
        let full = name_of(&face, &[4]).unwrap_or_else(|| format!("{fam} {sty}").trim().to_string());
        if faces.is_empty() {
            family = fam;
            style = sty;
            cff = face.tables().cff.is_some() || face.tables().cff2.is_some();
        }
        if !full.is_empty() {
            faces.push(full);
        }
    }
    if faces.is_empty() {
        return None;
    }
    Some(Meta { full: faces.join(" & "), family, style, cff, kind, faces })
}

// ---------- where fonts are installed ----------

/// The fonts folder for "Just me". CONVERTINO_FONTS_DIR overrides it (tests).
pub fn user_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("CONVERTINO_FONTS_DIR") {
        return PathBuf::from(d);
    }
    if cfg!(windows) {
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        local.join("Microsoft").join("Windows").join("Fonts")
    } else if cfg!(target_os = "macos") {
        home().join("Library").join("Fonts")
    } else {
        std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| home().join(".local/share")).join("fonts")
    }
}

pub(crate) fn home() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
}

/// Tests that point CONVERTINO_FONTS_DIR somewhere take this first.
#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The fonts folder for "Everyone on this PC".
pub fn system_dir() -> PathBuf {
    if std::env::var_os("CONVERTINO_FONTS_DIR").is_some() {
        return user_dir().join("_all-users");
    }
    if cfg!(windows) {
        std::env::var_os("SystemRoot").or_else(|| std::env::var_os("windir")).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows")).join("Fonts")
    } else if cfg!(target_os = "macos") {
        PathBuf::from("/Library/Fonts")
    } else {
        PathBuf::from("/usr/local/share/fonts")
    }
}

/// Lower-case full names of the fonts installed now, for "already installed".
/// Read fresh each time (another app may have installed some meanwhile).
pub fn installed_names() -> HashSet<String> {
    let mut names = HashSet::new();
    #[cfg(windows)]
    if std::env::var_os("CONVERTINO_FONTS_DIR").is_none() {
        for root in ["HKCU", "HKLM"] {
            for (name, _) in win::reg_values(&format!(r"{root}\{}", win::FONTS_KEY)) {
                for face in reg_name_faces(&name) {
                    names.insert(face.to_lowercase());
                }
            }
        }
        return names;
    }
    let mut dirs_to_read = vec![user_dir(), system_dir()];
    if std::env::var_os("CONVERTINO_FONTS_DIR").is_none() && cfg!(target_os = "macos") {
        dirs_to_read.extend(mac_system_dirs());
    }
    for d in dirs_to_read {
        for full in names_in(&d) {
            names.insert(full);
        }
    }
    names
}

#[cfg(target_os = "macos")]
fn mac_system_dirs() -> Vec<PathBuf> {
    vec![PathBuf::from("/System/Library/Fonts"), PathBuf::from("/System/Library/Fonts/Supplemental")]
}
#[cfg(not(target_os = "macos"))]
fn mac_system_dirs() -> Vec<PathBuf> {
    Vec::new()
}

/// Full names of the fonts in a folder. The Mac's system folders never change
/// while Convertino runs, so they're read once.
fn names_in(dir: &Path) -> Vec<String> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<PathBuf, (std::time::SystemTime, Vec<String>)>>> = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let stamp = std::fs::metadata(dir).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
    if let Some((t, v)) = cache.lock().ok().and_then(|c| c.get(dir).cloned()) {
        if t == stamp {
            return v;
        }
    }
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if !p.is_file() || !is_font_name(&p.to_string_lossy()) {
                continue;
            }
            if let Some(m) = std::fs::read(&p).ok().and_then(|b| meta(&b)) {
                out.extend(m.faces.iter().map(|f| f.to_lowercase()));
            }
        }
    }
    if let Ok(mut c) = cache.lock() {
        c.insert(dir.to_path_buf(), (stamp, out.clone()));
    }
    out
}

/// "Arial Bold & Arial Bold Italic (TrueType)" -> ["Arial Bold", "Arial Bold Italic"].
#[cfg_attr(not(windows), allow(dead_code))]
pub fn reg_name_faces(name: &str) -> Vec<String> {
    let base = match name.rfind(" (") {
        Some(i) if name.ends_with(')') => &name[..i],
        _ => name,
    };
    base.split(" & ").map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

/// The registry value name Windows uses for a font: "Satoshi Bold (OpenType)".
pub fn reg_name(m: &Meta) -> String {
    format!("{} ({})", m.full, if m.cff { "OpenType" } else { "TrueType" })
}

/// A font this job installed (what Undo takes out again).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Installed {
    pub path: PathBuf,
    pub reg: String,
    pub everyone: bool,
}

/// A font to install: the file name to give it, its bytes and names.
pub struct ToInstall {
    pub file_name: String,
    pub bytes: Vec<u8>,
    pub meta: Meta,
}

/// A free name for `file_name` in `dir`; the same name when that file is
/// already there with the same bytes (installing it again).
fn place_in(dir: &Path, file_name: &str, bytes: &[u8]) -> PathBuf {
    let first = dir.join(file_name);
    if !first.exists() || std::fs::read(&first).map(|b| b == bytes).unwrap_or(false) {
        return first;
    }
    let (stem, ext) = match file_name.rsplit_once('.') {
        Some((s, e)) => (s.to_string(), e.to_string()),
        None => (file_name.to_string(), String::new()),
    };
    crate::convert::unique_output(dir, &stem, &ext)
}

/// Installs the fonts; returns what was installed (in the same order).
pub fn install(fonts: &[ToInstall], everyone: bool) -> Result<Vec<Installed>, String> {
    if fonts.is_empty() {
        return Ok(Vec::new());
    }
    if everyone {
        return install_everyone(fonts);
    }
    let dir = user_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("Couldn't open the fonts folder {}: {e}", dir.display()))?;
    let mut done = Vec::new();
    for f in fonts {
        let path = place_in(&dir, &f.file_name, &f.bytes);
        if let Err(e) = std::fs::write(&path, &f.bytes) {
            // Take back what this call already put in, so nothing is half done.
            let _ = uninstall(&done);
            return Err(format!("Couldn't write {} into the fonts folder: {e}", f.file_name));
        }
        let reg = reg_name(&f.meta);
        #[cfg(windows)]
        if std::env::var_os("CONVERTINO_FONTS_DIR").is_none() {
            if let Err(e) = win::register(&reg, &path) {
                let _ = std::fs::remove_file(&path);
                let _ = uninstall(&done);
                return Err(e);
            }
        }
        done.push(Installed { path, reg, everyone: false });
    }
    announce();
    Ok(done)
}

/// Takes the fonts out again (Undo). Returns how many were removed.
pub fn uninstall(fonts: &[Installed]) -> Result<usize, String> {
    let (all, mine): (Vec<&Installed>, Vec<&Installed>) = fonts.iter().partition(|f| f.everyone);
    let mut n = 0;
    for f in mine {
        #[cfg(windows)]
        if std::env::var_os("CONVERTINO_FONTS_DIR").is_none() {
            win::unregister(&f.reg, &f.path);
        }
        // Windows may hold a font open for a moment after RemoveFontResource.
        let mut gone = false;
        for _ in 0..10 {
            if std::fs::remove_file(&f.path).is_ok() || !f.path.exists() {
                gone = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }
        if gone {
            n += 1;
        } else {
            log::warn!("couldn't remove {} (in use?)", f.path.display());
        }
    }
    if !all.is_empty() {
        n += uninstall_everyone(&all.into_iter().cloned().collect::<Vec<_>>())?;
    }
    announce();
    Ok(n)
}

/// Tells open apps the font list changed (Windows; a Mac notices by itself).
fn announce() {
    #[cfg(windows)]
    win::broadcast();
}

// ---------- "Everyone on this PC": one admin prompt ----------

#[cfg(windows)]
fn install_everyone(fonts: &[ToInstall]) -> Result<Vec<Installed>, String> {
    let tmp = crate::convert::TempDir::new("fonts")?;
    let dir = system_dir();
    let mut lines = vec!["$ErrorActionPreference = 'Stop'".to_string(), format!("$key = 'HKLM:\\{}'", win::FONTS_KEY)];
    let mut done = Vec::new();
    for (i, f) in fonts.iter().enumerate() {
        let staged = tmp.0.join(format!("{i}.{}", Path::new(&f.file_name).extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| "ttf".into())));
        std::fs::write(&staged, &f.bytes).map_err(|e| format!("Couldn't prepare {}: {e}", f.file_name))?;
        let dest = place_in(&dir, &f.file_name, &f.bytes);
        let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let reg = reg_name(&f.meta);
        lines.push(format!("Copy-Item -LiteralPath {} -Destination {} -Force", ps(&staged.to_string_lossy()), ps(&dest.to_string_lossy())));
        lines.push(format!("New-ItemProperty -Path $key -Name {} -Value {} -PropertyType String -Force | Out-Null", ps(&reg), ps(&name)));
        done.push(Installed { path: dest, reg, everyone: true });
    }
    win::run_elevated(&tmp.0, &lines)?;
    for f in &done {
        win::load(&f.path);
    }
    announce();
    Ok(done)
}

#[cfg(windows)]
fn uninstall_everyone(fonts: &[Installed]) -> Result<usize, String> {
    let tmp = crate::convert::TempDir::new("fonts")?;
    let mut lines = vec!["$ErrorActionPreference = 'Continue'".to_string(), format!("$key = 'HKLM:\\{}'", win::FONTS_KEY)];
    for f in fonts {
        win::unload(&f.path);
        lines.push(format!("Remove-ItemProperty -Path $key -Name {} -ErrorAction SilentlyContinue", ps(&f.reg)));
        lines.push(format!("Remove-Item -LiteralPath {} -Force -ErrorAction SilentlyContinue", ps(&f.path.to_string_lossy())));
    }
    win::run_elevated(&tmp.0, &lines)?;
    Ok(fonts.iter().filter(|f| !f.path.exists()).count())
}

#[cfg(windows)]
fn ps(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

#[cfg(target_os = "macos")]
fn install_everyone(fonts: &[ToInstall]) -> Result<Vec<Installed>, String> {
    let tmp = crate::convert::TempDir::new("fonts")?;
    let dir = system_dir();
    let mut cmds = Vec::new();
    let mut done = Vec::new();
    for (i, f) in fonts.iter().enumerate() {
        let staged = tmp.0.join(format!("{i}"));
        std::fs::write(&staged, &f.bytes).map_err(|e| format!("Couldn't prepare {}: {e}", f.file_name))?;
        let dest = place_in(&dir, &f.file_name, &f.bytes);
        cmds.push(format!("cp {} {}", sh(&staged.to_string_lossy()), sh(&dest.to_string_lossy())));
        done.push(Installed { path: dest, reg: reg_name(&f.meta), everyone: true });
    }
    crate::smart::mac_admin(&cmds.join(" && "))?;
    Ok(done)
}

#[cfg(target_os = "macos")]
fn uninstall_everyone(fonts: &[Installed]) -> Result<usize, String> {
    let cmds: Vec<String> = fonts.iter().map(|f| format!("rm -f {}", sh(&f.path.to_string_lossy()))).collect();
    crate::smart::mac_admin(&cmds.join(" ; "))?;
    Ok(fonts.iter().filter(|f| !f.path.exists()).count())
}

#[cfg(target_os = "macos")]
fn sh(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Elsewhere (Linux, tests): a plain folder.
#[cfg(not(any(windows, target_os = "macos")))]
fn install_everyone(fonts: &[ToInstall]) -> Result<Vec<Installed>, String> {
    let dir = system_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut done = Vec::new();
    for f in fonts {
        let path = place_in(&dir, &f.file_name, &f.bytes);
        std::fs::write(&path, &f.bytes).map_err(|e| e.to_string())?;
        done.push(Installed { path, reg: reg_name(&f.meta), everyone: true });
    }
    Ok(done)
}

#[cfg(not(any(windows, target_os = "macos")))]
fn uninstall_everyone(fonts: &[Installed]) -> Result<usize, String> {
    Ok(fonts.iter().filter(|f| std::fs::remove_file(&f.path).is_ok()).count())
}

#[cfg(windows)]
pub(crate) mod win {
    use ::windows::core::HSTRING;
    use ::windows::Win32::Foundation::{LPARAM, WPARAM};
    use ::windows::Win32::Graphics::Gdi::{AddFontResourceW, RemoveFontResourceW};
    use ::windows::Win32::System::Registry::{RegDeleteKeyValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ};
    use ::windows::Win32::UI::WindowsAndMessaging::{SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_FONTCHANGE};
    use std::os::windows::process::CommandExt;
    use std::path::Path;

    pub const FONTS_KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Fonts";
    const NO_WINDOW: u32 = 0x0800_0000;

    /// (name, data) of every value under a key, read with reg.exe.
    pub fn reg_values(key: &str) -> Vec<(String, String)> {
        let Ok(out) = std::process::Command::new("reg.exe").args(["query", key]).creation_flags(NO_WINDOW).output() else { return Vec::new() };
        let text = String::from_utf8_lossy(&out.stdout);
        text.lines()
            .filter_map(|l| {
                // "    Satoshi Bold (OpenType)    REG_SZ    C:\...\Satoshi-Bold.otf"
                let l = l.strip_prefix("    ")?;
                let (name, rest) = l.split_once("    REG_")?;
                let data = rest.split_once("    ").map(|(_, d)| d.to_string()).unwrap_or_default();
                Some((name.to_string(), data))
            })
            .collect()
    }

    pub fn register(name: &str, path: &Path) -> Result<(), String> {
        let wide: Vec<u16> = path.to_string_lossy().encode_utf16().chain([0]).collect();
        unsafe {
            RegSetKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(FONTS_KEY), &HSTRING::from(name), REG_SZ.0, Some(wide.as_ptr() as *const _), (wide.len() * 2) as u32)
                .ok()
                .map_err(|e| format!("Couldn't register the font with Windows: {}", e.message()))?;
        }
        load(path);
        Ok(())
    }

    pub fn unregister(name: &str, path: &Path) {
        unload(path);
        unsafe {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(FONTS_KEY), &HSTRING::from(name));
        }
    }

    /// Makes the font usable in this session at once (no sign-out).
    pub fn load(path: &Path) {
        unsafe {
            let n = AddFontResourceW(&HSTRING::from(path.as_os_str()));
            if n == 0 {
                log::warn!("AddFontResource found no fonts in {}", path.display());
            }
        }
    }

    pub fn unload(path: &Path) {
        unsafe {
            // A font loaded more than once needs as many removals.
            for _ in 0..4 {
                if !RemoveFontResourceW(&HSTRING::from(path.as_os_str())).as_bool() {
                    break;
                }
            }
        }
    }

    pub fn broadcast() {
        unsafe {
            let _ = SendMessageTimeoutW(HWND_BROADCAST, WM_FONTCHANGE, WPARAM(0), LPARAM(0), SMTO_ABORTIFHUNG, 1000, None);
        }
    }

    /// Runs PowerShell lines as administrator (one UAC prompt) and waits.
    /// A declined prompt is an error that says so.
    pub fn run_elevated(dir: &Path, lines: &[String]) -> Result<(), String> {
        let script = dir.join("run.ps1");
        let ok = dir.join("ok.txt");
        let mut body = lines.join("\r\n");
        body.push_str(&format!("\r\nSet-Content -LiteralPath '{}' -Value ok\r\n", ok.to_string_lossy().replace('\'', "''")));
        // PowerShell 5.1 reads a script as ANSI unless it starts with a BOM.
        std::fs::write(&script, [b"\xEF\xBB\xBF".as_slice(), body.as_bytes()].concat()).map_err(|e| e.to_string())?;
        let launcher = format!(
            "try {{ Start-Process -FilePath powershell.exe -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File','\"{}\"' }} catch {{ exit 5 }}",
            script.to_string_lossy().replace('\'', "''")
        );
        let status = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &launcher])
            .creation_flags(NO_WINDOW)
            .status()
            .map_err(|e| format!("Couldn't start PowerShell: {e}"))?;
        if !ok.exists() {
            return Err(if status.code() == Some(5) {
                "The admin prompt was declined, so nothing was changed.".into()
            } else {
                "Windows didn't finish the change as administrator, so nothing was changed.".into()
            });
        }
        Ok(())
    }
}

// ---------- web fonts out: WOFF and WOFF2 ----------

struct Table {
    tag: [u8; 4],
    checksum: u32,
    data: Vec<u8>,
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|x| u32::from_be_bytes([x[0], x[1], x[2], x[3]]))
}
fn be16(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at + 2).map(|x| u16::from_be_bytes([x[0], x[1]]))
}

/// The tables of a single desktop font (TTF/OTF; not a collection).
fn tables(sfnt: &[u8]) -> Result<(u32, Vec<Table>), String> {
    let flavor = be32(sfnt, 0).ok_or("This font is empty.")?;
    if kind_of(sfnt) == Some(Kind::Ttc) {
        return Err("Font collections (.ttc) can't become web fonts; each font in it would need its own file.".into());
    }
    if !matches!(kind_of(sfnt), Some(Kind::Ttf | Kind::Otf)) {
        return Err("This isn't a TTF or OTF font.".into());
    }
    let n = be16(sfnt, 4).ok_or("This font is damaged.")? as usize;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let rec = 12 + i * 16;
        let tag: [u8; 4] = sfnt.get(rec..rec + 4).ok_or("This font is damaged.")?.try_into().map_err(|_| "This font is damaged.")?;
        let checksum = be32(sfnt, rec + 4).ok_or("This font is damaged.")?;
        let offset = be32(sfnt, rec + 8).ok_or("This font is damaged.")? as usize;
        let length = be32(sfnt, rec + 12).ok_or("This font is damaged.")? as usize;
        let data = sfnt.get(offset..offset + length).ok_or("This font is damaged (a table runs past the end).")?.to_vec();
        out.push(Table { tag, checksum, data });
    }
    out.sort_by(|a, b| a.tag.cmp(&b.tag));
    Ok((flavor, out))
}

fn pad4(n: usize) -> usize {
    (n + 3) & !3
}

/// The sfnt size these tables make (WOFF headers carry it).
fn sfnt_size(tables: &[Table]) -> u32 {
    (12 + 16 * tables.len() + tables.iter().map(|t| pad4(t.data.len())).sum::<usize>()) as u32
}

/// TTF/OTF -> WOFF (each table zlib-compressed, or stored when that's smaller).
pub fn to_woff(sfnt: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::Write;
    let (flavor, tables) = tables(sfnt)?;
    let mut blocks = Vec::new();
    for t in &tables {
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        z.write_all(&t.data).map_err(|e| e.to_string())?;
        let packed = z.finish().map_err(|e| e.to_string())?;
        blocks.push(if packed.len() < t.data.len() { packed } else { t.data.clone() });
    }
    let header = 44;
    let dir = 20 * tables.len();
    let mut offset = header + dir;
    let mut out = vec![0u8; header + dir];
    let mut entries = Vec::new();
    for (t, b) in tables.iter().zip(&blocks) {
        entries.push((t.tag, offset as u32, b.len() as u32, t.data.len() as u32, t.checksum));
        out.extend_from_slice(b);
        out.resize(pad4(out.len()), 0);
        offset = out.len();
    }
    let total = out.len() as u32;
    let mut h = Vec::with_capacity(header);
    h.extend_from_slice(b"wOFF");
    h.extend_from_slice(&flavor.to_be_bytes());
    h.extend_from_slice(&total.to_be_bytes());
    h.extend_from_slice(&(tables.len() as u16).to_be_bytes());
    h.extend_from_slice(&0u16.to_be_bytes());
    h.extend_from_slice(&sfnt_size(&tables).to_be_bytes());
    h.extend_from_slice(&1u16.to_be_bytes());
    h.extend_from_slice(&0u16.to_be_bytes());
    h.extend_from_slice(&[0u8; 20]);
    out[..header].copy_from_slice(&h);
    for (i, (tag, off, comp, orig, sum)) in entries.into_iter().enumerate() {
        let at = header + i * 20;
        out[at..at + 4].copy_from_slice(&tag);
        out[at + 4..at + 8].copy_from_slice(&off.to_be_bytes());
        out[at + 8..at + 12].copy_from_slice(&comp.to_be_bytes());
        out[at + 12..at + 16].copy_from_slice(&orig.to_be_bytes());
        out[at + 16..at + 20].copy_from_slice(&sum.to_be_bytes());
    }
    Ok(out)
}

/// WOFF2's table tags with a one-byte code.
const WOFF2_TAGS: [&[u8; 4]; 63] = [
    b"cmap", b"head", b"hhea", b"hmtx", b"maxp", b"name", b"OS/2", b"post", b"cvt ", b"fpgm", b"glyf", b"loca", b"prep", b"CFF ", b"VORG", b"EBDT",
    b"EBLC", b"gasp", b"hdmx", b"kern", b"LTSH", b"PCLT", b"VDMX", b"vhea", b"vmtx", b"BASE", b"GDEF", b"GPOS", b"GSUB", b"EBSC", b"JSTF", b"MATH",
    b"CBDT", b"CBLC", b"COLR", b"CPAL", b"SVG ", b"sbix", b"acnt", b"avar", b"bdat", b"bloc", b"bsln", b"cvar", b"fdsc", b"feat", b"fmtx", b"fvar",
    b"gvar", b"hsty", b"just", b"lcar", b"mort", b"morx", b"opbd", b"prop", b"trak", b"Zapf", b"Silf", b"Glat", b"Gloc", b"Feat", b"Sill",
];

fn base128(mut n: u32, out: &mut Vec<u8>) {
    let mut bytes = vec![(n & 0x7f) as u8];
    n >>= 7;
    while n > 0 {
        bytes.push(((n & 0x7f) as u8) | 0x80);
        n >>= 7;
    }
    bytes.reverse();
    out.extend_from_slice(&bytes);
}

/// TTF/OTF -> WOFF2. Tables are kept as they are (the "null transform"), all
/// compressed together with Brotli: every browser reads it, and fonts come
/// out about 30% smaller than the TTF (a little bigger than tools that also
/// rewrite the glyph table).
pub fn to_woff2(sfnt: &[u8]) -> Result<Vec<u8>, String> {
    let (flavor, tables) = tables(sfnt)?;
    let mut dir = Vec::new();
    let mut stream = Vec::new();
    for t in &tables {
        let code = WOFF2_TAGS.iter().position(|x| **x == t.tag);
        // glyf and loca: transform 3 = kept as they are; other tables: 0 = kept as they are.
        let version: u8 = if &t.tag == b"glyf" || &t.tag == b"loca" { 3 } else { 0 };
        match code {
            Some(c) => dir.push((c as u8) | (version << 6)),
            None => {
                dir.push(63 | (version << 6));
                dir.extend_from_slice(&t.tag);
            }
        }
        base128(t.data.len() as u32, &mut dir);
        stream.extend_from_slice(&t.data);
    }
    let mut packed = Vec::new();
    let params = brotli::enc::BrotliEncoderParams { quality: 11, lgwin: 22, mode: brotli::enc::backward_references::BrotliEncoderMode::BROTLI_MODE_FONT, ..Default::default() };
    brotli::BrotliCompress(&mut stream.as_slice(), &mut packed, &params).map_err(|e| format!("Couldn't compress the font: {e}"))?;
    let header = 48;
    let body = header + dir.len() + packed.len();
    let total = pad4(body);
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"wOF2");
    out.extend_from_slice(&flavor.to_be_bytes());
    out.extend_from_slice(&(total as u32).to_be_bytes());
    out.extend_from_slice(&(tables.len() as u16).to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&sfnt_size(&tables).to_be_bytes());
    out.extend_from_slice(&(packed.len() as u32).to_be_bytes());
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&[0u8; 20]);
    out.extend_from_slice(&dir);
    out.extend_from_slice(&packed);
    out.resize(total, 0);
    Ok(out)
}

/// The font file conversions on the Font wheel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum To {
    Woff2,
    Woff,
    /// The TTF or OTF inside a web font.
    Desktop,
}

/// The extension a conversion of `input` ends in ("otf" or "ttf" for Desktop).
pub fn out_ext(input: &Path, to: To) -> String {
    match to {
        To::Woff2 => "woff2".into(),
        To::Woff => "woff".into(),
        To::Desktop => {
            let bytes = std::fs::read(input).unwrap_or_default();
            match desktop_bytes(&bytes).ok().and_then(|b| kind_of(&b)) {
                Some(Kind::Otf) => "otf".into(),
                _ => "ttf".into(),
            }
        }
    }
}

pub fn convert(input: &Path, to: To, output: &Path) -> Result<(), String> {
    let bytes = std::fs::read(input).map_err(|e| format!("Couldn't read {}: {e}", input.display()))?;
    let desktop = desktop_bytes(&bytes)?;
    let made = match to {
        To::Woff2 => to_woff2(&desktop)?,
        To::Woff => to_woff(&desktop)?,
        To::Desktop => {
            if !kind_of(&bytes).is_some_and(Kind::is_web) {
                return Err("This is a desktop font already.".into());
            }
            desktop
        }
    };
    std::fs::write(output, made).map_err(|e| format!("Couldn't save {}: {e}", output.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noto() -> Vec<u8> {
        std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../ui/vendor/fonts/NotoSans-Regular.ttf")).unwrap()
    }

    #[test]
    fn reads_names() {
        let m = meta(&noto()).unwrap();
        assert_eq!((m.full.as_str(), m.family.as_str(), m.style.as_str(), m.cff, m.kind), ("Noto Sans Regular", "Noto Sans", "Regular", false, Kind::Ttf));
        assert_eq!(reg_name(&m), "Noto Sans Regular (TrueType)");
    }

    #[test]
    fn registry_names_split_into_faces() {
        assert_eq!(reg_name_faces("Arial Bold & Arial Bold Italic (TrueType)"), vec!["Arial Bold", "Arial Bold Italic"]);
        assert_eq!(reg_name_faces("Satoshi Bold (OpenType)"), vec!["Satoshi Bold"]);
        assert_eq!(reg_name_faces("Wingdings"), vec!["Wingdings"]);
    }

    #[test]
    fn web_fonts_round_trip() {
        let ttf = noto();
        let w2 = to_woff2(&ttf).unwrap();
        assert_eq!(kind_of(&w2), Some(Kind::Woff2));
        assert!(w2.len() < ttf.len() * 8 / 10, "WOFF2 {} vs TTF {}", w2.len(), ttf.len());
        let w1 = to_woff(&ttf).unwrap();
        assert_eq!(kind_of(&w1), Some(Kind::Woff));
        assert!(w1.len() < ttf.len());
        for web in [&w2, &w1] {
            let back = desktop_bytes(web).unwrap();
            let (_, a) = tables(&ttf).unwrap();
            let (_, b) = tables(&back).unwrap();
            assert_eq!(a.len(), b.len());
            for (x, y) in a.iter().zip(&b) {
                assert_eq!(x.tag, y.tag);
                // Decoders write head's checkSumAdjustment afresh (bytes 8–11).
                let strip = |t: &Table| if &t.tag == b"head" { [&t.data[..8], &t.data[12..]].concat() } else { t.data.clone() };
                assert_eq!(strip(x), strip(y), "table {}", String::from_utf8_lossy(&x.tag));
            }
            assert_eq!(meta(web).unwrap().full, "Noto Sans Regular");
        }
    }

    #[test]
    fn installs_and_uninstalls_in_a_folder() {
        let _lock = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let d = std::env::temp_dir().join(format!("convertino-fonts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        // Only this test sets it, and the fonts tests that read it run in it.
        std::env::set_var("CONVERTINO_FONTS_DIR", &d);
        let bytes = noto();
        let m = meta(&bytes).unwrap();
        assert!(!installed_names().contains("noto sans regular"));
        let done = install(&[ToInstall { file_name: "NotoSans-Regular.ttf".into(), bytes: bytes.clone(), meta: m.clone() }], false).unwrap();
        assert_eq!(done[0].path, d.join("NotoSans-Regular.ttf"));
        assert!(installed_names().contains("noto sans regular"));
        // The same bytes again land on the same file; different bytes get their own name.
        let again = install(&[ToInstall { file_name: "NotoSans-Regular.ttf".into(), bytes: bytes.clone(), meta: m.clone() }], false).unwrap();
        assert_eq!(again[0].path, done[0].path);
        assert_eq!(uninstall(&done).unwrap(), 1);
        assert!(!d.join("NotoSans-Regular.ttf").exists());
        std::env::remove_var("CONVERTINO_FONTS_DIR");
        let _ = std::fs::remove_dir_all(&d);
    }
}
