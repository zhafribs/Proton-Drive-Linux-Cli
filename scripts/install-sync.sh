#!/usr/bin/env bash
# Install or uninstall the Proton Drive folder sync as a systemd USER service.
# No sudo needed - the unit lives in ~/.config/systemd/user.
#
#   ./scripts/install-sync.sh            # install + start now + start at login
#   ./scripts/install-sync.sh uninstall  # stop + remove (folder and state kept)
#
# What it does:
#   * local folder  ~/ProtonDrive  mirrors  /my-files of your Proton Drive
#   * runs scripts/proton-sync.py --loop with the official CLI AppImage
#   * local edits upload within seconds (inotify), remote changes appear
#     within PROTON_SYNC_INTERVAL (default 300s)
#   * never deletes anything (see PROTON_SYNC_DELETES in proton-sync.py)
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
UNIT_DIR="$HOME/.config/systemd/user"
UNIT="$UNIT_DIR/proton-sync.service"
SYNC="$ROOT/scripts/proton-sync.py"
PY="$(command -v python3)"

case "${1:-install}" in
  install)
    BIN_PATH="${PROTON_DRIVE_BIN:-$HOME/.local/bin/proton-drive}"
    if [ ! -x "$BIN_PATH" ]; then
      # Self-heal: a rebuilt/renamed dist AppImage can leave the link dangling.
      latest="$(ls -1t "$ROOT"/dist/ProtonDrive-CLI-*-$(uname -m).AppImage 2>/dev/null | head -1)"
      if [ -n "$latest" ]; then
        mkdir -p "$(dirname "$BIN_PATH")"
        ln -sfn "$latest" "$BIN_PATH"
      fi
    fi
    if [ ! -x "$BIN_PATH" ]; then
      BIN_PATH="$(command -v proton-drive || true)"
    fi
    [ -n "$BIN_PATH" ] && [ -x "$BIN_PATH" ] || {
      echo "official CLI not found - put the release AppImage at ~/.local/bin/proton-drive" >&2
      exit 1
    }
    [ -f "$SYNC" ] || { echo "missing $SYNC" >&2; exit 1; }

    mkdir -p "$UNIT_DIR"
    CONF="$HOME/.config/proton-sync.conf"
    if [ ! -f "$CONF" ]; then
        cat > "$CONF" <<EOF
# proton-sync settings - editable by hand or with the Proton Drive GUI
# (scripts/install-gui.sh). Changes apply after: systemctl --user restart proton-sync
PROTON_SYNC_LOCAL=$HOME/ProtonDrive
PROTON_SYNC_REMOTE=/my-files
PROTON_SYNC_INTERVAL=300
PROTON_SYNC_DELETES=0
PROTON_SYNC_INOTIFY=1
PROTON_SYNC_FULL_EVERY=600
PROTON_SYNC_WORKERS=12
PROTON_SYNC_RETRIES=3
PROTON_SYNC_SUBFOLDERS=
PROTON_SYNC_EXCLUDE=
EOF
        echo "created default settings at $CONF"
    fi
    cat > "$UNIT" <<EOF
[Unit]
Description=Proton Drive folder sync (official CLI, two-way)

[Service]
Type=simple
Environment=PROTON_DRIVE_BIN=$BIN_PATH
EnvironmentFile=-$CONF
ExecStart=$PY "$SYNC" --loop
Restart=on-failure
RestartSec=15

[Install]
WantedBy=default.target
EOF

    systemctl --user daemon-reload
    systemctl --user enable --now proton-sync.service
    echo "installed and started."
    echo "  status:  systemctl --user status proton-sync.service"
    echo "  logs:    journalctl --user -u proton-sync -f"
    echo "  folder:  $HOME/ProtonDrive"
    ;;
  uninstall)
    systemctl --user disable --now proton-sync.service 2>/dev/null || true
    rm -f "$UNIT"
    systemctl --user daemon-reload
    echo "service removed (sync folder and state kept)."
    ;;
  *)
    echo "usage: $0 [install|uninstall]" >&2
    exit 2
    ;;
esac
