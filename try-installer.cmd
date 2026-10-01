@echo off
rem Downloads the latest built Windows installer from GitHub and starts it.
cd /d "%~dp0"
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\try-installer.ps1"
