//! Compressed copies and their originals, for Compare (the wheel's Compare slot
//! and the Compare window).
//!
//! A pair is found two ways, whoever made the copy (TOOLS-CONTRACT.md):
//! - the Holding Area: each batch's `manifest.json` lists the original (`to`, where it
//!   waits now) and the copy that took its place (`replaced`). Drive Cleanup's batches
//!   (`shrink-<time>`, kind "shrink"), Convertino's Replace batches and Trimothy's alike;
//!   batches are told apart by their contents, never by their names.
//! - beside it: "clip (compressed).mp4" (or "clip (10 MB).mp4") next to "clip.mp4".
//!
//! A pair only counts when both files are there.
//!
//! The "Looks good" mark is shared with Drive Cleanup: `compare-marks.json` next to the
//! shared tools folder. A marked original shows as checked in Drive Cleanup's Compressed
//! Videos list, ready for its "Delete all checked"; Drive Cleanup does the deleting.

use crate::holding;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Who made the copy.
pub const DRIVE_CLEANUP: &str = "drive-cleanup";
pub const CONVERTINO: &str = "convertino";
pub const TRIMOTHY: &str = "trimothy";
/// "clip (compressed).mp4" next to "clip.mp4" (no Holding Area).
pub const BESIDE: &str = "beside";
/// A Compress job that just finished (or Settings' "Test on a file").
pub const JOB: &str = "job";

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Pair {
    pub original: PathBuf,
    pub copy: PathBuf,
    /// One of the makers above.
    pub maker: String,
    /// Held originals: their batch folder in the Holding Area.
    pub batch_dir: Option<PathBuf>,
    /// Held originals: where they were before they moved.
    pub from: Option<PathBuf>,
    /// When the batch was made, or the copy was written (ms since 1970).
    pub made_at: u64,
    /// The automatic check's complaint about the copy (Drive Cleanup keeps such copies for you to look at).
    pub flag: Option<String>,
    /// The copy is a clip that starts this many seconds into the original (Settings' test sample).
    pub offset: f64,
}

impl Pair {
    pub fn kind(&self) -> Option<&'static str> {
        kind_of(&self.copy).or_else(|| kind_of(&self.original))
    }
}

/// "video", "image" or "pdf": the kinds Compare shows (sound has nothing to look at).
pub fn kind_of(p: &Path) -> Option<&'static str> {
    let ext = p.extension()?.to_string_lossy().to_lowercase();
    match crate::wheel::family_of(&ext)?.0.as_str() {
        "video" => Some("video"),
        "image" | "raw" => Some("image"),
        "pdf" => Some("pdf"),
        _ => None,
    }
}

// ---------- paths ----------

/// The same file? (Windows and Mac paths ignore case.)
pub fn key(p: &Path) -> String {
    let s = p.to_string_lossy();
    if cfg!(windows) {
        s.replace('/', "\\").to_lowercase()
    } else if cfg!(target_os = "macos") {
        s.to_lowercase()
    } else {
        s.into_owned()
    }
}

fn same(a: &Path, b: &Path) -> bool {
    key(a) == key(b)
}

/// `p` is `root` or inside it.
fn under(p: &Path, root: &Path) -> bool {
    let (p, r) = (key(p), key(root));
    let r = r.trim_end_matches(['\\', '/']);
    p == r || p.strip_prefix(r).is_some_and(|rest| rest.starts_with(['\\', '/']))
}

fn is_file(p: &Path) -> bool {
    std::fs::metadata(p).map(|m| m.is_file()).unwrap_or(false)
}

