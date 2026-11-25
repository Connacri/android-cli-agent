<#
.SYNOPSIS
    Diagnoses the local android-cli setup.

.DESCRIPTION
    Checks the host prerequisites (adb, Rust toolchain, git) and, when
    android-cli is installed, runs its own `doctor` command.
    Exit code 0 = everything needed is present, 1 = something is missing.

.EXAMPLE
    ./scripts/doctor.ps1
#>
[CmdletBinding()]
param()

$ErrorActionPreference = "Continue"
Set-StrictMode -Version Latest

$failures = 0

function Check([string]$Name, [scriptblock]$Probe, [string]$Hint) {
    $result = & $Probe
    if ($result) {
        Write-Host "[OK]   $Name : $result"
    }
    else {
        Write-Host "[FAIL] $Name : $Hint"
        $script:failures++
    }
}

Check "adb" {
    $cmd = Get-Command adb -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    $sdk = Join-Path $env:LOCALAPPDATA "Android\Sdk\platform-tools\adb.exe"
    if (Test-Path $sdk) { return $sdk }
    return $null
} "install Android SDK Platform-Tools (or set ACA_ADB_PATH)"

Check "cargo (optional)" {
    $cmd = Get-Command cargo -ErrorAction SilentlyContinue
    if ($cmd) { return (cargo --version 2>$null) }
    return $null
} "needed only for building from source: https://rustup.rs"

Check "git (optional)" {
    $cmd = Get-Command git -ErrorAction SilentlyContinue
    if ($cmd) { return (git --version 2>$null) }
    return $null
} "needed only for building from source"

$cli = Get-Command android-cli -ErrorAction SilentlyContinue
if (-not $cli) {
    $fallback = Join-Path $env:LOCALAPPDATA "Programs\android-cli\android-cli.exe"
    if (Test-Path $fallback) { $cli = $fallback }
}

if ($cli) {
    Write-Host ""
    Write-Host "Running android-cli doctor..."
    & $cli doctor
    if ($LASTEXITCODE -ne 0) { $failures++ }
}
else {
    Write-Host ""
    Write-Host "[FAIL] android-cli : not installed (run .\scripts\install.ps1)"
    $failures++
}

Write-Host ""
if ($failures -eq 0) {
    Write-Host "Healthy: yes"
    exit 0
}
Write-Host "Healthy: no ($failures failing check(s))"
exit 1
