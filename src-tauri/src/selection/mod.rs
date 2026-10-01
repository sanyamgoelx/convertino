//! Reads the files the user has selected in the OS file manager.
//! Each OS has its own adapter; everything above this module is shared.

/// The files selected when the hotkey was pressed.
pub struct Selection {
    /// Where the selection came from, e.g. "File Explorer", "Desktop", "Finder".
    pub source: String,
    /// Absolute file-system paths. Virtual items (This PC, Recycle Bin) are skipped.
    pub paths: Vec<String>,
}

#[cfg(windows)]
mod win;
#[cfg(windows)]
pub use win::{current_selection, select_item_at, selection_for};

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
pub use mac::current_selection;

#[cfg(not(any(windows, target_os = "macos")))]
pub fn current_selection() -> Result<Selection, String> {
    Err("Convertino only supports Windows and macOS".into())
}
