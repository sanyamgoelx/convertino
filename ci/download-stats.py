#!/usr/bin/env python3
"""Download counts for each release, without test downloads.

GitHub counts every download of a release file, including the ones our own
checks make. This works out the real number for each release and system:

    real = GitHub's count - CI downloads - listed test downloads

CI downloads are read from GitHub's records of the Actions jobs that fetch
release files (see RULES). Each job is counted once and kept in a ledger
(stats/ci-ledger.json on the stats branch), so the numbers stay right after
GitHub deletes old runs. Test downloads made by hand or by the developer's own
copy of Convertino are listed in .github/stats-exclude.json.

    download-stats.py --ledger old.json --out downloads.json --new-ledger new.json
    (needs the gh CLI with a token; GITHUB_REPOSITORY picks the repo)
    download-stats.py --dump DIR ...   reads saved API answers instead (testing)
"""
import argparse, datetime, json, os, subprocess, sys

BUCKETS = ["windows", "macArm", "macIntel", "macUpdates", "updateChecks"]
WORKFLOWS = {"Build", "Release", "Release check"}


def bucket(name):
    if name.endswith(".sig"):
        return None
    if name.endswith("-setup.exe") or name.endswith(".msi"):
        return "windows"
    if name.endswith("aarch64.dmg"):
        return "macArm"
    if name.endswith("x64.dmg"):
        return "macIntel"
    if name.endswith(".app.tar.gz"):
        return "macUpdates"
    if name == "latest.json":
        return "updateChecks"
    return None


class GitHub:
    def __init__(self, repo, dump=None):
        self.repo, self.dump = repo, dump

    def api(self, path):
        out = subprocess.run(["gh", "api", f"repos/{self.repo}/{path}"], check=True,
                             capture_output=True, text=True).stdout
        return json.loads(out)

    def releases(self):
        if self.dump:
            return json.load(open(os.path.join(self.dump, "releases.json"), encoding="utf-8-sig"))
        return self.api("releases?per_page=100")

    def runs(self, since):
        if self.dump:
            pages = json.load(open(os.path.join(self.dump, "runs.json"), encoding="utf-8-sig"))
            return [r for p in pages for r in p["workflow_runs"]]
        runs, page = [], 1
        while True:
            got = self.api(f"actions/runs?per_page=100&page={page}&created=%3E%3D{since}")["workflow_runs"]
            runs += got
            if len(got) < 100:
                return runs
            page += 1

    def jobs(self, run_id, attempt):
        if self.dump:
            f = os.path.join(self.dump, "jobs", f"{run_id}-{attempt}.json")
            return json.load(open(f, encoding="utf-8-sig"))["jobs"] if os.path.exists(f) else []
        return self.api(f"actions/runs/{run_id}/attempts/{attempt}/jobs?per_page=100")["jobs"]


def stable_latest(releases, at, skip=None):
    """The release `gh release download` picks as latest at time `at`."""
    c = [r for r in releases if not r["draft"] and not r["prerelease"]
         and r["published_at"] and r["published_at"] <= at and r["tag_name"] != skip]
    return max(c, key=lambda r: r["published_at"])["tag_name"] if c else None


def assets_of(releases, tag):
    for r in releases:
        if r["tag_name"] == tag:
            return [a["name"] for a in r["assets"]]
    return []


# Which release files each CI step downloads. A job that fetches the same file
# twice within seconds is counted once by GitHub, so files are a set per job.
def ci_files(step, job, tag, releases):
    n, at = step["name"], step["started_at"]
    if n == "Previous release (Windows)":                    # build.yml: upgrade test
        p = stable_latest(releases, at)
        return {(p, "windows")} if p else set()
    if n == "Download the release and the one before it":    # release-verify.yml windows
        p = stable_latest(releases, at, skip=tag)
        return {(tag, "windows")} | ({(p, "windows")} if p else set())
    if n == "Download the release":                          # release-verify.yml mac
        return {(tag, "macArm" if "apple" in job["name"] else "macIntel")}
    if n == "latest.json and signatures":                    # ci/verify-latest-json.sh
        names = assets_of(releases, tag)
        files = {(tag, "updateChecks"), (tag, "windows")}
        files |= {(tag, "macUpdates#" + x) for x in names if x.endswith(".app.tar.gz")}
        return files
    return set()


