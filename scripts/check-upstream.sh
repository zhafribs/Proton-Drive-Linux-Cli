#!/usr/bin/env bash
#
# check-upstream.sh - Keep upstream.lock in sync with Proton's published CLI.
#
# Parses https://proton.me/download/drive/cli/index.html for the latest
# version, its release date and the per-platform SHA-512 checksums Proton
# publishes, compares them with upstream.lock and rewrites the lock when a new
# release (or a changed checksum) appears. build.sh then packages whatever the
# lock says and verifies the download against it.
#
# Usage:
#   ./scripts/check-upstream.sh             # update upstream.lock if outdated
#   ./scripts/check-upstream.sh --dry-run   # report only; exit 3 if outdated
#
# Emits (appended to $GITHUB_OUTPUT when that is set, so CI can branch on it):
#   updated=true|false   version=x.y.z   release_date=YYYY-MM-DD
#
# Exit codes: 0 ok, 2 parse/network failure, 3 update available (--dry-run only)
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LOCK="$ROOT/upstream.lock"
DRY_RUN=0
[ "${1:-}" = "--dry-run" ] && DRY_RUN=1

# Platforms this project packages. Must stay in sync with APPIMAGE_ARCH in build.sh.
# aarch64 was discontinued: releases are x86_64-only.
REQUIRED_PLATFORMS=(linux-x64)

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

die() { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 2; }
note() { printf '\033[1;35m==>\033[0m %s\n' "$*"; }

lock_get() {
    [ -f "$LOCK" ] || return 0
    awk -F= -v key="$1" '$1 == key { sub(/^[^=]*=/, ""); print; exit }' "$LOCK"
}
sha512_key() { printf 'SHA512_%s' "$1" | tr '[:lower:]-' '[:upper:]_'; }

PAGE_URL="$(lock_get PAGE_URL)"
DOWNLOAD_BASE="$(lock_get DOWNLOAD_BASE)"
[ -n "$PAGE_URL" ]       || PAGE_URL="https://proton.me/download/drive/cli/index.html"
[ -n "$DOWNLOAD_BASE" ]  || DOWNLOAD_BASE="https://proton.me/download/drive/cli"

fetch_page() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL --max-time 60 -o "$1" "$PAGE_URL"
    elif command -v wget >/dev/null 2>&1; then
        wget -q -T 60 -O "$1" "$PAGE_URL"
    else
        die "curl or wget is required"
    fi
}

parse_page() {
    local html="$1" rows="$TMP/rows" row platform sha

    # "<h1>Proton Drive CLI 0.9.0</h1>", falling back to any versioned URL
    PAGE_VERSION="$(grep -oE 'Proton Drive CLI [0-9]+\.[0-9]+\.[0-9]+' "$html" | head -n1 \
                     | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' || true)"
    [ -n "$PAGE_VERSION" ] || PAGE_VERSION="$(grep -oE '/download/drive/cli/[0-9]+\.[0-9]+\.[0-9]+/' "$html" \
                     | head -n1 | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' || true)"
    [ -n "$PAGE_VERSION" ] || die "could not parse a version from $PAGE_URL (page layout changed?)"

    PAGE_RELEASE_DATE="$(grep -oE 'Release date: [0-9]{4}-[0-9]{2}-[0-9]{2}' "$html" | head -n1 \
                     | grep -oE '[0-9]{4}-[0-9]{2}-[0-9]{2}' || true)"
    [ -n "$PAGE_RELEASE_DATE" ] || PAGE_RELEASE_DATE="unknown"

    # Collapse every <tr>...</tr> onto one line so URL + checksum stay together.
    awk '/<tr>/{buf=""; f=1} f{buf=buf " " $0} /<\/tr>/{if(f){print buf; f=0}}' "$html" > "$rows"

    declare -gA PAGE_SHA512=()
    while IFS= read -r row; do
        platform="$(printf '%s' "$row" \
            | grep -oE "${DOWNLOAD_BASE//./\\.}/[0-9]+\.[0-9]+\.[0-9]+/[a-z0-9._-]+/proton-drive(\.exe)?" | head -n1 \
            | sed -E 's|.*/cli/[0-9]+\.[0-9]+\.[0-9]+/([a-z0-9._-]+)/[^/]+$|\1|' || true)"
        sha="$(printf '%s' "$row" | grep -oE '[0-9a-f]{128}' | head -n1 || true)"
        [ -n "$platform" ] && [ -n "$sha" ] || continue
        PAGE_SHA512[$platform]="$sha"
    done < "$rows"

    [ "${#PAGE_SHA512[@]}" -gt 0 ] || die "parsed no checksum rows from $PAGE_URL (page layout changed?)"
}

