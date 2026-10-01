# Convertino — Build Plan

Copy of the live plan, 30 Sep 2026. The editable version, with the architecture diagram, is the Claude Doc:
https://claude.ai/code/artifact/b0f3dd09-4c90-45ce-9011-46a3c80540af

Clickable wheel mockup (Windows 11 and macOS): `docs/wheel-mockup.html`, also at https://claude.ai/artifact/Mgzvyv8hw5naoNhfUAGRqF

## Overview

Convertino is a small always-running app for Windows and macOS that you can share with friends: select file(s) in Explorer or Finder, press a hotkey or Alt/Option+right-click, pick a format on a radial wheel, and the converted file appears next to the original. Everything runs offline, and nothing needs Microsoft Office.

Decisions locked in:

- **Platforms:** Windows 10/11 and macOS (Apple Silicon and Intel), shared with friends through free installers and auto-updates.
- **Cost: zero.** Only free, open-source tools and services; no paid certificates, developer accounts or rented machines.
- **Triggers:** a hotkey (Ctrl+Alt+C on Windows, ⌃⌥C on Mac, changeable in Settings; on this PC Ctrl+Alt+C is taken, so it uses Ctrl+Alt+Shift+C) and Alt+right-click (Option+right-click on Mac). The wheel opens when you let go of the button; then you click a format. Both also work in Open and Save dialogs. No context-menu entry for now.
- **Look:** native to each OS with the same layout: Windows 11 acrylic and Fluent on Windows; macOS blur, San Francisco font and system accent on Mac.
- **Formats:** images, audio and video, PDF and documents, data and archives.
- **No Office needed:** LibreOffice handles Word, PowerPoint and Excel files.
- **Stack:** Tauri 2, a Rust core with a web UI, so one codebase covers both.
- **Licence:** GPL-3.0-or-later.
- **Testing:** no Mac on hand, so Mac builds come from GitHub Actions and friends test them.

## User experience

One gesture, one flick, done: the wheel opens at the cursor, you move toward a format, and it converts without any dialog.

**Opening the wheel**

- **Hotkey:** converts whatever is selected in the active Explorer or Finder window, or on the desktop.
- **Alt+right-click** (Option+right-click on Mac): the normal context menu is suppressed and the wheel opens where you clicked. If the file is part of a selection, the whole selection is used; if not, just that file.

**The wheel**

- The centre shows the file icon, name and count ("3 files · WAV").
- Up to 8 target formats sit around the ring, picked for that file family. A "More" slot opens an outer ring with the rest.
- Hovering a slot shows a hint under the wheel, e.g. "JPG · one image per page · 150 DPI".
- Mixed selections (a PDF and a WAV) show a split ring per family.

**Picking**

- Mouse: point at a slot and click. The wheel opens when you release the right button, so it never fights with a drag.
- Keyboard: arrow keys or number keys 1–8, Enter to confirm, Esc or a click outside to cancel.
- Shift+click a slot opens its options (quality, DPI, bitrate, resolution) before converting (Settings milestone).

**After picking**

- A progress card appears in the screen corner (bottom-right on Windows, top-right on Mac).
- On finish it says what was made, with **Open folder** and **Undo** (moves outputs to the Recycle Bin or Trash).
- The ring learns: your most-used target for each family moves to the top slot (Settings milestone).

## Windows and Mac differences

