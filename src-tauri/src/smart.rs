//! Smart actions: Convertino looks at what a file is (sniff.rs) and, when it
//! recognises something it can finish in one go, the wheel's first slot does
//! that: install the fonts in a zip, unpack every zip inside a zip, join an
//! archive in parts, fix a wrong extension, save the pictures from a PDF or
//! a PowerPoint, fix a CSV for Excel, split a recording's sound tracks, save
//! a video's subtitles, install LUTs or After Effects presets and panels.
//!
//! `offer` says what the slot shows (its label, the counts, every line of the
//! hint, its Shift+click options). `plan` turns the same look into steps, so
//! what runs is what the slot said. Undo puts things back the way each
//! action needs: fonts are uninstalled, a renamed file gets its name back,
//! copies made into app folders are removed.

use crate::convert::{self, Op, Step};
use crate::fonts;
use crate::sniff::{self, Entry, Listing};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

pub const FONTS: &str = "smart.fonts";
pub const EXTRACT: &str = "archive.extract";
pub const EXTRACT_ALL: &str = "smart.extract-all";
pub const JOIN: &str = "smart.join";
pub const FIX_NAME: &str = "smart.fix-name";
pub const MEDIA: &str = "smart.media";
pub const PDF_IMAGES: &str = "smart.pdf-images";
pub const EXCEL: &str = "smart.excel";
pub const SOUND: &str = "smart.sound";
pub const SUBTITLES: &str = "smart.subtitles";
pub const LUTS: &str = "smart.luts";
pub const AE: &str = "smart.ae";

/// Command-line words for each action (`--to fonts`).
pub const WORDS: [(&str, &str); 12] = [
    ("fonts", FONTS),
    ("install-fonts", FONTS),
    ("extract-all", EXTRACT_ALL),
    ("join", JOIN),
    ("fix-name", FIX_NAME),
    ("media", MEDIA),
    ("images", PDF_IMAGES),
    ("excel", EXCEL),
    ("split-sound", SOUND),
    ("subtitles", SUBTITLES),
    ("luts", LUTS),
    ("ae", AE),
];

/// The action a command-line word means.
pub fn from_word(word: &str) -> Option<&'static str> {
    let w = word.trim().to_lowercase();
    WORDS.iter().find(|(k, id)| *k == w || *id == w).map(|(_, id)| *id)
}

/// The command-line word for an action ("fonts" for smart.fonts).
pub fn word_of(id: &str) -> Option<&'static str> {
    if id == EXTRACT {
        return Some("extract");
    }
    WORDS.iter().find(|(_, x)| *x == id).map(|(w, _)| *w)
}

pub fn is_smart(id: &str) -> bool {
    id.starts_with("smart.")
}

/// Actions that change the computer rather than make a file (AI apps may not start these).
pub fn installs(id: &str) -> bool {
    matches!(id, FONTS | LUTS | AE)
}

/// What the done card calls a finished action.
pub fn done_title(id: &str, made: usize) -> Option<String> {
    let plural = |one: &str, many: &str| if made == 1 { one.to_string() } else { many.to_string() };
    Some(match id {
        EXTRACT_ALL => "Extracted every level".into(),
        JOIN => "Joined and extracted".into(),
        FIX_NAME => plural("Renamed", &format!("Renamed {made} files")),
        MEDIA => "Saved the media".into(),
        PDF_IMAGES => "Saved the pictures".into(),
        EXCEL => "Saved for Excel".into(),
        SOUND => plural("Saved the mixed copy", &format!("Saved {made} sound tracks")),
        SUBTITLES => plural("Saved the subtitles", &format!("Saved {made} subtitle files")),
        _ => return None,
    })
}

// ---------- what the slot shows ----------

/// One choice of a Shift+click option.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Choice {
    pub value: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub: Option<String>,
}

/// One Shift+click option. `kind`: "radio", "select", "check", "folder"
/// (a checkbox with a folder and a Change… button; its value is the folder
/// or null) or "path" (just the folder and Change…).
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Field {
    pub key: String,
    pub kind: &'static str,
    pub label: String,
    pub choices: Vec<Choice>,
    pub value: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub: Option<String>,
}

fn radio(key: &str, label: &str, value: &str, choices: &[(&str, &str, &str)]) -> Field {
    Field {
        key: key.into(),
        kind: "radio",
        label: label.into(),
        choices: choices.iter().map(|(v, l, s)| Choice { value: v.to_string(), label: l.to_string(), sub: (!s.is_empty()).then(|| s.to_string()) }).collect(),
        value: json!(value),
        sub: None,
    }
}
fn select(key: &str, label: &str, value: &str, choices: &[(&str, &str)]) -> Field {
    Field { key: key.into(), kind: "select", label: label.into(), choices: choices.iter().map(|(v, l)| Choice { value: v.to_string(), label: l.to_string(), sub: None }).collect(), value: json!(value), sub: None }
}
fn check(key: &str, label: &str, on: bool, sub: Option<String>) -> Field {
    Field { key: key.into(), kind: "check", label: label.into(), choices: Vec::new(), value: json!(on), sub }
}
fn folder_field(key: &str, label: &str, value: Option<&str>) -> Field {
    Field { key: key.into(), kind: "folder", label: label.into(), choices: Vec::new(), value: value.map(|v| json!(v)).unwrap_or(Value::Null), sub: None }
}

/// What the smart slot shows, and what it works on.
#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Offer {
    pub id: String,
    pub label: String,
    pub icon: String,
    /// Under the label ("8 fonts").
    pub sub: String,
    /// The hint's first line, after the label.
    pub hint: String,
    /// More lines for the hint (where things go, what's left out).
    pub lines: Vec<String>,
    /// A problem that stops it ("Part 3 is missing").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warn: Option<String>,
    /// Shown greyed out: `warn` says why.
    pub blocked: bool,
    /// Shift+click options.
    pub fields: Vec<Field>,
    /// The options card's button ("Install 8 fonts").
    pub go: String,
    /// The hub's second line for one file ("30 fonts inside").
    #[serde(skip)]
    pub hub: Option<String>,
    /// A normal slot this one takes the place of (Extract).
    #[serde(skip)]
    pub replaces: Option<&'static str>,
    /// The files it works on.
    #[serde(skip)]
    pub files: Vec<PathBuf>,
}

/// Options for an action: Settings' "Use these every time", then `given` on top.
pub fn options(id: &str, given: Option<&Value>) -> Value {
    let mut v = crate::settings::get().smart.get(id).cloned().unwrap_or_else(|| json!({}));
    if let (Some(Value::Object(g)), Value::Object(m)) = (given, &mut v) {
        for (k, x) in g {
            m.insert(k.clone(), x.clone());
        }
    }
    if !v.is_object() {
        v = json!({});
    }
    v
}

