#!/usr/bin/env bash
# Сборка свежего релизного бинаря: ./scripts/build.sh
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"

if ! command -v cargo >/dev/null 2>&1; then
    echo "cargo не найден. Установи Rust: https://rustup.rs" >&2
    exit 1
fi

cargo build --release
echo "Готово: $(pwd)/target/release/pz-mod-manager"
