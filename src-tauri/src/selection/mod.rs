//! Reads the files the user has selected in the OS file manager.
//! Each OS has its own adapter; everything above this module is shared.

/// The files selected when the hotkey was pressed.
#[derive(Default)]
pub struct Selection {
    /// Where the selection came from, e.g. "File Explorer", "Desktop", "Finder".
    pub source: String,
    /// Absolute file-system paths. Virtual items (This PC, Recycle Bin) are skipped.
    pub paths: Vec<String>,
    /// A Save dialog (its window handle) whose file doesn't exist yet: `paths`
    /// holds the name typed there, and the conversion runs once it's saved.
    pub pending_dialog: Option<isize>,
}

#[cfg_attr(not(windows), allow(dead_code))]
pub mod dialog;

#[cfg(windows)]
mod win;
#[cfg(windows)]
pub use win::{current_selection, select_item_at, selection_for, window_open};

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
pub use mac::current_selection;

/// Whether a window still exists (only Windows has dialogs to wait for).
#[cfg(not(windows))]
pub fn window_open(_hwnd: isize) -> bool {
    false
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn current_selection() -> Result<Selection, String> {
    Err("Convertino only supports Windows and macOS".into())
}
