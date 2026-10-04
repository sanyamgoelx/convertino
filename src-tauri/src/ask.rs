//! "Ask Claude" on the wheel: opens Claude with the files you right-clicked,
//! so you can tell it what to do with them (Claude then converts through
//! Convertino's MCP server).
//!
//! It uses Claude's own desktop links
//! (https://support.claude.com/en/articles/14729294-open-claude-desktop-with-a-link):
//!
//! - Cowork: `claude://cowork/new?file=…&folder=…&q=…` attaches the files (Claude
//!   asks the user to allow each one, so many files from one folder attach the
//!   folder instead, and the message names the files).
//! - Chat:   `claude://claude.ai/new?q=…` can only fill in text: the file paths.
//! - Code:   `claude://code/new?folder=…&q=…` opens a Claude Code session in the files' folder.
//!
//! The button shows only when Convertino is connected to Claude (Claude
//! Desktop for Cowork and Chat, Claude Code for Code) and AI apps are allowed
//! in Settings: without the connection Claude couldn't convert anything.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Cowork,
    Chat,
    Code,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Cowork, Mode::Chat, Mode::Code];

    pub fn id(self) -> &'static str {
        match self {
            Mode::Cowork => "cowork",
            Mode::Chat => "chat",
            Mode::Code => "code",
        }
    }

    pub fn from_id(id: &str) -> Option<Mode> {
        Mode::ALL.into_iter().find(|m| m.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Mode::Cowork => "Cowork",
            Mode::Chat => "Chat",
            Mode::Code => "Claude Code",
        }
    }

    /// What it does with the files, for the wheel's hint and Settings.
    pub fn does(self) -> &'static str {
        match self {
            Mode::Cowork => "attaches the files",
            Mode::Chat => "writes the file paths into a new chat",
            Mode::Code => "opens a session in the files' folder",
        }
    }
}

/// Which Claude apps have Convertino connected.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Connected {
    pub desktop: bool,
    pub code: bool,
}

impl Connected {
    /// The ways Ask Claude can open, best first.
    pub fn modes(self) -> Vec<Mode> {
        let mut m = Vec::new();
        if self.desktop {
            m.push(Mode::Cowork);
            m.push(Mode::Chat);
        }
        if self.code {
            m.push(Mode::Code);
        }
        m
    }
}

/// The mode to use: the one in Settings if it's available, else the first that is.
pub fn pick_mode(wanted: &str, available: &[Mode]) -> Option<Mode> {
    Mode::from_id(wanted).filter(|m| available.contains(m)).or_else(|| available.first().copied())
}

// ---------- is Convertino connected? ----------
// Reading the AI apps' settings files takes a moment (Claude Code's can be
// several MB), and the wheel must open at once: the answer is kept and
// refreshed in the background when it's older than a few seconds.

static CACHE: Mutex<Option<(Instant, Connected)>> = Mutex::new(None);
const FRESH: Duration = Duration::from_secs(15);

fn read_connected() -> Connected {
    let mut c = Connected::default();
    for s in crate::connect::status() {
        match s.id {
            "claude-desktop" => c.desktop = s.connected,
            "claude-code" => c.code = s.connected,
            _ => {}
        }
    }
    c
}

/// Reads the connection state now (at startup, and after Connect / Disconnect).
pub fn refresh() -> Connected {
    let c = read_connected();
    if let Ok(mut g) = CACHE.lock() {
        *g = Some((Instant::now(), c));
    }
    c
}

/// The last known state, refreshed in the background when it's stale.
pub fn connected() -> Connected {
    let cached = CACHE.lock().ok().and_then(|g| *g);
    match cached {
        Some((at, c)) => {
            if at.elapsed() > FRESH {
                std::thread::spawn(refresh);
            }
            c
        }
        None => refresh(),
    }
}

// ---------- what the wheel shows ----------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModeInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub does: &'static str,
}

/// The Ask Claude button on the wheel (absent: don't show it).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WheelAsk {
    /// The mode a plain click uses.
    pub default: &'static str,
    /// What Shift+click offers.
    pub modes: Vec<ModeInfo>,
}

pub fn for_wheel(prefs: &crate::settings::Settings, connected: Connected) -> Option<WheelAsk> {
    if !prefs.ai_apps || !prefs.ask_claude {
        return None;
    }
    let modes = connected.modes();
    let default = pick_mode(&prefs.ask_mode, &modes)?;
    Some(WheelAsk {
        default: default.id(),
        modes: modes.into_iter().map(|m| ModeInfo { id: m.id(), label: m.label(), does: m.does() }).collect(),
    })
}