write_lock() {
    local platform key
    {
        cat <<'EOF'
# upstream.lock - single source of truth for the packaged Proton Drive CLI.
#
# DO NOT EDIT BY HAND. scripts/check-upstream.sh parses
# https://proton.me/download/drive/cli/index.html and rewrites this file, and
# .github/workflows/release.yml runs that automatically on a schedule.
# build.sh refuses to package anything whose SHA-512 differs from the values
# recorded here, which are themselves copied from Proton's published table.
#
# To add another upstream platform (e.g. linux-x64-musl), add a
# SHA512_<PLATFORM> line (dashes become underscores, upper-case) and a matching
# entry in APPIMAGE_ARCH in build.sh.
EOF
        echo "PAGE_URL=$PAGE_URL"
        echo "DOWNLOAD_BASE=$DOWNLOAD_BASE"
        echo "VERSION=$PAGE_VERSION"
        echo "RELEASE_DATE=$PAGE_RELEASE_DATE"
        for platform in "${REQUIRED_PLATFORMS[@]}"; do
            key="$(sha512_key "$platform")"
            echo "$key=${PAGE_SHA512[$platform]}"
        done
    } > "$LOCK"
}

emit_outputs() {
    local updated="$1"
    if [ -n "${GITHUB_OUTPUT:-}" ]; then
        {
            echo "updated=$updated"
            echo "version=$PAGE_VERSION"
            echo "release_date=$PAGE_RELEASE_DATE"
        } >> "$GITHUB_OUTPUT"
    fi
}

note "checking $PAGE_URL"
fetch_page "$TMP/page.html" || die "failed to download $PAGE_URL"
parse_page "$TMP/page.html"

for platform in "${REQUIRED_PLATFORMS[@]}"; do
    [ -n "${PAGE_SHA512[$platform]:-}" ] || die "upstream page no longer lists $platform - update REQUIRED_PLATFORMS and APPIMAGE_ARCH"
done

# Informational: upstream ships more platforms than we package (musl, BSD, Windows).
unhandled="$(
    for platform in "${!PAGE_SHA512[@]}"; do
        case " ${REQUIRED_PLATFORMS[*]} " in *" $platform "*) ;; *) echo "$platform" ;; esac
    done | sort | tr '\n' ' ' | sed 's/ $//'
)"
[ -n "$unhandled" ] && note "upstream also publishes (not packaged here): $unhandled"

CURRENT_VERSION="$(lock_get VERSION)"
CURRENT_RELEASE_DATE="$(lock_get RELEASE_DATE)"
changed=0
[ "$CURRENT_VERSION" = "$PAGE_VERSION" ] || changed=1
for platform in "${REQUIRED_PLATFORMS[@]}"; do
    key="$(sha512_key "$platform")"
    [ "$(lock_get "$key")" = "${PAGE_SHA512[$platform]}" ] || changed=1
done

if [ "$changed" -eq 0 ]; then
    note "up to date: Proton Drive CLI $PAGE_VERSION ($PAGE_RELEASE_DATE)"
    emit_outputs false
    exit 0
fi

note "upstream changed: ${CURRENT_VERSION:-<no lock>} -> $PAGE_VERSION (released $PAGE_RELEASE_DATE)"
for platform in "${REQUIRED_PLATFORMS[@]}"; do
    key="$(sha512_key "$platform")"
    printf '    %-14s %.12s -> %.12s\n' "$platform" "$(lock_get "$key")" "${PAGE_SHA512[$platform]}"
done

if [ "$DRY_RUN" -eq 1 ]; then
    note "dry run: upstream.lock NOT written"
    emit_outputs true
    exit 3
fi

write_lock
note "upstream.lock updated to $PAGE_VERSION"
emit_outputs true
