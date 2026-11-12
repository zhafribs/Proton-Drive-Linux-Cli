#!/usr/bin/env bash
# Install the built Flatpak bundle, prove it starts, then remove it.
# Usage: scripts/check-flatpak.sh [bundle]
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="$(awk -F= '$1 == "VERSION" { sub(/^[^=]*=/, ""); print; exit }' "$ROOT/upstream.lock")"
GUI="$(sed -n 's/^pub const GUI_VERSION: &str = "\(.*\)";$/\1/p' "$ROOT/gui/src/main.rs" | head -1)"
BUNDLE="${1:-$ROOT/dist/proton-drive_${VERSION}+gui${GUI}_x86_64.flatpak}"
APP="io.github.zhafribs.protondrivecli"
[ -f "$BUNDLE" ] || { echo "check-flatpak: missing $BUNDLE" >&2; exit 1; }
command -v flatpak >/dev/null 2>&1 || { echo "check-flatpak: flatpak not found" >&2; exit 1; }
flatpak install --user -y --noninteractive --reinstall "$BUNDLE" >/dev/null
smoke="$(flatpak run --user "$APP" --smoke 2>&1)" || {
    flatpak uninstall --user -y "$APP" >/dev/null 2>&1 || true
    echo "check-flatpak: smoke failed: $smoke" >&2
    exit 1
}
grep -q "gui ${GUI}" <<<"$smoke" || { echo "check-flatpak: wrong version: $smoke" >&2; exit 1; }
flatpak uninstall --user -y "$APP" >/dev/null
echo "check-flatpak: OK — $smoke"
