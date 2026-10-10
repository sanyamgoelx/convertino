@echo off
rem One time: makes the update-signing key and gives it to GitHub. Log: logs\release-setup.log
title Convertino release setup
cd /d "%~dp0.."
if not exist logs mkdir logs
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0..\scripts\release-setup.ps1"
