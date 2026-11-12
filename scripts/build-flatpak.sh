#!/usr/bin/env bash
# Build a single-file x86_64 Flatpak from the tested AppImage.
# Requires flatpak and org.flatpak.Builder (or flatpak-builder) plus the
# org.freedesktop.Platform/Sdk 24.08 runtimes.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST="$ROOT/dist"
BUILD="$ROOT/build/flatpak"
LOCK="$ROOT/upstream.lock"
APP_ID="io.github.zhafribs.protondrivecli"

die() { printf 'build-flatpak: %s\n' "$*" >&2; exit 1; }
lock_get() { awk -F= -v key="$1" '$1 == key { sub(/^[^=]*=/, ""); print; exit }' "$LOCK"; }

VERSION="$(lock_get VERSION)"
GUI_VERSION="$(sed -n 's/^pub const GUI_VERSION: &str = "\(.*\)";$/\1/p' "$ROOT/gui/src/main.rs" | head -1)"
[ -n "$VERSION" ] && [ -n "$GUI_VERSION" ] || die "missing VERSION or GUI_VERSION"
app="$DIST/ProtonDrive-CLI-${VERSION}-GUI-${GUI_VERSION}-x86_64.AppImage"
[ -f "$app" ] || die "missing $app - run ./build.sh x86_64 first"
command -v flatpak >/dev/null 2>&1 || die "flatpak not found"

rm -rf "$BUILD"
mkdir -p "$BUILD"
chmod +x "$app"
( cd "$BUILD" && APPIMAGE_EXTRACT_AND_RUN=1 "$app" --appimage-extract >/dev/null )
mv "$BUILD/squashfs-root" "$BUILD/appdir"
cp "$ROOT/packaging/flatpak/io.github.zhafribs.protondrivecli.yml" "$BUILD/manifest.yml"
cp "$ROOT/packaging/flatpak/proton-drive-wrapper" "$BUILD/"
cp "$ROOT/LICENSE" "$BUILD/"

# A user-only runtime install is normally invisible to flatpak-builder's
# host-side `flatpak info` calls. Point the system lookup at it for local
# builds; CI installs the runtime system-wide and keeps the default.
if ! flatpak info --system org.freedesktop.Sdk 25.08 >/dev/null 2>&1 \
   && flatpak info --user org.freedesktop.Sdk 25.08 >/dev/null 2>&1; then
    export FLATPAK_SYSTEM_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/flatpak"
fi

if flatpak run --command=flatpak-builder org.flatpak.Builder --version >/dev/null 2>&1; then
    # The Builder sandbox needs the runtime installation and the staging
    # directory; --filesystem=home exposes both.
    builder=(flatpak run --filesystem=home --command=flatpak-builder org.flatpak.Builder)
elif command -v flatpak-builder >/dev/null 2>&1; then
    builder=(flatpak-builder)
else
    die "flatpak-builder not found (install org.flatpak.Builder or flatpak-builder)"
fi

"${builder[@]}" --user --force-clean --repo="$BUILD/repo" \
    "$BUILD/build" "$BUILD/manifest.yml"

out="$DIST/proton-drive_${VERSION}+gui${GUI_VERSION}_x86_64.flatpak"
rm -f "$out"
flatpak build-bundle --runtime-repo=https://flathub.org/repo/flathub.flatpakrepo \
    "$BUILD/repo" "$out" "$APP_ID"
echo "built $out ($(du -h "$out" | cut -f1))"
