//! Builds what the wheel shows for a set of selected files.
//!
//! The formats live in `formats.json` (one list shared by the wheel and, from
//! milestone 3, the converters). This module maps each file to a family,
//! picks the targets that make sense, and lays them out as a main ring of up
//! to 8 slots plus an optional "More" ring.

use crate::settings::{Quality, Settings};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::OnceLock;

const MAX_SLOTS: usize = 8;

#[derive(Deserialize)]
struct Registry {
    families: Vec<Family>,
}

#[derive(Deserialize)]
struct Family {
    id: String,
    label: String,
    color: String,
    extensions: Vec<String>,
    targets: Vec<Target>,
    #[serde(default)]
    more: Vec<Target>,
}

#[derive(Deserialize, Clone)]
struct Target {
    id: String,
    label: String,
    icon: String,
    hint: String,
    /// Output extension for plain format conversions.
    #[serde(default)]
    ext: Option<String>,
    /// An operation (compress, split...) rather than a format change.
    #[serde(default)]
    action: bool,
    /// Only offered when two or more files of the family are selected.
    #[serde(default)]
    multi: bool,
    /// Source extensions this target works from; empty means every file in the family.
    #[serde(default)]
    sources: Vec<String>,
}

impl Target {
    fn applies_to(&self, f: &FileInfo) -> bool {
        self.applies_to_ext(&f.ext)
    }

    /// Its sources allow the file, and it isn't a conversion to the format the file already has.
    fn applies_to_ext(&self, ext: &str) -> bool {
        let ext = canonical(&ext.to_lowercase());
        let same_format = !self.action && self.ext.as_deref().map(|e| canonical(e) == ext).unwrap_or(false);
        !same_format && (self.sources.is_empty() || self.sources.contains(&ext))
    }

    fn count(&self, files: &[&FileInfo]) -> usize {
        files.iter().filter(|f| self.applies_to(f)).count()
    }
}

/// Edit slots open a window instead of converting (the PDF and image editors).
pub fn opens_window(target_id: &str) -> bool {
    matches!(target_id, "pdf.edit" | "image.edit" | "raw.edit")
}

/// Whether a target works on a file with this extension (a PowerPoint in a
/// selection doesn't go to Markdown with the Word files, for example).
pub fn target_applies(target_id: &str, ext: &str) -> bool {
    registry()
        .families
        .iter()
        .flat_map(|f| f.targets.iter().chain(f.more.iter()))
        .find(|t| t.id == target_id)
        .map(|t| t.applies_to_ext(ext))
        .unwrap_or(true)
}

fn registry() -> &'static Registry {
    static REG: OnceLock<Registry> = OnceLock::new();
    REG.get_or_init(|| {
        serde_json::from_str(include_str!("../formats.json")).expect("formats.json is valid")
    })
}

/// Spellings that mean the same format.
fn canonical(ext: &str) -> String {
    match ext {
        "jpeg" | "jfif" | "jpe" => "jpg",
        "tif" => "tiff",
        "htm" => "html",
        "yml" => "yaml",
        "heif" => "heic",
        "aif" => "aiff",
        "tgz" | "gz" => "tgz",
        other => other,
    }
    .to_string()
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FileInfo {
    pub path: String,
    pub name: String,
    pub ext: String,
    pub family: String,
    pub size: u64,
}

#[derive(Serialize, Clone, PartialEq, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub enum SlotKind {
    #[default]
    Target,
    More,
    Back,
    /// Opens the Compare window for the compressed copies found (pairs.rs).
    Compare,
}

#[derive(Serialize, Clone, Default, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Slot {
    pub id: String,
    pub label: String,
    pub icon: String,
    pub hint: String,
    pub family: String,
    /// Family label, shown on the slot when the ring is split between families.
    pub tag: Option<String>,
    pub kind: SlotKind,
    /// How many selected files this slot applies to.
    pub count: usize,
    /// Folder slots: the count under the label ("40 videos").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub: Option<String>,
    /// Folder slots: a second line for the hint ("Leaves 2 files alone").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Folder slots: where the result goes ("→ new folder Clips (compressed)").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub out: Option<String>,
    /// The smart slot (smart.rs): first, tinted, Enter picks it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub smart: bool,
    /// Smart slot: the hint's further lines.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<String>,
    /// Smart slot: what stops it ("Part 2 is missing").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warn: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub blocked: bool,
    /// Smart slot: its Shift+click options, and the options card's button.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<crate::smart::Field>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub go: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct WheelModel {
    pub os: &'static str,
    pub accent: Option<String>,
    pub files: Vec<FileInfo>,
    /// Selected items Convertino can't convert (folders, unknown types).
    pub skipped: Vec<String>,
    pub hub_title: String,
    pub hub_subtitle: String,
    pub hub_family: String,
    pub hub_color: String,
    pub main: Vec<Slot>,
    pub more: Vec<Slot>,
    /// Settings the wheel needs: the progress ring, when it hands over, and
    /// the quality values Shift+click starts from.
    pub ring: bool,
    pub handoff_ms: u64,
    pub quality: Quality,
    /// Every preset as tuned, for Shift+click's Tune (see tune::resolved_json).
    pub presets: serde_json::Value,
    /// A Save dialog whose file the conversion waits for (see selection::Selection).
    #[serde(skip)]
    pub pending_dialog: Option<isize>,
    /// The Ask Claude button (None: hidden; see ask.rs).
    pub ask: Option<crate::ask::WheelAsk>,
    /// Everything that was selected, folders and unknown files too (what Ask Claude sends).
    #[serde(skip)]
    pub selected: Vec<String>,
    /// A folder's wheel (see folder.rs): what's in it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder: Option<FolderView>,
    /// The folder as scanned, for the pick.
    #[serde(skip)]
    pub scan: Option<std::sync::Arc<crate::folder::Scan>>,
    /// Compressed copies and their originals among what was clicked (the Compare slot).
    #[serde(skip)]
    pub compare: Vec<crate::pairs::Pair>,
    /// What the smart slot works on.
    #[serde(skip)]
    pub smart_files: Vec<String>,
}

