@echo off
rem Double-click to download (or trim) the converters Convertino uses.
title Convertino tools
cd /d "%~dp0"
if not exist logs mkdir logs
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\fetch-tools.ps1"
echo.
pause
