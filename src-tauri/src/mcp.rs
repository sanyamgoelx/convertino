//! MCP server: lets AI apps (Claude Desktop, Claude Code, Cursor, …) convert
//! files with Convertino.
//!
//! `convertino mcp` speaks the Model Context Protocol over stdio: one JSON-RPC
//! message per line on stdin and stdout (stdout carries nothing else; logs go
//! to stderr). The AI app starts it when it needs it; the Convertino app
//! doesn't have to be running.
//!
//! Each conversion runs on its own thread through engine.rs, exactly like a
//! wheel pick, so the same rules hold: results next to the originals (or
//! `outFolder`), originals never changed, nothing overwritten.
//!
//! While a job runs, its progress goes to `<config>/ai/live/<pid>.jsonl`;
//! the app (if it's running) tails those files and shows the corner card
//! "via Claude Desktop", with Cancel (a file in `<config>/ai/cancel/`), Open
//! folder and Undo. Finished jobs are added to `<config>/ai/jobs.jsonl` for
//! "Recent AI jobs" in Settings.

use crate::engine::{self, Event, Request, Sink};
use crate::{cli, settings, size};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Protocol versions this server speaks, newest first.
const VERSIONS: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

const INSTRUCTIONS: &str = "Convertino converts files on this computer: pictures (including camera RAW), audio, video, PDF, documents, spreadsheets/data and archives. \
Call list_conversions first to see what a file can become, then convert (or compress_to_size). \
Paths must be absolute. Results are saved next to the originals (or in outFolder); originals are never changed and existing files are never overwritten.";

// ---------- output ----------

type Out = Arc<Mutex<Box<dyn Write + Send>>>;

fn send(out: &Out, msg: &Value) {
    if let Ok(mut o) = out.lock() {
        let _ = writeln!(o, "{msg}");
        let _ = o.flush();
    }
}

fn reply(out: &Out, id: &Value, result: Value) {
    send(out, &json!({ "jsonrpc": "2.0", "id": id, "result": result }));
}

fn reply_error(out: &Out, id: &Value, code: i64, message: &str) {
    send(out, &json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }));
}

// ---------- the tools ----------

fn paths_schema() -> Value {
    json!({
        "type": "array",
        "items": { "type": "string" },
        "minItems": 1,
        "description": "Absolute paths of the files. ~ means the home folder. Wildcards like C:\\Photos\\*.CR3 work."
    })
}

pub fn tools() -> Value {
    json!([
        {
            "name": "list_conversions",
            "title": "What can these files become?",
            "description": "Lists, for each file, its kind and every format or action it can be converted to (the names to pass to convert as `to`).",
            "inputSchema": { "type": "object", "properties": { "paths": paths_schema() }, "required": ["paths"] },
            "annotations": { "readOnlyHint": true, "openWorldHint": false }
        },
        {
            "name": "convert",
            "title": "Convert files",
            "description": "Converts files to another format, or runs an action on them (compress, split, merge, extract, 720p, frames, camera-jpg, …). \
Pictures, RAW photos, audio, video, PDF, documents, data and archives. New files are saved next to the originals unless outFolder is given; originals are never changed and nothing is overwritten. \
Several files of the same kind are converted together (merge needs two or more PDFs).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "paths": paths_schema(),
                    "to": { "type": "string", "description": "A name from list_conversions, e.g. jpg, png, webp, pdf, mp3, mp4, docx, xlsx, json, zip, compress, split, merge, extract, 720p, camera-jpg." },
                    "quality": { "type": "string", "enum": ["small", "balanced", "best"], "description": "Optional. Default: as set in Convertino. balanced = looks the same as the original." },
                    "outFolder": { "type": "string", "description": "Optional absolute folder to save into instead of next to the originals." }
                },
                "required": ["paths", "to"]
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": false }
        },
        {
            "name": "compress_to_size",
            "title": "Compress to a file size",
            "description": "Makes pictures, RAW photos, videos, PDFs or audio fit a file size (for an upload or email limit). Convertino picks quality, resolution and frame rate. \
Files already under the size are left alone (except RAW photos, which always become a JPG).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "paths": paths_schema(),
                    "size": { "type": "string", "description": "Largest size, decimal units: \"25MB\", \"800KB\", \"1.5GB\" (10MB = 10,000,000 bytes)." },
                    "together": { "type": "boolean", "description": "With several files: the size is for all of them together. Default false (each file)." },
                    "outFolder": { "type": "string", "description": "Optional absolute folder to save into." }
                },
                "required": ["paths", "size"]
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": false }
        },
        {
            "name": "converters_status",
            "title": "Which converters are ready",
            "description": "Shows which converter programs (FFmpeg, ImageMagick, LibreOffice, …) are ready. Missing ones download by themselves the first time they're needed.",
            "inputSchema": { "type": "object", "properties": {} },
            "annotations": { "readOnlyHint": true, "openWorldHint": false }
        }
    ])
}

