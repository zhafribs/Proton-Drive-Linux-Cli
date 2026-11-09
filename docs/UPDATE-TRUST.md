# Update trust and key rotation

The updater accepts an AppImage only when `SHA256SUMS.sig` is a valid Ed25519
signature over `SHA256SUMS` by a key embedded in the running build, and the
AppImage digest matches that signed manifest. The GitHub API digest is only a
consistency check.

## Current key

`gui/src/release.rs` contains the public key list. The private key is kept in
the GitHub `UPDATE_SIGNING_KEY` secret and an offline backup; it is never in
this repository.

## Rotation

1. Generate a new Ed25519 key offline with
   `./scripts/new-update-key.sh ~/Backup/proton-update-successor-key.pem`
   and back it up.
2. Add its **public** key to `UPDATE_PUBLIC_KEYS` in a release signed by the
   current key.
3. After users have had time to install that release, sign new releases with
   the new key. Both keys are accepted during the migration window.
4. When the migration window closes, remove the old public key. A release
   signed by the new key may also add the old key to `REVOKED_UPDATE_KEYS` to
   reject it immediately.

## Revocation

`REVOKED_UPDATE_KEYS` is checked before every signature verification. A
revoked key is skipped even if its signature is cryptographically valid. This
protects users who install the revoking build.

## Limitation

There is no remote revocation feed. If the current signing key is compromised,
an attacker can sign updates until a build signed by a trusted successor key
is installed. Rotation is therefore preventative: add the successor key now,
keep the private keys offline, and publish the rotation release before the old
key is retired. A signed revocation feed is a future hardening step.
