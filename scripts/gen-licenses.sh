#!/usr/bin/env bash
#
# gen-licenses.sh - regenerate the GUI's aggregated third-party licence
# texts from the exact dependency graph pinned in gui/Cargo.lock. The
# output (src/licenses/proton-drive-gui-THIRD-PARTY.txt) ships inside every
# release AppImage, so every licence that covers the shipped binary is
# readable on the machine that runs it. Run it after any dependency change
# (build.sh refuses to package when the file is missing).
#
#   ./scripts/gen-licenses.sh      # needs: cargo install cargo-about
#
set -euo pipefail
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"

command -v cargo-about >/dev/null 2>&1 || {
    echo "error: cargo-about not found - install it with:" >&2
    echo "    cargo install cargo-about --locked --features cli" >&2
    exit 1
}

OUT="$ROOT/src/licenses/proton-drive-gui-THIRD-PARTY.txt"
cd "$ROOT/gui"
cargo about generate --config about.toml --workspace \
    license-template.hbs --output-file "$OUT"
echo "wrote $OUT ($(wc -l <"$OUT") lines)"