/// What the folder wheel shows about the folder.
#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct FolderView {
    pub name: String,
    pub roots: Vec<String>,
    /// Visible files (with subfolders), and how many of them are in subfolders.
    pub files: usize,
    pub in_subfolders: usize,
    pub subfolders: usize,
    pub bytes: u64,
    /// Hidden, system and other files left out, in words ("2 hidden, 1 shortcut").
    pub alone: usize,
    pub alone_words: String,
    /// Results of earlier runs (left alone; Zip and Fit to size take them).
    pub earlier: usize,
    /// Opened from empty space inside the folder.
    pub blank: bool,
    /// More than folder::MAX_FILES: only that many were read.
    pub truncated: bool,
    /// Still reading the folder: files found so far.
    pub scanning: Option<usize>,
    /// "Use these every time" choices from Settings.
    pub defaults: crate::folder::Defaults,
    /// An image sequence: the first and last frame's names, frames, missing frames, size.
    pub sequence: Option<SequenceView>,
    /// The audio file a sequence can use as its sound.
    pub sound: Option<String>,
    /// How many files each slot touches without subfolders (slot id -> count).
    pub top_counts: std::collections::BTreeMap<String, usize>,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct SequenceView {
    pub first: String,
    pub last: String,
    pub frames: usize,
    pub gaps: usize,
    pub width: u32,
    pub height: u32,
}

fn human_size(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.2} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.1} MB", b / 1e6)
    } else {
        format!("{} KB", ((b / 1e3).round() as u64).max(1))
    }
}

/// The hint with the numbers from Settings ("150 DPI" becomes "300 DPI").
fn live_hint(id: &str, hint: &str, q: &Quality) -> String {
    let fixed = q.image == "fixed";
    let mp3 = match q.mp3 {
        256.. => "About 245 kbps".to_string(),
        192.. => "About 190 kbps".to_string(),
        160.. => "About 165 kbps".to_string(),
        128.. => "About 130 kbps".to_string(),
        _ => "About 100 kbps".to_string(),
    };
    match id {
        "pdf.jpg" | "pdf.png" | "pdf.webp" => hint.replace("150 DPI", &format!("{} DPI", q.dpi)),
        "audio.mp3" | "video.mp3" => hint.replace("320 kbps", &mp3),
        "image.jpg" if fixed => hint.replace("Quality 90", &format!("Quality {}", q.jpg)),
        "image.webp" if fixed => hint.replace("Quality 85", &format!("Quality {}", q.webp)),
        "image.jpg" | "image.webp" | "image.avif" => "Smallest that still looks the same".to_string(),
        "video.gif" => hint.replace("480 px", &format!("{} px", q.gif_width)),
        _ => hint.to_string(),
    }
}

fn slot(t: &Target, fam: &Family, count: usize, tag: bool, q: &Quality) -> Slot {
    Slot {
        id: t.id.clone(),
        label: t.label.clone(),
        icon: t.icon.clone(),
        hint: live_hint(&t.id, &t.hint, q),
        family: fam.id.clone(),
        tag: tag.then(|| fam.label.clone()),
        kind: SlotKind::Target,
        count,
        ..Slot::default()
    }
}

/// The family's targets in the order Settings gives them: the order chosen
/// on the Wheel page (targets added in a later version go last), without the
/// hidden ones, and with "Put the formats I pick most first", by picks.
fn arranged<'a>(fam: &'a Family, prefs: &Settings) -> Vec<&'a Target> {
    let all: Vec<&Target> = fam.targets.iter().chain(fam.more.iter()).collect();
    let order = prefs.order.get(&fam.id);
    let pos = |t: &Target| order.and_then(|o| o.iter().position(|id| *id == t.id)).unwrap_or(usize::MAX);
    let mut list: Vec<(usize, &Target)> = all.iter().copied().enumerate().collect();
    list.sort_by_key(|(i, t)| (pos(t), *i));
    let mut list: Vec<&Target> = list.into_iter().map(|(_, t)| t).filter(|t| !prefs.hidden.contains(&t.id)).collect();
    if list.is_empty() {
        list = all; // everything hidden would leave an empty wheel
    }
    if prefs.learn {
        // One-off picks don't reshuffle the wheel; regular ones move forward.
        let picks = |t: &Target| prefs.picks.get(&t.id).copied().filter(|n| *n >= 2).unwrap_or(0);
        list.sort_by_key(|t| std::cmp::Reverse(picks(t)));
    }
    list
}

/// Targets worth offering for these files of one family.
fn usable<'a>(targets: Vec<&'a Target>, files: &[&FileInfo]) -> Vec<&'a Target> {
    let exts: Vec<String> = files.iter().map(|f| canonical(&f.ext)).collect();
    let all_same = exts.windows(2).all(|w| w[0] == w[1]);
    targets
        .into_iter()
        .filter(|t| !(t.multi && files.len() < 2))
        .filter(|t| t.count(files) > 0)
        .filter(|t| match (&t.ext, t.action) {
            // Converting a PNG to PNG is pointless; hide it when every file already is that format.
            (Some(ext), false) => !(all_same && exts.first() == Some(&canonical(ext))),
            _ => true,
        })
        .collect()
}

pub fn build(paths: &[String], accent: Option<String>) -> Result<WheelModel, String> {
    build_with(paths, accent, &crate::settings::get())
}

