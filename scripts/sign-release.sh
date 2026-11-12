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

# Regenerate the manifest over exactly this release's artifacts. Old AppImages
# kept in a developer's dist/ must not leak into the signed list.
VERSION="$(awk -F= '$1 == "VERSION" { sub(/^[^=]*=/, ""); print; exit }' "$ROOT/upstream.lock")"
GUI_VERSION="$(sed -n 's/^pub const GUI_VERSION: &str = "\(.*\)";$/\1/p' "$ROOT/gui/src/main.rs" | head -1)"
[ -n "$VERSION" ] && [ -n "$GUI_VERSION" ] || die "missing VERSION or GUI_VERSION"
artifacts=(
    "$ROOT/dist/ProtonDrive-CLI-${VERSION}-GUI-${GUI_VERSION}-x86_64.AppImage"
    "$ROOT/dist/ProtonDrive-CLI-${VERSION}-GUI-${GUI_VERSION}-aarch64.AppImage"
    "$ROOT/dist/proton-drive_${VERSION}+gui${GUI_VERSION}_amd64.deb"
    "$ROOT/dist/proton-drive_${VERSION}+gui${GUI_VERSION}_arm64.deb"
)
( cd "$ROOT/dist" && for f in "${artifacts[@]}"; do
    [ -f "$f" ] && sha256sum "$(basename "$f")"
done > SHA256SUMS )
[ -s "$MANIFEST" ] || die "no release artifacts found - run ./build.sh and scripts/build-deb.sh first"

openssl pkeyutl -sign -rawin -inkey "$key_file" -in "$MANIFEST" -out "$SIGNATURE"
openssl pkeyutl -verify -pubin -inkey "$VERIFY_KEY" -rawin -in "$MANIFEST" \
    -sigfile "$SIGNATURE" >/dev/null 2>&1 \
    || { rm -f "$SIGNATURE"; die "signature did not verify against $VERIFY_KEY"; }

# Release revocation feed: the updater refuses a revoked tag. The file is
# versioned in the repo and copied beside the manifest for the release assets.
POLICY_SRC="$ROOT/revocations.json"
POLICY="$ROOT/dist/revocations.json"
if [ -f "$POLICY_SRC" ]; then
    cp "$POLICY_SRC" "$POLICY"
    openssl pkeyutl -sign -rawin -inkey "$key_file" -in "$POLICY" -out "$POLICY.sig"
    openssl pkeyutl -verify -pubin -inkey "$VERIFY_KEY" -rawin -in "$POLICY" \
        -sigfile "$POLICY.sig" >/dev/null 2>&1 \
        || { rm -f "$POLICY.sig"; die "revocation feed signature did not verify"; }
    echo "sign-release: signed revocations.json"
fi

echo "sign-release: signed $(basename "$MANIFEST")"
