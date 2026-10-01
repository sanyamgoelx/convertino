# Puts Convertino on GitHub as a public repository (one time), then pushes.
# Sign-in happens in your browser through Git Credential Manager; no password is typed here.
$ErrorActionPreference = "Continue"  # git writes progress to stderr
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
Set-Location (Split-Path $PSScriptRoot -Parent)
$log = Join-Path (Get-Location) "github.log"
function Say($m) { Write-Host $m; Add-Content -Path $log -Value $m -Encoding utf8 }
"Convertino GitHub $(Get-Date -Format s)" | Set-Content -Path $log -Encoding utf8

if (-not (Get-Command git -ErrorAction SilentlyContinue)) { Say "Git isn't installed. Run setup-windows.cmd first."; exit 1 }

$user = (git config --global github.user) 2>$null
if (-not $user) {
  Write-Host ""
  $user = Read-Host "Your GitHub user name"
  $user = $user.Trim()
  if (-not $user) { Say "No user name given."; exit 1 }
  git config --global github.user $user
}
Say "GitHub user: $user"

if (-not (Test-Path ".git")) {
  git init -b main | Out-Null
  Say "Made a git repository here."
}
# Commits show your GitHub private address, not your real e-mail, unless you've set one yourself.
if (-not (git config user.name)) { git config user.name $user }
if (-not (git config user.email)) { git config user.email "$user@users.noreply.github.com" }
git config core.autocrlf true

git add -A
$pending = git status --porcelain
if ($pending) {
  git commit -q -m "Convertino: wheel, conversions, PDF editor, Settings, smallest-file search" | Out-Null
  Say "Committed $(@($pending).Count) file(s)."
}

$url = "https://github.com/$user/convertino.git"
if (-not (git remote 2>$null | Select-String -SimpleMatch "origin")) { git remote add origin $url }

# Does the repository exist yet?
$exists = $true
try { Invoke-WebRequest -UseBasicParsing -Method Head "https://github.com/$user/convertino" | Out-Null } catch { $exists = $false }
if (-not $exists) {
  Write-Host ""
  Write-Host "Opening GitHub to create the repository. Keep the name 'convertino', choose Public,"
  Write-Host "leave README / .gitignore / licence unticked, and click 'Create repository'."
  Start-Process "https://github.com/new?name=convertino&visibility=public&description=Convert%20any%20file%20from%20a%20radial%20wheel%20in%20Explorer%20or%20Finder"
  Read-Host "Press Enter here once it's created"
}

Say "Pushing to $url (a browser window may ask you to sign in to GitHub the first time)..."
git push -u origin main 2>&1 | ForEach-Object { Say "$_" }
if ($LASTEXITCODE -ne 0) { Say "PUSH FAILED (exit $LASTEXITCODE)"; exit $LASTEXITCODE }
Say "PUSHED: https://github.com/$user/convertino"
Say "Builds: https://github.com/$user/convertino/actions"
