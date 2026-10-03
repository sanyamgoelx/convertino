#!/usr/bin/env bash
# Checks a published release's latest.json the way installed copies use it:
# every platform's download exists and its signature verifies with the
# updater's public key (src-tauri/tauri.conf.json).
#   verify-latest-json.sh <tag>      (needs gh, jq, minisign)
set -euo pipefail
tag="$1"
here="$(cd "$(dirname "$0")" && pwd)"
work="$(mktemp -d)"
fail() { echo "::error::$*"; exit 1; }
gh release download "$tag" --repo "${GITHUB_REPOSITORY:-sanyamgoelx/convertino}" --pattern latest.json --dir "$work" --clobber
json="$work/latest.json"
want="${tag#v}"
[ "$(jq -r .version "$json")" = "$want" ] || fail "latest.json says $(jq -r .version "$json"), expected $want"
jq -r .plugins.updater.pubkey "$here/../src-tauri/tauri.conf.json" | base64 -d > "$work/updater.pub"
for p in $(jq -r '.platforms | keys[]' "$json"); do
  url="$(jq -r ".platforms[\"$p\"].url" "$json")"
  jq -r ".platforms[\"$p\"].signature" "$json" | base64 -d > "$work/$p.sig"
  curl -fsSL --retry 3 -o "$work/$p.bin" "$url" || fail "$p: can't download $url"
  minisign -Vm "$work/$p.bin" -x "$work/$p.sig" -p "$work/updater.pub" >/dev/null || fail "$p: signature doesn't verify"
  echo "  ok  $p: $(basename "$url") downloads and its signature verifies"
done
for need in windows-x86_64 darwin-aarch64 darwin-x86_64; do
  jq -e ".platforms[\"$need\"]" "$json" >/dev/null || fail "latest.json has no $need"
done
echo "latest.json checks passed."
