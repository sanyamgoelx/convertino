# How Crofty's tools work together

Applies to Convertino, Drive Cleanup, Trimothy, Prisca and Spoilaint (and any new tool in `F:\Tool`).
Decided 13 Oct 2026. Every tool may be public, so the rules are:

1. **Every tool works on its own.** Another tool is used only when it is installed, and only for the job it owns.
2. **One owner per job.** Convertino changes media (convert, compress, check a copy). Drive Cleanup knows what is on the disks and empties the Holding Area. Trimothy reviews and tags game clips. Prisca scans.
3. **When a task needs another tool, the tool offers to get it, in place** (see "Getting another tool"). It never fails silently or sends the person to a website when it can install the tool itself.
4. **Removed means the Holding Area**, in every tool (see below). Never the Recycle Bin: Windows deletes files too big for it without asking.
5. The honesty rule applies across tools: a tool only says another tool did something after that tool reported it.

## Finding each other

Each tool writes a small file about itself every time it starts:

`%LOCALAPPDATA%\sanyamgoelx\tools\<id>.json` (Mac: `~/Library/Application Support/sanyamgoelx/tools/<id>.json`)

```json
{
  "id": "convertino",
  "name": "Convertino",
  "version": "0.2.9",
  "app": "C:\\Users\\me\\AppData\\Local\\Convertino\\Convertino.exe",
  "cli": "C:\\Users\\me\\AppData\\Local\\Convertino\\convertino.exe",
  "can": ["convert", "compress", "check-copy", "hold", "compare"],
  "repo": "sanyamgoelx/convertino",
  "written": 1791867100
}
```

- Readers check that `app`/`cli` still exist and ask the CLI for its version (`--version`) when the version matters. A file is a hint, never proof: tools update themselves while others run.
- No file, or its paths are gone: the tool counts as not installed. Fallbacks (PATH, old fixed paths) stay only for tools that already used them.
- Writing the file is best effort; a tool never fails to start because it can't.

## Getting another tool

Each tool carries a small built-in list of its siblings (id, name, one line about it, repo, installer name pattern, update public key, public yes/no). Only public tools are ever offered.

When a task needs a sibling that isn't installed:
1. Say so where the task is, in one line, with the size: "Shrinking videos needs Convertino (free, about 15 MB)." Buttons: **Get Convertino** · Not now.
2. **Get** downloads the latest release's installer from GitHub with progress shown on the same spot, checks its signature against the sibling's update key (the same key its own auto-update uses), runs the installer silently, waits for the sibling's tool file to appear, then carries on with the task. Cancel stops the download and deletes the partial file.
3. Any failure says what happened and offers **Open the download page** as the fallback.
4. "Not now" is remembered for that task until the next time the person starts it. Never nag on startup.

## The Holding Area

Originals that a tool replaces or removes go to the Holding Area on the same drive, so the move is instant and nothing is lost:

- Files under the user folder (`%USERPROFILE%`): `%USERPROFILE%\_Quarantine\<batch>\…`
- Files on any other drive: `X:\_Quarantine\<batch>\…`
- Mac: `~/_Quarantine/<batch>/…` for the home folder, `/Volumes/<disk>/_Quarantine/<batch>/…` for other disks.

The folder is named `_Quarantine` for compatibility with Drive Cleanup; on screen it is always called **the Holding Area**.

`<batch>` is the start time (`2026-10-13-09-41-07`) plus, for tools other than Drive Cleanup, ` <tool id>` (`2026-10-13-09-41-07 convertino`). Inside it, each item keeps its path relative to the Holding Area's parent. Each batch has a `manifest.json`:

```json
{
  "batch": "2026-10-13-09-41-07 convertino",
  "kind": "convertino",
  "createdAt": 1791867100941,
  "entries": [
    { "from": "D:\\Clips\\a.mov", "to": "D:\\_Quarantine\\2026-10-13-09-41-07 convertino\\Clips\\a.mov",
      "bytes": 2040000000, "isDir": false, "label": "Replaced by its compressed copy",
      "replaced": "D:\\Clips\\a.mp4" }
  ]
}
```

