# Publishes the version in src-tauri\tauri.conf.json: pushes the latest code,
# then the tag v<version>, which starts the Release build on GitHub.
$ErrorActionPreference = "Continue"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
Set-Location (Split-Path $PSScriptRoot -Parent)
$log = Join-Path (Get-Location) "release.log"
function Say($m) { Write-Host $m; Add-Content -Path $log -Value $m -Encoding utf8 }
"Convertino release $(Get-Date -Format s)" | Set-Content -Path $log -Encoding utf8
$env:GIT_TERMINAL_PROMPT = "0"

$version = (Get-Content "src-tauri\tauri.conf.json" -Raw | ConvertFrom-Json).version
$tag = "v$version"
Say "Version $version"

& powershell -NoProfile -ExecutionPolicy Bypass -File "$PSScriptRoot\publish-github.ps1"
git fetch --tags --quiet 2>&1 | Out-Null
if (git tag --list $tag) {
  Say "$tag was released already. Raise the version in src-tauri\tauri.conf.json (and package.json, Cargo.toml) first."
  exit 1
}
git tag -a $tag -m "Convertino $version" 2>&1 | ForEach-Object { Say "  $_" }
git push origin $tag 2>&1 | ForEach-Object { Say "  $_" }
if ($LASTEXITCODE -ne 0) { Say "TAG PUSH FAILED"; exit 1 }
Say "RELEASING ${tag}: https://github.com/sanyamgoelx/convertino/actions (then https://github.com/sanyamgoelx/convertino/releases)"
