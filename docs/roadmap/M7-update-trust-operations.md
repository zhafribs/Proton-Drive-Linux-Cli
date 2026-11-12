# Milestone M7 — Update Trust Operations

**Status:** complete — released as `v0.9.0-gui3.27.0` and `v0.9.0-gui3.28.0`
**Theme:** make the update trust path operable, not just cryptographically
correct.

## Scope

- **Successor key provisioned:** an offline Ed25519 key was generated; its
  public key is embedded in `UPDATE_PUBLIC_KEYS`.
- **Signed release revocation feed:** `revocations.json` + `.sig` are release
  assets; the updater refuses a listed tag for releases at or after `3.27.0`.
- **Successor-signed key revocation feed:** `security/key-revocations.json`
  and its signature are fetched from `main`. From `3.28.0` a missing, malformed
  or badly signed feed fails closed, and a manifest signed by a listed key is
  rejected.
- **Reproducible build proven:** `build.sh` uses `SOURCE_DATE_EPOCH`; CI
  builds twice and requires byte equality, and `scripts/repro-check.sh` does
  the same locally.
- **Automated release verification:** `scripts/verify-release.sh` downloads a
  published release and checks both signatures, smoke and GLIBC floor.
- **Rotation/revocation documentation:** `docs/UPDATE-TRUST.md` covers the
  successor key, both feeds, signing and remaining limits.

## Release verification

### 3.27.0

- Tag `v0.9.0-gui3.27.0`; CI `42819252214` verify + release succeeded.
- `scripts/verify-release.sh v0.9.0-gui3.27.0` verified checksum, both
  signatures, smoke and GLIBC 2.28.
- 303 tests, clippy clean, release gate 12/12.
- Evidence: `docs/audit/evidence/release-3.27.0.txt`.

### 3.28.0

- Tag `v0.9.0-gui3.28.0`; evidence in
  `docs/audit/evidence/release-3.28.0.txt`.
- Key feed signed by the offline successor key and verified by the release
  gate.
- Reproducibility: two local builds were byte-identical; CI repeats the check
  on the release commit.