fn opt_str<'a>(o: &'a Value, k: &str, default: &'a str) -> &'a str {
    o.get(k).and_then(Value::as_str).unwrap_or(default)
}
fn opt_bool(o: &Value, k: &str) -> bool {
    o.get(k).and_then(Value::as_bool).unwrap_or(false)
}
fn opt_folder(o: &Value, k: &str) -> Option<PathBuf> {
    o.get(k).and_then(Value::as_str).filter(|s| !s.trim().is_empty()).map(PathBuf::from)
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn name(p: &Path) -> String {
    crate::engine::file_name(p)
}

fn stem_of(p: &Path) -> String {
    p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

fn ext_of(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

fn words(list: &[String]) -> String {
    match list {
        [] => String::new(),
        [one] => one.clone(),
        [a, b] => format!("{a} and {b}"),
        _ => format!("{} and {}", list[..list.len() - 1].join(", "), list[list.len() - 1]),
    }
}

fn os_fonts_folder() -> &'static str {
    if cfg!(windows) {
        "Windows' Fonts folder"
    } else if cfg!(target_os = "macos") {
        "your Fonts folder"
    } else {
        "the fonts folder"
    }
}

/// Listings of the archives among `files`, read once per look.
struct Look {
    listings: HashMap<PathBuf, Option<Listing>>,
}

impl Look {
    fn new() -> Self {
        Look { listings: HashMap::new() }
    }
    fn listing(&mut self, p: &Path) -> Option<&Listing> {
        if !sniff::is_archive_name(&name(p)) {
            return None;
        }
        self.listings.entry(p.to_path_buf()).or_insert_with(|| sniff::list(p)).as_ref()
    }
}

/// The smart slot for these items (files; folders are left out), if any.
pub fn offer(paths: &[PathBuf]) -> Option<Offer> {
    offer_with(paths, None)
}

/// The same with Shift+click options (the options card's live preview).
pub fn offer_with(paths: &[PathBuf], given: Option<&Value>) -> Option<Offer> {
    let files: Vec<PathBuf> = paths.iter().filter(|p| p.is_file()).cloned().collect();
    if files.is_empty() || files.len() > 50 {
        return None;
    }
    let mut look = Look::new();
    ORDER.iter().find_map(|id| one(id, &files, &mut look, given)).map(|x| finish(x, &files))
}

/// Which action wins when several fit (the slot shows the first).
const ORDER: [&str; 12] = [JOIN, FONTS, LUTS, AE, EXTRACT_ALL, EXTRACT, FIX_NAME, MEDIA, PDF_IMAGES, EXCEL, SOUND, SUBTITLES];

/// One action's offer for these files, whether or not it's the one the slot shows.
pub fn offer_for(id: &str, paths: &[PathBuf], given: Option<&Value>) -> Option<Offer> {
    let files: Vec<PathBuf> = paths.iter().filter(|p| p.is_file()).cloned().collect();
    if files.is_empty() {
        return None;
    }
    one(id, &files, &mut Look::new(), given).map(|x| finish(x, &files))
}

fn finish(mut x: Offer, files: &[PathBuf]) -> Offer {
    x.files = files.to_vec();
    if x.go.is_empty() {
        x.go = x.label.clone();
    }
    x
}

fn one(id: &str, files: &[PathBuf], look: &mut Look, given: Option<&Value>) -> Option<Offer> {
    let o = options(id, given);
    match id {
        JOIN => join_offer(files),
        FONTS => fonts_offer(files, look, &o),
        LUTS => luts_offer(files, look, &o),
        AE => ae_offer(files, look, &o),
        EXTRACT_ALL => extract_all_offer(files, look, &o),
        EXTRACT => one_item_offer(files, look),
        FIX_NAME => fix_name_offer(files),
        MEDIA => media_offer(files),
        PDF_IMAGES => pdf_images_offer(files),
        EXCEL => excel_offer(files),
        SOUND => sound_offer(files, &o),
        SUBTITLES => subtitles_offer(files),
        _ => None,
    }
}

// ---------- fonts ----------

/// One font found in what was picked.
#[derive(Clone)]
struct FontFound {
    /// The archive or font file it came from.
    from: PathBuf,
    file_name: String,
    bytes: Vec<u8>,
    meta: fonts::Meta,
}

/// Every font in `files`, or None when one of them holds no fonts.
fn font_sources(files: &[PathBuf], look: &mut Look) -> Option<Vec<FontFound>> {
    let mut all = Vec::new();
    for f in files {
        let before = all.len();
        if fonts::is_font_name(&name(f)) {
            let bytes = std::fs::read(f).ok()?;
            let meta = fonts::meta(&bytes)?;
            all.push(FontFound { from: f.clone(), file_name: name(f), bytes, meta });
        } else if let Some(l) = look.listing(f) {
            let entries: Vec<Entry> = l.entries.iter().filter(|e| !e.dir && fonts::is_font_name(e.name()) && e.size < 64 << 20).cloned().collect();
            for e in entries {
                let Ok(bytes) = sniff::read(f, &e) else { continue };
                if let Some(meta) = fonts::meta(&bytes) {
                    all.push(FontFound { from: f.clone(), file_name: e.name().to_string(), bytes, meta });
                }
            }
        }
        if all.len() == before {
            return None;
        }
    }
    Some(all)
}

struct FontPlan {
    install: Vec<FontFound>,
    already: Vec<FontFound>,
    /// The other format of a font being installed (TTF copies of OTFs).
    twins: Vec<FontFound>,
    twin_kind: Option<&'static str>,
    /// Web fonts left out, and web fonts turned into desktop fonts to install.
    web_left: usize,
    web_turned: usize,
    /// The same font more than once (in two folders of the pack).
    copies: usize,
    /// Both OTF and TTF of some font are there (the option only shows then).
    has_twins: bool,
    has_web_only: bool,
}

fn font_plan(found: &[FontFound], o: &Value, installed: &HashSet<String>) -> FontPlan {
    let prefer = opt_str(o, "both", "otf");
    let turn_web = opt_str(o, "web", "skip") == "convert";
    let reinstall = opt_bool(o, "reinstall");
    let mut p = FontPlan { install: Vec::new(), already: Vec::new(), twins: Vec::new(), twin_kind: None, web_left: 0, web_turned: 0, copies: 0, has_twins: false, has_web_only: false };
    // Group by full name, in the order found.
    let mut order: Vec<String> = Vec::new();
    let mut groups: HashMap<String, Vec<&FontFound>> = HashMap::new();
    for f in found {
        let k = f.meta.full.to_lowercase();
        if !groups.contains_key(&k) {
            order.push(k.clone());
        }
        groups.entry(k).or_default().push(f);
    }
    for k in order {
        let g = &groups[&k];
        let desktop: Vec<&FontFound> = g.iter().copied().filter(|f| !f.meta.kind.is_web()).collect();
        let web: Vec<&FontFound> = g.iter().copied().filter(|f| f.meta.kind.is_web()).collect();
        let mut pick: Vec<FontFound> = Vec::new();
        if desktop.is_empty() {
            p.has_web_only = true;
            if turn_web {
                // WOFF2 first: the newest and the one most packs are made from.
                let best = web.iter().find(|f| f.meta.kind == fonts::Kind::Woff2).or(web.first()).copied();
                if let Some(w) = best.and_then(|w| {
                    let bytes = fonts::desktop_bytes(&w.bytes).ok()?;
                    let meta = fonts::meta(&bytes)?;
                    let ext = if meta.cff { "otf" } else { "ttf" };
                    let stem = w.file_name.rsplit_once('.').map(|(s, _)| s.to_string()).unwrap_or(w.file_name.clone());
                    Some(FontFound { from: w.from.clone(), file_name: format!("{stem}.{ext}"), bytes, meta: fonts::Meta { kind: if ext == "otf" { fonts::Kind::Otf } else { fonts::Kind::Ttf }, ..meta } })
                }) {
                    pick.push(w);
                    p.web_turned += 1;
                    p.web_left += web.len() - 1;
                } else {
                    p.web_left += web.len();
                }
            } else {
                p.web_left += web.len();
            }
        } else {
            p.web_left += web.len();
            let cff: Vec<&FontFound> = desktop.iter().copied().filter(|f| f.meta.cff).collect();
            let tt: Vec<&FontFound> = desktop.iter().copied().filter(|f| !f.meta.cff).collect();
            let both_kinds = !cff.is_empty() && !tt.is_empty();
            p.has_twins |= both_kinds;
            let chosen: Vec<&FontFound> = if !both_kinds || prefer == "both" {
                // One of each kind (the rest are copies of the same file).
                [cff.first(), tt.first()].into_iter().flatten().copied().collect()
            } else if prefer == "ttf" {
                p.twins.extend(cff.iter().map(|f| (*f).clone()));
                p.twin_kind = Some("OTF");
                vec![tt[0]]
            } else {
                p.twins.extend(tt.iter().map(|f| (*f).clone()));
                p.twin_kind = Some("TTF");
                vec![cff[0]]
            };
            let twins_of_kind = if both_kinds && prefer != "both" { if prefer == "ttf" { cff.len() } else { tt.len() } } else { 0 };
            p.copies += desktop.len() - chosen.len() - twins_of_kind;
            pick.extend(chosen.into_iter().cloned());
        }
        for f in pick {
            let there = f.meta.faces.iter().all(|x| installed.contains(&x.to_lowercase()));
            if there && !reinstall {
                p.already.push(f);
            } else {
                p.install.push(f);
            }
        }
    }
    p
}

fn kind_words(list: &[FontFound]) -> String {
    let mut kinds: Vec<&str> = list.iter().map(|f| if f.meta.cff { "OTF" } else { "TTF" }).collect();
    kinds.sort();
    kinds.dedup();
    kinds.join(" and ")
}

fn fonts_offer(files: &[PathBuf], look: &mut Look, o: &Value) -> Option<Offer> {
    let found = font_sources(files, look)?;
    let installed = fonts::installed_names();
    let p = font_plan(&found, o, &installed);
    let everyone = opt_str(o, "for", "me") == "all";
    let n = p.install.len();
    let mut families: Vec<String> = p.install.iter().chain(p.already.iter()).map(|f| f.meta.family.clone()).collect();
    families.dedup();
    let family = if families.len() == 1 { format!("{} ", families[0]) } else { String::new() };
    let who = if everyone { "for everyone on this PC, one admin prompt" } else { "just for you, no admin prompt" };
    let hint = if n == 0 {
        format!("All {} already installed", plural(p.already.len(), "font is", "fonts are"))
    } else {
        format!("{} {family}{} ({}), {who}", n, if n == 1 { "font" } else { "fonts" }, kind_words(&p.install))
    };
    let archives = files.iter().any(|f| !fonts::is_font_name(&name(f)));
    let mut lines = Vec::new();
    if n > 0 {
        lines.push(if archives {
            format!("Straight into {}: nothing gets unzipped next to the {}", os_fonts_folder(), if files.len() == 1 { "zip" } else { "zips" })
        } else {
            format!("Copied into {}", os_fonts_folder())
        });
    }
    let mut left = Vec::new();
    if !p.already.is_empty() && n > 0 {
        left.push(format!("{} already installed", p.already.len()));
    }
    if !p.twins.is_empty() {
        left.push(format!("{} {} copies of the same fonts", p.twins.len(), p.twin_kind.unwrap_or("other")));
    }
    if p.copies > 0 {
        left.push(plural(p.copies, "duplicate", "duplicates"));
    }
    if p.web_left > 0 {
        left.push(plural(p.web_left, "web font", "web fonts"));
    }
    if !left.is_empty() {
        lines.push(format!("Left out: {}", left.join(" · ")));
    }
    if p.web_turned > 0 {
        lines.push(format!("{} turned into desktop fonts to install", plural(p.web_turned, "web font", "web fonts")));
    }
    let keep = opt_folder(o, "keep");
    if let (Some(k), true) = (&keep, n > 0) {
        lines.push(format!("A copy of each goes to {}", keep_dir(k, &p.install[0]).display()));
    }
    if n == 0 {
        lines.push("Shift+click to install them again".into());
    }
    let mut fields = vec![radio(
        "for",
        "Install for",
        if everyone { "all" } else { "me" },
        &[
            ("me", "Just me", "No admin prompt. Most apps see these."),
            ("all", if cfg!(target_os = "macos") { "Everyone on this Mac" } else { "Everyone on this PC" }, "One admin prompt. For older apps that only see fonts installed for everyone."),
        ],
    )];
    if p.has_twins {
        fields.push(select("both", "Same font as OTF and TTF", opt_str(o, "both", "otf"), &[("otf", "Install the OTF"), ("ttf", "Install the TTF"), ("both", "Install both")]));
    }
    if p.has_web_only {
        fields.push(select("web", "Web fonts (WOFF, WOFF2)", opt_str(o, "web", "skip"), &[("skip", "Leave them out"), ("convert", "Turn into desktop fonts and install")]));
    }
    let already_count = p.install.iter().chain(p.already.iter()).filter(|f| f.meta.faces.iter().all(|x| installed.contains(&x.to_lowercase()))).count();
    if already_count > 0 {
        fields.push(check("reinstall", &format!("Install the {already_count} already installed again"), opt_bool(o, "reinstall"), None));
    }
    fields.push(folder_field("keep", "Also keep the font files in", keep.as_ref().map(|k| k.to_string_lossy().into_owned()).as_deref()));
    let total: usize = found.len();
    Some(Offer {
        id: FONTS.into(),
        label: if n == 1 || (n == 0 && p.already.len() == 1) { "Install font".into() } else { "Install fonts".into() },
        icon: "font".into(),
        sub: if n == 0 { "Installed".into() } else { plural(n, "font", "fonts") },
        hint,
        lines,
        fields,
        go: if n == 0 { "Nothing to install".into() } else { format!("Install {}", plural(n, "font", "fonts")) },
        hub: (files.len() == 1 && archives).then(|| format!("{} inside", plural(total, "font", "fonts"))),
        ..Offer::default()
    })
}

/// Where "keep a copy" puts a font: the chosen folder, in a folder per family.
fn keep_dir(base: &Path, f: &FontFound) -> PathBuf {
    let fam: String = f.meta.family.chars().map(|c| if r#"\/:*?"<>|"#.contains(c) { ' ' } else { c }).collect();
    let fam = fam.trim();
    if fam.is_empty() {
        base.to_path_buf()
    } else {
        base.join(fam)
    }
}

fn run_fonts(step: &Step, o: &Value, progress: &mut dyn FnMut(f64)) -> Result<PathBuf, String> {
    let mut look = Look::new();
    let found = font_sources(&step.inputs, &mut look).ok_or("The fonts are no longer there.")?;
    progress(0.3);
    let p = font_plan(&found, o, &fonts::installed_names());
    let everyone = opt_str(o, "for", "me") == "all";
    let list: Vec<fonts::ToInstall> = p.install.iter().map(|f| fonts::ToInstall { file_name: f.file_name.clone(), bytes: f.bytes.clone(), meta: f.meta.clone() }).collect();
    let done = fonts::install(&list, everyone)?;
    if !done.is_empty() {
        record(Undo::Fonts(done.clone()));
    }
    progress(0.8);
    // Keep a copy of each (the ones installed now).
    let mut kept = Vec::new();
    let mut keep_note = None;
    if let Some(base) = opt_folder(o, "keep").filter(|_| !p.install.is_empty()) {
        for f in &p.install {
            let dir = keep_dir(&base, f);
            if std::fs::create_dir_all(&dir).is_err() {
                keep_note = Some(format!("Couldn't keep copies in {}.", dir.display()));
                break;
            }
            let dest = dir.join(&f.file_name);
            if dest.exists() && std::fs::read(&dest).map(|b| b == f.bytes).unwrap_or(false) {
                continue; // that very file is there already
            }
            let dest = if dest.exists() { convert::unique_output(&dir, &stem_of(&dest), &ext_of(&dest)) } else { dest };
            if std::fs::write(&dest, &f.bytes).is_ok() {
                kept.push(dest);
            }
        }
        if !kept.is_empty() {
            record(Undo::Remove(kept.clone()));
        }
    }
    // The done card.
    let n = done.len();
    let mut families: Vec<String> = p.install.iter().map(|f| f.meta.family.clone()).collect();
    families.dedup();
    let title = if n == 0 {
        "Already installed".to_string()
    } else if step.inputs.len() > 1 && step.inputs.iter().any(|f| !fonts::is_font_name(&name(f))) {
        format!("Installed {} from {}", plural(n, "font", "fonts"), plural(step.inputs.len(), "file", "files"))
    } else {
        format!("Installed {}", plural(n, "font", "fonts"))
    };
    let mut body = Vec::new();
    if n == 0 {
        body.push(format!("All {} already installed.", plural(p.already.len(), "font is", "fonts are")));
    } else {
        let fam = if families.len() == 1 { format!("{} · ", families[0]) } else { String::new() };
        body.push(format!(
            "{fam}{}. Apps opened from now on list them; restart an app that's open if it doesn't.",
            if everyone { "For everyone on this computer" } else { "Just for you (no admin prompt)" }
        ));
    }
    if step.inputs.len() > 1 {
        for f in &step.inputs {
            let ins = p.install.iter().filter(|x| &x.from == f).count();
            let al = p.already.iter().filter(|x| &x.from == f).count();
            let mut parts = vec![format!("{ins} installed")];
            if al > 0 {
                parts.push(format!("{al} already there"));
            }
            body.push(format!("{}: {}", name(f), parts.join(" · ")));
        }
    }
    let mut left = Vec::new();
    if !p.already.is_empty() && n > 0 {
        left.push(format!("{} already installed", p.already.len()));
    }
    if !p.twins.is_empty() {
        left.push(format!("{} {} copies of fonts installed as {}", p.twins.len(), p.twin_kind.unwrap_or("other"), if p.twin_kind == Some("TTF") { "OTF" } else { "TTF" }));
    }
    if p.copies > 0 {
        left.push(plural(p.copies, "duplicate", "duplicates"));
    }
    if p.web_left > 0 {
        left.push(plural(p.web_left, "web font", "web fonts"));
    }
    if !left.is_empty() {
        body.push(format!("Left out: {}", left.join(" · ")));
    }
    if !kept.is_empty() {
        body.push(format!("A copy of each is in {}.", opt_folder(o, "keep").unwrap_or_default().display()));
    }
    if let Some(k) = keep_note {
        body.push(k);
    }
    set_done(title, body.join("\n"), n > 0);
    progress(1.0);
    Ok(done.first().map(|d| d.path.clone()).unwrap_or_else(fonts::user_dir))
}

// ---------- LUTs ----------

fn is_lut(name: &str) -> bool {
    let l = name.to_lowercase();
    l.ends_with(".cube") || l.ends_with(".3dl")
}

/// LUT files in `files` (each from an archive or loose), or None when one has none.
fn lut_sources(files: &[PathBuf], look: &mut Look) -> Option<Vec<(PathBuf, Option<Entry>, String, u64)>> {
    let mut all = Vec::new();
    for f in files {
        let before = all.len();
        if is_lut(&name(f)) {
            all.push((f.clone(), None, name(f), std::fs::metadata(f).map(|m| m.len()).unwrap_or(0)));
        } else if let Some(l) = look.listing(f) {
            for e in l.entries.iter().filter(|e| !e.dir && is_lut(e.name())) {
                all.push((f.clone(), Some(e.clone()), e.name().to_string(), e.size));
            }
        }
        if all.len() == before {
            return None;
        }
    }
    Some(all)
}

/// Adobe's shared folder for this user (CONVERTINO_ADOBE_DIR in tests).
fn adobe_common() -> PathBuf {
    if let Some(d) = std::env::var_os("CONVERTINO_ADOBE_DIR") {
        return PathBuf::from(d);
    }
    if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| fonts::home().join("AppData").join("Roaming")).join("Adobe").join("Common")
    } else {
        fonts::home().join("Library").join("Application Support").join("Adobe").join("Common")
    }
}

fn lut_dir(o: &Value) -> PathBuf {
    match opt_str(o, "where", "creative") {
        "technical" => adobe_common().join("LUTs").join("Technical"),
        "folder" => opt_folder(o, "folder").unwrap_or_else(|| adobe_common().join("LUTs").join("Creative")),
        _ => adobe_common().join("LUTs").join("Creative"),
    }
}

fn luts_offer(files: &[PathBuf], look: &mut Look, o: &Value) -> Option<Offer> {
    let found = lut_sources(files, look)?;
    let dir = lut_dir(o);
    let there = found.iter().filter(|(_, _, n, size)| std::fs::metadata(dir.join(n)).map(|m| m.len() == *size).unwrap_or(false)).count();
    let n = found.len() - there;
    let place = opt_str(o, "where", "creative");
    let hint = match place {
        "technical" => format!("{} into Premiere Pro's input LUTs (Technical), just for you", plural(n, "LUT", "LUTs")),
        "folder" if opt_folder(o, "folder").is_some() => format!("{} copied into {}", plural(n, "LUT", "LUTs"), dir.display()),
        _ => format!("{} into Premiere Pro's Creative looks (Lumetri), just for you", plural(n, "look", "looks")),
    };
    let mut lines = vec![format!("Into {}", dir.display())];
    if place != "folder" {
        lines.push("Restart Premiere Pro to see them".into());
    }
    if there > 0 {
        lines.push(format!("Left out: {} already there", there));
    }
    Some(Offer {
        id: LUTS.into(),
        label: "Install LUTs".into(),
        icon: "install".into(),
        sub: plural(n, if place == "creative" { "look" } else { "LUT" }, if place == "creative" { "looks" } else { "LUTs" }),
        hint,
        lines,
        fields: vec![
            radio(
                "where",
                "Put them in",
                place,
                &[
                    ("creative", "Creative looks", "Lumetri › Creative › Look"),
                    ("technical", "Input LUTs", "Lumetri › Basic Correction › Input LUT"),
                    ("folder", "Another folder", "Copied there, nothing else"),
                ],
            ),
            Field { kind: "path", ..folder_field("folder", "Another folder", opt_folder(o, "folder").map(|f| f.to_string_lossy().into_owned()).as_deref()) },
        ],
        go: if n == 0 { "Nothing to install".into() } else { format!("Install {}", plural(n, "LUT", "LUTs")) },
        hub: (files.len() == 1 && !is_lut(&name(&files[0]))).then(|| format!("{} inside", plural(found.len(), "LUT", "LUTs"))),
        ..Offer::default()
    })
}

/// Copies `items` into `dir` (from archives or loose files): new names for
/// different files with the same name, and files already there skipped.
/// Returns the copies made.
fn copy_items(items: &[(PathBuf, Option<Entry>, String, u64)], dir: &Path) -> Result<(Vec<PathBuf>, usize), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("Couldn't open {}: {e}", dir.display()))?;
    let mut made = Vec::new();
    let mut skipped = 0;
    for (from, entry, file_name, _) in items {
        let bytes = match entry {
            Some(e) => sniff::read(from, e)?,
            None => std::fs::read(from).map_err(|e| format!("Couldn't read {}: {e}", from.display()))?,
        };
        let dest = dir.join(file_name);
        if dest.exists() && std::fs::read(&dest).map(|b| b == bytes).unwrap_or(false) {
            skipped += 1;
            continue;
        }
        let dest = if dest.exists() { convert::unique_output(dir, &stem_of(&dest), &ext_of(&dest)) } else { dest };
        if let Err(e) = std::fs::write(&dest, &bytes) {
            for m in &made {
                let _ = std::fs::remove_file(m);
            }
            return Err(format!("Couldn't write into {}: {e}", dir.display()));
        }
        made.push(dest);
    }
    Ok((made, skipped))
}

