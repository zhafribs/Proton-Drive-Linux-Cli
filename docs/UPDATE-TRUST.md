# Update trust and key rotation

The updater accepts an AppImage only when `SHA256SUMS.sig` is a valid Ed25519
signature over `SHA256SUMS` by a key embedded in the running build, and the
AppImage digest matches that signed manifest. The GitHub API digest is only a
consistency check.

## Current key

`gui/src/release.rs` contains the public key list. The private key is kept in
the GitHub `UPDATE_SIGNING_KEY` secret and an offline backup; it is never in
this repository.

## Trusted keys

The successor public key generated on 2026-11-10 is:

```
f0ed7f4fa4fac94e0bf72931ee0ab16fd4882867c167572c988bec85986444c1
```

Its private key is offline; it is not in the repository or CI.

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

## Release revocation feed

Every release from `3.27.0` carries:

- `revocations.json` — `{ "version": 1, "updated": "…",
  "revoked_tags": [ … ] }`
- `revocations.json.sig` — an Ed25519 signature over that file

`scripts/sign-release.sh` copies the repository's `revocations.json` into
`dist/` and signs it with the release key. The updater fetches both from the
candidate release, verifies the signature against a trusted, non-revoked key,
and refuses a tag listed in `revoked_tags`. Releases older than `3.27.0`
predate the feed and remain installable.

To revoke a bad release, add its tag to `revoked_tags`, run the next signed
release, and the feed from that release will block it.

## Verify a published release

`scripts/verify-release.sh [tag]` downloads the assets, verifies
`SHA256SUMS`, both signatures, the AppImage smoke test and the GLIBC 2.28
floor.

## Limitation

The feed is signed by a trusted release key. A compromised key can sign a feed
that omits its own revocation, so this protects against a bad **release**, not
a stolen **key**. Key rotation remains preventative: add the successor key now,
keep the private keys offline, and publish the rotation release before the old
key is retired.
