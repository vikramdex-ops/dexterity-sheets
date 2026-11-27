#!/usr/bin/env bash
# Regenerate every app icon (Dexterity Sheets).
#
# The icon is original artwork drawn in code by packaging/dexterity/make_icon.py (Pillow). It
# writes assets/app-icon/dexterity-sheets.{svg,ico,icns}, the 1024 px and macOS 512 px PNGs and
# the Linux hicolor PNGs/SVG. The outputs are committed, so builds and packaging never need it.
#
#   packaging/icons.sh
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PY="$(command -v python3 || command -v python || true)"
[[ -n "$PY" ]] || { echo "error: python not found (needs Python 3 with Pillow)" >&2; exit 1; }
"$PY" "$ROOT/packaging/dexterity/make_icon.py"
echo "icons written to $ROOT/assets/app-icon"
