#!/usr/bin/env bash
#
# build.sh - Build Proton Drive CLI AppImages (x86_64 and aarch64)
#
# Downloads the official Proton Drive CLI binaries, verifies them against the
# SHA-512 checksums published by Proton, builds this project's Rust GUI
# (gui/) and Rust sync engine (crates/engine), wraps it all in an AppDir and
# produces portable AppImages with appimagetool. The result needs nothing
# installed on the host beyond a normal desktop.
#
# Usage:
#   ./build.sh                   # build every supported architecture
#   ./build.sh x86_64            # build x86_64
#   ./build.sh aarch64           # build aarch64
#   ./build.sh all               # build every supported architecture
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
DESKTOP_ID="io.github.zhafribs.protondrivecli.desktop"
METAINFO_ID="io.github.zhafribs.protondrivecli.appdata.xml"

# Build tool pin: a moving `continuous` tag is a supply-chain hole. This
# release is checksum-verified on every run, cached or fresh.
APPIMAGETOOL_VERSION="1.9.1"
APPIMAGETOOL_SHA256="ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0"
# appimagetool downloads the AppImage type2 runtime at package time. Pin it,
# or the first code a user executes is an unverified `continuous` asset.
TYPE2_RUNTIME_VERSION="20251108"
declare -A TYPE2_RUNTIME_SHA256=(
    [x86_64]="2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d"
    [aarch64]="00cbdfcf917cc6c0ff6d3347d59e0ca1f7f45a6df1a428a0d6d8a78664d87444"
)
CROSS_TARGET=""
# CI installs zig here; make a local cross-build find it without extra setup.
if ! command -v zig >/dev/null 2>&1 && [ -x "$HOME/zig/zig" ]; then
    PATH="$HOME/zig:$PATH"
    export PATH
fi
HOST_ARCH="$(uname -m)"
case "$HOST_ARCH" in
    x86_64|amd64) HOST_ARCH="x86_64" ;;
    *) echo "build.sh runs on x86_64 (appimagetool host tool); got $HOST_ARCH" >&2; exit 2 ;;
esac

# upstream platform -> AppImage architecture, output suffix
declare -A APPIMAGE_ARCH=(
    [linux-x64]="x86_64"
    [linux-arm64]="aarch64"
)

TARGETS=("linux-x64" "linux-arm64")
[ $# -gt 0 ] && case "$1" in
    all) TARGETS=("linux-x64" "linux-arm64") ;;
    x86_64|amd64) TARGETS=("linux-x64") ;;
    aarch64|arm64) TARGETS=("linux-arm64") ;;
    *) echo "unknown target: $1 (use x86_64, aarch64 or all)" >&2; exit 2 ;;
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
    for platform in "${TARGETS[@]}"; do
        key="$(sha512_key "$platform")"
        value="$(lock_get "$key")"
        [ -n "$value" ] || die "upstream.lock is missing $key - run scripts/check-upstream.sh"
        UPSTREAM_SHA512[$platform]="$value"
    done
}

load_upstream

fetch_tool() {
    mkdir -p "$TOOLS"
    local tool="$TOOLS/appimagetool-${HOST_ARCH}.AppImage"
    local url="https://github.com/AppImage/appimagetool/releases/download/${APPIMAGETOOL_VERSION}/appimagetool-${HOST_ARCH}.AppImage"
    local tmp="$tool.download"

    if [ -f "$tool" ] && echo "${APPIMAGETOOL_SHA256}  $tool" | sha256sum -c - >/dev/null 2>&1; then
        return
    fi
    # A cached `continuous` binary (or a corrupted one) must not survive the
    # pin. Remove it, fetch the pinned release, verify before chmod/move.
    rm -f "$tool" "$tmp"
    log "downloading appimagetool ${APPIMAGETOOL_VERSION}"
    wget -q -O "$tmp" "$url" || die "appimagetool download failed"
    echo "${APPIMAGETOOL_SHA256}  $tmp" | sha256sum -c - >/dev/null \
        || { rm -f "$tmp"; die "appimagetool checksum mismatch - refusing to use it"; }
    chmod +x "$tmp"
    mv "$tmp" "$tool"
}