- `replaced`: the file now standing in for the original (Drive Cleanup's Put back keeps it as "(compressed)").
- The tool that made a batch may undo it; it then removes its entries and deletes the batch when empty.
- Only Drive Cleanup (or the person) empties the Holding Area. Other tools say where it is and that Drive Cleanup can review it.
- Every tool skips `_Quarantine` folders when it scans.

## Checking a copy before it replaces an original

Convertino owns this check (`verify.rs`, built 13 Oct 2026); other tools call Convertino rather than repeating it. An original is only replaced after the copy passes:

- **Source first (video):** the original is read packet by packet without decoding pictures (`ffprobe -show_entries packet=flags` or `ffmpeg -map 0:v:0 -c copy -f null -`; both report the same parser errors); a recording with broken sections (3+ damage lines, as NVIDIA DVR clips can have) is left exactly as it is.
- Same length (within 1 s or 0.2%), picture kept, sound kept if there was any.
- Same picture shape (aspect within 1%).
- 8 frames compared with the original (SSIM on small greyscale frames ≥ 0.6; good heavy compression scores 0.83–0.93, a rotated copy 0.20).
- Plays cleanly for 4 s at the start, middle and end.

A copy that fails is deleted and the original stays where it was, with the reason shown.

## Clip record (Trimothy ↔ Drive Cleanup)

Built 13 Oct 2026. Trimothy owns it; Drive Cleanup only reads it.
- Trimothy's entry in the tools folder lists `libraries` (recent folders holding a `cliptrim-index.json`).
- Each index is `{ clips: { <uid>: { path, tags, ... } } }`; every trimmed file also carries `comment=cliptrim/1 id=<uid> tags=…`.
- **Tags protect a clip from removal, not from compression**: Drive Cleanup shows the tags on Shrink rows, never
  suggests a tagged clip (or a folder holding one) for removal, and may compress it.
- A compressed clip keeps its uid, tags and marker (Trimothy re-stamps the copy), so it stays the same library clip.

## Who writes what into the Holding Area

| Tool | Batch name | Labels |
| --- | --- | --- |
| Drive Cleanup | `<time>` (removals), `shrink-<time>` with `"kind": "shrink"` (compressed videos) | its card labels; `shrink` |
| Convertino | `<time> convertino`, `watched <folder> convertino` | Replaced by Convertino |
| Trimothy | `<time> trimothy` (one per run) | Original, replaced by its trim · Rejected in Trimothy · Removed from the Trimothy library · Trim, replaced by its compressed copy |

## Comparing a copy with its original (Convertino ↔ Drive Cleanup)

Built 14 Oct 2026. Convertino owns comparing (one owner per job); Drive Cleanup owns deleting from the Holding Area.

- **Finding pairs** works by a manifest's contents, never its batch name: an entry with `replaced` pairs the original
  (`to`) with the copy in its place. Also "clip (compressed).mp4" (or "clip (10 MB).mp4") next to "clip.mp4". Both files must exist.
- **Convertino's wheel** gets a Compare slot (last on the ring) for a video, picture or PDF that has a pair, several selected
  files, a folder (pairs inside it, subfolders too) or a Holding Area batch folder (only Zip and Compare there).
  Its entry in the tools folder lists `"compare"` in `can`.
- **Shared "Looks good" mark:** `%LOCALAPPDATA%\sanyamgoelx\compare-marks.json` (Mac: `~/Library/Application Support/sanyamgoelx/`),
  `{ "schema": 1, "marks": { "<lower-case path of the original>": { "at": <ms>, "by": "convertino" | "drive-cleanup", "copy": "<path>" } } }`.
  A mark means "the original can be deleted". Either tool sets or clears it; Drive Cleanup lists marked originals under
  Compressed Videos › Checked and deletes them. Drive Cleanup moved its old `data/shrink-checked.json` in once.
- **Putting back:** only the tool that made the batch. In Convertino's Compare, Drive Cleanup's pairs get
  "Put back or delete in Drive Cleanup", which starts Drive Cleanup with `--compare "<original>"` (its tools-folder entry's `app`).
- **One way in:** while Convertino's entry has `"compare"`, Drive Cleanup leaves out its own Explorer entry
  "Compare with compressed copy" (it comes back by itself when Convertino is gone).

## Asking Convertino to check

`convertino check <video> [copy] [--json]` (Convertino 0.2.9+): one file = is it sound (no broken sections);
two files = does the copy look, sound and play like the original. Exit 0 = ok, 1 = not (`{schema, ok, why}`).
