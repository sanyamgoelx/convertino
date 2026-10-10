# Downloads the Windows installer from the latest GitHub build of
# main and starts it (to try the installer before a release).
$ErrorActionPreference = "Continue"
Set-Location (Split-Path $PSScriptRoot -Parent)
$gh = "$env:ProgramFiles\GitHub CLI\gh.exe"
$repo = "sanyamgoelx/convertino"
$log = "logs\try-installer.log"
$id = & $gh run list --repo $repo --workflow Build --branch main --limit 1 --json databaseId --jq ".[0].databaseId"
"run $id" | Set-Content -Path $log -Encoding utf8
$dir = ".sync\ci-installer-$id"
if (-not (Test-Path $dir)) {
  & $gh run download $id --repo $repo --name convertino-windows --dir $dir 2>&1 | Add-Content -Path $log -Encoding utf8
}
$exe = Get-ChildItem $dir -Recurse -Filter *-setup.exe | Select-Object -First 1
if ($exe) {
  "starting $($exe.FullName)" | Add-Content -Path $log -Encoding utf8
  Start-Process $exe.FullName
} else {
  "no setup.exe found" | Add-Content -Path $log -Encoding utf8
}
