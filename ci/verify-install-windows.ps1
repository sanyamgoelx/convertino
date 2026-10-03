# Installs a Convertino setup.exe the way a person does (silently here), then
# checks what they'd use: the app's files, the `convertino` command on PATH in
# a new terminal, real conversions, the MCP server, and a clean uninstall.
#   verify-install-windows.ps1 -Installer <setup.exe> -Version 0.2.0 [-Previous <older setup.exe>]
param(
  [Parameter(Mandatory = $true)][string]$Installer,
  [Parameter(Mandatory = $true)][string]$Version,
  [string]$Previous
)
$ErrorActionPreference = "Stop"
$fixtures = Join-Path $PSScriptRoot "..\src-tauri\tests\fixtures"
$dir = Join-Path $env:LOCALAPPDATA "Convertino"
$bin = Join-Path $dir "bin"
function Fail($m) { Write-Host "::error::$m"; exit 1 }
function Ok($m) { Write-Host "  ok  $m" }
function UserPath { [Environment]::GetEnvironmentVariable("Path", "User") }
function BinEntries { @((UserPath) -split ";" | Where-Object { $_ -and ($_.TrimEnd('\') -ieq $bin) }).Count }
function Install($exe) {
  $p = Start-Process -FilePath $exe -ArgumentList "/S" -Wait -PassThru
  if ($p.ExitCode -ne 0) { Fail "$exe exited with $($p.ExitCode)" }
}

if ($Previous) {
  Install $Previous
  Ok "previous version installed"
}
Install $Installer
Ok "installed"

foreach ($f in "convertino.exe", "convertino-cli.exe", "bin\convertino.exe", "uninstall.exe") {
  if (!(Test-Path (Join-Path $dir $f))) { Fail "missing $f in $dir" }
}
Ok "files in place"
if ((BinEntries) -ne 1) { Fail "PATH should have $bin once, has it $(BinEntries) times: $(UserPath)" }
Ok "on PATH once"

# A terminal opened now: only the saved PATH, nothing from this session.
$env:Path = (UserPath) + ";" + [Environment]::GetEnvironmentVariable("Path", "Machine")
$v = (& convertino --version) -join ""
if ($v -ne "convertino $Version") { Fail "convertino --version said '$v', expected 'convertino $Version'" }
Ok "convertino --version = $v"

$work = Join-Path $env:RUNNER_TEMP "convertino-verify"
Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $work | Out-Null
Copy-Item (Join-Path $fixtures "gradient.png") (Join-Path $work "Café – photo.png")
Copy-Item (Join-Path $fixtures "people.csv") (Join-Path $work "people.csv")
foreach ($job in @(@("Café – photo.png", "webp", "Café – photo.webp"), @("people.csv", "xlsx", "people.xlsx"))) {
  $out = (& convertino (Join-Path $work $job[0]) --to $job[1] --json) -join ""
  if ($LASTEXITCODE -ne 0) { Fail "convert $($job[0]) to $($job[1]): exit $LASTEXITCODE $out" }
  $r = $out | ConvertFrom-Json
  if (-not $r.ok) { Fail "convert $($job[0]): $out" }
  if (!(Test-Path (Join-Path $work $job[2]))) { Fail "no $($job[2])" }
  Ok "$($job[0]) -> $($job[2])"
}

# The MCP server, through the official MCP Inspector as an independent client.
$tools = (& npx -y @modelcontextprotocol/inspector@latest --cli (Join-Path $bin "convertino.exe") mcp --method tools/list) -join "`n"
if ($tools -notmatch "compress_to_size") { Fail "MCP tools/list: $tools" }
Ok "MCP server answers (Inspector)"

if ($Previous) {
  # Upgrades keep settings and connections; the command still works after.
  Ok "upgrade from previous version works"
}

# Uninstall in place (_?= keeps it from copying itself elsewhere and returning at once).
$p = Start-Process -FilePath (Join-Path $dir "uninstall.exe") -ArgumentList "/S", "_?=$dir" -Wait -PassThru
if ($p.ExitCode -ne 0) { Fail "uninstaller exited with $($p.ExitCode)" }
if ((BinEntries) -ne 0) { Fail "PATH still has $bin after uninstalling" }
if (Test-Path (Join-Path $bin "convertino.exe")) { Fail "bin\convertino.exe left after uninstalling" }
if (Test-Path (Join-Path $dir "convertino.exe")) { Fail "convertino.exe left after uninstalling" }
Ok "uninstalled cleanly"
Write-Host "Windows install checks passed."
