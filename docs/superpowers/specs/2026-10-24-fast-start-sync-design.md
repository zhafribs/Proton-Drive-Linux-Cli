# Fast-start, event-first sync for Proton-Drive-Linux-Cli

**Status:** design draft for approval  
**Repo:** `zhafribs/Proton-Drive-Linux-Cli`  
**Scope:** A first; B later  
**Date:** 2026-10-24

## Intent

Make the existing CLI-backed app feel faster in four places without changing
its authentication or sync model:

1. startup
2. first scan after a restart
3. change detection
4. transfer throughput and GUI feedback

The sync engine already scans **metadata only** (`proton-drive fs list --json`
returns names, types, sizes, and timestamps). It does not read file contents
during a scan. The remaining cost is that Proton's CLI lists one folder per
process, and each process costs roughly 3–4.6 seconds on the current account.

Current measurements:

- cold metadata walk: ~93 seconds for 1,230 remote entries
- warm snapshot cycle: ~3 seconds
- local change to cloud: ~6 seconds
- push change to local: ~1 second
- peak service memory during a scan: ~600 MB

## Goals

- Warm startup shows a trustworthy status immediately instead of waiting for a
  full walk.
- A cached remote snapshot is used immediately; a background audit reconciles
  it.
- Local changes keep syncing while the audit runs.
- Deletions are held until the current remote view is trusted.
- First-ever scan shows progress and uses adaptive concurrency.
- Transfers across independent folders can run with controlled parallelism.
- The GUI shows what is happening: cache age, audit progress, scan progress,
  transfer progress, and last full check.
- A single switch can restore the old behavior.

## Non-goals

- No new authentication path.
- No persistent Proton SDK/API helper in this phase. That is phase B.
- No change to trash-only deletion, conflict-copy policy, or selective sync.
- No permanent deletes.
- No new runtime dependency unless a measured problem requires it.
- No change to the CLI itself.

## Constraints

- `fs list` is one folder per CLI process; there is no recursive list.
- The engine, GUI, and health file are separate processes. Health JSON is the
  contract between them.
- The service must remain safe if the snapshot is missing, corrupt, or stale.
- Multiple CLI processes already run during scans; transfer concurrency must
  respect rate limits and keyring/session behavior.
- A future B implementation should be able to replace the remote backend
  without rewriting the planner or state store.

## Approaches considered

### A. Fast-start cache + event-first sync + controlled parallelism

Use the existing snapshot/state and push/inotify paths. Start from cache, audit
in the background, hold deletes until the audit is trusted, adapt scan workers,
parallelize independent transfer batches, and expose progress in the GUI.

**Trade-off:** safe and incremental; cold first-ever scan still pays the
per-folder CLI cost, though adaptive workers and progress help.

### B. Persistent metadata/sync helper

Replace one-process-per-folder scans with a long-running helper using Proton's
SDK/API session. This is the only fundamental fix for cold-scan and transfer
process overhead, but it is a larger component with more auth and maintenance
risk.

**Trade-off:** potentially much faster, but not now. A must leave a seam for B.

### C. Tuning and GUI only

Increase workers, adjust intervals, improve labels.

**Trade-off:** cheap, but does not change the startup architecture and cannot
meet the “fast first scan” goal.

**Chosen:** A first. B later if A's measured results are insufficient. Keep C's
safe tuning as part of A.

## Design

### 1. Fast start from the existing snapshot

The engine already has `remote-snapshot.json` (tree + fingerprint + timestamp)
and `state.json` (last observed paths). Add a startup mode:

- If the snapshot and state are usable, load them.
- Publish `health.status = "ok"` with:
  - `mode = "fast"`
  - `cache_age`
  - `audit_pending = true`
  - scan/transfer counters at zero
- Start inotify and the push listener immediately.
- Run the normal fast cycle using the cached remote tree.
- Start a background audit of the remote tree.
- When the audit completes, atomically replace the snapshot and clear
  `audit_pending`.

If the snapshot is missing or corrupt, fall back to the current full scan. Do
not pretend a fast start is available. “Usable” means:

- a snapshot with a valid signature and shape, and
- a non-empty `state.json` when the snapshot contains entries.

If state is missing/empty but the snapshot has entries, rebuild via a full
scan rather than treating every path as new.

### 2. Background audit

The audit is a full metadata scan, but it must not hold the app-wide CLI lock
for ~90 seconds or make the app look frozen.

- The audit is **chunked**, not one long lock. A chunk is a small set of
  folders sized to finish in roughly ≤10 seconds.
- The app-wide CLI lock is taken for a chunk and released between chunks.
  Local, push, and transfer work runs between chunks.
- The audit scans into a separate tree and writes progress to health:
  - `audit_phase`
  - `audit_done`
  - `audit_total`
  - `audit_seconds`
- Locally touched scopes (uploads, trashes, moves) are recorded. Before the
  snapshot swap, those scopes are merged or re-listed so the new tree does not
  make a just-written file look remotely deleted.
