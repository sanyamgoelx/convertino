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

#[derive(Serialize, Clone, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum SlotKind {
    Target,
    More,
    Back,
}

#[derive(Serialize, Clone)]
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
    /// A Save dialog whose file the conversion waits for (see selection::Selection).
    #[serde(skip)]
    pub pending_dialog: Option<isize>,
    /// The Ask Claude button (None: hidden; see ask.rs).
    pub ask: Option<crate::ask::WheelAsk>,
    /// Everything that was selected, folders and unknown files too (what Ask Claude sends).
    #[serde(skip)]
    pub selected: Vec<String>,
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

    if files.is_empty() {
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

    let first = fam(fam_ids[0]);
    let (hub_title, hub_subtitle) = if files.len() == 1 {
        // A RAW photo shows its megapixels ("RAW photo · 24 MP"), which says more than its size.
        let sub = (first.id == "raw").then(|| crate::raw::describe(Path::new(&files[0].path))).flatten();
        (files[0].name.clone(), sub.unwrap_or_else(|| format!("{} · {}", first.label, human_size(files[0].size))))
    } else if fam_ids.len() == 1 {
        (format!("{} files", files.len()), first.label.clone())
    } else {
        (format!("{} files", files.len()), format!("{} types", fam_ids.len()))
    };

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
        pending_dialog: None,
        ask: None,
        selected: paths.to_vec(),
    })
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
