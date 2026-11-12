#!/usr/bin/env bash
# Render every page at 50/100/200% through egui_kittest, then refresh the
# committed screenshot matrix. AccessKit checks each input widget's name and
# the keyboard test Tab-focuses the Settings nav row and activates it.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
export PROTON_UI_SCREENSHOTS="${PROTON_UI_SCREENSHOTS:-$ROOT/docs/audit/m8}"
cargo test -p proton-drive-gui ui::audit -- --nocapture
echo "ui-check: pages, dialogs, keyboard and screenshots passed"
