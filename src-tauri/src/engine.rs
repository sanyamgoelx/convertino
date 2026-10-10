//! Runs conversions without any window.
//!
//! The wheel's jobs (jobs.rs), the command line (cli.rs) and the MCP server
//! (mcp.rs) all go through here, so a file converts the same way whichever
//! one asked. Progress, finished files and converter downloads arrive as
//! [`Event`]s; the caller decides how to show them (a card, a terminal line,
//! an MCP progress notification).

use crate::convert;
use crate::settings::Quality;
use crate::size;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// A new job id (also the id procs.rs uses to cancel a job's programs).
pub fn new_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::SeqCst)
}

/// What a job reports while it runs.
#[derive(Debug, Clone)]
pub enum Event {
    /// Overall progress, 0–1, with a short text ("File 2 of 5", "40%").
    Progress { fraction: f64, detail: String },
    /// A converter is being downloaded for this job.
    Download { fraction: f64, detail: String },
    /// One step finished (in any order when several run at once).
    StepDone {
        #[allow(dead_code)]
        index: usize,
        inputs: Vec<PathBuf>,
        result: Result<PathBuf, String>,
        seconds: f64,
    },
}

pub type Sink = Arc<dyn Fn(Event) + Send + Sync>;

/// What to convert, and how.
#[derive(Debug, Clone, Default)]
pub struct Request {
    pub target_id: String,
    pub files: Vec<PathBuf>,
    /// None: the quality in Settings.
    pub quality: Option<Quality>,
    /// Compress to a size.
    pub size: Option<size::Ask>,
    /// Save here instead of next to the originals (or the Settings folder).
    pub out_dir: Option<PathBuf>,
}

/// The plan for a request.
#[derive(Debug)]
pub enum Plan {
    Steps { steps: Vec<convert::Step>, notes: Vec<String> },
    /// Every file is already small enough for the size asked: fine, nothing to do.
    NothingToDo(String),
}

/// Plans a request: the steps, or a reason it can't run.
pub fn plan(req: &Request) -> Result<Plan, String> {
    let quality = req.quality.clone().unwrap_or_else(|| crate::settings::get().quality);
    match convert::plan_sized(&req.target_id, &req.files, &quality, req.size) {
        Ok((steps, notes)) => Ok(Plan::Steps { steps, notes }),
        Err(reason) if reason.starts_with(size::ALREADY) => Ok(Plan::NothingToDo(reason[size::ALREADY.len()..].to_string())),
        Err(reason) => Err(reason),
    }
}

/// What a finished job made.
#[derive(Debug, Default)]
pub struct Outcome {
    /// Outputs, in the order of the steps.
    pub made: Vec<PathBuf>,
    /// (inputs of the step, output) for each output, in the same order.
    pub made_from: Vec<(Vec<PathBuf>, PathBuf)>,
    /// (first input, error) for each failed step.
    pub failed: Vec<(PathBuf, String)>,
    pub cancelled: bool,
    /// Things worth reading even though it worked.
    pub notes: Vec<String>,
}

impl Outcome {
    /// "name: error" for each failure, as the corner card shows them.
    pub fn errors(&self) -> Vec<String> {
        self.failed.iter().map(|(p, e)| format!("{}: {e}", file_name(p))).collect()
    }
}

pub fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// How many files of one job convert at the same time. Videos and Office
/// documents go one by one (each already uses the whole machine, or can't
/// share LibreOffice); RAW photos two at once (developing one takes a lot of
/// memory); pictures, audio and PDFs use about half the cores.
pub fn parallel_for(steps: &[convert::Step]) -> usize {
    let one_at_a_time = steps.iter().any(|s| {
        matches!(s.op, convert::Op::Video { .. } | convert::Op::Office { .. } | convert::Op::OfficeThenPandoc { .. } | convert::Op::PandocThenOffice { .. })
    });
    if one_at_a_time {
        return 1;
    }
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
    let most = if steps.iter().any(|s| s.inputs.first().is_some_and(|p| crate::raw::is_raw(p))) { 2 } else { 4 };
    (cores / 2).clamp(1, most).min(steps.len().max(1))
}

