@echo off
rem Saves the latest GitHub build log (failed steps) to ci.log.
cd /d "%~dp0"
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\ci-log.ps1"
