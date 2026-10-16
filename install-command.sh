#!/usr/bin/env bash
# install-command.sh - put the Proton Drive CLI AppImage on your PATH
#
# Usage: ./install-command.sh [path/to/ProtonDrive-CLI-<ver>-<arch>.AppImage]
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"

APPIMAGE="${1:-}"
if [ -z "$APPIMAGE" ]; then
    APPIMAGE="$(ls -1t "$ROOT"/dist/*.AppImage 2>/dev/null | head -n1 || true)"
fi
[ -n "$APPIMAGE" ] && [ -f "$APPIMAGE" ] || { echo "no AppImage found; build first with ./build.sh" >&2; exit 1; }

mkdir -p "$BIN_DIR"
ln -sfn "$(readlink -f "$APPIMAGE")" "$BIN_DIR/proton-drive"

echo "Installed: $BIN_DIR/proton-drive -> $APPIMAGE"
echo
echo "Try it:"
echo "  proton-drive --version"
echo "  proton-drive --help"
echo "  proton-drive auth login"
echo
echo "NOTE: if your shell caches command locations, run 'rehash' (zsh) or 'hash -r' (bash)."
