# Signed updates — design (Mission B2)

Date: 2026-11-05
Status: implemented in 3.20.10
Depends on: Mission B1 (release trust and packaging)

## Intent

B1 made the updater refuse untrusted hosts, malformed digests, oversized
files and unsafe install paths. It still trusted GitHub release-write: an
attacker who could replace the AppImage and `SHA256SUMS` together could
deliver a malicious build with a matching hash. B2 adds an Ed25519 signature
over the checksum manifest, so the updater verifies authenticity offline
before executing anything.

## Threat model

- **Protected:** GitHub release/asset replacement, a compromised CDN, or a
  stolen `GITHUB_TOKEN` that cannot read the signing secret. The updater
  refuses an unsigned or tampered `SHA256SUMS`.
- **Not protected:** a compromised release workflow that can read
  `UPDATE_SIGNING_KEY`. That key is in GitHub Secrets and is available to the
  release job. If that threat matters, sign offline or move signing to a
  protected environment with manual approval.
- The key is not a substitute for the existing checks: size, SHA-256, staged
  `--smoke`, and immediate rollback stay in place.

## Key policy

- Algorithm: Ed25519 (OpenSSL `pkeyutl -sign -rawin`).
- Private key: `UPDATE_SIGNING_KEY` repository secret; offline backup at
  `~/Backup/proton-update-signing-key.pem` (mode 0600). Never commit it.
- Public key: `src/update-signing-key.pub` (tracked) and the raw 32-byte hex
  embedded in `gui/src/release.rs`. `check-release.sh` derives the hex from the
  PEM and fails if the two differ.
- Rotation: ship a release signed with the old key that embeds the new public
  key, then switch the secret. The stock scripts support the transition:
  ```bash
  # transitional release: old key signs, new key is embedded
  SIGN_RELEASE_VERIFY_KEY=old.pub UPDATE_SIGNING_KEY_FILE=old.pem ./scripts/sign-release.sh
  CHECK_RELEASE_PUBLIC_KEY=old.pub ALLOW_KEY_ROTATION=1 ./scripts/check-release.sh
  ```
  Keep the old private key until no supported release still trusts it. Users
  who skip the transitional release will reject post-rotation releases; the
  release notes must call that out. Also regenerate the hardcoded self-test
  signature in `gui/src/release.rs` with the new key and pass the two rotation
  environment variables through the workflow (or run the transitional release
  from a trusted local machine). Losing the only private key stops new
  signatures, but users can still download manually.

## Release assets

Every release now publishes:

- `ProtonDrive-CLI-<cli>-GUI-<gui>-x86_64.AppImage`
- `SHA256SUMS`
- `SHA256SUMS.sig` (Ed25519 signature over the exact `SHA256SUMS` bytes)

CI signs after `build.sh` and before the release gate:

```bash
UPDATE_SIGNING_KEY="${{ secrets.UPDATE_SIGNING_KEY }}" ./scripts/sign-release.sh
./scripts/check-release.sh
```

`check-release.sh` verifies the signature against the tracked public key and
checks that the updater's embedded hex matches it. It fails the release if the
signature is missing or invalid, so an unsigned manifest cannot be published.

## Updater flow

1. Resolve the AppImage, `SHA256SUMS`, and `SHA256SUMS.sig` URLs; all must be
   the exact repository release path or GitHub's asset CDN.
2. Refuse before any big download if the install directory is not private and
   writable.
3. Fetch the manifest and signature; verify the signature with the embedded
   key. An unsigned/invalid manifest is a hard error.
4. Take the AppImage SHA-256 from the signed manifest. If GitHub's API digest
   is present, it must match; a mismatch is an error.
5. Create a private 0700 directory, download with `create_new`, cap the body,
   verify the hash, stage, run `--smoke`, then swap with rollback.
6. Update-recovery markers are validated against the current AppImage before
   any rename/delete.

## Tests

- `verify_manifest_with`: a test key signs a manifest; a one-byte change and a
  malformed key fail.
- `download_plan`: trusted/untrusted URLs, digest shape, signed-manifest assets,
  install target.
- `check-release` fixtures: 12 cases including missing/tampered signature,
  wrong signing key, embedded-key drift, stale version, empty changelog and
  force-rebuild override.
- Real build test: `./build.sh`, `scripts/sign-release.sh`,
  `scripts/check-release.sh` on the 3.20.10 AppImage.
- CI regenerates `src/licenses/proton-drive-gui-THIRD-PARTY.txt` with
  `cargo-about` and fails if the committed file differs, so a new dependency
  cannot ship without its licence text.

## Residual risks

- The first signed release (3.20.10) is installed by every pre-3.20.10
  version, none of which verify signatures. From 3.20.10 onward every update
  is verified.
- The release workflow can read the signing secret; CI compromise remains a
  trusted-root risk. Offline signing or a protected environment is the next
  step if that threat matters.
- The signature protects the manifest and therefore the listed hashes; it does
  not make a compromised build trustworthy if the build itself was produced by
  a compromised workflow before signing.
