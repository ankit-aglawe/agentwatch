# agentwatch installer for Windows (PowerShell 5+ / 7+)
#
# Usage:
#   irm https://agentwatch.sh/install.ps1 | iex
#
# Environment overrides:
#   $env:AGENTWATCH_VERSION       — git tag or "latest" (default: latest)
#   $env:AGENTWATCH_INSTALL_DIR   — destination dir (default: %LOCALAPPDATA%\agentwatch\bin)
#   $env:AGENTWATCH_NO_PROMPT     — set to 1 to skip the confirmation prompt

$ErrorActionPreference = "Stop"

$Repo       = "ankit-aglawe/agentwatch"
$Version    = if ($env:AGENTWATCH_VERSION)     { $env:AGENTWATCH_VERSION }     else { "latest" }
$InstallDir = if ($env:AGENTWATCH_INSTALL_DIR) { $env:AGENTWATCH_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA "agentwatch\bin" }
$NoPrompt   = $env:AGENTWATCH_NO_PROMPT

function Say($msg)  { Write-Host $msg -ForegroundColor Cyan }
function Dim($msg)  { Write-Host $msg -ForegroundColor DarkGray }
function Die($msg)  { Write-Error $msg; exit 1 }

$archEnum = [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture
$Arch = switch ($archEnum) {
    "X64"   { "x86_64" }
    "Arm64" { "aarch64" }
    default { Die "unsupported architecture: $archEnum" }
}
$Target = "$Arch-pc-windows-msvc"

Say "agentwatch installer"
Dim "  version : $Version"
Dim "  target  : $Target"
Dim "  install : $InstallDir"

if (-not $NoPrompt -and [Environment]::UserInteractive) {
    Read-Host "Press Enter to continue, Ctrl+C to abort" | Out-Null
}

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null

# TODO: enable once v0.1 is tagged in GitHub Releases.
# $tarball  = "agentwatch-$Version-$Target.zip"
# $base     = "https://github.com/$Repo/releases/download/$Version"
# Invoke-WebRequest "$base/$tarball"        -OutFile "$env:TEMP\$tarball"
# Invoke-WebRequest "$base/$tarball.sha256" -OutFile "$env:TEMP\$tarball.sha256"
# # verify checksum, expand archive, move binary to $InstallDir, cleanup …

Say "(v0.1 not yet released — track https://github.com/$Repo/releases)"

if (-not (($env:Path -split ';') -contains $InstallDir)) {
    Dim "note: add $InstallDir to your PATH"
}
