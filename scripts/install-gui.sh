#!/usr/bin/env bash
# Install the Proton Drive GUI app for source checkouts: the built Rust
# binary + app-menu entry + icon. No sudo needed, no host dependencies.
# (Release users need none of this - the AppImage already carries the app;
# this script is a dev convenience.)
#
#   ./scripts/install-gui.sh          # install
#   ./scripts/install-gui.sh remove   # remove menu entry + launcher
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_SRC="$ROOT/gui/target/release/proton-drive-gui"
BIN_DIR="$HOME/.local/bin"
APP_DIR="$HOME/.local/share/applications"
ICON_DIR="$HOME/.local/share/icons/hicolor/scalable/apps"
DESKTOP="$APP_DIR/proton-drive-gui.desktop"
SHIM="$BIN_DIR/proton-drive-gui"

case "${1:-install}" in
  install)
    [ -x "$BIN_SRC" ] || {
      echo "GUI binary not built yet. Build it first:" >&2
      echo "    cd gui && cargo build --release" >&2
      exit 1
    }

    mkdir -p "$BIN_DIR" "$APP_DIR" "$ICON_DIR"
    cp -f "$ROOT/src/protondrive-icon.svg" "$ICON_DIR/proton-drive-gui.svg"
    install -m 0755 "$BIN_SRC" "$SHIM"

    cat > "$DESKTOP" <<'EOF'
[Desktop Entry]
Type=Application
Version=1.0
Name=Proton Drive
GenericName=Cloud Storage Client
Comment=Sync and manage your Proton Drive (unofficial, official CLI backend)
Exec=proton-drive-gui
Icon=proton-drive-gui
Terminal=false
Categories=Network;FileTransfer;
Keywords=proton;drive;cloud;sync;storage;folder;
StartupNotify=true
EOF

    command -v update-desktop-database >/dev/null \
        && update-desktop-database "$APP_DIR" 2>/dev/null || true
    command -v gtk-update-icon-cache >/dev/null \
        && gtk-update-icon-cache -f -t "$HOME/.local/share/icons/hicolor" 2>/dev/null || true

    echo "installed."
    echo "  app menu : search for 'Proton Drive'"
    echo "  terminal : proton-drive-gui"
    ;;
  remove)
    rm -f "$SHIM" "$DESKTOP" "$ICON_DIR/proton-drive-gui.svg"
    command -v update-desktop-database >/dev/null \
        && update-desktop-database "$APP_DIR" 2>/dev/null || true
    echo "removed (sync service keeps running, folder untouched)."
    ;;
  *)
    echo "usage: $0 [install|remove]" >&2
    exit 2
    ;;
esac
