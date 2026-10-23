# Fast-Start Sync Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Proton-Drive-Linux-Cli start from its cached metadata immediately, audit the remote tree in small background chunks, hold unsafe actions until the audit is trusted, and expose measurable scan/transfer progress in the GUI.

**Architecture:** Keep the official CLI and the existing Python engine. Add pure policy helpers plus a chunked audit state machine to `scripts/proton-sync.py`; the GUI reads additive health fields. No new auth path and no new runtime dependency. Phase B is only a future seam.

**Tech Stack:** Python 3 standard library, existing `proton-drive` CLI, Rust + egui GUI, JSON health/snapshot files, systemd user service.

**Spec:** `docs/superpowers/specs/2026-10-24-fast-start-sync-design.md`

## Global Constraints

- Fast start is on by default; `PROTON_SYNC_FAST_START=0` restores the old behavior.
- Deletes are held while `audit_pending` is true.
- Existing-file edits are held while `audit_pending` is true; new files and new folders may upload.
- After an audit, only the side that changed wins; if both changed, keep both as conflict copies. Never use timestamps alone to overwrite.
- Scan workers: Auto starts at the configured value (12 on the current install), capped at 16; memory takes priority over the 20% first-scan target.
- Transfer concurrency: default 4, allowed 1–10; 10 is experimental until benchmarked.
- No new authentication mechanism, no permanent deletes, no new Python/third-party dependency.
- Health JSON changes must be additive; old GUI/engine combinations must not blank the status.
- Audit chunks target roughly 10 seconds; the app-wide CLI lock is released between chunks.
- Do not implement phase B in this plan.

## Review Focus

1. **Corrupt or missing snapshot with non-empty state** — the app must not treat every path as new or claim a fast start; it must fall back to a full scan.
2. **Local edit of an existing remote file during audit** — the edit must be held, then reconciled; both-side changes must produce conflict copies.
3. **Remote delete while the cache is stale** — deletes must stay held until the audit is trusted; no remote data loss.
4. **Audit failure or partial chunk** — the GUI must not say “up to date”; deletes stay held and the error is visible.
5. **Transfer concurrency 10** — rate-limit/session-refresh failures must back off without losing the keyring session or corrupting state.

---

### Task 1: Pure policy helpers and a test harness

**Files:**
- Modify: `scripts/proton-sync.py` (add functions near the planning/snapshot helpers)
- Create: `tests/test_sync_policy.py`
- Create: `tests/__init__.py` (empty)

**Interfaces:**
- Produces:
  ```python
  def snapshot_usable(snapshot: dict | None, state: dict) -> tuple[bool, str]
  def hold_actions_for_audit(actions: list[tuple], remote: dict, state: dict) -> tuple[list[tuple], int]
  def group_transfer_batches(actions: list[tuple]) -> list[dict[str, list[str]]]
  def adaptive_worker_limit(configured: int, available_mb: int | None) -> int
  ```
- `actions` are the existing tuples `(kind, rel, payload)`. `kind` is one of `up`, `dl`, `conflict`, `trash_remote`, `trash_local`, `mv_remote`, `mv_local`.
- `group_transfer_batches` returns one batch per parent for uploads and downloads, preserving parent order. Each batch is `{"parent": str, "uploads": list[str], "downloads": list[str]}` (never both kinds in one batch).

- [ ] **Step 1: Write the failing tests**

Add `tests/test_sync_policy.py` using `unittest` and `importlib` to load the hyphenated script:

```python
import importlib.util
import pathlib
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("proton_sync", ROOT / "scripts/proton-sync.py")
proton_sync = importlib.util.module_from_spec(spec)
spec.loader.exec_module(proton_sync)
```

Tests:

- `snapshot_usable(None, {}) == (False, "no snapshot")`
- a valid snapshot with a non-empty state is usable
- a snapshot with entries and empty state is not usable
- `hold_actions_for_audit` holds `trash_remote`, `trash_local`, `mv_remote`, `mv_local`, `conflict`
- `hold_actions_for_audit` holds an `up` for a file whose state has `r_mtime`
- `hold_actions_for_audit` allows an `up` for a file with no remote state
- `hold_actions_for_audit` allows `dl` and directory `up`
- `group_transfer_batches` groups two uploads in the same parent into one batch, and two parents into two batches
- `adaptive_worker_limit(12, 1024) == 7` and `adaptive_worker_limit(12, None) == 12`