fn run_luts(step: &Step, o: &Value) -> Result<PathBuf, String> {
    let mut look = Look::new();
    let found = lut_sources(&step.inputs, &mut look).ok_or("The LUTs are no longer there.")?;
    let dir = lut_dir(o);
    let (made, skipped) = copy_items(&found, &dir)?;
    if !made.is_empty() {
        record(Undo::Remove(made.clone()));
    }
    let mut body = vec![format!("In {}.", dir.display())];
    if opt_str(o, "where", "creative") != "folder" {
        body.push("Restart Premiere Pro to see them.".into());
    }
    if skipped > 0 {
        body.push(format!("{} already there, left as they were.", skipped));
    }
    set_done(if made.is_empty() { "Already installed".into() } else { format!("Installed {}", plural(made.len(), "LUT", "LUTs")) }, body.join("\n"), !made.is_empty());
    Ok(made.first().cloned().unwrap_or(dir))
}

// ---------- After Effects ----------

#[derive(Debug, Clone, PartialEq)]
pub struct AeVersion {
    /// "2025", "CC 2019".
    pub name: String,
    /// The program folder (Windows: …\Adobe After Effects 2025; Mac: /Applications/Adobe After Effects 2025).
    pub root: PathBuf,
}

impl AeVersion {
    fn scripts(&self, panel: bool) -> PathBuf {
        let base = if cfg!(target_os = "macos") { self.root.join("Scripts") } else { self.root.join("Support Files").join("Scripts") };
        if panel {
            base.join("ScriptUI Panels")
        } else {
            base
        }
    }
    fn presets(&self) -> PathBuf {
        documents().join("Adobe").join(format!("After Effects {}", self.name)).join("User Presets")
    }
}

