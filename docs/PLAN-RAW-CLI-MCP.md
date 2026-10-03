# Convertino: RAW photos, command line and MCP (plan)

3 Oct 2026. Status: **design approved ("good to go") and built.** Tests pass on Linux (cloud) and on the Windows PC (run-checks: 107 unit + 13 command-line + 6 MCP, smoke all green); pushed 9a92c0d for CI. Not released yet.
Mock-up (Design canvas): https://claude.ai/artifact/MbTKhjexfYJjFktUAJy4Ga

## Decisions (user, 3 Oct)

- **RAW:** read every common camera RAW format (DNG, CR2/CR3, NEF, ARW, RAF, ORF, RW2 and others) and convert it out. Convertino won't make DNG files.
- **CLI:** a `convertino` command that comes with the app (on PATH on Windows; on Mac, Settings has "Install command…").
- **MCP:** built in (`convertino mcp`), with one-click Connect for Claude Desktop, Claude Code and Cursor in Settings.
- **MCP access:** an AI app can convert any file it names. The wheel's rules still apply: output goes next to the source, originals are never changed or overwritten, and jobs are listed in Settings.

## What changes on screen (see the canvas)

1. **RAW photo wheel** (new family, teal #0E7C86): JPG, PNG, TIFF (16-bit), AVIF, Compress, PDF, WEBP, **Camera JPG** (pulls out the camera's own full-size JPEG; instant; renamed from "Preview" during polish). The hub shows "RAW photo · 24 MP".
2. **Corner card for AI jobs:** the same card as for wheel jobs, plus a "via Claude Desktop" badge. On by default; it can be turned off.
3. **Command line:** plain output, one line per file with sizes and time; progress bar for long jobs; `--json` for scripts; errors list what the file *can* become.
4. **Settings › AI & CLI** (new tab between Converters and About). It has a command line card (status, examples, Copy), a "Let AI apps convert files" master switch, a Connect/Disconnect row per app plus "Copy setup" for other apps, a "show a card" switch, and Recent AI jobs.
5. The **Mac** version of that tab has "Install command…", which asks for the password once.

## Shared groundwork (first)

- **Headless engine** (`engine.rs`): one job API (plan → run → progress → notes → cancel) with no window code. The wheel (`jobs.rs`), the CLI and MCP all use it. `convert::plan_sized` / `convert::run` already run without the app; jobs.rs gets thinner.
- Downloading converters on first use also works headless. Progress goes to the terminal (CLI) or arrives as MCP progress notifications.
- CLI and MCP read the same `settings.json` (quality, save folder). They never write it, apart from the MCP job log.
- **Second program `convertino-cli`** (console program) in the same crate: `src/bin/cli.rs`. The app itself is a GUI program on Windows and can't print to a terminal.
  - Windows: installed to `<install dir>\bin\convertino.exe`. The NSIS installer adds `bin` to the user PATH; the uninstaller removes it.
  - Mac: `Contents/MacOS/convertino-cli`. "Install command…" links `/usr/local/bin/convertino` to it (admin prompt through osascript).

## RAW photos

- **Extensions:** dng, cr2, cr3, crw, nef, nrw, arw, srf, sr2, raf, orf, rw2, pef, srw, x3f, 3fr, iiq, rwl, erf, kdc, dcr, mos, mrw.
- **Developing:** ImageMagick's built-in LibRaw (already shipped on both systems).
  - Settings: camera white balance, sRGB, 16-bit inside, rotated by the EXIF orientation.
  - *Check:* that the Windows portable build and the Mac Homebrew bundle both list DNG/CR3. CI fails if not.
- **Preview slot and file info:** the `rawler` crate (dnglab's library, LGPL-2.1, fine with GPL-3). It gives the embedded JPEG plus megapixels and camera for the hub. *Check:* CR3 preview support.
- **EXIF/GPS** are copied into JPG/PNG/TIFF/AVIF with a Rust EXIF crate. *Check:* which crate writes all four.
- **Compress to a size:** develop to a 16-bit temporary TIFF, then reuse the picture path in size.rs (JPG, look-the-same first).
- **Batches:** RAW develops are heavy, so 2 at once instead of 4.

## Command line

```
convertino <files…> --to <target>      jpg, mp3, pdf, compress, split, merge, extract, 720p, frames …
          [--size 25MB] [--quality small|balanced|best] [--out <folder>]
          [--together] [--json] [--quiet]
convertino formats <file>              what the file can become
convertino tools [status | install <name> | install --all]
convertino mcp                         MCP server over stdio (AI apps start this)
convertino --version | --help
```

- Targets are the wheel's slot ids, with friendly aliases (jpeg → jpg, `--size` alone means Compress).
- Wildcards are expanded by Convertino itself, because PowerShell doesn't. Paths with spaces and non-English characters are supported.
- **Exit codes:** 0 all done, 1 some failed (a converter that couldn't download counts as a failure), 2 usage error (with a suggestion), 130 cancelled.
- **Ctrl+C** stops the converter processes (procs.rs) and leaves no temporary files.
- Same output rules as the wheel; nothing is ever overwritten. `--json` has `"schema": 1` and stays stable.
- Works with or without the app running.

## MCP server

- `convertino mcp`: a stdio server, written directly (JSON-RPC lines; protocol versions 2025-11-25 back to 2024-11-05) instead of the `rmcp` SDK: no async runtime, fully under test, and checked against the official MCP Inspector.
- **Tools:**
  - `list_conversions(paths)`: what each file can become.
  - `convert(paths, to, options)`
  - `compress_to_size(paths, size, together?)`
  - `converters_status`
  - (file details are part of `list_conversions`; split/merge/extract are `convert` targets, so no separate tools)
- Progress notifications and cancellation are supported. Tool annotations say outputs are new files and nothing is destructive.
- **Paths:** must be absolute (`~` is allowed). A relative path is refused with a clear message.
- **Master switch off:** every tool replies "AI apps are turned off in Convertino Settings".
- **Corner card:** the MCP process tells the running app through a local pipe/socket. If the app isn't running, the job still runs, just without a card.
- **Job log:** `mcp-jobs.jsonl` in the app data folder feeds "Recent AI jobs".
- **Connect writes the app's config:**
  - Claude Desktop: `%APPDATA%\Claude\claude_desktop_config.json`, or the Microsoft Store path under `%LOCALAPPDATA%\Packages\Claude_*\LocalCache\Roaming\Claude`; on Mac, `~/Library/Application Support/Claude/`.
  - Claude Code: `claude mcp add --scope user convertino -- <cli> mcp` when `claude` is on PATH, otherwise `~/.claude.json`.
  - Cursor: `~/.cursor/mcp.json`.
  - How it writes: other servers are kept, a `.bak` is written before the first change, and a malformed file is never touched ("Open file" instead). At startup, Convertino re-points the config if the app has moved (Mac).

## Tests: so nothing breaks after publishing

**1. Unit tests** (`cargo test --lib`, Windows + both Macs in CI)

- CLI parsing table: every target id in formats.json, aliases, sizes ("25MB", "800 KB", "1.5GB"), bad input → exit 2 with a suggestion.
- `--json` schema snapshot.
- MCP `tools/list` snapshot (names + input schemas), so an accidental change fails CI. Each tool's input checks.
- Config merge: about 12 fixture files (missing, empty, other servers, BOM, CRLF, malformed, already connected with an old path, Store path). Connect, then Disconnect, gives back the original.
- Every RAW extension maps to the RAW family, and the wheel builds for each.

**2. Real conversions** (CI, sample files cached)

- **RAW sample set** from raw.pixls.us (CC0): phone DNG, Adobe-converted DNG, CR2, CR3, NEF, ARW, RAF (X-Trans), ORF, RW2, PEF, SRW, and one portrait-orientation shot. About 300 MB, kept in the Actions cache.
  - Each one is converted to every slot and to Compress to 2 MB.
  - Checks: the size matches the sensor, the orientation is right, it's not black or green (scored against the camera preview with SSIMULACRA2), EXIF date and camera are present, and the size target is reached.
- **CLI end-to-end** with the built program, not the library:
  - The whole smoke matrix through the CLI.
  - Exit codes; non-English names (`Café – मेनू.pdf`); spaces; wildcards.
  - Ctrl+C during a video leaves no FFmpeg running and no temporary files.
- **MCP end-to-end:**
  - Start `convertino mcp` and send real JSON-RPC: initialize, tools/list, every tool on real files.
  - Check that progress arrives, that cancel stops FFmpeg, that two calls at once both work, and that bad requests don't crash it.
  - Also run the official MCP Inspector CLI against it as an independent client.

**3. The installed product** (new CI job, on every build)

- **Windows:** run the real installer silently, open a new shell, check `convertino --version` is on PATH, convert a file, then MCP tools/list. Then uninstall and check the PATH entry and files are gone.
- **Mac:** mount the .dmg, copy the app to /Applications, run the CLI from inside the bundle and through the link, convert, MCP tools/list. Also with the quarantine flag set, like a downloaded copy.
- **Upgrade:** install v0.1.6, then the new build. Settings are kept, PATH isn't duplicated, and connected AI apps still point to a working path.

**4. Release gate**

- **Release candidate first:** tag `v0.2.0-rc.1` → published as a *pre-release*. The updater and the Download link use "latest", which skips pre-releases, so friends don't get it.
- **`release-verify.yml`** downloads the assets that were actually published and runs level 3 on them. It also checks that every URL in latest.json resolves and that the signatures verify against updater.pub.
- After you (and a Mac friend) try the RC, the final `v0.2.0` is tagged from the same commit, and release-verify runs again on the final downloads.
- `release.cmd` refuses to tag unless run-checks passed on this commit and the last CI run is green.

**By hand on your PC (about 15 min, once):** the RAW wheel on your own photos; Connect → ask Claude Desktop to convert something; a few CLI commands in PowerShell.

## Order

1. Engine split + CLI (Windows) + its tests.
2. RAW photos + sample-set tests.
3. MCP server + Connect + corner card + tests.
4. Installer PATH, Mac "Install command…", installed-product CI job.
5. release-verify, then v0.2.0-rc.1, then v0.2.0.

## How it was built (for later sessions)

- `engine.rs`: headless job runner (plan → run → events) used by jobs.rs (wheel), cli.rs and mcp.rs. `--out` uses a per-thread output override in convert.rs.
- `cli.rs` + `src/bin/convertino-cli.rs` (console program). Hidden `convertino setup add|remove <bin>` for the Windows installer (PATH via HKCU\Environment, shell.rs; remove also disconnects AI apps pointing there). settings.json is read-only from the command line (`settings::init_read_only`, `CONVERTINO_CONFIG_DIR` for tests).
- `raw.rs`: develop with ImageMagick's LibRaw (`dng:use-camera-wb`, 16-bit TIFF, auto-orient), rawler fallback (also JPEG XL DNGs), Camera JPG from rawler's preview (used only if ≥1200 px and ≥ half the sensor, else developed with a note), EXIF via little_exif (WebP written by our own VP8X code: little_exif can't convert simple WebP and once swapped a lossless WebP's size; every write is checked by reading the size back). RAW never "already fits" in Compress to a size. PNG/AVIF/PDF from RAW are 8-bit; TIFF 16-bit with 300 DPI.
- `mcp.rs`: tools list_conversions / convert / compress_to_size / converters_status; progress notifications; notifications/cancelled kills the job; live card events in `<config>/ai/live/<pid>.jsonl`, cancel files in `<config>/ai/cancel/<id>`, job log `<config>/ai/jobs.jsonl`. Card ids are (1<<40)+pid<<20+n.
- `connect.rs`: Connect/Disconnect for Claude Desktop (incl. the Microsoft Store copy), Claude Code (`claude mcp add --scope user` when found, else ~/.claude.json), Cursor (~/.cursor/mcp.json); keeps other servers, one `.convertino-backup`, never touches broken JSON; `repair()` at app start re-points entries if the app moved.
- `ai.rs`: app side: watches the live files and shows the corner card "via <app>", Settings commands. Settings fields `aiApps`, `aiCard`.
- Installer: NSIS hooks in installer/ui.nsh copy convertino-cli.exe to `<install>\bin\convertino.exe` (renaming a running old copy aside) and run `setup add`; uninstall runs `setup remove` unless it's part of an update.
- Mac: the command is `Contents/MacOS/convertino-cli`; current_exe is canonicalized so the /usr/local/bin link still finds the bundled converters.
- Found on the PC run: `convertino tools` was taken for a file when a "tools" folder existed in the current folder; subcommands now lose only to an actual file.
- CI: build.yml now also tests on Windows (converters cached per month), fetches RAW samples (ci/fetch-raw-samples.sh, from github.com/sdcb/Sdcb.LibRaw.TestData, SHA-256 checked), and a verify-install job installs the real installers (ci/verify-install-windows.ps1 over the previous release + uninstall; ci/verify-install-mac.sh). release.yml: `-` in the version makes a pre-release; release-verify.yml re-checks the published files and latest.json signatures (minisign). release.ps1 waits for a green Build run of the pushed commit before tagging.

- RAW photos + `convertino` command + MCP server (3 Oct 2026, pushed 9a92c0d; NOT released). Full notes: claude/convertino-raw-cli-mcp.md (also docs/PLAN-RAW-CLI-MCP.md). Mock-up approved ("good to go"): https://claude.ai/artifact/MbTKhjexfYJjFktUAJy4Ga. User choices: read all camera RAW (no DNG writing), CLI installed with the app, MCP built in + one-click Connect, AI apps may convert any file they name.
  - New modules: engine.rs (shared headless runner), cli.rs + bin/convertino-cli.rs, raw.rs (LibRaw via ImageMagick, rawler fallback + Camera JPG, EXIF via little_exif + own WebP VP8X writer), mcp.rs (own JSON-RPC stdio server, 4 tools), connect.rs (Claude Desktop/Claude Code/Cursor configs), shell.rs (PATH; Mac /usr/local/bin link), ai.rs (app side: "via …" corner cards, Settings › AI & CLI). rust-version 1.89 (rawler).
  - PC run-checks 3 Oct: 107 unit + 13 command-line + 6 MCP tests pass on Windows, smoke all green. RAW samples (public domain, sdcb/Sdcb.LibRaw.TestData on GitHub; raw.pixls.us is blocked by both proxies) in test-files/raw; ci/fetch-raw-samples.sh.
  - CI: Windows now runs all tests too; verify-install job installs the real installers (Windows over the previous release, PATH check, conversions, MCP Inspector, uninstall; Mac from the dmg through a /usr/local/bin link). release.yml: "-" in version → pre-release; release-verify.yml after every release (+ latest.json minisign check). release.ps1 waits for a green Build run of the pushed commit.
  - Cloud build tips: apt webkit2gtk dev libs; cross-checking Windows code isn't possible in the cloud (static.rust-lang.org blocked) — run run-checks on the PC.