fn modified_ms(p: &Path) -> u64 {
    std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// "clip (compressed)" / "clip (compressed 2)" / "clip (10 MB)" / "clip (10 MB, first 7 s)" -> "clip":
/// the names Convertino's Compress and Fit to size give their copies. None for other names.
pub fn compressed_base(stem: &str) -> Option<&str> {
    let start = stem.rfind(" (")?;
    let inner = stem[start + 2..].strip_suffix(')')?;
    let numbered = inner.strip_prefix("compressed").is_some_and(|r| r.is_empty() || (r.starts_with(' ') && r[1..].chars().all(|c| c.is_ascii_digit()) && r.len() > 1));
    let size_like = {
        let first = inner.split(',').next().unwrap_or("");
        let mut parts = first.split(' ');
        let num = parts.next().unwrap_or("");
        let unit = parts.next().unwrap_or("");
        !num.is_empty() && num.chars().all(|c| c.is_ascii_digit() || c == '.') && matches!(unit, "KB" | "MB" | "GB") && parts.next().is_none()
    };
    (numbered || size_like).then(|| &stem[..start]).filter(|b| !b.is_empty())
}

fn stem_of(p: &Path) -> String {
    p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

fn files_in(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|rd| rd.flatten().filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false)).map(|e| e.path()).collect())
        .unwrap_or_default()
}

// ---------- the Holding Area ----------

struct Entry {
    to: PathBuf,
    replaced: PathBuf,
    from: Option<PathBuf>,
    flag: Option<String>,
}

struct Batch {
    dir: PathBuf,
    maker: &'static str,
    made_at: u64,
    entries: Vec<Entry>,
}

/// The Holding Areas a file's pair can be in: the one for its drive (or user folder),
/// and the one it sits in when it is a held original itself.
fn areas_for(p: &Path) -> Vec<PathBuf> {
    let mut out = vec![holding::place(p).0];
    if let Some(a) = p.ancestors().find(|a| a.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(holding::DIR))) {
        out.push(a.to_path_buf());
    }
    out.dedup_by(|a, b| same(a, b));
    out
}

fn maker_of(m: &Value, dir: &Path) -> &'static str {
    let kind = m.get("kind").and_then(Value::as_str).unwrap_or("").to_lowercase();
    let name = dir.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    if kind == "convertino" || name.ends_with(" convertino") {
        CONVERTINO
    } else if kind == "trimothy" || name.ends_with(" trimothy") {
        TRIMOTHY
    } else {
        DRIVE_CLEANUP
    }
}