fn build_with(paths: &[String], accent: Option<String>, prefs: &Settings) -> Result<WheelModel, String> {
    let reg = registry();
    let mut files = Vec::new();
    let mut skipped = Vec::new();

    for p in paths {
        let path = Path::new(p);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| p.clone());
        let meta = std::fs::metadata(path).ok();
        if meta.as_ref().map(|m| m.is_dir()).unwrap_or(false) {
            skipped.push(name);
            continue;
        }
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        // A file whose name says something other than what it is: the wheel
        // offers what fits what it is (and the smart slot fixes the name).
        let ext = if paths.len() <= 50 { crate::sniff::wrong_ext(path).filter(|r| family_of(r).is_some()).map(str::to_string).unwrap_or(ext) } else { ext };
        match reg.families.iter().find(|f| f.extensions.iter().any(|e| *e == ext)) {
            Some(fam) => files.push(FileInfo {
                path: p.clone(),
                name,
                ext,
                family: fam.id.clone(),
                size: meta.map(|m| m.len()).unwrap_or(0),
            }),
            None => skipped.push(name),
        }
    }

    // What the files are can call for a smart slot (smart.rs); it works on
    // files Convertino otherwise can't convert too (archive parts, LUTs).
    let picked_all: Vec<std::path::PathBuf> = paths.iter().map(std::path::PathBuf::from).filter(|p| p.is_file()).collect();
    let offer = if paths.len() <= 50 { crate::smart::offer(&picked_all) } else { None };
    if files.is_empty() {
        if let Some(o) = &offer {
            return Ok(smart_only(o, paths, accent, prefs));
        }
        let folders = paths.iter().filter(|p| Path::new(p).is_dir()).count();
        return Err(if paths.is_empty() {
            "Nothing is selected. Select one or more files first.".into()
        } else if folders == paths.len() {
            "Folders can't be converted. Open the folder and select the files inside.".into()
        } else if skipped.len() == 1 {
            format!("Convertino can't convert \"{}\" yet.", skipped[0])
        } else {
            "Convertino can't convert any of the selected items yet.".into()
        });
    }

    // Families in the order their files appear in the selection.
    let mut fam_ids: Vec<&str> = Vec::new();
    for f in &files {
        if !fam_ids.contains(&f.family.as_str()) {
            fam_ids.push(&f.family);
        }
    }
    let fam = |id: &str| reg.families.iter().find(|f| f.id == id).expect("known family");
    let files_of = |id: &str| files.iter().filter(|f| f.family == id).collect::<Vec<_>>();

    let (main, more) = if fam_ids.len() == 1 {
        let family = fam(fam_ids[0]);
        let fs = files_of(fam_ids[0]);
        // Up to 8 fit on the main ring; with more, 7 stay and the rest go behind "More".
        let mut main_t = usable(arranged(family, prefs), &fs);
        let more_t = if main_t.len() > MAX_SLOTS { main_t.split_off(MAX_SLOTS - 1) } else { Vec::new() };
        let mut main: Vec<Slot> = main_t.iter().map(|t| slot(t, family, t.count(&fs), false, &prefs.quality)).collect();
        let mut more: Vec<Slot> = more_t.iter().map(|t| slot(t, family, t.count(&fs), false, &prefs.quality)).collect();
        if !more.is_empty() {
            main.push(Slot {
                id: "more".into(),
                label: "More".into(),
                icon: "more".into(),
                hint: format!(
                    "More: {}",
                    more.iter().map(|s| s.label.as_str()).collect::<Vec<_>>().join(", ")
                ),
                family: family.id.clone(),
                tag: None,
                kind: SlotKind::More,
                count: fs.len(),
                ..Slot::default()
            });
            more.push(Slot {
                id: "back".into(),
                label: "Back".into(),
                icon: "back".into(),
                hint: "Back to the main ring".into(),
                family: family.id.clone(),
                tag: None,
                kind: SlotKind::Back,
                count: fs.len(),
                ..Slot::default()
            });
        }
        (main, more)
    } else {
        // Mixed selection: split the ring between up to 4 families.
        let shown = &fam_ids[..fam_ids.len().min(4)];
        let per = MAX_SLOTS / shown.len();
        let main = shown
            .iter()
            .flat_map(|id| {
                let family = fam(id);
                let fs = files_of(id);
                usable(arranged(family, prefs), &fs)
                    .into_iter()
                    .take(per)
                    .map(|t| slot(t, family, t.count(&fs), true, &prefs.quality))
                    .collect::<Vec<_>>()
            })
            .collect();
        (main, Vec::new())
    };

    let mut main = main;
    let mut more = more;
    if let Some(o) = &offer {
        if fam_ids.len() == 1 {
            place_smart(&mut main, &mut more, smart_slot(o), o.replaces, fam_ids[0], files.len());
        } else {
            main.insert(0, smart_slot(o));
        }
    }
    let smart_files: Vec<String> = offer.as_ref().map(|o| o.files.iter().map(|p| p.to_string_lossy().into_owned()).collect()).unwrap_or_default();
    let picked: Vec<std::path::PathBuf> = files.iter().map(|f| std::path::PathBuf::from(&f.path)).collect();
    let compare = crate::pairs::for_files(&picked);
    if let Some(s) = compare_slot_files(&compare, &picked) {
        main.push(s);
    }

    let first = fam(fam_ids[0]);
    let (hub_title, mut hub_subtitle) = if files.len() == 1 {
        // A RAW photo shows its megapixels ("RAW photo · 24 MP"), which says more than its size.
        let sub = (first.id == "raw").then(|| crate::raw::describe(Path::new(&files[0].path))).flatten();
        (files[0].name.clone(), sub.unwrap_or_else(|| format!("{} · {}", first.label, human_size(files[0].size))))
    } else if fam_ids.len() == 1 {
        (format!("{} files", files.len()), first.label.clone())
    } else {
        (format!("{} files", files.len()), format!("{} types", fam_ids.len()))
    };

    if files.len() == 1 {
        if let Some(h) = offer.as_ref().and_then(|o| o.hub.clone()) {
            hub_subtitle = h;
        }
    }

    Ok(WheelModel {
        os: if cfg!(target_os = "macos") { "mac" } else { "win" },
        accent,
        hub_family: if fam_ids.len() == 1 { first.id.clone() } else { "mixed".into() },
        hub_color: if fam_ids.len() == 1 { first.color.clone() } else { "#6B6F7A".into() },
        files,
        skipped,
        hub_title,
        hub_subtitle,
        main,
        more,
        ring: prefs.progress_ring,
        handoff_ms: prefs.handoff_seconds as u64 * 1000,
        quality: prefs.quality.clone(),
        presets: crate::tune::resolved_json(&prefs.quality.tune),
        pending_dialog: None,
        ask: None,
        selected: paths.to_vec(),
        folder: None,
        scan: None,
        compare,
        smart_files,
    })
}

