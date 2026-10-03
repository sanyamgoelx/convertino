#!/usr/bin/env bash
# Installs Convertino from a .dmg the way a person does (copy to Applications),
# then checks the command inside it, through a /usr/local/bin link like
# "Install command…" makes, real conversions with the bundled converters, and
# the MCP server.
#   verify-install-mac.sh <Convertino.dmg> <version>
set -euo pipefail
dmg="$1"; version="$2"
here="$(cd "$(dirname "$0")" && pwd)"
fixtures="$here/../src-tauri/tests/fixtures"
fail() { echo "::error::$*"; exit 1; }
ok() { echo "  ok  $*"; }

mnt="$(mktemp -d)"
hdiutil attach -nobrowse -readonly -mountpoint "$mnt" "$dmg" >/dev/null
rm -rf /Applications/Convertino.app
cp -R "$mnt/Convertino.app" /Applications/
hdiutil detach "$mnt" >/dev/null
ok "copied to Applications"

app=/Applications/Convertino.app
cli="$app/Contents/MacOS/convertino-cli"
[ -x "$cli" ] || fail "no $cli"
[ -d "$app/Contents/Resources/tools" ] || fail "no bundled converters"
codesign --verify --deep "$app" 2>&1 || echo "::warning::codesign --verify complained (ad-hoc signature)"
[ "$("$cli" --version)" = "convertino $version" ] || fail "version: $("$cli" --version)"
ok "convertino --version = convertino $version"

# Like Settings › Install command…
sudo mkdir -p /usr/local/bin
sudo ln -sf "$cli" /usr/local/bin/convertino
hash -r
[ "$(convertino --version)" = "convertino $version" ] || fail "the link doesn't work"
ok "works through /usr/local/bin/convertino"

work="$(mktemp -d)"
cp "$fixtures/gradient.png" "$work/Café – photo.png"
cp "$fixtures/page.pdf" "$work/page.pdf"
cp "$fixtures/people.csv" "$work/people.csv"
# Run through the link: the bundled converters must still be found.
for job in "Café – photo.png:jpg:Café – photo.jpg" "page.pdf:png:page.png" "people.csv:xlsx:people.xlsx"; do
  IFS=: read -r src to made <<<"$job"
  out="$(convertino "$work/$src" --to "$to" --json)" || fail "$src -> $to: $out"
  echo "$out" | grep -q '"ok":true' || fail "$src -> $to: $out"
  [ -e "$work/$made" ] || fail "no $made"
  ok "$src -> $made"
done

tools="$(npx -y @modelcontextprotocol/inspector@latest --cli "$cli" mcp --method tools/list)"
echo "$tools" | grep -q compress_to_size || fail "MCP tools/list: $tools"
ok "MCP server answers (Inspector)"

# A downloaded copy carries the quarantine flag; the command must still start
# once the app was allowed (Open Anyway). Reported, not failed: CI can't click.
xattr -w com.apple.quarantine "0083;$(printf %x "$(date +%s)");Safari;" "$cli" || true
if "$cli" --version >/dev/null 2>&1; then ok "starts with the quarantine flag set"; else echo "::warning::quarantined command didn't start (expected until Open Anyway)"; fi
xattr -d com.apple.quarantine "$cli" 2>/dev/null || true
echo "Mac install checks passed."