/// Every batch in a Holding Area with at least one original replaced by a copy.
fn read_area(area: &Path) -> Vec<Batch> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(area) else { return out };
    for e in rd.flatten() {
        let dir = e.path();
        let Ok(text) = std::fs::read_to_string(dir.join("manifest.json")) else { continue };
        let Ok(m) = serde_json::from_str::<Value>(&text) else { continue };
        let entries: Vec<Entry> = m
            .get("entries")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter(|x| !x.get("isDir").and_then(Value::as_bool).unwrap_or(false))
                    .filter_map(|x| {
                        let to = x.get("to").and_then(Value::as_str)?;
                        let replaced = x.get("replaced").and_then(Value::as_str).filter(|r| !r.is_empty())?;
                        Some(Entry {
                            to: PathBuf::from(to),
                            replaced: PathBuf::from(replaced),
                            from: x.get("from").and_then(Value::as_str).map(PathBuf::from),
                            flag: x.get("flag").and_then(Value::as_str).filter(|f| !f.is_empty()).map(str::to_string),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        if entries.is_empty() {
            continue;
        }
        out.push(Batch { maker: maker_of(&m, &dir), made_at: m.get("createdAt").and_then(Value::as_u64).unwrap_or(0), dir, entries });
    }
    out
}

/// Reads each Holding Area once per lookup.
#[derive(Default)]
struct Areas(HashMap<String, Vec<Batch>>);

impl Areas {
    fn batches(&mut self, p: &Path) -> Vec<&Batch> {
        let areas = areas_for(p);
        for a in &areas {
            self.0.entry(key(a)).or_insert_with(|| read_area(a));
        }
        let keys: HashSet<String> = areas.iter().map(|a| key(a)).collect();
        self.0.iter().filter(|(k, _)| keys.contains(*k)).flat_map(|(_, v)| v.iter()).collect()
    }
}

fn held_pair(b: &Batch, e: &Entry) -> Pair {
    Pair {
        original: e.to.clone(),
        copy: e.replaced.clone(),
        maker: b.maker.to_string(),
        batch_dir: Some(b.dir.clone()),
        from: e.from.clone(),
        made_at: b.made_at,
        flag: e.flag.clone(),
        offset: 0.0,
    }
}

fn beside_pair(original: PathBuf, copy: PathBuf) -> Pair {
    let made_at = modified_ms(&copy);
    Pair { original, copy, maker: BESIDE.into(), batch_dir: None, from: None, made_at, flag: None, offset: 0.0 }
}

/// The original next to a "(compressed)" copy: same name before the brackets, same
/// extension preferred, never another copy.
fn original_beside(copy: &Path) -> Option<PathBuf> {
    let stem = stem_of(copy);
    let base = compressed_base(&stem)?.to_lowercase();
    let ext = copy.extension().map(|e| e.to_string_lossy().to_lowercase());
    let mut cands: Vec<PathBuf> = files_in(copy.parent()?)
        .into_iter()
        .filter(|f| !same(f, copy) && stem_of(f).to_lowercase() == base && kind_of(f).is_some())
        .collect();
    cands.sort_by_key(|f| (f.extension().map(|e| e.to_string_lossy().to_lowercase()) != ext, key(f)));
    cands.into_iter().next()
}

/// The newest "(compressed)" copy next to an original.
fn copy_beside(original: &Path) -> Option<PathBuf> {
    let stem = stem_of(original).to_lowercase();
    files_in(original.parent()?)
        .into_iter()
        .filter(|f| !same(f, original) && kind_of(f).is_some() && compressed_base(&stem_of(f)).is_some_and(|b| b.to_lowercase() == stem))
        .max_by_key(|f| modified_ms(f))
}

fn keep(out: &mut Vec<Pair>, seen: &mut HashSet<(String, String)>, p: Pair) {
    if p.kind().is_none() || !is_file(&p.original) || !is_file(&p.copy) || same(&p.original, &p.copy) {
        return;
    }
    if seen.insert((key(&p.original), key(&p.copy))) {
        out.push(p);
    }
}

/// Pairs for selected files: each file may be the original (held, or with a copy beside
/// it) or the copy (in the original's place, or "(compressed)" beside it).
pub fn for_files(paths: &[PathBuf]) -> Vec<Pair> {
    let mut areas = Areas::default();
    let (mut out, mut seen) = (Vec::new(), HashSet::new());
    for p in paths.iter().filter(|p| is_file(p)) {
        let mut found = false;
        for b in areas.batches(p) {
            for e in &b.entries {
                if same(&e.to, p) || same(&e.replaced, p) {
                    let before = out.len();
                    keep(&mut out, &mut seen, held_pair(b, e));
                    found |= out.len() > before;
                }
            }
        }
        if found {
            continue;
        }
        if let Some(o) = original_beside(p) {
            keep(&mut out, &mut seen, beside_pair(o, p.clone()));
        } else if let Some(c) = copy_beside(p) {
            keep(&mut out, &mut seen, beside_pair(p.clone(), c));
        }
    }
    out
}

/// Pairs inside folders (subfolders too): copies or held originals inside them, and
/// "(compressed)" copies with their originals beside them. A Holding Area batch folder
/// gives every pair in it.
pub fn for_folders(roots: &[PathBuf], scan: Option<&crate::folder::Scan>) -> Vec<Pair> {
    let mut areas = Areas::default();
    let (mut out, mut seen) = (Vec::new(), HashSet::new());
    for root in roots {
        for b in areas.batches(root) {
            for e in &b.entries {
                if under(&e.replaced, root) || under(&e.to, root) {
                    keep(&mut out, &mut seen, held_pair(b, e));
                }
            }
        }
    }
    if let Some(scan) = scan {
        // Earlier results ("clip (compressed).mp4" next to "clip.mp4") are items the scan marks as such.
        for c in scan.items.iter().filter(|i| i.earlier).map(|i| &i.path) {
            if let Some(o) = original_beside(c) {
                keep(&mut out, &mut seen, beside_pair(o, c.clone()));
            }
        }
    }
    out.sort_by_key(|p| key(&p.copy));
    out
}

// ---------- the shared "Looks good" mark ----------

const MARKS_ENV: &str = "CONVERTINO_MARKS_FILE";

/// `%LOCALAPPDATA%\sanyamgoelx\compare-marks.json` (next to the shared tools folder).
pub fn marks_file() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os(MARKS_ENV) {
        return Some(PathBuf::from(p));
    }
    Some(crate::siblings::dir()?.parent()?.join("compare-marks.json"))
}

fn read_marks(file: &Path) -> Value {
    std::fs::read_to_string(file).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).filter(Value::is_object).unwrap_or_else(|| json!({ "schema": 1, "marks": {} }))
}

/// When `original` was marked "Looks good" (ms), if it is.
pub fn marked(original: &Path) -> Option<u64> {
    let m = read_marks(&marks_file()?);
    m.get("marks")?.get(key(original))?.get("at")?.as_u64()
}

/// Marks (or unmarks) `original` for deletion. Returns the new state.
pub fn set_mark(original: &Path, copy: &Path, on: bool) -> Result<bool, String> {
    let file = marks_file().ok_or("There's no place to keep the mark on this computer")?;
    let mut m = read_marks(&file);
    if !m.get("marks").is_some_and(Value::is_object) {
        m["marks"] = json!({});
    }
    let marks = m["marks"].as_object_mut().expect("an object");
    if on {
        marks.insert(key(original), json!({ "at": now_ms(), "by": "convertino", "copy": copy.to_string_lossy() }));
    } else {
        marks.remove(&key(original));
    }
    m["schema"] = json!(1);
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Couldn't keep the mark: {e}"))?;
    }
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&m).map_err(|e| e.to_string())?).map_err(|e| format!("Couldn't keep the mark: {e}"))?;
    std::fs::rename(&tmp, &file).map_err(|e| format!("Couldn't keep the mark: {e}"))?;
    Ok(on)
}

