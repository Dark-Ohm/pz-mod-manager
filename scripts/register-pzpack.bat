@echo off
rem Associates .pzpack files with this program for the current Windows user (no admin rights needed):
rem double-click on a pack file then opens PZ Mod Manager with the "install pack" window.
rem Undo: scripts\register-pzpack.bat remove
setlocal
set "EXE=%~dp0..\target\release\pz-mod-manager.exe"
for %%I in ("%EXE%") do set "EXE=%%~fI"
if /i "%~1"=="remove" (
    reg delete "HKCU\Software\Classes\.pzpack" /f >nul 2>&1
    reg delete "HKCU\Software\Classes\PZModManager.Pack" /f >nul 2>&1
    echo .pzpack association removed
    exit /b 0
)
if not exist "%EXE%" (
    echo Build the program first: scripts\build.bat
    exit /b 1
)
reg add "HKCU\Software\Classes\.pzpack" /ve /d "PZModManager.Pack" /f >nul
reg add "HKCU\Software\Classes\PZModManager.Pack" /ve /d "PZ mod pack" /f >nul
reg add "HKCU\Software\Classes\PZModManager.Pack\shell\open\command" /ve /d "\"%EXE%\" \"%%1\"" /f >nul
echo .pzpack files now open with %EXE%