// ---------- the link ----------

/// Up to this many files attach one by one (Claude asks to allow each).
pub const MAX_FILES: usize = 5;
/// Kept well under what Windows passes to the app that opens the link.
const MAX_LINK: usize = 8000;

/// Percent-encodes a query value (RFC 3986 unreserved characters stay).
fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn query(base: &str, params: &[(&str, String)]) -> String {
    let q: Vec<String> = params.iter().filter(|(_, v)| !v.is_empty()).map(|(k, v)| format!("{k}={}", enc(v))).collect();
    if q.is_empty() {
        base.to_string()
    } else {
        format!("{base}?{}", q.join("&"))
    }
}

fn name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string_lossy().into_owned())
}

fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// The folder an item belongs to: a folder is its own, a file its parent's.
fn home_of(p: &Path) -> PathBuf {
    if p.is_dir() {
        p.to_path_buf()
    } else {
        p.parent().map(Path::to_path_buf).unwrap_or_else(|| p.to_path_buf())
    }
}

/// The folders the items are in, in order of first appearance.
fn folders(items: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for p in items {
        let f = home_of(p);
        if !out.contains(&f) {
            out.push(f);
        }
    }
    out
}

/// The deepest folder all the items are in.
fn common_folder(items: &[PathBuf]) -> Option<PathBuf> {
    let mut it = items.iter().map(|p| home_of(p));
    let mut common = it.next()?;
    for f in it {
        while !f.starts_with(&common) {
            common = common.parent()?.to_path_buf();
        }
    }
    Some(common)
}

/// "IMG_0411.CR3, IMG_0412.CR3 and 3 more", short enough for a message.
fn names(items: &[PathBuf], budget: usize) -> String {
    let all: Vec<String> = items.iter().map(|p| name_of(p)).collect();
    let mut shown = Vec::new();
    let mut used = 0;
    for n in &all {
        if used + n.len() + 2 > budget && !shown.is_empty() {
            break;
        }
        used += n.len() + 2;
        shown.push(n.clone());
    }
    let rest = all.len() - shown.len();
    match (shown.len(), rest) {
        (_, 0) if shown.len() > 1 => {
            let last = shown.pop().unwrap_or_default();
            format!("{} and {last}", shown.join(", "))
        }
        (_, 0) => shown.join(""),
        _ => format!("{} and {rest} more", shown.join(", ")),
    }
}

/// The link that opens Claude with `items` (files or folders, absolute paths).
pub fn link(mode: Mode, items: &[PathBuf]) -> String {
    match mode {
        Mode::Cowork => cowork_link(items),
        Mode::Chat => chat_link(items),
        Mode::Code => code_link(items),
    }
}

fn cowork_link(items: &[PathBuf]) -> String {
    const BASE: &str = "claude://cowork/new";
    let files: Vec<&PathBuf> = items.iter().filter(|p| !p.is_dir()).collect();
    let dirs: Vec<&PathBuf> = items.iter().filter(|p| p.is_dir()).collect();
    // A few things: attach each one.
    if items.len() <= MAX_FILES {
        let mut params: Vec<(&str, String)> = dirs.iter().map(|d| ("folder", path_str(d))).collect();
        params.extend(files.iter().map(|f| ("file", path_str(f))));
        let url = query(BASE, &params);
        if url.len() <= MAX_LINK {
            return url;
        }
    }
    // Many: attach their folders (one approval each) and say which files.
    let mut fs = folders(items);
    fs.truncate(MAX_FILES);
    let mut params: Vec<(&str, String)> = fs.iter().map(|d| ("folder", path_str(d))).collect();
    let picked: Vec<PathBuf> = files.iter().map(|p| (*p).clone()).collect();
    if !picked.is_empty() {
        let used = query(BASE, &params).len() + 40;
        let budget = MAX_LINK.saturating_sub(used) / 3;
        params.push(("q", format!("The files I picked ({}): {}\n\n", picked.len(), names(&picked, budget.max(60)))));
    }
    query(BASE, &params)
}

