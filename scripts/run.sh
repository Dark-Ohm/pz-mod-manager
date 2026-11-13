#!/usr/bin/env bash
# Собирает свежий бинарь и запускает его: ./scripts/run.sh
# Без пересборки: ./scripts/run.sh --no-build
set -euo pipefail
SCRIPTS="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPTS/.."

if [[ "${1:-}" == "--no-build" ]]; then
    shift
else
    "$SCRIPTS/build.sh"
fi

exec ./target/release/pz-mod-manager "$@"