fn documents() -> PathBuf {
    if let Some(d) = std::env::var_os("CONVERTINO_DOCUMENTS") {
        return PathBuf::from(d);
    }
    #[cfg(windows)]
    {
        // Documents may have moved (OneDrive): Explorer's own record says where.
        use std::os::windows::process::CommandExt;
        if let Ok(out) = std::process::Command::new("reg.exe")
            .args(["query", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\Shell Folders", "/v", "Personal"])
            .creation_flags(0x0800_0000)
            .output()
        {
            let text = String::from_utf8_lossy(&out.stdout);
            if let Some(p) = text.lines().find_map(|l| l.split_once("REG_SZ").map(|(_, v)| v.trim().to_string())).filter(|p| !p.is_empty()) {
                return PathBuf::from(p);
            }
        }
    }
    fonts::home().join("Documents")
}

/// After Effects versions installed, newest first.
pub fn ae_versions() -> Vec<AeVersion> {
    let roots: Vec<PathBuf> = if let Some(d) = std::env::var_os("CONVERTINO_AE_ROOT") {
        vec![PathBuf::from(d)]
    } else if cfg!(windows) {
        let pf = std::env::var_os("ProgramFiles").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Program Files"));
        vec![pf.join("Adobe")]
    } else {
        vec![PathBuf::from("/Applications")]
    };
    let mut v = Vec::new();
    for r in roots {
        let Ok(rd) = std::fs::read_dir(&r) else { continue };
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            let Some(ver) = n.strip_prefix("Adobe After Effects ") else { continue };
            if e.path().is_dir() {
                v.push(AeVersion { name: ver.trim().to_string(), root: e.path() });
            }
        }
    }
    // "2025" > "2024" > "CC 2019": by the last number in the name.
    let year = |a: &AeVersion| a.name.split(|c: char| !c.is_ascii_digit()).filter_map(|x| x.parse::<u32>().ok()).last().unwrap_or(0);
    v.sort_by_key(|a| std::cmp::Reverse(year(a)));
    v
}

#[derive(Clone)]
struct AeItem {
    from: PathBuf,
    entry: Option<Entry>,
    file_name: String,
    size: u64,
    /// A preset (.ffx); else a script.
    preset: bool,
    /// A ScriptUI panel (Window menu); else a plain script (File › Scripts).
    panel: bool,
}

fn ae_kind(name: &str) -> Option<bool> {
    let l = name.to_lowercase();
    if l.ends_with(".ffx") {
        Some(true)
    } else if l.ends_with(".jsx") || l.ends_with(".jsxbin") {
        Some(false)
    } else {
        None
    }
}

/// A script is a panel when it sits in a "ScriptUI Panels" folder in the
/// pack, or (a readable .jsx) builds its window on the panel AE gives it.
fn is_panel(path_in_pack: &str, bytes: Option<&[u8]>) -> bool {
    let p = path_in_pack.to_lowercase();
    if p.contains("scriptui panels/") {
        return true;
    }
    if p.contains("scripts/") && !p.contains("scriptui") {
        return false;
    }
    match bytes {
        Some(b) if p.ends_with(".jsx") => {
            let t = String::from_utf8_lossy(b);
            t.contains("instanceof Panel")
        }
        _ => true, // .jsxbin can't be read: most tools sold this way are panels
    }
}

/// A .jsx is an After Effects script only when it talks to After Effects
/// (React components are .jsx files too).
fn is_ae_script(name: &str, bytes: Option<&[u8]>) -> bool {
    if name.to_lowercase().ends_with(".jsxbin") {
        return true;
    }
    let Some(b) = bytes else { return false };
    let t = String::from_utf8_lossy(b);
    ["app.project", "CompItem", "instanceof Panel", "ScriptUI", "#target aftereffects", "#target \"aftereffects", "app.beginUndoGroup", "AVLayer"].iter().any(|k| t.contains(k))
}

fn ae_sources(files: &[PathBuf], look: &mut Look) -> Option<Vec<AeItem>> {
    let mut all = Vec::new();
    for f in files {
        let before = all.len();
        if let Some(preset) = ae_kind(&name(f)) {
            let bytes = if preset || ext_of(f) != "jsx" { None } else { std::fs::read(f).ok() };
            if preset || is_ae_script(&name(f), bytes.as_deref()) {
                all.push(AeItem { from: f.clone(), entry: None, file_name: name(f), size: std::fs::metadata(f).map(|m| m.len()).unwrap_or(0), preset, panel: !preset && is_panel(&name(f), bytes.as_deref()) });
            }
        } else if let Some(l) = look.listing(f) {
            let entries: Vec<Entry> = l.entries.iter().filter(|e| !e.dir && ae_kind(e.name()).is_some()).cloned().collect();
            for e in entries {
                let preset = ae_kind(e.name()) == Some(true);
                let bytes = if !preset && e.ext() == "jsx" && e.size < 8 << 20 { sniff::read(f, &e).ok() } else { None };
                if !preset && !is_ae_script(e.name(), bytes.as_deref()) {
                    continue;
                }
                let panel = !preset && is_panel(&e.path, bytes.as_deref());
                all.push(AeItem { from: f.clone(), file_name: e.name().to_string(), size: e.size, entry: Some(e), preset, panel });
            }
        }
        if all.len() == before {
            return None;
        }
    }
    Some(all)
}

fn ae_offer(files: &[PathBuf], look: &mut Look, o: &Value) -> Option<Offer> {
    let items = ae_sources(files, look)?;
    let versions = ae_versions();
    let presets = items.iter().filter(|i| i.preset).count();
    let panels = items.iter().filter(|i| !i.preset && i.panel).count();
    let scripts = items.len() - presets - panels;
    let mut what = Vec::new();
    if presets > 0 {
        what.push(format!("{} into User Presets", plural(presets, "preset", "presets")));
    }
    if panels > 0 {
        what.push(format!("{} into ScriptUI Panels", plural(panels, "panel", "panels")));
    }
    if scripts > 0 {
        what.push(format!("{} into Scripts", plural(scripts, "script", "scripts")));
    }
    let Some(chosen) = versions.iter().find(|v| v.name == opt_str(o, "version", "")).or(versions.first()).cloned() else {
        return Some(Offer {
            id: AE.into(),
            label: "Install for After Effects".into(),
            icon: "install".into(),
            sub: "Not installed".into(),
            hint: words(&what),
            warn: Some("After Effects isn't installed on this computer".into()),
            blocked: true,
            ..Offer::default()
        });
    };
    let mut lines = Vec::new();
    if panels + scripts > 0 && cfg!(windows) {
        lines.push("Scripts go under Program Files: one admin prompt".into());
    }
    if panels > 0 {
        lines.push("Restart After Effects, then open the panel from the Window menu".into());
    } else if scripts > 0 {
        lines.push("Restart After Effects, then run it from File › Scripts".into());
    } else {
        lines.push("Presets show in Effects & Presets after a restart".into());
    }
    let mut fields = Vec::new();
    if versions.len() > 1 {
        let choices: Vec<(String, String)> = versions.iter().map(|v| (v.name.clone(), format!("After Effects {}", v.name))).collect();
        let refs: Vec<(&str, &str)> = choices.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        fields.push(select("version", "Install into", &chosen.name, &refs));
    }
    Some(Offer {
        id: AE.into(),
        label: "Install for After Effects".into(),
        icon: "install".into(),
        sub: format!("AE {}", chosen.name),
        hint: words(&what),
        lines,
        fields,
        go: format!("Install into After Effects {}", chosen.name),
        ..Offer::default()
    })
}

fn run_ae(step: &Step, o: &Value) -> Result<PathBuf, String> {
    let mut look = Look::new();
    let items = ae_sources(&step.inputs, &mut look).ok_or("The presets and scripts are no longer there.")?;
    let versions = ae_versions();
    let v = versions.iter().find(|v| v.name == opt_str(o, "version", "")).or(versions.first()).cloned().ok_or("After Effects isn't installed on this computer.")?;
    let as_tuple = |i: &AeItem| (i.from.clone(), i.entry.clone(), i.file_name.clone(), i.size);
    let presets: Vec<_> = items.iter().filter(|i| i.preset).map(as_tuple).collect();
    let mut made = Vec::new();
    let mut skipped = 0;
    if !presets.is_empty() {
        let (m, s) = copy_items(&presets, &v.presets())?;
        if !m.is_empty() {
            record(Undo::Remove(m.clone()));
        }
        made.extend(m);
        skipped += s;
    }
    let mut elevated_made = Vec::new();
    for panel in [true, false] {
        let list: Vec<_> = items.iter().filter(|i| !i.preset && i.panel == panel).map(as_tuple).collect();
        if list.is_empty() {
            continue;
        }
        let dir = v.scripts(panel);
        match copy_items(&list, &dir) {
            Ok((m, s)) => {
                if !m.is_empty() {
                    record(Undo::Remove(m.clone()));
                }
                made.extend(m);
                skipped += s;
            }
            Err(_) => {
                // Program Files (or /Applications) needs admin: one prompt for these.
                let (m, s) = copy_as_admin(&list, &dir)?;
                elevated_made.extend(m.clone());
                made.extend(m);
                skipped += s;
            }
        }
    }
    if !elevated_made.is_empty() {
        record(Undo::RemoveAsAdmin(elevated_made));
    }
    let mut body = vec![format!("After Effects {}.", v.name)];
    if items.iter().any(|i| !i.preset) {
        body.push("Restart After Effects to see the scripts.".into());
    }
    if skipped > 0 {
        body.push(format!("{} already there, left as they were.", skipped));
    }
    set_done(if made.is_empty() { "Already installed".into() } else { format!("Installed {} for After Effects", plural(made.len(), "file", "files")) }, body.join("\n"), !made.is_empty());
    Ok(made.first().cloned().unwrap_or_else(|| v.scripts(true)))
}

/// Copies into a folder only an administrator may write, with one prompt.
fn copy_as_admin(items: &[(PathBuf, Option<Entry>, String, u64)], dir: &Path) -> Result<(Vec<PathBuf>, usize), String> {
    let tmp = convert::TempDir::new("admin")?;
    let staged = tmp.0.join("files");
    let (copies, _) = copy_items(items, &staged)?;
    let mut made = Vec::new();
    let mut skipped = 0;
    let mut pairs = Vec::new();
    for c in &copies {
        let bytes = std::fs::read(c).unwrap_or_default();
        let dest = dir.join(c.file_name().unwrap_or_default());
        if dest.exists() && std::fs::read(&dest).map(|b| b == bytes).unwrap_or(false) {
            skipped += 1;
            continue;
        }
        let dest = if dest.exists() { convert::unique_output(dir, &stem_of(&dest), &ext_of(&dest)) } else { dest };
        pairs.push((c.clone(), dest.clone()));
        made.push(dest);
    }
    if pairs.is_empty() {
        return Ok((made, skipped));
    }
    #[cfg(windows)]
    {
        let q = |s: &Path| format!("'{}'", s.to_string_lossy().replace('\'', "''"));
        let mut lines = vec!["$ErrorActionPreference = 'Stop'".to_string(), format!("New-Item -ItemType Directory -Force -Path {} | Out-Null", q(dir))];
        for (from, to) in &pairs {
            lines.push(format!("Copy-Item -LiteralPath {} -Destination {} -Force", q(from), q(to)));
        }
        fonts::win::run_elevated(&tmp.0, &lines)?;
    }
    #[cfg(target_os = "macos")]
    {
        let mut cmds = vec![format!("mkdir -p {}", sh(dir))];
        for (from, to) in &pairs {
            cmds.push(format!("cp {} {}", sh(from), sh(to)));
        }
        mac_admin(&cmds.join(" && "))?;
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = &pairs;
        return Err(format!("Couldn't write into {}.", dir.display()));
    }
    #[allow(unreachable_code)]
    Ok((made, skipped))
}

#[cfg(target_os = "macos")]
fn sh(p: &Path) -> String {
    format!("'{}'", p.to_string_lossy().replace('\'', r"'\''"))
}

/// Runs a shell command as administrator on a Mac (the system's password prompt).
#[cfg(target_os = "macos")]
pub fn mac_admin(cmd: &str) -> Result<(), String> {
    let script = format!("do shell script \"{}\" with administrator privileges", cmd.replace('\\', "\\\\").replace('"', "\\\""));
    let out = std::process::Command::new("osascript").args(["-e", &script]).output().map_err(|e| format!("Couldn't ask for the password: {e}"))?;
    if out.status.success() {
        Ok(())
    } else if String::from_utf8_lossy(&out.stderr).contains("-128") {
        Err("The password prompt was cancelled, so nothing was changed.".into())
    } else {
        Err(format!("macOS didn't make the change: {}", convert::last_lines(&String::from_utf8_lossy(&out.stderr))))
    }
}

fn remove_as_admin(files: &[PathBuf]) -> Result<usize, String> {
    let there: Vec<&PathBuf> = files.iter().filter(|f| f.exists()).collect();
    if there.is_empty() {
        return Ok(0);
    }
    #[cfg(windows)]
    {
        let tmp = convert::TempDir::new("admin")?;
        let mut lines = vec!["$ErrorActionPreference = 'Continue'".to_string()];
        for f in &there {
            lines.push(format!("Remove-Item -LiteralPath '{}' -Force -ErrorAction SilentlyContinue", f.to_string_lossy().replace('\'', "''")));
        }
        fonts::win::run_elevated(&tmp.0, &lines)?;
    }
    #[cfg(target_os = "macos")]
    {
        let cmds: Vec<String> = there.iter().map(|f| format!("rm -f {}", sh(f))).collect();
        mac_admin(&cmds.join(" ; "))?;
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    for f in &there {
        let _ = std::fs::remove_file(f);
    }
    Ok(there.iter().filter(|f| !f.exists()).count())
}

// ---------- archives: every level, one folder, in parts ----------

fn extract_all_offer(files: &[PathBuf], look: &mut Look, o: &Value) -> Option<Offer> {
    let mut nested = 0;
    let mut zips_only = true;
    let mut partial = false;
    for f in files {
        let l = look.listing(f)?;
        if l.nested == 0 {
            nested = 0;
            break;
        }
        nested += l.nested;
        partial |= l.partial;
        zips_only &= l.entries.iter().filter(|e| !e.dir && sniff::is_archive_name(e.name())).all(|e| e.ext() == "zip");
    }
    if nested > 0 {
        let keep = opt_bool(o, "keep");
        let what = if zips_only { plural(nested, "zip", "zips") } else { plural(nested, "archive", "archives") };
        let one = files.len() == 1;
        let mut lines = vec![if keep { "The inner archives are kept next to what they unpack into".to_string() } else { "The inner archives aren't kept once they're unpacked · Shift+click to keep them".to_string() }];
        if partial {
            lines.push("Archives inside those are unpacked too".into());
        }
        if one {
            lines.push(where_extract(look.listing(&files[0])?, &files[0]));
        }
        return Some(Offer {
            id: EXTRACT_ALL.into(),
            label: "Extract all".into(),
            icon: "extract".into(),
            sub: what.clone(),
            hint: if one { format!("Unpacks {} and the {what} inside it, each into its own folder", name(&files[0])) } else { format!("Unpacks the {} and the {what} inside them", plural(files.len(), "archive", "archives")) },
            lines,
            fields: vec![check("keep", "Keep the inner archives too", keep, None)],
            go: "Extract all".into(),
            hub: one.then(|| format!("{what} inside")),
            ..Offer::default()
        });
    }
    None
}

/// One archive holding one folder or one file: Extract already puts that
/// item next to the archive; the slot says so (and takes Extract's place).
fn one_item_offer(files: &[PathBuf], look: &mut Look) -> Option<Offer> {
    let [only] = files else { return None };
    let l = look.listing(only)?;
    let mut tops: Vec<String> = l.top().map(|e| e.path.trim_end_matches('/').split('/').next().unwrap_or("").to_string()).filter(|t| !t.is_empty()).collect();
    tops.sort();
    tops.dedup();
    let [top] = tops.as_slice() else { return None };
    let is_dir = l.top().any(|e| e.path.trim_end_matches('/') != top.as_str() || e.dir);
    // A .tar.gz holding its .tar is the usual case, not one item.
    if !is_dir && sniff::is_archive_name(top) {
        return None;
    }
    let zip_word = if ext_of(only) == "zip" { "zip" } else { "archive" };
    Some(Offer {
        id: EXTRACT.into(),
        label: "Extract".into(),
        icon: "extract".into(),
        sub: if is_dir { "One folder".into() } else { "Just the file".into() },
        hint: if is_dir { format!("Extracts as \"{top}\" next to the {zip_word}") } else { format!("Puts {top} next to the {zip_word}") },
        lines: vec![if is_dir { format!("The folder inside becomes that folder, so no {top}{}{top}", std::path::MAIN_SEPARATOR) } else { "No folder is made for one file".into() }],
        hub: Some(if is_dir { format!("Holds one folder: {top}") } else { format!("Holds one file: {top}") }),
        replaces: Some(EXTRACT),
        ..Offer::default()
    })
}

/// "Into \"Brand Kit\" next to the zip" for a whole archive's extraction.
fn where_extract(l: &Listing, archive: &Path) -> String {
    let mut tops: Vec<&str> = l.top().map(|e| e.path.trim_end_matches('/').split('/').next().unwrap_or("")).filter(|t| !t.is_empty()).collect();
    tops.sort();
    tops.dedup();
    match tops.as_slice() {
        [one] => format!("Into \"{one}\" next to the {}", if ext_of(archive) == "zip" { "zip" } else { "archive" }),
        _ => format!("Into \"{}\" next to the {}", crate::archive::archive_stem(archive), if ext_of(archive) == "zip" { "zip" } else { "archive" }),
    }
}

fn join_offer(files: &[PathBuf]) -> Option<Offer> {
    let (shown, found, missing) = sniff::parts(&files[0])?;
    // Every picked file must be a part of this one set.
    if !files.iter().all(|f| found.contains(f)) || found.len() < 2 {
        return None;
    }
    let picked = sniff::part_of(&files[0]).map(|(_, n, _)| n).unwrap_or(1);
    let mut o = Offer {
        id: JOIN.into(),
        label: "Join and extract".into(),
        icon: "join".into(),
        sub: plural(found.len(), "part", "parts"),
        hint: format!("Reads the {} parts in order and extracts into \"{shown}\"", found.len()),
        lines: vec!["Works from any part you click".into()],
        hub: Some(format!("Part {picked} · {} found", found.len())),
        ..Offer::default()
    };
    if let Some(m) = missing {
        o.blocked = true;
        o.warn = Some(format!("Part {m} is missing, so the parts can't be joined"));
        o.hint = format!("{} parts found, part {m} isn't here", found.len());
        o.lines.clear();
    }
    Some(o)
}

fn run_join(step: &Step, progress: &mut dyn FnMut(f64)) -> Result<PathBuf, String> {
    let (shown, found, missing) = sniff::parts(&step.inputs[0]).ok_or("The parts are no longer there.")?;
    if let Some(m) = missing {
        return Err(format!("Part {m} is missing, so the parts can't be joined."));
    }
    let first = found.first().ok_or("The parts are no longer there.")?;
    let made = crate::archive::extract_named(first, &step.dir, &shown, &mut |p| progress(p * 0.6))?;
    // Parts made by splitting a .zip (rather than by 7-Zip or WinRAR's own
    // volumes) unpack into that .zip: unpack it too.
    if made.is_file() && sniff::is_archive_name(&name(&made)) {
        let inner = crate::archive::extract_named(&made, made.parent().unwrap_or(&step.dir), &shown, &mut |p| progress(0.6 + p * 0.4))?;
        let _ = std::fs::remove_file(&made); // ours: just written
        return Ok(inner);
    }
    Ok(made)
}

fn run_extract_all(step: &Step, o: &Value, progress: &mut dyn FnMut(f64)) -> Result<PathBuf, String> {
    let keep = opt_bool(o, "keep");
    let top = crate::archive::extract(&step.inputs[0], &step.dir, &mut |p| progress(p * 0.4))?;
    let mut unpacked = 0;
    let mut failed = Vec::new();
    // Up to 4 levels deep: each pass unpacks the archives the last one made.
    // Each archive is tried once (a kept or protected one isn't tried again).
    let mut tried: HashSet<PathBuf> = HashSet::new();
    for level in 0..4 {
        let inner: Vec<PathBuf> = archives_in(&top).into_iter().filter(|a| !tried.contains(a)).collect();
        if inner.is_empty() {
            break;
        }
        for (i, a) in inner.iter().enumerate() {
            progress(0.4 + 0.6 * ((level as f64 + i as f64 / inner.len() as f64) / 4.0).min(1.0));
            tried.insert(a.clone());
            let dir = a.parent().unwrap_or(&top).to_path_buf();
            match crate::archive::extract(a, &dir, &mut |_| {}) {
                Ok(made) => {
                    tried.insert(made);
                    unpacked += 1;
                    if !keep {
                        let _ = std::fs::remove_file(a); // a copy that came out of the archive
                    }
                }
                Err(e) => failed.push(format!("{}: {e}", name(a))),
            }
        }
    }
    let mut body = vec![format!("{} · {} unpacked inside it", name(&top), plural(unpacked, "archive", "archives"))];
    if !failed.is_empty() {
        body.push(format!("{} couldn't be unpacked: {}", plural(failed.len(), "archive", "archives"), failed[0]));
    }
    set_done("Extracted every level".into(), body.join("\n"), true);
    Ok(top)
}

/// Archives inside a folder, at any depth.
fn archives_in(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    if dir.is_file() {
        return out;
    }
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if sniff::is_archive_name(&name(&p)) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

// ---------- a wrong extension ----------

fn kind_word(ext: &str) -> &'static str {
    match crate::wheel::family_of(ext).map(|(id, _)| id).as_deref() {
        Some("image") => "picture",
        Some("video") => "video",
        Some("audio") => "sound file",
        Some("pdf") => "PDF",
        Some("doc") => "document",
        Some("data") => "data file",
        Some("archive") => "archive",
        _ => "file",
    }
}

fn fix_name_offer(files: &[PathBuf]) -> Option<Offer> {
    let fixes: Vec<(&PathBuf, &str)> = files.iter().map(|f| sniff::wrong_ext(f).map(|r| (f, r))).collect::<Option<_>>()?;
    let upper = |e: &str| e.to_uppercase();
    if let [(f, real)] = fixes.as_slice() {
        let had = ext_of(f);
        let new_name = format!("{}.{real}", if had.is_empty() { name(f) } else { stem_of(f) });
        return Some(Offer {
            id: FIX_NAME.into(),
            label: "Fix name".into(),
            icon: "tag".into(),
            sub: format!("It's a {}", upper(real)),
            hint: if had.is_empty() { format!("{} has no extension; it's a {} {}", name(f), upper(real), kind_word(real)) } else { format!("{} is really a {} {}", name(f), upper(real), kind_word(real)) },
            lines: vec![format!("Renames it to {new_name}: the same file, nothing re-encoded"), format!("The other slots convert it as the {} it is", upper(real))],
            hub: Some(format!("{} inside · {}", upper(real), crate::engine::human_size(std::fs::metadata(f).map(|m| m.len()).unwrap_or(0)))),
            ..Offer::default()
        });
    }
    let mut kinds: Vec<String> = fixes.iter().map(|(_, r)| upper(r)).collect();
    kinds.sort();
    kinds.dedup();
    Some(Offer {
        id: FIX_NAME.into(),
        label: format!("Fix {} names", fixes.len()),
        icon: "tag".into(),
        sub: kinds.join(", "),
        hint: format!("They're really {}", words(&kinds)),
        lines: vec!["Each gets the extension of what it is: the same files, nothing re-encoded".into()],
        go: format!("Fix {} names", fixes.len()),
        ..Offer::default()
    })
}

fn run_fix_name(step: &Step) -> Result<PathBuf, String> {
    let f = &step.inputs[0];
    let real = sniff::wrong_ext(f).ok_or("Its name already matches what it is.")?;
    let dir = f.parent().unwrap_or(Path::new(".")).to_path_buf();
    let base = if ext_of(f).is_empty() { name(f) } else { stem_of(f) };
    let to = convert::unique_output(&dir, &base, real);
    std::fs::rename(f, &to).map_err(|e| format!("Couldn't rename {}: {e}", name(f)))?;
    record(Undo::Rename { from: f.clone(), to: to.clone() });
    Ok(to)
}

// ---------- pictures and videos inside Office files and PDFs ----------

fn office_media(path: &Path) -> Option<Vec<String>> {
    if !matches!(ext_of(path).as_str(), "docx" | "pptx" | "xlsx") {
        return None;
    }
    let f = std::fs::File::open(path).ok()?;
    let zip = zip::ZipArchive::new(std::io::BufReader::new(f)).ok()?;
    let media: Vec<String> = zip.file_names().filter(|n| ["word/media/", "ppt/media/", "xl/media/"].iter().any(|p| n.starts_with(p)) && !n.ends_with('/')).map(str::to_string).collect();
    (!media.is_empty()).then_some(media)
}

fn is_video_name(n: &str) -> bool {
    let l = n.to_lowercase();
    [".mp4", ".mov", ".m4v", ".wmv", ".avi", ".mpg", ".mpeg", ".webm", ".mkv"].iter().any(|e| l.ends_with(e))
}
fn is_sound_name(n: &str) -> bool {
    let l = n.to_lowercase();
    [".mp3", ".wav", ".m4a", ".wma", ".aac", ".ogg", ".mid"].iter().any(|e| l.ends_with(e))
}

fn media_offer(files: &[PathBuf]) -> Option<Offer> {
    let lists: Vec<Vec<String>> = files.iter().map(|f| office_media(f)).collect::<Option<_>>()?;
    let all: Vec<&String> = lists.iter().flatten().collect();
    let videos = all.iter().filter(|n| is_video_name(n)).count();
    let sounds = all.iter().filter(|n| is_sound_name(n)).count();
    let pictures = all.len() - videos - sounds;
    let mut what = Vec::new();
    if pictures > 0 {
        what.push(plural(pictures, "picture", "pictures"));
    }
    if videos > 0 {
        what.push(plural(videos, "video", "videos"));
    }
    if sounds > 0 {
        what.push(plural(sounds, "sound", "sounds"));
    }
    let app = match ext_of(&files[0]).as_str() {
        "pptx" => "PowerPoint",
        "xlsx" => "Excel",
        _ => "Word",
    };
    let one = files.len() == 1;
    Some(Offer {
        id: MEDIA.into(),
        label: "Extract media".into(),
        icon: "image".into(),
        sub: plural(all.len(), "file", "files"),
        hint: format!("Saves the {} inside at full quality, exactly as they were put in", words(&what)),
        lines: vec![if one { format!("Into \"{} (media)\" next to it · No {app} needed", stem_of(&files[0])) } else { format!("Into a \"(media)\" folder next to each · No {app} needed") }],
        hub: one.then(|| format!("{} inside", words(&what))),
        ..Offer::default()
    })
}

fn run_media(step: &Step, output: &Path) -> Result<PathBuf, String> {
    let input = &step.inputs[0];
    let f = std::fs::File::open(input).map_err(|e| format!("Couldn't read {}: {e}", name(input)))?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(f)).map_err(|e| format!("Couldn't read {}: {e}", name(input)))?;
    let names = office_media(input).ok_or("There's no media inside any more.")?;
    for n in &names {
        let mut e = zip.by_name(n).map_err(|e| e.to_string())?;
        let file = n.rsplit('/').next().unwrap_or(n);
        let mut out = std::fs::File::create(output.join(file)).map_err(|e| format!("Couldn't save {file}: {e}"))?;
        std::io::copy(&mut e, &mut out).map_err(|e| format!("Couldn't save {file}: {e}"))?;
    }
    Ok(output.to_path_buf())
}

fn pdf_images_offer(files: &[PathBuf]) -> Option<Offer> {
    if !files.iter().all(|f| ext_of(f) == "pdf") {
        return None;
    }
    let counts: Vec<usize> = files.iter().map(|f| sniff::pdf_pictures(f).map(|rows| rows.iter().filter(|r| r.kept()).count())).collect::<Option<_>>()?;
    // A logo or two isn't worth a slot: offer it for PDFs with real pictures.
    if counts.iter().any(|c| *c < 2) {
        return None;
    }
    let n: usize = counts.iter().sum();
    let one = files.len() == 1;
    Some(Offer {
        id: PDF_IMAGES.into(),
        label: "Extract images".into(),
        icon: "image".into(),
        sub: plural(n, "picture", "pictures"),
        hint: format!("Saves the {} inside, each as it's stored (a JPG stays a JPG)", plural(n, "picture", "pictures")),
        lines: vec![
            if one { format!("Into \"{} (images)\" next to it", stem_of(&files[0])) } else { "Into an \"(images)\" folder next to each".into() },
            format!("Icons and tiny pieces under {} px are left out", sniff::MIN_PICTURE),
        ],
        hub: one.then(|| format!("{} inside", plural(n, "picture", "pictures"))),
        ..Offer::default()
    })
}

fn run_pdf_images(step: &Step, output: &Path, progress: &mut dyn FnMut(f64)) -> Result<PathBuf, String> {
    let input = &step.inputs[0];
    let exe = crate::tools::require(crate::tools::Tool::Pdfimages)?;
    let list = crate::tools::command(&exe).arg("-list").arg(input).output().map_err(|e| format!("Couldn't start Poppler: {e}"))?;
    let rows = sniff::parse_pdfimages(&String::from_utf8_lossy(&list.stdout));
    progress(0.2);
    let tmp = convert::TempDir::new("pdfimages")?;
    let mut cmd = crate::tools::command(&exe);
    cmd.arg("-all").arg(input).arg(tmp.0.join("img"));
    convert::exec(crate::tools::Tool::Pdfimages, cmd)?;
    progress(0.8);
    let keep: HashSet<usize> = rows.iter().filter(|r| r.kept()).map(|r| r.num).collect();
    let stem = stem_of(input);
    let mut n = 0;
    let mut odd = 0;
    for f in convert::files_in(&tmp.0)? {
        // img-007.jpg: picture number 7.
        let num = stem_of(&f).rsplit('-').next().and_then(|x| x.parse::<usize>().ok());
        if !num.is_some_and(|x| keep.contains(&x)) {
            continue;
        }
        let ext = ext_of(&f);
        if !matches!(ext.as_str(), "jpg" | "png" | "jp2" | "tif") {
            odd += 1; // JBIG2 or CCITT fax pictures: no app opens these
            continue;
        }
        n += 1;
        convert::move_file(&f, &output.join(format!("{stem} {n:03}.{ext}")))?;
    }
    if n == 0 {
        return Err("No pictures could be saved from this PDF.".into());
    }
    if odd > 0 {
        convert::note(format!("{} stored as scanned black-and-white images couldn't be saved as pictures.", plural(odd, "picture", "pictures")));
    }
    Ok(output.to_path_buf())
}

// ---------- CSV for Excel ----------

fn excel_offer(files: &[PathBuf]) -> Option<Offer> {
    let looks: Vec<sniff::CsvLook> = files.iter().map(|f| sniff::csv_look(f)).collect::<Option<_>>()?;
    let sample: Vec<char> = looks.iter().flat_map(|l| l.sample.iter().copied()).take(3).collect();
    let semis = looks.iter().any(|l| l.semicolons);
    let one = files.len() == 1;
    let hint = match (sample.is_empty(), semis) {
        (false, _) => format!("Has {} that Excel would show wrong", sample.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(" ")),
        (true, _) => "Separated by semicolons; Excel expects commas".into(),
    };
    let mut lines = vec![if one { format!("Saves \"{} (Excel).csv\" next to it; the original stays as it is", stem_of(&files[0])) } else { "Saves an \"(Excel)\" copy next to each; the originals stay as they are".into() }];
    if semis && !sample.is_empty() {
        lines.push("Semicolons are switched to commas too".into());
    }
    Some(Offer { id: EXCEL.into(), label: "Fix for Excel".into(), icon: "table".into(), sub: "CSV".into(), hint, lines, ..Offer::default() })
}

fn run_excel(step: &Step, output: &Path) -> Result<PathBuf, String> {
    let input = &step.inputs[0];
    let bytes = std::fs::read(input).map_err(|e| format!("Couldn't read {}: {e}", name(input)))?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let first = text.lines().next().unwrap_or("");
    let semis = first.contains(';') && !first.contains(',') && !first.contains('\t');
    let body = if semis {
        let mut r = csv::ReaderBuilder::new().delimiter(b';').has_headers(false).flexible(true).from_reader(text.as_bytes());
        let mut w = csv::WriterBuilder::new().flexible(true).from_writer(Vec::new());
        for rec in r.records() {
            let rec = rec.map_err(|e| format!("Couldn't read row: {e}"))?;
            w.write_record(&rec).map_err(|e| e.to_string())?;
        }
        w.into_inner().map_err(|e| e.to_string())?
    } else {
        bytes
    };
    std::fs::write(output, [b"\xEF\xBB\xBF".as_slice(), &body].concat()).map_err(|e| format!("Couldn't save {}: {e}", name(output)))?;
    Ok(output.to_path_buf())
}

// ---------- a video's sound tracks and subtitles ----------

/// FFmpeg's description of a video, kept while the file stays the same
/// (the wheel opens on the same clip again and again).
fn tracks_of(p: &Path) -> Option<sniff::Tracks> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, (u64, std::time::SystemTime, sniff::Tracks)>>> = OnceLock::new();
    let m = std::fs::metadata(p).ok()?;
    let stamp = (m.len(), m.modified().ok()?);
    let cache = CACHE.get_or_init(Default::default);
    if let Some((l, t, v)) = cache.lock().ok()?.get(p).cloned() {
        if (l, t) == stamp {
            return Some(v);
        }
    }
    let t = sniff::tracks(p)?;
    if let Ok(mut c) = cache.lock() {
        c.insert(p.to_path_buf(), (stamp.0, stamp.1, t.clone()));
    }
    Some(t)
}

