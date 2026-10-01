@echo off
rem One time: makes the update-signing key and gives it to GitHub. Log: release-setup.log
title Convertino release setup
cd /d "%~dp0"
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\release-setup.ps1"
