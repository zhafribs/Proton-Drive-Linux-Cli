# Milestone M3 — Update Trust

**Status:** complete — released as `v0.9.0-gui3.23.0`  
**Theme:** make key rotation and revocation possible without changing the
on-disk trust model.

## Scope

- Accept a signature from any listed, non-revoked update public key.
- Add a revoked-key list checked before every manifest signature.
- Document the rotation and revocation procedure, including its limitation.
- Keep the existing SHA-256 + Ed25519 verification and rollback marker.

## Acceptance

| Check | Result |
|---|---|
| A signature from a second listed key is accepted | `signed_manifest_verifies_and_tampering_fails` |
| A revoked key is rejected even with a valid signature | Same test |
| The real project key still verifies the shipped self-test | `project_key_verifies_an_openssl_signature` |
| Release gate and smoke | CI + signed AppImage |

## Not covered

- No remote revocation feed; a compromised key remains usable until a build
  signed by a trusted successor key is installed.
- Key generation and offline backup remain operator procedures.

## Release verification

- Tag `v0.9.0-gui3.23.0`; CI `42595328464` verify + release succeeded.
- SHA-256, Ed25519 signature, smoke and GLIBC 2.28 verified.
- Evidence: `docs/audit/evidence/release-3.23.0.txt`.
