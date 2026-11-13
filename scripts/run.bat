@echo off
chcp 65001 >nul
rem Собирает свежий бинарь и запускает его: scripts\run.bat
rem Без пересборки: scripts\run.bat --no-build
setlocal
cd /d "%~dp0.."

if /i "%~1"=="--no-build" goto launch

call "%~dp0build.bat"
if errorlevel 1 (
    echo Сборка не удалась, запуск отменён.
    pause
    exit /b 1
)

:launch
start "" "%CD%\target\release\pz-mod-manager.exe"
endlocal