/// A wheel with only the smart slot: files Convertino can't convert, but
/// knows what to do with (an archive's parts, LUTs, After Effects scripts).
fn smart_only(o: &crate::smart::Offer, paths: &[String], accent: Option<String>, prefs: &Settings) -> WheelModel {
    let files: Vec<FileInfo> = o
        .files
        .iter()
        .map(|p| FileInfo {
            path: p.to_string_lossy().into_owned(),
            name: file_name_of(p),
            ext: p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default(),
            family: "smart".into(),
            size: std::fs::metadata(p).map(|m| m.len()).unwrap_or(0),
        })
        .collect();
    let skipped: Vec<String> = paths.iter().map(Path::new).filter(|p| !o.files.iter().any(|f| f == p)).map(file_name_of).collect();
    let (hub_title, hub_subtitle) = match files.as_slice() {
        [one] => (one.name.clone(), o.hub.clone().unwrap_or_else(|| human_size(one.size))),
        many => (format!("{} files", many.len()), o.hub.clone().unwrap_or_else(|| o.label.clone())),
    };
    WheelModel {
        os: if cfg!(target_os = "macos") { "mac" } else { "win" },
        accent,
        hub_family: "smart".into(),
        hub_color: "#6B6F7A".into(),
        smart_files: o.files.iter().map(|p| p.to_string_lossy().into_owned()).collect(),
        files,
        skipped,
        hub_title,
        hub_subtitle,
        main: vec![smart_slot(o)],
        more: Vec::new(),
        ring: prefs.progress_ring,
        handoff_ms: prefs.handoff_seconds as u64 * 1000,
        quality: prefs.quality.clone(),
        presets: crate::tune::resolved_json(&prefs.quality.tune),
        pending_dialog: None,
        ask: None,
        selected: paths.to_vec(),
        folder: None,
        scan: None,
        compare: Vec::new(),
    }
}

/// The smart slot for what was picked.
fn smart_slot(o: &crate::smart::Offer) -> Slot {
    Slot {
        id: o.id.clone(),
        label: o.label.clone(),
        icon: o.icon.clone(),
        hint: o.hint.clone(),
        family: "smart".into(),
        tag: None,
        kind: SlotKind::Target,
        count: o.files.len(),
        sub: Some(o.sub.clone()).filter(|s| !s.is_empty()),
        smart: true,
        lines: o.lines.clone(),
        warn: o.warn.clone(),
        blocked: o.blocked,
        fields: o.fields.clone(),
        go: Some(o.go.clone()),
        ..Slot::default()
    }
}

fn more_slot(more: &[Slot], family: &str, count: usize) -> Slot {
    Slot {
        id: "more".into(),
        label: "More".into(),
        icon: "more".into(),
        hint: format!("More: {}", more.iter().filter(|s| s.kind == SlotKind::Target).map(|s| s.label.as_str()).collect::<Vec<_>>().join(", ")),
        family: family.into(),
        tag: None,
        kind: SlotKind::More,
        count,
        ..Slot::default()
    }
}

/// Puts the smart slot first. The slot it replaces goes; when the ring is
/// then over-full, its last format moves behind More (made if needed).
fn place_smart(main: &mut Vec<Slot>, more: &mut Vec<Slot>, smart: Slot, replaces: Option<&str>, family: &str, count: usize) {
    if let Some(r) = replaces {
        main.retain(|s| s.id != r);
        more.retain(|s| s.id != r);
        if more.iter().all(|s| s.kind != SlotKind::Target) {
            more.clear();
            main.retain(|s| s.kind != SlotKind::More);
        }
    }
    main.insert(0, smart);
    if main.len() <= MAX_SLOTS {
        return;
    }
    let has_more = main.last().is_some_and(|s| s.kind == SlotKind::More);
    let at = if has_more { main.len() - 2 } else { main.len() - 1 };
    let moved = main.remove(at);
    if has_more {
        more.insert(0, moved);
        let n = main.len() - 1;
        main[n] = more_slot(more, family, count);
    } else {
        more.push(moved);
        more.push(Slot { id: "back".into(), label: "Back".into(), icon: "back".into(), hint: "Back to the main ring".into(), family: family.into(), kind: SlotKind::Back, count, ..Slot::default() });
        main.push(more_slot(more, family, count));
    }
}
pub fn build_folder(scan: std::sync::Arc<crate::folder::Scan>, accent: Option<String>, blank: bool) -> Result<WheelModel, String> {
    build_folder_with(scan, accent, blank, &crate::settings::get())
}

fn build_folder_with(scan: std::sync::Arc<crate::folder::Scan>, accent: Option<String>, blank: bool, prefs: &Settings) -> Result<WheelModel, String> {
    use crate::folder;
    let d = prefs.folder;
    let name = scan.name();
    if scan.items.is_empty() {
        let hidden = if scan.alone.is_empty() { String::new() } else { format!(" (only {}: {})", if scan.alone.len() == 1 { "1 file that's left alone" } else { "files that are left alone" }, scan.alone_words()) };
        return Err(format!("{name} is empty{hidden}."));
    }
    let (mut main, mut more) = folder::slots(&scan, &d, prefs);
    let compare = crate::pairs::for_folders(&scan.roots, Some(&scan));
    // Inside the Holding Area nothing gets converted: only Zip, and Compare.
    if scan.roots.iter().any(|r| in_holding_area(r)) {
        main.retain(|s| s.id == folder::Action::Zip.id());
        more.clear();
    }
    if let Some(s) = compare_slot_folder(&compare, &name) {
        main.push(s);
    }
    // Counts without subfolders, for Shift+click's "Include subfolders".
    let top = folder::Defaults { subfolders: false, ..d };
    let mut top_counts = std::collections::BTreeMap::new();
    for (a, p) in folder::candidates(&scan, &top, prefs) {
        top_counts.insert(a.id(), p.files.len());
    }
    let top_files = scan.files(false).len();
    top_counts.insert(folder::Action::Zip.id(), top_files);
    top_counts.insert(folder::Action::Fit.id(), top_files);
    let files = scan.files(true);
    let seq = folder::find_sequence(&files).map(|s| SequenceView {
        first: s.frames.first().map(|(_, p)| file_name_of(p)).unwrap_or_default(),
        last: s.frames.last().map(|(_, p)| file_name_of(p)).unwrap_or_default(),
        frames: s.frames.len(),
        gaps: s.gaps.len(),
        width: s.width,
        height: s.height,
    });
    let bytes = scan.bytes();
    let view = FolderView {
        name: name.clone(),
        roots: scan.roots.iter().map(|r| r.to_string_lossy().into_owned()).collect(),
        files: files.len(),
        in_subfolders: files.iter().filter(|i| i.in_subfolder()).count(),
        subfolders: scan.subfolders,
        bytes,
        alone: scan.alone.len() + scan.skipped_dirs.iter().filter(|(_, w)| *w != crate::folder::Alone::Earlier).count(),
        earlier: scan.earlier(),
        alone_words: scan.alone_words(),
        blank,
        truncated: scan.truncated,
        scanning: None,
        defaults: d,
        sequence: seq,
        sound: files.iter().find(|i| i.family.as_deref() == Some("audio")).map(|i| file_name_of(&i.path)),
        top_counts,
    };
    let n = files.len();
    Ok(WheelModel {
        os: if cfg!(target_os = "macos") { "mac" } else { "win" },
        accent,
        files: Vec::new(),
        skipped: vec![name.clone()],
        hub_title: name,
        hub_subtitle: format!("{} · {}", if n == 1 { "1 file".to_string() } else { format!("{n} files") }, human_size(bytes)),
        hub_family: "folder".into(),
        hub_color: "#E8B84A".into(),
        main,
        more,
        ring: prefs.progress_ring,
        handoff_ms: prefs.handoff_seconds as u64 * 1000,
        quality: prefs.quality.clone(),
        presets: crate::tune::resolved_json(&prefs.quality.tune),
        pending_dialog: None,
        ask: None,
        selected: scan.roots.iter().map(|r| r.to_string_lossy().into_owned()).collect(),
        folder: Some(view),
        scan: Some(scan),
        compare,
        smart_files: Vec::new(),
    })
}

