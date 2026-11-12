#!/usr/bin/env bash
# Build the AppImage twice with a fixed source date and require byte equality.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct)}"
./build.sh x86_64 >/tmp/proton-repro-1.log 2>&1
cp "$(ls -t dist/ProtonDrive-CLI-*-x86_64.AppImage | head -1)" /tmp/proton-repro-1.AppImage
./build.sh x86_64 >/tmp/proton-repro-2.log 2>&1
cp "$(ls -t dist/ProtonDrive-CLI-*-x86_64.AppImage | head -1)" /tmp/proton-repro-2.AppImage
sha256sum /tmp/proton-repro-1.AppImage /tmp/proton-repro-2.AppImage
if cmp -s /tmp/proton-repro-1.AppImage /tmp/proton-repro-2.AppImage; then
    echo "REPRODUCIBLE: the two builds are byte-identical"
else
    echo "NOT REPRODUCIBLE: the two builds differ" >&2
    exit 1
fi
