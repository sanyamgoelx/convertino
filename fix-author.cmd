@echo off
rem One time: credits all commits and release tags to your signed-in GitHub account.
cd /d "%~dp0"
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\fix-author.ps1"
