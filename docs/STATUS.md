# Convertino — status

Last updated: 1 Oct 2026. Shared with the Claude Project "Convertino" (claude/convertino-status.md).

## Where things are

- Code: this folder (`F:\Tool\convertino`), Tauri 2 app.
- Plan: `docs/BUILD-PLAN.md` (live, editable copy: https://claude.ai/code/artifact/b0f3dd09-4c90-45ce-9011-46a3c80540af)
- Mockup: `docs/wheel-mockup.html` (online: https://claude.ai/artifact/Mgzvyv8hw5naoNhfUAGRqF)
- Scripts (double-click): `setup-windows.cmd` (installs everything), `run-dev.cmd` (starts Convertino), `fetch-tools.cmd` (runs the app's downloader: gets any missing converter into `src-tauri/tools` and trims the ones already there).
- `run-checks.cmd` runs fetch-tools, run-tests and run-smoke in a row.
- `run-tests.cmd` runs the Rust tests with real conversions (all 14 pass on Windows and Linux).
- Logs: `dev.log` (app), `setup.log`, `tools.log`, `test.log`.

## Milestones

| # | Milestone | State |
| --- | --- | --- |
| 1 | Tray app, hotkey, read Explorer selection | Done, tested |
| 2 | Radial wheel | Done, tested |
| 3 | Image + audio conversions, progress and done cards, Undo | Done, tested |
| 4 | Alt+right-click (Windows) | Done, tested |
| 5 | Mac port | Not started |
| 6 | PDF and documents | Done, tested |
| 7 | Video, data, archives; progress ring; PDF editor | Built, being tested |
| 8 | Settings; Shift+click options; download on first use | Built, being tested |
| 8b | Smallest file that looks the same (pictures, PDFs, audio, video) | Built, being tested |
| 9 | Open and Save dialogs | Not started |
| 10 | Release to friends | Not started |

## Notes

- Messaging: Alt+right-click (Option+right-click on Mac) is THE default way to use Convertino, first in the README, Settings (how-to card at the top of General, "Default" badge) and notices. The keyboard shortcut stays registered in the background but is presented as optional, set up in Settings.
- Faster CI (2026-10-02): thin LTO instead of full LTO; Build and Release share the Rust cache (rust-cache shared-key = matrix name; only main saves); Mac converters cached per month/script hash (actions/cache); npm ci with npm cache; Intel Mac skips the download + real-conversion tests except on manual runs (Apple silicon still runs them). First run after the change refilled the caches (Intel 882 s compile, cold).
- Hotkey on this PC: Ctrl+Alt+Shift+C (another app owns Ctrl+Alt+C).
- Licence: GPL-3.0-or-later. Zero cost: free tools and services only.
- Formats live in `src-tauri/formats.json`; conversions in `src-tauri/src/convert.rs` (plus `video.rs`, `data.rs`, `archive.rs`).
- PDF editor: `ui/editor.*` (PDF.js renders, pdf-lib saves a copy `<name> (edited).pdf`); Rust side `open_editor` / `editor_*` commands in lib.rs.
- Video picks the best working encoder at run time (x264/x265 if the FFmpeg build has them, else NVIDIA/Intel/AMD/Apple hardware, else Windows Media Foundation or OpenH264). The LGPL FFmpeg build has no x264/x265.
- Converters download on first use (`src-tauri/src/install.rs`): official releases (GitHub, documentfoundation.org) via the curl built into Windows, unpacked with 7-Zip (itself fetched first, 2 MB), then trimmed: only the programs Convertino runs and the DLLs they import (read from the PE import tables), and for LibreOffice the known extras (other UI languages, spelling dictionaries and thesauri, help, icon themes, PDF import). Hyphenation patterns stay. Progress shows on the job's ring/card; Cancel stops it. Installed builds keep them in %LOCALAPPDATA%\Convertino\tools; dev builds in src-tauri/tools. A converter already installed on the PC (e.g. LibreOffice in Program Files) is used instead and never trimmed.
- Sizes once set up: FFmpeg 190 MB (x264/x265 build), ImageMagick 33, Poppler ~60, Ghostscript ~43, Pandoc 224, LibreOffice ~630, 7-Zip 2 (about 1.1 GB if all are used, was 2.5 GB). First-use downloads: 77, 11, 42, 62, 40, ~350, 2 MB.
- Ghostscript and ImageMagick need the Visual C++ runtime: if System32 lacks it, Microsoft's vc_redist.x64.exe is downloaded and run (one UAC prompt). LibreOffice gets its own copy next to soffice.
- Mac downloads: not yet (Mac milestone).
- Settings (ui/settings.*, src-tauri/src/settings.rs): settings.json in the app config folder; every change saves at once (settings_set merges a JSON patch). Pages: General (shortcut recorder: the current hotkey is paused while recording, a taken combo is refused with a message and the old one kept; Alt+right-click; start at sign-in via HKCU Run / a LaunchAgent, inert in dev builds; progress ring on/off; save next to the original or in one folder; hand-over seconds), Wheel (order by dragging the grip or Alt+arrows, hide, "pick most first" counts picks ≥2), Quality (JPG/WebP quality, size when converting, Resize size, PDF DPI and Compress level, MP3 bitrate, video quality, GIF width/length), Converters (status, Download now/all, updates, remove, FFmpeg standard/x264 build, encoder in use), About (licences, logs, activity log).
- Wheel hints show the current values ("300 DPI"). Shift+click (or Shift+Enter / Shift+number) opens options for that conversion; "Use these every time" saves them.
- Tray: left-click opens Settings; menu: Settings, Activity log, Quit. The first run opens Settings.
- GitHub repo: postponed; will ask before creating it.
- Smallest file that looks the same (Settings > Quality > File size: Smaller / Balanced (default) / Best / Fixed):
  - Pictures (`look.rs`): JPG/WebP/AVIF quality found by bisection on a 768 px centre crop, judged with SSIMULACRA2 (targets 70/80/87); PNGs (and PDF PNG pages) go through oxipng, pixel-identical. PDF JPG/WebP pages: quality searched on page 1, used for all.
  - PDF Compress: Ghostscript at 4 strengths in parallel (pictures untouched, 300, 150, 72 DPI); first/middle/last pages rendered and compared; the smallest that reaches the bar wins (bar is relative to the untouched version, so font re-rendering doesn't count). Under 5% smaller: "already compact".
  - Video (`video.rs`): 3 samples of 3 s (short clips: 4 s from the middle) encoded at a few CRF/CQ values, scored with VMAF (targets 89/93/95.5; SSIM if the FFmpeg build lacks libvmaf); the highest value that passes is used for the whole video. Compress = H.265 MP4, conversions H.264, WebM VP9. When x265/x264 and the graphics card both work, both are measured and the card wins unless software is >15% smaller (kept in encoder-choice.json, re-measured every 10 uses). Compress stops early when samples say <10% saving. Small AAC is copied.
  - Audio: MP3 is VBR (V0 ≈ 245 kbps at the top setting), never above a lossy source's bitrate; FLAC level 8.
  - Batches: pictures/audio/PDFs convert up to 4 at a time (half the cores); video and Office one by one.
  - FFmpeg now defaults to the x264/x265 (GPL) build; settings.json gets `version: 2` and older files move to it; an older FFmpeg copy is swapped in the background (or by fetch-tools).
  - Smoke report shows the size change next to the original.