- [ ] **Step 2: Run tests to verify they fail**

Run:
```bash
python3 -m unittest discover -s tests -v
```
Expected: import/attribute errors for the missing functions.

- [ ] **Step 3: Implement the helpers in `scripts/proton-sync.py`**

- `snapshot_usable`: require a dict with `sig`, `tree`, and `fp`; require `state` to be a dict with at least one record when `tree` is non-empty. Return a short reason string.
- `hold_actions_for_audit`: iterate actions; hold deletes, moves, conflicts, and file `up` actions whose `state[rel]` contains `r_mtime`; allow `dl`, directory `up`, and file `up` with no remote state. Return `(allowed, held_count)`.
- `group_transfer_batches`: group `up` and `dl` actions by `(kind, parent)`; return one dict per parent, with uploads/downloads lists.
- `adaptive_worker_limit`: reuse the current RAM budget formula (`CLI_PROCESS_MB`, `MEM_RESERVE_MB`) and cap by `configured`; return configured count when available memory is unknown.

- [ ] **Step 4: Run tests to verify they pass**

Run:
```bash
python3 -m unittest discover -s tests -v
```
Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add scripts/proton-sync.py tests/__init__.py tests/test_sync_policy.py
git commit -m "test: add sync policy helpers for fast start"
```

---

### Task 2: Health fields and phase timing

**Files:**
- Modify: `scripts/proton-sync.py` (`write_health` calls in `run_once`/`loop_forever`)
- Modify: `gui/src/health.rs`
- Test: `gui/src/health.rs` (`#[cfg(test)]` module)

**Interfaces:**
- Add optional health fields: `cache_age`, `audit_pending`, `audit_done`, `audit_total`, `audit_phase`, `audit_seconds`, `phase`, `cycle_reason`, `effective_workers`, `full_scan_seconds`, `transfer_active`, `transfer_done`, `transfer_total`, `transfer_bytes`.
- GUI `Health` gains optional fields with `#[serde(default)]`:
  ```rust
  pub cache_age: Option<u64>,
  pub audit_pending: Option<bool>,
  pub audit_done: Option<u64>,
  pub audit_total: Option<u64>,
  pub audit_phase: Option<String>,
  pub audit_seconds: Option<f64>,
  pub phase: Option<String>,
  pub cycle_reason: Option<String>,
  pub effective_workers: Option<u64>,
  pub transfer_active: Option<bool>,
  pub transfer_done: Option<u64>,
  pub transfer_total: Option<u64>,
  pub transfer_bytes: Option<u64>,
  ```
- Add a small engine helper:
  ```python
  def health_phase(name: str, started: float, **extra) -> dict
  ```
  returning `phase`, `phase_seconds`, and extras. Use it at scan/plan/execute/audit boundaries.

- [ ] **Step 1: Write the failing GUI test**

Add a test in `gui/src/health.rs` parsing a fast-start health JSON with:
```json
{
  "status": "ok",
  "mode": "fast",
  "cache_age": 120,
  "audit_pending": true,
  "audit_done": 420,
  "audit_total": 1230,
  "phase": "audit",
  "cycle_reason": "startup",
  "effective_workers": 12
}
```
Assert the fields parse with their exact values. The exact fast-start wording in `synced_line` is pinned by Task 6.

- [ ] **Step 2: Run the GUI test to verify it fails**

Run:
```bash
cargo test --manifest-path gui/Cargo.toml health::tests -v
```
Expected: compile error for missing fields.

- [ ] **Step 3: Implement the health fields and engine phase helper**

Add the fields to `Health`; make `synced_line` return the current line for now (the fast-start wording is Task 6). Add `health_phase` and write it at the start/end of remote scan, local scan, plan, execute, and audit chunks. Include `cycle_reason` in each terminal health write.

- [ ] **Step 4: Run both test suites**

