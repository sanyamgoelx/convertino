# Cancels runs still in progress, then saves the cancelled jobs' logs so far to ci.log.
Set-Location (Split-Path $PSScriptRoot -Parent)
$gh = "$env:ProgramFiles\GitHub CLI\gh.exe"
$repo = "sanyamgoelx/convertino"
$ids = & $gh run list --repo $repo --status in_progress --json databaseId --jq ".[].databaseId"
"cancelling: $ids" | Set-Content ci-cancel.log -Encoding utf8
foreach ($id in $ids) { & $gh run cancel $id --repo $repo 2>&1 | Add-Content ci-cancel.log -Encoding utf8 }
"done" | Add-Content ci-cancel.log -Encoding utf8