fn chat_link(items: &[PathBuf]) -> String {
    const BASE: &str = "claude://claude.ai/new";
    let mut lines: Vec<String> = Vec::new();
    let mut used = BASE.len() + 3;
    let mut left = items.len();
    for p in items {
        let line = path_str(p);
        // Each character can take up to 3 once encoded (plus the line break).
        let cost = enc(&line).len() + 3;
        if used + cost > MAX_LINK - 120 {
            break;
        }
        used += cost;
        lines.push(line);
        left -= 1;
    }
    if left > 0 {
        let rest: Vec<PathBuf> = items[items.len() - left..].to_vec();
        let where_ = common_folder(&rest).map(|f| format!(" in {}", path_str(&f))).unwrap_or_default();
        lines.push(format!("…and {left} more{where_}"));
    }
    query(BASE, &[("q", format!("{}\n\n", lines.join("\n")))])
}

fn code_link(items: &[PathBuf]) -> String {
    const BASE: &str = "claude://code/new";
    let folder = common_folder(items);
    let rel: Vec<PathBuf> = match &folder {
        Some(f) => items.iter().map(|p| p.strip_prefix(f).map(Path::to_path_buf).unwrap_or_else(|_| p.clone())).collect(),
        None => items.to_vec(),
    };
    let budget = MAX_LINK.saturating_sub(folder.as_ref().map(|f| enc(&path_str(f)).len()).unwrap_or(0) + 80) / 3;
    let q = if rel.iter().all(|p| p.as_os_str().is_empty()) { String::new() } else { format!("Files: {}\n\n", names(&rel, budget)) };
    query(BASE, &[("folder", folder.map(|f| path_str(&f)).unwrap_or_default()), ("q", q)])
}

