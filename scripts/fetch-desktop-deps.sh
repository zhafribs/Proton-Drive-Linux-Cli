#!/usr/bin/env bash
#
# fetch-desktop-deps.sh - download the portable runtime that gets bundled
# into the AppImage, so a fresh computer needs NOTHING installed: a
# standalone CPython (the sync engine runs on it) plus jeepney (its Secret
# Service client). The GUI has not been Python since 3.14.0 - it is a
# self-contained Rust binary built by build.sh.
#
#   ./scripts/fetch-desktop-deps.sh x86_64
#
# This only downloads and checksums - nothing is executed. Files land in
# download/bundle/<arch>/ (kept by the CI cache). aarch64 was discontinued:
# releases are x86_64-only.
#
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
ARCH="${1:?usage: fetch-desktop-deps.sh x86_64}"
OUT="$ROOT/download/bundle/$ARCH"
mkdir -p "$OUT"

# Portable CPython from the python-build-standalone project (PSF-2.0
# license, redistributable). Version and checksum are pinned together:
# after bumping PY_RELEASE, take the new hashes from that release's
# SHA256SUMS asset.
PY_RELEASE="20261013"
PY_VERSION="3.12.15+${PY_RELEASE}"

case "$ARCH" in
    x86_64)
        PY_TAG="x86_64-unknown-linux-gnu"
        PY_SHA256="5d546fc004fc2435fa142eb8f42ed8de5eb4725963dc22f7e6671014a80eb560"
        ;;
    *)
        echo "unknown arch: $ARCH (only x86_64 is built; aarch64 was discontinued)" >&2
        exit 2
        ;;
esac

PY_FILE="cpython-${PY_VERSION}-${PY_TAG}-install_only.tar.gz"
PY_URL="https://github.com/astral-sh/python-build-standalone/releases/download/${PY_RELEASE}/${PY_FILE}"

if [ ! -f "$OUT/$PY_FILE" ] \
   || [ "$(sha256sum "$OUT/$PY_FILE" | cut -d' ' -f1)" != "$PY_SHA256" ]; then
    echo "==> downloading portable CPython ${PY_VERSION} (${ARCH})"
    wget -q -O "$OUT/$PY_FILE" "$PY_URL"
    echo "${PY_SHA256}  $OUT/$PY_FILE" | sha256sum -c - >/dev/null \
        || { echo "error: checksum mismatch for ${PY_FILE}" >&2; exit 1; }
fi

# jeepney (pure-python D-Bus client, py3-none-any): the sync engine's push
# listener reads the official CLI's session from the OS Secret Service with it
echo "==> downloading jeepney (arch-independent, pinned)"
rm -rf "$OUT/wheels"
mkdir -p "$OUT/wheels"
python3 -m pip download "jeepney==0.9.0" \
    --only-binary=:all: --no-deps -q -d "$OUT/wheels"

echo "==> desktop bundle ready: $OUT"
