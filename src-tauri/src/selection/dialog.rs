//! Open and Save dialogs: turning what a file dialog shows into file paths.
//!
//! The dialog belongs to another app, so its folder and file names are read
//! from the outside (the address bar's text, the selected items' names, the
//! "File name" box). These helpers do the plain-text part, so they can be
//! tested anywhere; win.rs does the reading.

use std::path::{Path, PathBuf};

/// "Address: C:\Users\Crofty\Pictures" -> "C:\Users\Crofty\Pictures".
/// The label is localised ("Adresse: …"), so everything up to the first ": " goes.
pub fn parse_address(text: &str) -> String {
    let t = text.trim();
    match t.find(": ") {
        Some(i) => t[i + 2..].trim().to_string(),
        None => t.to_string(),
    }
}

/// The "File name" box: one name, or several as "a.jpg" "b.jpg".
pub fn parse_names(text: &str) -> Vec<String> {
    let t = text.trim();
    if t.contains('"') {
        t.split('"').skip(1).step_by(2).map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect()
    } else if t.is_empty() {
        Vec::new()
    } else {
        vec![t.to_string()]
    }
}

/// The first real extension in a "Save as type" entry: "PNG (*.png;*.apng)" -> "png".
pub fn filter_extension(text: &str) -> Option<String> {
    text.split("*.").skip(1).find_map(|rest| {
        let ext: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
        (!ext.is_empty()).then(|| ext.to_lowercase())
    })
}

/// Files in `folder` for the names the dialog shows. Explorer hides known
/// extensions by default, so "holiday" also finds "holiday.jpg" (every file
/// with that name, if several types share it).
pub fn resolve(folder: &Path, names: &[String]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let entries: Vec<PathBuf> = std::fs::read_dir(folder)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect())
        .unwrap_or_default();
    for name in names {
        let direct = if Path::new(name).is_absolute() { PathBuf::from(name) } else { folder.join(name) };
        if direct.is_file() {
            out.push(direct);
            continue;
        }
        let lower = name.to_lowercase();
        let mut same_stem: Vec<PathBuf> = entries
            .iter()
            .filter(|p| p.file_stem().map(|s| s.to_string_lossy().to_lowercase() == lower).unwrap_or(false))
            .cloned()
            .collect();
        same_stem.sort();
        out.extend(same_stem);
    }
    out.dedup();
    out
}

/// The file a Save dialog will write: the typed name in the folder, with the
/// chosen type's extension when the name has none.
pub fn save_target(folder: &Path, name: &str, type_text: &str) -> Option<PathBuf> {
    let name = name.trim();
    if name.is_empty() || name.contains('*') || name.contains('?') {
        return None;
    }
    let mut path = if Path::new(name).is_absolute() { PathBuf::from(name) } else { folder.join(name) };
    let has_ext = path.extension().map(|e| !e.is_empty()).unwrap_or(false);
    if !has_ext {
        let ext = filter_extension(type_text)?;
        path.set_extension(ext);
    }
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_text() {
        assert_eq!(parse_address("Address: C:\\Users\\Crofty\\Pictures"), "C:\\Users\\Crofty\\Pictures");
        assert_eq!(parse_address("Adresse: D:\\Fotos"), "D:\\Fotos");
        assert_eq!(parse_address("C:\\plain"), "C:\\plain");
        assert_eq!(parse_address("Address: Downloads"), "Downloads");
    }

    #[test]
    fn file_name_box() {
        assert_eq!(parse_names("  holiday.jpg "), vec!["holiday.jpg"]);
        assert_eq!(parse_names("\"a.jpg\" \"b c.png\""), vec!["a.jpg", "b c.png"]);
        assert!(parse_names("").is_empty());
    }

    #[test]
    fn save_types() {
        assert_eq!(filter_extension("PNG (*.png;*.apng)").as_deref(), Some("png"));
        assert_eq!(filter_extension("All files (*.*)"), None);
        assert_eq!(filter_extension("JPEG (*.jpg, *.jpeg)").as_deref(), Some("jpg"));
        let dir = Path::new("/tmp/x");
        assert_eq!(save_target(dir, "poster", "PNG (*.png)"), Some(dir.join("poster.png")));
        assert_eq!(save_target(dir, "poster.webp", "PNG (*.png)"), Some(dir.join("poster.webp")));
        assert_eq!(save_target(dir, "poster", "All files (*.*)"), None);
        assert_eq!(save_target(dir, "*.png", "PNG (*.png)"), None);
    }

    #[test]
    fn names_without_extensions_are_found() {
        let d = std::env::temp_dir().join(format!("convertino-dialog-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        for f in ["holiday.jpg", "notes.txt", "notes.md", "song.mp3"] {
            std::fs::write(d.join(f), b"x").unwrap();
        }
        assert_eq!(resolve(&d, &["holiday".into()]), vec![d.join("holiday.jpg")]);
        assert_eq!(resolve(&d, &["song.mp3".into()]), vec![d.join("song.mp3")]);
        assert_eq!(resolve(&d, &["notes".into()]).len(), 2);
        assert!(resolve(&d, &["missing".into()]).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }
}
