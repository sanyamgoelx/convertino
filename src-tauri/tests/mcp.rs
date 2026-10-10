//! End-to-end tests of `convertino mcp`: the real program, spoken to the way
//! an AI app does (JSON-RPC lines over stdin/stdout).

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_convertino-cli");
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

struct Mcp {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Value>,
    seen: Vec<Value>,
    dir: PathBuf,
    config: PathBuf,
}

impl Mcp {
    fn start(name: &str, settings: Option<Value>) -> Mcp {
        let dir = std::env::temp_dir().join(format!("convertino-mcp-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let config = dir.join("_config");
        std::fs::create_dir_all(&config).unwrap();
        if let Some(s) = settings {
            std::fs::write(config.join("settings.json"), s.to_string()).unwrap();
        }
        let mut child = Command::new(BIN)
            .arg("mcp")
            .env("CONVERTINO_CONFIG_DIR", &config)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let v: Value = serde_json::from_str(&line).unwrap_or_else(|e| panic!("stdout must be JSON only ({e}): {line}"));
                if tx.send(v).is_err() {
                    break;
                }
            }
        });
        let mut m = Mcp { child, stdin, rx, seen: Vec::new(), dir, config };
        let init = m.request(1, "initialize", json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "claude-ai", "version": "1.0" } }));
        assert_eq!(init["result"]["serverInfo"]["name"], "convertino");
        m.notify("notifications/initialized", json!({}));
        m
    }

    fn send(&mut self, v: Value) {
        writeln!(self.stdin, "{v}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    /// Sends a request and waits for its answer (other messages are kept in `seen`).
    fn request(&mut self, id: i64, method: &str, params: Value) -> Value {
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        self.wait(id, Duration::from_secs(300))
    }

    fn wait(&mut self, id: i64, limit: Duration) -> Value {
        if let Some(i) = self.seen.iter().position(|m| m["id"] == id) {
            return self.seen.remove(i);
        }
        let until = Instant::now() + limit;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            let m = self.rx.recv_timeout(left).unwrap_or_else(|_| panic!("no answer to request {id}"));
            if m["id"] == id {
                return m;
            }
            self.seen.push(m);
        }
    }

    fn call(&mut self, id: i64, tool: &str, args: Value) -> Value {
        let r = self.request(id, "tools/call", json!({ "name": tool, "arguments": args, "_meta": { "progressToken": format!("p{id}") } }));
        r["result"].clone()
    }

    fn file(&self, fixture: &str, as_name: &str) -> PathBuf {
        let to = self.dir.join(as_name);
        std::fs::copy(Path::new(FIXTURES).join(fixture), &to).unwrap();
        to
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn lists_tools_and_converts() {
    let mut m = Mcp::start("convert", None);
    let tools = m.request(2, "tools/list", json!({}));
    let names: Vec<&str> = tools["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["list_conversions", "convert", "compress_to_size", "converters_status"]);

    let png = m.file("gradient.png", "a photo.png");
    let r = m.call(3, "list_conversions", json!({ "paths": [png] }));
    assert_eq!(r["isError"], false);
    assert!(r["content"][0]["text"].as_str().unwrap().contains("jpg"));

    let r = m.call(4, "convert", json!({ "paths": [png], "to": "jpg" }));
    assert_eq!(r["isError"], false, "{r}");
    let out = PathBuf::from(r["structuredContent"]["outputs"][0]["path"].as_str().unwrap());
    assert_eq!(out.file_name().unwrap(), "a photo.jpg");
    assert!(out.is_file());
    assert!(r["content"][0]["text"].as_str().unwrap().contains("a photo.jpg"));

    // Progress notifications for that call only went up.
    let ps: Vec<f64> = m.seen.iter().filter(|x| x["method"] == "notifications/progress" && x["params"]["progressToken"] == "p4").map(|x| x["params"]["progress"].as_f64().unwrap()).collect();
    assert!(ps.windows(2).all(|w| w[1] > w[0]), "{ps:?}");

    // The job is in the log Settings shows, with who asked.
    let log = std::fs::read_to_string(m.config.join("ai").join("jobs.jsonl")).unwrap();
    let last: Value = serde_json::from_str(log.lines().last().unwrap()).unwrap();
    assert_eq!(last["client"], "Claude Desktop");
    assert_eq!(last["ok"], true);
    // And the corner card heard about it.
    let live = std::fs::read_dir(m.config.join("ai").join("live")).unwrap().flatten().next().unwrap().path();
    let events = std::fs::read_to_string(live).unwrap();
    assert!(events.contains("\"started\"") && events.contains("\"done\"") && events.contains("Claude Desktop"));
}

#[test]
fn mistakes_are_tool_errors_not_crashes() {
    let mut m = Mcp::start("mistakes", None);
    let png = m.file("gradient.png", "a.png");
    let r = m.call(2, "convert", json!({ "paths": ["a.png"], "to": "jpg" }));
    assert_eq!(r["isError"], true);
    assert!(r["content"][0]["text"].as_str().unwrap().contains("absolute"));
    let r = m.call(3, "convert", json!({ "paths": [png], "to": "docx" }));
    assert_eq!(r["isError"], true);
    assert!(r["content"][0]["text"].as_str().unwrap().contains("can become"));
    let r = m.call(4, "compress_to_size", json!({ "paths": [png], "size": "huge" }));
    assert_eq!(r["isError"], true);
    let r = m.call(5, "frobnicate", json!({}));
    assert_eq!(r["isError"], true);
    // Preset changes: a typo is a tool error, a number is the look score.
    let r = m.call(7, "convert", json!({ "paths": [png], "to": "jpg", "tune": { "encoder": "quantum" } }));
    assert_eq!(r["isError"], true, "{r}");
    assert!(r["content"][0]["text"].as_str().unwrap().contains("encoder"));
    let r = m.call(8, "convert", json!({ "paths": [png], "to": "jpg", "quality": 75, "tune": { "stripGps": true, "floor": 50 } }));
    assert_eq!(r["isError"], false, "{r}");
    // Still alive.
    assert_eq!(m.request(6, "ping", json!({}))["result"], json!({}));
}

#[test]
fn turned_off_in_settings() {
    let mut m = Mcp::start("off", Some(json!({ "version": 2, "aiApps": false })));
    let png = m.file("gradient.png", "a.png");
    let r = m.call(2, "convert", json!({ "paths": [png], "to": "jpg" }));
    assert_eq!(r["isError"], true);
    assert!(r["content"][0]["text"].as_str().unwrap().contains("turned off"));
    assert!(!m.dir.join("a.jpg").exists());
    // Looking is still fine.
    assert_eq!(m.call(3, "list_conversions", json!({ "paths": [png] }))["isError"], false);
}

#[test]
fn two_calls_at_once() {
    let mut m = Mcp::start("parallel", None);
    let a = m.file("gradient.png", "a.png");
    let b = m.file("people.csv", "b.csv");
    m.send(json!({ "jsonrpc": "2.0", "id": 10, "method": "tools/call", "params": { "name": "convert", "arguments": { "paths": [a], "to": "webp" } } }));
    m.send(json!({ "jsonrpc": "2.0", "id": 11, "method": "tools/call", "params": { "name": "convert", "arguments": { "paths": [b], "to": "xlsx" } } }));
    let r11 = m.wait(11, Duration::from_secs(120));
    let r10 = m.wait(10, Duration::from_secs(120));
    assert_eq!(r10["result"]["isError"], false, "{r10}");
    assert_eq!(r11["result"]["isError"], false, "{r11}");
    assert!(m.dir.join("a.webp").is_file() && m.dir.join("b.xlsx").is_file());
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

#[test]
fn cancelling_stops_the_job_and_the_server_carries_on() {
    let Some(ff) = ffmpeg() else {
        eprintln!("no ffmpeg: skipped");
        return;
    };
    let mut m = Mcp::start("cancel", None);
    let video = m.dir.join("long.mp4");
    assert!(Command::new(&ff)
        .args(["-v", "error", "-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=30", "-t", "40", "-c:v", "libx264", "-preset", "ultrafast"])
        .arg(&video)
        .status()
        .unwrap()
        .success());
    m.send(json!({ "jsonrpc": "2.0", "id": 20, "method": "tools/call", "params": { "name": "convert", "arguments": { "paths": [video], "to": "webm" } } }));
    std::thread::sleep(Duration::from_secs(4));
    m.notify("notifications/cancelled", json!({ "requestId": 20, "reason": "test" }));
    std::thread::sleep(Duration::from_secs(2));
    // Still answering, and nothing half-made is left.
    assert_eq!(m.request(21, "ping", json!({}))["result"], json!({}));
    let until = Instant::now() + Duration::from_secs(20);
    while m.dir.join("long.webm").exists() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(!m.dir.join("long.webm").exists());
    #[cfg(unix)]
    {
        let left = Command::new("pgrep").args(["-f", &m.dir.to_string_lossy()]).output().unwrap();
        assert!(String::from_utf8_lossy(&left.stdout).trim().is_empty(), "converter still running");
    }
}

#[test]
fn closing_the_connection_ends_the_server() {
    let mut m = Mcp::start("close", None);
    drop(std::mem::replace(&mut m.stdin, {
        // Swap in a dummy pipe so Drop still has something; the real stdin closes here.
        let mut dummy = Command::new(BIN).arg("--version").stdin(Stdio::piped()).stdout(Stdio::null()).spawn().unwrap();
        let s = dummy.stdin.take().unwrap();
        let _ = dummy.wait();
        s
    }));
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = m.child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < until, "server kept running after stdin closed");
        std::thread::sleep(Duration::from_millis(100));
    }
}
