# Re-runs the failed jobs of the latest GitHub build (e.g. after a network hiccup).
$ErrorActionPreference = "Continue"
Set-Location (Split-Path $PSScriptRoot -Parent)
$gh = "$env:ProgramFiles\GitHub CLI\gh.exe"
$id = & $gh run list --repo sanyamgoelx/convertino --limit 1 --json databaseId --jq ".[0].databaseId"
& $gh run rerun $id --repo sanyamgoelx/convertino --failed 2>&1 | ForEach-Object { "$_" } | Set-Content -Path ci-rerun.log -Encoding utf8
