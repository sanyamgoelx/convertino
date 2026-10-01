# Replaces the published release of the current version with a fresh build of
# the latest code (only sensible before anyone has downloaded it).
$ErrorActionPreference = "Continue"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
Set-Location (Split-Path $PSScriptRoot -Parent)
$log = Join-Path (Get-Location) "release.log"
"Convertino redo release $(Get-Date -Format s)" | Set-Content -Path $log -Encoding utf8
$gh = "$env:ProgramFiles\GitHub CLI\gh.exe"
$version = (Get-Content "src-tauri\tauri.conf.json" -Raw | ConvertFrom-Json).version
$tag = "v$version"
& $gh release delete $tag --repo sanyamgoelx/convertino --yes --cleanup-tag 2>&1 | ForEach-Object { "  $_" } | Add-Content -Path $log -Encoding utf8
git tag -d $tag 2>&1 | ForEach-Object { "  $_" } | Add-Content -Path $log -Encoding utf8
"Removed $tag; publishing it again" | Add-Content -Path $log -Encoding utf8
& powershell -NoProfile -ExecutionPolicy Bypass -File "$PSScriptRoot\release.ps1"
