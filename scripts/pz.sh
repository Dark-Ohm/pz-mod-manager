#!/usr/bin/env bash
# Short commands for an environment's server and client. Environments (stable, lab, …) and their
# folders live in the manager's config: see `pz-mod-manager env list`.
#   ./scripts/pz.sh server lab                         start the server and wait until it is ready
#   ./scripts/pz.sh server lab --admin-password <pw>   first start (creates the ini)
#   ./scripts/pz.sh stop lab                           graceful stop (saves the world)
#   ./scripts/pz.sh client stable [--nosteam]          start the game client with that environment's data
#   ./scripts/pz.sh status lab                         server and client status
# Everything after the environment is passed to the manager command.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$ROOT/target/release/pz-mod-manager"
[[ -x "$BIN" ]] || "$ROOT/scripts/build.sh"

usage() { echo "usage: $0 <server|stop|client|status> <env> [options…]" >&2; exit 2; }
[[ $# -ge 2 ]] || usage
what="$1"; env="$2"; shift 2

case "$what" in
    server) exec "$BIN" server start "$env" --wait "$@" ;;
    stop)   exec "$BIN" server stop "$env" "$@" ;;
    client) exec "$BIN" client start "$env" "$@" ;;
    status) exec "$BIN" env show "$env" "$@" ;;
    *) usage ;;
esac
