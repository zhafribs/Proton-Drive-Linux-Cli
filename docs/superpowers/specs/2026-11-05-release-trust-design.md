# Release trust and packaging — design (Mission B1)

Date: 2026-11-05
Status: approved and implemented in 3.20.9
Mission: B (first slice; signed updates deferred)

## Intent

3.20.8 made the sync engine safe. The release pipeline is still the weak link:
CI publishes without running tests, the engine may not be glibc-pinned, the
updater downloads into a predictable path, and the AppStream metadata names the
wrong project. This slice makes the pipeline verify what it ships.

## Goals

1. CI runs the test suite and clippy before any build or publish.
2. A release cannot publish without a matching CHANGELOG section and correct
   metadata versions.
3. The GUI, engine and AppRun all build against the 2.28 glibc floor; a missing
   engine or a higher symbol version fails the build.
4. Downloaded build tools are pinned and checksum-verified.
5. The updater uses a private download directory, refuses untrusted hosts and
   oversized files, and does not follow a planted symlink.
6. The AppStream/desktop metadata identifies this project correctly and gets
   its version from `GUI_VERSION` at build time.

## Non-goals

- Signed releases. This needs a key policy (minisign key in a secret vs.
  Sigstore keyless) and a user decision; it is Mission B2.
- Pinning GitHub Actions to commit SHAs and a full `cargo deny` policy.
- New user-facing features.
- Formatting the whole repository with rustfmt (it is not fmt-clean today).

## Design

### 1. CI verification (`.github/workflows/release.yml`)

Two jobs, because PR code must never run with a write token:

- `verify` — top-level `contents: read`, checkout with
  `persist-credentials: false`; installs the pinned Rust 1.99.1 toolchain plus
  clippy, `binutils`, fonts and the GUI system libraries; runs
  `cargo test --workspace --all-targets`,
  `cargo clippy --release --workspace --all-targets -- -D warnings`,
  `bash -n`, and `scripts/test-check-release.sh`.
- `release` — `needs: verify`,
  `if: github.event_name != 'pull_request'`, job-level `contents: write`.
  Only this trusted job builds, commits an updated `upstream.lock`, publishes
  and cleans old releases.

`fetch-depth: 0` and `git fetch --tags --force` give the release gate the tags
it needs. A failed verify stops the job before the lock commit, build, or
publish. The toolchain is pinned by `rust-toolchain.toml`.

### 2. Release gate (`scripts/check-release.sh`)

A small script run after `build.sh` and before publish:

- `GUI_VERSION` exists and `CHANGELOG.md` has `## <GUI_VERSION> — `.
- The built AppDir's metainfo has `<release version="<GUI_VERSION>"`.
- The built desktop file has `X-AppImage-Version=<GUI_VERSION>`.
- `usr/bin/proton-engine` exists and is executable.
- `dist/SHA256SUMS` names the AppImage and the hash verifies.

The workflow calls it with the AppDir path, so a stale or generic changelog
stub fails the release instead of publishing.

### 3. glibc-pinned engine (`build.sh`)

- Make the cross target explicit in `build_one`: `CROSS_TARGET` is
  `x86_64-unknown-linux-gnu.2.28` when zigbuild and zig are available.
- Pass it to `build_gui`, `build_apprun`, and `make_appdir` (no more local
  `target` leaking out of scope).
- Build `proton-engine` with `cargo zigbuild --release -p proton-engine
  --target "$CROSS_TARGET"`.
- After installing each binary, run `check_glibc_floor`:
  `readelf --version-info` must show no symbol newer than `GLIBC_2.28`.
  The check is enforced when a cross target was used (CI); a local native
  build warns instead.
- `die` if any of the three binaries is missing from the AppDir.
- Fix the usage header ("both architectures" is stale).

### 4. Pinned build tools

- `build.sh` downloads appimagetool **1.9.1** instead of `continuous`, verifies
  SHA-256 `ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0`
  every run (cached or fresh), and refuses a mismatch.
