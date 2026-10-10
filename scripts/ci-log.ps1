# Saves the logs of the latest failed (or cancelled) GitHub run's failed jobs to logs\ci.log (for Claude to read).
$ErrorActionPreference = "Continue"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
Set-Location (Split-Path $PSScriptRoot -Parent)
$gh = "$env:ProgramFiles\GitHub CLI\gh.exe"
$repo = "sanyamgoelx/convertino"
# The newest run with a failed or cancelled job, even while its other jobs still run.
$runs = (& $gh run list --repo $repo --limit 10 --json databaseId) -join "`n" | ConvertFrom-Json
$id = $null
foreach ($r in $runs) {
  $jobs = (& $gh api "repos/$repo/actions/runs/$($r.databaseId)/jobs") -join "`n" | ConvertFrom-Json
  if ($jobs.jobs | Where-Object { $_.conclusion -in @("failure", "cancelled") }) { $id = $r.databaseId; break }
}
"run $id" | Set-Content -Path logs\ci.log -Encoding utf8
$data = (& $gh api "repos/$repo/actions/runs/$id/jobs") -join "`n" | ConvertFrom-Json
foreach ($j in $data.jobs) {
  if ($j.conclusion -ne "failure" -and $j.conclusion -ne "cancelled") { continue }
  "===== JOB $($j.name) ($($j.id))" | Add-Content -Path logs\ci.log -Encoding utf8
  & $gh api "repos/$repo/actions/jobs/$($j.id)/logs" --allow-escape-sequences 2>&1 | ForEach-Object { "$_" } | Add-Content -Path logs\ci.log -Encoding utf8
}
"done" | Add-Content -Path logs\ci.log -Encoding utf8