- Push events received during the audit are queued. After the swap, only the
  affected folders are re-listed; no immediate second full walk.
- On failure, keep the cached view and retry on the normal retry policy.
- If the audit cannot complete, deletes stay held. There is no automatic
  release without a trusted full check.

If the audit cannot run because the remote is unavailable, keep the cache and
show an error; do not silently claim “up to date.”

### 3. Deletion safety while the cache is being audited

The cache is useful for display and ordinary local work, but it may be missing
remote changes that happened while the app was closed.

While `audit_pending` is true:

- Hold all delete actions (`trash_remote`, `trash_local`).
- New local files and new local folders may upload immediately; they cannot
  overwrite an unknown remote node by name.
- Edits to files that already exist remotely are held until the audit verifies
  the remote side.
- Remote downloads proceed only from the cached view and are reconciled by the
  audit.
- The health/GUI states that deletes and existing-file edits are held for the
  audit.

This preserves the existing “never lose data” direction and prevents a stale
snapshot from trashing a remote file that changed while the app was offline.

### 4. Event-first change detection

Keep the current mechanisms and tighten their use:

- inotify local wake, coalesced with the existing debounce.
- Proton push listener started before the first cycle.
- Push events re-list only affected folders when the snapshot can resolve them.
- A push event at the volume root still falls back to a full walk.
- The audit replaces the old blocking `PEEK_AGE`/`FULL_EVERY` full walks while
  fast start is enabled. `PROTON_SYNC_AUDIT_EVERY` controls the periodic audit;
  `PROTON_SYNC_FAST_START=0` restores the old fingerprint-first/full-walk
  behavior.
- With `PROTON_SYNC_INTERVAL=0` (“Off”), the startup audit still runs if the
  cache is stale, but no periodic audits are scheduled. Push, inotify, and
  manual checks still wake the engine.
- Health records why a cycle ran: `push`, `local`, `manual`, `peek`, `audit`,
  or `poll`.

### 5. Adaptive scan concurrency

Replace the fixed default with an auto mode:

- Start at the current configured value (12 in the measured install), not
  below it; this keeps the memory baseline unchanged.
- Cap at 16; never let the service create hundreds of CLI processes.
- Measure per-list latency and rate-limit/error responses.
- Reduce concurrency on errors/429s, increase gradually when stable.
- Keep a manual override in Settings for users who want a fixed value.
- Write the effective value to health/diagnostics.

The target is not “maximum processes”; it is lower wall-clock time without
memory spikes or Proton throttling. If the 20% first-scan target conflicts with
the no-memory-increase target, **keep the memory cap** and report the measured
result.

### 6. First-ever scan

With no snapshot, a full scan is still required. Improve it by:

- showing folder counts and progress in health/GUI
- scanning breadth-first for deterministic progress
- using the adaptive worker cap
- optionally prioritizing the folder currently open in the Cloud browser
- reporting elapsed time and per-folder latency in diagnostics

A 90-second first scan cannot be removed without B. The goal for A is a
measurable reduction plus a GUI that explains what is happening. This phase
does **not** publish a partial remote tree: the Cloud browser uses the old
snapshot or waits for the first complete scan.

### 7. Transfer parallelism

The engine already batches uploads and downloads by parent folder. Add:

- `PROTON_SYNC_TRANSFERS` default `4`; allowed range 1–10.
- `Auto` targets 4–6 based on rate-limit/error feedback; manual `10` is
  available as an experimental high-throughput setting.
- Run independent parent-folder transfer batches concurrently.
- Keep one batch per parent to preserve CLI behavior.
- Back off on rate-limit errors and session-refresh failures.
- Benchmark 2/4/6/10; keep the fastest stable value. Do not hard-default to 10
  without measurement.
- Health fields:
  - `transfer_active`
  - `transfer_done`
  - `transfer_total`
  - `transfer_bytes`
- GUI shows “Uploading 4 of 18” / “Downloading 2 folders” rather than only a
  generic syncing state.

Single large files remain a single CLI transfer; this phase does not add
multi-part transfer support.

### 8. GUI and settings

The current GUI already covers folder selection, interval, deletes, workers,
retries, excludes, selective sync, notifications, autostart, pause, and tray.
Do not add a wall of new knobs.

Add one **Performance** card:

- **Fast start:** On (recommended) / Off
  - explanation: “Start from the last metadata snapshot, then audit in the
    background. Deletes are held until the audit finishes.”
- **Scan concurrency:** Auto / Low / High
  - manual numeric override remains in Advanced.
- **Transfer concurrency:** Auto / 2 / 4 / 6 / 10
- **Run full check now** button
- **Clear snapshot cache** (snapshot only, not `state.json`, with confirmation)

Home/status additions:

- “Ready from cache · checking 420 / 1,230 folders”
- “Cached view · deletes held until check completes”
- “Last full check 2h ago”
- active transfer progress when present
- “Copy diagnostics” action with health/config summary, no credentials

All new health fields are optional in the GUI parser so old/new engine versions
remain compatible.

