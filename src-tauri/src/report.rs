//! Error reports: a failed job's card offers "Send report", which posts what
//! went wrong to the developer's Discord channel (a webhook baked in at build
//! time from the CONVERTINO_REPORT_WEBHOOK secret; without it there's no button).
//!
//! What's sent: Convertino's version, the system, the action, the kinds of
//! files (extensions and sizes, never names), the error, which converters are
//! installed, and the recent log. File and folder names in the error and the
//! log are replaced by placeholders before anything leaves the computer.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Manager};

const WEBHOOK: Option<&str> = option_env!("CONVERTINO_REPORT_WEBHOOK");

/// Whether this build can send reports.
pub fn enabled() -> bool {
    WEBHOOK.is_some_and(|w| w.starts_with("https://"))
}

struct Failure {
    action: String,
    inputs: Vec<(String, u64)>,
    errors: Vec<String>,
}

fn failures() -> &'static Mutex<HashMap<u64, Failure>> {
    static F: OnceLock<Mutex<HashMap<u64, Failure>>> = OnceLock::new();
    F.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Keeps a failed job's details for "Send report"; true when the card should offer it.
pub fn remember(id: u64, action: &str, files: &[PathBuf], errors: &[String]) -> bool {
    if !enabled() {
        return false;
    }
    let inputs = files
        .iter()
        .take(20)
        .map(|f| {
            let ext = f.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_else(|| "(none)".into());
            (ext, f.metadata().map(|m| m.len()).unwrap_or(0))
        })
        .collect();
    if let Ok(mut m) = failures().lock() {
        m.insert(id, Failure { action: action.into(), inputs, errors: errors.iter().take(5).cloned().collect() });
    }
    true
}

/// "Send report" on a failed job's card. `note`: what the person typed, if anything.
pub fn send(app: &AppHandle, id: u64, note: Option<String>) -> Result<(), String> {
    let webhook = WEBHOOK.filter(|_| enabled()).ok_or("Reports aren't set up in this version.")?;
    let failure = failures().lock().ok().and_then(|mut m| m.remove(&id)).ok_or("This report was already sent.")?;
    let text = build(app, &failure, note.as_deref());
    let summary = format!(
        "**{}** on {} · Convertino {}\n{}",
        failure.action,
        system(),
        app.package_info().version,
        scrub(failure.errors.first().map(String::as_str).unwrap_or("")).chars().take(300).collect::<String>()
    );
    post(webhook, &summary, &text)
}

fn build(app: &AppHandle, f: &Failure, note: Option<&str>) -> String {
    let mut t = String::new();
    t.push_str(&format!("Convertino {}\n", app.package_info().version));
    t.push_str(&format!("System: {}\n", system()));
    t.push_str(&format!("Action: {}\n", f.action));
    let files: Vec<String> = f.inputs.iter().map(|(ext, size)| format!(".{ext} ({:.1} MB)", *size as f64 / 1e6)).collect();
    t.push_str(&format!("Files: {}\n", files.join(", ")));
    if let Some(n) = note.map(str::trim).filter(|n| !n.is_empty()) {
        t.push_str(&format!("\nWhat they were doing:\n{}\n", n.chars().take(1000).collect::<String>()));
    }
    t.push_str("\nErrors:\n");
    for e in &f.errors {
        t.push_str(&format!("- {}\n", scrub(e)));
    }
    t.push_str("\nConverters:\n");
    for s in crate::install::status() {
        t.push_str(&format!("- {}: {}{}\n", s.name, s.state, s.version.map(|v| format!(" {v}")).unwrap_or_default()));
    }
    let set = crate::settings::get();
    t.push_str(&format!("\nSettings: ffmpeg {}, save mode {}\n", set.ffmpeg_build, set.save_mode));
    t.push_str("\nRecent log:\n");
    t.push_str(&recent_log(app, 250));
    t
}

/// The last `lines` lines of Convertino's log, scrubbed.
fn recent_log(app: &AppHandle, lines: usize) -> String {
    let Ok(dir) = app.path().app_log_dir() else { return "(no log)\n".into() };
    let mut logs: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "log")).collect())
        .unwrap_or_default();
    // Oldest first, so the newest lines come last.
    logs.sort_by_key(|p| p.metadata().and_then(|m| m.modified()).ok());
    let mut all: Vec<String> = Vec::new();
    for p in logs {
        if let Ok(bytes) = std::fs::read(&p) {
            all.extend(String::from_utf8_lossy(&bytes).lines().map(String::from));
        }
    }
    let start = all.len().saturating_sub(lines);
    all[start..].iter().map(|l| scrub(l) + "\n").collect()
}

