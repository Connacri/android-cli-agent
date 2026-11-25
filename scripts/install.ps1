<#
.SYNOPSIS
    Installs android-cli on Windows.

.DESCRIPTION
    Downloads the published android-cli.exe from the latest GitHub release,
    verifies that it runs, installs it under %LOCALAPPDATA%\Programs\android-cli
    and adds that directory to the user PATH.

    Release artifacts are produced by GitHub Actions only; this script never
    builds the project. For development builds use -FromBuild.

.PARAMETER Version
    Release version to install (without the leading 'v'). Default: latest.

.PARAMETER InstallDir
    Target directory. Default: %LOCALAPPDATA%\Programs\android-cli

.PARAMETER FromBuild
    Copy an existing locally built android-cli.exe instead of downloading
    (development mode, e.g. .\target\debug\android-cli.exe).

.EXAMPLE
    ./scripts/install.ps1
    ./scripts/install.ps1 -Version 0.1.0
    ./scripts/install.ps1 -FromBuild ./target/debug/android-cli.exe
#>
[CmdletBinding()]
param(
    [string]$Version,
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA "Programs\android-cli"),
    [string]$FromBuild
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

# Windows PowerShell 5.1 needs an explicit TLS setting for HTTPS.
try {
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
} catch { }

$AssetName = "android-cli-windows-x86_64.exe"
$ExePath = Join-Path $InstallDir "android-cli.exe"

function Add-ToUserPath([string]$Dir) {
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if ($null -eq $userPath) { $userPath = "" }
    $entries = @($userPath -split ';' | Where-Object { $_ -ne "" })
    if ($entries -contains $Dir) {
        Write-Host "PATH already contains $Dir"
        return
    }
    $newPath = ($entries + $Dir) -join ';'
    [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
    Write-Host "Added $Dir to the user PATH (open a new terminal to use it)."
}

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null

if ($FromBuild) {
    if (-not (Test-Path $FromBuild)) {
        Write-Error "Build artifact not found: $FromBuild"
        exit 1
    }
    Copy-Item -Force $FromBuild $ExePath
    Write-Host "Installed $FromBuild -> $ExePath"
}
else {
    if (-not $Version) {
        Write-Host "Querying the latest release..."
        $release = Invoke-RestMethod -Uri "https://api.github.com/repos/Connacri/android-cli-agent/releases/latest"
        $Version = $release.tag_name -replace '^v', ''
        $asset = $release.assets | Where-Object { $_.name -eq $AssetName } | Select-Object -First 1
        if (-not $asset) {
            Write-Error "Release v$Version has no asset named $AssetName."
            exit 1
        }
        $url = $asset.browser_download_url
    }
    else {
        $url = "https://github.com/Connacri/android-cli-agent/releases/download/v$Version/$AssetName"
    }

    Write-Host "Downloading $url"
    $tmp = Join-Path $env:TEMP "android-cli-install.exe"
    Invoke-WebRequest -Uri $url -OutFile $tmp -UseBasicParsing
    Move-Item -Force $tmp $ExePath
    Write-Host "Installed v$Version -> $ExePath"
}

# Smoke test: the binary must at least report its version.
$versionOutput = & $ExePath --version 2>&1
if ($LASTEXITCODE -ne 0) {
    Write-Error "Installed binary failed to run (exit $LASTEXITCODE): $versionOutput"
    exit 1
}
Write-Host "Verified: $versionOutput"

Add-ToUserPath $InstallDir
Write-Host ""
Write-Host "Next: run .\scripts\doctor.ps1  (or open a new terminal and run 'android-cli doctor')"
exit 0
