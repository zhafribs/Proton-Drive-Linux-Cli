#!/usr/bin/env bash
#
# check-release.sh - fail a release that does not match its own metadata.
#
# Run after ./build.sh, before publishing. It checks the built AppDir, the
# changelog section, the version against the latest tag, and the checksums.
# CHECK_RELEASE_ROOT and CHECK_RELEASE_SKIP_SMOKE exist for the fixture tests.
set -euo pipefail

ROOT="${CHECK_RELEASE_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
BUILD="$ROOT/build"
DIST="$ROOT/dist"
APP_DIR="$BUILD/ProtonDrive-CLI.AppDir"
DESKTOP_ID="io.github.zhafribs.protondrivecli.desktop"
METAINFO_ID="io.github.zhafribs.protondrivecli.appdata.xml"

die() { printf 'check-release: %s\n' "$*" >&2; exit 1; }

[ -f "$ROOT/upstream.lock" ] || die "upstream.lock missing"
[ -f "$ROOT/CHANGELOG.md" ] || die "CHANGELOG.md missing"
[ -d "$APP_DIR" ] || die "built AppDir missing - run ./build.sh first"
[ -d "$DIST" ] || die "dist/ missing - run ./build.sh first"

VERSION="$(awk -F= '$1 == "VERSION" { sub(/^[^=]*=/, ""); print; exit }' "$ROOT/upstream.lock")"
GUI_VERSION="$(sed -n 's/^pub const GUI_VERSION: &str = "\(.*\)";$/\1/p' "$ROOT/gui/src/main.rs" | head -1)"
[ -n "$VERSION" ] || die "upstream.lock has no VERSION"
[ -n "$GUI_VERSION" ] || die "gui/src/main.rs has no GUI_VERSION"

APPIMAGE="$DIST/ProtonDrive-CLI-${VERSION}-GUI-${GUI_VERSION}-x86_64.AppImage"
[ -f "$APPIMAGE" ] || die "missing artifact $(basename "$APPIMAGE")"
[ -f "$DIST/SHA256SUMS" ] || die "dist/SHA256SUMS missing"

# --- changelog: the FIRST section must be this GUI version, and it must have
# at least one bullet. A header-only section is the "Maintenance rebuild"
# stub the release process is supposed to prevent.
first_heading="$(grep -m1 '^## ' "$ROOT/CHANGELOG.md" || true)"
case "$first_heading" in
    "## ${GUI_VERSION} — "[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]) ;;
    *) die "CHANGELOG.md top section is not '## ${GUI_VERSION} — YYYY-MM-DD' (got: ${first_heading:-none})" ;;
esac
body="$(awk -v v="$GUI_VERSION" '
    $1 == "##" && $2 == v { on = 1; next }
    on && $1 == "##" { exit }
    on { print }
' "$ROOT/CHANGELOG.md" | grep -E '^[[:space:]]*-[[:space:]]+' | head -1 || true)"
[ -n "$body" ] || die "CHANGELOG.md section for ${GUI_VERSION} has no content"

# --- built metadata: exact version, no unsubstituted template, correct IDs.
desktop="$APP_DIR/$DESKTOP_ID"
desktop_installed="$APP_DIR/usr/share/applications/$DESKTOP_ID"
metainfo="$APP_DIR/usr/share/metainfo/$METAINFO_ID"
[ -f "$desktop" ] || die "AppDir desktop file missing"
[ -f "$desktop_installed" ] || die "installed desktop file missing"
[ -f "$metainfo" ] || die "AppDir metainfo file missing"
for f in "$desktop" "$desktop_installed"; do
    grep -qx "X-AppImage-Version=${GUI_VERSION}" "$f" \
        || die "$(basename "$f") does not carry X-AppImage-Version=${GUI_VERSION}"
done
if [ -n "${SOURCE_DATE_EPOCH:-}" ]; then
    metainfo_date="$(date -u -d "@${SOURCE_DATE_EPOCH}" +%F)"
