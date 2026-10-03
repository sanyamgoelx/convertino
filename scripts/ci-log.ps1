# Saves the logs of the latest failed (or cancelled) GitHub run's failed jobs to ci.log (for Claude to read).
$ErrorActionPreference = "Continue"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
Set-Location (Split-Path $PSScriptRoot -Parent)
$gh = "$env:ProgramFiles\GitHub CLI\gh.exe"
$repo = "sanyamgoelx/convertino"
$runs = (& $gh run list --repo $repo --status completed --limit 10 --json databaseId,conclusion) -join "`n" | ConvertFrom-Json
$id = ($runs | Where-Object { $_.conclusion -in @("failure", "cancelled") } | Select-Object -First 1).databaseId
"run $id" | Set-Content -Path ci.log -Encoding utf8
$data = (& $gh api "repos/$repo/actions/runs/$id/jobs") -join "`n" | ConvertFrom-Json
foreach ($j in $data.jobs) {
  if ($j.conclusion -ne "failure" -and $j.conclusion -ne "cancelled") { continue }
  "===== JOB $($j.name) ($($j.id))" | Add-Content -Path ci.log -Encoding utf8
  & $gh api "repos/$repo/actions/jobs/$($j.id)/logs" --allow-escape-sequences 2>&1 | ForEach-Object { "$_" } | Add-Content -Path ci.log -Encoding utf8
}
"done" | Add-Content -Path ci.log -Encoding utf8