/// The wheel while a big folder is still being read: no slots yet.
pub fn scanning_folder(roots: &[std::path::PathBuf], found: usize, accent: Option<String>, blank: bool) -> WheelModel {
    let prefs = crate::settings::get();
    let name = match roots {
        [one] => crate::folder::folder_name(one),
        many => format!("{} folders", many.len()),
    };
    WheelModel {
        os: if cfg!(target_os = "macos") { "mac" } else { "win" },
        accent,
        files: Vec::new(),
        skipped: vec![name.clone()],
        hub_title: name.clone(),
        hub_subtitle: format!("Reading… {found} files"),
        hub_family: "folder".into(),
        hub_color: "#E8B84A".into(),
        main: Vec::new(),
        more: Vec::new(),
        ring: prefs.progress_ring,
        handoff_ms: prefs.handoff_seconds as u64 * 1000,
        quality: prefs.quality.clone(),
        presets: crate::tune::resolved_json(&prefs.quality.tune),
        pending_dialog: None,
        ask: None,
        selected: roots.iter().map(|r| r.to_string_lossy().into_owned()).collect(),
        folder: Some(FolderView { name, roots: roots.iter().map(|r| r.to_string_lossy().into_owned()).collect(), blank, scanning: Some(found), defaults: prefs.folder, ..FolderView::default() }),
        scan: None,
        compare: Vec::new(),
        smart_files: Vec::new(),
    }
}

// ---------- the Compare slot ----------

/// A folder inside a Holding Area (`_Quarantine`).
fn in_holding_area(p: &std::path::Path) -> bool {
    p.ancestors().any(|a| a.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(crate::holding::DIR)))
}

fn maker_words(maker: &str) -> &'static str {
    match maker {
        crate::pairs::DRIVE_CLEANUP => "Drive Cleanup",
        crate::pairs::CONVERTINO => "Convertino",
        crate::pairs::TRIMOTHY => "Trimothy",
        _ => "Convertino",
    }
}

/// "Drive Cleanup, 11 Oct" / "Drive Cleanup and Convertino" for a set of pairs.
fn made_by(pairs: &[crate::pairs::Pair]) -> String {
    let mut who: Vec<&str> = Vec::new();
    for p in pairs.iter().filter(|p| p.batch_dir.is_some() || p.maker != crate::pairs::BESIDE) {
        let w = maker_words(&p.maker);
        if !who.contains(&w) {
            who.push(w);
        }
    }
    match who.len() {
        0 => String::new(),
        1 => who[0].to_string(),
        _ => format!("{} and {}", who[..who.len() - 1].join(", "), who[who.len() - 1]),
    }
}

fn flagged_words(pairs: &[crate::pairs::Pair]) -> Option<String> {
    let n = pairs.iter().filter(|p| p.flag.is_some()).count();
    (n > 0).then(|| if pairs.len() == 1 { "Its copy didn't pass the automatic check".to_string() } else { format!("{n} didn't pass the automatic check") })
}

fn compare_slot(sub: String, hint: String, detail: Option<String>, count: usize) -> Slot {
    Slot { id: "compare".into(), label: "Compare".into(), icon: "compare".into(), hint, family: "compare".into(), kind: SlotKind::Compare, count, sub: Some(sub), detail, ..Slot::default() }
}

/// The Compare slot for selected files, when any of them has a compressed copy (or is one).
fn compare_slot_files(pairs: &[crate::pairs::Pair], picked: &[std::path::PathBuf]) -> Option<Slot> {
    if pairs.is_empty() {
        return None;
    }
    let n = pairs.len();
    let with = |p: &std::path::Path| pairs.iter().any(|x| crate::pairs::key(&x.original) == crate::pairs::key(p) || crate::pairs::key(&x.copy) == crate::pairs::key(p));
    let without = picked.iter().filter(|p| !with(p)).count();
    let who = made_by(pairs);
    let mut lines: Vec<String> = Vec::new();
    let (sub, hint) = if n == 1 && picked.len() == 1 {
        let p = &pairs[0];
        let is_original = crate::pairs::key(&picked[0]) == crate::pairs::key(&p.original);
        let size = |q: &std::path::Path| human_size(std::fs::metadata(q).map(|m| m.len()).unwrap_or(0));
        let place = if p.maker == crate::pairs::BESIDE { "right beside it".to_string() } else if is_original { "in its place".to_string() } else { "in the Holding Area".to_string() };
        let mut line = if is_original { format!("{} copy {place}", size(&p.copy)) } else { format!("{} original {place}", size(&p.original)) };
        if !who.is_empty() && p.maker != crate::pairs::BESIDE {
            line.push_str(&format!(" · {who}"));
        }
        lines.push(line);
        if is_original {
            ("with copy".to_string(), "Compare with its compressed copy".to_string())
        } else {
            ("with original".to_string(), "Compare with the original".to_string())
        }
    } else {
        if without > 0 {
            lines.push(format!("{without} of the {} selected {} no compressed copy, so {} left out", picked.len(), if without == 1 { "has" } else { "have" }, if without == 1 { "it's" } else { "they're" }));
        }
        if !who.is_empty() {
            lines.push(format!("Made by {who}"));
        }
        (if without > 0 { format!("{n} of {}", picked.len()) } else { format!("{n} pairs") }, format!("Compare {n} compressed {}", if n == 1 { "copy" } else { "copies" }))
    };
    if let Some(f) = flagged_words(pairs) {
        lines.push(f);
    }
    Some(compare_slot(sub, hint, (!lines.is_empty()).then(|| lines.join(" · ")), n))
}

