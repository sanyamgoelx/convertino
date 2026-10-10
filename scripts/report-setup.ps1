# One time: gives GitHub the Discord webhook that error reports are posted to.
# Builds made after this have a "Send report" button on failed jobs.
# The webhook stays out of the source code (it's an Actions secret).
$ErrorActionPreference = "Continue"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
Set-Location (Split-Path $PSScriptRoot -Parent)
$log = Join-Path (Get-Location) "logs\report-setup.log"
function Say($m) { Write-Host $m; Add-Content -Path $log -Value $m -Encoding utf8 }
"Convertino report setup $(Get-Date -Format s)" | Set-Content -Path $log -Encoding utf8
$gh = "$env:ProgramFiles\GitHub CLI\gh.exe"
$repo = "sanyamgoelx/convertino"

Write-Host ""
Write-Host "Paste the Discord webhook URL (right-click to paste), then press Enter:"
$url = (Read-Host).Trim()
if ($url -notmatch '^https://(discord\.com|discordapp\.com|ptb\.discord\.com|canary\.discord\.com)/api/webhooks/\d+/[\w-]+$') {
  Say "That doesn't look like a Discord webhook URL (https://discord.com/api/webhooks/...). Nothing was changed."
  Read-Host "Press Enter to close"
  exit 1
}
# A test message, so you can see it arrive in the channel.
$body = @{ username = "Convertino reports"; content = "Convertino error reports are connected to this channel." } | ConvertTo-Json
try {
  Invoke-RestMethod -Uri $url -Method Post -ContentType "application/json" -Body $body | Out-Null
  Say "Test message sent to the channel."
} catch {
  Say "The test message couldn't be sent: $($_.Exception.Message)"
  Read-Host "Press Enter to close"
  exit 1
}
$url | & $gh secret set CONVERTINO_REPORT_WEBHOOK --repo $repo 2>&1 | ForEach-Object { Say "  $_" }
$names = & $gh secret list --repo $repo 2>&1
if (($names -join " ") -match "CONVERTINO_REPORT_WEBHOOK") { Say "REPORT SETUP DONE" } else { Say "SECRET NOT SET" }
Start-Sleep -Seconds 3
