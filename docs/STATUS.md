# Convertino — status

Last updated: 7 Oct 2026. Shared with the Claude Project "Convertino" (claude/convertino-status.md).

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
- Faster CI (2026-10-02): thin LTO instead of full LTO; Build and Release share the Rust cache (rust-cache shared-key = matrix name; only main saves); Mac converters cached per month/script hash (actions/cache); npm ci with npm cache; Intel Mac skips the download + real-conversion tests except on manual runs (Apple silicon still runs them). First run after the change refilled the caches (Intel 882 s compile, cold). v0.1.4 release: 8.5 min total (was ~22). With cached converters Homebrew is absent, which exposed (a) the CI Ghostscript check relying on Homebrew's data path (now sets GS_LIB like tools.rs), (b) intermittent LibreOffice download failures ("Couldn't connect") from download.documentfoundation.org's mirror redirect: install.rs now tries LIBREOFFICE_MIRRORS in order (TDF redirector, ftp.fau.de, netcologne, rwth-aachen; same /tdf/libreoffice/stable/ layout), and CI retries fetch_tools once. ci-log.cmd now picks the newest FAILED run.
- Hotkey on this PC: Ctrl+Alt+Shift+C (another app owns Ctrl+Alt+C).
- Licence: GPL-3.0-or-later. Zero cost: free tools and services only.
- Formats live in `src-tauri/formats.json`; conversions in `src-tauri/src/convert.rs` (plus `video.rs`, `data.rs`, `archive.rs`).
- PDF editor: `ui/editor.*` (PDF.js renders, pdf-lib saves a copy `<name> (edited).pdf`); Rust side `open_editor` / `editor_*` commands in lib.rs.
- PDF editor "Nothing to save" fix (7 Oct): if a window's custom-protocol IPC fetch ever fails, Tauri switches that window to postMessage for good and raw bytes arrive as a JSON number array. `editor_save` now accepts both (logs a warning when it falls back). Not yet retested on the PC. User said "Publish": version raised to 0.2.1 (five files); release.cmd still to be run by the user.
- Video picks the best working encoder at run time (x264/x265 if the FFmpeg build has them, else NVIDIA/Intel/AMD/Apple hardware, else Windows Media Foundation or OpenH264). The LGPL FFmpeg build has no x264/x265.
- Converters download on first use (`src-tauri/src/install.rs`): official releases (GitHub, documentfoundation.org) via the curl built into Windows, unpacked with 7-Zip (itself fetched first, 2 MB), then trimmed: only the programs Convertino runs and the DLLs they import (read from the PE import tables), and for LibreOffice the known extras (other UI languages, spelling dictionaries and thesauri, help, icon themes, PDF import). Hyphenation patterns stay. Progress shows on the job's ring/card; Cancel stops it. Installed builds keep them in %LOCALAPPDATA%\Convertino\tools; dev builds in src-tauri/tools. A converter already installed on the PC (e.g. LibreOffice in Program Files) is used instead and never trimmed.
- Sizes once set up: FFmpeg 190 MB (x264/x265 build), ImageMagick 33, Poppler ~60, Ghostscript ~43, Pandoc 224, LibreOffice ~630, 7-Zip 2 (about 1.1 GB if all are used, was 2.5 GB). First-use downloads: 77, 11, 42, 62, 40, ~350, 2 MB.
- Ghostscript and ImageMagick need the Visual C++ runtime: if System32 lacks it, Microsoft's vc_redist.x64.exe is downloaded and run (one UAC prompt). LibreOffice gets its own copy next to soffice.
- Mac downloads: not yet (Mac milestone).
- Settings (ui/settings.*, src-tauri/src/settings.rs): settings.json in the app config folder; every change saves at once (settings_set merges a JSON patch). Pages: General (shortcut recorder: the current hotkey is paused while recording, a taken combo is refused with a message and the old one kept; Alt+right-click; start at sign-in via HKCU Run / a LaunchAgent, inert in dev builds; progress ring on/off; save next to the original or in one folder; hand-over seconds), Wheel (order by dragging the grip or Alt+arrows, hide, "pick most first" counts picks ≥2), Quality (JPG/WebP quality, size when converting, PDF DPI and Compress level, MP3 bitrate, video quality, GIF width/length), Converters (status, Download now/all, updates, remove, FFmpeg standard/x264 build, encoder in use), About (licences, logs, activity log).
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

## Compress to a size (2026-10-02)