/// A tool's answer: text for the model, the same as JSON, and whether it failed.
pub struct ToolResult {
    pub text: String,
    pub data: Value,
    pub error: bool,
}

impl ToolResult {
    fn fail(text: impl Into<String>) -> ToolResult {
        let text = text.into();
        ToolResult { data: json!({ "ok": false, "error": text }), text, error: true }
    }
    fn to_value(&self) -> Value {
        json!({ "content": [{ "type": "text", "text": self.text }], "structuredContent": self.data, "isError": self.error })
    }
}

fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// Absolute, existing files from what the AI app sent.
pub fn files_from(args: &Value) -> Result<Vec<PathBuf>, String> {
    let list: Vec<String> = match args.get("paths") {
        Some(Value::Array(a)) => a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
        Some(Value::String(s)) => vec![s.clone()],
        _ => return Err("`paths` is missing: give the absolute paths of the files.".into()),
    };
    if list.is_empty() {
        return Err("`paths` is empty.".into());
    }
    let mut out = Vec::new();
    for raw in list {
        let p = match raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\")) {
            Some(rest) => home().map(|h| h.join(rest)).unwrap_or_else(|| PathBuf::from(&raw)),
            None if raw == "~" => home().unwrap_or_else(|| PathBuf::from(&raw)),
            None => PathBuf::from(&raw),
        };
        if !p.is_absolute() {
            return Err(format!("\"{raw}\" isn't an absolute path. Give the full path, like {}", if cfg!(windows) { r"C:\Users\you\Pictures\photo.jpg" } else { "/Users/you/Pictures/photo.jpg" }));
        }
        let found = cli::expand(&[p.to_string_lossy().into_owned()]).map_err(|e| e.lines().next().unwrap_or("").to_string())?;
        out.extend(found);
    }
    Ok(out)
}

fn out_folder(args: &Value) -> Result<Option<PathBuf>, String> {
    match args.get("outFolder").and_then(Value::as_str).filter(|s| !s.trim().is_empty()) {
        None => Ok(None),
        Some(s) => {
            let p = PathBuf::from(s);
            if !p.is_absolute() {
                return Err(format!("outFolder \"{s}\" must be an absolute path."));
            }
            std::fs::create_dir_all(&p).map_err(|e| format!("Couldn't create {s}: {e}"))?;
            Ok(Some(p))
        }
    }
}

fn list_conversions(args: &Value) -> ToolResult {
    let files = match files_from(args) {
        Ok(f) => f,
        Err(e) => return ToolResult::fail(e),
    };
    let list = cli::formats_of(&files);
    let mut text = String::new();
    for f in &list {
        let name = engine::file_name(&f.path);
        match &f.family {
            None => text.push_str(&format!("{name}: Convertino can't convert this kind of file.\n")),
            Some(fam) => {
                let names: Vec<String> = f.conversions.iter().map(|c| if c.multi { format!("{} (2+ files)", c.name) } else { c.name.clone() }).collect();
                text.push_str(&format!("{name} ({}, {}): {}\n", fam.to_lowercase(), engine::human_size(f.bytes), names.join(", ")));
            }
        }
    }
    ToolResult { text: text.trim_end().to_string(), data: json!({ "files": list }), error: false }
}

