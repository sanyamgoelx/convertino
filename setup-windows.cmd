@echo off
rem Double-click to install everything Convertino needs on Windows, then start it.
title Convertino setup
cd /d "%~dp0"
if not exist logs mkdir logs
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\setup-windows.ps1"
echo.
pause
