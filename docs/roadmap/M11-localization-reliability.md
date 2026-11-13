# Milestone M11 — Localization & Reliability Evidence

**Status:** in progress
**Theme:** make the interface usable in Malay and turn the reliability claims
into repeatable evidence.

## Part 1 — Reliability evidence (`3.32.0`)

- Sixty-cycle soak test with two-sided churn.
- Opt-in real-account live E2E (`scripts/live-e2e.sh`).
- Fault matrix retained: failed upload, short download, session/network
  classification, update rollback.
- Scan baseline re-run and recorded.

## Part 2 — Complete localization (`3.33.0`)

- Every page, dialog, menu, empty state, error and notification available in
  English and Bahasa Melayu.
- Locale-aware dates and relative times.
- A completeness check so a new English string cannot ship untranslated.
- Remove the “beta” note from the language selector.

## Not covered

- Flatpak distribution/Flathub/aarch64 (deliberately omitted for now).
- Running a real Proton account inside CI.
