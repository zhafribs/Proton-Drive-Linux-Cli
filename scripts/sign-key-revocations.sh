#!/usr/bin/env bash
# Sign the successor-key revocation feed with the offline successor key.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FEED="$ROOT/security/key-revocations.json"
SIG="$FEED.sig"
PUB="$ROOT/src/update-signing-successor.pub"
KEY="${UPDATE_SUCCESSOR_KEY_FILE:-$HOME/Backup/proton-update-successor-key.pem}"
[ -f "$FEED" ] || { echo "missing $FEED" >&2; exit 1; }
[ -f "$PUB" ] || { echo "missing $PUB" >&2; exit 1; }
[ -f "$KEY" ] || { echo "missing successor key at $KEY" >&2; exit 1; }
openssl pkeyutl -sign -rawin -inkey "$KEY" -in "$FEED" -out "$SIG"
openssl pkeyutl -verify -pubin -inkey "$PUB" -rawin -in "$FEED" -sigfile "$SIG" >/dev/null
echo "sign-key-revocations: signed $(basename "$FEED")"