/// A finished conversion as text and JSON.
fn summary(out: &engine::Outcome, skipped: &[(PathBuf, String)], place: &str) -> ToolResult {
    let mut text = String::new();
    if !out.made.is_empty() {
        text.push_str(&format!("Made {} file{} ({place}):\n", out.made.len(), if out.made.len() == 1 { "" } else { "s" }));
        for (inputs, p) in &out.made_from {
            let from: u64 = inputs.iter().map(|i| engine::disk_size(i)).sum();
            text.push_str(&format!("- {} ({}, from {})\n", p.display(), engine::human_size(engine::disk_size(p)), engine::human_size(from)));
        }
    }
    let failed: Vec<(PathBuf, String)> = skipped.iter().cloned().chain(out.failed.iter().cloned()).collect();
    if !failed.is_empty() {
        text.push_str("Failed:\n");
        for (p, e) in &failed {
            text.push_str(&format!("- {}: {e}\n", p.display()));
        }
    }
    for n in &out.notes {
        text.push_str(&format!("Note: {n}\n"));
    }
    if out.cancelled {
        text.push_str("Cancelled before it finished.\n");
    }
    if text.is_empty() {
        text = "Nothing to do.".into();
    }
    let data = json!({
        "ok": failed.is_empty() && !out.cancelled,
        "cancelled": out.cancelled,
        "outputs": out.made_from.iter().map(|(i, p)| json!({ "path": p, "sources": i, "bytes": engine::disk_size(p) })).collect::<Vec<_>>(),
        "failed": failed.iter().map(|(p, e)| json!({ "source": p, "error": e })).collect::<Vec<_>>(),
        "notes": out.notes,
    });
    ToolResult { text: text.trim_end().to_string(), data, error: out.made.is_empty() && (!failed.is_empty() || out.cancelled) }
}

/// What a running tool call reports.
pub struct Ctx {
    /// Job ids of this call, for cancelling.
    pub jobs: Arc<Mutex<Vec<u64>>>,
    /// Progress 0–1 with a short text.
    pub progress: Arc<dyn Fn(f64, &str) + Send + Sync>,
    /// Who asked (for the corner card and the job log).
    pub client: String,
}

