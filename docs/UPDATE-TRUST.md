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

## Key revocation feed

A compromised primary key must be rejectable by builds that already trusted
it. From `3.28.0` the updater fetches:

```
security/key-revocations.json
security/key-revocations.json.sig
```

from `main` on `raw.githubusercontent.com`. The feed is:

```json
{ "version": 1, "updated": "…", "revoked_keys": [ "<64-hex public key>" ] }
```

Its signature is made by the **successor key**, not the release key. The
updater requires the feed: a missing, malformed or badly signed feed fails
closed. Releases before `3.28.0` do not check it.

Sign the feed offline after every edit:

```sh
./scripts/sign-key-revocations.sh
```

The script uses `UPDATE_SUCCESSOR_KEY_FILE` (default
`~/Backup/proton-update-successor-key.pem`) and verifies the result against
`src/update-signing-successor.pub`. Commit both the feed and signature.

To revoke the primary key, add its hex to `revoked_keys`, sign, commit and
push. Builds at `3.28.0` or later then reject manifests signed by it, even
though the key remains in `UPDATE_PUBLIC_KEYS`, so a stolen primary key
cannot sign an update that users install.

## Reproducible builds

`build.sh` honours `SOURCE_DATE_EPOCH` for the AppStream release date. CI
builds with the source commit's timestamp, so the same commit produces the
same bytes. Prove it locally with:

```sh
./scripts/repro-check.sh
```

The script builds twice with the same `SOURCE_DATE_EPOCH` and fails unless the
two AppImages are byte-identical.

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

## Remaining limits

The successor key is now the root of trust for key revocation. Its private key
must stay offline; if it is compromised, no shipped build can revoke it
without a manual recovery release. The feed lives on `main`, so an outage or
deletion fails closed rather than accepting updates. Release-tag revocation is
still signed by the release key and protects against a bad release, not a
stolen key.
