#!/usr/bin/env bash
# Установка зависимостей мод-менеджера и сборка: ./scripts/install.sh
# Ставит системные пакеты (нужен sudo), Rust-тулчейн и собирает релизный бинарь.
# Повторный запуск безопасен: уже установленное пропускается.
# Только зависимости, без сборки: ./scripts/install.sh --deps-only
set -euo pipefail
cd "$(dirname "$0")/.."

DEPS_ONLY=0
[[ "${1:-}" == "--deps-only" ]] && DEPS_ONLY=1

say()  { printf '\033[1;32m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m!!\033[0m %s\n' "$*" >&2; }

if [[ $EUID -eq 0 ]]; then
    warn "Не запускай от root: Rust должен ставиться в твой домашний каталог."
    exit 1
fi
SUDO=sudo
command -v sudo >/dev/null 2>&1 || SUDO=""

# --- 1. Системные пакеты ----------------------------------------------------
# build tools: линковка Rust-кода
# libxkbcommon/wayland/GL: окно egui (Wayland и X11)
# xdg-desktop-portal: диалоги выбора файлов (rfd)
# 32-битные libc/libgcc: нужны SteamCMD (скачивание воркшопа и сервера)
# curl: загрузка rustup
if [[ -r /etc/os-release ]]; then . /etc/os-release; fi
ID_ALL="${ID:-} ${ID_LIKE:-}"

if [[ " $ID_ALL " == *" arch "* ]]; then
    say "Arch-based система: ставлю пакеты через pacman"
    if ! grep -qE '^\[multilib\]' /etc/pacman.conf; then
        warn "Репозиторий [multilib] выключен в /etc/pacman.conf — раскомментируй его и сделай 'sudo pacman -Syu',"
        warn "иначе lib32-пакеты для SteamCMD не поставятся."
    fi
    $SUDO pacman -S --needed --noconfirm \
        base-devel curl rustup \
        libxkbcommon wayland libglvnd mesa vulkan-icd-loader \
        xdg-desktop-portal \
        lib32-glibc lib32-gcc-libs
elif [[ " $ID_ALL " == *" debian "* || " $ID_ALL " == *" ubuntu "* ]]; then
    say "Debian/Ubuntu: ставлю пакеты через apt"
    $SUDO dpkg --add-architecture i386 || true
    $SUDO apt-get update
    $SUDO apt-get install -y \
        build-essential pkg-config curl ca-certificates \
        libxkbcommon-dev libxkbcommon-x11-0 libwayland-dev libgl1 libegl1 \
        xdg-desktop-portal \
        lib32gcc-s1 libc6-i386
elif [[ " $ID_ALL " == *" fedora "* ]]; then
    say "Fedora: ставлю пакеты через dnf"
    $SUDO dnf install -y \
        gcc make pkgconf-pkg-config curl \
        libxkbcommon-devel wayland-devel mesa-libGL mesa-libEGL \
        xdg-desktop-portal \
        glibc.i686 libgcc.i686
else
    warn "Неизвестный дистрибутив ($ID_ALL). Поставь вручную: компилятор C, pkg-config, curl,"
    warn "libxkbcommon, wayland, OpenGL/EGL, xdg-desktop-portal и 32-битные libc/libgcc (для SteamCMD)."
fi

# --- 2. Rust ----------------------------------------------------------------
export PATH="$HOME/.cargo/bin:$PATH"

if command -v rustup >/dev/null 2>&1; then
    if ! rustup show active-toolchain >/dev/null 2>&1; then
        say "Ставлю Rust stable (rustup default stable)"
        rustup default stable
    fi
elif ! command -v cargo >/dev/null 2>&1; then
    say "Ставлю Rust через rustup.rs"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
fi

if ! command -v cargo >/dev/null 2>&1; then
    warn "cargo так и не появился в PATH. Открой новый терминал и запусти ./scripts/install.sh ещё раз."
    exit 1
fi
say "Rust: $(rustc --version)"

if [[ $DEPS_ONLY -eq 1 ]]; then
    say "Зависимости готовы."
    exit 0
fi

# --- 3. Сборка --------------------------------------------------------------
say "Собираю мод-менеджер"
./build.sh

say "Готово. Запуск: ./run.sh --no-build"
echo "Если cargo не находится в новых терминалах, добавь ~/.cargo/bin в PATH (fish: fish_add_path ~/.cargo/bin)."