```bash
python3 -m unittest discover -s tests -v
cargo test --manifest-path gui/Cargo.toml -v
```
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add scripts/proton-sync.py gui/src/health.rs
git commit -m "feat: add fast-start health telemetry"
```

---

### Task 3: Fast-start view and held actions

**Files:**
- Modify: `scripts/proton-sync.py` (`loop_forever`, `run_once`, globals/config)
- Modify: `tests/test_sync_policy.py` if a helper contract changes
- Test: `tests/test_sync_policy.py` (hold decisions)

**Interfaces:**
- Add config constants near the existing knobs:
  ```python
  FAST_START = cfg("PROTON_SYNC_FAST_START", "1") == "1"
  AUTO_WORKERS = cfg("PROTON_SYNC_AUTO_WORKERS", "1") == "1"
  TRANSFERS = max(1, min(10, int(cfg("PROTON_SYNC_TRANSFERS", "4"))))
  AUDIT_EVERY = max(0, int(cfg("PROTON_SYNC_AUDIT_EVERY", "1800")))
  AUDIT_CHUNK_FOLDERS = max(1, int(cfg("PROTON_SYNC_AUDIT_CHUNK", "40")))
  ```
- Add globals:
  ```python
  FAST_VIEW = None
  AUDIT_PENDING = False
  AUDIT_STATE = None
  ```
- Add:
  ```python
  def start_fast_view() -> bool
  def fast_cycle_remote(push_pending) -> tuple[dict, str]
  ```
  `start_fast_view` loads snapshot/state, calls `snapshot_usable`, sets `FAST_VIEW`/`AUDIT_PENDING`, writes fast health, and returns whether fast start was used.
- `run_once` with `FAST_VIEW`:
  - use `dict(FAST_VIEW)` as the remote base,
  - apply push scopes when available,
  - never call `remote_fingerprint` or `scan_remote`,
  - set `mode="fast"` and `cycle_reason` (`startup`, `local`, `push`, `manual`),
  - call `hold_actions_for_audit` before `execute`, add the held count to `holds`,
  - update `FAST_VIEW` after successful uploads/moves using the existing parent re-list logic.

- [ ] **Step 1: Write the failing hold test**

Extend `tests/test_sync_policy.py` with a state containing `r_mtime` for an existing remote file and verify:
- a local edit (`up` file) is held while audit is pending,
- a new file (`up` file with no state) is allowed,
- a delete is held,
- a download is allowed.

- [ ] **Step 2: Run the test to verify it fails**

```bash
python3 -m unittest discover -s tests -v
```
Expected: new hold cases fail against the current helper contract if not already covered.

- [ ] **Step 3: Implement fast view and action holding**

Wire `start_fast_view()` into `loop_forever` before the first `run_once`. In `run_once`, branch on `FAST_VIEW is not None`; do not perform a full walk. Apply `hold_actions_for_audit` and pass held paths to health (`holds`, `audit_pending`). Keep `PROTON_SYNC_FAST_START=0` on the old path.

- [ ] **Step 4: Verify with tests and a dry run**

```bash
python3 -m unittest discover -s tests -v
python3 scripts/proton-sync.py --once --dry-run --local /tmp/fast-start-smoke --remote /my-files
```
Expected: tests pass; dry run shows no traceback and reports `mode=fast` when a snapshot exists, or `mode=full` when none exists. The manual fast-start acceptance is completed in Task 7.

- [ ] **Step 5: Commit**

```bash
git add scripts/proton-sync.py tests/test_sync_policy.py
git commit -m "feat: add fast-start cached cycles and held actions"
```

---

### Task 4: Chunked background audit

**Files:**
- Modify: `scripts/proton-sync.py` (`_list_folder`, `scan_remote`, new audit state, `loop_forever`, `run_once`)
- Test: `tests/test_sync_audit.py`

**Interfaces:**
- Extend the first-ever scan with progress:
  ```python
  def scan_remote(start_rel=None, progress=None):
      # progress(done: int, total: int) after each BFS round; total is the
      # number of folders discovered so far, not the final unknown count.
  ```
  `run_once` in full mode calls `write_health(phase="full-scan", audit_done=done,
  audit_total=total, audit_pending=FAST_START)` so the GUI can show progress
  even before a snapshot exists.
- Add:
  ```python
  class AuditState:
      def __init__(self, root_rpath: str): ...
      def next_chunk(self, list_folder, max_folders: int = AUDIT_CHUNK_FOLDERS) -> int: ...
      @property
      def complete(self) -> bool: ...
      def health(self) -> dict: ...
  ```
  Fields: `pending: list[tuple[str, str]]` (relative path, remote path), `tree: dict`, `rel_of: dict`, `visited: set`, `done: int`, `total: int`, `started: float`, `touched: set`.
- Add:
  ```python
  def scan_remote_chunk(state: AuditState, list_folder, max_folders: int) -> int
  def audit_tick() -> bool
  def finish_audit_if_ready() -> bool
  def merge_touched_scopes(state: AuditState) -> None
  ```
- `AuditState.next_chunk` uses breadth-first pending folders; it lists at most `max_folders`, appends child folders, and increments `done`.
- `audit_tick` acquires `cli_lock()`, calls `probe_session()`, lists one chunk with `effective_workers()`, writes progress health, and returns whether the audit is complete.
- `finish_audit_if_ready` re-lists `state.touched` scopes, swaps `FAST_VIEW`, saves the snapshot, clears `AUDIT_PENDING`, and writes `audit_pending=false`.

- [ ] **Step 1: Write the failing audit tests**

Create `tests/test_sync_audit.py` with a fake lister:
```python
def fake_list(rpath):
    return {
        "/my-files": [("a", "/my-files/a", {"uid": "1", "type": "dir", "mtime": "", "size": 0})],
        "/my-files/a": [("b", "/my-files/a/b.txt", {"uid": "2", "type": "file", "mtime": "2026-01-01T00:00:00Z", "size": 3})],
    }[rpath]
