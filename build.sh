#!/usr/bin/env bash
#
# build.sh - Build Proton Drive CLI AppImage (x86_64; aarch64 discontinued)
#
# Downloads the official Proton Drive CLI binaries, verifies them against the
# SHA-512 checksums published by Proton, builds this project's Rust GUI
# (gui/), adds the sync-engine script plus a portable CPython runtime
# (scripts/fetch-desktop-deps.sh), wraps it all in an AppDir and produces
# portable AppImages with appimagetool. The result needs nothing installed on
# the host beyond a normal desktop.
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
)

TARGETS=("linux-x64")
[ $# -gt 0 ] && case "$1" in
    x86_64|amd64) TARGETS=("linux-x64") ;;
    aarch64|arm64) echo "aarch64 builds were discontinued - releases are x86_64-only" >&2; exit 2 ;;
    *) echo "unknown target: $1 (only x86_64 is built)" >&2; exit 2 ;;
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
    # The GUI carries its own version; AppImages are named for BOTH versions.
    GUI_VERSION="$(sed -n 's/^pub const GUI_VERSION: &str = "\(.*\)";$/\1/p' "$ROOT/gui/src/main.rs" | head -1)"
    [ -n "$GUI_VERSION" ]       || die "gui/src/main.rs is missing GUI_VERSION"
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

# Build the Rust GUI for one architecture. Release AppImages cross-compile
# with cargo-zigbuild so the binary's glibc floor is pinned to 2.28 (older
# than every desktop we support) instead of inheriting the build host's
# glibc; a plain cargo build is the fallback for local dev machines that
# match the target and have no zigbuild.
build_gui() {
    local arch="$1" target bin_dir
    [ "$arch" = "x86_64" ] || die "no GUI target for ${arch} - releases are x86_64-only"
    target="x86_64-unknown-linux-gnu.2.28"
    if command -v cargo-zigbuild >/dev/null 2>&1; then
        log "building GUI (${target}, zigbuild)"
        ( cd "$ROOT/gui" && cargo zigbuild --release --target "$target" )
        # cargo-zigbuild strips the glibc suffix before invoking cargo, so the
        # artifacts usually land in the plain-triple directory; probe both.
        for bin_dir in "$ROOT/gui/target/$target/release" \
                       "$ROOT/gui/target/x86_64-unknown-linux-gnu/release"; do
            [ -x "$bin_dir/proton-drive-gui" ] && break
        done
    elif [ "$(uname -m)" = "$arch" ]; then
        log "building GUI (${arch}, native build - install cargo-zigbuild for the glibc-pinned one)"
        ( cd "$ROOT/gui" && cargo build --release )
        bin_dir="$ROOT/gui/target/release"
    else
        die "cargo-zigbuild not found and this host is not ${arch} - cannot build the GUI"
    fi
    [ -x "$bin_dir/proton-drive-gui" ] \
        || die "GUI binary missing at $bin_dir/proton-drive-gui"
    GUI_BIN="$bin_dir/proton-drive-gui"
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
             "$appdir/usr/lib/proton-sync" \
             "$appdir/usr/share/applications" \
             "$appdir/usr/share/icons/hicolor/256x256/apps" \
             "$appdir/usr/share/icons/hicolor/512x512/apps" \
             "$appdir/usr/share/metainfo" \
             "$appdir/usr/share/licenses/proton-drive"

    install -m 0755 "$SRC/AppRun" "$appdir/AppRun"
    install -m 0755 "$payload"    "$appdir/usr/bin/proton-drive"
    # GUI: Rust binary
    install -m 0755 "$GUI_BIN" "$appdir/usr/bin/proton-drive-gui"
    # Build and ship Rust engine only
    if command -v cargo >/dev/null 2>&1; then
        if command -v cargo-zigbuild >/dev/null 2>&1 && [ -n "${target:-}" ]; then
            ( cd "$ROOT" && cargo zigbuild --release -p proton-engine --target "$target" ) || die "proton-engine build failed"
        elif [ "$(uname -m)" = "$arch" ]; then
            ( cd "$ROOT" && cargo build --release -p proton-engine ) || die "proton-engine build failed"
        fi
        for ebin in "$ROOT/gui/target/${target:-}/release/proton-engine" "$ROOT/gui/target/x86_64-unknown-linux-gnu/release/proton-engine" "$ROOT/target/${target:-}/release/proton-engine" "$ROOT/target/x86_64-unknown-linux-gnu/release/proton-engine" "$ROOT/target/release/proton-engine"; do
            [ -x "$ebin" ] && { install -m 0755 "$ebin" "$appdir/usr/bin/proton-engine"; break; }
        done
    fi
    # The app icon
    install -m 0644 "$SRC/protondrive-icon.svg" \
        "$appdir/usr/lib/proton-sync/protondrive-icon.svg"

    install -m 0644 "$SRC/$DESKTOP_ID" "$appdir/$DESKTOP_ID"
    install -m 0644 "$SRC/$DESKTOP_ID" "$appdir/usr/share/applications/$DESKTOP_ID"
    install -m 0644 "$SRC/$METAINFO_ID" "$appdir/usr/share/metainfo/$METAINFO_ID"
    # MIT requires the upstream copyright + permission notice to travel with
    # every copy of the Proton payload, so it ships inside the AppImage too.
    install -m 0644 "$SRC/licenses/Proton-Drive-SDK-MIT.txt" \
        "$appdir/usr/share/licenses/proton-drive/LICENSE"
    # The bundled runtime is CPython (PSF-2.0), whose licence must accompany
    # the binary, and jeepney (MIT) for the engine's Secret Service reads -
    # jeepney's own licence travels inside its wheel's dist-info. The GUI
    # brings no copyleft of its own: Qt/PyQt6 are gone since 3.14.0. The
    # aggregated texts of the GUI's crate graph (generated from Cargo.lock
    # by scripts/gen-licenses.sh) travel next to it.
    install -m 0644 "$SRC/licenses/PSF-2.0.txt" "$appdir/usr/share/licenses/PSF-2.0.txt"
    [ -f "$SRC/licenses/proton-drive-gui-THIRD-PARTY.txt" ] \
        || die "missing $SRC/licenses/proton-drive-gui-THIRD-PARTY.txt - run scripts/gen-licenses.sh"
    install -m 0644 "$SRC/licenses/proton-drive-gui-THIRD-PARTY.txt" \
        "$appdir/usr/share/licenses/proton-drive-gui-THIRD-PARTY.txt"

    install -m 0644 "$BUILD/icon-256.png" "$appdir/proton-drive.png"
    install -m 0644 "$BUILD/icon-256.png" "$appdir/usr/share/icons/hicolor/256x256/apps/proton-drive.png"
    install -m 0644 "$BUILD/icon-512.png" "$appdir/usr/share/icons/hicolor/512x512/apps/proton-drive.png"
    ln -sfn proton-drive.png "$appdir/.DirIcon"
}

