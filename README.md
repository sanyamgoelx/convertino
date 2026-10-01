# Convertino

Convert any file without leaving Explorer or Finder. Select files, press a hotkey
(or Alt/Option+right-click), pick a format on a radial wheel, and the converted
file appears next to the original. Everything runs offline, on Windows and macOS.

**Status:** milestones 1–4 and 6–8 of 10. The hotkey and Alt+right-click open the wheel;
images, audio, video, PDFs, documents, data files and archives convert for real,
with a progress ring at the pointer and Undo. "Edit" on the PDF wheel opens an editor:
rearrange, rotate, delete and add pages, and mark up (text, highlight, draw, signature,
pictures, form filling), saved as a copy. Settings (click the tray icon) holds the shortcut,
the wheel's order, quality, and the converters, which download the first time they're needed.
Shift+click a format on the wheel to set options for one conversion. On a Mac, Option-right-click opens the wheel; Settings walks you through the
two permissions macOS needs. Open/Save dialogs come next.

- [Build plan](docs/BUILD-PLAN.md)
- [Interactive wheel mockup](docs/wheel-mockup.html) (download and open in a browser)

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
     what makes Option-right-click work.
   - **Finder**: click *Allow* and then **OK**, so Convertino can see your selection.
4. Select files in Finder and press **⌃⌥⇧C**, or Option-right-click a file.

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
