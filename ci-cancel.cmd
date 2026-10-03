@echo off
rem Cancels the GitHub Actions runs that are still going (log: ci-cancel.log).
cd /d "%~dp0"
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\ci-cancel.ps1"