fetch_runtime() {
    local arch="$1"
    mkdir -p "$TOOLS"
    local out="$TOOLS/runtime-${arch}"
    local url="https://github.com/AppImage/type2-runtime/releases/download/${TYPE2_RUNTIME_VERSION}/runtime-${arch}"
    local tmp="$out.download"
    local expected="${TYPE2_RUNTIME_SHA256[$arch]}"
    [ -n "$expected" ] || die "no pinned type2 runtime for ${arch}"
    if [ -f "$out" ] && echo "${expected}  $out" | sha256sum -c - >/dev/null 2>&1; then
        return
    fi
    rm -f "$out" "$tmp"
    log "downloading type2 runtime ${TYPE2_RUNTIME_VERSION} (${arch})"
    wget -q -O "$tmp" "$url" || die "type2 runtime download failed"
    echo "${expected}  $tmp" | sha256sum -c - >/dev/null \
        || { rm -f "$tmp"; die "type2 runtime checksum mismatch - refusing to embed it"; }
    mv "$tmp" "$out"
}

fetch_payload() {
    local platform="$1" out="$DOWNLOAD/proton-drive-${VERSION}-${platform}"
    mkdir -p "$DOWNLOAD"
    local expected="${UPSTREAM_SHA512[$platform]}"
    if [ -f "$out" ] && ! echo "$expected  $out" | sha512sum -c - >/dev/null 2>&1; then
        log "cached ${platform} payload does not match the lock; re-downloading"
        rm -f "$out"
    fi
    if [ ! -f "$out" ]; then
        log "downloading ${platform} binary from Proton"
        wget -q -O "$out" "${BASE_URL}/${platform}/proton-drive"
    fi
    log "verifying SHA-512 for ${platform}"
    echo "$expected  $out" | sha512sum -c - >/dev/null \
        || die "checksum mismatch for ${platform} - refusing to package a tampered binary"
}

# A built binary must not require a glibc newer than the advertised floor.
# Enforced whenever CROSS_TARGET is set (CI/release); a plain local build
# skips it because it inherits the host's glibc.
check_glibc_floor() {
    local bin="$1" max
    [ -n "$CROSS_TARGET" ] || return 0
    command -v readelf >/dev/null 2>&1 \
        || die "readelf not found - install binutils to verify the glibc floor"
    max="$(readelf --version-info "$bin" 2>/dev/null \
        | grep -oE 'GLIBC_[0-9.]+|GLIBC_ABI_DT_RELR' | sort -Vu | tail -1)"
    [ -n "$max" ] || die "could not read glibc symbols from $bin"
    if [ "$(printf '%s\n%s\n' "$max" "GLIBC_2.28" | sort -V | tail -1)" != "GLIBC_2.28" ]; then
        die "$bin requires $max, above the GLIBC_2.28 floor"
    fi
}

# Build the Rust GUI for one architecture. Release AppImages cross-compile
# with cargo-zigbuild so the binary's glibc floor is pinned to 2.28 (older
# than every desktop we support) instead of inheriting the build host's
# glibc; a plain cargo build is the fallback for local dev machines that
# match the target and have no zigbuild.
build_gui() {
    local arch="$1" bin_dir
    case "$arch" in
        x86_64) RUST_TARGET="x86_64-unknown-linux-gnu" ;;
        aarch64) RUST_TARGET="aarch64-unknown-linux-gnu" ;;
        *) die "no GUI target for ${arch}" ;;
    esac
    if [ "${REQUIRE_GLIBC_FLOOR:-0}" = "1" ] && [ -z "$CROSS_TARGET" ]; then
        die "REQUIRE_GLIBC_FLOOR is set but cargo-zigbuild/zig are unavailable"
    fi
    if [ -n "$CROSS_TARGET" ]; then
        log "building GUI (${CROSS_TARGET}, zigbuild)"
        ( cd "$ROOT/gui" && cargo zigbuild --release --target "$CROSS_TARGET" )
        for bin_dir in "$ROOT/gui/target/$RUST_TARGET/release" \
                       "$ROOT/gui/target/$CROSS_TARGET/release" \
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
    check_glibc_floor "$GUI_BIN"
}

