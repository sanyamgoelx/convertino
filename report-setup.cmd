@echo off
rem One time: connects error reports to your Discord channel.
cd /d "%~dp0"
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\report-setup.ps1"
