# Lists the GitHub downloads made by this PC's own Convertino (update checks
# and updates) in .github/stats-exclude.json, so the download counts leave
# them out. Runs before every push (publish-github.ps1). Reads the installed
# app's update-history.log, and for older events Convertino.log.
$ErrorActionPreference = "Continue"
Set-Location (Split-Path $PSScriptRoot -Parent)
$file = ".github\stats-exclude.json"
$logs = Join-Path $env:LOCALAPPDATA "com.crofty.convertino\logs"
$why = "developer's own copy of Convertino"
$epoch = [DateTime]::new(1970, 1, 1, 0, 0, 0, [DateTimeKind]::Utc)

$data = if (Test-Path $file) { Get-Content $file -Raw | ConvertFrom-Json } else { $null }
$entries = New-Object System.Collections.ArrayList
$ids = @{}
if ($data -and $data.entries) { foreach ($e in $data.entries) { [void]$entries.Add($e); if ($e.id) { $ids[$e.id] = $true } } }

# Events: what (check/download), version, time (UTC).
$events = New-Object System.Collections.ArrayList
$hist = Join-Path $logs "update-history.log"
$first = [DateTime]::MaxValue
if (Test-Path $hist) {
  foreach ($line in Get-Content $hist) {
    $p = $line.Trim() -split " "
    if ($p.Count -ne 3) { continue }
    $t = $epoch.AddSeconds([double]$p[0])
    if ($t -lt $first) { $first = $t }
    [void]$events.Add([pscustomobject]@{ what = $p[1]; version = $p[2]; at = $t })
  }
}
foreach ($log in Get-ChildItem $logs -Filter "Convertino*.log" -ErrorAction SilentlyContinue) {
  foreach ($line in Get-Content $log.FullName) {
    if ($line -notmatch '^\[(\d{4}-\d\d-\d\d)\]\[(\d\d:\d\d:\d\d)\]\[convertino_lib::update\]\[INFO\] (updating to|update available:) (\S+)') { continue }
    $t = [DateTime]::SpecifyKind([DateTime]::ParseExact("$($matches[1]) $($matches[2])", "yyyy-MM-dd HH:mm:ss", $null), [DateTimeKind]::Utc)
    if ($t -ge $first) { continue }   # update-history.log has these
    [void]$events.Add([pscustomobject]@{ what = "check"; version = $matches[4]; at = $t })
    if ($matches[3] -eq "updating to") { [void]$events.Add([pscustomobject]@{ what = "download"; version = $matches[4]; at = $t }) }
  }
}

# GitHub counts the same file fetched again within a minute once.
$lastSeen = @{}
$added = 0
foreach ($e in ($events | Sort-Object at)) {
  $bucket = if ($e.what -eq "download") { "windows" } else { "updateChecks" }
  $k = "$bucket $($e.version)"
  if ($lastSeen.ContainsKey($k) -and ($e.at - $lastSeen[$k]).TotalSeconds -lt 60) { continue }
  $lastSeen[$k] = $e.at
  $at = $e.at.ToString("yyyy-MM-ddTHH:mm:ssZ")
  $id = "own:$bucket`:$($e.version):$at"
  if ($ids.ContainsKey($id)) { continue }
  $ids[$id] = $true
  [void]$entries.Add([pscustomobject]@{ id = $id; tag = "v$($e.version)"; bucket = $bucket; count = 1; at = $at; why = $why })
  $added++
}

$note = "Test downloads left out of the download counts (ci/download-stats.py). CI downloads are worked out automatically; this lists the rest. own:* entries come from scripts/stats-own.ps1."
$sorted = @($entries | Sort-Object at)
$out = [pscustomobject]@{ note = $note; entries = $sorted }
$json = $out | ConvertTo-Json -Depth 5
[IO.File]::WriteAllText((Join-Path (Get-Location) $file), $json + "`n", [Text.UTF8Encoding]::new($false))
Write-Host "Own downloads listed: $added new, $($sorted.Count) in total."