// ---------- putting an original back (Convertino's own batches) ----------

/// A free name next to `dir`: "stem (compressed).ext", then "stem (compressed 2).ext"…
fn free_copy_name(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let dot = if ext.is_empty() { String::new() } else { format!(".{ext}") };
    let first = dir.join(format!("{stem} (compressed){dot}"));
    if !first.exists() {
        return first;
    }
    (2..).map(|n| dir.join(format!("{stem} (compressed {n}){dot}"))).find(|p| !p.exists()).expect("a free name")
}

/// Puts a held original back where it was. Only for Convertino's batches: the tool
/// that made a batch is the one that undoes it (Drive Cleanup does its own).
/// The copy stays: next to the original as "name (compressed).ext" when it had the
/// original's name. Returns what happened, in words.
pub fn put_back(p: &Pair) -> Result<String, String> {
    if p.maker != CONVERTINO {
        return Err("Only the tool that made this batch can put the original back".into());
    }
    let from = p.from.clone().ok_or("The Holding Area's list doesn't say where the original was")?;
    if !is_file(&p.original) {
        return Err("The original isn't in the Holding Area any more".into());
    }
    let name = |q: &Path| q.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut copy_now = p.copy.clone();
    if same(&p.copy, &from) {
        let dir = from.parent().ok_or("No folder to put it back in")?;
        let ext = from.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
        let aside = free_copy_name(dir, &stem_of(&from), &ext);
        std::fs::rename(&p.copy, &aside).map_err(|e| format!("Couldn't move the compressed copy aside: {e}"))?;
        copy_now = aside;
    } else if from.exists() {
        return Err(format!("Something else is called {} now, so the original stayed in the Holding Area", name(&from)));
    }
    if let Some(dir) = from.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::rename(&p.original, &from) {
        if copy_now != p.copy {
            let _ = std::fs::rename(&copy_now, &p.copy);
        }
        return Err(format!("Couldn't put the original back: {e}"));
    }
    holding::forget(&p.original);
    if marked(&p.original).is_some() {
        let _ = set_mark(&p.original, &p.copy, false);
    }
    Ok(format!("{} is back in its folder. The compressed copy stays as {}.", name(&from), name(&copy_now)))
}

