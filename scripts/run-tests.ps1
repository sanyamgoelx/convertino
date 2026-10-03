# Runs the Rust tests (wheel layout, real conversions, the convertino command and its MCP server) and mirrors the output to test.log.
$root = Split-Path -Parent $PSScriptRoot
$log = Join-Path $root 'test.log'
Set-Location (Join-Path $root 'src-tauri')
Set-Content -Path $log -Value "Convertino tests $(Get-Date -Format s)" -Encoding UTF8
$ErrorActionPreference = 'Continue'
# cargo prints UTF-8 (file names like "Café – मेनू.pdf").
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
& cargo test --color never 2>&1 | ForEach-Object {
    $line = if ($_ -is [System.Management.Automation.ErrorRecord]) { [string]$_.TargetObject } else { "$_" }
    Write-Host $line
    Add-Content -Path $log -Value $line -Encoding UTF8
}
if ($LASTEXITCODE -eq 0) { Write-Host 'All tests passed.' -ForegroundColor Green } else { Write-Host 'Some tests failed; see test.log.' -ForegroundColor Red }
Add-Content -Path $log -Value "exit $LASTEXITCODE" -Encoding UTF8
