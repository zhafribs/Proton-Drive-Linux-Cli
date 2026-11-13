# Milestone M11 — Localization & Reliability Evidence

**Status:** complete — `3.32.0` (reliability) and `3.33.0` (localization)
**Theme:** make the interface usable in Malay and turn the reliability claims
into repeatable evidence.

## Part 1 — Reliability evidence (shipped in `3.33.0`)

- Sixty-cycle soak test with two-sided churn.
- Opt-in real-account live E2E (`scripts/live-e2e.sh`).
- Fault matrix retained: failed upload, short download, session/network
  classification, update rollback.
- Scan baseline re-run and recorded.

## Part 2 — Complete localization (shipped in `3.33.0`)

- Every page, dialog, menu, empty state, app error, notification and tray
  entry is available in English and Bahasa Melayu.
- Counts, relative times and journal verbs translate by shape.
- The “beta” note is gone. File names and raw CLI errors stay as reported.
- Malay page/dialog screenshots are in `docs/audit/m11/`.

## Not covered

- Flatpak distribution/Flathub/aarch64 (deliberately omitted for now).
- Running a real Proton account inside CI.
