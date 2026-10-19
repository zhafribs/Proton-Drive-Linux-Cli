#!/usr/bin/env bash
#
# fetch-desktop-deps.sh - download the portable desktop stack that gets
# bundled into the AppImage, so a fresh computer needs NOTHING installed:
# a standalone CPython plus the PyQt6/Qt6 wheels, for either architecture.
#
#   ./scripts/fetch-desktop-deps.sh x86_64|aarch64
#
# This only downloads and checksums - nothing is executed - so an x86_64
# machine can prepare the aarch64 bundle too. Files land in
# download/bundle/<arch>/ (kept by the CI cache).
#
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
ARCH="${1:?usage: fetch-desktop-deps.sh x86_64|aarch64}"
OUT="$ROOT/download/bundle/$ARCH"
mkdir -p "$OUT"

# Portable CPython from the python-build-standalone project (PSF-2.0
# license, redistributable). Version and checksum are pinned together:
# after bumping PY_RELEASE, take the new hashes from that release's
# SHA256SUMS asset.
PY_RELEASE="20261013"
PY_VERSION="3.12.15+${PY_RELEASE}"

# PyQt6 trio - pinned to a matching set (see PyQt6's own Requires-Dist:
# PyQt6 6.11 wants PyQt6-Qt6 6.11 and PyQt6-sip >=13.8,<14). The wheels
# carry the whole Qt6 runtime; the platform tags below decide the oldest
# glibc each architecture supports.
PYQT_VERSION="6.11.0"
SIP_VERSION="13.13.0"

case "$ARCH" in
    x86_64)
        PY_TAG="x86_64-unknown-linux-gnu"
        PY_SHA256="5d546fc004fc2435fa142eb8f42ed8de5eb4725963dc22f7e6671014a80eb560"
        PIP_PLATFORMS=(manylinux_2_34_x86_64 manylinux_2_28_x86_64 manylinux2014_x86_64)
        ;;
    aarch64)
        PY_TAG="aarch64-unknown-linux-gnu"
        PY_SHA256="34a43010cc05982d8b5debbad7d09f7b1200117ebf018f7acfd656895e408205"
        PIP_PLATFORMS=(manylinux_2_39_aarch64 manylinux_2_28_aarch64 manylinux2014_aarch64)
        ;;
    *)
        echo "unknown arch: $ARCH (use x86_64 or aarch64)" >&2
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

echo "==> downloading PyQt6 ${PYQT_VERSION} wheels (${ARCH})"
rm -rf "$OUT/wheels"
mkdir -p "$OUT/wheels"
plat_args=()
for p in "${PIP_PLATFORMS[@]}"; do
    plat_args+=(--platform "$p")
done
python3 -m pip download "PyQt6==${PYQT_VERSION}" "PyQt6-Qt6==${PYQT_VERSION}" \
    "PyQt6-sip==${SIP_VERSION}" \
    --only-binary=:all: --python-version 3.12 --implementation cp \
    "${plat_args[@]}" --no-deps -q -d "$OUT/wheels"
# jeepney (pure-python D-Bus client, py3-none-any): the sync engine's push
# listener reads the official CLI's session from the OS Secret Service with it
python3 -m pip download "jeepney==0.9.0" \
    --only-binary=:all: --no-deps -q -d "$OUT/wheels"

echo "==> desktop bundle ready: $OUT"
