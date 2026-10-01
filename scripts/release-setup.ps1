# One time: makes the key that signs updates, keeps it in your user folder
# (%USERPROFILE%\.convertino), puts the public half in the project, and gives
# the private half to GitHub (as Actions secrets) so release builds can sign.
# Keep a copy of %USERPROFILE%\.convertino somewhere safe: without that key,
# copies already installed can't update to newer versions.
$ErrorActionPreference = "Continue"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
Set-Location (Split-Path $PSScriptRoot -Parent)
$log = Join-Path (Get-Location) "release-setup.log"
function Say($m) { Write-Host $m; Add-Content -Path $log -Value $m -Encoding utf8 }
"Convertino release setup $(Get-Date -Format s)" | Set-Content -Path $log -Encoding utf8

$gh = "$env:ProgramFiles\GitHub CLI\gh.exe"
$repo = "sanyamgoelx/convertino"
$dir = Join-Path $env:USERPROFILE ".convertino"
$key = Join-Path $dir "updater.key"
$pwFile = Join-Path $dir "updater.password.txt"

if (-not (Test-Path $key)) {
  New-Item -ItemType Directory -Force $dir | Out-Null
  $pw = [guid]::NewGuid().ToString("N")
  Set-Content -Path $pwFile -Value $pw -NoNewline -Encoding ascii
  npx tauri signer generate --ci -p $pw -w $key 2>&1 | ForEach-Object { Say "  $_" }
  if (-not (Test-Path $key)) { Say "COULDN'T MAKE THE KEY"; exit 1 }
  Say "Made the update-signing key in $dir"
} else {
  Say "Using the existing key in $dir"
}
$pw = (Get-Content $pwFile -Raw).Trim()

Copy-Item "$key.pub" (Join-Path (Get-Location) "src-tauri\updater.pub") -Force
Say "Public key copied to src-tauri\updater.pub"

Get-Content $key -Raw | & $gh secret set TAURI_SIGNING_PRIVATE_KEY --repo $repo 2>&1 | ForEach-Object { Say "  $_" }
$pw | & $gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD --repo $repo 2>&1 | ForEach-Object { Say "  $_" }
$names = & $gh secret list --repo $repo 2>&1
Say "GitHub secrets now: $($names -join ', ')"
if (($names -join " ") -match "TAURI_SIGNING_PRIVATE_KEY_PASSWORD") { Say "RELEASE SETUP DONE" } else { Say "SECRETS NOT SET" }
