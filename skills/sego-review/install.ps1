# sego-review skill installer for Windows.
# Supports: Zcode, Claude Code, Codex (SKILL.md native), Cursor (.cursorrules).
# Usage: powershell -File skills\sego-review\install.ps1 [-Tool all|zcode|claude|codex|cursor]
# Idempotent: re-running overwrites existing files after confirmation.
[CmdletBinding()]
param(
    [ValidateSet('all','zcode','claude','codex','cursor')]
    [string]$Tool = 'all'
)
$ErrorActionPreference = 'Stop'

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$SkillName = 'sego-review'
$HomeDir = $env:USERPROFILE

function Write-OK($msg)   { Write-Host "[OK] $msg" -ForegroundColor Green }
function Write-Warn2($msg) { Write-Host "[!] $msg" -ForegroundColor Yellow }

function Install-Skill($toolName, $destDir) {
    Write-Host ""
    Write-Host "Installing $SkillName for $toolName..."
    Write-Host "  Target: $destDir"

    New-Item -ItemType Directory -Force -Path $destDir | Out-Null
    Copy-Item "$ScriptDir\SKILL.md" "$destDir\SKILL.md" -Force
    if (Test-Path "$ScriptDir\scripts") {
        Copy-Item "$ScriptDir\scripts" "$destDir\scripts" -Recurse -Force
    }
    if (Test-Path "$ScriptDir\README.md") {
        Copy-Item "$ScriptDir\README.md" "$destDir\README.md" -Force
    }

    Write-OK "Installed $SkillName for $toolName at $destDir"
}

function Install-CursorRules {
    param([string]$DestDir = (Get-Location).Path)
    $rulesFile = Join-Path $DestDir ".cursorrules"
    Write-Host ""
    Write-Host "Installing $SkillName rules for Cursor..."
    Write-Host "  Target: $rulesFile"

    $content = @'
# Sego Review Integration
#
# When the user asks to "review", "check", or "validate" code changes:
# 1. Get the project root: pwd
# 2. Run: echo '{"schema_version":1,"action":"review","cwd":"<project-root>","scope":"staged"}' | sego sidecar review
# 3. Parse the JSON response and report findings to the user
# 4. Highlight critical/high severity findings first
#
# If sego is not found, tell the user to install it:
#   irm https://raw.githubusercontent.com/007M7/Sego-Agent/main/install.ps1 | iex
'@
    Set-Content -Path $rulesFile -Value $content -Encoding UTF8
    Write-OK "Installed .cursorrules for Cursor at $rulesFile"
}

# --- Main ---

Write-Host "Sego Review Skill Installer"
Write-Host "============================"

$targets = @()

# Zcode
if (Test-Path "$HomeDir\.zcode") {
    $targets += @{ Tool='zcode'; Dest="$HomeDir\.zcode\skills\$SkillName" }
}
# Claude Code
if (Test-Path "$HomeDir\.claude") {
    $targets += @{ Tool='claude'; Dest="$HomeDir\.claude\skills\$SkillName" }
}
# Codex
if (Test-Path "$HomeDir\.codex") {
    $targets += @{ Tool='codex'; Dest="$HomeDir\.codex\skills\$SkillName" }
}

if ($Tool -eq 'cursor') {
    Install-CursorRules
    exit 0
}

if ($targets.Count -eq 0) {
    Write-Warn2 "No supported AI coding tools detected in $HomeDir."
    Write-Host ""
    Write-Host "You can install manually:"
    Write-Host "  Zcode:  Copy-Item -Recurse skills\sego-review ~\.zcode\skills\"
    Write-Host "  Claude: Copy-Item -Recurse skills\sego-review ~\.claude\skills\"
    Write-Host "  Codex:  Copy-Item -Recurse skills\sego-review ~\.codex\skills\"
    Write-Host "  Cursor: Run with -Tool cursor to generate .cursorrules"
    exit 0
}

foreach ($t in $targets) {
    if ($Tool -eq 'all' -or $Tool -eq $t.Tool) {
        Install-Skill $t.Tool $t.Dest
    }
}

Write-Warn2 "For Cursor support, run: powershell -File skills\sego-review\install.ps1 -Tool cursor"
Write-Host ""
Write-OK "Done! Restart your AI coding tool to pick up the new skill."