/// Opens a claude:// link with the system (Claude Desktop handles it).
pub fn open(url: &str) -> Result<(), String> {
    if !url.starts_with("claude://") {
        return Err("Not a Claude link".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("rundll32.exe")
            .args(["url.dll,FileProtocolHandler", url])
            .creation_flags(0x0800_0000)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        let program = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
        std::process::Command::new(program).arg(url).spawn().map(|_| ()).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decodes a query value back (for checking the links).
    fn dec(s: &str) -> String {
        let b = s.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'%' && i + 2 < b.len() + 1 {
                out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap());
                i += 3;
            } else {
                out.push(b[i]);
                i += 1;
            }
        }
        String::from_utf8(out).unwrap()
    }

    fn params(url: &str) -> Vec<(String, String)> {
        url.split_once('?')
            .map(|(_, q)| q.split('&').map(|kv| {
                let (k, v) = kv.split_once('=').unwrap();
                (k.to_string(), dec(v))
            }).collect())
            .unwrap_or_default()
    }

    struct Tmp(PathBuf);
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn tmp(name: &str, files: &[&str]) -> (Tmp, Vec<PathBuf>) {
        let d = std::env::temp_dir().join(format!("convertino-ask-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let ps = files.iter().map(|f| {
            let p = d.join(f);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&p, b"x").unwrap();
            p
        }).collect();
        (Tmp(d), ps)
    }

    #[test]
    fn modes_follow_what_is_connected() {
        assert!(Connected::default().modes().is_empty());
        assert_eq!(Connected { desktop: true, code: false }.modes(), vec![Mode::Cowork, Mode::Chat]);
        assert_eq!(Connected { desktop: false, code: true }.modes(), vec![Mode::Code]);
        assert_eq!(Connected { desktop: true, code: true }.modes(), vec![Mode::Cowork, Mode::Chat, Mode::Code]);
    }

    #[test]
    fn the_mode_in_settings_falls_back_to_one_that_works() {
        let both = [Mode::Cowork, Mode::Chat, Mode::Code];
        assert_eq!(pick_mode("chat", &both), Some(Mode::Chat));
        assert_eq!(pick_mode("cowork", &[Mode::Code]), Some(Mode::Code));
        assert_eq!(pick_mode("nonsense", &both), Some(Mode::Cowork));
        assert_eq!(pick_mode("cowork", &[]), None);
    }

    #[test]
    fn the_button_only_shows_when_claude_can_convert() {
        let mut s = crate::settings::Settings::default();
        let desk = Connected { desktop: true, code: false };
        assert!(for_wheel(&s, Connected::default()).is_none(), "nothing connected: no button");
        let w = for_wheel(&s, desk).unwrap();
        assert_eq!(w.default, "cowork");
        assert_eq!(w.modes.iter().map(|m| m.id).collect::<Vec<_>>(), vec!["cowork", "chat"]);
        s.ai_apps = false;
        assert!(for_wheel(&s, desk).is_none(), "AI apps turned off: no button");
        s.ai_apps = true;
        s.ask_claude = false;
        assert!(for_wheel(&s, desk).is_none(), "button turned off");
        s.ask_claude = true;
        s.ask_mode = "code".into();
        assert_eq!(for_wheel(&s, desk).unwrap().default, "cowork", "Code isn't connected");
        assert_eq!(for_wheel(&s, Connected { desktop: true, code: true }).unwrap().default, "code");
    }

    #[test]
    fn cowork_attaches_a_few_files_one_by_one() {
        let (_t, ps) = tmp("few", &["IMG 0411.CR3", "IMG_0412.CR3", "notes & ideas.txt"]);
        let url = link(Mode::Cowork, &ps);
        assert!(url.starts_with("claude://cowork/new?"), "{url}");
        let p = params(&url);
        assert_eq!(p.len(), 3);
        assert!(p.iter().all(|(k, _)| k == "file"));
        assert_eq!(p[0].1, path_str(&ps[0]), "spaces and & come back exactly");
        assert_eq!(p[2].1, path_str(&ps[2]));
        assert!(!url.contains(' ') && !url[url.find('?').unwrap() + 1..].contains("&&"));
    }

    #[test]
    fn cowork_attaches_the_folder_for_many_files() {
        let names: Vec<String> = (1..=24).map(|i| format!("IMG_{i:04}.CR3")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let (t, ps) = tmp("many", &refs);
        let url = link(Mode::Cowork, &ps);
        let p = params(&url);
        assert_eq!(p[0], ("folder".to_string(), path_str(&t.0)));
        assert_eq!(p.iter().filter(|(k, _)| k == "folder").count(), 1);
        let q = &p.iter().find(|(k, _)| k == "q").unwrap().1;
        assert!(q.starts_with("The files I picked (24): IMG_0001.CR3, IMG_0002.CR3"), "{q}");
        assert!(q.contains("IMG_0024.CR3"), "all 24 fit: {q}");
    }

    #[test]
    fn cowork_attaches_a_selected_folder_as_a_folder() {
        let (t, _) = tmp("dir", &["Trip/a.jpg"]);
        let dir = t.0.join("Trip");
        let p = params(&link(Mode::Cowork, std::slice::from_ref(&dir)));
        assert_eq!(p, vec![("folder".to_string(), path_str(&dir))]);
    }

    #[test]
    fn chat_writes_the_paths() {
        let (_t, ps) = tmp("chat", &["a.pdf", "b c.pdf"]);
        let url = link(Mode::Chat, &ps);
        assert!(url.starts_with("claude://claude.ai/new?q="), "{url}");
        let q = &params(&url)[0].1;
        assert_eq!(q, &format!("{}\n{}\n\n", path_str(&ps[0]), path_str(&ps[1])));
    }

    #[test]
    fn links_stay_short_enough_to_open() {
        let names: Vec<String> = (1..=900).map(|i| format!("a-rather-long-photo-name-{i:04}.CR3")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let (_t, ps) = tmp("long", &refs);
        for mode in Mode::ALL {
            let url = link(mode, &ps);
            assert!(url.len() <= MAX_LINK, "{mode:?}: {} chars", url.len());
        }
        let q = params(&link(Mode::Chat, &ps))[0].1.clone();
        assert!(q.contains("more in "), "says how many are left out: {}", &q[q.len() - 120..]);
        let q = params(&link(Mode::Cowork, &ps)).into_iter().find(|(k, _)| k == "q").unwrap().1;
        assert!(q.starts_with("The files I picked (900):") && q.contains(" more"), "{}", &q[..80]);
    }

    #[test]
    fn code_opens_the_common_folder() {
        let (t, ps) = tmp("code", &["src/a.rs", "src/b/c.rs"]);
        let p = params(&link(Mode::Code, &ps));
        assert_eq!(p[0], ("folder".to_string(), path_str(&t.0.join("src"))));
        let q = &p[1].1;
        assert!(q.starts_with("Files: a.rs and "), "{q}");
    }

    #[test]
    fn only_claude_links_open() {
        assert!(open("https://example.com").is_err());
        assert!(open("file:///etc/passwd").is_err());
    }
}