```
Tests:
- two calls with `max_folders=1` complete the tree and report `done == 2`
- `complete` is false after the first chunk
- a permanent lister error leaves `AUDIT_PENDING` true and records an error
- a touched scope is re-listed before the snapshot swap

- [ ] **Step 2: Run to verify failure**

```bash
python3 -m unittest tests.test_sync_audit -v
```
Expected: import/attribute errors.

- [ ] **Step 3: Implement AuditState and `scan_remote_chunk`**

Use the existing `_list_folder` and `_children_nodes`; keep the same BFS ordering and CLI lock behavior. Do not reuse the long-lived `scan_remote` lock. Ensure no second audit worker starts while one is active.

- [ ] **Step 4: Wire audit ticks into the loop**

In `loop_forever`, after fast view is started:
- if `AUDIT_PENDING` and not complete, call `audit_tick()` when the loop is idle / after a cycle,
- if complete, call `finish_audit_if_ready()`,
- health reports `audit_done/audit_total/audit_seconds`,
- on failure keep the cached view, set `audit_pending=true`, and expose the error.

- [ ] **Step 5: Verify**

```bash
python3 -m unittest discover -s tests -v
```
Expected: all policy/audit tests pass. Then run a real service restart with a disposable local folder and confirm health progresses from 0 to the folder count for both the first-ever full scan and the background audit.

- [ ] **Step 6: Commit**

```bash
git add scripts/proton-sync.py tests/test_sync_audit.py
git commit -m "feat: add chunked background remote audit"
```

---

### Task 5: Adaptive scan workers and transfer concurrency

**Files:**
- Modify: `scripts/proton-sync.py` (`effective_workers`, `execute`, transfer functions, health)
- Modify: `tests/test_sync_policy.py`
- Test: `tests/test_sync_policy.py` (scheduling/limits)

**Interfaces:**
- `effective_workers()` keeps the RAM cap and uses:
  ```python
  def adaptive_worker_limit(configured: int, available_mb: int | None) -> int
  def adjust_worker_limit(current: int, saw_rate_limit: bool, clean_chunks: int) -> int
  ```
  `adjust_worker_limit` returns `max(1, current - 2)` on a rate limit;
  otherwise after 3 clean chunks it returns `min(16, current + 1)`;
  otherwise it returns `current`.
- Add:
  ```python
  def group_transfer_batches(actions: list[tuple]) -> list[dict[str, list[str]]]
  def run_transfer_batches(batches, upload_fn, download_fn, max_workers: int) -> tuple[set[str], set[str]]
  ```
- `execute` uses `run_transfer_batches` for independent parent batches. Each batch keeps the existing one-parent CLI call.
- The parent thread acquires `cli_lock()` once for the transfer group, calls `probe_session()`, then submits batches with the worker-thread `locked=False` flag. This mirrors `scan_remote()` and keeps other app processes from refreshing the keyring during the group.
- Change the transfer helpers to accept that flag:
  ```python
  def ensure_remote_dir(rel, tree, created, locked=True)
  def upload_batch(parent, rels, tree, created, locked=True)
  def download_files(parent, rels, locked=True)
  def cli_transfer(args, locked=True)
  ```
- Health adds `effective_workers`, `transfer_active`, `transfer_done`, `transfer_total`, `transfer_bytes`.

- [ ] **Step 1: Write failing tests**

- `adaptive_worker_limit(12, 1024) == 7`, `adaptive_worker_limit(12, None) == 12`
- 10 batches with `max_workers=4` never exceed 4 concurrent calls (fake function records active/max)
- two uploads in one parent become one batch; two parents become two batches
- a 429-like error reduces the next worker count by 2

- [ ] **Step 2: Run to verify failure**

```bash
python3 -m unittest discover -s tests -v
```
Expected: missing functions/assertions fail.

- [ ] **Step 3: Implement worker and transfer scheduling**

Use a `ThreadPoolExecutor(max_workers=min(TRANSFERS, len(batches)))`; do not create more than 10. Ensure each CLI call still uses the app-wide session lock exactly as designed. Keep the existing per-parent `upload_batch`/`download_files` calls; do not split a parent batch.

- [ ] **Step 4: Verify**

```bash
python3 -m unittest discover -s tests -v
```
Expected: pass. Manually benchmark 2/4/6/10 concurrent parent batches against a disposable folder and record results; do not change the default above 4 without evidence.

- [ ] **Step 5: Commit**

```bash
git add scripts/proton-sync.py tests/test_sync_policy.py
git commit -m "feat: add adaptive scans and parallel transfer batches"
```

---

### Task 6: GUI performance status and settings

**Files:**
- Modify: `gui/src/health.rs` (`synced_line`, `hero_sub`, tests)
- Modify: `gui/src/ui/settings.rs` (Draft, load, `do_save`, Performance card)
- Modify: `gui/src/ui/home.rs` (fast-start/audit status)
- Modify: `gui/src/service.rs` (request full check / clear snapshot cache)
- Modify: `scripts/proton-sync.py` (audit-request/clear-cache file support)
- Test: `gui/src/health.rs`, `gui/src/ui/settings.rs`

**Interfaces:**
- `Health` fields from Task 2.
- `synced_line`:
  - `audit_pending && audit_total > 0` -> `"Ready from cache · checking {done} / {total} folders"`
  - `audit_pending` with no total -> `"Ready from cache · background check running"`
  - `cache_age` present -> `"Cached view · {age}s old"`
  - otherwise existing behavior.
- Settings `Draft` adds:
  ```rust
  pub fast_start: bool,
  pub auto_workers: bool,
  pub transfers: i32,
  ```
  Defaults `true`, `true`, `4`; load from `PROTON_SYNC_FAST_START`, `PROTON_SYNC_AUTO_WORKERS`, `PROTON_SYNC_TRANSFERS`; save those keys.
- Add helpers:
  ```rust
  pub fn parse_transfers(raw: &str) -> i32; // 1..=10, default 4
  pub fn performance_values(fast_start: bool, auto_workers: bool, transfers: i32) -> BTreeMap<String, String>;
  ```
- Performance card:
  - Fast start checkbox with explanation.
  - Auto concurrency checkbox + manual workers DragValue 1..=16.
  - Transfer concurrency DragValue 1..=10.
  - “Run full check now” calls `service::request_full_check(&app.paths)`.
  - “Clear snapshot cache” calls `service::clear_snapshot_cache(&app.paths)` behind a confirmation.
- Add exact service functions:
  ```rust
  pub fn request_full_check(paths: &crate::paths::Paths) -> std::io::Result<()>
  pub fn clear_snapshot_cache(paths: &crate::paths::Paths) -> std::io::Result<()>
  ```
  `request_full_check` creates `state_dir/audit-request`; `clear_snapshot_cache` removes `snapshot_file` only.
- Home status shows the audit line and “deletes/existing edits held” while pending.

- [ ] **Step 1: Write failing Rust tests**

- Parse the fast-start JSON and assert `synced_line` contains `checking 420 / 1230`.
- Parse a normal health JSON and assert the old “Last checked” line is unchanged.
- `parse_transfers("4") == 4`, `parse_transfers("10") == 10`, `parse_transfers("99") == 10`, and invalid text falls back to 4.
- `performance_values(true, true, 4)` returns `PROTON_SYNC_FAST_START=1`,
  `PROTON_SYNC_AUTO_WORKERS=1`, and `PROTON_SYNC_TRANSFERS=4`.
- `service::request_full_check` with a temp `Paths` creates `state_dir/audit-request`; `service::clear_snapshot_cache` removes only `snapshot_file`.

- [ ] **Step 2: Run to verify failure**

```bash
cargo test --manifest-path gui/Cargo.toml -v
```
Expected: compile/test failures.

- [ ] **Step 3: Implement health copy, card, and file requests**

Keep the existing visual style and `widgets::card`. Do not add a second settings page; put Performance inside Settings, and keep raw worker count under Advanced. Add the engine file checks:
  - `AUDIT_REQUEST_FILE = STATE_DIR / "audit-request"`: presence forces a new audit and is removed after it starts.
  - clear-cache removes `SNAPSHOT_FILE` only; state/local files stay.

- [ ] **Step 4: Verify**

```bash
cargo test --manifest-path gui/Cargo.toml -v
python3 -m unittest discover -s tests -v
```
Expected: pass.

- [ ] **Step 5: Commit**

```bash
git add gui/src/health.rs gui/src/ui/settings.rs gui/src/ui/home.rs gui/src/service.rs scripts/proton-sync.py
git commit -m "feat: show fast-start progress and performance settings"
```

---

### Task 7: Benchmark, acceptance, and rollback docs

**Files:**
- Create: `docs/performance/fast-start-sync.md`
- Modify: `README.md` (performance section)
- Modify: `CHANGELOG.md`
- Test: `tests/test_sync_policy.py` (regression guard)
- Manual: service restart with the existing 1,230-entry dataset

**Interfaces:**
- No new runtime functions. This task pins the measured values and rollback.

- [ ] **Step 1: Benchmark the baseline and the new build**

Use a disposable local/remote folder for functional checks, and the existing account for read-only scan timing:

```bash
# Baseline: PROTON_SYNC_FAST_START=0
time systemctl --user restart proton-sync.service
# read health until status=ok
# New: PROTON_SYNC_FAST_START=1
```

Record:
- warm start to `status=ok`
- cold first scan time + folder count
- local edit to cloud
- push change to local
- transfer 2/4/6/10 results
- peak RSS

- [ ] **Step 2: Run the acceptance matrix**

- valid snapshot warm start
- missing/corrupt snapshot fallback
- local new file during audit (uploads)
- local edit of existing remote file during audit (held, then reconciled)
- remote edit during audit (conflict after audit)
- local delete during audit (held)
- remote delete during audit (held)
- audit failure (deletes stay held, error visible)
- 10-way transfer stability and backoff
- restart during audit
- `PROTON_SYNC_FAST_START=0` rollback

- [ ] **Step 3: Write the performance/rollback doc**

Include the measured table, the exact commands used, the default values, and the one-key rollback. Note that first-ever scan still requires the CLI walk and B is the future fix if needed.

- [ ] **Step 4: Run all tests**

```bash
python3 -m unittest discover -s tests -v
cargo test --manifest-path gui/Cargo.toml -v
```

- [ ] **Step 5: Commit**

```bash
git add docs/performance/fast-start-sync.md README.md CHANGELOG.md tests/test_sync_policy.py
git commit -m "docs: record fast-start benchmarks and rollback"
```

---

## Self-review checklist

- [ ] Spec goal “fast warm start” → Tasks 2–4.
- [ ] Spec goal “fast first scan” → Task 4 progress + Task 5 adaptive workers; measured in Task 7.
- [ ] Change detection unchanged except startup/audit → Task 3/4.
- [ ] Hold deletes and existing edits → Task 3/4.
- [ ] Conflict on both sides after audit → Task 3/4 + Task 7.
- [ ] Transfer 4 default / 10 max → Task 5.
- [ ] GUI performance card/diagnostics → Task 6.
- [ ] Rollback switch → Task 3/7.
- [ ] B only a seam, no B code → no task implements a persistent helper.