fn convert_tool(args: &Value, ctx: &Ctx, size: Option<size::Ask>) -> ToolResult {
    if !settings::get().ai_apps {
        return ToolResult::fail("AI apps are turned off in Convertino Settings (AI & CLI). Ask the person to turn them on.");
    }
    let files = match files_from(args) {
        Ok(f) => f,
        Err(e) => return ToolResult::fail(e),
    };
    let out_dir = match out_folder(args) {
        Ok(o) => o,
        Err(e) => return ToolResult::fail(e),
    };
    let to = if size.is_some() { "compress".to_string() } else { args.get("to").and_then(Value::as_str).unwrap_or("").trim().to_string() };
    if to.is_empty() {
        return ToolResult::fail("`to` is missing. Call list_conversions to see the names, e.g. jpg, mp3, pdf.");
    }
    let quality = match args.get("quality").and_then(Value::as_str) {
        None => None,
        Some(q @ ("small" | "balanced" | "best")) => Some(q.to_string()),
        Some(q) => return ToolResult::fail(format!("quality is small, balanced or best, not \"{q}\".")),
    };
    let (groups, skipped) = cli::group(&files, &to);
    if groups.is_empty() {
        let why: Vec<String> = skipped.iter().map(|(p, e)| format!("{}: {e}", engine::file_name(p))).collect();
        return ToolResult::fail(why.join("\n"));
    }
    let mut all = engine::Outcome::default();
    let total = groups.len() as f64;
    for (n, g) in groups.iter().enumerate() {
        let id = engine::new_id();
        if let Ok(mut j) = ctx.jobs.lock() {
            j.push(id);
        }
        let title = match g.files.as_slice() {
            [one] => format!("{} → {}", engine::file_name(one), g.label),
            many => format!("{} files → {}", many.len(), g.label),
        };
        let live = Live::start(id, &ctx.client, &title);
        let progress = ctx.progress.clone();
        let l2 = live.clone();
        let sink: Sink = Arc::new(move |e| match e {
            Event::Progress { fraction, detail } => {
                progress((n as f64 + fraction) / total, &detail);
                l2.progress(fraction, &detail);
            }
            Event::Download { fraction, detail } => {
                progress((n as f64) / total, &detail);
                l2.progress(fraction, &detail);
            }
            Event::StepDone { .. } => {}
        });
        let cancel_watch = live.watch_cancel(id);
        let req = Request {
            target_id: g.target_id.clone(),
            files: g.files.clone(),
            quality: cli::quality_for(quality.as_deref()),
            size,
            out_dir: out_dir.clone(),
        };
        let result = engine::convert(id, &req, &sink);
        drop(cancel_watch);
        match result {
            Ok(o) => {
                live.done(&o, &title);
                all.made.extend(o.made);
                all.made_from.extend(o.made_from);
                all.failed.extend(o.failed);
                all.notes.extend(o.notes);
                all.cancelled |= o.cancelled;
            }
            Err(e) => {
                live.failed(&e, &title);
                all.failed.extend(g.files.iter().map(|f| (f.clone(), e.clone())));
            }
        }
        if all.cancelled {
            break;
        }
    }
    let place = match &out_dir {
        Some(d) => format!("saved in {}", d.display()),
        None => "saved next to the originals".into(),
    };
    let result = summary(&all, &skipped, &place);
    log_job(&ctx.client, &to, &files, &all, &result);
    result
}

fn converters_status() -> ToolResult {
    let list = crate::install::status();
    let text = list
        .iter()
        .map(|t| format!("{}: {}", t.name, match t.state { "ready" | "system" => "ready".to_string(), "downloading" => "downloading".into(), _ => format!("downloads when first needed (about {} MB)", t.download_mb) }))
        .collect::<Vec<_>>()
        .join("\n");
    ToolResult { text, data: json!({ "converters": list }), error: false }
}

/// Runs one tool.
pub fn call(name: &str, args: &Value, ctx: &Ctx) -> ToolResult {
    match name {
        "list_conversions" => list_conversions(args),
        "convert" => convert_tool(args, ctx, None),
        "compress_to_size" => {
            let size = match args.get("size") {
                Some(Value::Number(n)) => n.as_u64(),
                Some(Value::String(s)) => cli::parse_size(s),
                _ => None,
            };
            let Some(bytes) = size else {
                return ToolResult::fail("`size` should look like \"25MB\", \"800KB\" or \"1.5GB\".");
            };
            let together = args.get("together").and_then(Value::as_bool).unwrap_or(false);
            convert_tool(args, ctx, Some(size::Ask { bytes, together, trim: None }))
        }
        "converters_status" => converters_status(),
        other => ToolResult::fail(format!("Unknown tool \"{other}\".")),
    }
}

// ---------- the app's corner card and job log ----------

/// `<config>/ai`, where the live progress, cancel requests and job log go.
pub fn ai_dir() -> Option<PathBuf> {
    settings::config_dir().or_else(settings::default_config_dir).map(|d| d.join("ai"))
}

fn now_ms() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

/// Ids the app shows: unique across MCP processes and apart from the app's own jobs.
pub fn card_id(local: u64) -> u64 {
    (1u64 << 40) + ((std::process::id() as u64 & 0xFFFFF) << 20) + (local & 0xFFFFF)
}