- appimagetool normally pulls the AppImage type2 runtime from the moving
  `continuous` tag. `build.sh` now fetches type2-runtime **20251108**, verifies
  SHA-256 `2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d`,
  and passes it with `--runtime-file`. The runtime is the first code that runs
  when a user launches the AppImage, so it is part of the trust chain.
- The workflow verifies the Zig 0.15.2 tarball against
  `02aa270f183da276e5b5920b1dac44a63f1a49e55050ebde3aecc9eb82f93239`,
  checks `zig version` even on a cache hit, and uses an exact cache key.
- `cargo-zigbuild` stays pinned by version; crates.io verifies its checksum.
- The verify job runs the gate's fixture suite (`scripts/test-check-release.sh`,
  8 cases, including the git-backed version-bump guard) so a gate that always
  returns success cannot pass.

### 5. Updater hardening (`gui/src/release.rs`)

- `download_update` creates a private directory
  `<base>/.proton-update-<pid>-<nanos>/` with mode 0700 and downloads
  `proton-update-<asset>` inside it. `download()` uses `create_new`, so a
  pre-created symlink is refused, and caps the body at 256 MiB.
- Asset URLs must be `https://github.com/` or
  `https://objects.githubusercontent.com/`; the API digest must be 64 hex
  characters or the update is refused.
- `install_target` returns the running AppImage only when its parent directory
  is writable; otherwise no target is offered instead of failing mid-swap.
- The smoke check accepts an empty sync path (a fresh install with no folder
  still updates), while still requiring an absolute path when one is set.
- Update-recovery markers are validated before any rename/delete: `new` must
  equal the current `APPIMAGE`, `old` must be its `.old` sibling, and both must
  live in the same directory. A tampered `update-pending.json` is ignored.
- Tests cover trusted/untrusted URLs, digest shape, marker validation, and a
  read-only install directory.

### 6. Metadata

- Rename the desktop/metainfo files and ids from `zhafrisbs` to `zhafribs`.
- Developer is this project, not Proton AG:
  `id="io.github.zhafribs"`, name `Zhafri Syazwi`; homepage and VCS point at
  the GitHub repository.
- Source desktop/metainfo use `@GUI_VERSION@` and `@RELEASE_DATE@`; `build.sh`
  substitutes them into the AppDir, so the metadata always matches the build.
- `appstreamcli validate --no-net --strict` runs on the built metainfo. It is
  fatal because the metadata identity is part of the release gate; `--no-net`
  keeps it independent of GitHub/Proton reachability. The earlier
  `validate-tree` advisory behavior is replaced deliberately.

### 7. Deferred to B2 (needs a decision)

- **Signed updates:** publish a detached minisign/cosign signature for the
  AppImage and verify it in `release.rs` before executing. Options:
  (a) minisign keypair, private key in a GitHub secret, public key embedded;
  (b) Sigstore keyless attestations. Option (a) is simpler and offline; it
  needs a key backup and rotation plan.
- Actions pinned by commit SHA and `cargo deny`/`cargo audit` in CI.

## Risks

- **Pinned tool checksum:** if appimagetool 1.9.1 is unavailable, the build
  fails rather than silently using a moving binary. Intentional.
- **glibc check:** a local native build may warn; CI enforces the floor.
- **Metadata rename:** the desktop file id changes, so an existing menu entry
  may be replaced on the next install. The AppImage integration updates it.
- **CI time:** tests add ~1 minute; the cache keeps it bounded.

## Verification

- `cargo test --workspace` and release-profile clippy on the dev machine.
- `bash -n build.sh scripts/check-release.sh`.
- `scripts/test-check-release.sh` (8 fixtures).
- A local `./build.sh x86_64` run if zig/cargo-zigbuild are available;
  otherwise CI is the packaging proof. A native local build warns that it is
  not glibc-pinned and must not be published.
- `gh run watch` the first release after the change.
