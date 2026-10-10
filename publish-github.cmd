@echo off
rem Double-click to put Convertino on GitHub (public) and push the latest code.
title Convertino to GitHub
cd /d "%~dp0"
if not exist logs mkdir logs
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\publish-github.ps1"
echo.
pause
