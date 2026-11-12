#!/usr/bin/env bash
# Download a published release and verify checksums, signatures, smoke,
# GLIBC floor and the .deb metadata. Usage: scripts/verify-release.sh [tag]
# (default: latest)
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

glibc_floor() {
    local f="$1" floor
    floor="$(objdump -T "$f" 2>/dev/null | grep -o 'GLIBC_[0-9.]*' | sort -Vu | tail -1)"
    printf '%s: %s\n' "$f" "$floor"
    [ "$floor" = "GLIBC_2.28" ] || { echo "GLIBC floor is not 2.28: $f" >&2; exit 1; }
}

for app in ProtonDrive-CLI-*-x86_64.AppImage ProtonDrive-CLI-*-aarch64.AppImage; do
    [ -f "$app" ] || { echo "missing AppImage: $app" >&2; exit 1; }
    arch="$(file -b "$app")"
    case "$arch" in
        *ARM*aarch64*) want=aarch64 ;;
        *x86-64*) want=x86_64 ;;
        *) echo "unknown AppImage architecture: $app ($arch)" >&2; exit 1 ;;
    esac
    chmod +x "$app"
    if [ "$want" = "$(uname -m)" ]; then
        HOME="$TMP/smoke-home-$want" XDG_CONFIG_HOME="$TMP/smoke-config-$want" "$app" --smoke
        "$app" --appimage-extract >/dev/null 2>&1
        root="squashfs-root"
    else
        off="$(grep -abo 'hsqs' "$app" | head -1 | cut -d: -f1)"
        [ -n "$off" ] || { echo "no squashfs offset in $app" >&2; exit 1; }
        root="$TMP/$want-root"
        if command -v unsquashfs >/dev/null 2>&1; then
            unsquashfs -o "$off" -d "$root" "$app" >/dev/null
        elif command -v 7z >/dev/null 2>&1; then
            dd if="$app" bs=1 skip="$off" of="$TMP/$want.squashfs" status=none
            mkdir -p "$root"
            7z x -o"$root" "$TMP/$want.squashfs" >/dev/null
        else
            echo "$want: checksum OK; install squashfs-tools or 7z to inspect its contents" >&2
            continue
        fi
    fi
    for f in "$root/AppRun" "$root/usr/bin/proton-drive-gui" "$root/usr/bin/proton-engine"; do
        glibc_floor "$f"
    done
    rm -rf "$root"
done

for deb in proton-drive_*_amd64.deb proton-drive_*_arm64.deb; do
    [ -f "$deb" ] || { echo "missing deb: $deb" >&2; exit 1; }
    control="$(ar p "$deb" control.tar.xz | tar -xJOf - ./control)"
    darch="${deb##*_}"
    darch="${darch%.deb}"
    grep -qx "Architecture: ${darch}" <<<"$control" \
        || { echo "$deb architecture mismatch" >&2; exit 1; }
    data_list="$(ar p "$deb" data.tar.xz | tar -tJf -)"
    grep -qx './usr/lib/proton-drive/ProtonDrive.AppImage' <<<"$data_list" \
        || { echo "$deb does not install the AppImage" >&2; exit 1; }
    echo "deb: $deb OK"
done