else
    metainfo_date="$(date -u +%F)"
fi
grep -q "<release version=\"${GUI_VERSION}\" date=\"${metainfo_date}\"/>" "$metainfo" \
    || die "metainfo release version/date does not match ${GUI_VERSION} (${metainfo_date})"
grep -q '<id>io.github.zhafribs.protondrivecli</id>' "$metainfo" \
    || die "metainfo component id is not io.github.zhafribs.protondrivecli"
grep -q '<launchable type="desktop-id">io.github.zhafribs.protondrivecli.desktop</launchable>' "$metainfo" \
    || die "metainfo launchable does not match the desktop id"
grep -q '<developer id="io.github.zhafribs">' "$metainfo" \
    || die "metainfo developer id is not io.github.zhafribs"
grep -q '<url type="homepage">https://github.com/zhafribs/Proton-Drive-Linux-Cli</url>' "$metainfo" \
    || die "metainfo homepage does not point at the repository"
if grep -qE '@(GUI_VERSION|RELEASE_DATE)@' "$desktop" "$desktop_installed" "$metainfo"; then
    die "unsubstituted template placeholder in built metadata"
fi

# --- all three shipped binaries exist; the GUI must report this version.
[ -x "$APP_DIR/usr/bin/proton-engine" ] || die "proton-engine missing from the AppDir"
[ -x "$APP_DIR/usr/bin/proton-drive-gui" ] || die "GUI missing from the AppDir"
[ -x "$APP_DIR/AppRun" ] || die "AppRun missing from the AppDir"

if [ "${CHECK_RELEASE_SKIP_SMOKE:-0}" != "1" ]; then
    smoke="$(PROTON_SYNC_SMOKE=1 APPIMAGE_EXTRACT_AND_RUN=1 "$APP_DIR/AppRun" --smoke 2>&1)" \
        || die "AppRun --smoke failed: ${smoke}"
    grep -q "gui ${GUI_VERSION}" <<<"$smoke" \
        || die "smoke output does not report gui ${GUI_VERSION}: ${smoke}"
fi

# --- checksums and the version-bump guard.
( cd "$DIST" && sha256sum -c --ignore-missing SHA256SUMS >/dev/null ) \
    || die "dist/SHA256SUMS does not match the built AppImage"
grep -q "  $(basename "$APPIMAGE")\$" "$DIST/SHA256SUMS" \
    || die "dist/SHA256SUMS has no entry for $(basename "$APPIMAGE")"

# The updater refuses unsigned manifests; the release gate must too.
PUBLIC_KEY="${CHECK_RELEASE_PUBLIC_KEY:-$ROOT/src/update-signing-key.pub}"
[ -f "$PUBLIC_KEY" ] || die "src/update-signing-key.pub is missing"
[ -f "$DIST/SHA256SUMS.sig" ] || die "dist/SHA256SUMS.sig is missing - run scripts/sign-release.sh"
command -v openssl >/dev/null 2>&1 || die "openssl not found"
openssl pkeyutl -verify -pubin -inkey "$PUBLIC_KEY" -rawin \
    -in "$DIST/SHA256SUMS" -sigfile "$DIST/SHA256SUMS.sig" >/dev/null 2>&1 \
    || die "SHA256SUMS.sig does not verify against the project public key"

# A release that carries a revocation feed must carry a valid signature.
if [ -f "$DIST/revocations.json" ]; then
    [ -f "$DIST/revocations.json.sig" ] \
        || die "dist/revocations.json.sig is missing - run scripts/sign-release.sh"
    openssl pkeyutl -verify -pubin -inkey "$PUBLIC_KEY" -rawin \
        -in "$DIST/revocations.json" -sigfile "$DIST/revocations.json.sig" >/dev/null 2>&1 \
        || die "revocations.json.sig does not verify against the project public key"
fi

