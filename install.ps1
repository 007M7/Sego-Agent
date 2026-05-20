# Sego Agent — Windows one-liner installer (PowerShell)
# Run: irm https://raw.githubusercontent.com/007M7/Sego-Agent/main/install.ps1 | iex

$ErrorActionPreference = "Stop"
$Repo = "007M7/Sego-Agent"
$Binary = "sego.exe"
$InstallDir = "$env:USERPROFILE\sego"
$BinPath = "$InstallDir\$Binary"

Write-Host "🦞 Sego Agent Installer" -ForegroundColor Cyan
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
    $SrcDir = "$env:TEMP\sego-src"
    git clone "https://github.com/$Repo.git" $SrcDir
    Push-Location "$SrcDir\rust"
    cargo build --release
    Copy-Item "target\release\$Binary" $BinPath
    Pop-Location
}

Write-Host "Installed to $BinPath" -ForegroundColor Green
Write-Host ""

# Add to PATH
$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($UserPath -notlike "*$InstallDir*") {
    [Environment]::SetEnvironmentVariable("Path", "$UserPath;$InstallDir", "User")
    $env:Path += ";$InstallDir"
    Write-Host "Added to PATH. Restart terminal or run: `$env:Path += ';$InstallDir'" -ForegroundColor Green
}

Write-Host ""
Write-Host "Setup complete! Run 'sego' and set your API key:" -ForegroundColor Cyan
Write-Host "  set ANTHROPIC_API_KEY=sk-your-key" -ForegroundColor White
Write-Host "  set ANTHROPIC_BASE_URL=https://api.deepseek.com/anthropic" -ForegroundColor White
Write-Host "  sego" -ForegroundColor White
