# Convertino: RAW photos, command line and MCP (plan)

3 Oct 2026. Status: **plan and mock-up waiting for approval.** Nothing has been built yet.
Mock-up (Design canvas): https://claude.ai/artifact/MbTKhjexfYJjFktUAJy4Ga

## Decisions (user, 3 Oct)

- **RAW:** read every common camera RAW format (DNG, CR2/CR3, NEF, ARW, RAF, ORF, RW2 and others) and convert it out. Convertino won't make DNG files.
- **CLI:** a `convertino` command that comes with the app (on PATH on Windows; on Mac, Settings has "Install command…").
- **MCP:** built in (`convertino mcp`), with one-click Connect for Claude Desktop, Claude Code and Cursor in Settings.
- **MCP access:** an AI app can convert any file it names. The wheel's rules still apply: output goes next to the source, originals are never changed or overwritten, and jobs are listed in Settings.

## What changes on screen (see the canvas)

1. **RAW photo wheel** (new family, teal #0E7C86): JPG, PNG, TIFF (16-bit), AVIF, Compress, PDF, WEBP, **Preview** (pulls out the camera's own full-size JPEG; instant). The hub shows "RAW photo · 24 MP".
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
- **Exit codes:** 0 all done, 1 some failed, 2 usage error (with a suggestion), 3 converter missing and couldn't be downloaded, 130 cancelled.
- **Ctrl+C** stops the converter processes (procs.rs) and leaves no temporary files.
- Same output rules as the wheel; nothing is ever overwritten. `--json` has `"schema": 1` and stays stable.
- Works with or without the app running.

## MCP server

- `convertino mcp`: a stdio server built on the official Rust SDK (`rmcp`).
- **Tools:**
  - `list_conversions(paths)`: what each file can become.
  - `convert(paths, to, options)`
  - `compress_to_size(paths, size, together?)`
  - `file_info(paths)`
  - `pdf_pages(path, split | merge | extract pages)`
  - `converters_status`
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
