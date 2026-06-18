# Sego Agent ? Windows one-liner installer (PowerShell)
# Run: irm https://raw.githubusercontent.com/007M7/Sego-Agent/main/install.ps1 | iex

$ErrorActionPreference = "Stop"
$Repo = "007M7/Sego-Agent"
$Binary = "sego.exe"
$InstallDir = "$env:USERPROFILE\sego"
$BinPath = "$InstallDir\$Binary"
$LauncherPath = "$InstallDir\Sego.cmd"

Write-Host "Sego Agent Installer" -ForegroundColor Cyan
Write-Host ""

# Create install directory
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null

# Download latest release
$ReleaseUrl = "https://github.com/$Repo/releases/latest/download/$Binary"
Write-Host "Downloading $Binary ..." -ForegroundColor Yellow
try {
    Invoke-WebRequest -Uri $ReleaseUrl -OutFile $BinPath -UseBasicParsing
} catch {
    # Fallback: build from source
    Write-Host "No prebuilt binary found. Building from source..." -ForegroundColor Yellow
    Write-Host "This requires Rust: https://rustup.rs" -ForegroundColor Yellow
    $SrcDir = Join-Path $env:TEMP ("sego-src-" + [guid]::NewGuid().ToString("N"))
    git clone "https://github.com/$Repo.git" $SrcDir
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
cd /d "%USERPROFILE%"

if "%DEEPSEEK_API_KEY%%ANTHROPIC_API_KEY%"=="" (
  echo [Sego] No API key was found in your environment.
  echo [Sego] Configure one of these before model calls:
  echo   setx DEEPSEEK_API_KEY "your-key"
  echo   setx ANTHROPIC_API_KEY "your-key"
  echo.
  echo [Sego] After running setx, close this window and open Sego again.
  echo.
)

set "SEGO_PAUSE_ON_ERROR=1"
"%~dp0sego.exe" %*
set "SEGO_EXIT=%ERRORLEVEL%"
echo.
echo Sego exited with code %SEGO_EXIT%.
pause
exit /b %SEGO_EXIT%
'@
Set-Content -Path $LauncherPath -Value $LauncherContent -Encoding ASCII
Write-Host "Created launcher: $LauncherPath" -ForegroundColor Green

# Create desktop shortcut for normal Windows users.
try {
    $DesktopPath = [Environment]::GetFolderPath("Desktop")
    if (-not [string]::IsNullOrWhiteSpace($DesktopPath)) {
        $ShortcutPath = Join-Path $DesktopPath "Sego.lnk"
        $Shell = New-Object -ComObject WScript.Shell
        $Shortcut = $Shell.CreateShortcut($ShortcutPath)
        $Shortcut.TargetPath = $LauncherPath
        $Shortcut.WorkingDirectory = $env:USERPROFILE
        $Shortcut.IconLocation = "$BinPath,0"
        $Shortcut.Description = "Open Sego Agent"
        $Shortcut.Save()
        Write-Host "Created desktop shortcut: $ShortcutPath" -ForegroundColor Green
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
Write-Host "Or double-click the Sego desktop shortcut / $LauncherPath." -ForegroundColor White
Write-Host "Tip: do not double-click sego.exe directly; use Sego.cmd so errors stay visible." -ForegroundColor Yellow
