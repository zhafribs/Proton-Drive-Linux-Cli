#!/usr/bin/env bash
#
# build.sh - Build Proton Drive CLI AppImages (x86_64 + aarch64)
#
# Downloads the official Proton Drive CLI binaries, verifies them against the
# SHA-512 checksums published by Proton, wraps them in an AppDir and produces
# portable AppImages with appimagetool.
#
# Usage:
#   ./build.sh                   # build both architectures
#   ./build.sh x86_64            # build one architecture
#   ./scripts/check-upstream.sh  # refresh upstream.lock first (or let CI do it)
#
# The upstream version and the SHA-512 checksums Proton publishes live in
# upstream.lock. Never edit that file by hand - scripts/check-upstream.sh
# parses proton.me and rewrites it, so verification against Proton's own
# published checksums stays automatic as new releases appear.
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC="$ROOT/src"
DOWNLOAD="$ROOT/download"
TOOLS="$ROOT/tools"
BUILD="$ROOT/build"
DIST="$ROOT/dist"
LOCK="$ROOT/upstream.lock"
APPIMAGE_NAME="ProtonDrive-CLI"
# AppStream/desktop ids: appimagetool requires the metainfo file to be named
# "<desktop-basename>.appdata.xml", and appstreamcli requires that basename to
# equal the component id - so the desktop file, metainfo file and component id
# all share this value.
DESKTOP_ID="io.github.zhafrisbs.ProtonDriveCLI.desktop"
METAINFO_ID="io.github.zhafrisbs.ProtonDriveCLI.appdata.xml"

# upstream platform -> AppImage architecture, output suffix
declare -A APPIMAGE_ARCH=(
    [linux-x64]="x86_64"
    [linux-arm64]="aarch64"
)

TARGETS=("linux-x64" "linux-arm64")
[ $# -gt 0 ] && case "$1" in
    x86_64|amd64) TARGETS=("linux-x64") ;;
    aarch64|arm64) TARGETS=("linux-arm64") ;;
    *) echo "unknown target: $1 (use x86_64 or aarch64)" >&2; exit 2 ;;
esac