fn videos(files: &[PathBuf]) -> bool {
    files.len() <= 5 && files.iter().all(|f| crate::wheel::family_of(&ext_of(f)).map(|(id, _)| id == "video").unwrap_or(false))
}

fn clean(s: &str) -> String {
    let c: String = s.chars().map(|c| if r#"\/:*?"<>|"#.contains(c) || c.is_control() { ' ' } else { c }).collect();
    c.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn track_name(t: &sniff::Track) -> String {
    t.title.as_deref().map(clean).filter(|s| !s.is_empty()).unwrap_or_else(|| format!("Track {}", t.index + 1))
}

/// Track names, made unique ("Mic", "Mic 2").
fn track_names(list: &[sniff::Track]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in list {
        let base = track_name(t);
        let mut n = base.clone();
        let mut k = 2;
        while out.contains(&n) {
            n = format!("{base} {k}");
            k += 1;
        }
        out.push(n);
    }
    out
}

fn sound_offer(files: &[PathBuf], o: &Value) -> Option<Offer> {
    if !videos(files) {
        return None;
    }
    let all: Vec<sniff::Tracks> = files.iter().map(|f| tracks_of(f)).collect::<Option<_>>()?;
    if all.iter().any(|t| t.audio.len() < 2) {
        return None;
    }
    let mix = opt_str(o, "mode", "split") == "mix";
    let one = files.len() == 1;
    let names = track_names(&all[0].audio);
    let n: usize = all.iter().map(|t| t.audio.len()).sum();
    let ext = ext_of(&files[0]);
    let (hint, lines) = if mix {
        (
            format!("Saves a copy with the {} mixed into one, for phones and Discord", plural(all[0].audio.len(), "sound track", "sound tracks")),
            vec![if one { format!("\"{} (mixed).{ext}\" next to it · the picture is copied, not re-encoded", stem_of(&files[0])) } else { "A \"(mixed)\" copy next to each · the picture is copied, not re-encoded".into() }],
        )
    } else {
        (
            if one { format!("Saves {} as separate WAVs next to the video", words(&names.iter().map(|x| format!("\"{x}\"")).collect::<Vec<_>>())) } else { format!("Saves the {n} sound tracks as separate WAVs next to each video") },
            vec!["The video isn't touched · Shift+click: one copy with the tracks mixed".into()],
        )
    };
    Some(Offer {
        id: SOUND.into(),
        label: "Split sound".into(),
        icon: "split".into(),
        sub: if mix { "Mixed".into() } else { plural(n, "track", "tracks") },
        hint,
        lines,
        fields: vec![radio(
            "mode",
            "Make",
            if mix { "mix" } else { "split" },
            &[("split", "A WAV per track", "For editing: each track on its own"), ("mix", "One copy with the tracks mixed", "For phones, Discord and WhatsApp, which play only the first track")],
        )],
        go: if mix { "Mix the sound".into() } else { format!("Save {}", plural(n, "track", "tracks")) },
        hub: one.then(|| format!("{} sound tracks", all[0].audio.len())),
        ..Offer::default()
    })
}

/// Two-letter codes for the common three-letter ones ("eng" -> "en").
fn lang2(code: &str) -> String {
    let m = [
        ("eng", "en"), ("hin", "hi"), ("spa", "es"), ("fra", "fr"), ("fre", "fr"), ("deu", "de"), ("ger", "de"), ("ita", "it"), ("jpn", "ja"), ("kor", "ko"), ("zho", "zh"), ("chi", "zh"),
        ("por", "pt"), ("rus", "ru"), ("ara", "ar"), ("tur", "tr"), ("nld", "nl"), ("dut", "nl"), ("ben", "bn"), ("tam", "ta"), ("tel", "te"), ("mar", "mr"), ("urd", "ur"), ("guj", "gu"),
        ("kan", "kn"), ("mal", "ml"), ("pan", "pa"), ("pol", "pl"), ("swe", "sv"), ("ind", "id"), ("tha", "th"), ("vie", "vi"),
    ];
    m.iter().find(|(a, _)| *a == code).map(|(_, b)| b.to_string()).unwrap_or_else(|| code.to_string())
}

/// "Interview.en" for each text subtitle (made unique), in order.
fn subtitle_stems(video: &Path, list: &[sniff::Track]) -> Vec<(usize, String)> {
    let stem = stem_of(video);
    let mut out: Vec<(usize, String)> = Vec::new();
    for t in list.iter().filter(|t| sniff::text_subtitle(&t.codec)) {
        let tag = t.language.as_deref().map(lang2).unwrap_or_else(|| format!("track{}", t.index + 1));
        let mut s = format!("{stem}.{tag}");
        let mut k = 2;
        while out.iter().any(|(_, x)| *x == s) {
            s = format!("{stem}.{tag}{k}");
            k += 1;
        }
        out.push((t.index, s));
    }
    out
}

fn subtitles_offer(files: &[PathBuf]) -> Option<Offer> {
    if !videos(files) {
        return None;
    }
    let all: Vec<sniff::Tracks> = files.iter().map(|f| tracks_of(f)).collect::<Option<_>>()?;
    let text: usize = all.iter().map(|t| t.subtitles.iter().filter(|s| sniff::text_subtitle(&s.codec)).count()).sum();
    if all.iter().any(|t| !t.subtitles.iter().any(|s| sniff::text_subtitle(&s.codec))) {
        return None;
    }
    let one = files.len() == 1;
    let mut lines = Vec::new();
    let names: Vec<String> = subtitle_stems(&files[0], &all[0].subtitles).into_iter().map(|(_, s)| format!("\"{s}.srt\"")).collect();
    let pictures: Vec<String> = all.iter().flat_map(|t| t.subtitles.iter().filter(|s| !sniff::text_subtitle(&s.codec))).map(|s| s.language.as_deref().map(lang_name).unwrap_or("one").to_string()).collect();
    if !pictures.is_empty() {
        lines.push(format!("Picture subtitles can't become SRT: {} (Blu-ray or DVD style)", words(&pictures)));
    }
    lines.push("Ready for QuickCaptions, YouTube or any player".into());
    Some(Offer {
        id: SUBTITLES.into(),
        label: "Save subtitles".into(),
        icon: "subtitles".into(),
        sub: format!("{text} inside"),
        hint: if one { format!("Saves {} next to the video", words(&names)) } else { format!("Saves the {} as SRT next to each video", plural(text, "subtitle track", "subtitle tracks")) },
        lines,
        hub: one.then(|| plural(text, "subtitle track", "subtitle tracks")),
        ..Offer::default()
    })
}

fn lang_name(code: &str) -> &'static str {
    match code {
        "eng" => "English",
        "hin" => "Hindi",
        "spa" => "Spanish",
        "fra" | "fre" => "French",
        "deu" | "ger" => "German",
        "jpn" => "Japanese",
        "kor" => "Korean",
        "zho" | "chi" => "Chinese",
        "tam" => "Tamil",
        "tel" => "Telugu",
        _ => "another language",
    }
}

// ---------- planning ----------

fn step(inputs: Vec<PathBuf>, dir: PathBuf, stem: String, ext: &str, folder: bool, op: SmartOp) -> Step {
    Step { inputs, dir, stem, ext: ext.into(), folder, op: Op::Smart(Box::new(op)) }
}

/// What a smart step does (convert.rs runs it through `run`).
#[derive(Debug, Clone)]
pub enum SmartOp {
    Fonts(Value),
    ExtractAll(Value),
    Join,
    FixName,
    Media,
    PdfImages,
    Excel,
    Track { index: usize, pcm: &'static str },
    Mix { tracks: usize },
    Subtitle { index: usize },
    Luts(Value),
    Ae(Value),
}

impl SmartOp {
    /// It writes where it must itself (a fonts folder, an app's folder, the
    /// archive's folder), not to the output path convert.rs reserves.
    pub fn writes_itself(&self) -> bool {
        matches!(self, SmartOp::Fonts(_) | SmartOp::ExtractAll(_) | SmartOp::Join | SmartOp::FixName | SmartOp::Luts(_) | SmartOp::Ae(_))
    }
}

/// The steps for a smart action on `files`, with `given` options (None: Settings').
pub fn plan(id: &str, files: &[PathBuf], given: Option<&Value>) -> Result<Vec<Step>, String> {
    let o = options(id, given);
    let dir_of = |f: &PathBuf| f.parent().map(Path::to_path_buf).unwrap_or_default();
    let first = files.first().ok_or("There was nothing to work on.")?;
    let Some(offer) = offer_for(id, files, given) else {
        return Err(match id {
            FONTS => "There are no fonts in this.".into(),
            EXTRACT_ALL => "There are no archives inside this one.".into(),
            JOIN => "These aren't the parts of one archive.".into(),
            FIX_NAME => "The name already matches what it is.".into(),
            MEDIA => "There are no pictures or videos inside.".into(),
            PDF_IMAGES => "There are no pictures worth saving in this PDF.".into(),
            EXCEL => "Excel already reads this CSV correctly.".into(),
            SOUND => "This video has only one sound track.".into(),
            SUBTITLES => "There are no subtitles stored as text in this video.".into(),
            LUTS => "There are no LUTs (.cube, .3dl) in this.".into(),
            AE => "There are no After Effects presets or scripts in this.".into(),
            _ => format!("Convertino doesn't know \"{id}\"."),
        });
    };
    if offer.blocked {
        return Err(offer.warn.unwrap_or_else(|| "This can't be done.".into()));
    }
    Ok(match id {
        FONTS => vec![step(files.to_vec(), dir_of(first), "fonts".into(), "", false, SmartOp::Fonts(o))],
        LUTS => vec![step(files.to_vec(), dir_of(first), "luts".into(), "", false, SmartOp::Luts(o))],
        AE => vec![step(files.to_vec(), dir_of(first), "ae".into(), "", false, SmartOp::Ae(o))],
        JOIN => {
            let (shown, found, _) = sniff::parts(first).ok_or("The parts are no longer there.")?;
            vec![step(found, dir_of(first), shown, "", false, SmartOp::Join)]
        }
        EXTRACT_ALL => files.iter().map(|f| step(vec![f.clone()], dir_of(f), crate::archive::archive_stem(f), "", false, SmartOp::ExtractAll(o.clone()))).collect(),
        FIX_NAME => files.iter().map(|f| step(vec![f.clone()], dir_of(f), stem_of(f), "", false, SmartOp::FixName)).collect(),
        MEDIA => files.iter().map(|f| step(vec![f.clone()], dir_of(f), format!("{} (media)", stem_of(f)), "", true, SmartOp::Media)).collect(),
        PDF_IMAGES => files.iter().map(|f| step(vec![f.clone()], dir_of(f), format!("{} (images)", stem_of(f)), "", true, SmartOp::PdfImages)).collect(),
        EXCEL => files.iter().map(|f| step(vec![f.clone()], dir_of(f), format!("{} (Excel)", stem_of(f)), "csv", false, SmartOp::Excel)).collect(),
        SOUND => {
            let mut steps = Vec::new();
            for f in files {
                let t = tracks_of(f).ok_or("FFmpeg couldn't read the video.")?;
                if opt_str(&o, "mode", "split") == "mix" {
                    steps.push(step(vec![f.clone()], dir_of(f), format!("{} (mixed)", stem_of(f)), &ext_of(f), false, SmartOp::Mix { tracks: t.audio.len() }));
                } else {
                    for (tr, n) in t.audio.iter().zip(track_names(&t.audio)) {
                        let pcm = if tr.codec.contains("24") || tr.codec.contains("32") || tr.codec == "flac" { "pcm_s24le" } else { "pcm_s16le" };
                        steps.push(step(vec![f.clone()], dir_of(f), format!("{} - {n}", stem_of(f)), "wav", false, SmartOp::Track { index: tr.index, pcm }));
                    }
                }
            }
            steps
        }
        SUBTITLES => {
            let mut steps = Vec::new();
            for f in files {
                let t = tracks_of(f).ok_or("FFmpeg couldn't read the video.")?;
                for (index, stem) in subtitle_stems(f, &t.subtitles) {
                    steps.push(step(vec![f.clone()], dir_of(f), stem, "srt", false, SmartOp::Subtitle { index }));
                }
            }
            steps
        }
        _ => return Err(format!("Convertino doesn't know \"{id}\".")),
    })
}

/// Runs one smart step (from convert::run).
pub fn run(op: &SmartOp, step: &Step, output: &Path, progress: &mut dyn FnMut(f64)) -> Result<PathBuf, String> {
    let input = &step.inputs[0];
    match op {
        SmartOp::Fonts(o) => run_fonts(step, o, progress),
        SmartOp::Luts(o) => run_luts(step, o),
        SmartOp::Ae(o) => run_ae(step, o),
        SmartOp::Join => run_join(step, progress),
        SmartOp::ExtractAll(o) => run_extract_all(step, o, progress),
        SmartOp::FixName => run_fix_name(step),
        SmartOp::Media => run_media(step, output),
        SmartOp::PdfImages => run_pdf_images(step, output, progress),
        SmartOp::Excel => run_excel(step, output),
        SmartOp::Track { index, pcm } => {
            let args = convert::s(&["-map", &format!("0:a:{index}"), "-vn", "-sn", "-c:a", pcm]);
            convert::run_ffmpeg(input, &args, output, progress)?;
            Ok(output.to_path_buf())
        }
        SmartOp::Mix { tracks } => {
            let inputs: String = (0..*tracks).map(|i| format!("[0:a:{i}]")).collect();
            let args = convert::s(&[
                "-filter_complex",
                &format!("{inputs}amix=inputs={tracks}:duration=longest:normalize=0[a]"),
                "-map",
                "0:v?",
                "-map",
                "[a]",
                "-c:v",
                "copy",
                "-c:a",
                "aac",
                "-b:a",
                "192k",
            ]);
            convert::run_ffmpeg(input, &args, output, progress)?;
            Ok(output.to_path_buf())
        }
        SmartOp::Subtitle { index } => {
            let args = convert::s(&["-map", &format!("0:s:{index}"), "-c:s", "srt"]);
            convert::run_ffmpeg(input, &args, output, progress)?;
            Ok(output.to_path_buf())
        }
    }
}

// ---------- the done card and Undo ----------

fn done_map() -> &'static Mutex<HashMap<u64, (String, String, bool)>> {
    static M: OnceLock<Mutex<HashMap<u64, (String, String, bool)>>> = OnceLock::new();
    M.get_or_init(Default::default)
}

fn set_done(title: String, body: String, changed: bool) {
    if let Ok(mut m) = done_map().lock() {
        m.insert(crate::procs::current_job(), (title, body, changed));
    }
}

/// The done card's title and body an action wrote, and whether it changed anything.
pub fn take_done(job: u64) -> Option<(String, String, bool)> {
    done_map().lock().ok()?.remove(&job)
}

#[derive(Debug, Clone)]
enum Undo {
    Fonts(Vec<fonts::Installed>),
    Rename { from: PathBuf, to: PathBuf },
    /// Copies this job made (LUTs, presets, kept font files).
    Remove(Vec<PathBuf>),
    RemoveAsAdmin(Vec<PathBuf>),
}

fn undo_map() -> &'static Mutex<HashMap<u64, Vec<Undo>>> {
    static M: OnceLock<Mutex<HashMap<u64, Vec<Undo>>>> = OnceLock::new();
    M.get_or_init(Default::default)
}

