<#
.SYNOPSIS
    Uninstalls android-cli from Windows.

.DESCRIPTION
    Removes the installed binaries and the PATH entry created by install.ps1.
    The state directory (~/.android-cli-agent) is kept unless -PurgeState is
    passed; purging asks for confirmation because it deletes local data.

.PARAMETER InstallDir
    Target directory. Default: %LOCALAPPDATA%\Programs\android-cli

.PARAMETER PurgeState
    Also delete the state directory (Wi-Fi endpoints). Requires confirmation
    unless -Yes is given.

.PARAMETER Yes
    Skip the confirmation prompts (for scripted use).

.EXAMPLE
    ./scripts/uninstall.ps1
    ./scripts/uninstall.ps1 -PurgeState
#>
[CmdletBinding()]
param(
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA "Programs\android-cli"),
    [switch]$PurgeState,
    [switch]$Yes
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$StateDir = Join-Path $HOME ".android-cli-agent"

# --- binaries -------------------------------------------------------------
if (Test-Path $InstallDir) {
    Remove-Item -Recurse -Force $InstallDir
    Write-Host "Removed $InstallDir"
}
else {
    Write-Host "Nothing installed at $InstallDir"
}

# --- PATH entry -----------------------------------------------------------
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($null -ne $userPath) {
    $entries = @($userPath -split ';' | Where-Object { $_ -ne "" -and $_ -ne $InstallDir })
    if ($entries.Count -ne (@($userPath -split ';' | Where-Object { $_ -ne "" })).Count) {
        [Environment]::SetEnvironmentVariable("Path", ($entries -join ';'), "User")
        Write-Host "Removed $InstallDir from the user PATH"
    }
}

# --- state (destructive) --------------------------------------------------
if ($PurgeState) {
    if (-not (Test-Path $StateDir)) {
        Write-Host "No state directory at $StateDir"
    }
    elseif (-not $Yes) {
        $answer = Read-Host "CONFIRM DESTRUCTIVE: delete state $StateDir (saved Wi-Fi endpoints)? [y/N]"
        if ($answer -notmatch '^(y|yes)$') {
            Write-Host "Kept $StateDir"
            $PurgeState = $false
        }
    }
    if ($PurgeState -and (Test-Path $StateDir)) {
        Remove-Item -Recurse -Force $StateDir
        Write-Host "Removed $StateDir"
    }
}
else {
    Write-Host "Kept $StateDir (use -PurgeState to delete it)"
}

Write-Host "android-cli uninstalled."
exit 0
