#!/usr/bin/env bash
#
# sign-release.sh - sign dist/SHA256SUMS with the project Ed25519 key.
#
# CI sets UPDATE_SIGNING_KEY (the PEM private key from the repository secret).
# A local release can set UPDATE_SIGNING_KEY_FILE to the offline key backup.
# The signature is written to dist/SHA256SUMS.sig and verified immediately.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MANIFEST="$ROOT/dist/SHA256SUMS"
SIGNATURE="$MANIFEST.sig"
PUBLIC="$ROOT/src/update-signing-key.pub"
VERIFY_KEY="${SIGN_RELEASE_VERIFY_KEY:-$PUBLIC}"

die() { printf 'sign-release: %s\n' "$*" >&2; exit 1; }

[ -f "$MANIFEST" ] || die "dist/SHA256SUMS missing - run ./build.sh first"
[ -f "$PUBLIC" ] || die "src/update-signing-key.pub missing"
[ -f "$VERIFY_KEY" ] || die "verification key not found at $VERIFY_KEY"
command -v openssl >/dev/null 2>&1 || die "openssl not found"

tmp_key=""
cleanup() { if [ -n "$tmp_key" ]; then rm -f "$tmp_key"; fi; }
trap cleanup EXIT

if [ -n "${UPDATE_SIGNING_KEY:-}" ]; then
    tmp_key="$(mktemp)"
    chmod 600 "$tmp_key"
    printf '%s\n' "$UPDATE_SIGNING_KEY" > "$tmp_key"
    key_file="$tmp_key"
elif [ -n "${UPDATE_SIGNING_KEY_FILE:-}" ]; then
    key_file="$UPDATE_SIGNING_KEY_FILE"
else
    die "set UPDATE_SIGNING_KEY (CI) or UPDATE_SIGNING_KEY_FILE (local)"
fi
[ -f "$key_file" ] || die "signing key not found at $key_file"

openssl pkeyutl -sign -rawin -inkey "$key_file" -in "$MANIFEST" -out "$SIGNATURE"
openssl pkeyutl -verify -pubin -inkey "$VERIFY_KEY" -rawin -in "$MANIFEST" \
    -sigfile "$SIGNATURE" >/dev/null 2>&1 \
    || { rm -f "$SIGNATURE"; die "signature did not verify against $VERIFY_KEY"; }

echo "sign-release: signed $(basename "$MANIFEST")"
