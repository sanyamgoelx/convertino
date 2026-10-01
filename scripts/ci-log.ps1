# Saves the logs of the latest GitHub build's failed jobs to ci.log (for Claude to read).
$ErrorActionPreference = "Continue"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
Set-Location (Split-Path $PSScriptRoot -Parent)
$gh = "$env:ProgramFiles\GitHub CLI\gh.exe"
$repo = "sanyamgoelx/convertino"
$id = & $gh run list --repo $repo --limit 1 --json databaseId --jq ".[0].databaseId"
"run $id" | Set-Content -Path ci.log -Encoding utf8
$data = (& $gh api "repos/$repo/actions/runs/$id/jobs") -join "`n" | ConvertFrom-Json
foreach ($j in $data.jobs) {
  if ($j.conclusion -ne "failure") { continue }
  "===== JOB $($j.name) ($($j.id))" | Add-Content -Path ci.log -Encoding utf8
  & $gh api "repos/$repo/actions/jobs/$($j.id)/logs" --allow-escape-sequences 2>&1 | ForEach-Object { "$_" } | Add-Content -Path ci.log -Encoding utf8
}
"done" | Add-Content -Path ci.log -Encoding utf8
