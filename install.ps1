# agentwatch installer for Windows  (PowerShell 5.1+ / 7+)
#
# Usage:
#   irm https://raw.githubusercontent.com/ankit-aglawe/agentwatch/main/install.ps1 | iex
#
# Environment overrides:
#   $env:AGENTWATCH_VERSION         git tag or "latest"   (default: latest)
#   $env:AGENTWATCH_INSTALL_DIR     destination directory (default: %LOCALAPPDATA%\agentwatch\bin)
#   $env:AGENTWATCH_NO_PROMPT       set to 1 to skip the Y/n confirmation
#   $env:AGENTWATCH_NO_MODIFY_PATH  set to 1 to skip persistent PATH update
#   $env:AGENTWATCH_FORCE           set to 1 to overwrite an existing install
#
# What this does (transparency before piping to iex):
#   1. Enforces TLS 1.2/1.3 (Windows defaults can be older).
#   2. Detects CPU architecture.
#   3. Confirms with the user (unless non-interactive).
#   4. Downloads the right asset from GitHub Releases.
#   5. Verifies the SHA-256 checksum (when published).
#   6. Atomically installs to $InstallDir.
#   7. Adds $InstallDir to your User PATH (persistent across sessions).

$ErrorActionPreference = "Stop"

# -------- TLS hardening ------------------------------------------------------

try {
    [Net.ServicePointManager]::SecurityProtocol = `
        [Net.SecurityProtocolType]::Tls12 -bor `
        ([Net.SecurityProtocolType]::Tls13 -as [Net.SecurityProtocolType])
} catch {
    # Tls13 enum may not exist on older runtimes - Tls12 alone is fine.
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
}

# -------- Constants ----------------------------------------------------------

$Repo       = "ankit-aglawe/agentwatch"
$Binary     = "agentwatch"

$Version       = if ($env:AGENTWATCH_VERSION)        { $env:AGENTWATCH_VERSION }        else { "latest" }
$InstallDir    = if ($env:AGENTWATCH_INSTALL_DIR)    { $env:AGENTWATCH_INSTALL_DIR }    else { Join-Path $env:LOCALAPPDATA "agentwatch\bin" }
$NoPrompt      = $env:AGENTWATCH_NO_PROMPT
$NoModifyPath  = $env:AGENTWATCH_NO_MODIFY_PATH
$Force         = $env:AGENTWATCH_FORCE

# -------- Output helpers -----------------------------------------------------

function Step($msg) { Write-Host $msg -ForegroundColor Cyan }
function Sub($msg)  { Write-Host "  $msg" -ForegroundColor DarkGray }
function Warn($msg) { Write-Host "warn: $msg" -ForegroundColor Yellow }
function Die($msg)  { Write-Host "error: $msg" -ForegroundColor Red; exit 1 }

# -------- Prereq + target detection ------------------------------------------

if ($PSVersionTable.PSVersion.Major -lt 5) {
    Die "PowerShell 5.1 or later required. You are running $($PSVersionTable.PSVersion)."
}

$ArchEnum = [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture
$Arch = switch ("$ArchEnum") {
    "X64"   { "x86_64" }
    "Arm64" { "aarch64" }
    default { Die "unsupported architecture: $ArchEnum" }
}
$Target = "$Arch-pc-windows-msvc"
$Asset  = "$Binary-$Target.zip"

$UrlBase = if ($Version -eq "latest") {
    "https://github.com/$Repo/releases/latest/download"
} else {
    "https://github.com/$Repo/releases/download/$Version"
}

Step "agentwatch installer"
Sub  "version : $Version"
Sub  "target  : $Target"
Sub  "install : $InstallDir\$Binary.exe"

# Detect existing install (informational).
$ExistingPath = Join-Path $InstallDir "$Binary.exe"
if (Test-Path $ExistingPath) {
    try {
        $ExistingVer = & $ExistingPath --version 2>$null | Select-Object -First 1
        Sub "existing: $ExistingVer (will be overwritten)"
    } catch {
        Sub "existing: <unknown> (will be overwritten)"
    }
}

# -------- Confirm ------------------------------------------------------------

if (-not $NoPrompt -and [Environment]::UserInteractive) {
    $answer = Read-Host "Proceed? [Y/n]"
    if ($answer -and ($answer.ToLower() -notin @('y','yes'))) {
        Die "aborted by user"
    }
}

# -------- Download + install -------------------------------------------------

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
$Tmp = Join-Path $env:TEMP ("agentwatch-install-" + [System.Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $Tmp | Out-Null

try {
    Step "downloading $Asset"
    try {
        Invoke-WebRequest -UseBasicParsing -Uri "$UrlBase/$Asset" `
            -OutFile (Join-Path $Tmp $Asset) -ErrorAction Stop
    } catch {
        Write-Host ""
        Write-Host "error: could not download $UrlBase/$Asset" -ForegroundColor Red
        Write-Host "  $($_.Exception.Message)"
        Write-Host @"

Possible causes:
  - No release has been published yet for $Repo.
    Check: https://github.com/$Repo/releases
  - The latest release has no binary for $Target.
  - Network / proxy issue.

Until binaries are published, install from source:
  cargo install agentwatch

(Pushing code to main does NOT create a release. Cut one by tagging:
   git tag v0.1.0 ; git push origin v0.1.0
 The release workflow at .github/workflows/release.yml will build the
 binaries and attach them to the release.)
"@
        exit 1
    }

    # Checksum verification - soft-fail if the release didn't publish one.
    try {
        Invoke-WebRequest -UseBasicParsing -Uri "$UrlBase/$Asset.sha256" `
            -OutFile (Join-Path $Tmp "$Asset.sha256") -ErrorAction Stop
        $Expected = ((Get-Content (Join-Path $Tmp "$Asset.sha256")) -split '\s+')[0].ToLower()
        $Actual   = (Get-FileHash -Algorithm SHA256 (Join-Path $Tmp $Asset)).Hash.ToLower()
        if ($Expected -ne $Actual) {
            Die "checksum mismatch`n  expected: $Expected`n  actual:   $Actual"
        }
        Sub "checksum: ok"
    } catch {
        Warn "no checksum at $UrlBase/$Asset.sha256 - skipping verification"
    }

    Expand-Archive -Force -Path (Join-Path $Tmp $Asset) -DestinationPath $Tmp
    $ExePath = Join-Path $Tmp "$Binary.exe"
    if (-not (Test-Path $ExePath)) {
        Die "archive did not contain $Binary.exe"
    }

    Move-Item -Force $ExePath (Join-Path $InstallDir "$Binary.exe")

    Step "installed agentwatch -> $InstallDir\$Binary.exe"
    try { & (Join-Path $InstallDir "$Binary.exe") --version | Select-Object -First 1 } catch {}

    # -------- Persistent PATH update -----------------------------------------

    if (-not $NoModifyPath) {
        $UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
        $Already  = ($UserPath -split ';') | Where-Object { $_ -eq $InstallDir }
        if (-not $Already) {
            $NewPath = if ($UserPath) { "$InstallDir;$UserPath" } else { $InstallDir }
            [Environment]::SetEnvironmentVariable("Path", $NewPath, "User")
            Sub "added $InstallDir to user PATH"
            Sub "open a new terminal for PATH changes to take effect"
        }
    } else {
        Warn "$InstallDir is not on PATH; add it yourself or unset AGENTWATCH_NO_MODIFY_PATH"
    }

    Step "done. run ``agentwatch --help`` to get started."
} finally {
    Remove-Item -Recurse -Force $Tmp -ErrorAction SilentlyContinue
}