# The public key file and the key embedded in the updater must be the same.
# Fixtures override the path with a throwaway key, so only check the real one.
if [ -z "${CHECK_RELEASE_PUBLIC_KEY:-}" ] && [ "${ALLOW_KEY_ROTATION:-0}" != "1" ]; then
    embedded="$(sed -n 's/.*UPDATE_PUBLIC_KEY_HEX: &str = "\([0-9a-f]*\)";/\1/p' \
        "$ROOT/gui/src/release.rs" | head -1)"
    derived="$(openssl pkey -pubin -in "$PUBLIC_KEY" -outform DER 2>/dev/null \
        | tail -c 32 | od -An -tx1 | tr -d ' \n')"
    [ -n "$embedded" ] && [ "$embedded" = "$derived" ] \
        || die "gui/src/release.rs embedded update key does not match src/update-signing-key.pub"
fi

# The key-revocation feed is fetched at update time from the repository, so
# the committed copy must carry a valid successor signature. Like the key
# check above, this only applies to a real release, not a gate fixture.
if [ -z "${CHECK_RELEASE_PUBLIC_KEY:-}" ]; then
    KEY_FEED="$ROOT/security/key-revocations.json"
    SUCCESSOR_PUBLIC="$ROOT/src/update-signing-successor.pub"
    [ -f "$KEY_FEED" ] || die "security/key-revocations.json is missing"
    [ -f "$KEY_FEED.sig" ] || die "security/key-revocations.json.sig is missing - run scripts/sign-key-revocations.sh"
    [ -f "$SUCCESSOR_PUBLIC" ] || die "src/update-signing-successor.pub is missing"
    openssl pkeyutl -verify -pubin -inkey "$SUCCESSOR_PUBLIC" -rawin \
        -in "$KEY_FEED" -sigfile "$KEY_FEED.sig" >/dev/null 2>&1 \
        || die "key-revocations.json.sig does not verify against the successor key"
    grep -q '"revoked_keys"' "$KEY_FEED" \
        || die "security/key-revocations.json has no revoked_keys list"
    embedded_successor="$(sed -n '/UPDATE_SUCCESSOR_PUBLIC_KEY_HEX/,/;/s/.*"\([0-9a-f]\{64\}\)".*/\1/p' \
        "$ROOT/gui/src/release.rs" | head -1)"
    derived_successor="$(openssl pkey -pubin -in "$SUCCESSOR_PUBLIC" -outform DER 2>/dev/null \
        | tail -c 32 | od -An -tx1 | tr -d ' \n')"
    [ -n "$embedded_successor" ] && [ "$embedded_successor" = "$derived_successor" ] \
        || die "gui/src/release.rs embedded successor key does not match src/update-signing-successor.pub"
fi

allow_same="${ALLOW_SAME_VERSION:-0}"
case "$allow_same" in
    1|true|TRUE|yes) allow_same=1 ;;
    *) allow_same=0 ;;
esac
if git -C "$ROOT" rev-parse --git-dir >/dev/null 2>&1 && [ "$allow_same" != "1" ]; then
    latest_tag="$(git -C "$ROOT" tag --list 'v*-gui*' --sort=-v:refname | head -1)"
    if [ -n "$latest_tag" ]; then
        tag_gui="${latest_tag##*-gui}"
        if [ "$tag_gui" = "$GUI_VERSION" ]; then
            changed="$(git -C "$ROOT" diff --name-only "$latest_tag" -- \
                gui crates src build.sh scripts .github Cargo.toml Cargo.lock \
                rust-toolchain.toml vendor | head -1 || true)"
            [ -z "$changed" ] \
                || die "GUI_VERSION ${GUI_VERSION} was not bumped for changes since ${latest_tag} (set ALLOW_SAME_VERSION=1 only for an intentional rebuild)"
        fi
    fi
fi

printf 'check-release: OK — CLI %s, GUI %s\n' "$VERSION" "$GUI_VERSION"
