# Milestone M4 — Path Safety

**Status:** complete — released as `v0.9.0-gui3.24.0`  
**Theme:** catch path problems before a sync cycle, and keep Recovery fast on
large libraries.

## Scope

- **Symlink preflight:** saving a changed sync root scans the folder first and
  lists links. Links are never followed; the user can remove them or add an
  exclude pattern. A **Check folder** button runs the same scan on demand.
- **Conflict scan cache:** `conflicts-cache.json` stores the last scan with a
  root, age and per-file fingerprint. It is reused only when every entry still
  matches; the engine removes it when it creates a conflict; a five-minute TTL
  bounds manual changes.
- **Local trash usage:** Recovery shows total trash size, warns above 1 GB, and
  includes the size in the recovery check.
- **Non-UTF-8 paths:** already refused visibly (3.20.52); unchanged here.

## Acceptance

| Check | Result |
|---|---|
| Symlink scan finds nested links, does not follow them | `symlink_preflight_lists_links_without_following_them` |
| Cache reuses unchanged entries, invalidates on change | `cache_reuses_unchanged_entries_and_invalidates_on_change` |
| Engine invalidates cache on a new conflict | Code review; `invalidate_cache` call in `execute` |
| Root-change Save blocks on links | Manual / code review |
| Trash usage visible and warns above 1 GB | Code review |
| Release | Signed AppImage, smoke, GLIBC 2.28 |

## Not covered

- Full `OsString` CLI argument support for non-UTF-8 upload paths.
- A time/space retention policy that deletes trash automatically; deletion
  stays explicit by design.

## Release verification

- Tag `v0.9.0-gui3.24.0`; CI `42596386270` verify + release succeeded.
- 299 tests, clippy clean, release gate 12/12.
- SHA-256, Ed25519 signature, smoke and GLIBC 2.28 verified.
- Evidence: `docs/audit/evidence/release-3.24.0.txt`.
