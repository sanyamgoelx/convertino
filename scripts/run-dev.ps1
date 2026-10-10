# Starts Convertino in development mode and mirrors the output to logs\dev.log.
$root = Split-Path -Parent $PSScriptRoot
$log = Join-Path $root 'logs\dev.log'
Set-Location $root
Set-Content -Path $log -Value "Convertino dev run $(Get-Date -Format s)" -Encoding UTF8
Write-Host 'Starting Convertino. Keep this window open; press Ctrl+C to stop.' -ForegroundColor Cyan
$ErrorActionPreference = 'Continue'
& npm run dev 2>&1 | ForEach-Object {
    $line = "$_"
    Write-Host $line
    Add-Content -Path $log -Value $line -Encoding UTF8
}
