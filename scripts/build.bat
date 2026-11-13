@echo off
chcp 65001 >nul
rem Сборка свежего релизного бинаря: scripts\build.bat
setlocal
cd /d "%~dp0.."
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"

where cargo >nul 2>nul
if errorlevel 1 (
    echo cargo не найден. Установи Rust: https://rustup.rs
    exit /b 1
)

cargo build --release
if errorlevel 1 exit /b 1

echo Готово: %CD%\target\release\pz-mod-manager.exe
endlocal