/// The Compare slot for a folder: every pair inside it, subfolders included.
fn compare_slot_folder(pairs: &[crate::pairs::Pair], name: &str) -> Option<Slot> {
    if pairs.is_empty() {
        return None;
    }
    let n = pairs.len();
    let held = pairs.iter().filter(|p| p.batch_dir.is_some()).count();
    let mut lines = vec![format!(
        "In {name} and its subfolders · {}",
        if held == n { "originals in the Holding Area".to_string() } else if held == 0 { "originals beside their copies".to_string() } else { format!("{held} originals in the Holding Area, {} beside their copies", n - held) }
    )];
    let who = made_by(pairs);
    if !who.is_empty() {
        lines.push(format!("Made by {who}"));
    }
    if let Some(f) = flagged_words(pairs) {
        lines.push(f);
    }
    Some(compare_slot(if n == 1 { "1 copy".into() } else { format!("{n} copies") }, format!("Compare {n} compressed {}", if n == 1 { "copy" } else { "copies" }), Some(lines.join(" · ")), n))
}

fn file_name_of(p: &std::path::Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn all_targets() -> impl Iterator<Item = (&'static Family, &'static Target)> {
    registry().families.iter().flat_map(|f| f.targets.iter().chain(f.more.iter()).map(move |t| (f, t)))
}

/// A known target id ("video.mp4").
pub fn is_target(id: &str) -> bool {
    all_targets().any(|(_, t)| t.id == id)
}

/// The kind of file a target belongs to ("video").
pub fn target_family(id: &str) -> Option<String> {
    all_targets().find(|(_, t)| t.id == id).map(|(f, _)| f.id.clone())
}

/// A target's label on the wheel ("MP4").
pub fn target_label(id: &str) -> Option<String> {
    all_targets().find(|(_, t)| t.id == id).map(|(_, t)| t.label.clone())
}

/// A target's output extension ("mp4"), for format conversions.
pub fn target_ext(id: &str) -> Option<String> {
    all_targets().find(|(_, t)| t.id == id).and_then(|(_, t)| if t.action { None } else { t.ext.clone() })
}

/// The same format, whatever the spelling ("jpeg" and "jpg").
pub fn same_format(a: &str, b: &str) -> bool {
    canonical(&a.to_lowercase()) == canonical(&b.to_lowercase())
}

/// A target's icon name.
pub fn target_icon(id: &str) -> &'static str {
    all_targets().find(|(_, t)| t.id == id).map(|(_, t)| t.icon.as_str()).unwrap_or("doc")
}

/// A family's format conversions (not actions, not editors) in the order Settings gives them.
pub fn format_targets(family: &str, prefs: &Settings) -> Vec<(String, String)> {
    let Some(fam) = registry().families.iter().find(|f| f.id == family) else { return Vec::new() };
    let plain = Settings { learn: false, ..prefs.clone() };
    arranged(fam, &plain)
        .into_iter()
        .filter(|t| !t.action && t.ext.is_some() && !opens_window(&t.id))
        .map(|t| (t.id.clone(), t.label.clone()))
        .collect()
}

/// "About 245 kbps" for an MP3 setting.
pub fn mp3_words(kbps: u32) -> String {
    live_hint("audio.mp3", "320 kbps", &Quality { mp3: kbps, ..Quality::default() })
}

/// Every kind of file with all its targets in the current order, for the
/// Wheel page of Settings.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FamilyInfo {
    pub id: String,
    pub label: String,
    pub targets: Vec<TargetInfo>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TargetInfo {
    pub id: String,
    pub label: String,
    pub hint: String,
    pub hidden: bool,
}

pub fn families(prefs: &Settings) -> Vec<FamilyInfo> {
    // The order as chosen, without learning or hiding (those show as their own controls).
    let plain = Settings { learn: false, hidden: Default::default(), ..prefs.clone() };
    registry()
        .families
        .iter()
        .map(|f| FamilyInfo {
            id: f.id.clone(),
            label: f.label.clone(),
            targets: arranged(f, &plain)
                .into_iter()
                .map(|t| TargetInfo { id: t.id.clone(), label: t.label.clone(), hint: t.hint.clone(), hidden: prefs.hidden.contains(&t.id) })
                .collect(),
        })
        .collect()
}

// ---------- the format list for the command line and MCP ----------

/// A conversion, as the command line and MCP list it.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Conversion {
    /// The slot id ("image.jpg").
    pub id: String,
    /// What to type after --to ("jpg", "compress", "720p").
    pub name: String,
    pub label: String,
    pub hint: String,
    /// Needs two or more files (Merge).
    pub multi: bool,
}

/// The name to type for a target: its output format, else the part after the dot.
fn short_name(t: &Target) -> String {
    match (&t.ext, t.action) {
        (Some(e), false) => canonical(e),
        _ => t.id.split_once('.').map(|(_, n)| n.to_string()).unwrap_or_else(|| t.id.clone()),
    }
}

/// Family (id, label) of a file extension.
pub fn family_of(ext: &str) -> Option<(String, String)> {
    let ext = ext.to_lowercase();
    registry().families.iter().find(|f| f.extensions.iter().any(|e| *e == ext)).map(|f| (f.id.clone(), f.label.clone()))
}

/// What a file with this extension can become (no window-only actions such as the PDF editor).
pub fn conversions_for(ext: &str) -> Vec<Conversion> {
    let ext = ext.to_lowercase();
    let Some(fam) = registry().families.iter().find(|f| f.extensions.iter().any(|e| *e == ext)) else { return Vec::new() };
    fam.targets
        .iter()
        .chain(fam.more.iter())
        .filter(|t| !opens_window(&t.id) && t.applies_to_ext(&ext))
        .map(|t| Conversion { id: t.id.clone(), name: short_name(t), label: t.label.clone(), hint: t.hint.clone(), multi: t.multi })
        .collect()
}

