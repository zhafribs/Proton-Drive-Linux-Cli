#!/usr/bin/env bash
# Install the Proton Drive GUI app: launcher + app-menu entry + icon.
# No sudo needed. Uses PyQt6 from the OFFICIAL distro repos (never AUR).
#
#   ./scripts/install-gui.sh          # install
#   ./scripts/install-gui.sh remove   # remove menu entry + launcher
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
GUI="$ROOT/scripts/proton-sync-gui.py"
BIN_DIR="$HOME/.local/bin"
APP_DIR="$HOME/.local/share/applications"
ICON_DIR="$HOME/.local/share/icons/hicolor/scalable/apps"
DESKTOP="$APP_DIR/proton-drive-gui.desktop"
SHIM="$BIN_DIR/proton-drive-gui"

case "${1:-install}" in
  install)
    [ -f "$GUI" ] || { echo "missing $GUI" >&2; exit 1; }
    python3 -c "import PyQt6.QtWidgets" 2>/dev/null || {
      echo "PyQt6 is not installed yet. Install it from the OFFICIAL repos:" >&2
      echo "    sudo pacman -S python-pyqt6        # Arch / CachyOS" >&2
      echo "    sudo apt install python3-pyqt6     # Debian / Ubuntu" >&2
      echo "    sudo dnf install python3-pyqt6     # Fedora" >&2
      exit 1
    }

    mkdir -p "$BIN_DIR" "$APP_DIR" "$ICON_DIR"
    cp -f "$ROOT/src/protondrive-icon.svg" "$ICON_DIR/proton-drive-gui.svg"

    cat > "$SHIM" <<EOF
#!/usr/bin/env bash
exec python3 "$GUI" "\$@"
EOF
    chmod +x "$SHIM"

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
