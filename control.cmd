@echo off
rem Opens the Don't Forget control panel (PowerShell 7).
pwsh -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0tools\control.ps1" %*
