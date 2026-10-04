<img src="assets/app-icon.png" width="96" height="96" alt="Convertino icon">

# Convertino

Convert any file without leaving Explorer or Finder. **Hold Alt and right-click a
file**, pick a format on the radial wheel, and the converted file appears next to
the original. Everything runs offline, on Windows and macOS.

## How to use it

| | Windows | Mac |
|---|---|---|
| **Default: Alt+right-click** | Hold **Alt** and right-click a file in File Explorer or on the desktop | Hold **⌥ Option** (the Alt key) and right-click a file in Finder |
| Keyboard shortcut (optional) | Select files, press **Ctrl+Alt+C** | Select files, press **⌃⌥C** |

Alt+right-click is the easiest way and works right after you install (on a Mac,
after allowing Accessibility once): nothing to set up,
no shortcut to remember. The wheel opens at your pointer, on the file you clicked
(or on all selected files if you click one of them). If you'd rather use the
keyboard, the shortcut is there too; change it to anything you like in
**Settings › General**.

**Status:** milestones 1–4 and 6–8 of 10. Alt+right-click (and the optional shortcut) open the wheel;
images, audio, video, PDFs, documents, data files and archives convert for real,
with a progress ring at the pointer and Undo. "Edit" on the PDF wheel opens an editor:
rearrange, rotate, delete and add pages, and mark up (text, highlight, draw, signature,
pictures, form filling), saved as a copy. Settings (click the tray icon) holds the optional shortcut,
the wheel's order, quality, and the converters, which download the first time they're needed.
Shift+click a format on the wheel to set options for one conversion. On a Mac, Option-right-click opens the wheel; Settings walks you through the
two permissions macOS needs. Open/Save dialogs come next.

- [Build plan](docs/BUILD-PLAN.md)
- [Interactive wheel mockup](docs/wheel-mockup.html) (download and open in a browser)

## Camera RAW photos

DNG, CR2/CR3, NEF, ARW, RAF, ORF, RW2, PEF, SRW and other camera RAW files get their
own wheel: JPG, PNG, TIFF (16-bit), AVIF, WebP, PDF, Compress (to a size) and
**Camera JPG**, the picture the camera stored inside the RAW (its own look, instant).
Photos are developed with the camera's white balance and turned the right way up;
the date, camera, lens, exposure and GPS are copied into the result.

## Command line

The installer adds a `convertino` command (Windows: open a new terminal after
installing; Mac: **Settings › AI & CLI › Install command…**).

```
convertino photo.CR3 --to jpg              convert (jpg, png, webp, mp3, mp4, pdf, docx, xlsx, …)
convertino *.wav --to mp3                  wildcards work in PowerShell and Command Prompt too
convertino video.mp4 --size 25MB           compress to a size (decimal units: 10MB = 10,000,000 bytes)
convertino a.jpg b.jpg --size 5MB --together
convertino report.docx --to pdf --out D:\Converted
convertino formats photo.CR3               what a file can become
convertino tools                           which converters are ready (tools install --all to fetch them now)
```

Options: `--quality small|balanced|best`, `--json` (one JSON result for scripts),
`--quiet`. Results go next to the originals (or `--out`); originals are never changed
and existing files are never overwritten. Exit codes: 0 done, 1 some files failed,
2 the command was wrong, 130 cancelled with Ctrl+C.

## AI apps (MCP)