/// Progress lines for the app's corner card.
#[derive(Clone)]
struct Live {
    file: Option<PathBuf>,
    card: u64,
    last: Arc<Mutex<Instant>>,
}

impl Live {
    fn start(local: u64, client: &str, title: &str) -> Live {
        let file = ai_dir().map(|d| d.join("live").join(format!("{}.jsonl", std::process::id())));
        let live = Live { file, card: card_id(local), last: Arc::new(Mutex::new(Instant::now() - Duration::from_secs(1))) };
        live.write(json!({ "type": "started", "id": live.card, "client": client, "title": title, "at": now_ms() as u64 }));
        live
    }

    fn write(&self, v: Value) {
        let Some(f) = &self.file else { return };
        if let Some(dir) = f.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(f) {
            let _ = writeln!(file, "{v}");
        }
    }

    fn progress(&self, fraction: f64, detail: &str) {
        let Ok(mut last) = self.last.lock() else { return };
        if last.elapsed() < Duration::from_millis(250) && fraction < 1.0 {
            return;
        }
        *last = Instant::now();
        self.write(json!({ "type": "progress", "id": self.card, "fraction": fraction, "detail": detail }));
    }

    fn done(&self, o: &engine::Outcome, title: &str) {
        let body = match o.made.as_slice() {
            [] => o.failed.first().map(|(_, e)| e.clone()).or_else(|| o.notes.first().cloned()).unwrap_or_else(|| "Nothing to do.".into()),
            [one] => format!("{} · {}", engine::file_name(one), engine::human_size(engine::disk_size(one))),
            many => format!("{} files · {}", many.len(), engine::human_size(many.iter().map(|p| engine::disk_size(p)).sum())),
        };
        let ok = !o.made.is_empty() || (o.failed.is_empty() && !o.cancelled);
        let head = if o.cancelled { "Cancelled".to_string() } else if o.made.is_empty() && !o.failed.is_empty() { format!("Couldn't convert: {title}") } else { title.to_string() };
        self.write(json!({ "type": "done", "id": self.card, "ok": ok, "title": head, "body": body, "outputs": o.made }));
    }

    fn failed(&self, e: &str, title: &str) {
        self.write(json!({ "type": "done", "id": self.card, "ok": false, "title": format!("Couldn't convert: {title}"), "body": e, "outputs": [] }));
    }

    /// Cancel from the card: the app creates `<ai>/cancel/<card id>`.
    fn watch_cancel(&self, local: u64) -> CancelWatch {
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = ai_dir().map(|d| d.join("cancel").join(self.card.to_string()));
        let s = stop.clone();
        std::thread::spawn(move || {
            while !s.load(Ordering::SeqCst) {
                if flag.as_ref().is_some_and(|f| f.exists()) {
                    if let Some(f) = &flag {
                        let _ = std::fs::remove_file(f);
                    }
                    crate::procs::cancel(local);
                    break;
                }
                std::thread::sleep(Duration::from_millis(300));
            }
        });
        CancelWatch(stop)
    }
}