/// Runs the steps of job `id` (several at once when they allow it) and
/// reports to `sink`. Cancel with `procs::cancel(id)`.
pub fn run_steps(id: u64, steps: &[convert::Step], out_dir: Option<&Path>, sink: &Sink) -> Outcome {
    let total = steps.len();
    let workers = parallel_for(steps);
    let next = AtomicU64::new(0);
    let fractions = Mutex::new(vec![0.0f64; total]);
    let results: Mutex<Vec<Option<Result<PathBuf, String>>>> = Mutex::new(vec![None; total]);
    let last_emit = Mutex::new(Instant::now() - Duration::from_secs(1));
    let report_all = |i: usize, p: f64| {
        let overall = {
            let Ok(mut f) = fractions.lock() else { return };
            f[i] = p;
            f.iter().sum::<f64>() / total as f64
        };
        let Ok(mut last) = last_emit.lock() else { return };
        if last.elapsed() < Duration::from_millis(80) && p < 1.0 {
            return;
        }
        *last = Instant::now();
        let detail = if total > 1 {
            let done = results.lock().map(|r| r.iter().filter(|x| x.is_some()).count()).unwrap_or(0);
            format!("File {} of {total}", (done + 1).min(total))
        } else {
            format!("{}%", (p * 100.0).round() as u32)
        };
        sink(Event::Progress { fraction: overall, detail });
    };
    let setup_thread = || {
        crate::procs::set_current_job(id);
        convert::set_out_override(out_dir.map(Path::to_path_buf));
        let s = sink.clone();
        let last = std::cell::Cell::new(Instant::now() - Duration::from_secs(1));
        crate::install::set_reporter(move |fraction, detail| {
            // Status lines ("Setting up …") always; percentages 5 times a second.
            if last.get().elapsed() < Duration::from_millis(200) && !detail.ends_with('…') {
                return;
            }
            last.set(Instant::now());
            s(Event::Download { fraction, detail: detail.to_string() });
        });
    };
    let work = || loop {
        if crate::procs::is_cancelled(id) {
            break;
        }
        let i = next.fetch_add(1, Ordering::SeqCst) as usize;
        let Some(step) = steps.get(i) else { break };
        log::info!("job {id}: {} ({} input(s))", step.inputs[0].display(), step.inputs.len());
        report_all(i, 0.0);
        let started = Instant::now();
        let r = convert::run(step, &mut |p| report_all(i, p));
        if let Ok(made) = &r {
            crate::stats::after_step(step, made, started.elapsed().as_secs_f64());
        }
        if r.as_ref().err().map(|e| e != crate::procs::CANCELLED).unwrap_or(true) {
            sink(Event::StepDone { index: i, inputs: step.inputs.clone(), result: r.clone(), seconds: started.elapsed().as_secs_f64() });
        }
        if let Ok(mut all) = results.lock() {
            all[i] = Some(r);
        }
    };
    if workers <= 1 {
        setup_thread();
        work();
        convert::set_out_override(None);
    } else {
        std::thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|| {
                    setup_thread();
                    work();
                });
            }
        });
    }

    let mut out = Outcome { cancelled: crate::procs::is_cancelled(id), ..Outcome::default() };
    for (step, r) in steps.iter().zip(results.into_inner().unwrap_or_default()) {
        match r {
            Some(Ok(output)) => {
                log::info!("job {id}: made {}", output.display());
                out.made.push(output.clone());
                out.made_from.push((step.inputs.clone(), output));
            }
            Some(Err(e)) if e == crate::procs::CANCELLED => out.cancelled = true,
            Some(Err(e)) => {
                log::warn!("job {id}: failed on {}: {e}", step.inputs[0].display());
                out.failed.push((step.inputs[0].clone(), e));
            }
            None => {}
        }
    }
    out.notes.extend(convert::take_notes(id));
    out
}

/// Plans and runs a request in one go (command line and MCP).
/// Err: it couldn't start at all (unknown target, nothing to convert).
pub fn convert(id: u64, req: &Request, sink: &Sink) -> Result<Outcome, String> {
    match plan(req)? {
        Plan::NothingToDo(why) => Ok(Outcome { notes: vec![why], ..Outcome::default() }),
        Plan::Steps { steps, notes } => {
            let mut out = run_steps(id, &steps, req.out_dir.as_deref(), sink);
            // "Already compact" isn't a failure: there was just nothing to gain.
            let (compact, failed): (Vec<_>, Vec<_>) = out.failed.drain(..).partition(|(_, e)| e.contains("is already compact"));
            out.failed = failed;
            out.notes.extend(compact.into_iter().map(|(p, e)| format!("{}: {e}", file_name(&p))));
            let mut all = notes;
            all.append(&mut out.notes);
            out.notes = all;
            Ok(out)
        }
    }
}

/// Size of a file, or of everything inside a folder.
pub fn disk_size(p: &Path) -> u64 {
    match p.metadata() {
        Ok(m) if m.is_dir() => std::fs::read_dir(p).map(|rd| rd.flatten().map(|e| disk_size(&e.path())).sum()).unwrap_or(0),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

pub fn human_size(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.2} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.1} MB", b / 1e6)
    } else {
        format!("{} KB", ((b / 1e3).round() as u64).max(1))
    }
}
