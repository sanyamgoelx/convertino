"""Decides whether a Build run may skip the tests with real conversions.

They are skipped only when everything since the last release tag is the
interface's own files (ui/, but not the libraries in ui/vendor), notes and
docs, or the version number in the five version files. Anything else (Rust
code, the converters, CI, a dependency) runs every test. Writes ui_only=true
or false to $GITHUB_OUTPUT and the reason to the run's summary.
"""
import json
import os
import re
import subprocess
import sys

VERSION_FILES = {"package.json", "package-lock.json", "src-tauri/Cargo.toml", "src-tauri/Cargo.lock", "src-tauri/tauri.conf.json"}


def git(*args):
    return subprocess.run(["git", *args], capture_output=True, text=True, check=True).stdout


def allowed(path):
    if path.startswith("ui/vendor/"):
        return False
    return path.startswith(("ui/", "docs/", "stats/")) or path.endswith(".md") or path in (".github/stats-exclude.json", "LICENSE")


def app_version(rev):
    try:
        return json.loads(git("show", f"{rev}:src-tauri/tauri.conf.json"))["version"]
    except Exception:
        return None


def version_only(path, base, versions):
    """Every changed line is a version line set to the old or new app version."""
    diff = git("diff", "--unified=0", base, "HEAD", "--", path)
    for line in diff.splitlines():
        if line.startswith(("+++", "---", "@@", "diff ", "index ")) or not line[:1] in "+-":
            continue
        m = re.match(r'^[+-]\s*"?version"?\s*[:=]\s*"([^"]+)"', line)
        if not m or m.group(1) not in versions:
            return False
    return True


def decide():
    if os.environ.get("GITHUB_EVENT_NAME") == "workflow_dispatch":
        return False, "started by hand: every test runs"
    try:
        base = git("describe", "--tags", "--abbrev=0", "--match", "v*", "HEAD").strip()
    except subprocess.CalledProcessError:
        return False, "no earlier release tag to compare with"
    files = [f for f in git("diff", "--name-only", base, "HEAD").splitlines() if f]
    if not files:
        return False, f"nothing changed since {base}"
    versions = {v for v in (app_version(base), app_version("HEAD")) if v}
    for f in files:
        if f in VERSION_FILES:
            if not version_only(f, base, versions):
                return False, f"{f} changed beyond the version number (since {base})"
        elif not allowed(f):
            return False, f"{f} changed (since {base})"
    return True, f"only the interface, docs and version changed since {base}: {', '.join(files)}"


def main():
    skip, why = decide()
    print(("Skipping" if skip else "Running") + " the tests with real conversions: " + why)
    for var, text in (("GITHUB_OUTPUT", f"ui_only={'true' if skip else 'false'}\n"), ("GITHUB_STEP_SUMMARY", f"**Conversion tests {'skipped' if skip else 'run'}**: {why}\n")):
        if os.environ.get(var):
            with open(os.environ[var], "a", encoding="utf-8") as fh:
                fh.write(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
