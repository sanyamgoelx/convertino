//! Child processes of conversion jobs: cancelling, time limits, and friendly errors.
//!
//! Every converter program is started through `spawn`, which remembers it
//! under the job running on the current thread. Cancelling a job kills its
//! programs (with their own children: LibreOffice starts a second process),
//! and quitting Convertino cancels everything so nothing keeps running.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

pub const CANCELLED: &str = "Cancelled";

thread_local! {
    static CURRENT_JOB: Cell<u64> = const { Cell::new(0) };
}

struct Registry {
    running: HashMap<u64, HashSet<u32>>,
    cancelled: HashSet<u64>,
}

fn registry() -> &'static Mutex<Registry> {
    static R: OnceLock<Mutex<Registry>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(Registry { running: HashMap::new(), cancelled: HashSet::new() }))
}

/// Marks this thread as working for `job` (0: none).
pub fn set_current_job(job: u64) {
    CURRENT_JOB.with(|j| j.set(job));
}

pub fn current_job() -> u64 {
    CURRENT_JOB.with(|j| j.get())
}

pub fn is_cancelled(job: u64) -> bool {
    registry().lock().map(|r| r.cancelled.contains(&job)).unwrap_or(false)
}

/// True when the job on this thread was cancelled.
pub fn cancelled_here() -> bool {
    let job = current_job();
    job != 0 && is_cancelled(job)
}

/// A started program, forgotten again when dropped.
pub struct Tracked {
    child: Option<Child>,
    job: u64,
    pid: u32,
}

impl Tracked {
    pub fn child(&mut self) -> &mut Child {
        self.child.as_mut().expect("child present until output()")
    }
}

impl Drop for Tracked {
    fn drop(&mut self) {
        if let Ok(mut r) = registry().lock() {
            if let Some(set) = r.running.get_mut(&self.job) {
                set.remove(&self.pid);
                if set.is_empty() {
                    r.running.remove(&self.job);
                }
            }
        }
    }
}

/// Starts `cmd` for the job on this thread.
pub fn spawn(cmd: &mut Command, tool: &str) -> Result<Tracked, String> {
    let job = current_job();
    if job != 0 && is_cancelled(job) {
        return Err(CANCELLED.into());
    }
    let child = cmd.spawn().map_err(|e| format!("Couldn't start {tool}: {e}"))?;
    let pid = child.id();
    let cancelled_meanwhile = match registry().lock() {
        Ok(mut r) => {
            r.running.entry(job).or_default().insert(pid);
            job != 0 && r.cancelled.contains(&job)
        }
        Err(_) => false,
    };
    // Cancel was pressed while the program was starting (before it was in the
    // registry, so cancel() couldn't see it): stop it now.
    if cancelled_meanwhile {
        kill_tree(pid);
    }
    Ok(Tracked { child: Some(child), job, pid })
}

/// Kills a program and everything it started.
pub fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("pkill").args(["-KILL", "-P", &pid.to_string()]).status();
        let _ = Command::new("kill").args(["-KILL", &pid.to_string()]).status();
    }
}

/// Stops a job: its programs are killed and its later steps won't start.
pub fn cancel(job: u64) {
    let pids: Vec<u32> = match registry().lock() {
        Ok(mut r) => {
            r.cancelled.insert(job);
            r.running.get(&job).map(|s| s.iter().copied().collect()).unwrap_or_default()
        }
        Err(_) => Vec::new(),
    };
    log::info!("job {job}: cancelling {} program(s)", pids.len());
    for pid in pids {
        kill_tree(pid);
    }
}

/// Stops every job (when Convertino quits).
pub fn cancel_all() {
    let jobs: Vec<u64> = registry().lock().map(|r| r.running.keys().copied().collect()).unwrap_or_default();
    for job in jobs {
        cancel(job);
    }
}

