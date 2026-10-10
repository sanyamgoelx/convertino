@echo off
rem Saves the latest GitHub build log (failed steps) to logs\ci.log.
cd /d "%~dp0"
if not exist logs mkdir logs
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\ci-log.ps1"
