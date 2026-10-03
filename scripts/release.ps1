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

# Only a commit whose Build run passed on all three machines (tests, installers,
# installing them) gets released. Waits for the run the push above started.
$sha = (git rev-parse HEAD).Trim()
Say "Waiting for the Build checks of $($sha.Substring(0, 7)) (tests on Windows and Mac, installing the installers)..."
$deadline = (Get-Date).AddMinutes(75)
while ($true) {
  $runs = gh run list --repo sanyamgoelx/convertino --workflow build.yml --commit $sha --json status,conclusion,url --limit 1 2>$null | ConvertFrom-Json
  if ($runs -and $runs[0].status -eq "completed") {
    if ($runs[0].conclusion -eq "success") { Say "Build checks passed."; break }
    Say "Build checks FAILED ($($runs[0].conclusion)): $($runs[0].url)"
    Say "Not releasing. Fix it (ci-log.cmd saves the log), then run release.cmd again."
    exit 1
  }
  if ((Get-Date) -gt $deadline) { Say "Build checks didn't finish in 75 minutes; not releasing."; exit 1 }
  Start-Sleep -Seconds 60
}
if ($version -match "-") { Say "$tag is a test release (pre-release): installed copies won't update to it." }
git tag -a $tag -m "Convertino $version" 2>&1 | ForEach-Object { Say "  $_" }
git push origin $tag 2>&1 | ForEach-Object { Say "  $_" }
if ($LASTEXITCODE -ne 0) { Say "TAG PUSH FAILED"; exit 1 }
Say "RELEASING ${tag}: https://github.com/sanyamgoelx/convertino/actions (then https://github.com/sanyamgoelx/convertino/releases)"
Say "After publishing, the 'Release check' run installs the published files on Windows and both Macs."
