@echo off
rem Re-runs the failed jobs of the latest GitHub build.
cd /d "%~dp0"
if not exist logs mkdir logs
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\ci-rerun.ps1"