fn record(u: Undo) {
    if let Ok(mut m) = undo_map().lock() {
        m.entry(crate::procs::current_job()).or_default().push(u);
    }
}

/// Actions whose Undo isn't "move the results to the bin".
pub fn own_undo(id: &str) -> bool {
    matches!(id, FONTS | LUTS | AE | FIX_NAME)
}

/// Undoes what a smart job did; None when it kept no record (nothing of its own to undo).
pub fn undo(job: u64) -> Option<Result<usize, String>> {
    let list = undo_map().lock().ok()?.remove(&job)?;
    let mut n = 0;
    for u in list.into_iter().rev() {
        let r = match u {
            Undo::Fonts(f) => fonts::uninstall(&f),
            Undo::Rename { from, to } => {
                if from.exists() {
                    Err(format!("{} is back already, so {} keeps its name.", name(&from), name(&to)))
                } else {
                    std::fs::rename(&to, &from).map(|_| 1).map_err(|e| format!("Couldn't rename {} back: {e}", name(&to)))
                }
            }
            Undo::Remove(files) => {
                let there: Vec<&PathBuf> = files.iter().filter(|f| f.exists()).collect();
                if there.is_empty() {
                    Ok(0)
                } else {
                    trash::delete_all(&there).map(|_| there.len()).map_err(|e| format!("Couldn't move to the Recycle Bin: {e}"))
                }
            }
            Undo::RemoveAsAdmin(files) => remove_as_admin(&files),
        };
        match r {
            Ok(k) => n += k,
            Err(e) => return Some(Err(e)),
        }
    }
    Some(Ok(n))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("convertino-smart-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn make_zip(path: &Path, files: &[(&str, &[u8])]) {
        let f = std::fs::File::create(path).unwrap();
        let mut z = zip::ZipWriter::new(f);
        for (n, data) in files {
            z.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
            std::io::Write::write_all(&mut z, data).unwrap();
        }
        z.finish().unwrap();
    }

    fn noto() -> Vec<u8> {
        std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../ui/vendor/fonts/NotoSans-Regular.ttf")).unwrap()
    }

    /// The same font under another name (so it isn't "the same font twice").
    fn renamed_font(full: &str) -> Vec<u8> {
        // Rewrite every name record's text to `full`: enough for the tests'
        // purposes (each family/full name differs).
        let ttf = noto();
        let face = ttf_parser::Face::parse(&ttf, 0).unwrap();
        let _ = face; // proves it parses
        let mut out = ttf.clone();
        let needle: Vec<u8> = "Noto Sans".encode_utf16().flat_map(|c| c.to_be_bytes()).collect();
        let repl: Vec<u8> = full.chars().chain(std::iter::repeat(' ')).take(9).collect::<String>().encode_utf16().flat_map(|c| c.to_be_bytes()).collect();
        let ascii_needle = b"Noto Sans".to_vec();
        let ascii_repl: Vec<u8> = full.bytes().chain(std::iter::repeat(b' ')).take(9).collect();
        for (needle, repl) in [(needle, repl), (ascii_needle, ascii_repl)] {
            let mut i = 0;
            while i + needle.len() <= out.len() {
                if out[i..i + needle.len()] == needle[..] {
                    out[i..i + needle.len()].copy_from_slice(&repl);
                    i += needle.len();
                } else {
                    i += 1;
                }
            }
        }
        out
    }

    #[test]
    fn fonts_zip_counts_match_what_installs_and_undo_takes_them_out() {
        let _lock = fonts::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let d = dir("fonts");
        let fonts_dir = d.join("installed");
        std::env::set_var("CONVERTINO_FONTS_DIR", &fonts_dir);
        let a = renamed_font("Alphaxxxx");
        let b = renamed_font("Betaxxxxx");
        let web = fonts::to_woff2(&renamed_font("Gammaxxxx")).unwrap();
        let twin = fonts::to_woff2(&a).unwrap(); // a web copy of a desktop font: left out
        let zip = d.join("Pack.zip");
        make_zip(&zip, &[("Pack/Alpha.ttf", &a), ("Pack/Static/Alpha.ttf", &a), ("Pack/Beta.ttf", &b), ("Web/Gamma.woff2", &web), ("Web/Alpha.woff2", &twin), ("License.txt", b"x")]);
        let o = offer(&[zip.clone()]).unwrap();
        assert_eq!(o.id, FONTS);
        assert_eq!(o.sub, "2 fonts");
        assert_eq!(o.go, "Install 2 fonts");
        assert!(o.lines.iter().any(|l| l == "Left out: 1 duplicate · 2 web fonts"), "{:?}", o.lines);
        assert_eq!(o.hub.as_deref(), Some("5 fonts inside"));
        // Shift+click: turn the web-only font into a desktop font too.
        let three = offer_with(&[zip.clone()], Some(&json!({"web": "convert"}))).unwrap();
        assert_eq!(three.go, "Install 3 fonts");

        crate::procs::set_current_job(9001);
        let steps = plan(FONTS, &[zip.clone()], None).unwrap();
        assert_eq!(steps.len(), 1);
        let made = convert::run(&steps[0], &mut |_| {}).unwrap();
        assert!(made.starts_with(&fonts_dir));
        let installed: Vec<_> = std::fs::read_dir(&fonts_dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(installed.len(), 2, "{installed:?}");
        let (title, body, _) = take_done(9001).unwrap();
        assert_eq!(title, "Installed 2 fonts");
        assert!(body.contains("Left out: 1 duplicate · 2 web fonts"), "{body}");
        // Now they're installed: the slot says so, and nothing would install.
        let again = offer(&[zip.clone()]).unwrap();
        assert_eq!((again.sub.as_str(), again.go.as_str()), ("Installed", "Nothing to install"));
        assert_eq!(undo(9001).unwrap().unwrap(), 2);
        assert_eq!(std::fs::read_dir(&fonts_dir).unwrap().count(), 0);
        std::env::remove_var("CONVERTINO_FONTS_DIR");
    }

    #[test]
    fn one_folder_inside_says_so_and_takes_extracts_place() {
        let d = dir("one");
        let zip = d.join("Brand Kit.zip");
        make_zip(&zip, &[("Brand Kit/logo.svg", b"<svg/>"), ("Brand Kit/colors.txt", b"x")]);
        let o = offer(&[zip.clone()]).unwrap();
        assert_eq!((o.id.as_str(), o.sub.as_str()), (EXTRACT, "One folder"));
        assert_eq!(o.hint, "Extracts as \"Brand Kit\" next to the zip");
        assert_eq!(o.replaces, Some(EXTRACT));
        let single = d.join("logo-final.zip");
        make_zip(&single, &[("logo-final.svg", b"<svg/>")]);
        let o = offer(&[single]).unwrap();
        assert_eq!((o.sub.as_str(), o.hint.as_str()), ("Just the file", "Puts logo-final.svg next to the zip"));
        let many = d.join("Loose.zip");
        make_zip(&many, &[("a.txt", b"a"), ("b.txt", b"b")]);
        assert!(offer(&[many]).is_none(), "a plain archive keeps the plain wheel");
    }

    #[test]
    fn extract_all_unpacks_every_level() {
        if crate::tools::find(crate::tools::Tool::SevenZip).is_none() {
            eprintln!("7-Zip isn't installed here; skipped");
            return;
        }
        let d = dir("all");
        let inner = d.join("inner.zip");
        make_zip(&inner, &[("deep.txt", b"deep")]);
        let mid = d.join("mid.zip");
        make_zip(&mid, &[("inner.zip", &std::fs::read(&inner).unwrap()), ("mid.txt", b"m")]);
        let pack = d.join("Envato Pack.zip");
        make_zip(&pack, &[("mid.zip", &std::fs::read(&mid).unwrap()), ("readme.txt", b"r")]);
        std::fs::remove_file(&inner).unwrap();
        std::fs::remove_file(&mid).unwrap();
        let o = offer(&[pack.clone()]).unwrap();
        assert_eq!((o.id.as_str(), o.sub.as_str()), (EXTRACT_ALL, "2 zips"));
        crate::procs::set_current_job(9002);
        let steps = plan(EXTRACT_ALL, &[pack.clone()], None).unwrap();
        let top = convert::run(&steps[0], &mut |_| {}).unwrap();
        assert_eq!(top, d.join("Envato Pack"));
        assert!(top.join("mid").join("inner").join("deep.txt").exists() || top.join("mid").join("deep.txt").exists(), "{:?}", std::fs::read_dir(&top).unwrap().flatten().map(|e| e.path()).collect::<Vec<_>>());
        assert!(archives_in(&top).is_empty(), "inner archives aren't kept");
        let (_, body, _) = take_done(9002).unwrap();
        assert!(body.contains("2 archives unpacked inside it"), "{body}");
    }

    #[test]
    fn joins_split_zips() {
        if crate::tools::find(crate::tools::Tool::SevenZip).is_none() {
            eprintln!("7-Zip isn't installed here; skipped");
            return;
        }
        let d = dir("join");
        let zip = d.join("Footage.zip");
        // Bytes that don't compress, so the zip really needs 4 parts.
        let mut x: u32 = 2463534242;
        let big: Vec<u8> = (0..300_000)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x >> 24) as u8
            })
            .collect();
        make_zip(&zip, &[("Footage/clip.bin", &big), ("Footage/notes.txt", b"n")]);
        // Split the zip into raw pieces, as 7-Zip's -v does for a zip.
        let bytes = std::fs::read(&zip).unwrap();
        std::fs::remove_file(&zip).unwrap();
        for (i, chunk) in bytes.chunks(100_000).enumerate() {
            std::fs::write(d.join(format!("Footage.zip.{:03}", i + 1)), chunk).unwrap();
        }
        let part3 = d.join("Footage.zip.003");
        let o = offer(&[part3.clone()]).unwrap();
        assert_eq!((o.id.as_str(), o.sub.as_str(), o.blocked), (JOIN, "4 parts", false));
        assert_eq!(o.hub.as_deref(), Some("Part 3 · 4 found"));
        let steps = plan(JOIN, &[part3.clone()], None).unwrap();
        let made = convert::run(&steps[0], &mut |_| {}).unwrap();
        assert_eq!(made, d.join("Footage"));
        assert_eq!(std::fs::read(made.join("clip.bin")).unwrap(), big);
        assert!(!d.join("Footage.zip").exists(), "the joined zip in between isn't left behind");
        // A missing part: greyed out, says which.
        std::fs::remove_file(d.join("Footage.zip.002")).unwrap();
        let o = offer(&[part3]).unwrap();
        assert!(o.blocked);
        assert_eq!(o.warn.as_deref(), Some("Part 2 is missing, so the parts can't be joined"));
    }

    #[test]
    fn fixes_a_wrong_extension_and_undo_renames_it_back() {
        let d = dir("fix");
        let f = d.join("hero-banner.jpg");
        std::fs::write(&f, b"RIFF\x10\x00\x00\x00WEBPVP8 data").unwrap();
        let o = offer(&[f.clone()]).unwrap();
        assert_eq!((o.id.as_str(), o.sub.as_str()), (FIX_NAME, "It's a WEBP"));
        assert_eq!(o.hint, "hero-banner.jpg is really a WEBP picture");
        assert_eq!(o.lines[0], "Renames it to hero-banner.webp: the same file, nothing re-encoded");
        crate::procs::set_current_job(9003);
        let steps = plan(FIX_NAME, &[f.clone()], None).unwrap();
        let made = convert::run(&steps[0], &mut |_| {}).unwrap();
        assert_eq!(made, d.join("hero-banner.webp"));
        assert!(!f.exists());
        assert_eq!(undo(9003).unwrap().unwrap(), 1);
        assert!(f.exists() && !made.exists());
    }

    #[test]
    fn office_media_and_excel() {
        let d = dir("office");
        let deck = d.join("Pitch Deck.pptx");
        make_zip(&deck, &[("[Content_Types].xml", b"x"), ("ppt/slides/slide1.xml", b"x"), ("ppt/media/image1.png", b"\x89PNG"), ("ppt/media/image2.jpeg", b"\xFF\xD8\xFF"), ("ppt/media/media1.mp4", b"v")]);
        let o = offer(&[deck.clone()]).unwrap();
        assert_eq!((o.id.as_str(), o.sub.as_str()), (MEDIA, "3 files"));
        assert_eq!(o.hint, "Saves the 2 pictures and 1 video inside at full quality, exactly as they were put in");
        let steps = plan(MEDIA, &[deck.clone()], None).unwrap();
        let out = convert::run(&steps[0], &mut |_| {}).unwrap();
        assert_eq!(out, d.join("Pitch Deck (media)"));
        assert_eq!(std::fs::read_dir(&out).unwrap().count(), 3);

        let csv = d.join("orders.csv");
        std::fs::write(&csv, "name;city\nRavi;कानपुर\n").unwrap();
        let o = offer(&[csv.clone()]).unwrap();
        assert_eq!(o.id, EXCEL);
        assert!(o.lines.iter().any(|l| l == "Semicolons are switched to commas too"));
        let steps = plan(EXCEL, &[csv.clone()], None).unwrap();
        let out = convert::run(&steps[0], &mut |_| {}).unwrap();
        assert_eq!(out, d.join("orders (Excel).csv"));
        assert_eq!(std::fs::read(&out).unwrap(), "\u{FEFF}name,city\nRavi,कानपुर\n".as_bytes());
    }

    #[test]
    fn luts_and_after_effects_go_into_their_folders() {
        let d = dir("adobe");
        std::env::set_var("CONVERTINO_ADOBE_DIR", d.join("Common"));
        let pack = d.join("Cinematic LUTs.zip");
        make_zip(&pack, &[("LUTs/Teal.cube", b"LUT_3D_SIZE 2"), ("LUTs/Warm.CUBE", b"LUT_3D_SIZE 2"), ("Preview.jpg", b"x")]);
        let o = offer(&[pack.clone()]).unwrap();
        assert_eq!((o.id.as_str(), o.sub.as_str(), o.go.as_str()), (LUTS, "2 looks", "Install 2 LUTs"));
        crate::procs::set_current_job(9004);
        let steps = plan(LUTS, &[pack.clone()], None).unwrap();
        convert::run(&steps[0], &mut |_| {}).unwrap();
        let dest = d.join("Common").join("LUTs").join("Creative");
        assert!(dest.join("Teal.cube").exists() && dest.join("Warm.CUBE").exists());
        let o = offer(&[pack.clone()]).unwrap();
        assert!(o.lines.iter().any(|l| l == "Left out: 2 already there"), "{:?}", o.lines);
        std::env::remove_var("CONVERTINO_ADOBE_DIR");

        std::env::set_var("CONVERTINO_AE_ROOT", d.join("Programs"));
        std::env::set_var("CONVERTINO_DOCUMENTS", d.join("Docs"));
        let ae_pack = d.join("Text Animator.zip");
        make_zip(&ae_pack, &[("Presets/Pop.ffx", b"ffx"), ("Text Animator.jsx", b"var w = (this instanceof Panel) ? this : new Window('palette');")]);
        let o = offer(&[ae_pack.clone()]).unwrap();
        assert!(o.blocked, "no After Effects installed");
        std::fs::create_dir_all(d.join("Programs").join("Adobe After Effects 2024")).unwrap();
        std::fs::create_dir_all(d.join("Programs").join("Adobe After Effects 2025")).unwrap();
        let o = offer(&[ae_pack.clone()]).unwrap();
        assert_eq!((o.sub.as_str(), o.hint.as_str()), ("AE 2025", "1 preset into User Presets and 1 panel into ScriptUI Panels"));
        assert_eq!(o.fields[0].key, "version");
        crate::procs::set_current_job(9005);
        let steps = plan(AE, &[ae_pack.clone()], None).unwrap();
        convert::run(&steps[0], &mut |_| {}).unwrap();
        assert!(d.join("Docs").join("Adobe").join("After Effects 2025").join("User Presets").join("Pop.ffx").exists());
        let panels = d.join("Programs").join("Adobe After Effects 2025").join(if cfg!(target_os = "macos") { "Scripts" } else { "Support Files/Scripts" }).join("ScriptUI Panels");
        assert!(panels.join("Text Animator.jsx").exists());
        std::env::remove_var("CONVERTINO_AE_ROOT");
        std::env::remove_var("CONVERTINO_DOCUMENTS");
    }

    #[test]
    fn splits_sound_tracks_and_saves_subtitles() {
        let Some(ff) = crate::tools::find(crate::tools::Tool::Ffmpeg) else { return };
        let d = dir("tracks");
        let srt = d.join("s.srt");
        std::fs::write(&srt, "1\n00:00:00,000 --> 00:00:01,000\nHello\n").unwrap();
        let mkv = d.join("Interview.mkv");
        let ok = std::process::Command::new(&ff)
            .args(["-y", "-v", "error", "-f", "lavfi", "-i", "testsrc=size=160x90:rate=10:duration=2", "-f", "lavfi", "-i", "sine=frequency=440:duration=2", "-f", "lavfi", "-i", "sine=frequency=880:duration=2", "-i"])
            .arg(&srt)
            .args(["-map", "0", "-map", "1", "-map", "2", "-map", "3", "-c:v", "mpeg4", "-c:a", "aac", "-c:s", "srt", "-metadata:s:a:0", "title=Game", "-metadata:s:a:1", "title=Mic", "-metadata:s:s:0", "language=eng"])
            .arg(&mkv)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let o = offer(&[mkv.clone()]).unwrap();
        assert_eq!((o.id.as_str(), o.sub.as_str()), (SOUND, "2 tracks"));
        assert_eq!(o.hint, "Saves \"Game\" and \"Mic\" as separate WAVs next to the video");
        let steps = plan(SOUND, &[mkv.clone()], None).unwrap();
        let made: Vec<PathBuf> = steps.iter().map(|s| convert::run(s, &mut |_| {}).unwrap()).collect();
        assert_eq!(made, vec![d.join("Interview - Game.wav"), d.join("Interview - Mic.wav")]);
        let mix = plan(SOUND, &[mkv.clone()], Some(&json!({"mode": "mix"}))).unwrap();
        assert_eq!(mix.len(), 1);
        let m = convert::run(&mix[0], &mut |_| {}).unwrap();
        assert_eq!(m, d.join("Interview (mixed).mkv"));
        let t = sniff::tracks(&m).unwrap();
        assert_eq!(t.audio.len(), 1);
        // Subtitles: the next smart action for this video, by its own id.
        let subs = plan(SUBTITLES, &[mkv.clone()], None).unwrap();
        let s = convert::run(&subs[0], &mut |_| {}).unwrap();
        assert_eq!(s, d.join("Interview.en.srt"));
        assert!(std::fs::read_to_string(&s).unwrap().contains("Hello"));
    }

    #[test]
    fn no_sound_keeps_the_picture_and_drops_only_the_sound() {
        let Some(ff) = crate::tools::find(crate::tools::Tool::Ffmpeg) else { return };
        let d = dir("mute");
        let mp4 = d.join("clip.mp4");
        assert!(std::process::Command::new(&ff)
            .args(["-y", "-v", "error", "-f", "lavfi", "-i", "testsrc=size=160x90:rate=10:duration=2", "-f", "lavfi", "-i", "sine=duration=2", "-c:v", "mpeg4", "-c:a", "aac", "-shortest"])
            .arg(&mp4)
            .status()
            .unwrap()
            .success());
        let steps = convert::plan("video.mute", &[mp4.clone()]).unwrap();
        let out = convert::run(&steps[0], &mut |_| {}).unwrap();
        assert_eq!(out, d.join("clip (no sound).mp4"));
        let t = sniff::tracks(&out).unwrap();
        assert!(t.audio.is_empty());
        let again = convert::plan("video.mute", &[out.clone()]).unwrap();
        assert_eq!(convert::run(&again[0], &mut |_| {}).unwrap_err(), "This video has no sound to remove.");
    }
}
