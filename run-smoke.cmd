@echo off
rem Double-click to convert every file in test-files the way the wheel would (results in test-files\results).
title Convertino smoke test
cd /d "%~dp0"
if not exist logs mkdir logs
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\run-smoke.ps1"
echo.
pause
