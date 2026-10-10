@echo off
rem Cancels the GitHub Actions runs that are still going (log: logs\ci-cancel.log).
cd /d "%~dp0.."
if not exist logs mkdir logs
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0..\scripts\ci-cancel.ps1"