/// Runs a program to the end and returns its output. Programs that report no
/// progress get a time limit, so a stuck one (a document that makes
/// LibreOffice wait for a password, say) can't hang its job forever.
pub fn output(cmd: &mut Command, tool: &str, limit: Option<Duration>) -> Result<Output, String> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut tracked = spawn(cmd, tool)?;
    let pid = tracked.pid;
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let (timeout_tx, timeout_rx) = mpsc::channel::<bool>();
    if let Some(limit) = limit {
        std::thread::spawn(move || {
            let timed_out = matches!(done_rx.recv_timeout(limit), Err(mpsc::RecvTimeoutError::Timeout));
            if timed_out {
                kill_tree(pid);
            }
            let _ = timeout_tx.send(timed_out);
        });
    } else {
        drop(done_rx);
        let _ = timeout_tx.send(false);
    }
    // The registry entry (in `tracked`) lives until the program has ended.
    let child = tracked.child.take().expect("just spawned");
    let out = child.wait_with_output().map_err(|e| format!("{tool} stopped unexpectedly: {e}"));
    let _ = done_tx.send(());
    let timed_out = timeout_rx.recv().unwrap_or(false);
    drop(tracked);
    if cancelled_here() {
        return Err(CANCELLED.into());
    }
    if timed_out {
        return Err(format!("{tool} stopped responding, so Convertino stopped it."));
    }
    out
}

/// A plain-language version of a converter's error; the raw text is logged.
pub fn friendly(tool: &str, raw: &str) -> String {
    let lower = raw.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| lower.contains(w));
    if has(&["password", "encrypted"]) {
        return "This file is password-protected. Convertino can't open protected files yet.".into();
    }
    if has(&["no space left", "not enough space", "disk full", "enospc"]) {
        return "There isn't enough free space on the drive.".into();
    }
    if has(&["permission denied", "access is denied", "operation not permitted"]) {
        return "Convertino wasn't allowed to read the file or save next to it.".into();
    }
    if has(&[
        "invalid data found when processing input",
        "moov atom not found",
        "improper image header",
        "no decode delegate",
        "not a jpeg file",
        "corrupt",
        "may not be a pdf file",
        "couldn't find trailer",
        "end of file",
        "unexpected eof",
        "truncated",
    ]) {
        return "This file seems to be damaged, or isn't really the type its name says.".into();
    }
    let lines: Vec<&str> = raw.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let tail = lines[lines.len().saturating_sub(2)..].join(" ");
    if tail.is_empty() {
        format!("{tool} couldn't convert this file.")
    } else {
        format!("{tool}: {tail}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_are_explained() {
        assert!(friendly("Poppler", "Command Line Error: Incorrect password").contains("password-protected"));
        assert!(friendly("FFmpeg", "x.mp4: Invalid data found when processing input").contains("damaged"));
        assert!(friendly("7-Zip", "ERROR: There is not enough space on the disk").contains("free space"));
        assert_eq!(friendly("Pandoc", "line one\nline two\nline three"), "Pandoc: line two line three");
    }

    #[test]
    fn cancelling_kills_the_program() {
        let job = 987_654;
        set_current_job(job);
        let started = std::time::Instant::now();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            cancel(job);
        });
        #[cfg(windows)]
        let mut cmd = {
            let mut c = Command::new("ping");
            c.args(["-n", "30", "127.0.0.1"]);
            c
        };
        #[cfg(not(windows))]
        let mut cmd = {
            let mut c = Command::new("sleep");
            c.arg("30");
            c
        };
        let r = output(&mut cmd, "sleep", None);
        t.join().unwrap();
        assert_eq!(r.unwrap_err(), CANCELLED);
        assert!(started.elapsed() < Duration::from_secs(10));
        // Later steps of a cancelled job don't start at all.
        assert_eq!(spawn(&mut Command::new("true"), "true").err().as_deref(), Some(CANCELLED));
        set_current_job(0);
    }

    #[test]
    fn stuck_programs_time_out() {
        set_current_job(0);
        #[cfg(not(windows))]
        {
            let mut c = Command::new("sleep");
            c.arg("30");
            let r = output(&mut c, "Sleepy", Some(Duration::from_millis(300)));
            assert!(r.unwrap_err().contains("stopped responding"));
        }
    }
}
