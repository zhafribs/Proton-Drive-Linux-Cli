#!/usr/bin/env bash
#
# test-check-release.sh - fixture tests for check-release.sh.
# Runs in CI's verify job. No network, no real AppImage.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GATE="$ROOT/scripts/check-release.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fixture() {
    local root="$TMP/repo"
    rm -rf "$root"
    mkdir -p "$root/gui/src" "$root/build/ProtonDrive-CLI.AppDir/usr/bin" \
             "$root/build/ProtonDrive-CLI.AppDir/usr/share/applications" \
             "$root/build/ProtonDrive-CLI.AppDir/usr/share/metainfo" \
             "$root/dist"
    printf 'VERSION=1.2.3\nRELEASE_DATE=2026-01-01\n' > "$root/upstream.lock"
    printf 'pub const GUI_VERSION: &str = "9.9.9";\n' > "$root/gui/src/main.rs"
    printf '## 9.9.9 — %s\n\n- A real change.\n' "$(date -u +%F)" > "$root/CHANGELOG.md"

    cat > "$root/build/ProtonDrive-CLI.AppDir/usr/bin/proton-engine" <<'EOF'
#!/bin/sh
exit 0
EOF
    cat > "$root/build/ProtonDrive-CLI.AppDir/usr/bin/proton-drive-gui" <<'EOF'
#!/bin/sh
exit 0
EOF
    cat > "$root/build/ProtonDrive-CLI.AppDir/AppRun" <<'EOF'
#!/bin/sh
echo "smoke ok: gui 9.9.9, cli true"
EOF
    chmod +x "$root/build/ProtonDrive-CLI.AppDir/AppRun" \
             "$root/build/ProtonDrive-CLI.AppDir/usr/bin/proton-engine" \
             "$root/build/ProtonDrive-CLI.AppDir/usr/bin/proton-drive-gui"

    local desktop="$root/build/ProtonDrive-CLI.AppDir/io.github.zhafribs.protondrivecli.desktop"
    printf '[Desktop Entry]\nX-AppImage-Version=9.9.9\n' > "$desktop"
    cp "$desktop" "$root/build/ProtonDrive-CLI.AppDir/usr/share/applications/"
    cat > "$root/build/ProtonDrive-CLI.AppDir/usr/share/metainfo/io.github.zhafribs.protondrivecli.appdata.xml" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<component type="desktop-application">
  <id>io.github.zhafribs.protondrivecli</id>
  <launchable type="desktop-id">io.github.zhafribs.protondrivecli.desktop</launchable>
  <url type="homepage">https://github.com/zhafribs/Proton-Drive-Linux-Cli</url>
  <developer id="io.github.zhafribs"><name>Zhafri Syazwi</name></developer>
  <releases><release version="9.9.9" date="$(date -u +%F)"/></releases>
</component>
EOF
    printf 'appimage-bytes\n' > "$root/dist/ProtonDrive-CLI-1.2.3-GUI-9.9.9-x86_64.AppImage"
    ( cd "$root/dist" && sha256sum ProtonDrive-CLI-1.2.3-GUI-9.9.9-x86_64.AppImage > SHA256SUMS )
    # A throwaway signing key: the gate only needs a valid signature.
    mkdir -p "$root/src"
    openssl genpkey -algorithm ED25519 -out "$root/test-key.pem" 2>/dev/null
    openssl pkey -in "$root/test-key.pem" -pubout -out "$root/src/update-signing-key.pub" 2>/dev/null
    openssl pkeyutl -sign -rawin -inkey "$root/test-key.pem" \
        -in "$root/dist/SHA256SUMS" -out "$root/dist/SHA256SUMS.sig" 2>/dev/null
    echo "$root"
}

run() {
    local root="$1"
    CHECK_RELEASE_ROOT="$root" \
        CHECK_RELEASE_PUBLIC_KEY="$root/src/update-signing-key.pub" \
        "$GATE" >/dev/null 2>&1
}

