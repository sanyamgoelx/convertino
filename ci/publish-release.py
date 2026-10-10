#!/usr/bin/env python3
"""Publishes a GitHub Release from the installers a passed Build run already made.

Build makes and signs (for the updater) the same installers Release used to
build a second time, so Release only gathers them, names them the way
tauri-action did, writes latest.json and uploads everything.

  publish-release.py <tag> <folder with convertino-windows/, convertino-macos-apple-silicon/, convertino-macos-intel/> [--dry-run]

Asset names and latest.json match what tauri-action published up to v0.2.4,
so installed copies update exactly as before.
"""
import datetime
import json
import pathlib
import subprocess
import sys
import time

REPO_ENV = "GITHUB_REPOSITORY"


def one(folder: pathlib.Path, pattern: str) -> pathlib.Path:
    hits = sorted(p for p in folder.rglob(pattern) if p.is_file())
    if len(hits) != 1:
        sys.exit(f"Expected one {pattern} in {folder}, found {[str(h) for h in hits]}")
    return hits[0]


def gather(tag: str, root: pathlib.Path):
    """[(file, published name)] and the latest.json platforms."""
    version = tag[1:] if tag.startswith("v") else tag
    files, platforms = [], {}
    base = f"https://github.com/{repo()}/releases/download/{tag}"

    win = root / "convertino-windows"
    setup = one(win, f"*_{version}_x64-setup.exe")
    setup_sig = one(win, f"*_{version}_x64-setup.exe.sig")
    files += [(setup, setup.name), (setup_sig, setup_sig.name)]
    entry = {"signature": setup_sig.read_text().strip(), "url": f"{base}/{setup.name}"}
    platforms["windows-x86_64"] = entry
    platforms["windows-x86_64-nsis"] = entry

    for folder, arch, key in (("convertino-macos-apple-silicon", "aarch64", "darwin-aarch64"), ("convertino-macos-intel", "x64", "darwin-x86_64")):
        mac = root / folder
        dmg = one(mac, f"*_{version}_{arch}.dmg")
        tgz = one(mac, "*.app.tar.gz")
        tgz_sig = one(mac, "*.app.tar.gz.sig")
        stem = tgz.name[: -len(".app.tar.gz")]  # "Convertino"
        name = f"{stem}_{arch}.app.tar.gz"
        files += [(dmg, dmg.name), (tgz, name), (tgz_sig, name + ".sig")]
        entry = {"signature": tgz_sig.read_text().strip(), "url": f"{base}/{name}"}
        platforms[key] = entry
        platforms[key + "-app"] = entry
    return version, files, platforms


def repo() -> str:
    import os
    return os.environ.get(REPO_ENV, "sanyamgoelx/convertino")


def gh(*args, tries=4):
    for t in range(1, tries + 1):
        r = subprocess.run(["gh", *args], capture_output=True, text=True)
        if r.returncode == 0:
            return r.stdout
        print(f"gh {' '.join(args[:3])} failed (try {t} of {tries}): {r.stderr.strip()[-300:]}", flush=True)
        if t < tries:
            time.sleep(10 * t)
    sys.exit(f"gh {' '.join(args)} kept failing")


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    dry = "--dry-run" in sys.argv
    if len(args) != 2:
        sys.exit(__doc__)
    tag, root = args[0], pathlib.Path(args[1])
    if not tag.startswith("v"):
        sys.exit(f"{tag} isn't a version tag (v0.2.5); start the workflow from the tag.")
    here = pathlib.Path(__file__).parent
    notes = (here / "release-notes.md").read_text(encoding="utf-8").strip()
    version, files, platforms = gather(tag, root)
    pub = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%S.%f")[:-3] + "Z"
    latest = {"version": version, "notes": notes, "pub_date": pub, "platforms": platforms}
    out = root / "latest.json"
    out.write_text(json.dumps(latest, indent=2, ensure_ascii=False), encoding="utf-8")
    files.append((out, "latest.json"))

    # Published names: copy each file to its name in one folder.
    pub_dir = root / "publish"
    pub_dir.mkdir(exist_ok=True)
    for src, name in files:
        (pub_dir / name).write_bytes(src.read_bytes())
    names = sorted(p.name for p in pub_dir.iterdir())
    print("Files:", *names, sep="\n  ")
    if dry:
        print(out.read_text(encoding="utf-8"))
        return

    pre = "-" in version
    exists = subprocess.run(["gh", "release", "view", tag, "--repo", repo()], capture_output=True).returncode == 0
    if not exists:
        notes_file = root / "notes.md"
        notes_file.write_text(notes, encoding="utf-8")
        cmd = ["release", "create", tag, "--repo", repo(), "--verify-tag", "--title", f"Convertino {tag}", "--notes-file", str(notes_file)]
        cmd += ["--prerelease", "--latest=false"] if pre else ["--latest"]
        gh(*cmd)
    # One file per upload, retried: a GitHub hiccup costs one file, not the release.
    for name in names:
        gh("release", "upload", tag, str(pub_dir / name), "--repo", repo(), "--clobber")
        print("uploaded", name, flush=True)
    print(f"Published {tag}{' (pre-release)' if pre else ''}: https://github.com/{repo()}/releases/tag/{tag}")


if __name__ == "__main__":
    main()