/// Windows 11 (build 26100) x86_64 / macOS 15.3 arm64.
fn system() -> String {
    #[cfg(windows)]
    {
        let build = reg_value(r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion", "CurrentBuild").unwrap_or_default();
        let display = reg_value(r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion", "DisplayVersion").unwrap_or_default();
        let n: u32 = build.parse().unwrap_or(0);
        let name = if n >= 22000 { "Windows 11" } else { "Windows 10" };
        format!("{name} {display} (build {build}) {}", std::env::consts::ARCH)
    }
    #[cfg(target_os = "macos")]
    {
        let v = std::process::Command::new("sw_vers")
            .arg("-productVersion")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        format!("macOS {v} {}", std::env::consts::ARCH)
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        format!("{} {}", std::env::consts::OS, std::env::consts::ARCH)
    }
}

#[cfg(windows)]
fn reg_value(key: &str, name: &str) -> Option<String> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("reg.exe")
        .args(["query", key, "/v", name])
        .creation_flags(0x0800_0000)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let line = text.lines().find(|l| l.trim_start().starts_with(name))?;
    line.split("REG_SZ").nth(1).map(|v| v.trim().to_string())
}

/// Hides file and folder names: the home folder becomes "~", and a path
/// outside Convertino's own folders keeps only its extension
/// ("C:\Users\Asha\Videos\trip.mp4" -> "~\…\<file>.mp4").
pub fn scrub(text: &str) -> String {
    let mut s = text.to_string();
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
    if home.len() > 3 {
        s = replace_ci(&s, &home, "~");
    }
    // Paths: a drive letter, "~", or "/" followed by path characters.
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let starts = (i + 2 < chars.len() && chars[i].is_ascii_alphabetic() && chars[i + 1] == ':' && (chars[i + 2] == '\\' || chars[i + 2] == '/')
            && (i == 0 || !chars[i - 1].is_ascii_alphanumeric()))
            || (chars[i] == '~' && i + 1 < chars.len() && (chars[i + 1] == '\\' || chars[i + 1] == '/'))
            || (chars[i] == '/' && i + 1 < chars.len() && chars[i + 1].is_ascii_alphabetic() && (i == 0 || chars[i - 1] == ' ' || chars[i - 1] == '"' || chars[i - 1] == '\''));
        if !starts {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        // A path runs to a quote, a parenthesis, a line end, or ": " / ", ".
        let mut j = i;
        while j < chars.len() {
            let c = chars[j];
            if c == '"' || c == '\'' || c == '\n' || c == '(' || c == ')' || c == '<' || c == '>' {
                break;
            }
            if (c == ':' || c == ',') && j > i + 2 && chars.get(j + 1) == Some(&' ') {
                break;
            }
            j += 1;
        }
        let path: String = chars[i..j].iter().collect();
        out.push_str(&scrub_path(path.trim_end()));
        out.push_str(&chars[i + path.trim_end().chars().count()..j].iter().collect::<String>());
        i = j;
    }
    out
}

fn scrub_path(p: &str) -> String {
    let lower = p.to_lowercase().replace('\\', "/");
    let ours = ["/convertino/tools", "/program files", "/windows/", "/appdata/local/temp", "/tmp/", "/applications/", "/library/application support/convertino", "/opt/homebrew", "/usr/"];
    if ours.iter().any(|o| lower.contains(o)) || lower.contains("com.crofty.convertino") {
        return p.to_string();
    }
    let ext = Path::new(p).extension().map(|e| e.to_string_lossy().to_string());
    let root = if p.starts_with('~') { "~" } else if p.chars().nth(1) == Some(':') { &p[..2] } else { "" };
    match ext.filter(|e| e.len() <= 5 && !e.contains(' ')) {
        Some(e) => format!("{root}…<file>.{e}"),
        None => format!("{root}…<folder>"),
    }
}

fn replace_ci(s: &str, what: &str, with: &str) -> String {
    let lower = s.to_lowercase();
    let w = what.to_lowercase();
    let mut out = String::new();
    let mut last = 0;
    for (i, _) in lower.match_indices(&w) {
        out.push_str(&s[last..i]);
        out.push_str(with);
        last = i + w.len();
    }
    out.push_str(&s[last..]);
    out
}

/// Posts the summary and the full report (as report.txt) to the webhook.
fn post(webhook: &str, summary: &str, text: &str) -> Result<(), String> {
    let dir = std::env::temp_dir().join(format!("convertino-report-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let file = dir.join("report.txt");
    std::fs::write(&file, text).map_err(|e| format!("Couldn't prepare the report: {e}"))?;
    let payload = serde_json::json!({
        "username": "Convertino reports",
        "content": summary.chars().take(1900).collect::<String>(),
        "allowed_mentions": { "parse": [] },
    });
    let payload_file = dir.join("payload.json");
    std::fs::write(&payload_file, payload.to_string()).map_err(|e| format!("Couldn't prepare the report: {e}"))?;

    #[cfg(windows)]
    let curl = PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into())).join("System32").join("curl.exe");
    #[cfg(not(windows))]
    let curl = PathBuf::from("/usr/bin/curl");
    let mut cmd = crate::tools::command(&curl);
    cmd.args(["-fsS", "--connect-timeout", "20", "-F"])
        .arg(format!("payload_json=<{}", payload_file.display()))
        .arg("-F")
        .arg(format!("files[0]=@{};filename=report.txt", file.display()))
        .arg(webhook);
    let out = crate::procs::output(&mut cmd, "curl", Some(Duration::from_secs(60)));
    let _ = std::fs::remove_dir_all(&dir);
    match out {
        Ok(o) if o.status.success() => {
            log::info!("report sent");
            Ok(())
        }
        Ok(o) => {
            log::warn!("report not sent: {}", String::from_utf8_lossy(&o.stderr).trim());
            Err("The report couldn't be sent. Check the internet connection and try again.".into())
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_names() {
        let s = scrub("ffmpeg failed on C:\\Data\\Holiday trip\\clip one.mp4: Permission denied");
        assert!(!s.contains("Holiday"), "{s}");
        assert!(s.contains("<file>.mp4"), "{s}");
        assert!(s.contains(": Permission denied"), "{s}");
        let t = scrub("found D:\\x\\Convertino\\tools\\ghostscript\\bin\\gswin64c.exe");
        assert!(t.contains("gswin64c.exe"), "{t}");
        let u = scrub("job 3: made /Users/asha/Desktop/secret plan.pdf");
        assert!(!u.contains("secret"), "{u}");
        assert!(u.contains("<file>.pdf"), "{u}");
    }
}
