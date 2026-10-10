@echo off
rem Double-click to run all three checks in a row: fetch-tools, run-tests, run-smoke (logs: logs\tools.log, logs\test.log, logs\smoke.log).
title Convertino checks
cd /d "%~dp0"
if not exist logs mkdir logs
echo ==^> Converters
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\fetch-tools.ps1"
echo ==^> Tests
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\run-tests.ps1"
echo ==^> Smoke run
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\run-smoke.ps1"
echo.
pause
