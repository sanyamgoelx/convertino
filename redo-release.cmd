@echo off
rem Rebuilds and republishes the current version from the latest code. Log: release.log
cd /d "%~dp0"
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\redo-release.ps1"
