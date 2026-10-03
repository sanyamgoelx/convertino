//! Puts the `convertino` command where terminals find it.
//!
//! Windows: the installer copies the command to `<install>\bin\convertino.exe`
//! and runs `convertino setup add <that folder>`, which adds the folder to the
//! user's PATH (HKCU\Environment). The uninstaller runs `setup remove`, which
//! takes it out again and disconnects AI apps that pointed at it.
//!
//! Mac: Settings › AI & CLI › "Install command…" links
//! `/usr/local/bin/convertino` to the command inside the app (macOS asks for
//! the password once).

use std::path::{Path, PathBuf};

#[cfg_attr(not(windows), allow(dead_code))]
fn same_dir(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.trim().trim_end_matches(['\\', '/']).to_lowercase();
    !a.trim().is_empty() && norm(a) == norm(b)
}

/// PATH with `dir` added at the end, or None if it's already there.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn path_with(path: &str, dir: &str) -> Option<String> {
    if path.split(';').any(|p| same_dir(p, dir)) {
        return None;
    }
    let base = path.trim_end_matches(';');
    Some(if base.is_empty() { dir.to_string() } else { format!("{base};{dir}") })
}

/// PATH without `dir`, or None if it wasn't there.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn path_without(path: &str, dir: &str) -> Option<String> {
    let parts: Vec<&str> = path.split(';').collect();
    let kept: Vec<&str> = parts.iter().copied().filter(|p| !same_dir(p, dir)).collect();
    (kept.len() != parts.len()).then(|| kept.join(";"))
}

#[cfg(windows)]
mod win {
    use ::windows::core::{HSTRING, PCWSTR};
    use ::windows::Win32::Foundation::{LPARAM, WPARAM};
    use ::windows::Win32::System::Registry::{RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_EXPAND_SZ, RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ};
    use ::windows::Win32::UI::WindowsAndMessaging::{SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE};

    pub fn read_user_path() -> String {
        let key = HSTRING::from("Environment");
        let name = HSTRING::from("Path");
        unsafe {
            let mut size = 0u32;
            let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND;
            if RegGetValueW(HKEY_CURRENT_USER, &key, &name, flags, None, None, Some(&mut size as *mut u32)).is_err() || size == 0 {
                return String::new();
            }
            let mut buf = vec![0u16; (size as usize).div_ceil(2) + 1];
            if RegGetValueW(HKEY_CURRENT_USER, &key, &name, flags, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut size as *mut u32)).is_err() {
                return String::new();
            }
            let len = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
            String::from_utf16_lossy(&buf[..len])
        }
    }

    pub fn write_user_path(value: &str) -> Result<(), String> {
        let key = HSTRING::from("Environment");
        let name = HSTRING::from("Path");
        let wide: Vec<u16> = value.encode_utf16().chain([0]).collect();
        unsafe {
            RegSetKeyValueW(HKEY_CURRENT_USER, &key, &name, REG_EXPAND_SZ.0, Some(wide.as_ptr() as *const _), (wide.len() * 2) as u32)
                .ok()
                .map_err(|e| e.message())?;
            // New terminals (and Explorer) pick up the change.
            let env: Vec<u16> = "Environment".encode_utf16().chain([0]).collect();
            let _ = SendMessageTimeoutW(HWND_BROADCAST, WM_SETTINGCHANGE, WPARAM(0), LPARAM(PCWSTR(env.as_ptr()).0 as isize), SMTO_ABORTIFHUNG, 3000, None);
        }
        Ok(())
    }
}

