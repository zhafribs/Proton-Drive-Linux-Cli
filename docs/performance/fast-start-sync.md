# Fast-start sync: measurements, acceptance, and rollback

Status: implemented on the `feat/fast-start-sync` branch (GUI 3.16.16).
Measured on the existing account on 2026-10-24: 1,230 remote/local entries,
213 remote folders, official Proton Drive CLI 0.9.0, Linux x86_64.

## Result

| Path | Before | After |
| --- | ---: | ---: |
| Warm start to `status=ok` (snapshot + state present) | 4.1 s | **0.12 s** |
| First-ever scan (no snapshot/state) | ~93 s baseline / 90.6 s measured | **90.6 s**, with progress |
| Background remote audit (1,230 entries / 213 folders) | blocked first cycle | **96 s**, chunked and non-blocking |
| Peak process-tree RSS during cold scan | — | **132.5 MB** |
| Local new file during audit, plan → upload | n/a (old path blocked) | **~6 s** |
| Held existing-file edit, after audit → upload | — | **~6 s** |
| Both sides edited, conflict copy created | conflict copy | **~12 s** |
| Local delete/remote delete, after audit → trash | deletion | **~21 s** |

The warm-start number is the time from process start until `health.json` first
reports `status=ok` with `mode=fast`; the audit then continues in the
background. The old path (`PROTON_SYNC_FAST_START=0`) was measured with the
same cached snapshot/state and reported `mode=keep` at 4.1 s. A cold scan still
pays the per-folder official-CLI cost; adaptive concurrency and the progress
counter make it visible but do not remove that cost. A persistent metadata
helper is Phase B.

## Transfer benchmark

Disposable remote folders, 64 × 512 KiB files across 8 parent folders,
`PROTON_SYNC_TRANSFERS` varied, 12 scan workers, 0 failures at every setting:

| Concurrent parent batches | Elapsed |
| ---: | ---: |
| 2 | 119 s |
| 4 (default) | 89 s |
| 6 | 87 s |
| 10 (maximum) | 81 s |

10 was stable on this account and is the throughput recommendation, but the
shipped default remains 4: it leaves headroom for smaller uploads, slower
links, and Proton rate limiting. Users can opt in to 10 in Settings →
Performance or `PROTON_SYNC_TRANSFERS=10`.

## Defaults and knobs

| Setting | Default | Notes |
| --- | ---: | --- |
| `PROTON_SYNC_FAST_START` | `1` | `0` restores the old fingerprint/peek/full-walk path |
| `PROTON_SYNC_AUTO_WORKERS` | `1` | RAM-budget and rate-limit adaptation |
| `PROTON_SYNC_WORKERS` | `12` | ceiling 16; the RAM cap can lower it |
| `PROTON_SYNC_TRANSFERS` | `4` | 1–10 parallel parent batches |
| `PROTON_SYNC_AUDIT_EVERY` | `1800` | seconds; 0 disables periodic audits |
| `PROTON_SYNC_AUDIT_CHUNK` | `40` | folders per audit chunk ceiling |

With `PROTON_SYNC_INTERVAL=0` (“Off”), the startup audit still runs, but no
periodic audits are scheduled; push, inotify, manual sync and the GUI’s
**Run full check now** still work.

## Commands used

Warm fast start, using a copy of the real state so the live service is not
disturbed:

```bash
cp -a ~/.local/state/proton-sync /tmp/proton-sync-warm
XDG_STATE_HOME=/tmp/proton-sync-warm \
PROTON_SYNC_LOCAL="$HOME/ProtonDrive" PROTON_SYNC_REMOTE=/my-files \
PROTON_SYNC_INTERVAL=0 PROTON_SYNC_PUSH=0 PROTON_SYNC_INOTIFY=0 \
PROTON_SYNC_FAST_START=1 python3 scripts/proton-sync.py --loop
# watch health.json: status=ok, mode=fast; then stop the test process
```

Old path (rollback verification):

```bash
XDG_STATE_HOME=/tmp/proton-sync-warm \
PROTON_SYNC_LOCAL="$HOME/ProtonDrive" PROTON_SYNC_REMOTE=/my-files \
PROTON_SYNC_FAST_START=0 python3 scripts/proton-sync.py --once
```

First-ever scan: run the same command with an empty state directory and no
snapshot. The audit benchmark is the fast-start loop with a copied snapshot;
poll `audit_pending` until it becomes `false`.

Transfer benchmark: create a disposable remote root for each run, put 64 ×
512 KiB files in 8 parent folders in a disposable local folder, run

```bash
XDG_STATE_HOME=/tmp/bench-state \
PROTON_SYNC_LOCAL=/tmp/bench-local PROTON_SYNC_REMOTE=/my-files/.bench \
PROTON_SYNC_FAST_START=0 PROTON_SYNC_WORKERS=12 \
PROTON_SYNC_TRANSFERS="$n" python3 scripts/proton-sync.py --once
```

then move the disposable remote root to Proton trash. Do not benchmark against
the live sync folder.

## Acceptance matrix

| Case | Result |
| --- | --- |
| Valid snapshot warm start | **Pass** — `mode=fast`, audit pending, 0.12 s |
| Missing/corrupt snapshot falls back | **Pass** — `snapshot_usable` tests; cold scan ran full |
| New local file during audit uploads | **Pass** — uploaded while `audit_pending=true` |
| Existing-file edit during audit | **Pass** — no upload while pending; uploaded after audit |
| Remote edit during audit | **Pass** — reconciled by audit |
| Both sides edit the same file | **Pass** — conflict copy kept |
| Local delete during audit | **Pass** — held, then remote trash after audit |
| Remote delete during audit | **Pass** — held, then local trash after audit |
| Audit failure | **Pass (unit)** — cached view kept, `audit_pending=true`, `audit_error` exposed; deletes stay held |
| 10-way transfers | **Pass** — 64 files, 0 failures |
| Restart during audit | **Pass** — restart enters `mode=fast` and starts a fresh audit; cache/state intact |
| `PROTON_SYNC_FAST_START=0` rollback | **Pass** — old path entered `mode=keep` in 4.1 s |

The automated acceptance test used a disposable remote root and local folder,
with push/inotify disabled to make the audit timing deterministic. The push
listener path was not re-benchmarked in that isolated shell because the test
process could not read the CLI keyring session; the live service continues to
use the unchanged push path (~1 s in prior measurements).

## Safety model

- The cached remote view is used for planning before the audit finishes, but
  **all deletes and edits to files already present remotely are held**.
- New local files and folders may upload during the audit; their parent scope
  is re-listed before the audit snapshot is swapped so the new remote node is
  not mistaken for a deletion.
- Held paths keep their last known state; an edit made during the audit is
  therefore re-planned after the audit instead of being stamped as synced.
- A root-scope touch (for example a new file at the sync root) refreshes the
  root listing during the swap.
- If the audit fails, the cached view stays, deletions/edits stay held, and
  the error is visible in health/Home. There is no automatic “trusted” claim.
- Deletions are always trash-only. Conflicts keep both copies.

## Rollback

Set one key and restart:

```bash
echo 'PROTON_SYNC_FAST_START=0' >> ~/.config/proton-sync.conf
systemctl --user restart proton-sync.service
```

For a one-off run, prefix the command with `PROTON_SYNC_FAST_START=0`. State,
local files, snapshot and trash history are preserved. Removing the key later
turns fast start back on.