# Build the tiny Rust AppImage entry point that replaces the old shell
# `src/AppRun`. It only dispatches to the GUI / engine / upstream CLI.
build_apprun() {
    local arch="$1" bin_dir
    case "$arch" in
        x86_64) RUST_TARGET="x86_64-unknown-linux-gnu" ;;
        aarch64) RUST_TARGET="aarch64-unknown-linux-gnu" ;;
        *) die "no AppRun target for ${arch}" ;;
    esac
    if [ "${REQUIRE_GLIBC_FLOOR:-0}" = "1" ] && [ -z "$CROSS_TARGET" ]; then
        die "REQUIRE_GLIBC_FLOOR is set but cargo-zigbuild/zig are unavailable"
    fi
    if [ -n "$CROSS_TARGET" ]; then
        log "building AppRun launcher (${CROSS_TARGET}, zigbuild)"
        ( cd "$ROOT" && cargo zigbuild --release -p proton-drive-apprun --target "$CROSS_TARGET" ) \
            || die "AppRun build failed"
        for bin_dir in "$ROOT/gui/target/$RUST_TARGET/release" \
                       "$ROOT/gui/target/$CROSS_TARGET/release" \
                       "$ROOT/gui/target/x86_64-unknown-linux-gnu/release"; do
            [ -x "$bin_dir/proton-drive-apprun" ] && break
        done
    elif [ "$(uname -m)" = "$arch" ]; then
        log "building AppRun launcher (native)"
        ( cd "$ROOT" && cargo build --release -p proton-drive-apprun ) \
            || die "AppRun build failed"
        bin_dir="$ROOT/gui/target/release"
    else
        die "cargo-zigbuild not found and this host is not ${arch} - cannot build the AppRun"
    fi
    [ -x "$bin_dir/proton-drive-apprun" ] \
        || die "AppRun binary missing at $bin_dir/proton-drive-apprun"
    APPRUN_BIN="$bin_dir/proton-drive-apprun"
    check_glibc_floor "$APPRUN_BIN"
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

    install -m 0755 "$APPRUN_BIN" "$appdir/AppRun"
    install -m 0755 "$payload"    "$appdir/usr/bin/proton-drive"
    check_glibc_floor "$payload"
    # GUI: Rust binary
    install -m 0755 "$GUI_BIN" "$appdir/usr/bin/proton-drive-gui"
    # Build and ship the Rust engine. This used to fall through to a native
    # build because `target` was local to the GUI/AppRun functions, which is
    # how a GLIBC_2.39 engine slipped into a "2.28" release.
    local engine_bin=""
    if [ -n "$CROSS_TARGET" ]; then
        ( cd "$ROOT" && cargo zigbuild --release -p proton-engine --target "$CROSS_TARGET" ) \
            || die "proton-engine cross-build failed"
        for ebin in "$ROOT/gui/target/$RUST_TARGET/release/proton-engine" \
                    "$ROOT/gui/target/$CROSS_TARGET/release/proton-engine" \
                    "$ROOT/gui/target/x86_64-unknown-linux-gnu/release/proton-engine" \
                    "$ROOT/target/$RUST_TARGET/release/proton-engine" \
                    "$ROOT/target/$CROSS_TARGET/release/proton-engine" \
                    "$ROOT/target/x86_64-unknown-linux-gnu/release/proton-engine"; do
            [ -x "$ebin" ] && { engine_bin="$ebin"; break; }
        done
    else
        ( cd "$ROOT" && cargo build --release -p proton-engine ) \
            || die "proton-engine build failed"
        for ebin in "$ROOT/gui/target/release/proton-engine" \
                    "$ROOT/target/release/proton-engine"; do
            [ -x "$ebin" ] && { engine_bin="$ebin"; break; }
        done
    fi
    [ -n "$engine_bin" ] || die "proton-engine binary missing after the build"
    install -m 0755 "$engine_bin" "$appdir/usr/bin/proton-engine"
    check_glibc_floor "$appdir/usr/bin/proton-engine"
    # The app icon
    install -m 0644 "$SRC/protondrive-icon.svg" \
        "$appdir/usr/lib/proton-sync/protondrive-icon.svg"

    # Fill the versions at build time so AppStream, the desktop entry and the
    # binary can never disagree.
    local release_date
    if [ -n "${SOURCE_DATE_EPOCH:-}" ]; then
        release_date="$(date -u -d "@${SOURCE_DATE_EPOCH}" +%F)"
    else
        release_date="$(date -u +%F)"
    fi
    sed "s/@GUI_VERSION@/${GUI_VERSION}/g" "$SRC/$DESKTOP_ID" > "$appdir/$DESKTOP_ID"
    install -m 0644 "$appdir/$DESKTOP_ID" "$appdir/usr/share/applications/$DESKTOP_ID"
    sed "s/@GUI_VERSION@/${GUI_VERSION}/g; s/@RELEASE_DATE@/${release_date}/g" \
        "$SRC/$METAINFO_ID" > "$appdir/usr/share/metainfo/$METAINFO_ID"
    # MIT requires the upstream copyright + permission notice to travel with
    # every copy of the Proton payload, so it ships inside the AppImage too.
    install -m 0644 "$SRC/licenses/Proton-Drive-SDK-MIT.txt" \
        "$appdir/usr/share/licenses/proton-drive/LICENSE"
    # This app is Rust-only: no CPython, no Python engine, no jeepney. The
    # aggregated texts of the GUI's crate graph (generated from Cargo.lock by
    # scripts/gen-licenses.sh) travel next to it.
    [ -f "$SRC/licenses/proton-drive-gui-THIRD-PARTY.txt" ] \
        || die "missing $SRC/licenses/proton-drive-gui-THIRD-PARTY.txt - run scripts/gen-licenses.sh"
    install -m 0644 "$SRC/licenses/proton-drive-gui-THIRD-PARTY.txt" \
        "$appdir/usr/share/licenses/proton-drive-gui-THIRD-PARTY.txt"

    install -m 0644 "$BUILD/icon-256.png" "$appdir/proton-drive.png"
    install -m 0644 "$BUILD/icon-256.png" "$appdir/usr/share/icons/hicolor/256x256/apps/proton-drive.png"
    install -m 0644 "$BUILD/icon-512.png" "$appdir/usr/share/icons/hicolor/512x512/apps/proton-drive.png"
    ln -sfn proton-drive.png "$appdir/.DirIcon"
}

