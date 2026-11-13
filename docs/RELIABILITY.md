# Reliability evidence

This page lists what is proven automatically, what needs a real account, and
what is still only a design argument.

## Automated in CI

| Area | Evidence |
|---|---|
| Full sync cycle | `crates/engine/tests/full_cycle.rs`: upload, download, remote delete, local delete, state |
| Soak/churn | 60 consecutive cycles with files added and removed on both sides; trees and `state.json` converge with no conflicts |
| Failed upload | The local file stays; state is not stamped as synced |
| Short download | A truncated staged file never becomes the destination |
| Recovery drill | Snapshot restore, local trash restore, conflict actions |
| Session vs network | `failure_status` keeps timeouts/proxies as `error`; only 401/login/keyring become `needs_login` |
| Session refresh | `warm_session` runs once under the app-wide lock before parallel batches; a dead session fails before workers start |
| Update trust | Signed `SHA256SUMS`, key-revocation feed, revoked tags/keys, atomic swap marker, rollback |
| Update download | Truncation is rejected; a bad digest/signature refuses install |
| UI | Headless pages, dialogs, onboarding, update progress, AccessKit names, keyboard-only Settings activation |
| Reproducibility | Two local builds and the CI release build are byte-identical |

## Needs a real account (opt-in)

```bash
PROTON_SYNC_LIVE=1 ./scripts/live-e2e.sh
```

This creates a temporary folder under `/my-files`, uploads one file, lists it,
downloads it, compares bytes, and trashes the folder. It is intentionally not
part of CI: CI has no Proton account, and a real upload is a write.

## Performance signal

`./scripts/bench-scan.sh 10000` scans 10,000 local files in ~14 ms on the
development machine. This is a regression signal, not a whole-cycle claim:
network, hashing and transfers are excluded.

## Not proven

- A live Proton account run in CI (credentials and destructive writes).
- Real network failure injection against Proton's API.
- aarch64 execution; packages are checked structurally and by GLIBC floor.
- Long-running multi-day memory usage.
