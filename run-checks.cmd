@echo off
rem Double-click to run all three checks in a row: fetch-tools, run-tests, run-smoke (logs: tools.log, test.log, smoke.log).
title Convertino checks
cd /d "%~dp0"
echo ==^> Converters
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\fetch-tools.ps1"
echo ==^> Tests
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\run-tests.ps1"
echo ==^> Smoke run
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\run-smoke.ps1"
echo.
echo ALL CHECKS FINISHED > checks.done
pause
