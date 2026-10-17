#!/usr/bin/env bash
#
# release-notes.sh - Render release-notes.md.tmpl for the current build.
#
# Usage: ./scripts/release-notes.sh [version]   (defaults to upstream.lock)
#
# Must run AFTER ./build.sh: the notes embed the SHA-256 of the artifacts that
# were actually built, so readers can verify what they download.
#
set -euo pipefail

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TEMPLATE="$ROOT/release-notes.md.tmpl"
LOCK="$ROOT/upstream.lock"
DIST="$ROOT/dist"

lock_get() { awk -F= -v key="$1" '$1 == key { sub(/^[^=]*=/, ""); print; exit }' "$LOCK"; }

VERSION="${1:-$(lock_get VERSION)}"
[ -n "$VERSION" ] || { echo "error: no version given and none in upstream.lock" >&2; exit 1; }

# The GUI version travels in the tag and in every asset name.
GUI_VERSION="$(sed -n 's/^GUI_VERSION = "\(.*\)"$/\1/p' "$ROOT/scripts/proton-sync-gui.py" | head -1)"
[ -n "$GUI_VERSION" ] || { echo "error: GUI_VERSION not found in proton-sync-gui.py" >&2; exit 1; }
[ -f "$ROOT/CHANGELOG.md" ] || { echo "error: CHANGELOG.md missing" >&2; exit 1; }

RELEASE_DATE="$(lock_get RELEASE_DATE)"
PAGE_URL="$(lock_get PAGE_URL)"
SHA512_X64="$(lock_get SHA512_LINUX_X64)"
SHA512_ARM64="$(lock_get SHA512_LINUX_ARM64)"

X86="$DIST/ProtonDrive-CLI-${VERSION}-GUI-${GUI_VERSION}-x86_64.AppImage"
ARM="$DIST/ProtonDrive-CLI-${VERSION}-GUI-${GUI_VERSION}-aarch64.AppImage"
for artifact in "$X86" "$ARM" "$DIST/SHA256SUMS"; do
    [ -f "$artifact" ] || { echo "error: missing $(basename "$artifact") - run ./build.sh first" >&2; exit 1; }
done

# Sanity: SHA256SUMS must describe exactly the artifacts we are about to announce.
( cd "$DIST" && sha256sum -c --ignore-missing SHA256SUMS >/dev/null ) \
    || { echo "error: dist/SHA256SUMS does not match the built artifacts" >&2; exit 1; }

REPO="$(git -C "$ROOT" remote get-url origin 2>/dev/null \
    | sed -E 's#^(git@github.com:|https://github.com/)##; s#\.git$##')"
REPO="${REPO:-zhafribs/Proton-Drive-Linux-Cli}"

sha256() { sha256sum "$1" | cut -d' ' -f1; }

RENDERED="$TMP/rendered.md"
sed \
    -e "s|{{VERSION}}|${VERSION}|g" \
    -e "s|{{RELEASE_DATE}}|${RELEASE_DATE}|g" \
    -e "s|{{BUILD_DATE}}|$(date -u +%Y-%m-%d)|g" \
    -e "s|{{REPO}}|${REPO}|g" \
    -e "s|{{SHA256_X86_64}}|$(sha256 "$X86")|g" \
    -e "s|{{SHA256_AARCH64}}|$(sha256 "$ARM")|g" \
    -e "s|{{SHA512_LINUX_X64}}|${SHA512_X64}|g" \
    -e "s|{{SHA512_LINUX_ARM64}}|${SHA512_ARM64}|g" \
    -e "s|{{GUI_VERSION}}|${GUI_VERSION}|g" \
    -e "s|{{PAGE_URL}}|${PAGE_URL}|g" \
    -e "/{{CHANGELOG}}/r $ROOT/CHANGELOG.md" \
    -e "/{{CHANGELOG}}/d" \
    "$TEMPLATE" > "$RENDERED"

# Fail loudly if a placeholder survived - a typo must not reach a release.
if grep -q '{{[A-Z_]*}}' "$RENDERED"; then
    echo "error: unresolved placeholder(s) in release notes:" >&2
    grep -o '{{[A-Z_]*}}' "$RENDERED" | sort -u >&2
    exit 1
fi

cat "$RENDERED"
