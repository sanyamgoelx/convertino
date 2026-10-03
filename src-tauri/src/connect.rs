//! Connects Convertino's MCP server to AI apps (Settings › AI & CLI).
//!
//! "Connect" adds an entry to the app's MCP settings file:
//!
//! ```json
//! "mcpServers": { "convertino": { "command": "<the convertino command>", "args": ["mcp"] } }
//! ```
//!
//! Other servers and settings in the file are kept as they are (same order),
//! a copy of the original is kept once as `<file>.convertino-backup`, and a
//! file that isn't valid JSON is never touched. Disconnect removes only our
//! entry. Claude Code is connected with its own `claude mcp add` command when
//! it can be found, since it rewrites its settings file while running.

use serde::Serialize;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const NAME: &str = "convertino";

/// The `convertino` command next to the running app (what AI apps start).
pub fn cli_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().and_then(std::fs::canonicalize).ok().map(|p| crate::cli::strip_verbatim(&p))?;
    let dir = exe.parent()?;
    let name = |n: &str| if cfg!(windows) { format!("{n}.exe") } else { n.to_string() };
    // Installed on Windows: <install>\bin\convertino.exe (on PATH); Mac and dev builds: beside the app.
    [dir.join("bin").join(name("convertino")), dir.join(name("convertino-cli"))].into_iter().find(|p| p.is_file()).map(|p| crate::cli::strip_verbatim(&p))
}

fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// Our entry for an MCP settings file.
pub fn entry(cli: &Path) -> Value {
    json!({ "command": cli.to_string_lossy(), "args": ["mcp"] })
}

#[derive(Debug, PartialEq)]
pub enum MergeError {
    /// Not JSON, or not shaped like an MCP settings file: left alone.
    Malformed(String),
}

/// The settings file's text with our entry set (Some) or removed (None).
/// `text`: the file as it is (None: there's no file yet).
pub fn merge(text: Option<&str>, ours: Option<Value>) -> Result<Option<String>, MergeError> {
    let text = text.map(|t| t.trim_start_matches('\u{feff}')).filter(|t| !t.trim().is_empty());
    let mut root: Value = match text {
        None => Value::Object(Map::new()),
        Some(t) => serde_json::from_str(t).map_err(|e| MergeError::Malformed(format!("isn't valid JSON ({e})")))?,
    };
    let Some(obj) = root.as_object_mut() else { return Err(MergeError::Malformed("isn't a JSON object".into())) };
    match ours {
        Some(entry) => {
            let servers = obj.entry("mcpServers").or_insert_with(|| Value::Object(Map::new()));
            let Some(servers) = servers.as_object_mut() else { return Err(MergeError::Malformed("has an mcpServers that isn't an object".into())) };
            if servers.get(NAME) == Some(&entry) {
                return Ok(None); // already so: nothing to write
            }
            servers.insert(NAME.into(), entry);
        }
        None => {
            let Some(servers) = obj.get_mut("mcpServers").and_then(Value::as_object_mut) else { return Ok(None) };
            if servers.remove(NAME).is_none() {
                return Ok(None);
            }
        }
    }
    let mut out = serde_json::to_string_pretty(&root).map_err(|e| MergeError::Malformed(e.to_string()))?;
    out.push('\n');
    Ok(Some(out))
}

/// Our entry in a settings file, if any.
pub fn current(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    let v: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()?;
    v.get("mcpServers")?.get(NAME).cloned()
}

/// Sets or removes our entry in `path`, keeping a backup of the original.
pub fn write_entry(path: &Path, ours: Option<Value>) -> Result<bool, String> {
    let before = std::fs::read_to_string(path).ok();
    let Some(after) = merge(before.as_deref(), ours).map_err(|MergeError::Malformed(why)| format!("{} {why}, so Convertino left it alone.", path.display()))? else {
        return Ok(false);
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
    }
    let backup = path.with_file_name(format!("{}.convertino-backup", path.file_name().unwrap_or_default().to_string_lossy()));
    if before.is_some() && !backup.exists() {
        let _ = std::fs::copy(path, &backup);
    }
    let tmp = path.with_file_name(format!("{}.convertino-tmp", path.file_name().unwrap_or_default().to_string_lossy()));
    std::fs::write(&tmp, after).map_err(|e| format!("Couldn't write {}: {e}", path.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("Couldn't write {}: {e}", path.display())
    })?;
    Ok(true)
}