log() { printf '\033[1;35m==>\033[0m %s\n' "$*"; }
die() { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

# lock_get KEY  -> value of KEY=VALUE in upstream.lock ('' if absent)
lock_get() {
    [ -f "$LOCK" ] || return 0
    awk -F= -v key="$1" '$1 == key { sub(/^[^=]*=/, ""); print; exit }' "$LOCK"
}

# SHA512 key for a platform id: linux-x64 -> SHA512_LINUX_X64
sha512_key() {
    printf 'SHA512_%s' "$1" | tr '[:lower:]-' '[:upper:]_'
}

# Load VERSION / checksums / URLs from upstream.lock into the globals used below.
load_upstream() {
    [ -f "$LOCK" ] || die "upstream.lock not found - run scripts/check-upstream.sh"
    VERSION="$(lock_get VERSION)"
    RELEASE_DATE="$(lock_get RELEASE_DATE)"
    PAGE_URL="$(lock_get PAGE_URL)"
    DOWNLOAD_BASE="$(lock_get DOWNLOAD_BASE)"
    [ -n "$VERSION" ]           || die "upstream.lock is missing VERSION - run scripts/check-upstream.sh"
    [ -n "$DOWNLOAD_BASE" ]     || die "upstream.lock is missing DOWNLOAD_BASE - run scripts/check-upstream.sh"
    [ -n "$PAGE_URL" ]          || PAGE_URL="https://proton.me/download/drive/cli/index.html"
    [ -n "$RELEASE_DATE" ]      || RELEASE_DATE="unknown"
    BASE_URL="${DOWNLOAD_BASE}/${VERSION}"

    declare -gA UPSTREAM_SHA512=()
    local platform key value
    for platform in "${!APPIMAGE_ARCH[@]}"; do
        key="$(sha512_key "$platform")"
        value="$(lock_get "$key")"
        [ -n "$value" ] || die "upstream.lock is missing $key - run scripts/check-upstream.sh"
        UPSTREAM_SHA512[$platform]="$value"
    done
}

load_upstream

fetch_tool() {
    mkdir -p "$TOOLS"
    if [ ! -x "$TOOLS/appimagetool-x86_64.AppImage" ]; then
        log "downloading appimagetool"
        wget -q -O "$TOOLS/appimagetool-x86_64.AppImage" \
            "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage"
        chmod +x "$TOOLS/appimagetool-x86_64.AppImage"
    fi
    if [ ! -s "$TOOLS/runtime-aarch64" ]; then
        log "downloading aarch64 AppImage runtime"
        wget -q -O "$TOOLS/runtime-aarch64" \
            "https://github.com/AppImage/AppImageKit/releases/download/continuous/runtime-aarch64"
    fi
}

fetch_payload() {
    local platform="$1" out="$DOWNLOAD/proton-drive-${VERSION}-${platform}"
    mkdir -p "$DOWNLOAD"
    if [ ! -f "$out" ]; then
        log "downloading ${platform} binary from Proton"
        wget -q -O "$out" "${BASE_URL}/${platform}/proton-drive"
    fi
    log "verifying SHA-512 for ${platform}"
    echo "${UPSTREAM_SHA512[$platform]}  $out" | sha512sum -c - >/dev/null \
        || die "checksum mismatch for ${platform} - refusing to package a tampered binary"
}

render_icon() {
    # original cloud-upload glyph (src/protondrive-icon.svg), Proton-purple
    # gradient baked into the SVG - rendered straight to PNG for the AppDir
    [ -f "$SRC/protondrive-icon.svg" ] || die "missing $SRC/protondrive-icon.svg"
    command -v rsvg-convert >/dev/null || die "rsvg-convert not found (pacman -S librsvg / apt install librsvg2-bin / dnf install librsvg2-tools)"
    rsvg-convert -w 256 -h 256 "$SRC/protondrive-icon.svg" -o "$BUILD/icon-256.png"
    rsvg-convert -w 512 -h 512 "$SRC/protondrive-icon.svg" -o "$BUILD/icon-512.png"
}

make_appdir() {
    local platform="$1" appdir="$2"
    local payload="$DOWNLOAD/proton-drive-${VERSION}-${platform}"

    rm -rf "$appdir"
    mkdir -p "$appdir/usr/bin" \
             "$appdir/usr/share/applications" \
             "$appdir/usr/share/icons/hicolor/256x256/apps" \
             "$appdir/usr/share/icons/hicolor/512x512/apps" \
             "$appdir/usr/share/metainfo" \
             "$appdir/usr/share/licenses/proton-drive"

    install -m 0755 "$SRC/AppRun" "$appdir/AppRun"
    install -m 0755 "$payload"    "$appdir/usr/bin/proton-drive"

    install -m 0644 "$SRC/$DESKTOP_ID" "$appdir/$DESKTOP_ID"
    install -m 0644 "$SRC/$DESKTOP_ID" "$appdir/usr/share/applications/$DESKTOP_ID"
    install -m 0644 "$SRC/$METAINFO_ID" "$appdir/usr/share/metainfo/$METAINFO_ID"
    # MIT requires the upstream copyright + permission notice to travel with
    # every copy of the Proton payload, so it ships inside the AppImage too.
    install -m 0644 "$SRC/licenses/Proton-Drive-SDK-MIT.txt" \
        "$appdir/usr/share/licenses/proton-drive/LICENSE"

    install -m 0644 "$BUILD/icon-256.png" "$appdir/proton-drive.png"
    install -m 0644 "$BUILD/icon-256.png" "$appdir/usr/share/icons/hicolor/256x256/apps/proton-drive.png"
    install -m 0644 "$BUILD/icon-512.png" "$appdir/usr/share/icons/hicolor/512x512/apps/proton-drive.png"
    ln -sfn proton-drive.png "$appdir/.DirIcon"
}

validate() {
    local appdir="$1"
    if command -v desktop-file-validate >/dev/null; then
        desktop-file-validate "$appdir/$DESKTOP_ID"
    fi
    if command -v appstreamcli >/dev/null; then
        # validate-tree resolves the shipped binary and compares the metainfo
        # filename against the component id, so both AppStream checks apply here
        appstreamcli validate-tree "$appdir" >"$BUILD/appstream-validate.log" 2>&1 \
            || log "appstreamcli reported issues (see build/appstream-validate.log)"
        grep -E '^(  [EWI]:|✘|✔)' "$BUILD/appstream-validate.log" || echo "AppStream: clean"
    fi
}

build_one() {
    local platform="$1"
    local arch="${APPIMAGE_ARCH[$platform]}"
    local appdir="$BUILD/${APPIMAGE_NAME}.AppDir"
    local out="$DIST/${APPIMAGE_NAME}-${VERSION}-${arch}.AppImage"

    fetch_payload "$platform"
    make_appdir "$platform" "$appdir"
    validate "$appdir"

    mkdir -p "$DIST"
    log "building ${arch} AppImage"
    rm -f "$out"

    local -a tool_args=( "$appdir" "$out" )
    [ "$arch" = "aarch64" ] && tool_args=( --runtime-file "$TOOLS/runtime-aarch64" "${tool_args[@]}" )

    local tool_log="$BUILD/appimagetool-${arch}.log"
    ARCH="$arch" APPIMAGE_EXTRACT_AND_RUN=1 "$TOOLS/appimagetool-x86_64.AppImage" "${tool_args[@]}" \
        >"$tool_log" 2>&1 || { cat "$tool_log" >&2; die "appimagetool failed for ${arch}"; }
    grep -E 'AppStream upstream metadata|WARNING|Error|Success' "$tool_log" || true

    chmod +x "$out"
    log "built $out ($(du -h "$out" | cut -f1))"
}

log "Proton Drive CLI ${VERSION} -> AppImage"
fetch_tool
mkdir -p "$BUILD" "$DIST"
render_icon

for platform in "${TARGETS[@]}"; do
    build_one "$platform"
done

log "checksums"
( cd "$DIST" && sha256sum ./*.AppImage | tee SHA256SUMS )
