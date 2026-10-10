# Puts Convertino on GitHub as a public repository (one time), then pushes.
# Sign-in uses the GitHub CLI: it shows a one-time code and opens github.com in your browser.
# No password is typed here, and the sign-in stays on this PC.
$ErrorActionPreference = "Continue"  # git and gh write progress to stderr
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
Set-Location (Split-Path $PSScriptRoot -Parent)
$log = Join-Path (Get-Location) "logs\github.log"
function Say($m) { Write-Host $m; Add-Content -Path $log -Value $m -Encoding utf8 }
"Convertino GitHub $(Get-Date -Format s)" | Set-Content -Path $log -Encoding utf8
$env:GIT_TERMINAL_PROMPT = "0"

if (-not (Get-Command git -ErrorAction SilentlyContinue)) { Say "Git isn't installed. Run setup-windows.cmd first."; exit 1 }

# GitHub CLI: installed with winget the first time.
function Find-Gh {
  $c = Get-Command gh -ErrorAction SilentlyContinue
  if ($c) { return $c.Source }
  foreach ($p in @("$env:ProgramFiles\GitHub CLI\gh.exe", "$env:LOCALAPPDATA\Programs\GitHub CLI\gh.exe")) { if (Test-Path $p) { return $p } }
  return $null
}
$gh = Find-Gh
if (-not $gh) {
  Say "Installing the GitHub CLI (free, from Microsoft's winget)..."
  winget install --id GitHub.cli -e --silent --accept-source-agreements --accept-package-agreements 2>&1 | ForEach-Object { Say "  $_" }
  $gh = Find-Gh
  if (-not $gh) { Say "The GitHub CLI didn't install. Install it from https://cli.github.com and run this again."; exit 1 }
}
Say "GitHub CLI: $gh"

& $gh auth status --hostname github.com *> $null
if ($LASTEXITCODE -ne 0) {
  Write-Host ""
  Write-Host "Signing in to GitHub: copy the one-time code shown below, press Enter,"
  Write-Host "and paste it in the browser page that opens."
  & $gh auth login --hostname github.com --git-protocol https --web
  if ($LASTEXITCODE -ne 0) { Say "GitHub sign-in didn't finish."; exit 1 }
}
& $gh auth setup-git --hostname github.com | Out-Null
$user = (& $gh api user --jq .login).Trim()
Say "Signed in as: $user"
git config --global github.user $user

if (-not (Test-Path ".git")) { git init -b main | Out-Null; Say "Made a git repository here." }
# Commits here are always credited to the signed-in account (repo setting,
# so another identity in the global git config isn't used for Convertino).
$uid = (& $gh api user --jq .id).Trim()
git config user.name $user
git config user.email "$uid+$user@users.noreply.github.com"
git config core.autocrlf true

# A leftover lock (an interrupted git command) would stop the commit.
if (Test-Path ".git\index.lock") { Remove-Item ".git\index.lock" -Force -ErrorAction SilentlyContinue }
# This PC's own update checks and updates are left out of the download counts.
& powershell -NoProfile -ExecutionPolicy Bypass -File "$PSScriptRoot\stats-own.ps1" 2>&1 | ForEach-Object { Say "  $_" }
git add -A
$pending = git status --porcelain
if ($pending) {
  git commit -q -m "Convertino: latest changes" | Out-Null
  Say "Committed $(@($pending).Count) file(s)."
}

$url = "https://github.com/$user/convertino.git"
if (git remote 2>$null | Select-String -SimpleMatch "origin") { git remote set-url origin $url } else { git remote add origin $url }

& $gh repo view "$user/convertino" *> $null
if ($LASTEXITCODE -ne 0) {
  Say "Creating the public repository $user/convertino..."
  & $gh repo create "$user/convertino" --public --description "Convert any file from a radial wheel in Explorer or Finder" 2>&1 | ForEach-Object { Say "  $_" }
  if ($LASTEXITCODE -ne 0) { Say "COULDN'T CREATE THE REPOSITORY"; exit 1 }
}

Say "Pushing to $url ..."
git push -u origin main 2>&1 | ForEach-Object { Say "$_" }
if ($LASTEXITCODE -ne 0) { Say "PUSH FAILED (exit $LASTEXITCODE)"; exit $LASTEXITCODE }
Say "PUSHED: https://github.com/$user/convertino"
Say "Builds: https://github.com/$user/convertino/actions"