- Replaces image Resize; Compress on every family (image, video, PDF; audio under More) opens a size ring: up to 5 sizes fitted to the file (Discord 10 MB, Email 18 MB, then steps down from the estimated "looks the same" size; unreachable, already-fitting and near-duplicate sizes hidden) plus Custom…. Each slot's tag says what it costs ("720p", "15 fps", "Full quality").
- Custom… or typing a number turns the hub into a size box (MB/KB button, K/M keys); the hint previews the plan live (compress_preview). Too small: "Make it <smallest>" and, for video, "Keep 720p, first N s" (trim).
- Several files: Each / Together toggle in the hub; Together shares the size by each file's need; files already small enough (or tiny) are left alone with a note.
- Engine: src-tauri/src/size.rs (plans, previews, presets, picture + audio encoders), video.rs run_to_size (generous size: look-the-same Compress with a maxrate cap, if the samples predict it fits; else H.265 at the size's bitrate, x265 two-pass, graphics card for long/huge videos; resolution ladder for camera video, frame rate first for screen recordings — detected by name or mpdecimate on 4 s), convert.rs pdf_to_size (8 Ghostscript strengths, gentlest that fits; none fits → smallest kept, card says so).
- Compare: finished Compress jobs (not audio) get a Compare button (ring pill and corner card) → compare window (ui/compare.*, compare.rs): original vs result under a slider; video = middle frame, PDF = page 1.
- Not adopted (user): re-checking the output and redoing it when it comes out over the size.

## RAW photos, command line, MCP (released 3 Oct 2026 as 0.2.0-rc.1, a pre-release)

- Plan and how it was built: `docs/PLAN-RAW-CLI-MCP.md`. Mock-up (approved, "good to go"): https://claude.ai/artifact/MbTKhjexfYJjFktUAJy4Ga
- RAW photo wheel (DNG, CR2/CR3, NEF, ARW, RAF, ORF, RW2, PEF, SRW, …): JPG, PNG, TIFF 16-bit, AVIF, WEBP, PDF, Compress, Camera JPG; EXIF copied.
- `convertino` command (installed to `<install>\bin`, on PATH; Mac: Settings › AI & CLI › Install command…).
- `convertino mcp` MCP server + Settings › AI & CLI (Connect for Claude Desktop / Claude Code / Cursor, Copy setup, AI on/off, corner card "via …", recent AI jobs).
- Checked on the PC with run-checks (3 Oct): tests 107 + 13 command line + 6 MCP all pass, smoke all green; RAW samples in test-files/raw.
- CI now: Windows tests too, RAW samples, and installs the built installers (Windows over the previous release, then uninstall; Mac from the .dmg). Releases: a version with "-" (0.2.0-rc.1) is a pre-release; "Release check" re-installs the published files and verifies latest.json signatures; release.cmd waits for a green Build run before tagging.
- RAW on Mac: Homebrew builds ImageMagick with `--with-raw=no`, so its DNG coder reads nothing. The Mac bundle now ships LibRaw's `dcraw_emu` next to `magick` (ci/bundle-mac-tools.sh); raw.rs `develop_with_libraw` uses it when present (same settings: camera WB, sRGB, 16-bit), else ImageMagick; rawler stays the fallback. The real-RAW test requires LibRaw for DNG/NEF/ARW and lets rawler cover CR3/CRW/RAF when the LibRaw version can't.
- CI (3 Oct): all green at 95093b7 (run 37128840130): Windows, Intel Mac, Apple silicon, and the install checks on Windows and Mac. ci-log.cmd now also reads runs that are still going.
- 3 Oct: user said "Publish": version raised to 0.2.0-rc.1 (package.json, package-lock.json, Cargo.toml, Cargo.lock, tauri.conf.json), release.cmd run. Build da18d16 green, tag v0.2.0-rc.1 pushed, Release run 37134573580 green including Release check (installs on Windows, Intel Mac, Apple silicon; updater files verify). https://github.com/sanyamgoelx/convertino/releases/tag/v0.2.0-rc.1
- Next: install 0.2.0-rc.1 from GitHub Releases and try Settings › AI & CLI; a final 0.2.0 later (installed copies only update to non-pre-releases). Computer use can't click while Valorant or Task Manager is in front.

## Ask Claude (released 4 Oct 2026 as 0.2.0-rc.2, a pre-release)

- Design: boards 6–9 on https://claude.ai/artifact/MbTKhjexfYJjFktUAJy4Ga (approved: "Build now"; the button shows only when Convertino is connected to Claude).
- Wheel: an **Ask Claude** pill on the ring's bottom edge (key C; Shift+click / Shift+C: Cowork, Chat or Claude Code, "Use this every time"). Hidden in the size ring, for Save-dialog conversions, when AI apps are off, when the setting is off, and when no Claude app has Convertino connected (Cowork/Chat need Claude Desktop, Code needs Claude Code). Window 420 × 490 (was 460); the hint moves to 394 px.
- `src-tauri/src/ask.rs`: Claude's desktop links (support.claude.com "Open Claude Desktop with a link"): `claude://cowork/new?file=…&folder=…&q=…` (≤5 items one by one, more: their folders + "The files I picked (N): …"), `claude://claude.ai/new?q=<paths>`, `claude://code/new?folder=<common folder>&q=Files: …`. Links capped at 8000 chars. Connection state cached 15 s, refreshed at startup and after Connect/Disconnect. Opened with rundll32 url.dll (Windows) / `open` (Mac). Everything selected goes, folders and unknown files too.
- Settings › AI & CLI: "Ask Claude from the wheel" (switch + Open in: Cowork / Chat / Claude Code, only the connected ones); without a connection it says to connect first. Settings keys `askClaude`, `askMode`.
- Tests: 10 in ask.rs (modes, visibility, link shapes, encoding of spaces and &, length caps, folder attach). Unit tests 117 pass on Linux.
- CI: run 37175841321 green (Windows, both Macs, install checks).
- To check on the PC: Ask Claude with 1, 3 and 10 files in Claude Desktop (does Cowork attach several `file=` at once? does the folder prompt show once?).
- 4 Oct, tried on the PC: **Cowork mode failed** (the Cowork task runs in the cloud; the attached JPG never got copied over, "can't reach your computer"), **Chat mode worked**. User agreed to drop Cowork: modes are now **Chat** (default; Claude Desktop) and **Claude Code**. Settings saying "cowork" become "chat". The Open in choice only shows when both Claude apps are connected. Connect a folder in Cowork stays documented as the other way (needs Claude linked to the computer).
- CI 4 Oct (635a2e3): Windows green; Apple silicon failed making the .dmg (`bundle_dmg.sh`, hdiutil "Resource busy", random on GitHub's Macs). Fix: Build stops XProtect first and retries the Mac build up to 3 times; Release stops XProtect and uses tauri-action `retryAttempts: 2`.
- CI 4 Oct (4a2a0b7, run 37179644708): all green, including the install checks. Ready for "Publish" (0.2.0-rc.2).
- 4 Oct: user said "Publish": version raised to 0.2.0-rc.2 (five files), release.cmd run.
- Released: Build 22a3c23 green, tag v0.2.0-rc.2, Release run 37182245503 green including Release check (installs on Windows, Intel Mac, Apple silicon; updater files verify). https://github.com/sanyamgoelx/convertino/releases/tag/v0.2.0-rc.2
- Next: install rc.2 and use Ask Claude for a while; a final 0.2.0 (non-pre-release, so installed copies update) when happy.

## Claude didn't use Convertino from a chat (released 5 Oct 2026 as 0.2.0-rc.3, a pre-release)

- Seen: Ask Claude (Chat) with three Valorant clips + "compress these": Claude said the files didn't upload and it can't reach E:, and gave FFmpeg commands. The MCP itself was fine (converters_status answered from Cowork).
- Cause: the Chat link sent only paths; Claude Desktop keeps connected tools deferred behind tool search, and Convertino's descriptions never said they reach local files, so Claude didn't look for them.
- Fix: `ask::CHAT_INTRO` first line of the Chat link ("These files are on my computer (Convertino can open and convert them):"); mcp.rs INSTRUCTIONS rewritten (local paths, don't ask for an upload, compress vs compress_to_size, report where/how big); list_conversions / convert / compress_to_size descriptions start with "Works on files on the user's computer (local paths like E:\\Videos\\clip.mp4 or ~/Photos/…); no upload needed." Tool names and inputs unchanged.
- Tests (Linux, cloud): lib 116 pass (2 ignored) incl. new `instructions_point_at_local_files` and the description check in `tool_list_is_stable`; tests/mcp.rs 6 pass.
- Pushed 49300bd: Build failed on Apple silicon only: `ask::tests::links_stay_short_enough_to_open` (Chat link 8001 chars > 8000). The intro line ate into the fixed 120-char reserve for the "…and N more in <folder>" line, and the Mac temp folder is long. Fix: chat_link now builds the whole link and drops paths until it fits (no fixed reserve). Swept 160 temp-folder lengths on Linux: old code failed 38, new 0.
- Pushed 79ac591 after run-checks on the PC: Build run 37219515910 all green (Windows, Intel Mac, Apple silicon, install checks on Windows and Mac).
- 5 Oct: user said "Publish": version raised to 0.2.0-rc.3 (five files), release.cmd run.
- Released: Build 8e10f8a green, tag v0.2.0-rc.3, Release run 37235070191 green including Release check (installs on Windows, Intel Mac, Apple silicon; updater files verify). https://github.com/sanyamgoelx/convertino/releases/tag/v0.2.0-rc.3
- Next: install rc.3, then re-try "compress these" with the three Valorant clips from a Claude chat (pasted paths, and via Ask Claude). Then re-try the same chat (with and without Ask Claude).

## 0.2.0 (final, 6 Oct 2026)

- 6 Oct: rc.3 installed and checked on the PC: Claude now uses Convertino from a chat ("Working"). User said release it.
- Version raised to 0.2.0 (five files), release.cmd run: Build 2d788d2 green (run 37467310948), tag v0.2.0 pushed.
- Released: Release run 37469840342 green including Release check (installs on Windows, Intel Mac, Apple silicon; updater files verify). Marked latest (not a pre-release). https://github.com/sanyamgoelx/convertino/releases/tag/v0.2.0
- First non-pre-release since 0.1.6: installed copies (and the README Download link) update to it.
- Next: let the installed copy update itself to 0.2.0 (Settings › About); then Release to friends (Mac testers first). Housekeeping: milestone table is stale (Mac builds ship; Save-dialog conversions exist); CI actions on Node 20 are deprecated.
