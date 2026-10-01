@echo off
rem Publishes the current version as a GitHub Release. Log: release.log
title Convertino release
cd /d "%~dp0"
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\release.ps1"
