@echo off
REM ============================================================================
REM Sego Agent — Dev Build Script (Windows)
REM ============================================================================
REM One-command full build cycle: format → lint → test → build release
REM Usage: scripts\dev-build.bat [--fast] [--release] [--check-only]
REM ============================================================================

setlocal enabledelayedexpansion
cd /d "%~dp0..\rust"

set FAST=false
set RELEASE=false
set CHECK_ONLY=false

:parse_args
if "%~1"=="" goto :run
if "%~1"=="--fast" set FAST=true
if "%~1"=="--release" set RELEASE=true
if "%~1"=="--check-only" set CHECK_ONLY=true
if "%~1"=="--help" goto :help
if "%~1"=="-h" goto :help
shift
goto :parse_args

:help
echo Usage: scripts\dev-build.bat [OPTIONS]
echo.
echo Options:
echo   --fast        Skip full workspace tests (CLI tests only)
echo   --release     Also build release binary
echo   --check-only  Only format + clippy, no tests
exit /b 0

:run
echo ============================================
echo Sego Agent — Dev Build
echo ============================================
echo.

REM Step 1: Format
echo ━ Step 1/3: cargo fmt --all --check
cargo fmt --all --check
if errorlevel 1 (
    echo ❌ Format failed. Fix with: cargo fmt --all
    exit /b 1
)
echo ✅ Format OK
echo.

REM Step 2: Clippy
echo ━ Step 2/3: cargo clippy --workspace --all-targets -- -D warnings
cargo clippy --workspace --all-targets -- -D warnings
if errorlevel 1 (
    echo ❌ Clippy failed. Fix warnings above.
    exit /b 1
)
echo ✅ Clippy OK
echo.

REM Step 3: Tests
if "%CHECK_ONLY%"=="true" (
    echo ━ Step 3/3: Skipped (--check-only)
) else if "%FAST%"=="true" (
    echo ━ Step 3/3: cargo test -p rusty-claude-cli (--fast)
    cargo test -p rusty-claude-cli
    if errorlevel 1 (
        echo ❌ CLI tests failed
        exit /b 1
    )
    echo ✅ CLI tests OK
) else (
    echo ━ Step 3/3: cargo test --workspace
    cargo test --workspace
    if errorlevel 1 (
        echo ❌ Workspace tests failed
        exit /b 1
    )
    echo ✅ Workspace tests OK
)
echo.

REM Step 4: Build release
if "%RELEASE%"=="true" (
    echo ━ Step 4/4: cargo build --release
    cargo build --release
    if errorlevel 1 (
        echo ❌ Release build failed
        exit /b 1
    )
    echo ✅ Release build OK
    echo    Binary: rust\target\release\sego.exe
    echo.
)

echo ============================================
echo ✅ All checks passed!
echo ============================================
