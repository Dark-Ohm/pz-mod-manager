@echo off
chcp 65001 >nul
rem Установка зависимостей мод-менеджера и сборка: scripts\install.bat
rem Ставит Visual Studio Build Tools (C++ линкер), Rust и собирает релизный бинарь.
rem Повторный запуск безопасен: уже установленное пропускается.
rem Только зависимости, без сборки: scripts\install.bat --deps-only
setlocal
cd /d "%~dp0.."

set "DEPS_ONLY=0"
if /i "%~1"=="--deps-only" set "DEPS_ONLY=1"
set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"

where winget >nul 2>nul
if errorlevel 1 goto no_winget

rem --- 1. Visual Studio Build Tools (нужны Rust-у для линковки) ---------------
set "VSWHERE=%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe"
set "HAVE_MSVC="
call :find_msvc
if defined HAVE_MSVC goto msvc_ok

echo [+] Ставлю Visual Studio Build Tools с C++ (появится запрос UAC, установка займёт несколько минут)
winget install --id Microsoft.VisualStudio.2022.BuildTools -e --accept-package-agreements --accept-source-agreements --override "--passive --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
call :find_msvc
if not defined HAVE_MSVC (
    echo [!] C++ инструменты не обнаружены. Открой Visual Studio Installer, выбери Build Tools
    echo [!] и поставь рабочую нагрузку "Разработка классических приложений на C++".
    pause
    exit /b 1
)
:msvc_ok
echo [+] MSVC: %HAVE_MSVC%

rem --- 2. Rust ----------------------------------------------------------------
where rustup >nul 2>nul
if errorlevel 1 (
    echo [+] Ставлю rustup
    winget install --id Rustlang.Rustup -e --accept-package-agreements --accept-source-agreements
)

where rustup >nul 2>nul
if errorlevel 1 (
    echo [!] rustup не найден в PATH. Закрой это окно, открой новое и запусти scripts\install.bat ещё раз.
    pause
    exit /b 1
)

rustup show active-toolchain >nul 2>nul
if errorlevel 1 (
    echo [+] Ставлю Rust stable
    rustup default stable-msvc
    if errorlevel 1 exit /b 1
)

where cargo >nul 2>nul
if errorlevel 1 (
    echo [!] cargo не найден в PATH. Закрой это окно, открой новое и запусти scripts\install.bat ещё раз.
    pause
    exit /b 1
)
for /f "delims=" %%v in ('rustc --version') do echo [+] Rust: %%v

if "%DEPS_ONLY%"=="1" (
    echo [+] Зависимости готовы.
    goto done
)

rem --- 3. Сборка --------------------------------------------------------------
echo [+] Собираю мод-менеджер
call "%~dp0build.bat"
if errorlevel 1 (
    echo [!] Сборка не удалась.
    pause
    exit /b 1
)
echo [+] Готово. Запуск: scripts\run.bat --no-build
goto done

:no_winget
echo [!] Не найден winget (App Installer из Microsoft Store). Поставь вручную:
echo [!]   1. Visual Studio Build Tools с рабочей нагрузкой C++: https://aka.ms/vs/17/release/vs_BuildTools.exe
echo [!]   2. Rust: https://rustup.rs
echo [!] Затем запусти scripts\build.bat.
pause
exit /b 1

:find_msvc
if not exist "%VSWHERE%" exit /b 0
for /f "usebackq delims=" %%i in (`"%VSWHERE%" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "HAVE_MSVC=%%i"
exit /b 0

:done
endlocal