def update_ledger(gh, releases, ledger):
    """Adds every finished job not yet in the ledger."""
    jobs_seen = ledger.setdefault("jobs", {})
    done = set(ledger.get("attemptsDone", []))
    last = ledger.get("checkedUpTo", "2026-09-01T00:00:00Z")
    since = (datetime.datetime.fromisoformat(last.replace("Z", "+00:00"))
             - datetime.timedelta(days=3)).strftime("%Y-%m-%d")
    newest_open = None
    for run in gh.runs(since):
        if run["name"] not in WORKFLOWS:
            continue
        tag = run["head_branch"]
        if not str(tag).startswith("v"):   # hand-started Release check: "Release check v0.2.2"
            tag = (run.get("display_title") or "").split()[-1] if run.get("display_title") else tag
        for attempt in range(1, run["run_attempt"] + 1):
            ra = f"{run['id']}-{attempt}"
            if ra in done:
                continue
            finished = attempt < run["run_attempt"] or run["status"] == "completed"
            for job in gh.jobs(run["id"], attempt):
                key = str(job["id"])
                if key in jobs_seen:
                    continue
                if job["status"] != "completed":
                    newest_open = min(newest_open or job["started_at"], job["started_at"])
                    continue
                files = set()
                for s in job.get("steps") or []:
                    if s.get("conclusion") in (None, "skipped") or not s.get("started_at"):
                        continue
                    files |= ci_files(s, job, tag, releases)
                jobs_seen[key] = {
                    "at": job["started_at"], "run": run["id"], "job": job["name"],
                    "files": sorted([t, b.split("#")[0]] for t, b in files if t),
                }
            if finished:
                done.add(ra)
    now = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    ledger["checkedUpTo"] = newest_open or now
    ledger["attemptsDone"] = sorted(done)
    return ledger


def raw_counts(releases):
    out = []
    for r in releases:
        if r["draft"]:
            continue
        c = dict(tag=r["tag_name"], publishedAt=r["published_at"], **{b: 0 for b in BUCKETS})
        for a in r["assets"]:
            b = bucket(a["name"])
            if b:
                c[b] += a["download_count"]
        out.append(c)
    return out


def excluded_until(ledger, manual, until=None):
    """{(tag, bucket): n} of test downloads made up to `until` (all if None)."""
    ex = {}
    for j in ledger.get("jobs", {}).values():
        if until and j["at"] > until:
            continue
        for t, b in j["files"]:
            ex[(t, b)] = ex.get((t, b), 0) + 1
    for e in manual:
        if until and e.get("at", "") > until:
            continue
        ex[(e["tag"], e["bucket"])] = ex.get((e["tag"], e["bucket"]), 0) + e.get("count", 1)
    return ex


def real_counts(raw, ex):
    rels = []
    for r in raw:
        c = {k: r[k] for k in ("tag", "publishedAt")}
        c.update({b: max(0, r[b] - ex.get((r["tag"], b), 0)) for b in BUCKETS})
        c["raw"] = {b: r[b] for b in BUCKETS}
        rels.append(c)
    return rels


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--ledger")
    ap.add_argument("--exclude", default=".github/stats-exclude.json")
    ap.add_argument("--out", default="downloads.json")
    ap.add_argument("--new-ledger", default="ci-ledger.json")
    ap.add_argument("--dump")
    a = ap.parse_args()
    gh = GitHub(os.environ.get("GITHUB_REPOSITORY", "sanyamgoelx/convertino"), a.dump)
    ledger = json.load(open(a.ledger, encoding="utf-8-sig")) if a.ledger and os.path.exists(a.ledger) else {}
    manual = json.load(open(a.exclude, encoding="utf-8-sig")).get("entries", []) if os.path.exists(a.exclude) else []
    releases = gh.releases()
    ledger = update_ledger(gh, releases, ledger)
    ex = excluded_until(ledger, manual)
    now = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    out = {"takenAt": now, "testDownloadsRemoved": True, "releases": real_counts(raw_counts(releases), ex)}
    json.dump(out, open(a.out, "w"), indent=1)
    json.dump(ledger, open(a.new_ledger, "w"), indent=1, sort_keys=True)
    print(json.dumps(out, indent=1))


if __name__ == "__main__":
    main()
