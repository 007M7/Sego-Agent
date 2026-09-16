# Sego Agent — Windows one-liner installer (PowerShell)
# Run: irm https://raw.githubusercontent.com/007M7/Sego-Agent/main/install.ps1 | iex
#
# The installer pins a release tag and verifies the published SHA-256 before
# anything is written into place. Overrides, both explicit:
#   $env:SEGO_INSTALL_VERSION = "latest"            # float to the newest release
#   $env:SEGO_INSTALL_ALLOW_UNVERIFIED = "1"        # skip the checksum check

$ErrorActionPreference = "Stop"
$Repo = "007M7/Sego-Agent"
$Binary = "sego.exe"
$InstallDir = "$env:USERPROFILE\sego"
$BinPath = "$InstallDir\$Binary"
$LauncherPath = "$InstallDir\Sego.cmd"
$UpdaterPath = "$InstallDir\Update Sego.cmd"

# Bumped when a release is cut. The default is deliberately a tag, not
# "latest": a floating pointer means the bytes a user installs change without
# anything in this file changing.
$SegoVersion = $env:SEGO_INSTALL_VERSION
if ([string]::IsNullOrWhiteSpace($SegoVersion)) { $SegoVersion = "v0.1.9" }

Write-Host "Sego Agent Installer" -ForegroundColor Cyan
Write-Host ""

# Create install directory
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null

function Get-ReleaseBaseUrl {
    if ($SegoVersion -eq "latest") {
        return "https://github.com/$Repo/releases/latest/download"
    }
    return "https://github.com/$Repo/releases/download/$SegoVersion"
}

function Get-ExpectedSha256 {
    param([string]$ChecksumsPath, [string]$FileName)
    foreach ($line in Get-Content -Path $ChecksumsPath) {
        $parts = $line -split '\s+', 2
        if ($parts.Count -lt 2) { continue }
        $listed = $parts[1].Trim()
        if ($listed -eq $FileName -or $listed -eq "*$FileName") {
            return $parts[0].Trim().ToLower()
        }
    }
    return $null
}

# Download the release binary and verify it before it replaces anything.
$BaseUrl = Get-ReleaseBaseUrl
$TempBinary = Join-Path $env:TEMP ("sego-" + [guid]::NewGuid().ToString("N") + ".exe")
$Downloaded = $false

Write-Host "Downloading $Binary ($SegoVersion) ..." -ForegroundColor Yellow
try {
    Invoke-WebRequest -Uri "$BaseUrl/$Binary" -OutFile $TempBinary -UseBasicParsing
    $Downloaded = $true
} catch {
    Write-Host "No prebuilt binary found for $SegoVersion." -ForegroundColor Yellow
}

if ($Downloaded) {
    if ($env:SEGO_INSTALL_ALLOW_UNVERIFIED -eq "1") {
        Write-Host "WARNING: SEGO_INSTALL_ALLOW_UNVERIFIED=1 - skipping checksum verification." -ForegroundColor Red
    } else {
        $ChecksumsPath = Join-Path $env:TEMP ("sego-checksums-" + [guid]::NewGuid().ToString("N") + ".txt")
        Invoke-WebRequest -Uri "$BaseUrl/checksums.txt" -OutFile $ChecksumsPath -UseBasicParsing
        $Expected = Get-ExpectedSha256 -ChecksumsPath $ChecksumsPath -FileName $Binary
        if (-not $Expected) {
            Remove-Item -Force $TempBinary, $ChecksumsPath -ErrorAction SilentlyContinue
            throw "checksums.txt for $SegoVersion does not list $Binary; refusing to install an unverified binary."
        }
        $Actual = (Get-FileHash -Path $TempBinary -Algorithm SHA256).Hash.ToLower()
        Remove-Item -Force $ChecksumsPath -ErrorAction SilentlyContinue
        if ($Actual -ne $Expected) {
            Remove-Item -Force $TempBinary -ErrorAction SilentlyContinue
            throw "Checksum mismatch for $Binary.`n  expected $Expected`n  actual   $Actual`nThe download was not installed. Do not run the downloaded file."
        }
        Write-Host "Checksum verified ($Expected)." -ForegroundColor Green
    }
    # Only now does the existing installation get replaced.
    Move-Item -Force -Path $TempBinary -Destination $BinPath
} else {
    # Fallback: build from source, pinned to the same tag.
    Write-Host "Building from source at $SegoVersion..." -ForegroundColor Yellow
    Write-Host "This requires Rust: https://rustup.rs" -ForegroundColor Yellow
    $SrcDir = Join-Path $env:TEMP ("sego-src-" + [guid]::NewGuid().ToString("N"))
    if ($SegoVersion -eq "latest") {
        git clone "https://github.com/$Repo.git" $SrcDir
    } else {
        git clone --branch $SegoVersion --depth 1 "https://github.com/$Repo.git" $SrcDir
    }
    Push-Location "$SrcDir\rust"
    cargo build --release
    Copy-Item "target\release\$Binary" $BinPath
    Pop-Location
}

