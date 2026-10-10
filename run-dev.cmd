@echo off
rem Double-click to start Convertino (development build).
title Convertino
cd /d "%~dp0"
if not exist logs mkdir logs
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\run-dev.ps1"
echo.
pause
