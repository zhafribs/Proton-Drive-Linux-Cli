# Milestone M1 — Recovery Center

**Status:** complete — released as `v0.9.0-gui3.21.0`
**Target release:** `v0.9.0-gui3.21.0` (one signed release)  
**Theme:** make every destructive path reviewable and reversible from the GUI.

## Why this milestone

The audit closed the data-safety defects, but recovery still depends on tools
outside the app:

- Local deletions are moved to `~/.local/state/proton-sync/trash`, but the GUI
  has no local-trash view and no restore action.
- Conflict copies are written correctly and announced in the journal, but
  there is no place to compare the two versions and choose one.
- `state.json` has an archive-on-root-change path, but no visible restore
  point or repair action.
- There is no user-facing way to run a recovery drill and see that an
  interrupted cycle left the library consistent.

The result is safe behavior that is hard to verify and hard to undo without a
terminal. M1 closes that gap.

## Outcome

After M1, a user can:

1. Open **Recovery** and see local trash, cloud trash and conflict copies in
   one place.
2. Restore a locally trashed file to its original relative path.
3. Review a conflict group, open either copy, and keep local, keep cloud, or
   keep both.
4. Create or restore a state snapshot before a risky change.
5. Run **Check my sync** and get a plain-language result for an interrupted
   cycle, a missing state file, or failed transfers.

## Scope

### 1. Local trash manifest and view

- Write `trash-manifest.json` beside the local trash directory: original rel
  path, trashed-at timestamp, size, and a stable id.
- Keep the existing path-preserving layout and collision suffixes.
- Add a **Local trash** view with restore, delete permanently, and open
  containing folder.
- Refuse restore when the destination exists; use the same no-overwrite rules
  as downloads.

### 2. Conflict Center

- Detect conflict copies by their existing `(conflict ...)` name shape and the
  engine's journal entries; do not guess from content.
- Group copies with their canonical path and show size, local mtime, and
  remote mtime when known.
- Actions: keep both, replace canonical with the conflict copy (backup first),
  move the conflict copy to local trash, open folder.
- A conflict is never silently deleted.

### 3. State snapshots and repair card

- Rotate a small number of `state.json` snapshots (for example the last three)
  before a root change, a mass-delete approval, or an update swap.
- Show the snapshot time and file count in Settings → Recovery.
- Restore is explicit, verified against a checksum, and followed by a normal
  planning cycle before any transfer.
- Cap total snapshot disk use and prune oldest first.

### 4. Recovery drill and evidence

- Add a CI/integration drill that:
  - interrupts a download between staging and placement;
  - corrupts or truncates `state.json`;
  - restores the snapshot;
  - runs a plan and proves no local file is lost.
- Surface the last drill result in About → Diagnostics.
- Record the drill output in `docs/audit/evidence/`.

## Non-goals

- No permanent-delete bypass; deletes still go to trash first.
- No cloud version-history UI until the upstream CLI exposes it.
- No second account, new sync roots, or cloud sharing changes.
- No formatter rewrite.

## Acceptance criteria

| Area | Criterion |
|---|---|
| Local trash | Every local trash entry can be restored or permanently deleted from the GUI. |
| Conflicts | Every conflict copy is visible in the Conflict Center; keep-both remains the default. |
| State | A missing/corrupt `state.json` is restored from a verified snapshot without losing local files. |
| Interruption | The drill passes with a killed download and a truncated state file. |
| Accessibility | All Recovery views work at 720x400 and 50–200%, keyboard-only, with headings and focus rings. |
| Safety | Restore and replace actions re-check fingerprints and never overwrite a newer local file. |
| Release | 282+ tests, clippy clean, release gate, signed AppImage, smoke, GLIBC 2.28, evidence file. |

## Delivery sequence

| Phase | Work | Evidence |
|---|---|---|
| P1 | Trash manifest, local-trash view, restore/delete | Engine tests, GUI code review, 720x400 screenshots |
| P2 | Conflict grouping and actions | Grouping tests, conflict drill |
| P3 | Snapshot rotation, Repair card, diagnostics | Corruption drill, state tests |
| P4 | Keyboard/layout pass, docs, signed `3.21.0` | Full gate + release evidence |

## Risks and mitigations

- **Manifest drift:** entries created before M1 have no manifest. Fall back to
  the on-disk relative path and mark the entry as legacy.
- **Conflict detection:** only use the engine's generated name shape and
  journal; a user file that happens to contain “conflict” is not a conflict.
- **Snapshot disk use:** cap by count and bytes; snapshots are state metadata,
  not file copies.
- **Restore races:** re-check the destination and the recorded fingerprint
  immediately before writing.
