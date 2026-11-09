# Milestone M5 — App Decomposition

**Status:** complete — released as `v0.9.0-gui3.25.1`
**Theme:** reduce the `App` hub without changing behavior.

## What changed

- `AppCtx` owns shared infrastructure: paths, prefs, conf, channels, health,
  identity.
- Every page receives a narrow view instead of `&mut App`:
  - `RecoveryCtx`
  - `PhotoView`
  - `SettingsView`
  - `AboutView`
  - `HomeView`
  - `ActivityView`
  - `CloudView`
- `Back` still carries every worker message; routing is unchanged.
- Page-specific actions are returned to `App` (login, updates, restart, page
  navigation) instead of pages calling App methods directly.

## Result

| Metric | Before | After |
|---|---:|---:|
| `App` graph edges | 229 | **75** |
| Page modules importing `App` | 7 | **0** |
| Tests | 299 | 299 |
| Clippy / gate | clean / 12/12 | clean / 12/12 |
| User-visible behavior | — | unchanged |

## Method

One page per commit, each step verified with tests, clippy, release-gate
fixtures and a UI smoke check. The graph was rebuilt after each slice to keep
the measurement honest.

## Release

`v0.9.0-gui3.25.1`; evidence in
`docs/audit/evidence/release-3.25.1.txt`.

