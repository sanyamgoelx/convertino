@echo off
rem Publishes the current version as a GitHub Release. Log: logs\release.log
title Convertino release
cd /d "%~dp0"
if not exist logs mkdir logs
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\release.ps1"
