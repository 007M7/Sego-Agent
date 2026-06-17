# sego-review.ps1 — Invoke Sego sidecar review from a skill package (Windows).
#
# Reads a JSON request from stdin, passes it to `sego sidecar review`,
# and outputs the JSON response to stdout.
#
# Usage:
#   $request | powershell -File sego-review.ps1
#   # or pipe directly:
#   '{"schema_version":1,"action":"review","cwd":"C:\\project","scope":"staged"}' | powershell -File sego-review.ps1
[CmdletBinding()]
param()
$ErrorActionPreference = "Stop"

# Locate the sego binary: local cargo build > CARGO_TARGET_DIR > PATH.
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $ScriptDir))

$SegoBin = $null

if (Test-Path (Join-Path $RepoRoot "rust\target\debug\sego.exe")) {
    $SegoBin = Join-Path $RepoRoot "rust\target\debug\sego.exe"
} elseif (Test-Path (Join-Path $RepoRoot "rust\target\release\sego.exe")) {
    $SegoBin = Join-Path $RepoRoot "rust\target\release\sego.exe"
} elseif ($env:CARGO_TARGET_DIR -and (Test-Path (Join-Path $env:CARGO_TARGET_DIR "debug\sego.exe"))) {
    $SegoBin = Join-Path $env:CARGO_TARGET_DIR "debug\sego.exe"
} else {
    $found = $null
    try { $found = (Get-Command sego -ErrorAction Stop).Source } catch {}
    if ($found) {
        $SegoBin = "sego"
    }
}

if (-not $SegoBin) {
    $err = @{ schema_version = 1; status = "error"; error = @{ code = "sego_not_found"; message = "Sego binary not found. Build with: cargo build -p rusty-claude-cli, or add sego to PATH." } } | ConvertTo-Json -Compress
    [Console]::Error.WriteLine($err)
    exit 1
}

# Read stdin and pipe to sego sidecar review.
$input | & $SegoBin sidecar review
exit $LASTEXITCODE
