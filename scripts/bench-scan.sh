#!/usr/bin/env bash
# Repeatable local-scan benchmark. Creates a disposable tree, runs the
# release engine's scan-local three times and prints milliseconds.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FILES="${1:-10000}"
TMP="$(mktemp -d /tmp/proton-scan-bench.XXXXXX)"
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/tree"
for i in $(seq 1 "$FILES"); do
    d="$TMP/tree/d$((i % 100))"
    mkdir -p "$d"
    printf 'bench\n' > "$d/f$i.txt"
done
cargo build --release -p proton-engine >/dev/null
BIN="$ROOT/gui/target/release/proton-engine"
for run in 1 2 3; do
    start="$(date +%s%N)"
    "$BIN" scan-local --root "$TMP/tree" >/dev/null
    end="$(date +%s%N)"
    printf 'run %d: %d ms (%d files)\n' "$run" "$(((end - start) / 1000000))" "$FILES"
done