Convertino is also an [MCP](https://modelcontextprotocol.io) server, so AI apps can
convert files for you ("turn these RAW photos into JPGs", "make this video fit in
25 MB"). In **Settings › AI & CLI**, click **Connect** next to Claude Desktop, Claude
Code or Cursor, then restart that app. For any other MCP app, **Copy setup** gives
the entry to paste:

```json
{ "mcpServers": { "convertino": { "command": "<path to convertino>", "args": ["mcp"] } } }
```

Tools: `list_conversions`, `convert`, `compress_to_size`, `converters_status`.
Same rules as the wheel (originals untouched, nothing overwritten); while an AI app
converts, the corner card shows which app asked, with Open folder and Undo. A switch
in Settings turns AI access off.

**No paths to type.** Two ways to give Claude your files:

- **Ask Claude on the wheel** (key **C**): Alt+right-click files (Option+right-click
  on a Mac) and click **Ask Claude** under the ring. A new Claude chat opens with
  the file paths written in and the cursor after them; say what you want ("make
  these JPGs under 1 MB each"). Claude Desktop runs Convertino on this computer.
  With Claude Code connected too, Shift+click offers a Claude Code session in the
  files' folder instead. The button only shows once Convertino is connected to Claude.
- **Connect a folder** to a Cowork task in Claude (+ Add folder) and ask about the
  files in it: "turn every RAW photo in here into a JPG". This needs Claude to be
  linked to your computer.

## Support

Convertino is free. If it saves you time, you can support its development through
**[GitHub Sponsors](https://github.com/sponsors/sanyamgoelx)** or, in India, UPI to
`sanyamgoel.pc@okicici` (both also in Settings › About).

## Download

Get the latest version from **[Releases](https://github.com/sanyamgoelx/convertino/releases/latest)**:

- Windows 10/11: `Convertino_…_x64-setup.exe`
- Mac with Apple silicon (M1 and later): `Convertino_…_aarch64.dmg`
- Mac with Intel: `Convertino_…_x64.dmg`

Installed copies update themselves (Settings › About). The first time, Windows may
show "Windows protected your PC": click **More info › Run anyway** (the installer
isn't code-signed yet; see `docs/SIGNING.md`).

## Making a release (maintainers)

1. Once: double-click **`release-setup.cmd`**. It makes the key that signs updates,
   stores it in `%USERPROFILE%\.convertino` (back that folder up), and gives it to
   GitHub as Actions secrets.
2. Each release: raise `version` in `src-tauri/tauri.conf.json` (and `package.json`,
   `src-tauri/Cargo.toml`), then double-click **`release.cmd`**. It pushes the code and
   the tag `v<version>`; GitHub builds Windows and both Macs and publishes the release
   with `latest.json` for the updater.

What has to pass before and after a release:

- **Before tagging:** `release.cmd` waits for the Build run of the commit it pushed and
  refuses to tag unless it passed. That run tests on Windows and Apple silicon (unit
  tests; real conversions with downloaded converters and public-domain RAW samples;
  the `convertino` command and its MCP server end to end), builds the installers, then
  **installs them** like a person would: Windows over the previous release, the
  command on PATH in a new terminal, conversions, the MCP server through the official
  MCP Inspector, then a clean uninstall; Mac from the .dmg, through a
  `/usr/local/bin` link, with the bundled converters.
- **Test releases first:** a version like `0.2.0-rc.1` is published as a pre-release.
  Installed copies and the Download link follow "latest", which skips pre-releases.
- **After publishing:** the "Release check" run downloads the published files and
  installs them again on Windows and both Macs, and checks that every file in
  `latest.json` downloads and its signature verifies with the updater key.

## Try the spike

1. Run the app (see below).
2. Select one or more files in File Explorer, on the desktop, or in Finder.
3. Press **Ctrl+Alt+C** on Windows or **⌃⌥C** (Control+Option+C) on Mac.
4. Pick a format on the wheel. Settings and the activity log (every path read and
   every pick) are on the tray or menu-bar icon; quit from there too.

On Mac, the first press asks for permission to control Finder. Click **OK**.

## Set up on Windows (one time)

Double-click **`setup-windows.cmd`**. It installs everything with winget (all free,
official sources), skipping anything you already have, then starts Convertino:

- Git
- Node.js LTS
- Visual Studio 2022 C++ Build Tools (the Rust compiler needs its linker; large download)
- Rust (MSVC toolchain)
- WebView2 runtime (already part of Windows 11)
- The converters, into `src-tauri/tools` (also on its own: `fetch-tools.cmd`): FFmpeg (LGPL build),
  ImageMagick, Poppler, Ghostscript, Pandoc, LibreOffice (no Microsoft Office needed) and 7-Zip.
  This uses the app's own downloader, the one that gets each converter on a friend's PC the first
  time a conversion needs it, and trims each to the files Convertino runs (about 1.1 GB for all
  seven, instead of 2.5 GB).

Windows asks for permission once, for the Build Tools. Click **Yes**.
Progress is written to `setup.log`, and the app's build output to `dev.log`.

After setup, start Convertino any time by double-clicking **`run-dev.cmd`**.
**`run-tests.cmd`** runs the tests, including real conversions with each tool (output in `test.log`).
**`run-smoke.cmd`** converts every sample in `test-files` to every format its wheel offers, into
`test-files\results` (summary in `smoke-report.txt` there).

The first build takes a few minutes while Rust compiles everything; later runs are fast.
`npm run build` makes the installers in `src-tauri/target/release/bundle/`.

## Builds for Mac (and Windows) without a Mac

Every push to GitHub builds a Windows installer and two Mac `.dmg`s (Apple silicon
and Intel) for free; download them from the run's **Artifacts** section under
**Actions**. On Windows, double-click **`publish-github.cmd`** to push (the first
time it installs the GitHub CLI and asks you to sign in in the browser).
**`ci-log.cmd`** saves the logs of a failed build to `ci.log`.

The Mac builds carry ImageMagick, Poppler and Ghostscript inside the app
(`ci/bundle-mac-tools.sh` makes them self-contained from Homebrew's builds), and
download FFmpeg, Pandoc, 7-Zip and LibreOffice the first time they're needed.
The Mac jobs then run the tests with real conversions.

### Installing on a Mac (for friends)

1. Open the `.dmg` for your Mac (Apple menu > About This Mac: "Apple M…" chip =
   Apple silicon, "Intel" = Intel) and drag Convertino to Applications.
2. Open it. macOS says it can't check the app: open **System Settings > Privacy &
   Security**, scroll down and click **Open Anyway** (once; it's free software
   without a paid Apple certificate).
3. Convertino's Settings opens at **Permissions**:
   - **Accessibility**: click *Open System Settings* and turn Convertino on. This is
     what makes Option-right-click work, the main way to use Convertino.
   - **Finder**: click *Allow* and then **OK**, so Convertino can see your selection.
4. Hold **Option** and right-click a file in Finder. (Or select files and press the
   optional shortcut, shown in Settings.)

After an update, macOS may ask for Accessibility again (the app's signature changes).

Needs macOS 14 or later on Apple silicon, macOS 15 or later on Intel (the bundled
converters come from Homebrew builds for those versions).

## Project layout

```
ui/                    window HTML, CSS, JS (shared by both OSes)
src-tauri/src/lib.rs   tray, hotkey, windows, Settings commands
src-tauri/src/settings.rs   settings.json (shortcut, wheel order, quality, save folder…)
src-tauri/src/install.rs    downloads and trims converters on first use
src-tauri/src/convert.rs    conversion plans (+ video.rs, data.rs, archive.rs)
src-tauri/src/selection/
  win.rs               reads the Explorer / desktop selection (Shell COM)
  mac.rs               reads the Finder selection (Apple Events)
src-tauri/src/mac.rs   Option-right-click (event tap) and the Mac permissions
ci/bundle-mac-tools.sh makes the Mac converters self-contained for the .dmg
.github/workflows/     free CI builds for Windows and macOS, Mac tests
```

## Third-party code

The PDF editor bundles PDF.js, pdf-lib, fontkit and three OFL fonts; see `ui/vendor/LICENSES.md`.

## Licence

GPL-3.0-or-later. See [LICENSE](LICENSE).
