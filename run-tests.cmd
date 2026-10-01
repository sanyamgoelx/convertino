@echo off
rem Double-click to run Convertino's tests, including real conversions with the tools in src-tauri\tools.
title Convertino tests
cd /d "%~dp0"
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\run-tests.ps1"
echo.
pause
