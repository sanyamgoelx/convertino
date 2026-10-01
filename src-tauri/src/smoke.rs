//! Hands-on check with real files, the same way a wheel pick runs them.
//!
//! Not part of the normal test run. `run-smoke.cmd` runs it:
//!   CONVERTINO_SMOKE_DIR=<folder> cargo test --lib smoke -- --ignored --nocapture
//! Every file in the folder is copied into `<folder>/results`, the wheel is
//! built for it, and every slot the wheel offers is converted there.

use crate::{convert, wheel};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

fn ext_of(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

fn describe(p: &Path) -> String {
    if p.is_dir() {
        let n = std::fs::read_dir(p).map(|r| r.count()).unwrap_or(0);
        format!("folder, {n} files")
    } else {
        format!("{} bytes", p.metadata().map(|m| m.len()).unwrap_or(0))
    }
}

fn convert_one(target: &str, files: &[PathBuf], report: &mut String) -> bool {
    let started = Instant::now();
    if target == "pdf.edit" {
        let _ = writeln!(report, "  note {target}: opens the PDF editor (checked separately)");
        return true;
    }
    let steps = match convert::plan(target, files) {
        Ok(s) => s,
        Err(e) => {
            let _ = writeln!(report, "  SKIP {target}: {e}");
            return true;
        }
    };
    let mut ok = true;
    for step in &steps {
        let mut last = 0.0;
        match convert::run(step, &mut |p| last = p) {
            Ok(out) => {
                let name = out.file_name().unwrap().to_string_lossy();
                // Size next to the original (one input, one file out).
                let change = match (step.inputs.as_slice(), out.is_file()) {
                    ([one], true) => {
                        let before = one.metadata().map(|m| m.len()).unwrap_or(0) as f64;
                        let after = out.metadata().map(|m| m.len()).unwrap_or(0) as f64;
                        if before > 0.0 { format!(", {:+.0}%", (after / before - 1.0) * 100.0) } else { String::new() }
                    }
                    _ => String::new(),
                };
                let _ = writeln!(
                    report,
                    "  ok   {target:<14} -> {name} ({}{change}, {:.1}s)",
                    describe(&out),
                    started.elapsed().as_secs_f64()
                );
                if last < 1.0 {
                    let _ = writeln!(report, "       progress stopped at {:.0}%", last * 100.0);
                }
            }
            Err(e) if e.contains("already compact") || e.contains("no selectable text") || e.contains("or smaller") => {
                let _ = writeln!(report, "  note {target}: {e}");
            }
            Err(e) => {
                ok = false;
                let _ = writeln!(report, "  FAIL {target}: {e}");
            }
        }
    }
    ok
}

#[test]
#[ignore]
fn smoke() {
    let Some(dir) = std::env::var_os("CONVERTINO_SMOKE_DIR").map(PathBuf::from) else {
        eprintln!("Set CONVERTINO_SMOKE_DIR to a folder of sample files.");
        return;
    };
    let results = dir.join("results");
    let _ = std::fs::remove_dir_all(&results);
    std::fs::create_dir_all(&results).unwrap();

    let mut sources: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    sources.sort();

    let mut report = String::new();
    let _ = writeln!(
        report,
        "Video encoders on this machine: H.264 = {}, H.265 = {}",
        crate::video::encoder(crate::video::Codec::H264).unwrap_or("none"),
        crate::video::encoder(crate::video::Codec::Hevc).unwrap_or("none (Compress uses H.264)")
    );
    let mut all_ok = true;
    let mut pdfs = Vec::new();
    // Copy everything first so outputs never collide with a later copy.
    let copies: Vec<PathBuf> = sources
        .iter()
        .map(|src| {
            let copy = results.join(src.file_name().unwrap());
            std::fs::copy(src, &copy).unwrap();
            copy
        })
        .collect();
    for copy in &copies {
        if ext_of(copy) == "pdf" {
            pdfs.push(copy.clone());
        }
        let _ = writeln!(report, "\n{}", copy.file_name().unwrap().to_string_lossy());
        let model = match wheel::build(&[copy.to_string_lossy().into_owned()], None) {
            Ok(m) => m,
            Err(e) => {
                let _ = writeln!(report, "  wheel: {e}");
                continue;
            }
        };
        let label = |v: &[wheel::Slot]| v.iter().map(|s| s.label.clone()).collect::<Vec<_>>().join(", ");
        let _ = writeln!(report, "  wheel: {} | more: {}", label(&model.main), label(&model.more));
        let targets = model
            .main
            .iter()
            .chain(model.more.iter())
            .filter(|s| s.kind == wheel::SlotKind::Target)
            .map(|s| s.id.clone())
            .collect::<Vec<_>>();
        for target in &targets {
            all_ok &= convert_one(target, &[copy.clone()], &mut report);
        }
    }
    if pdfs.len() >= 2 {
        let _ = writeln!(report, "\n{} PDFs together", pdfs.len());
        all_ok &= convert_one("pdf.merge", &pdfs, &mut report);
    }

    println!("{report}");
    std::fs::write(results.join("smoke-report.txt"), &report).unwrap();
    assert!(all_ok, "some conversions failed; see smoke-report.txt");
}
