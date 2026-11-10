# Milestone M6 — Engine Confidence

**Status:** complete — released as `v0.9.0-gui3.26.0`  
**Theme:** prove the sync cycle end to end, not only unit by unit.

## Scope

- **Full-cycle E2E:** a stateful fake CLI keeps a remote tree on disk, so one
  `--once` run exercises list, upload, download, trash, state and deletes.
- **Fault matrix:** failed upload does not advance state; a short download
  never becomes the destination; corrupt state aborts before any CLI call;
  conflict copies keep both revisions; the recovery drill restores a snapshot.
- **Benchmark:** `scripts/bench-scan.sh` measures local scan time on 5k/10k
  files and records a baseline.

## Not covered

- Real Proton account E2E (requires credentials) and live network failure
  injection.
- Automated UI screenshots, keyboard-only CI and screen-reader checks; those
  belong to the UI confidence milestone.
- A hard performance gate in CI; the benchmark is a repeatable local signal.

## Release verification

- Tag `v0.9.0-gui3.26.0`; CI `42817271289` verify + release succeeded.
- 302 tests, clippy clean, release gate 12/12.
- Signed AppImage, smoke and GLIBC 2.28 verified.
- Evidence: `docs/audit/evidence/release-3.26.0.txt`.
