@echo off
rem Checks which LibreOffice downloads the mirrors have right now (log: lo-check.log).
cd /d "%~dp0"
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\lo-check.ps1"
