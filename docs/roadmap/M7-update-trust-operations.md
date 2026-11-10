# Milestone M7 — Update Trust Operations

**Status:** complete — released as `v0.9.0-gui3.27.0`  
**Theme:** make the update trust path operable, not just cryptographically
correct.

## Scope

- **Successor key provisioned:** an offline Ed25519 key was generated; its
  public key is embedded in `UPDATE_PUBLIC_KEYS`.
- **Signed revocation feed:** `revocations.json` + `.sig` are release assets;
  the updater refuses a listed tag for releases at or after `3.27.0`.
- **Automated release verification:** `scripts/verify-release.sh` downloads a
  published release and checks both signatures, smoke and GLIBC floor.
- **Rotation/revocation documentation:** `docs/UPDATE-TRUST.md` covers the
  successor key, feed format, signing and limitations.

## Not covered

- A remote **key** revocation feed that a compromised current key cannot
  suppress. That requires a successor-signed policy and an operational
  decision; the feed here is release-tag revocation.
- Reproducible-build attestation; the build is deterministic in inputs but not
  yet proven bit-for-bit in CI.

## Release verification

- Tag `v0.9.0-gui3.27.0`; CI `42819252214` verify + release succeeded.
- `scripts/verify-release.sh v0.9.0-gui3.27.0` verified checksum, both
  signatures, smoke and GLIBC 2.28.
- 303 tests, clippy clean, release gate 12/12.
- Evidence: `docs/audit/evidence/release-3.27.0.txt`.
