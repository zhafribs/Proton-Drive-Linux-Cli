#!/usr/bin/env bash
# Download a published release and verify its checksum, signatures, smoke and
# GLIBC floor. Usage: scripts/verify-release.sh [tag] (default: latest)
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TAG="${1:-latest}"
REPO="zhafribs/Proton-Drive-Linux-Cli"
PUBLIC="${VERIFY_RELEASE_KEY:-$ROOT/src/update-signing-key.pub}"
TMP="$(mktemp -d /tmp/proton-verify.XXXXXX)"
trap 'rm -rf "$TMP"' EXIT

gh release download "$TAG" -R "$REPO" -D "$TMP"
cd "$TMP"
sha256sum -c SHA256SUMS
openssl pkeyutl -verify -pubin -inkey "$PUBLIC" -rawin \
    -in SHA256SUMS -sigfile SHA256SUMS.sig >/dev/null
echo "signature: SHA256SUMS OK"
if [ -f revocations.json ]; then
    [ -f revocations.json.sig ] || { echo "revocations.json.sig missing" >&2; exit 1; }
    openssl pkeyutl -verify -pubin -inkey "$PUBLIC" -rawin \
        -in revocations.json -sigfile revocations.json.sig >/dev/null
    echo "signature: revocations.json OK"
fi
APP=$(find . -maxdepth 1 -name 'ProtonDrive-CLI-*-x86_64.AppImage' -print -quit)
[ -n "$APP" ] || { echo "AppImage asset missing" >&2; exit 1; }
chmod +x "$APP"
HOME="$TMP/smoke-home" XDG_CONFIG_HOME="$TMP/smoke-config" "$APP" --smoke
"$APP" --appimage-extract >/dev/null 2>&1
for f in squashfs-root/AppRun squashfs-root/usr/bin/proton-drive-gui squashfs-root/usr/bin/proton-engine; do
    floor="$(objdump -T "$f" 2>/dev/null | grep -o 'GLIBC_[0-9.]*' | sort -Vu | tail -1)"
    printf '%s: %s\n' "$f" "$floor"
    [ "$floor" = "GLIBC_2.28" ] || { echo "GLIBC floor is not 2.28: $f" >&2; exit 1; }
done
