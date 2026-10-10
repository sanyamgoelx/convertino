# Gets every converter Convertino uses into src-tauri\tools, using the app's
# own downloader (src-tauri\src\install.rs): the same downloads, trimming and
# checks a friend's PC goes through the first time it needs each converter.
# Converters already there are trimmed instead of downloaded again.
# Output is mirrored to logs\tools.log.
$root = Split-Path -Parent $PSScriptRoot
$log = Join-Path $root 'logs\tools.log'
Set-Location (Join-Path $root 'src-tauri')
Set-Content -Path $log -Value "Convertino tools $(Get-Date -Format s)" -Encoding UTF8
$ErrorActionPreference = 'Continue'
# cargo prints UTF-8 (file names like "Café – मेनू.pdf").
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
Write-Host 'Building the downloader (the first time takes a few minutes)...' -ForegroundColor Cyan
& cargo test --lib --color never install::tests::fetch_tools -- --ignored --nocapture 2>&1 | ForEach-Object {
    $line = if ($_ -is [System.Management.Automation.ErrorRecord]) { [string]$_.TargetObject } else { "$_" }
    if ($line -match '^\s+(Compiling|Running|Finished)') { return }
    Write-Host $line
    Add-Content -Path $log -Value $line -Encoding UTF8
}
if ($LASTEXITCODE -eq 0) { Write-Host 'TOOLS READY' -ForegroundColor Green } else { Write-Host 'Some tools are missing; see logs\tools.log.' -ForegroundColor Red }
Add-Content -Path $log -Value "exit $LASTEXITCODE" -Encoding UTF8