// ---------- the apps ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum App {
    ClaudeDesktop,
    ClaudeCode,
    Cursor,
}

impl App {
    pub const ALL: [App; 3] = [App::ClaudeDesktop, App::ClaudeCode, App::Cursor];

    pub fn id(self) -> &'static str {
        match self {
            App::ClaudeDesktop => "claude-desktop",
            App::ClaudeCode => "claude-code",
            App::Cursor => "cursor",
        }
    }

    pub fn from_id(id: &str) -> Option<App> {
        App::ALL.into_iter().find(|a| a.id() == id)
    }

    pub fn name(self) -> &'static str {
        match self {
            App::ClaudeDesktop => "Claude Desktop",
            App::ClaudeCode => "Claude Code",
            App::Cursor => "Cursor",
        }
    }

    /// Its MCP settings file, and whether the app seems to be installed.
    pub fn config(self) -> Option<(PathBuf, bool)> {
        let home = home()?;
        match self {
            App::ClaudeDesktop => {
                if cfg!(windows) {
                    let appdata = PathBuf::from(std::env::var_os("APPDATA")?);
                    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
                    // The Microsoft Store / MSIX app reads its own copy when it has one.
                    let packaged = local.as_ref().and_then(|l| {
                        std::fs::read_dir(l.join("Packages")).ok()?.flatten().map(|e| e.path()).find(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("Claude_")))
                    });
                    if let Some(pkg) = &packaged {
                        let private = pkg.join("LocalCache").join("Roaming").join("Claude").join("claude_desktop_config.json");
                        if private.is_file() {
                            return Some((private, true));
                        }
                    }
                    let found = packaged.is_some() || appdata.join("Claude").is_dir() || local.is_some_and(|l| l.join("AnthropicClaude").is_dir());
                    Some((appdata.join("Claude").join("claude_desktop_config.json"), found))
                } else if cfg!(target_os = "macos") {
                    let dir = home.join("Library/Application Support/Claude");
                    let found = dir.is_dir() || Path::new("/Applications/Claude.app").is_dir() || home.join("Applications/Claude.app").is_dir();
                    Some((dir.join("claude_desktop_config.json"), found))
                } else {
                    let dir = home.join(".config/Claude");
                    Some((dir.join("claude_desktop_config.json"), dir.is_dir()))
                }
            }
            App::ClaudeCode => {
                let f = home.join(".claude.json");
                let found = f.is_file() || home.join(".claude").is_dir() || claude_command().is_some();
                Some((f, found))
            }
            App::Cursor => {
                let dir = home.join(".cursor");
                Some((dir.join("mcp.json"), dir.is_dir()))
            }
        }
    }
}

/// Claude Code's `claude` command, wherever its installers put it (an app
/// started from the Dock or Start menu doesn't get the terminal's PATH).
fn claude_command() -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) { &["claude.exe", "claude.cmd"] } else { &["claude"] };
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    if let Some(h) = home() {
        dirs.push(h.join(".local").join("bin"));
        dirs.push(h.join(".claude").join("local"));
        if cfg!(windows) {
            if let Some(a) = std::env::var_os("APPDATA") {
                dirs.push(PathBuf::from(a).join("npm"));
            }
        }
    }
    if !cfg!(windows) {
        dirs.push("/opt/homebrew/bin".into());
        dirs.push("/usr/local/bin".into());
    }
    dirs.iter().flat_map(|d| names.iter().map(move |n| d.join(n))).find(|p| p.is_file())
}