/// The target a typed name means for a file with this extension ("jpeg",
/// "JPG", "image.jpg" and "Compress" all work), or None.
pub fn resolve(word: &str, ext: &str) -> Option<Conversion> {
    let w = canonical(&word.trim().trim_start_matches('.').to_lowercase());
    conversions_for(ext).into_iter().find(|c| c.id == w || c.name == w || c.label.to_lowercase() == w)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> String {
        let p = std::env::temp_dir().join(name);
        std::fs::write(&p, b"x").unwrap();
        p.to_string_lossy().into_owned()
    }

    fn zip_at(path: &std::path::Path, files: &[(&str, &[u8])]) {
        let f = std::fs::File::create(path).unwrap();
        let mut z = zip::ZipWriter::new(f);
        for (n, data) in files {
            z.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
            std::io::Write::write_all(&mut z, data).unwrap();
        }
        z.finish().unwrap();
    }

    #[test]
    fn smart_slot_comes_first_and_says_what_it_does() {
        let _lock = crate::fonts::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let d = std::env::temp_dir().join(format!("convertino-wheel-smart-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::env::set_var("CONVERTINO_FONTS_DIR", d.join("installed"));
        let s = |p: &std::path::Path| p.to_string_lossy().into_owned();
        let font = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../ui/vendor/fonts/NotoSans-Regular.ttf")).unwrap();

        // A zip of fonts: Install fonts first, the archive slots after it.
        let fonts = d.join("Noto.zip");
        zip_at(&fonts, &[("Noto/NotoSans-Regular.ttf", &font), ("Noto/OFL.txt", b"licence")]);
        let m = build(&[s(&fonts)], None).unwrap();
        assert!(m.main[0].smart);
        assert_eq!((m.main[0].id.as_str(), m.main[0].sub.as_deref(), m.main[0].family.as_str()), ("smart.fonts", Some("1 font"), "smart"));
        assert_eq!(m.main[0].go.as_deref(), Some("Install 1 font"));
        assert!(m.main.iter().any(|x| x.id == "archive.extract"));
        assert_eq!(m.hub_subtitle, "1 font inside");
        assert_eq!(m.smart_files, vec![s(&fonts)]);

        // One folder inside: Extract itself, first, with what it'll do.
        let kit = d.join("Brand Kit.zip");
        zip_at(&kit, &[("Brand Kit/a.txt", b"a"), ("Brand Kit/b.txt", b"b")]);
        let m = build(&[s(&kit)], None).unwrap();
        assert_eq!((m.main[0].id.as_str(), m.main[0].smart), ("archive.extract", true));
        assert_eq!(m.main.iter().filter(|x| x.id == "archive.extract").count(), 1);

        // A WEBP named .jpg: Fix name first, and the ring is a WEBP's (JPG offered, not WEBP).
        let webp = d.join("hero-banner.jpg");
        std::fs::write(&webp, b"RIFF\x10\x00\x00\x00WEBPVP8 data").unwrap();
        let m = build(&[s(&webp)], None).unwrap();
        assert_eq!(m.main[0].id, "smart.fix-name");
        let all: Vec<&str> = m.main.iter().chain(m.more.iter()).map(|x| x.id.as_str()).collect();
        assert!(all.contains(&"image.jpg") && !all.contains(&"image.webp"), "{all:?}");
        assert!(m.main.len() <= MAX_SLOTS, "the ring stays at {MAX_SLOTS}: {}", m.main.len());
        assert_eq!(m.files[0].ext, "webp");

        // Parts of an archive: a file Convertino can't convert, but the wheel opens with Join.
        for n in 1..=3 {
            std::fs::write(d.join(format!("Footage.zip.{n:03}")), b"x").unwrap();
        }
        let m = build(&[s(&d.join("Footage.zip.002"))], None).unwrap();
        assert_eq!(m.main.len(), 1);
        assert_eq!((m.main[0].id.as_str(), m.hub_subtitle.as_str()), ("smart.join", "Part 2 · 3 found"));
        std::env::remove_var("CONVERTINO_FONTS_DIR");
    }

    #[test]
    fn compare_slot_is_last_and_only_with_a_copy() {
        let d = std::env::temp_dir().join(format!("convertino-wheel-cmp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let (o, c, lone) = (d.join("clip.mp4"), d.join("clip (compressed).mp4"), d.join("other.mp4"));
        for p in [&o, &c, &lone] {
            std::fs::write(p, b"x").unwrap();
        }
        let s = |p: &std::path::Path| p.to_string_lossy().into_owned();
        let m = build(&[s(&o)], None).unwrap();
        let last = m.main.last().unwrap();
        assert_eq!((last.kind.clone(), last.sub.as_deref()), (SlotKind::Compare, Some("with copy")));
        assert_eq!(m.compare.len(), 1);
        assert!(m.main[..m.main.len() - 1].iter().all(|x| x.kind != SlotKind::Compare), "the other slots keep their places");
        let m = build(&[s(&c)], None).unwrap();
        assert_eq!(m.main.last().unwrap().sub.as_deref(), Some("with original"));
        let m = build(&[s(&o), s(&lone)], None).unwrap();
        let last = m.main.last().unwrap();
        assert_eq!(last.sub.as_deref(), Some("1 of 2"));
        assert!(last.detail.as_deref().unwrap_or("").contains("1 of the 2 selected has no compressed copy"));
        let m = build(&[s(&lone)], None).unwrap();
        assert!(m.compare.is_empty() && m.main.iter().all(|x| x.kind != SlotKind::Compare));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn folder_compare_counts_pairs_and_a_holding_area_folder_only_zips() {
        use std::sync::atomic::AtomicBool;
        let d = std::env::temp_dir().join(format!("convertino-wheel-fcmp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let clips = d.join("Clips");
        let sub = clips.join("Old");
        std::fs::create_dir_all(&sub).unwrap();
        for p in [clips.join("a.mp4"), clips.join("a (compressed).mp4"), sub.join("b.mov"), sub.join("b (compressed).mp4"), clips.join("c.mp4")] {
            std::fs::write(p, b"x").unwrap();
        }
        let scan = crate::folder::scan(std::slice::from_ref(&clips), &AtomicBool::new(false), &|_| {}).unwrap();
        let m = build_folder(std::sync::Arc::new(scan), None, false).unwrap();
        let last = m.main.last().unwrap();
        assert_eq!((last.kind.clone(), last.sub.as_deref()), (SlotKind::Compare, Some("2 copies")));
        assert!(m.main.len() > 2, "the usual folder slots stay");
        // A batch folder in the Holding Area: nothing is converted there.
        let batch = d.join(crate::holding::DIR).join("shrink-2026-10-11-09-58-57");
        std::fs::create_dir_all(batch.join("Clips")).unwrap();
        let (held, copy) = (batch.join("Clips").join("d.mp4"), clips.join("d.mp4"));
        std::fs::write(&held, b"x").unwrap();
        std::fs::write(&copy, b"x").unwrap();
        let man = serde_json::json!({ "batch": "shrink-2026-10-11-09-58-57", "kind": "shrink", "createdAt": 1, "entries": [ { "from": copy.to_string_lossy(), "to": held.to_string_lossy(), "isDir": false, "replaced": copy.to_string_lossy() } ] });
        std::fs::write(batch.join("manifest.json"), man.to_string()).unwrap();
        let scan = crate::folder::scan(std::slice::from_ref(&batch), &AtomicBool::new(false), &|_| {}).unwrap();
        let m = build_folder(std::sync::Arc::new(scan), None, false).unwrap();
        let ids: Vec<&str> = m.main.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["folder.zip", "compare"]);
        assert_eq!(m.main[1].sub.as_deref(), Some("1 copy"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn single_pdf_hides_merge_and_has_more() {
        let m = build(&[tmp("a.pdf")], None).unwrap();
        let ids: Vec<_> = m.main.iter().map(|s| s.id.as_str()).collect();
        assert!(!ids.contains(&"pdf.merge"));
        assert_eq!(ids.last(), Some(&"more"));
        assert_eq!(m.more.last().unwrap().kind, SlotKind::Back);
        assert!(m.main.len() <= MAX_SLOTS);
    }

    #[test]
    fn two_pdfs_offer_merge() {
        let m = build(&[tmp("a.pdf"), tmp("b.pdf")], None).unwrap();
        assert!(m.main.iter().chain(m.more.iter()).any(|s| s.id == "pdf.merge"));
        assert_eq!(m.hub_title, "2 files");
    }

    #[test]
    fn wav_hides_wav_target() {
        let m = build(&[tmp("v.wav")], None).unwrap();
        assert!(!m.main.iter().any(|s| s.id == "audio.wav"));
        assert!(m.main.iter().any(|s| s.id == "audio.mp3"));
    }

    #[test]
    fn jpeg_counts_as_jpg() {
        let m = build(&[tmp("p.jpeg")], None).unwrap();
        assert!(!m.main.iter().any(|s| s.id == "image.jpg"));
    }

    #[test]
    fn mixed_selection_splits_ring() {
        let m = build(&[tmp("x.pdf"), tmp("y.wav")], None).unwrap();
        assert_eq!(m.hub_family, "mixed");
        assert!(m.main.iter().all(|s| s.tag.is_some()));
        assert!(m.main.len() <= MAX_SLOTS);
        assert!(m.main.iter().any(|s| s.family == "pdf") && m.main.iter().any(|s| s.family == "audio"));
    }

    #[test]
    fn unknown_file_is_an_error() {
        assert!(build(&[tmp("z.qqq")], None).is_err());
        assert!(build(&[], None).is_err());
    }

    #[test]
    fn picks_only_take_files_the_target_works_on() {
        assert!(target_applies("doc.md", "docx"));
        assert!(!target_applies("doc.md", "pptx"));
        assert!(target_applies("image.png", "HEIC"));
        // A JPG among PNGs isn't re-encoded when the pick is JPG.
        assert!(!target_applies("image.jpg", "jpeg"));
        let m = build(&[tmp("a.png"), tmp("b.jpg"), tmp("c.png")], None).unwrap();
        assert_eq!(m.main.iter().find(|s| s.id == "image.jpg").unwrap().count, 2);
    }

    #[test]
    fn settings_order_hide_and_learn() {
        let mut prefs = Settings::default();
        let ids = |m: &WheelModel| m.main.iter().map(|s| s.id.clone()).collect::<Vec<_>>();
        let m = build_with(&[tmp("d.docx")], None, &prefs).unwrap();
        assert_eq!(ids(&m)[0], "doc.pdf");
        // Chosen order, with a hidden one.
        prefs.order.insert("doc".into(), vec!["doc.md".into(), "doc.txt".into()]);
        prefs.hidden.insert("doc.html".into());
        let m = build_with(&[tmp("d.docx")], None, &prefs).unwrap();
        let got = ids(&m);
        assert_eq!(&got[..3], ["doc.md", "doc.txt", "doc.pdf"]);
        assert!(!got.contains(&"doc.html".to_string()));
        // Picked often: first. Picked once: stays put.
        prefs.picks.insert("doc.odt".into(), 3);
        prefs.picks.insert("doc.pdf".into(), 1);
        let m = build_with(&[tmp("d.docx")], None, &prefs).unwrap();
        assert_eq!(&ids(&m)[..2], ["doc.odt", "doc.md"]);
        prefs.learn = false;
        let m = build_with(&[tmp("d.docx")], None, &prefs).unwrap();
        assert_eq!(ids(&m)[0], "doc.md");
        prefs.quality.dpi = 300;
        let m = build_with(&[tmp("q.pdf")], None, &prefs).unwrap();
        assert!(m.main.iter().find(|s| s.id == "pdf.jpg").unwrap().hint.contains("300 DPI"));
        // Hiding everything leaves the wheel as it was rather than empty.
        prefs.hidden = ["doc.pdf", "doc.txt", "doc.html", "doc.md", "doc.docx", "doc.odt", "doc.csv"].iter().map(|s| s.to_string()).collect();
        assert!(!build_with(&[tmp("d.docx")], None, &prefs).unwrap().main.is_empty());
        // Families list every target, hidden ones flagged.
        let fams = families(&prefs);
        let doc = fams.iter().find(|f| f.id == "doc").unwrap();
        assert_eq!(doc.targets.len(), 7);
        assert!(doc.targets.iter().all(|t| t.hidden));
    }

    #[test]
    fn slides_only_offer_what_works() {
        let m = build(&[tmp("deck.pptx")], None).unwrap();
        let ids: Vec<_> = m.main.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["doc.pdf"]);
        let m = build(&[tmp("notes.md")], None).unwrap();
        assert!(!m.main.iter().any(|s| s.id == "doc.md"));
        assert!(m.main.iter().any(|s| s.id == "doc.docx"));
        let m = build(&[tmp("r.docx"), tmp("s.pptx")], None).unwrap();
        let md = m.main.iter().find(|s| s.id == "doc.md").unwrap();
        assert_eq!(md.count, 1);
    }
}
