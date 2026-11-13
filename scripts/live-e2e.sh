#!/usr/bin/env bash
# Opt-in real-account E2E. It creates a temporary folder under /my-files,
# uploads one file, downloads it, compares bytes and trashes the folder.
#
# It is deliberately NOT run in CI: it needs a signed-in Proton account.
#   PROTON_SYNC_LIVE=1 ./scripts/live-e2e.sh
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CLI="${PROTON_DRIVE_BIN:-$ROOT/download/proton-drive-0.9.0-linux-x64}"
[ "${PROTON_SYNC_LIVE:-0}" = "1" ] || {
    echo "live-e2e: set PROTON_SYNC_LIVE=1 to allow real cloud writes" >&2
    exit 2
}
[ -x "$CLI" ] || { echo "live-e2e: CLI not executable: $CLI" >&2; exit 2; }
command -v cmp >/dev/null 2>&1 || { echo "live-e2e: cmp not found" >&2; exit 2; }

TMP="$(mktemp -d /tmp/proton-live-e2e.XXXXXX)"
REMOTE="/my-files/.proton-sync-e2e-$$"
NAME="probe-$$.txt"
cleanup() {
    "$CLI" fs trash "$REMOTE" >/dev/null 2>&1 || true
    rm -rf "$TMP"
}
trap cleanup EXIT

printf 'proton-sync live e2e %s\n' "$$" > "$TMP/$NAME"
"$CLI" fs create-folder /my-files ".proton-sync-e2e-$$" >/dev/null
"$CLI" fs upload "$TMP/$NAME" "$REMOTE" >/dev/null
listing="$("$CLI" --json fs list "$REMOTE")"
grep -q "$NAME" <<<"$listing" || {
    echo "live-e2e: uploaded file is not in the listing" >&2
    exit 1
}
"$CLI" fs download "$REMOTE/$NAME" "$TMP/out" >/dev/null
cmp "$TMP/$NAME" "$TMP/out/$NAME"
echo "live-e2e: OK — upload, list and download round-tripped $NAME"