// ---------- Drive Cleanup ----------

/// Drive Cleanup's launcher, from its entry in the shared tools folder, when it is installed.
pub fn drive_cleanup_app() -> Option<PathBuf> {
    let text = std::fs::read_to_string(crate::siblings::dir()?.join("drive-cleanup.json")).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let app = PathBuf::from(v.get("app")?.as_str()?);
    is_file(&app).then_some(app)
}

/// Opens Drive Cleanup on this pair ("Compare with compressed copy", where it puts back or deletes).
pub fn open_in_drive_cleanup(original: &Path) -> Result<(), String> {
    let app = drive_cleanup_app().ok_or("Drive Cleanup isn't installed")?;
    let is_vbs = app.extension().is_some_and(|e| e.eq_ignore_ascii_case("vbs"));
    let mut cmd = if is_vbs {
        let sys = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        let mut c = std::process::Command::new(Path::new(&sys).join("System32").join("wscript.exe"));
        c.arg(&app);
        c
    } else {
        std::process::Command::new(&app)
    };
    cmd.arg("--compare").arg(original);
    cmd.spawn().map(|_| ()).map_err(|e| format!("Couldn't start Drive Cleanup: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("convertino-pairs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn touch(p: &Path) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, "x").unwrap();
    }

    #[test]
    fn compressed_names() {
        assert_eq!(compressed_base("clip (compressed)"), Some("clip"));
        assert_eq!(compressed_base("clip (compressed 2)"), Some("clip"));
        assert_eq!(compressed_base("trip (10 MB)"), Some("trip"));
        assert_eq!(compressed_base("trip (9.5 MB, first 7 s)"), Some("trip"));
        assert_eq!(compressed_base("clip (720p)"), None);
        assert_eq!(compressed_base("clip (compressedx)"), None);
        assert_eq!(compressed_base("clip"), None);
    }

    /// A Drive Cleanup batch (named shrink-<time>, kind "shrink") is found from the held
    /// original, from the copy in its place and from the folder; a removal entry (no
    /// copy) and a missing copy give nothing.
    #[test]
    fn drive_cleanup_batches_are_found_by_contents() {
        let root = tmp("dc");
        let area = root.join(holding::DIR);
        let batch = area.join("shrink-2026-10-11-09-58-57");
        let clips = root.join("Videos").join("Valorant");
        let (held, copy) = (batch.join("Videos/Valorant/a.mp4"), clips.join("a.mp4"));
        let (held2, gone) = (batch.join("Videos/Valorant/b.mp4"), clips.join("b.mp4"));
        let removed = batch.join("Videos/Valorant/old.mp4");
        for p in [&held, &copy, &held2, &removed] {
            touch(p);
        }
        let m = json!({ "batch": "shrink-2026-10-11-09-58-57", "kind": "shrink", "createdAt": 1791700000000u64, "entries": [
            { "from": copy.to_string_lossy(), "to": held.to_string_lossy(), "bytes": 1, "isDir": false, "label": "shrink", "replaced": copy.to_string_lossy(), "flag": "2 of 8 frames" },
            { "from": gone.to_string_lossy(), "to": held2.to_string_lossy(), "bytes": 1, "isDir": false, "label": "shrink", "replaced": gone.to_string_lossy() },
            { "from": clips.join("old.mp4").to_string_lossy(), "to": removed.to_string_lossy(), "bytes": 1, "isDir": false, "label": "Junk" }
        ]});
        std::fs::write(batch.join("manifest.json"), m.to_string()).unwrap();
        let _base = holding::test_base();
        // areas_for() also reads the area a held file sits in, which is how this test's
        // area is found; the copy side goes through the folder lookup.
        let from_held = for_files(std::slice::from_ref(&held));
        assert_eq!(from_held.len(), 1);
        assert_eq!(from_held[0].copy, copy);
        assert_eq!(from_held[0].maker, DRIVE_CLEANUP);
        assert_eq!(from_held[0].flag.as_deref(), Some("2 of 8 frames"));
        assert!(for_files(std::slice::from_ref(&held2)).is_empty(), "the copy is gone");
        let from_batch = for_folders(std::slice::from_ref(&batch), None);
        assert_eq!(from_batch.len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn beside_copies_pair_both_ways() {
        let d = tmp("beside");
        let (o, c, other) = (d.join("hero.mov"), d.join("hero (compressed).mp4"), d.join("hero (720p).mp4"));
        for p in [&o, &c, &other] {
            touch(p);
        }
        let a = for_files(std::slice::from_ref(&o));
        assert_eq!(a.len(), 1);
        assert_eq!((a[0].original.clone(), a[0].copy.clone(), a[0].maker.as_str()), (o.clone(), c.clone(), BESIDE));
        let b = for_files(std::slice::from_ref(&c));
        assert_eq!(b, a);
        assert!(for_files(std::slice::from_ref(&other)).is_empty(), "a 720p conversion isn't a compressed copy");
        // Selecting both gives the pair once.
        assert_eq!(for_files(&[o.clone(), c.clone()]).len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn marks_are_shared_and_keyed_by_path() {
        let d = tmp("marks");
        let file = d.join("compare-marks.json");
        std::env::set_var(MARKS_ENV, &file);
        let o = d.join("Clip.mp4");
        assert!(marked(&o).is_none());
        assert!(set_mark(&o, &d.join("c.mp4"), true).unwrap());
        assert!(marked(&o).is_some());
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(v["schema"], 1);
        assert_eq!(v["marks"][key(&o)]["by"], "convertino");
        assert!(!set_mark(&o, &d.join("c.mp4"), false).unwrap());
        assert!(marked(&o).is_none());
        std::env::remove_var(MARKS_ENV);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn put_back_moves_a_same_name_copy_aside() {
        let root = tmp("putback");
        let batch = root.join(holding::DIR).join("2026-10-13-09-41-07 convertino");
        let from = root.join("Clips").join("a.mp4");
        let held = batch.join("Clips").join("a.mp4");
        touch(&held);
        touch(&from);
        std::fs::write(&from, "copy").unwrap();
        holding::record(&held, &from, &from, 1).unwrap();
        let p = for_files(std::slice::from_ref(&held)).pop().expect("the pair");
        assert_eq!(p.maker, CONVERTINO);
        let said = put_back(&p).unwrap();
        assert!(said.contains("a (compressed).mp4"), "{said}");
        assert_eq!(std::fs::read_to_string(&from).unwrap(), "x");
        assert_eq!(std::fs::read_to_string(root.join("Clips").join("a (compressed).mp4")).unwrap(), "copy");
        assert!(!batch.exists(), "the empty batch went");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn put_back_refuses_other_tools_batches() {
        let p = Pair { original: "/x/a".into(), copy: "/y/a".into(), maker: DRIVE_CLEANUP.into(), batch_dir: None, from: Some("/y/a".into()), made_at: 0, flag: None, offset: 0.0 };
        assert!(put_back(&p).is_err());
    }
}
