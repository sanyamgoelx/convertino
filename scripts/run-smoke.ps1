# Converts every sample in test-files the way the wheel would, into test-files\results.
# The summary is test-files\results\smoke-report.txt (also mirrored to smoke.log).
$root = Split-Path -Parent $PSScriptRoot
$log = Join-Path $root 'smoke.log'
$env:CONVERTINO_SMOKE_DIR = Join-Path $root 'test-files'
Set-Location (Join-Path $root 'src-tauri')
Set-Content -Path $log -Value "Convertino smoke run $(Get-Date -Format s)" -Encoding UTF8
$ErrorActionPreference = 'Continue'
# cargo prints UTF-8 (file names like "Café – मेनू.pdf").
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
& cargo test --lib --color never smoke -- --ignored --nocapture 2>&1 | ForEach-Object {
    $line = if ($_ -is [System.Management.Automation.ErrorRecord]) { [string]$_.TargetObject } else { "$_" }
    Write-Host $line
    Add-Content -Path $log -Value $line -Encoding UTF8
}
Add-Content -Path $log -Value "exit $LASTEXITCODE" -Encoding UTF8