expect_ok() {
    local root; root="$(fixture)"
    run "$root" || { echo "FAIL: valid fixture rejected"; exit 1; }
}

expect_fail() {
    local label="$1" mutate="$2" root
    root="$(fixture)"
    eval "$mutate"
    if run "$root"; then
        echo "FAIL: $label was accepted" >&2
        exit 1
    fi
}

expect_ok

expect_fail "stale changelog" \
    'printf "## 9.9.8 — %s\n\n- old\n" "$(date -u +%F)" > "$root/CHANGELOG.md"'
expect_fail "empty changelog body" \
    'printf "## 9.9.9 — %s\n\n" "$(date -u +%F)" > "$root/CHANGELOG.md"'
expect_fail "wrong desktop version" \
    'sed -i "s/X-AppImage-Version=9.9.9/X-AppImage-Version=9.9.8/" "$root/build/ProtonDrive-CLI.AppDir/io.github.zhafribs.protondrivecli.desktop"'
expect_fail "missing engine" \
    'rm "$root/build/ProtonDrive-CLI.AppDir/usr/bin/proton-engine"'
expect_fail "wrong checksum" \
    'printf "%s  %s\n" "0000000000000000000000000000000000000000000000000000000000000000" "ProtonDrive-CLI-1.2.3-GUI-9.9.9-x86_64.AppImage" > "$root/dist/SHA256SUMS"'
expect_fail "unsubstituted template" \
    'printf "@GUI_VERSION@\n" >> "$root/build/ProtonDrive-CLI.AppDir/usr/share/metainfo/io.github.zhafribs.protondrivecli.appdata.xml"'
expect_fail "missing manifest signature" \
    'rm "$root/dist/SHA256SUMS.sig"'
expect_fail "tampered manifest signature" \
    'printf "bad" >> "$root/dist/SHA256SUMS.sig"'
expect_fail "wrong signing key" \
    'openssl genpkey -algorithm ED25519 -out "$root/other-key.pem" 2>/dev/null; openssl pkey -in "$root/other-key.pem" -pubout -out "$root/src/update-signing-key.pub" 2>/dev/null'

# Embedded-key drift is only checked when CHECK_RELEASE_PUBLIC_KEY is unset.
root="$(fixture)"
mkdir -p "$root/gui/src"
printf 'const UPDATE_PUBLIC_KEY_HEX: &str = "%064d";\n' 0 > "$root/gui/src/release.rs"
if CHECK_RELEASE_ROOT="$root" "$GATE" >/dev/null 2>&1; then
    echo "FAIL: embedded key drift was accepted" >&2
    exit 1
fi

# The version-bump guard needs a real git tag, so it gets its own fixture.
root="$(fixture)"
git -C "$root" init -q
git -C "$root" -c user.name=test -c user.email=test@example.com add -A
git -C "$root" -c user.name=test -c user.email=test@example.com commit -q -m base
git -C "$root" tag v1.2.3-gui9.9.9
printf '\n// source change without a GUI bump\n' >> "$root/gui/src/main.rs"
if CHECK_RELEASE_ROOT="$root" CHECK_RELEASE_PUBLIC_KEY="$root/src/update-signing-key.pub" "$GATE" >/dev/null 2>&1; then
    echo "FAIL: same GUI_VERSION with a source change was accepted" >&2
    exit 1
fi
ALLOW_SAME_VERSION=1 CHECK_RELEASE_ROOT="$root" CHECK_RELEASE_PUBLIC_KEY="$root/src/update-signing-key.pub" "$GATE" >/dev/null 2>&1 \
    || { echo "FAIL: force-rebuild override was rejected" >&2; exit 1; }
ALLOW_SAME_VERSION=true CHECK_RELEASE_ROOT="$root" CHECK_RELEASE_PUBLIC_KEY="$root/src/update-signing-key.pub" "$GATE" >/dev/null 2>&1 \
    || { echo "FAIL: force-rebuild true override was rejected" >&2; exit 1; }

echo "check-release fixtures: 12/12 passed"