# Assemble the AppDir. This app is Rust-only: the upstream Proton CLI is Rust,
# this project's GUI and sync engine are Rust, and the AppRun entry point is a
# compiled Rust launcher. There is no Python runtime to bundle.

validate() {
    local appdir="$1"
    if command -v desktop-file-validate >/dev/null; then
        desktop-file-validate "$appdir/$DESKTOP_ID"
    fi
    if command -v appstreamcli >/dev/null; then
        # --no-net keeps an unattended release independent of GitHub/Proton
        # reachability; --strict still fails on real metadata problems.
        if ! appstreamcli validate --no-net --strict \
            "$appdir/usr/share/metainfo/$METAINFO_ID" \
            >"$BUILD/appstream-validate.log" 2>&1; then
            cat "$BUILD/appstream-validate.log" >&2
            die "AppStream metadata validation failed"
        fi
        grep -E '^(  [EWI]:|✘|✔)' "$BUILD/appstream-validate.log" || echo "AppStream: clean"
    fi
}

build_one() {
    local platform="$1"
    local arch="${APPIMAGE_ARCH[$platform]}"
    local appdir="$BUILD/${APPIMAGE_NAME}.AppDir"
    local out="$DIST/${APPIMAGE_NAME}-${VERSION}-GUI-${GUI_VERSION}-${arch}.AppImage"

    fetch_payload "$platform"
    case "$arch" in
        x86_64) RUST_TARGET="x86_64-unknown-linux-gnu" ;;
        aarch64) RUST_TARGET="aarch64-unknown-linux-gnu" ;;
        *) die "no Rust target for ${arch}" ;;
    esac
    if [ "${REQUIRE_GLIBC_FLOOR:-0}" = "1" ]; then
        command -v cargo-zigbuild >/dev/null 2>&1 && command -v zig >/dev/null 2>&1 \
            || die "REQUIRE_GLIBC_FLOOR is set but cargo-zigbuild/zig are unavailable"
        rustup target list --installed | grep -qx "$RUST_TARGET" \
            || die "rustup target add $RUST_TARGET"
        CROSS_TARGET="${RUST_TARGET}.2.28"
    elif command -v cargo-zigbuild >/dev/null 2>&1 && command -v zig >/dev/null 2>&1; then
        rustup target list --installed | grep -qx "$RUST_TARGET" \
            || die "rustup target add $RUST_TARGET"
        CROSS_TARGET="${RUST_TARGET}.2.28"
    else
        CROSS_TARGET=""
        log "warning: cargo-zigbuild/zig not found - this local build is NOT glibc-pinned"
    fi
    build_gui "$arch"
    build_apprun "$arch"
    make_appdir "$platform" "$appdir"
    validate "$appdir"

    mkdir -p "$DIST"
    log "building ${arch} AppImage"
    rm -f "$out"

    fetch_runtime "$arch"
    local -a tool_args=( --no-appstream --runtime-file "$TOOLS/runtime-${arch}" "$appdir" "$out" )

    local tool_log="$BUILD/appimagetool-${arch}.log"
    ARCH="$arch" APPIMAGE_EXTRACT_AND_RUN=1 "$TOOLS/appimagetool-${HOST_ARCH}.AppImage" "${tool_args[@]}" \
        >"$tool_log" 2>&1 || { cat "$tool_log" >&2; die "appimagetool failed for ${arch}"; }
    grep -E 'AppStream upstream metadata|WARNING|Error|Success' "$tool_log" || true

    chmod +x "$out"
    log "built $out ($(du -h "$out" | cut -f1))"
}

log "Proton Drive CLI ${VERSION} -> AppImages: ${TARGETS[*]}"
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
