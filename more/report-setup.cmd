@echo off
rem One time: connects error reports to your Discord channel.
cd /d "%~dp0.."
if not exist logs mkdir logs
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0..\scripts\report-setup.ps1"
