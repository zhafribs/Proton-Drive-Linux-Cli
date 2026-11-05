# Data-safety hardening — design

Date: 2026-11-05
Status: approved and implemented in 3.20.8
Mission: A (first of A/B/C)

## Intent

The engine can lose user data in three concrete paths. This change removes those
paths before any feature work. The app's promise is "deletes only go to trash,
conflicts keep both versions"; the code does not currently honour that promise.

## Goals

1. A conflict never destroys the local version and never uploads the wrong bytes.
2. An interrupted or short download never replaces a good cloud revision with a
   truncated local file.
3. A malformed or unexpectedly short Photos timeline never permanently deletes
   local photos, and local photo removals go to trash.
4. A corrupt `state.json` fails closed instead of silently disabling every guard.
5. Each fix has a focused test at its seam.

## Non-goals

- Rewriting the planner or adding a transactional journal.
- Content hashing for change detection (separate change).
- CLI lock re-entrancy, event-stream restart, GUI accessibility (missions B/C).
- New user-facing settings or options.

## Approaches considered

**A. Guard rails only.** Add size checks before trusting a download and an
empty-timeline check. Small, but conflicts still overwrite the local path, and
a short download still has a window where it is the local truth.

**B. Stage, verify, then commit (chosen).** Every download lands in a staging
directory beside the sync root, is size-verified, then renamed into place.
Conflicts copy the remote revision to the conflict name first and only then
upload the untouched local file. State reads are checked. Photos deletions are
guarded and use trash.

**C. Transactional engine.** Journal every action with crash recovery. Correct
long-term, far larger than this mission, and not needed to close the known
paths. Deferred.

## Design

### 1. Staged downloads (`execute.rs`, `firstsync.rs`, `filter.rs`)

Add one helper used by both download paths:

```rust
/// Download `rels` under `parent` into a staging dir, verify each file's
/// size, then rename each into place. Returns the rels that failed.
fn staged_download(
    cli: &Cli,
    remote_root: &str,
    local_root: &Path,
    parent: &str,
    rels: &[(String, u64)],
    retries: usize,
) -> BTreeSet<String>
```

- Staging root: `local_root/<parent>/.proton-sync-staging/<pid>-<seq>-<nanos>/`.
  The root is rejected if it is a symlink or not a directory; the batch
  directory is created with `create_dir` so a stale/planted name is never
  reused. Staging in the destination's own parent keeps rename on the same
  filesystem for nested mounts.
- One CLI call per batch, same flags (`-f remove -d merge`).
- For each rel: staged file must exist and `len() == expected size`, then the
  file is fsynced, `create_dir_all(final parent)` runs, and `fs::rename`
  replaces the final path. A short or missing file is left in staging
  (discarded afterwards) and the final path is untouched; the rel is returned
  as failed. If any rel is unsafe, the whole batch fails rather than reporting
  un-attempted files as downloaded.
- `Filter::skip_name` gains `^\.proton-sync-staging$` so a crash-left directory
  is never scanned or uploaded.
- `execute()`: deferred downloads carry `(rel, expected size)`; failures feed
  the existing `failed` set, successes feed `downloaded`.
- `firstsync.rs`: `Batch` carries `(rel, size)` pairs; its worker calls the same
  helper. Retries stay in the helper (same 2s×attempt backoff).

### 1b. Cloud-name validation (`filter.rs`, `scan.rs`, `firstsync.rs`, `execute.rs`)

A remote listing may only contribute plain path components. `valid_remote_name`
rejects empty, `.`, `..`, embedded `/` and NUL; both listing parsers skip such
entries. `safe_rel` re-checks every component in the staging and conflict
helpers as defence in depth. This blocks a malformed/malicious listing from
escaping the sync root via `Path::join`.

Residual risk stated explicitly: size verification catches truncation and
interrupted transfers, not a same-length corrupt payload. A content hash is out
of scope because the CLI listing exposes no remote hash; transport integrity is
the CLI's TLS layer.

### 2. Copy-first conflicts (`execute.rs`)

Replace the current download-into-canonical block with:

1. Build a collision-free conflict path
   (`<stem> (conflict <UTC>)[.ext]`, suffix ` 2`, ` 3`, … if it exists).
2. Download the remote revision into staging and verify its size.
3. Rename the verified staged file to the conflict path. The canonical local
   path is never written.
4. Only now upload the canonical local file.
5. Any failure marks the rel failed and skips the upload; the local file stays
   exactly as the user left it.

The conflict copy is a new local-only file and uploads on a later cycle, as it
does today.

