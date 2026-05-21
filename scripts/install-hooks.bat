@echo off
REM ============================================================================
REM Sego Agent — Install Git Hooks (Windows)
REM ============================================================================
REM Installs pre-commit hook for Windows
REM Usage: scripts\install-hooks.bat
REM ============================================================================

set "SCRIPT_DIR=%~dp0"
set "HOOK_SOURCE=%SCRIPT_DIR%pre-commit"
set "HOOK_DEST=%SCRIPT_DIR%..\.git\hooks\pre-commit"

if not exist "%HOOK_SOURCE%" (
    echo ❌ Hook source not found: %HOOK_SOURCE%
    exit /b 1
)

if not exist "%SCRIPT_DIR%..\.git\hooks" (
    echo ❌ Not a git repository (no .git\hooks directory)
    exit /b 1
)

copy /Y "%HOOK_SOURCE%" "%HOOK_DEST%" >nul
echo ✅ Pre-commit hook installed: %HOOK_DEST%
echo.
echo The hook will run:
echo   - cargo fmt --all --check
echo   - cargo clippy --workspace --all-targets -- -D warnings
echo.
echo To skip the hook for a commit: git commit --no-verify
