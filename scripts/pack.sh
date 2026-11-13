#!/usr/bin/env bash
# Archive of the manager's sources to hand to someone else (or to another agent):
#   ./scripts/pack.sh                 -> ../dist/pz-mod-manager-<version>-<date>.zip
#   ./scripts/pack.sh /path/out.zip
# Leaves out build output (target/), git metadata and ini/backup leftovers. The recipient builds with
# scripts/install.sh (Linux) or scripts\install.bat (Windows); AGENTS.md explains the first run.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VER="$(sed -n 's/^version *= *"\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"
OUT="${1:-$(dirname "$ROOT")/dist/pz-mod-manager-${VER}-$(date +%Y%m%d).zip}"
mkdir -p "$(dirname "$OUT")"
OUT="$(cd "$(dirname "$OUT")" && pwd)/$(basename "$OUT")"
rm -f "$OUT"

if ! command -v python3 >/dev/null 2>&1; then
    echo "python3 not found: falling back to tar.gz" >&2
    OUT="${OUT%.zip}.tar.gz"
    tar -C "$(dirname "$ROOT")" --exclude='target' --exclude='.git' --exclude='*.bak-*' -czf "$OUT" "$(basename "$ROOT")"
    echo "$OUT"
    exit 0
fi

# zip with unix permissions kept (scripts stay executable after unzip on Linux)
python3 -I - "$ROOT" "$OUT" <<'PY'
import os, sys, zipfile, fnmatch
root, out = sys.argv[1], sys.argv[2]
top = os.path.basename(root)
skip_dirs = {"target", ".git", "__pycache__"}
skip_files = ["*.bak-*", ".DS_Store", "*.swp", "*~"]
n = 0
with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
    for d, dirs, files in os.walk(root):
        dirs[:] = sorted(x for x in dirs if x not in skip_dirs)
        for f in sorted(files):
            if any(fnmatch.fnmatch(f, p) for p in skip_files):
                continue
            path = os.path.join(d, f)
            if os.path.islink(path):
                continue
            arc = os.path.join(top, os.path.relpath(path, root))
            zi = zipfile.ZipInfo.from_file(path, arc)
            zi.compress_type = zipfile.ZIP_DEFLATED
            with open(path, "rb") as fh:
                z.writestr(zi, fh.read())
            n += 1
print(f"{out} ({n} files, {os.path.getsize(out) // 1024} KB)")
PY
