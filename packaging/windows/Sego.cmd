@echo off
setlocal EnableExtensions
title Sego Agent

if not exist "%~dp0sego.exe" (
  echo [Sego] sego.exe was not found next to this launcher.
  echo [Sego] Download the Windows release package again, or run install.ps1.
  pause
  exit /b 1
)

if "%DEEPSEEK_API_KEY%%ANTHROPIC_API_KEY%"=="" (
  echo [Sego] No API key was found in your environment.
  echo [Sego] Configure one of these before model calls:
  echo   setx DEEPSEEK_API_KEY "your-key"
  echo   setx ANTHROPIC_API_KEY "your-key"
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