fn run_claude(args: &[&str]) -> Result<(), String> {
    let exe = claude_command().ok_or("Claude Code's command wasn't found")?;
    let mut cmd = if exe.extension().is_some_and(|e| e == "cmd") {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(&exe);
        c
    } else {
        Command::new(&exe)
    };
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = crate::procs::output(&mut cmd, "Claude Code", Some(std::time::Duration::from_secs(60)))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// How an app stands, for Settings.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStatus {
    pub id: &'static str,
    pub name: &'static str,
    pub found: bool,
    /// Our entry is there.
    pub connected: bool,
    /// Our entry points at another copy of Convertino (it moved): reconnect.
    pub outdated: bool,
    pub config: Option<PathBuf>,
}

pub fn status() -> Vec<AppStatus> {
    let cli = cli_path();
    App::ALL
        .into_iter()
        .map(|app| {
            let (config, found) = match app.config() {
                Some((p, f)) => (Some(p), f),
                None => (None, false),
            };
            let ours = config.as_deref().and_then(current);
            let outdated = match (&ours, &cli) {
                (Some(e), Some(c)) => e.get("command").and_then(Value::as_str) != Some(&*c.to_string_lossy()),
                _ => false,
            };
            AppStatus { id: app.id(), name: app.name(), found, connected: ours.is_some(), outdated, config }
        })
        .collect()
}

pub fn connect(app: App) -> Result<(), String> {
    let cli = cli_path().ok_or("The convertino command isn't installed next to the app. Reinstall Convertino.")?;
    let (config, _) = app.config().ok_or("Couldn't find your home folder.")?;
    if app == App::ClaudeCode && claude_command().is_some() {
        let _ = run_claude(&["mcp", "remove", "--scope", "user", NAME]);
        let c = cli.to_string_lossy().into_owned();
        match run_claude(&["mcp", "add", "--scope", "user", NAME, "--", &c, "mcp"]) {
            Ok(()) => return Ok(()),
            Err(e) => log::warn!("connect: claude mcp add failed ({e}); editing {} instead", config.display()),
        }
    }
    write_entry(&config, Some(entry(&cli))).map(|_| ())
}

pub fn disconnect(app: App) -> Result<(), String> {
    let (config, _) = app.config().ok_or("Couldn't find your home folder.")?;
    if app == App::ClaudeCode && claude_command().is_some() && run_claude(&["mcp", "remove", "--scope", "user", NAME]).is_ok() {
        return Ok(());
    }
    write_entry(&config, None).map(|_| ())
}

/// At startup: entries that point at an old copy of Convertino (moved, or a
/// different install folder) are pointed at this one.
pub fn repair() {
    for s in status().into_iter().filter(|s| s.outdated) {
        if let Some(app) = App::from_id(s.id) {
            match connect(app) {
                Ok(()) => log::info!("connect: {} now points at this copy of Convertino", s.name),
                Err(e) => log::warn!("connect: couldn't update {}: {e}", s.name),
            }
        }
    }
}