### 9. Configuration and rollback

New config keys:

```text
PROTON_SYNC_FAST_START=1          # 1 default; 0 restores full-scan startup
PROTON_SYNC_AUTO_WORKERS=1        # 1 default; 0 uses PROTON_SYNC_WORKERS
PROTON_SYNC_TRANSFERS=4           # 1-10; 10 is experimental
PROTON_SYNC_AUDIT_EVERY=1800      # background audit interval
```

Rollback is one key: `PROTON_SYNC_FAST_START=0`, restart the service, and the
old full-scan startup returns. The new GUI fields degrade gracefully.

### 10. Seam for phase B

Keep remote reads and writes behind the existing functions
(`remote_fingerprint`, `scan_remote`, `scan_scope`, `upload_batch`,
`download_files`, `trash` calls). Add no B code now.

When B starts, a persistent helper can implement the same operations and the
planner/state/GUI can remain. A's health and progress fields are the contract
B must keep.

## Success metrics

Measured on the existing account/data set, before and after:

| Metric | Current | A target |
|---|---:|---:|
| Warm start to usable status | ~90s cold / 3s warm | <2s from cache |
| First-ever full scan | ~93s | at least 20% faster; progress visible |
| Local change to cloud | ~6s | no regression |
| Push change to local | ~1s | no regression |
| Multi-folder transfer | current batching | 1.3–1.5× where safe |
| Transfer 10-way stability | unknown | benchmark before recommending |
| Peak scan memory | ~600MB | no increase at scan default; manual 10-way transfer may reach ~1.5GB |
| Deletes on stale cache | unsafe/undefined | held until audit |

If the first-scan target is not met, do not hide it; report the measured result
and proceed to B only with approval.

## Testing

Automated:

- snapshot load/trust and corrupt-snapshot fallback
- delete hold while `audit_pending`
- health JSON old/new compatibility
- worker auto/backoff decisions
- transfer batch scheduling (one parent per batch)
- config parsing and rollback

Manual:

- warm start with a valid snapshot: status appears immediately
- cold start with no snapshot: progress and eventual completion
- new local file during audit: uploads; deletes held
- existing-file edit during audit: held, then reconciled after the audit
- both sides changed after audit: conflict copy, no timestamp-only overwrite
- remote push during audit: queued and applied after audit
- restart during audit
- corrupted/missing snapshot
- 429/rate-limit behavior
- conflict and delete-to-trash regression
- GUI performance card and diagnostics

## Risks

- A background audit and push events can race. Resolve by serializing snapshot
  swaps and re-listing only queued scopes after the swap.
- A stale cache can hide remote deletes. Hold deletes until audit.
- Higher transfer concurrency can trigger Proton rate limits or session-refresh
  races. Default to 4, cap at 10, back off on errors, and benchmark before
  recommending 10.
- GUI/engine schema drift. All new health fields are optional.
- B may still be required for a fast first-ever scan. A does not pretend to
  solve that.

## Decisions (my calls unless corrected)

1. Fast start is **on by default**; it can be disabled in Performance settings.
2. Deletes are held for the whole background audit, not just above an age
   threshold.
3. The audit is chunked (≤10s per chunk) and releases the CLI lock between
   chunks so local work and push events can interleave.
4. Locally touched scopes are re-listed/merged before the audit snapshot swap.
5. Scan concurrency defaults to **Auto**, starting at the current configured
   value (12 here) and capping at 16; no memory increase takes priority over the
   20% first-scan target.
6. Transfer concurrency defaults to **4**; maximum is **10**, with 10 treated as an experimental manual setting until benchmarked.
7. “Run full check now” is the explicit way to force a complete audit.
8. `PROTON_SYNC_FAST_START=0` restores the old behavior; `INTERVAL=0` keeps the
   startup audit but disables periodic audits.
9. No new runtime dependencies in A.

## Resolved safety decision: held edits after the audit

While `audit_pending` is true, edits to files that already exist remotely are
held. New files and new folders upload immediately.

After the audit completes, each held path is reconciled using the existing
state plus the audited remote metadata:

- only the local side changed since the last sync → upload the local version
- only the remote side changed → download the remote version
- both sides changed → keep both, using the existing conflict-copy behavior
- metadata is ambiguous or incomplete → keep both rather than guessing

Newest-wins is only used when one side changed. If both sides changed, timestamps
alone are not trusted because of clock skew and provider metadata differences.
The GUI will say the file was waiting for verification and then show whether it
uploaded, downloaded, or produced a conflict copy.

This preserves the current “never lose the other version” policy while still
letting the audit make the decision instead of a stale cache.

## Implementation order (for the later plan)

1. Add phase timings, cache/audit/transfer fields to health; keep backward
   compatibility.
2. Add fast-start snapshot load and background audit with delete holds.
3. Add adaptive workers and first-scan progress.
4. Add transfer concurrency and progress.
5. Add Performance card and diagnostics.
6. Test, benchmark, document rollback.
7. Only then plan B.
