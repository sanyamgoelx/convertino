# Convertino - one-time Windows setup.
# Installs everything needed to build and run Convertino, then starts it.
# Safe to run again: anything already installed is skipped.
#
# Installs (all free, from official sources via winget):
#   Git, Node.js LTS, Visual Studio 2022 C++ Build Tools, Rust (MSVC), WebView2 runtime

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$log = Join-Path $root 'logs\setup.log'
Start-Transcript -Path $log -Append | Out-Null

function Step($msg) { Write-Host ''; Write-Host "==> $msg" -ForegroundColor Cyan }
function Ok($msg) { Write-Host "    $msg" -ForegroundColor Green }
function Have($cmd) { [bool](Get-Command $cmd -ErrorAction SilentlyContinue) }

function Refresh-Path {
    $env:Path = [Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' +
                [Environment]::GetEnvironmentVariable('Path', 'User')
    $cargoBin = Join-Path $env:USERPROFILE '.cargo\bin'
    if ((Test-Path $cargoBin) -and ($env:Path -notlike "*$cargoBin*")) { $env:Path += ";$cargoBin" }
}

function Run-Native($exe, [string[]]$argList) {
    # Native tools write progress to stderr; don't let PowerShell treat that as a failure.
    $prev = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        & $exe @argList 2>&1 | ForEach-Object { Write-Host "    $_" }
        return $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $prev
    }
}

function Winget-Install($id, [string[]]$extra) {
    Write-Host "    Installing $id (this can take a while)..."
    $wargs = @('install', '--id', $id, '-e', '--source', 'winget',
               '--accept-package-agreements', '--accept-source-agreements',
               '--disable-interactivity') + $extra
    $code = Run-Native 'winget' $wargs
    # -1978335189 = no newer version, -1978335135 = already installed
    if ($code -ne 0 -and $code -ne -1978335189 -and $code -ne -1978335135) {
        throw "winget couldn't install $id (exit code $code)"
    }
    Refresh-Path
}

$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
function Has-CppTools {
    if (-not (Test-Path $vswhere)) { return $false }
    $p = & $vswhere -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    return [bool]$p
}

try {
    Write-Host 'Convertino setup' -ForegroundColor White
    Write-Host "Log: $log"

    Step 'Checking winget'
    if (-not (Have 'winget')) {
        throw "winget is missing. Install 'App Installer' from the Microsoft Store, then run this again."
    }
    Ok 'winget found'

    Step 'Git'
    if (Have 'git') { Ok 'already installed' } else { Winget-Install 'Git.Git' @('--silent'); Ok 'installed' }

    Step 'Node.js LTS'
    if (Have 'node') { Ok ("already installed: " + (& node --version)) } else { Winget-Install 'OpenJS.NodeJS.LTS' @('--silent'); Ok 'installed' }

    Step 'Visual Studio C++ Build Tools (large download, 10-20 minutes)'
    if (Has-CppTools) {
        Ok 'already installed'
    } else {
        Write-Host '    Windows will ask for permission. Click Yes.' -ForegroundColor Yellow
        Winget-Install 'Microsoft.VisualStudio.2022.BuildTools' @('--override',
            '--quiet --wait --norestart --nocache --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended')
        if (-not (Has-CppTools)) {
            # Build Tools were already there without the C++ workload: add it.
            $bt = & $vswhere -products Microsoft.VisualStudio.Product.BuildTools -property installationPath | Select-Object -First 1
            $installer = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\setup.exe'
            if ($bt -and (Test-Path $installer)) {
                Write-Host '    Adding the C++ workload. Click Yes if Windows asks.' -ForegroundColor Yellow
                Start-Process -FilePath $installer -Verb RunAs -Wait -ArgumentList @(
                    'modify', '--installPath', "`"$bt`"", '--add', 'Microsoft.VisualStudio.Workload.VCTools',
                    '--includeRecommended', '--quiet', '--norestart')
            }
        }
        if (-not (Has-CppTools)) { throw 'The C++ Build Tools did not install. Was the permission prompt declined?' }
        Ok 'installed'
    }

    Step 'Rust'
    Refresh-Path
    if (-not (Have 'rustup')) { Winget-Install 'Rustlang.Rustup' @('--silent') }
    if (-not (Have 'rustup')) { throw 'rustup did not install' }
    $null = Run-Native 'rustup' @('toolchain', 'install', 'stable-msvc', '--profile', 'minimal')
    $null = Run-Native 'rustup' @('default', 'stable-msvc')
    Refresh-Path
    Ok ("ready: " + (& rustc --version))

    Step 'WebView2 runtime'
    $wv = 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}'
    if (Test-Path $wv) { Ok 'already installed' } else { Winget-Install 'Microsoft.EdgeWebView2Runtime' @('--silent'); Ok 'installed' }

    Step 'Project files'
    $wfDir = Join-Path $root '.github\workflows'
    $src = Join-Path $root 'ci\build.yml'
    if ((Test-Path $src) -and -not (Test-Path (Join-Path $wfDir 'build.yml'))) {
        New-Item -ItemType Directory -Force -Path $wfDir | Out-Null
        Move-Item $src (Join-Path $wfDir 'build.yml')
        Ok 'moved the build workflow to .github\workflows'
    } else { Ok 'build workflow already in place' }

    Step 'Converter tools (FFmpeg, ImageMagick)'
    & (Join-Path $PSScriptRoot 'fetch-tools.ps1')
    if ($LASTEXITCODE -ne 0) { throw 'Downloading the converter tools failed (see logs\tools.log)' }
    Start-Transcript -Path $log -Append | Out-Null

    Step 'JavaScript packages'
    Push-Location $root
    $code = Run-Native 'npm' @('install', '--no-audit', '--no-fund')
    if ($code -ne 0) { throw "npm install failed (exit code $code)" }
    Ok 'installed'

    Write-Host ''
    Write-Host 'SETUP COMPLETE' -ForegroundColor Green
    Write-Host ''
    Step 'Starting Convertino (the first build takes 5-10 minutes)'
    Write-Host '    When the window opens: select files in File Explorer and press Ctrl+Alt+C.'
    Write-Host '    Keep this window open while Convertino runs. Press Ctrl+C here to stop it.'
    Stop-Transcript | Out-Null
    & (Join-Path $PSScriptRoot 'run-dev.ps1')
    Pop-Location
}
catch {
    Write-Host ''
    Write-Host "SETUP FAILED: $($_.Exception.Message)" -ForegroundColor Red
    Write-Host "Details are in $log - Claude can read it from there."
    try { Stop-Transcript | Out-Null } catch {}
    Read-Host 'Press Enter to close'
    exit 1
}