# Assemble the portable runtime - CPython plus the engine's wheels (jeepney,
# for the Secret Service session reads) - into the AppDir so the AppImage
# needs nothing installed on the host. The GUI has not been Python since
# 3.14.0 (it is a self-contained Rust binary). This only extracts and copies -
# no target-architecture binary is ever executed. Trimmed: Python pieces a
# sync client never imports (tcl/tk, idle, 2to3, pip). Licence texts are
# kept - they are the compliance obligation.
bundle_python() {
    local arch="$1" appdir="$2"
    local bdir="$ROOT/download/bundle/$arch"
    local py_file dest sp whl
    [ "${BUNDLE_PYTHON:-0}" = "0" ] && { BUNDLE_PY=0; return 0; }
    py_file="$(ls -1 "$bdir"/cpython-*-install_only.tar.gz 2>/dev/null | head -1 || true)"
    [ -n "$py_file" ] || die "portable CPython missing for ${arch} - run scripts/fetch-desktop-deps.sh ${arch}"
    ls -1 "$bdir"/wheels/*.whl >/dev/null 2>&1 || die "engine wheels missing for ${arch} - run scripts/fetch-desktop-deps.sh ${arch}"

    dest="$appdir/usr/lib/bundled-python"
    log "bundling portable CPython (${arch})"
    rm -rf "$dest"
    mkdir -p "$dest"
    tar -xzf "$py_file" -C "$dest" --strip-components=1
    sp="$dest/lib/python3.12/site-packages"
    mkdir -p "$sp"
    for whl in "$bdir"/wheels/*.whl; do
        python3 -m zipfile -e "$whl" "$sp"
    done
    # What a sync client never touches: tcl/tk (and its whole runtime - the
    # trimmed tkinter module leaves the shared libraries behind), pip, idle,
    # 2to3, pydoc, the dev configs and the test suite.
    rm -rf "$dest/lib/python3.12/idlelib" \
           "$dest/lib/python3.12/lib2to3" \
           "$dest/lib/python3.12/turtledemo" \
           "$dest/lib/python3.12/tkinter" \
           "$dest/lib/python3.12/test" \
           "$dest/lib/python3.12/ensurepip" \
           "$dest/lib/python3.12/site-packages/pip" \
           "$dest"/lib/python3.12/site-packages/pip-*.dist-info \
           "$dest"/lib/itcl* "$dest"/lib/tcl9* "$dest"/lib/tk9* \
           "$dest"/lib/thread3* "$dest"/lib/libtcl* "$dest"/lib/libtk* \
           "$dest"/lib/pkgconfig "$dest"/lib/python3.12/lib-dynload/_tkinter* \
           "$dest"/bin/2to3* "$dest"/bin/idle* "$dest"/bin/pip* \
           "$dest"/bin/pydoc* "$dest"/bin/python*-config
    # Strip debug symbols from libpython (114 MB unstripped; distro packages
    # strip theirs too). The interpreter BINARY is deliberately NOT stripped:
    # python-build-standalone's link layout puts .dynstr outside its segment,
    # and strip corrupts it - verified: the stripped binary dies with
    # "undefined symbol: , version" at first run.
    strip --strip-unneeded "$dest"/lib/libpython3.12.so.1.0 \
        || die "stripping the bundled libpython failed"
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
    local out="$DIST/${APPIMAGE_NAME}-${VERSION}-GUI-${GUI_VERSION}-${arch}.AppImage"

    fetch_payload "$platform"
    build_gui "$arch"
    make_appdir "$platform" "$appdir"
    "$ROOT/scripts/fetch-desktop-deps.sh" "$arch"
    BUNDLE_PYTHON=${BUNDLE_PYTHON:-0} bundle_python "$arch" "$appdir"
    validate "$appdir"

    mkdir -p "$DIST"
    log "building ${arch} AppImage"
    rm -f "$out"

    local -a tool_args=( "$appdir" "$out" )

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
# Plain file names: the in-app updater (PyQt <= 3.13.2 as well as the Rust
# GUI) looks the file name up exactly, and sha256sum prints a "./" prefix
# for path arguments - a prefix no shipped updater has ever accepted.
( cd "$DIST" && sha256sum *.AppImage | tee SHA256SUMS )
