//! macOS adapter: asks Finder for its selection through Apple Events.
//!
//! The first call triggers the "Convertino wants to control Finder" prompt.
//! Finder's selection covers both Finder windows and the desktop.

use super::Selection;
use std::process::Command;

const SCRIPT: &str = r#"tell application "Finder"
    set out to ""
    repeat with f in (get selection)
        set out to out & POSIX path of (f as alias) & linefeed
    end repeat
    return out
end tell"#;

pub fn current_selection() -> Result<Selection, String> {
    let output = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(SCRIPT)
        .output()
        .map_err(|e| format!("Couldn't run osascript: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // -1743 = the user hasn't allowed Convertino to control Finder.
        if stderr.contains("-1743") {
            return Err("Convertino isn't allowed to control Finder yet. Turn it on in \
                 System Settings > Privacy & Security > Automation."
                .into());
        }
        return Err(format!("Finder didn't answer: {}", stderr.trim()));
    }

    let paths = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();

    Ok(Selection {
        source: "Finder".into(),
        paths,
    })
}
