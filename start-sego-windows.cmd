@echo off
setlocal EnableExtensions
title Sego Agent Bootstrap

set "ROOT=%~dp0"
set "INSTALL_DIR=%USERPROFILE%\sego"
set "LAUNCHER=%INSTALL_DIR%\Sego.cmd"

echo Sego Agent Windows Bootstrap
echo.
echo This script is for users who downloaded the GitHub source ZIP.
echo It installs the latest Sego release binary, creates a desktop shortcut,
echo and then opens Sego.
echo.

if not exist "%LAUNCHER%" (
  powershell -NoProfile -ExecutionPolicy Bypass -File "%ROOT%install.ps1"
  if errorlevel 1 (
    echo.
    echo Sego installation failed.
    pause
    exit /b 1
  )
)

if not exist "%LAUNCHER%" (
  echo.
  echo Could not find "%LAUNCHER%".
  echo Try running install.ps1 manually from PowerShell.
  pause
  exit /b 1
)

call "%LAUNCHER%" %*
