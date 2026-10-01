# One time: credits every commit and release tag to the signed-in GitHub
# account (they were made with another account's name and email), then
# replaces the history on GitHub. Releases and their downloads stay as they are.
$ErrorActionPreference = "Continue"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
Set-Location (Split-Path $PSScriptRoot -Parent)
$log = Join-Path (Get-Location) "fix-author.log"
function Say($m) { Write-Host $m; Add-Content -Path $log -Value $m -Encoding utf8 }
"Convertino fix-author $(Get-Date -Format s)" | Set-Content -Path $log -Encoding utf8
$gh = "$env:ProgramFiles\GitHub CLI\gh.exe"
$repo = "sanyamgoelx/convertino"

$user = (& $gh api user --jq .login).Trim()
$uid = (& $gh api user --jq .id).Trim()
if (-not $user -or -not $uid) { Say "Not signed in to GitHub (run publish-github.cmd first)."; exit 1 }
$email = "$uid+$user@users.noreply.github.com"
Say "Crediting everything to $user <$email>"
git config user.name $user
git config user.email $email
git config core.autocrlf true

if (Test-Path ".git\index.lock") { Remove-Item ".git\index.lock" -Force -ErrorAction SilentlyContinue }
git fetch origin --tags 2>&1 | ForEach-Object { Say "  $_" }
git add -A
git diff --cached --quiet
if ($LASTEXITCODE -ne 0) { git commit -q -m "Convertino: latest changes"; Say "Committed the latest changes." }

# Moving the tags would otherwise rebuild both releases.
& $gh workflow disable release.yml --repo $repo 2>&1 | ForEach-Object { Say "  $_" }

$env:FILTER_BRANCH_SQUELCH_WARNING = "1"
$filter = "export GIT_AUTHOR_NAME='$user' GIT_AUTHOR_EMAIL='$email' GIT_COMMITTER_NAME='$user' GIT_COMMITTER_EMAIL='$email'"
git filter-branch -f --env-filter $filter --tag-name-filter cat -- --all 2>&1 | ForEach-Object { Say "  $_" }

# Release tags: made again so the tag itself names the new account too.
foreach ($t in (git tag -l)) {
  $c = (git rev-list -n 1 $t).Trim()
  $msg = (git tag -l --format='%(contents)' $t) -join "`n"
  if (-not $msg.Trim()) { $msg = "Convertino $t" }
  git tag -f -a $t -m $msg $c 2>&1 | Out-Null
  Say "Tag $t -> $($c.Substring(0,7))"
}

# Drop the backup refs filter-branch leaves behind.
git for-each-ref --format="%(refname)" refs/original/ | ForEach-Object { git update-ref -d $_ }

Say "Authors now:"
git log --all --format="%an <%ae> | %cn <%ce>" | Sort-Object -Unique | ForEach-Object { Say "  $_" }
git for-each-ref refs/tags --format="%(refname:short) tagger: %(taggername) <%(taggeremail)>" | ForEach-Object { Say "  $_" }

git push --force origin main 2>&1 | ForEach-Object { Say "  $_" }
$okMain = $LASTEXITCODE -eq 0
git push --force origin --tags 2>&1 | ForEach-Object { Say "  $_" }
$okTags = $LASTEXITCODE -eq 0
& $gh workflow enable release.yml --repo $repo 2>&1 | ForEach-Object { Say "  $_" }
if ($okMain -and $okTags) { Say "FIX-AUTHOR DONE" } else { Say "PUSH FAILED" }
Start-Sleep -Seconds 3
