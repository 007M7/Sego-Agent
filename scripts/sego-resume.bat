@echo off
REM sego-resume.bat - Resume the latest Sego session after a crash or interruption.
REM
REM Usage:
REM   sego-resume                  Restore latest session, enter REPL
REM   sego-resume /status          Show status of restored session
REM   sego-resume /recovery-export Export recovery summary to .sego/recovery/recovery-summary.md
REM   sego-resume latest /status   Equivalent (explicit "latest" alias)
REM
REM Any extra args are forwarded to the resumed session.

setlocal enabledelayedexpansion

REM Locate the sego binary: prefer a local cargo build, then PATH.
set "SEGO_BIN="

if exist "%~dp0..\rust\target\debug\sego.exe" (
    set "SEGO_BIN=%~dp0..\rust\target\debug\sego.exe"
) else if defined CARGO_TARGET_DIR (
    if exist "%CARGO_TARGET_DIR%\debug\sego.exe" (
        set "SEGO_BIN=%CARGO_TARGET_DIR%\debug\sego.exe"
    )
)

if not defined SEGO_BIN (
    where sego >nul 2>nul
    if !errorlevel! equ 0 (
        set "SEGO_BIN=sego"
    ) else (
        echo sego-resume: sego binary not found.
        echo Build it first with:  cargo build -p rusty-claude-cli
        echo Or ensure 'sego' is on your PATH.
        exit /b 1
    )
)

"%SEGO_BIN%" --resume latest %*
endlocal