/// The setup to paste into any other AI app.
pub fn setup_snippet() -> String {
    let cli = cli_path().unwrap_or_else(|| PathBuf::from(if cfg!(windows) { "convertino.exe" } else { "convertino" }));
    let v = json!({ "mcpServers": { NAME: entry(&cli) } });
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ours() -> Value {
        entry(Path::new("C:\\Program Files\\Convertino\\bin\\convertino.exe"))
    }

    #[test]
    fn new_file() {
        let out = merge(None, Some(ours())).unwrap().unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["mcpServers"]["convertino"]["args"][0], "mcp");
        assert_eq!(merge(Some("   \n"), Some(ours())).unwrap().unwrap(), out, "empty file = no file");
    }

    #[test]
    fn keeps_everything_else_in_order() {
        let before = r#"{"globalShortcut":"Ctrl+Space","mcpServers":{"zeta":{"command":"z"},"alpha":{"command":"a","env":{"K":"V"}}},"theme":"dark"}"#;
        let out = merge(Some(before), Some(ours())).unwrap().unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["globalShortcut"], "Ctrl+Space");
        assert_eq!(v["theme"], "dark");
        assert_eq!(v["mcpServers"]["alpha"]["env"]["K"], "V");
        let keys: Vec<&String> = v["mcpServers"].as_object().unwrap().keys().collect();
        assert_eq!(keys, vec!["zeta", "alpha", "convertino"]);
        let top: Vec<&String> = v.as_object().unwrap().keys().collect();
        assert_eq!(top, vec!["globalShortcut", "mcpServers", "theme"]);
    }

    #[test]
    fn connect_then_disconnect_gives_the_original_back() {
        let before = r#"{"mcpServers":{"other":{"command":"x","args":["y"]}}}"#;
        let connected = merge(Some(before), Some(ours())).unwrap().unwrap();
        let back = merge(Some(&connected), None).unwrap().unwrap();
        let a: Value = serde_json::from_str(before).unwrap();
        let b: Value = serde_json::from_str(&back).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn nothing_to_write_when_already_so() {
        let connected = merge(None, Some(ours())).unwrap().unwrap();
        assert_eq!(merge(Some(&connected), Some(ours())).unwrap(), None);
        assert_eq!(merge(Some(r#"{"mcpServers":{}}"#), None).unwrap(), None);
        assert_eq!(merge(Some(r#"{"a":1}"#), None).unwrap(), None);
    }

    #[test]
    fn an_old_path_is_replaced() {
        let old = merge(None, Some(entry(Path::new("/Old/Convertino.app/Contents/MacOS/convertino-cli")))).unwrap().unwrap();
        let new = merge(Some(&old), Some(ours())).unwrap().unwrap();
        assert!(new.contains("Program Files"));
        assert!(!new.contains("/Old/"));
    }

    #[test]
    fn bom_and_crlf_are_fine() {
        let before = "\u{feff}{\r\n  \"mcpServers\": {\r\n    \"x\": {\"command\": \"y\"}\r\n  }\r\n}\r\n";
        let out = merge(Some(before), Some(ours())).unwrap().unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["mcpServers"]["x"]["command"], "y");
        assert!(!out.starts_with('\u{feff}'));
    }

    #[test]
    fn broken_files_are_left_alone() {
        for bad in ["{\"mcpServers\": {", "[1,2]", "\"text\"", r#"{"mcpServers": []}"#, r#"{"mcpServers": "x"}"#] {
            assert!(matches!(merge(Some(bad), Some(ours())), Err(MergeError::Malformed(_))), "{bad}");
        }
    }

    #[test]
    fn writes_atomically_with_one_backup() {
        let dir = std::env::temp_dir().join(format!("convertino-connect-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("claude_desktop_config.json");
        let original = r#"{"mcpServers":{"other":{"command":"x"}}}"#;
        std::fs::write(&file, original).unwrap();
        assert!(write_entry(&file, Some(ours())).unwrap());
        assert!(current(&file).is_some());
        let backup = dir.join("claude_desktop_config.json.convertino-backup");
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), original);
        // Again: nothing changes, the backup stays the original.
        assert!(!write_entry(&file, Some(ours())).unwrap());
        assert!(write_entry(&file, None).unwrap());
        assert!(current(&file).is_none());
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), original);
        assert!(!dir.join("claude_desktop_config.json.convertino-tmp").exists());
        // A broken file is never written.
        std::fs::write(&file, "{ broken").unwrap();
        assert!(write_entry(&file, Some(ours())).unwrap_err().contains("left it alone"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{ broken");
        // A file that doesn't exist yet (the app was never opened) is created with its folder.
        let fresh = dir.join("new").join("mcp.json");
        assert!(write_entry(&fresh, Some(ours())).unwrap());
        assert!(current(&fresh).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn snippet_is_valid_json() {
        let v: Value = serde_json::from_str(&setup_snippet()).unwrap();
        assert_eq!(v["mcpServers"]["convertino"]["args"][0], "mcp");
    }

    #[test]
    fn app_ids_round_trip() {
        for a in App::ALL {
            assert_eq!(App::from_id(a.id()), Some(a));
        }
    }
}