| | Windows | Mac |
| --- | --- | --- |
| Hotkey | Ctrl+Alt+C | ⌃⌥C (⌘⌥C is Finder's Copy as Pathname, so avoided) |
| Mouse trigger | Alt+right-click | Option+right-click or Option+Control-click; can move to ⌃⌥+right-click |
| Reads selection from | Explorer, Win11 tabs, desktop | Finder windows and desktop |
| Wheel material | Acrylic, Fluent icons, Windows accent | Vibrancy blur, San Francisco font, macOS accent; the hovered slot fills with the accent like a Mac menu |
| Lives in | System tray | Menu bar |
| Notification | Bottom-right card | Top-right card |
| Duplicate names | song (1).mp3 | song 2.mp3 |
| Undo sends to | Recycle Bin | Trash |
| First-run setup | None | Grant Accessibility and Automation (Finder) in System Settings; Convertino walks you through it |
| Settings file | %AppData%\Convertino | ~/Library/Application Support/Convertino |

## Architecture

One Tauri app runs in the system tray (Windows) or menu bar (Mac) and starts at login. Only the OS adapters have separate Windows and Mac code; both hand the same list of file paths to the shared core:

Global hotkey / Alt+right-click → Explorer adapter (Shell COM, UI Automation) or Finder adapter (Apple Events, Accessibility) → format registry (`src-tauri/formats.json`) → radial wheel window → job queue → engines (FFmpeg, ImageMagick, PDFium, LibreOffice, 7-Zip + Rust) → output writer + progress/done card.

## Tech stack

| Layer | Choice | Notes |
| --- | --- | --- |
| App shell | Tauri 2 (Rust), transparent always-on-top windows | Mac transparency needs Tauri's private-API flag; fine outside the Mac App Store |
| Wheel, cards, settings | HTML, CSS, JavaScript, shared | Real acrylic/vibrancy blur still to do (solid surface for now) |
| Hotkey | Tauri global-shortcut plugin | Falls back to a free shortcut if the default is taken |
| Alt / Option+right-click | Windows: low-level mouse hook. Mac: CGEventTap | Mac needs Accessibility permission |
| Reading the selection | Windows: Shell COM + UI Automation. Mac: Finder via Apple Events + Accessibility | Mac asks once for permission to control Finder |
| Audio / video | FFmpeg (LGPL build by BtbN) | Progress via `-progress pipe:1` |
| Images | ImageMagick portable (replaced the planned libvips) | One program covers JPG, PNG, WEBP, AVIF, HEIC, TIFF, GIF, ICO, PDF |
| PDF | PDFium for rendering, lopdf for building and merging | Next milestone |
| Documents | LibreOffice (headless, downloaded on first use), Pandoc | No Microsoft Office needed |
| Data | Rust crates: calamine, rust_xlsxwriter, csv, serde_json, serde_yaml | No extra programs |
| Archives | 7-Zip | ZIP, 7Z, RAR, TAR |
| Updates | Tauri updater plugin, files on GitHub Releases | Its own free signing keys |

## Format matrix

| Family | Sources | First-ring targets | Engine |
| --- | --- | --- | --- |
| Image | JPG, PNG, WEBP, AVIF, HEIC, BMP, TIFF, GIF, SVG, ICO | JPG, PNG, WEBP, AVIF, PDF, ICO, Resize | ImageMagick |
| Audio | WAV, MP3, FLAC, AAC, M4A, OGG, OPUS, WMA, AIFF | MP3, WAV, FLAC, AAC, OGG, OPUS, M4A, Trim silence | FFmpeg |
| Video | MP4, MOV, MKV, WEBM, AVI, WMV | MP4, WEBM, GIF, MP3 (extract audio), Compress, 720p, Frames | FFmpeg |
| PDF | PDF | JPG, PNG, WEBP, TXT, Split, Compress, Merge (multi-select) | PDFium, lopdf |
| Document | DOCX, DOC, PPTX, XLSX, ODT, RTF, MD, HTML, TXT | PDF, TXT, HTML, Markdown, DOCX, ODT | LibreOffice, Pandoc |
| Data | CSV, TSV, JSON, YAML, XML | XLSX, CSV, JSON, YAML, TSV | Rust crates |
| Archive | ZIP, 7Z, RAR, TAR, GZ | Extract, ZIP, 7Z, TAR.GZ | 7-Zip |

Not in v1: PDF to editable DOCX and RAW camera files.

## Hard parts and how they're solved

**On Windows**

1. **Reading the Explorer selection.** Match the foreground window in `ShellWindows`, pick the active Win11 tab via its `ShellTabWindowClass` child, read `IFolderView2::GetSelection`. Desktop: `FindWindowSW` with the desktop flag.
2. **Alt+right-click.** A low-level mouse hook swallows the right-button press and release when Alt is held over Explorer's file view or the desktop icons. UI Automation selects the file under the pointer if it isn't selected. An unassigned key is injected so releasing Alt doesn't trigger Explorer's keyboard shortcuts.
3. **A blurred, round wheel.** Acrylic applies to a whole window, not a circle; circular region + acrylic, with a solid fallback.
4. **Hook safety.** The hook only checks window classes; all real work runs on a thread.
5. **Long jobs.** Each conversion runs on its own background thread, so the wheel is never blocked.

**On the Mac**

1. **Reading the Finder selection.** Apple Events (`tell application "Finder" to get selection`), Accessibility as fallback.
2. **Option+right-click.** A CGEventTap swallows the click; an Accessibility hit-test finds the file.
3. **Permissions.** First launch walks through Accessibility and Automation.
4. **Keyboard focus.** The wheel is a panel that takes focus briefly and hands it back.
5. **No Mac to debug on.** CI builds and tests on GitHub's free Mac machines; friends send debug logs.

## Open and Save dialogs

**Open dialog: convert, then pick the result.** Select the file in the dialog, trigger the wheel, pick a format the dialog accepts ("Accepted here"); Convertino saves the converted file next to the original and types its name into the File name box. If the dialog's filter hides the file, pressing the hotkey with nothing selected switches the view to all files.

**Save dialog: save, then convert.** Trigger the wheel in the Save dialog, pick the format; after the app saves, Convertino converts the file. The original is kept.

Dialogs are read with UI Automation (Ctrl+C in the file view as fallback). On the Mac this is best effort and comes after Windows.

## Output rules and settings

- **Naming:** `song.wav` becomes `song.mp3`. If that exists: `song (1).mp3` on Windows, `song 2.mp3` on Mac.
- **Many outputs from one file:** PDF pages and video frames go into a subfolder.
- **Metadata:** EXIF, ID3 tags and dates are kept where the target format supports them.
- **Originals:** always kept.

Settings (tray / menu-bar icon): shortcut recorder, mouse trigger choice, default quality per format, pin/reorder/hide wheel slots, output location, start at login, automatic updates.

## Milestones

Windows first, because it's the machine available for testing.

1. **Spike:** tray app, hotkey, read the Explorer selection. **Done.**
2. **Wheel:** transparent window, slots, hover and keyboard, animations, light and dark. **Done.**
3. **First conversions:** background jobs, progress and done cards, images and audio. **Built, being tested.**
4. **Alt+right-click (Windows).** **Done.**
5. **Mac port:** menu-bar app, Finder selection, ⌃⌥C, Option+right-click, permission setup, Mac look.
6. **PDF and documents:** PDF to images, images to PDF, merge, LibreOffice on first use.
7. **Video, data, archives:** the rest of the format matrix.
8. **Settings:** shortcut recorder, per-format options on Shift+click, slot reordering by use.
9. **Open and Save dialogs:** Windows first, then Mac best effort.
10. **Release to friends:** free installers, auto-updates, install guide, licences screen; apply for free Windows signing (SignPath Foundation).

## Sharing with friends

| | Windows | Mac |
| --- | --- | --- |
| Installer | Setup .exe or .msi | .dmg with one universal app |
| Built and hosted on | GitHub Actions and Releases, free for public repos | Same |
| Signing | Unsigned at first, then SignPath Foundation (free for open source) | Free self-signed certificate; no notarization (needs a paid account) |
| What friends see once | "Windows protected your PC" → More info → Run anyway | System Settings → Privacy & Security → Open Anyway |
| Updates | Tauri updater with its own free signing keys | Same |

Licences: ship the LGPL build of FFmpeg and an About screen listing FFmpeg, ImageMagick, LibreOffice, Pandoc, 7-Zip and PDFium with their licences and source links.

## Risks

| Risk | Fallback |
| --- | --- |
| Friends are put off by the first-open security prompt | Install guide with screenshots; free Windows signing after the first release |
| Mac bugs are hard to fix without a Mac | Automated tests on GitHub's Mac machines, friends' debug logs |
| Friends skip the Mac permission steps | Setup screen shows which toggle is missing |
| Mac forgets permissions after an update | Same self-signed certificate on every build |
| Round blur looks wrong on some GPUs | Solid surface with shadow |
| An OS update changes Explorer or Finder internals | Clipboard / Accessibility fallbacks |
| Antivirus flags the unsigned mouse hook | Hotkey works without it; signing later |
| LibreOffice is a large download | Fetched only on first document conversion |

## Open questions

- Is ⌃⌥C a good Mac default?
- Create the public GitHub repo (postponed until after the wheel; ask again).