/// Adds `dir` to the user's PATH (Windows). True if it changed.
pub fn add_to_path(dir: &Path) -> Result<bool, String> {
    #[cfg(windows)]
    {
        let dir = crate::cli::strip_verbatim(dir).to_string_lossy().into_owned();
        match path_with(&win::read_user_path(), &dir) {
            Some(p) => win::write_user_path(&p).map(|_| true),
            None => Ok(false),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = dir;
        Ok(false)
    }
}

/// Takes `dir` out of the user's PATH (Windows). True if it changed.
pub fn remove_from_path(dir: &Path) -> Result<bool, String> {
    #[cfg(windows)]
    {
        let dir = crate::cli::strip_verbatim(dir).to_string_lossy().into_owned();
        match path_without(&win::read_user_path(), &dir) {
            Some(p) => win::write_user_path(&p).map(|_| true),
            None => Ok(false),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = dir;
        Ok(false)
    }
}

/// Whether a terminal opened now finds the command, and where.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandStatus {
    pub installed: bool,
    pub path: Option<PathBuf>,
    /// Mac: Settings offers "Install command…".
    pub can_install: bool,
}

#[cfg(target_os = "macos")]
const LINK: &str = "/usr/local/bin/convertino";

pub fn status() -> CommandStatus {
    let cli = crate::connect::cli_path();
    #[cfg(windows)]
    {
        let installed = cli.as_ref().and_then(|c| c.parent()).is_some_and(|bin| {
            bin.file_name().is_some_and(|n| n == "bin") && win::read_user_path().split(';').any(|p| same_dir(p, &bin.to_string_lossy()))
        });
        CommandStatus { installed, path: cli, can_install: false }
    }
    #[cfg(target_os = "macos")]
    {
        let link = std::fs::read_link(LINK).ok();
        let installed = matches!((&link, &cli), (Some(l), Some(c)) if l == c);
        CommandStatus { installed, path: cli.clone(), can_install: cli.is_some() }
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        CommandStatus { installed: false, path: cli, can_install: false }
    }
}

/// Mac: links /usr/local/bin/convertino to the command inside the app
/// (asks for the password). Elsewhere the installer does it.
pub fn install_command() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let cli = crate::connect::cli_path().ok_or("The command isn't inside this copy of Convertino.")?;
        let q = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"").replace('\'', "'\\''");
        let script = format!("mkdir -p /usr/local/bin && ln -sf '{}' '{LINK}'", q(&cli.to_string_lossy()));
        let apple = format!("do shell script \"{}\" with administrator privileges", script.replace('\\', "\\\\").replace('"', "\\\""));
        let out = std::process::Command::new("osascript").args(["-e", &apple]).output().map_err(|e| e.to_string())?;
        if out.status.success() {
            Ok(())
        } else {
            let err = String::from_utf8_lossy(&out.stderr);
            Err(if err.contains("-128") { "Cancelled.".into() } else { err.trim().to_string() })
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("The installer sets up the command on this system.".into())
    }
}

/// `convertino setup add|remove <bin folder>` (run by the Windows installer).
pub fn setup(args: &[String]) -> i32 {
    let (Some(action), Some(dir)) = (args.first(), args.get(1)) else {
        eprintln!("usage: convertino setup add|remove <folder>");
        return 2;
    };
    let dir = PathBuf::from(dir);
    let r = match action.as_str() {
        "add" => add_to_path(&dir).map(|_| ()),
        "remove" => {
            let r = remove_from_path(&dir).map(|_| ());
            // AI apps would keep trying to start a command that's gone.
            for s in crate::connect::status().into_iter().filter(|s| s.connected) {
                let points_here = s.config.as_deref().and_then(crate::connect::current).and_then(|e| e.get("command").and_then(|c| c.as_str()).map(PathBuf::from)).is_some_and(|c| c.starts_with(&dir));
                if points_here {
                    if let Some(app) = crate::connect::App::from_id(s.id) {
                        let _ = crate::connect::disconnect(app);
                    }
                }
            }
            r
        }
        other => Err(format!("unknown setup action {other}")),
    };
    match r {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adding_to_path() {
        let bin = r"C:\Users\a\AppData\Local\Convertino\bin";
        assert_eq!(path_with("", bin).unwrap(), bin);
        assert_eq!(path_with(r"C:\x;C:\y", bin).unwrap(), format!(r"C:\x;C:\y;{bin}"));
        assert_eq!(path_with(r"C:\x;", bin).unwrap(), format!(r"C:\x;{bin}"));
        // Already there (any case, with or without a trailing backslash): unchanged.
        assert_eq!(path_with(&format!(r"C:\x;{}\", bin.to_uppercase()), bin), None);
        // %VARIABLES% are kept as they are.
        assert!(path_with(r"%USERPROFILE%\bin", bin).unwrap().starts_with(r"%USERPROFILE%\bin;"));
    }

    #[test]
    fn removing_from_path() {
        let bin = r"C:\Users\a\AppData\Local\Convertino\bin";
        assert_eq!(path_without(&format!(r"C:\x;{bin};C:\y"), bin).unwrap(), r"C:\x;C:\y");
        assert_eq!(path_without(&format!(r"{bin}\"), bin).unwrap(), "");
        assert_eq!(path_without(r"C:\x;C:\y", bin), None);
        // Removing what adding did gives the original back.
        let original = r"C:\x;%USERPROFILE%\go\bin";
        assert_eq!(path_without(&path_with(original, bin).unwrap(), bin).unwrap(), original);
    }
}
