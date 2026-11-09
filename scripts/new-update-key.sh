#!/usr/bin/env bash
# Generate an offline Ed25519 successor key for the update manifest.
# This script does not publish anything: it prints the public key and the
# rotation steps. The private key must stay offline and backed up.
set -euo pipefail

KEY="${1:-$HOME/Backup/proton-update-successor-key.pem}"
if [ -e "$KEY" ]; then
    echo "refusing to overwrite $KEY" >&2
    exit 1
fi
mkdir -p "$(dirname "$KEY")"
umask 077
openssl genpkey -algorithm ED25519 -out "$KEY"
PUB="$(openssl pkey -in "$KEY" -pubout -outform DER | tail -c 32 | xxd -p -c 32)"
printf 'Private key: %s (0600, keep offline)\n' "$KEY"
printf 'Public key hex: %s\n\n' "$PUB"
cat <<'TEXT'
Next steps:
1. Back up the private key offline.
2. Add the public hex to UPDATE_PUBLIC_KEYS in gui/src/release.rs in a
   release signed by the current key.
3. Keep both keys accepted for the migration window.
4. Sign new releases with the successor key:
     UPDATE_SIGNING_KEY_FILE=/path/to/successor.pem ./scripts/sign-release.sh
5. Remove the old key from UPDATE_PUBLIC_KEYS and, when the migration window
   ends, add it to REVOKED_UPDATE_KEYS.
TEXT