Write-Host "Installed to $BinPath" -ForegroundColor Green
Write-Host ""

# Create a double-click launcher that keeps the console open.
$LauncherContent = @'
@echo off
setlocal EnableExtensions
title Sego Agent

if "%DEEPSEEK_API_KEY%%ANTHROPIC_API_KEY%"=="" (
  echo [Sego] No API key was found in your environment.
  echo [Sego] Configure one of these before model calls:
  echo   setx DEEPSEEK_API_KEY "your-key"
  echo   setx ANTHROPIC_API_KEY "your-key"
  echo.
  echo [Sego] After running setx, close this window and open Sego again.
  echo.
)

echo [Sego] Active workspace: %CD%
echo [Sego] Tip: inside Sego, type /cd "D:\YourProject" or launch with:
echo        Sego.cmd --cwd "D:\YourProject"
echo.

set "SEGO_PAUSE_ON_ERROR=1"
"%~dp0sego.exe" %*
set "SEGO_EXIT=%ERRORLEVEL%"
if not "%SEGO_EXIT%"=="0" (
  echo.
  echo Sego exited with code %SEGO_EXIT%.
)
exit /b %SEGO_EXIT%
'@
Set-Content -Path $LauncherPath -Value $LauncherContent -Encoding ASCII
Write-Host "Created launcher: $LauncherPath" -ForegroundColor Green

$UpdaterContent = @'
@echo off
setlocal EnableExtensions
title Update Sego
"%~dp0sego.exe" update
echo.
pause
'@
Set-Content -Path $UpdaterPath -Value $UpdaterContent -Encoding ASCII
Write-Host "Created updater: $UpdaterPath" -ForegroundColor Green

# Create desktop shortcut for normal Windows users.
try {
    $DesktopPath = [Environment]::GetFolderPath("Desktop")
    if (-not [string]::IsNullOrWhiteSpace($DesktopPath)) {
        $ShortcutPath = Join-Path $DesktopPath "Sego.lnk"
        $Shell = New-Object -ComObject WScript.Shell
        $Shortcut = $Shell.CreateShortcut($ShortcutPath)
        $Shortcut.TargetPath = $LauncherPath
        $Shortcut.WorkingDirectory = $InstallDir
        $Shortcut.IconLocation = "$BinPath,0"
        $Shortcut.Description = "Open Sego Agent"
        $Shortcut.Save()
        Write-Host "Created desktop shortcut: $ShortcutPath" -ForegroundColor Green

        $UpdateShortcutPath = Join-Path $DesktopPath "Update Sego.lnk"
        $UpdateShortcut = $Shell.CreateShortcut($UpdateShortcutPath)
        $UpdateShortcut.TargetPath = $UpdaterPath
        $UpdateShortcut.WorkingDirectory = $env:USERPROFILE
        $UpdateShortcut.IconLocation = "$BinPath,0"
        $UpdateShortcut.Description = "Update Sego Agent"
        $UpdateShortcut.Save()
        Write-Host "Created desktop shortcut: $UpdateShortcutPath" -ForegroundColor Green
    }
} catch {
    Write-Host "Could not create desktop shortcut: $($_.Exception.Message)" -ForegroundColor Yellow
    Write-Host "You can still open Sego with: $LauncherPath" -ForegroundColor Yellow
}

# Add to PATH
$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($UserPath -notlike "*$InstallDir*") {
    [Environment]::SetEnvironmentVariable("Path", "$UserPath;$InstallDir", "User")
    $env:Path += ";$InstallDir"
    Write-Host "Added to PATH. Restart terminal or run: `$env:Path += ';$InstallDir'" -ForegroundColor Green
}

Write-Host ""
Write-Host "Setup complete! Configure your model:" -ForegroundColor Cyan
Write-Host ""
Write-Host "  # DeepSeek (recommended, native support)" -ForegroundColor White
Write-Host "  setx DEEPSEEK_API_KEY ""sk-your-deepseek-key""" -ForegroundColor White
Write-Host "  setx DEEPSEEK_MODEL ""deepseek-v4-flash""    # optional, defaults to flash" -ForegroundColor White
Write-Host ""
Write-Host "  # Or Anthropic (alternative)" -ForegroundColor White
Write-Host "  setx ANTHROPIC_API_KEY ""sk-your-anthropic-key""" -ForegroundColor White
Write-Host ""
Write-Host "Run from terminal: sego" -ForegroundColor White
Write-Host "Update from terminal: sego update" -ForegroundColor White
Write-Host "Or double-click the Sego desktop shortcut / $LauncherPath." -ForegroundColor White
Write-Host "Or double-click Update Sego / $UpdaterPath." -ForegroundColor White
Write-Host "Tip: do not double-click sego.exe directly; use Sego.cmd so errors stay visible." -ForegroundColor Yellow
