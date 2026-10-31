# Rust port of the sync engine

Status: **Phase 1 landed** on branch `rust-engine`.
`main` stays untouched; nothing here is pushed to GitHub.

## Goal

Replace `scripts/proton-sync.py` and its bundled CPython runtime with a Rust
engine, so the AppImage ships three Rust binaries: our GUI, our engine, and
Proton's official `proton-drive` CLI.

## Non-goals

- Do **not** reimplement Proton's protocol or replace the official CLI. The
  CLI stays a subprocess; that boundary is an account-safety decision.
- Do **not** change user-visible behaviour, the updater, the installed
  AppImage filename, or the systemd unit path.

## Invariants (byte-compatible during the transition)

Both engines must keep producing the same:

- journal verbs and bodies (`UPLOAD`, `DOWNLOAD`, `MKDIR`, `TRASH`,
  `CONFLICT`, including `DOWNLOAD dir  X/ (whole subtree)`);
- `health.json` fields and meanings;
- `state.json` and `remote-snapshot.json` shapes;
- config keys and environment variables;
- systemd unit invocation (`AppImage __sync-engine`) and lock files.

The GUI parses the journal and health files directly, so a mismatch breaks
Activity/status even if syncing itself works.

## Layout

```
Cargo.toml              workspace (release profile lives here)
crates/core/            shared schema: conf, health, state, snapshot, time
gui/                    existing eframe GUI, depends on proton-core
crates/engine/          (phase 2) Rust engine
scripts/proton-sync.py  reference implementation + fallback
tests/test_sync_*.py    parity reference for engine behaviour
```

## Phases

### Phase 1 - shared schema (done)

- Cargo workspace; `proton-core` owns conf/health/state/snapshot types.
- GUI reads the same types instead of re-declaring them.
- Build output intentionally stays at `gui/target` (`.cargo/config.toml`),
  because `build.sh` and the release workflow cache/probe that path.
- Verification: `cargo test -p proton-core` (15), `cargo test -p
  proton-drive-gui` (64), release build + `--smoke` ok.

### Phase 2 - engine skeleton and parity harness

- `crates/engine`: CLI wrapper, local scan, remote scan, planner, state.
- `--dry-run` mode that prints the plan without acting.
- Harness compares Rust vs Python plans on synthetic trees covering:
  new/deleted/renamed files, both-sides changes, conflict naming, deletes
  on/off, selective sync, excludes, empty folders, symlinks, Unicode names.
- No cutover; no daemon loop yet.

### Phase 3 - loop

- Port inotify, interval scheduling, peek/audit and fast-start cache behind
  `--engine=rust`; keep Python as the default.
- Shadow-run both engines on a test account and diff plans/logs/health for at
  least a week, including restarts and a lost session.

### Phase 4 - push listener (done in 3.16.86)

- Live event stream via the official CLI's Secret Service session (`zbus`),
  HTTP (`ureq`) and the CLI's shared `events.json` cursor. Events re-list only
  the changed folder. The fingerprint watcher and interval remain fallbacks.

### Phase 5 - cutover (done)

- Systemd unit now sets `PROTON_SYNC_ENGINE=rust` by default; `PROTON_SYNC_ENGINE=python`
  remains available as a fallback. AppRun prefers the Rust engine when present.
- The Rust engine is built and shipped in the AppImage alongside the Python fallback.
- Bundled CPython/jeepney remain for now as a safety fallback; they can be removed in a future cleanup release once the Rust engine has been shadow-run in production.

## Cancelling the project

From the main checkout:

```sh
git worktree remove ~/.local/share/opencode/worktree/60e02c/rust-engine
git branch -D rust-engine
```

No `main` commit, push, release, or installed AppImage is touched by this
branch.