struct CancelWatch(Arc<std::sync::atomic::AtomicBool>);
impl Drop for CancelWatch {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// Keeps the last 200 AI jobs for Settings.
fn log_job(client: &str, to: &str, files: &[PathBuf], o: &engine::Outcome, r: &ToolResult) {
    let Some(dir) = ai_dir() else { return };
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("jobs.jsonl");
    let what = match files {
        [one] => format!("{} → {}", engine::file_name(one), to),
        many => format!("{} files → {}", many.len(), to),
    };
    let state = if o.cancelled {
        "Cancelled".to_string()
    } else if !o.made.is_empty() && o.failed.is_empty() {
        "Done".into()
    } else if !o.made.is_empty() {
        format!("{} failed", o.failed.len())
    } else {
        o.failed.first().map(|(_, e)| e.clone()).unwrap_or_else(|| "Nothing to do".into())
    };
    let folder = o.made.first().and_then(|p| p.parent()).map(|p| p.to_path_buf());
    let line = json!({ "at": now_ms() as u64, "client": client, "what": what, "ok": !r.error, "state": state, "outputs": o.made, "folder": folder });
    let _lock = LOG_LOCK.lock();
    let mut lines: Vec<String> = std::fs::read_to_string(&path).map(|t| t.lines().map(str::to_string).collect()).unwrap_or_default();
    lines.push(line.to_string());
    if lines.len() > 250 {
        lines.drain(..lines.len() - 200);
    }
    let tmp = path.with_extension("jsonl.tmp");
    if std::fs::write(&tmp, lines.join("\n") + "\n").is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

static LOG_LOCK: Mutex<()> = Mutex::new(());

/// The AI app's name for people ("Claude Desktop").
pub fn friendly_client(name: &str, title: Option<&str>) -> String {
    let n = name.to_lowercase();
    if n.contains("claude-code") || n == "claude code" {
        "Claude Code".into()
    } else if n.contains("claude") {
        "Claude Desktop".into()
    } else if n.contains("cursor") {
        "Cursor".into()
    } else if n.contains("vscode") || n.contains("visual studio code") {
        "VS Code".into()
    } else if let Some(t) = title.filter(|t| !t.is_empty()) {
        t.to_string()
    } else if name.is_empty() {
        "An AI app".into()
    } else {
        name.to_string()
    }
}

// ---------- the server loop ----------

struct Server {
    out: Out,
    client: Mutex<String>,
    /// Running calls: request id (as JSON text) → job ids.
    running: Mutex<HashMap<String, Arc<Mutex<Vec<u64>>>>>,
    threads: AtomicU64,
}

impl Server {
    fn handle(self: &Arc<Self>, msg: Value) {
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        match (method, id) {
            ("initialize", Some(id)) => {
                let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or(VERSIONS[0]);
                let version = if VERSIONS.contains(&asked) { asked } else { VERSIONS[0] };
                let info = params.get("clientInfo");
                let name = info.and_then(|i| i.get("name")).and_then(Value::as_str).unwrap_or("");
                let title = info.and_then(|i| i.get("title")).and_then(Value::as_str);
                if let Ok(mut c) = self.client.lock() {
                    *c = friendly_client(name, title);
                }
                reply(
                    &self.out,
                    &id,
                    json!({
                        "protocolVersion": version,
                        "capabilities": { "tools": { "listChanged": false } },
                        "serverInfo": { "name": "convertino", "title": "Convertino", "version": env!("CARGO_PKG_VERSION") },
                        "instructions": INSTRUCTIONS,
                    }),
                );
            }
            ("ping", Some(id)) => reply(&self.out, &id, json!({})),
            ("tools/list", Some(id)) => reply(&self.out, &id, json!({ "tools": tools() })),
            ("tools/call", Some(id)) => {
                let name = params.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
                let token = params.get("_meta").and_then(|m| m.get("progressToken")).cloned();
                let jobs = Arc::new(Mutex::new(Vec::new()));
                let key = id.to_string();
                if let Ok(mut r) = self.running.lock() {
                    r.insert(key.clone(), jobs.clone());
                }
                let me = self.clone();
                me.threads.fetch_add(1, Ordering::SeqCst);
                std::thread::spawn(move || {
                    let out = me.out.clone();
                    let last = Mutex::new(-1.0f64);
                    let progress: Arc<dyn Fn(f64, &str) + Send + Sync> = Arc::new(move |p: f64, detail: &str| {
                        let Some(tok) = &token else { return };
                        let pct = (p.clamp(0.0, 1.0) * 100.0).round();
                        // Progress must only go up.
                        let Ok(mut l) = last.lock() else { return };
                        if pct <= *l {
                            return;
                        }
                        *l = pct;
                        send(&out, &json!({ "jsonrpc": "2.0", "method": "notifications/progress", "params": { "progressToken": tok, "progress": pct, "total": 100, "message": detail } }));
                    });
                    let client = me.client.lock().map(|c| c.clone()).unwrap_or_default();
                    let ctx = Ctx { jobs, progress, client };
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| call(&name, &args, &ctx)))
                        .unwrap_or_else(|_| ToolResult::fail("Convertino ran into a problem with this file."));
                    let still_wanted = me.running.lock().map(|mut r| r.remove(&key).is_some()).unwrap_or(true);
                    // A cancelled request gets no answer (the AI app stopped waiting).
                    if still_wanted {
                        reply(&me.out, &id, result.to_value());
                    }
                    me.threads.fetch_sub(1, Ordering::SeqCst);
                });
            }
            ("notifications/cancelled", None) => {
                let rid = params.get("requestId").map(|v| v.to_string()).unwrap_or_default();
                if let Some(jobs) = self.running.lock().ok().and_then(|mut r| r.remove(&rid)) {
                    for j in jobs.lock().map(|j| j.clone()).unwrap_or_default() {
                        crate::procs::cancel(j);
                    }
                }
            }
            (m, Some(id)) if !m.is_empty() => reply_error(&self.out, &id, -32601, &format!("Method not found: {m}")),
            (_, Some(id)) if msg.get("result").is_none() && msg.get("error").is_none() => reply_error(&self.out, &id, -32600, "Invalid request"),
            _ => {} // other notifications, and answers to requests we never send
        }
    }
}