### 3. Fail-closed state (`core/state.rs`, `loop_.rs`, `main.rs`)

- Add `read_checked(path) -> io::Result<Option<State>>`:
  - missing file → `Ok(None)`
  - unreadable / malformed → `Err(InvalidData)`
  - valid → `Ok(Some(state))`
- `read()` stays as the lenient status reader, implemented over a shared parser.
- Every planning path uses `read_checked`: `loop_::run_cycle` (both reads),
  the fast-edit path, and `main.rs --once`. An error aborts the cycle with a
  clear health error; no scan, transfer, or deletion runs on unknown history.
- `state::write` calls `sync_all()` on the file before the atomic rename and
  best-effort `sync_all()` on the parent directory after it.
- The loop logs a failed state write instead of discarding the result.

### 4. Photos deletion guards (`photos.rs`)

- `timeline()` returns an error when the CLI response is not a JSON array, or
  when a non-empty array yields zero readable entries (schema drift). It no
  longer turns "unreadable" into "empty". Names and uids are validated before
  they can reach a join or a `/photos/<uid>` path.
- In `sync()`, before any transfer: skipped entries, an empty timeline with a
  populated `state.synced`, or a timeline that loses more than half of a known
  library hold the **whole** photo cycle (no uploads, downloads, deletions, or
  state rebuild) and write a visible `Summary.error`. Holding only deletions
  while rebuilding `state.synced` from the short list would disarm the guard
  next cycle. The error names `photos-state.json`; a user who genuinely deleted
  the photos in Proton removes that file after confirming the cloud, which is
  the explicit reset path. Deletions past `MAX_DELETES` crawl, and their state
  entries are preserved so the next cycle does not re-download them as new.
- An empty photo folder with a populated `state.synced` holds the **whole**
  cycle, not just deletions: downloading a new cloud photo there would make
  the folder non-empty and defeat the guard next cycle.
- The post-cycle re-list may account for remote deletions this cycle executed,
  but it must still see every known photo that should remain
  (`known_remote - executed_trash`); otherwise the old state is kept. Local
  removals are not subtracted because those names were already absent from the
  initial timeline.
- `plan()` also refuses deletions when the remote listing does not look
  present, so a direct caller cannot bypass the guard.
- A hold is persisted in `photos-state.json` (`hold: true`). The GUI also
  writes `<photo-dir>/.proton-photos-hold` before any download while fewer
  than all known photos are local; the engine turns that marker into the same
  hold on its next pass. It is released only when every known photo still in
  the cloud is present locally, so a partial GUI download cannot make the
  folder non-empty and re-arm the guard. `repair_failed` and old state are kept
  while held.
- `remove_local` uses `trash_local_photo()`: a rename into
  `<photo-dir>/.proton-photos-trash/<name>` with a collision suffix. It is on
  the same filesystem as the photos, never `remove_file`.

### 5. Tests

- `execute`/staging unit tests with a fake CLI (`Cli::for_test`):
  - verified download lands at the final path;
  - wrong-size download leaves the old final file untouched and reports failure;
  - conflict keeps the local bytes canonical and stores remote bytes in a
    unique conflict file;
  - a failed conflict download does not upload.
- `core::state`: `read_checked` missing/malformed/valid; `write` still
  round-trips atomically.
- `photos`: non-array timeline is an error; empty remote with populated state
  holds; `remote_looks_present=false` clears deletions; trash helper keeps
  collisions.
- Existing 229 tests must stay green.

### 6. Release

- Bump `GUI_VERSION` and add a CHANGELOG entry describing the three safety
  fixes and the fail-closed state behaviour.
- No new dependencies.

## Risks and mitigations

- **Extra disk during download:** up to one batch (`MAX_BATCH`) per transfer
  worker; staging is cleaned in all paths and skipped by the scanner.
- **Rename across filesystems:** staging lives under the destination's parent
  (`<local_root>/<parent>/.proton-sync-staging`), so rename stays on the same
  device even for nested mounts.
- **Empty timeline is a real user action:** the cycle holds and reports; the
  message names the photo folder and `photos-state.json` for an explicit reset
  after the user confirms the cloud. Chosen over silent loss.
- **Behaviour change on corrupt state:** a corrupt file now stops sync instead
  of re-uploading; the error is visible in health and logs.

## Resolved review questions

1. `read_checked` in the fast-edit path aborts; a full walk is not a safe
   fallback because the planner would still lack trustworthy history.
2. A short photos timeline holds every transfer, not only deletions, and
   freezes the state rebuild.
3. Per-batch staging is enough; each file is individually verified, fsynced,
   and renamed, and every stage directory is fresh.
