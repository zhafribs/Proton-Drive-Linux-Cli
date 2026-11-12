#!/usr/bin/env bash
# Build a .deb that installs the tested AppImage plus its launcher and metadata.
# Usage: scripts/build-deb.sh [x86_64|aarch64|all]
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST="$ROOT/dist"
BUILD="$ROOT/build"
LOCK="$ROOT/upstream.lock"
PKG="proton-drive"

die() { printf 'build-deb: %s\n' "$*" >&2; exit 1; }
lock_get() { awk -F= -v key="$1" '$1 == key { sub(/^[^=]*=/, ""); print; exit }' "$LOCK"; }

VERSION="$(lock_get VERSION)"
GUI_VERSION="$(sed -n 's/^pub const GUI_VERSION: &str = "\(.*\)";$/\1/p' "$ROOT/gui/src/main.rs" | head -1)"
[ -n "$VERSION" ] && [ -n "$GUI_VERSION" ] || die "missing VERSION or GUI_VERSION"

TARGETS=("x86_64" "aarch64")
if [ $# -gt 0 ]; then
    case "$1" in
        all) ;;
        x86_64|amd64) TARGETS=("x86_64") ;;
        aarch64|arm64) TARGETS=("aarch64") ;;
        *) die "unknown target: $1 (use x86_64, aarch64 or all)" ;;
    esac
fi

# Debian architecture and the package the AppImage runtime needs for FUSE.
deb_arch() { case "$1" in x86_64) echo amd64 ;; aarch64) echo arm64 ;; esac; }

for arch in "${TARGETS[@]}"; do
    app="$DIST/ProtonDrive-CLI-${VERSION}-GUI-${GUI_VERSION}-${arch}.AppImage"
    [ -f "$app" ] || die "missing $app - run ./build.sh ${arch} first"
    darch="$(deb_arch "$arch")"
    out="$DIST/${PKG}_${VERSION}+gui${GUI_VERSION}_${darch}.deb"
    work="$(mktemp -d /tmp/proton-deb.XXXXXX)"
    trap 'rm -rf "$work"' EXIT

    mkdir -p "$work/DEBIAN" "$work/usr/bin" "$work/usr/lib/proton-drive" \
             "$work/usr/share/applications" "$work/usr/share/metainfo" \
             "$work/usr/share/icons/hicolor/256x256/apps" \
             "$work/usr/share/icons/hicolor/512x512/apps" \
             "$work/usr/share/doc/$PKG"
    install -m 0755 "$app" "$work/usr/lib/proton-drive/ProtonDrive.AppImage"

    cat > "$work/usr/bin/proton-drive" <<'WRAPPER'
#!/bin/sh
exec /usr/lib/proton-drive/ProtonDrive.AppImage "$@"
WRAPPER
    chmod 0755 "$work/usr/bin/proton-drive"

    # Metadata and icons come from the same AppDir the AppImage was built from,
    # so the package can never describe a different GUI version.
    desktop="$BUILD/ProtonDrive-CLI.AppDir/usr/share/applications/io.github.zhafribs.protondrivecli.desktop"
    metainfo="$BUILD/ProtonDrive-CLI.AppDir/usr/share/metainfo/io.github.zhafribs.protondrivecli.appdata.xml"
    [ -f "$desktop" ] || die "missing generated desktop file - run ./build.sh first"
    [ -f "$metainfo" ] || die "missing generated metainfo - run ./build.sh first"
    install -m 0644 "$desktop" "$work/usr/share/applications/"
    install -m 0644 "$metainfo" "$work/usr/share/metainfo/"
    install -m 0644 "$BUILD/icon-256.png" "$work/usr/share/icons/hicolor/256x256/apps/proton-drive.png"
    install -m 0644 "$BUILD/icon-512.png" "$work/usr/share/icons/hicolor/512x512/apps/proton-drive.png"
    install -m 0644 "$ROOT/LICENSE" "$work/usr/share/doc/$PKG/copyright"

    # Debian policy metadata. md5sums is generated over the installed tree.
    ( cd "$work" && find usr -type f -print0 | sort -z | xargs -0 md5sum ) \
        > "$work/DEBIAN/md5sums"
    size="$(du -sk "$work" | cut -f1)"
    cat > "$work/DEBIAN/control" <<CONTROL
Package: $PKG
Version: ${VERSION}+gui${GUI_VERSION}
Architecture: $darch
Maintainer: Zhafri Syazwi <zhafribs@users.noreply.github.com>
Section: utils
Priority: optional
Installed-Size: $size
Depends: libfuse2 | libfuse2t64
Description: Unofficial Proton Drive desktop app
 Two-way folder sync, a cloud file manager, photos, recovery and in-app
 updates, built on Proton's official unmodified Drive CLI. This package
 installs the same AppImage that is published for this release.
CONTROL

    # Build the ar archive directly: no dpkg-deb dependency, and every member
    # timestamp is pinned so the .deb is reproducible too.
    epoch="${SOURCE_DATE_EPOCH:-$(git -C "$ROOT" log -1 --format=%ct 2>/dev/null || date +%s)}"
    ( cd "$work" && tar --sort=name --mtime="@${epoch}" --owner=0 --group=0 \
        --numeric-owner -cJf "$work/control.tar.xz" -C DEBIAN . )
    ( cd "$work" && tar --sort=name --mtime="@${epoch}" --owner=0 --group=0 \
        --numeric-owner -cJf "$work/data.tar.xz" ./usr )
    printf '2.0\n' > "$work/debian-binary"
    rm -f "$out"
    ( cd "$work" && ar rcD "$out" debian-binary control.tar.xz data.tar.xz )
    echo "built $out ($(du -h "$out" | cut -f1))"
    rm -rf "$work"
    trap - EXIT
done