/// Serves over `input`/`output` until the input ends. Returns the exit code.
pub fn serve_on(input: impl BufRead, output: Box<dyn Write + Send>) -> i32 {
    let server = Arc::new(Server { out: Arc::new(Mutex::new(output)), client: Mutex::new("An AI app".into()), running: Mutex::new(HashMap::new()), threads: AtomicU64::new(0) });
    for line in input.lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(line) {
            Ok(Value::Array(batch)) => batch.into_iter().for_each(|m| server.handle(m)),
            Ok(m @ Value::Object(_)) => server.handle(m),
            _ => send(&server.out, &json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": "Parse error" } })),
        }
    }
    // The AI app closed the connection: stop what's still running.
    let jobs: Vec<u64> = server.running.lock().map(|r| r.values().flat_map(|j| j.lock().map(|j| j.clone()).unwrap_or_default()).collect()).unwrap_or_default();
    for j in jobs {
        crate::procs::cancel(j);
    }
    let until = Instant::now() + Duration::from_secs(5);
    while server.threads.load(Ordering::SeqCst) > 0 && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
    if let Some(f) = ai_dir().map(|d| d.join("live").join(format!("{}.jsonl", std::process::id()))) {
        let _ = std::fs::remove_file(f);
    }
    0
}

/// `convertino mcp`: serves on stdin/stdout.
pub fn serve() -> i32 {
    log::info!("mcp: started");
    serve_on(std::io::stdin().lock(), Box::new(std::io::stdout()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_list_is_stable() {
        // Changing a tool's name or inputs breaks AI apps that learned them: update on purpose.
        let t = tools();
        let names: Vec<&str> = t.as_array().unwrap().iter().map(|x| x["name"].as_str().unwrap()).collect();
        assert_eq!(names, vec!["list_conversions", "convert", "compress_to_size", "converters_status"]);
        let required = |n: usize| t[n]["inputSchema"]["required"].as_array().map(|a| a.iter().map(|v| v.as_str().unwrap().to_string()).collect::<Vec<_>>()).unwrap_or_default();
        assert_eq!(required(0), vec!["paths"]);
        assert_eq!(required(1), vec!["paths", "to"]);
        assert_eq!(required(2), vec!["paths", "size"]);
        for x in t.as_array().unwrap() {
            assert_eq!(x["inputSchema"]["type"], "object");
            assert!(x["description"].as_str().unwrap().len() > 40);
            assert!(x["annotations"]["readOnlyHint"].is_boolean());
        }
    }

    #[test]
    fn paths_must_be_absolute_and_exist() {
        assert!(files_from(&json!({ "paths": ["photo.jpg"] })).unwrap_err().contains("absolute"));
        assert!(files_from(&json!({})).unwrap_err().contains("missing"));
        assert!(files_from(&json!({ "paths": [] })).unwrap_err().contains("empty"));
        let missing = std::env::temp_dir().join("convertino-not-here.png");
        assert!(files_from(&json!({ "paths": [missing] })).unwrap_err().contains("not found"));
        let real = std::env::temp_dir().join(format!("convertino-mcp-{}.png", std::process::id()));
        std::fs::write(&real, b"x").unwrap();
        assert_eq!(files_from(&json!({ "paths": real })).unwrap().len(), 1, "a single string works too");
        let _ = std::fs::remove_file(real);
    }

    #[test]
    fn client_names() {
        assert_eq!(friendly_client("claude-ai", None), "Claude Desktop");
        assert_eq!(friendly_client("claude-code", None), "Claude Code");
        assert_eq!(friendly_client("cursor-vscode", None), "Cursor");
        assert_eq!(friendly_client("zed", Some("Zed")), "Zed");
        assert_eq!(friendly_client("", None), "An AI app");
    }

    #[test]
    fn card_ids_stay_apart_from_the_apps_jobs() {
        assert!(card_id(1) > 1u64 << 40);
        assert!(card_id(1) < 1u64 << 53, "JavaScript numbers hold it exactly");
        assert_ne!(card_id(1), card_id(2));
    }

    /// The protocol, end to end through serve_on.
    #[test]
    fn handshake_list_call_and_errors() {
        let input = [
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "claude-code", "version": "1" } } }),
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
            json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "list_conversions", "arguments": { "paths": ["relative.png"] } } }),
            json!({ "jsonrpc": "2.0", "id": 4, "method": "nope" }),
            json!({ "jsonrpc": "2.0", "id": 5, "method": "ping" }),
        ]
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join("\n")
            + "\nnot json\n";
        let buf = Arc::new(Mutex::new(Vec::new()));
        struct W(Arc<Mutex<Vec<u8>>>);
        impl Write for W {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(b);
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        serve_on(std::io::Cursor::new(input), Box::new(W(buf.clone())));
        let text = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        let msgs: Vec<Value> = text.lines().map(|l| serde_json::from_str(l).expect("every line is JSON")).collect();
        let by_id = |id: i64| msgs.iter().find(|m| m["id"] == id).unwrap_or_else(|| panic!("no answer to {id}: {text}"));
        assert_eq!(by_id(1)["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(by_id(1)["result"]["serverInfo"]["name"], "convertino");
        assert_eq!(by_id(2)["result"]["tools"].as_array().unwrap().len(), 4);
        assert_eq!(by_id(3)["result"]["isError"], true);
        assert!(by_id(3)["result"]["content"][0]["text"].as_str().unwrap().contains("absolute"));
        assert_eq!(by_id(4)["error"]["code"], -32601);
        assert_eq!(by_id(5)["result"], json!({}));
        assert!(msgs.iter().any(|m| m["error"]["code"] == -32700), "bad JSON gets a parse error");
        assert!(!msgs.iter().any(|m| m.get("method").is_some_and(|x| x == "notifications/initialized")));
    }

    #[test]
    fn unknown_protocol_version_gets_ours() {
        let input = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "1999-01-01" } }).to_string() + "\n";
        let buf = Arc::new(Mutex::new(Vec::new()));
        struct W(Arc<Mutex<Vec<u8>>>);
        impl Write for W {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(b);
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        serve_on(std::io::Cursor::new(input), Box::new(W(buf.clone())));
        let v: Value = serde_json::from_slice(buf.lock().unwrap().split(|b| *b == b'\n').next().unwrap()).unwrap();
        assert_eq!(v["result"]["protocolVersion"], VERSIONS[0]);
    }
}
