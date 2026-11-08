# Full product audit — final report

**Program:** Proton Drive for Linux full-product audit  
**Period:** 2026-11-08  
**Director:** approved scope, 50–200% interface size, file-coverage model,
soft freeze until G5, one signed release per milestone  
**Status:** G0–G5 complete; releases `v0.9.0-gui3.20.49`, patches
`v0.9.0-gui3.20.50`, `v0.9.0-gui3.20.51` and maintenance patch
`v0.9.0-gui3.20.52` verified; program acceptance recorded in section 7.

## 1. What was audited

Every Rust file in the workspace (60 files, 33,244 lines), every UI page,
dialog, context menu and tray state, every GUI/CLI/engine command, every state
file and lock, the release pipeline, the dependency/licence graph, the update
trust path and the legal notices.

The audit was performed by four councils:

| Council | Contribution |
|---|---|
| Director | Scope, interface-size range, risk acceptance, release gates |
| Design Council | Page/dialog layout at 50–200%, focus, contrast, headings, selection |
| Engineering Council | Command contracts, state ownership, data safety, dead code |
| Trust Council | Keyring/session, subprocesses, paths/symlinks, updates, CI, licences |
| Audit Secretariat | Inventory, coverage ledger, findings register, evidence |

## 2. Releases

| Gate | Release | Scope |
|---|---|---|
| G0 Baseline | `v0.9.0-gui3.20.43` | Inventory, ledgers, baseline tests/clippy/gate |
| G1 Interface size | `v0.9.0-gui3.20.44` | Five presets, 50–200% slider, migration, scale-aware window |
| G1 patch | `v0.9.0-gui3.20.45` | Slider zoom on release, not mid-drag |
| G2 UI/frontend | `v0.9.0-gui3.20.46` | Keyboard/contrast/selection/layout audit |
| G3 Backend/contract | `v0.9.0-gui3.20.47` | CLI timeouts, state paths, service safety |
| G4 Security/legal/release | `v0.9.0-gui3.20.48` | Advisory updates, 0600 config, action pins, threat model/licences |
| G5 Consolidation | `v0.9.0-gui3.20.49` | Data-safety P0/P1 fixes, dead deps, final screenshots |
| G5 patch | `v0.9.0-gui3.20.50` | Non-UTF-8 AppRun dispatch and tray name-collision safety |
| G5 residual patch | `v0.9.0-gui3.20.51` | Residual data-safety P2 fixes (F-086, F-087, F-088, F-091) |
| Post-audit maintenance | `v0.9.0-gui3.20.52` | Upload target, selection keyboard/status, picker errors, tree elision, root/scale warnings |

All ten releases have a signed `SHA256SUMS`, an Ed25519 signature, a smoke
test and a verified GLIBC 2.28 floor. The releases page is never pruned.

## 3. Verification

| Check | Result |
|---|---|
| `cargo test --workspace` | 282 passed, 4 ignored |
| `cargo clippy --release --workspace --all-targets -- -D warnings` | clean |
| Release-gate fixtures | 12/12 |
| `cargo audit` | 0 vulnerabilities; 3 unmaintained zbus-family warnings documented |
| `cargo machete` | no unused dependencies (vendored ksni exception removed) |
| AppImage smoke | passed on every published release |
| SHA-256 + Ed25519 | verified on every published release |
| GLIBC floor | 2.28 on AppRun, GUI and engine |
| Layout screenshots | Home/Files/Photos/Activity/Settings/About/dialogs at 50–200% |
| Contract matrix | every GUI/tray action mapped to CLI argv, engine action and state owner |

Evidence lives in `docs/audit/evidence/`, layout screenshots in
`docs/audit/layout/`, the file ledger in `COVERAGE.md`, and every finding in
`FINDINGS.md`.

## 4. Data-safety findings fixed in G5

The final audit found and fixed one P0 and a set of P1 paths that could lose or
misattribute user data:

1. **P0 — a download could overwrite an edit made after the scan.** The
   planner's local fingerprint is re-checked immediately before placement;
   existing files are backed up and restored if the bytes changed; new files
   use no-replace hard links.
2. **P1 — delete propagation ignored an edit on the surviving side.** A local
   delete paired with a cloud edit (or the reverse) now restores/uploads the
   surviving revision instead of destroying it.
3. **P1 — scan failures looked like deletions.** Read errors, symlinks,
   special files and malformed cloud listings fail the cycle instead of
   producing an empty tree.
4. **P1 — small libraries could be wiped.** The wipe guard now arms on any
   non-empty history, not only at five files.
5. **P1 — folder trashes under-counted the approval.** Every descendant is
   included in the held count/digest.
6. **P1 — cloud moves paired with locally edited files.** The move is only
   paired when the local survivor still matches state.
7. **P1 — the fast-edit shortcut stamped edits as synced.** It was removed;
   all event updates use the normal scoped listing + planner.
8. **P1 — state/caches were not scoped to roots/account.** Changing a sync
   root archives the old history; signing out clears the account state and
   cloud caches.
9. **P1 — Cloud mutations bypassed the CLI lock.** They now use the shared
   lock, which fails closed instead of running unprotected.
10. **P1 — download temp dirs were predictable and left behind.** Private
    0700 unique dirs, RAII cleanup, collision-safe names, recursive folders.
11. **P1 — Trash kept the previous view's rows/selection.** Views clear on
    switch; trash rows use node UIDs so duplicate names stay distinct.
12. **P1 — a failed sidebar tree listing spawned a CLI process per frame.**
    A cooldown and request timestamp bound the retries and stop stale
    overwrites.
13. **P1 — journal timestamps failed on systemd <255.** `+0800`, precise
    fractions, conflict-copy parentheses and leading-verb notifications are
    handled.
14. **P1 — partial failures still showed “up to date”.** The engine writes
    `error` when a cycle had failed transfers.

The residual patch `3.20.51` closed four more data-safety P2s:

15. **P2 — a post-download edit could be stamped as synced.** The fingerprint
    of the bytes actually placed is carried into `state_update`; a rescan
    after placement cannot hide an edit made between the download and the
    state write.
16. **P2 — pause and resume could interleave.** One service-control lane and a
    desired-state bit make the last click win; a quick pause→resume cannot
    leave the engine in the opposite state.
17. **P2 — deletion approvals were unkeyed and root-agnostic.** The one-shot
    approval now uses SHA-256 over the side and exact path set and is bound to
    the local/remote roots; a stale path set or a different root fails closed.
18. **P2 — keyring lookup could pick the wrong Proton session.** The exact CLI
    item is preferred; a single renamed auth-session is tolerated, but two
    ambiguous sessions no longer route quota/event calls to the wrong account.

The maintenance patch `3.20.52` closed the remaining UX/robustness findings:

19. **P2 — an upload could land in a folder the user had already left.** The
    remote target is captured when the picker opens, not when it closes.
20. **P2 — context-menu actions had no keyboard path and hid the status line
    during a selection.** The selection bar now has a keyboard-reachable
    **More** menu (Open, Rename, Duplicate, Details, Move), **F2** renames, and
    the status/error text stays visible.
21. **P3 — a missing portal/zenity/kdialog looked like a cancel.** Picker
    failures now surface as an error toast.
22. **P3 — the sidebar tree label used a fixed character count.** It is elided
    to the real width and shows the full name on hover.
23. **P3 — changing a sync root silently archived history.** The save
    confirmation now explains that history is archived and cloud caches are
    cleared.
24. **P3 — a second launch during shutdown could leave no instance running.**
    It waits for the single-instance lock and starts normally if the previous
    process has exited.
25. **P3 — non-UTF-8 picker paths were passed lossily.** They are refused with
    a visible message instead of uploading a different path.
26. **P3 — 50% was easy to select without understanding the readability
    trade-off.** Appearance now shows a warning at the minimum.

## 5. Open P2/P3 backlog

No P0, P1, P2 or P3 implementation finding is open. F-004 (formatter) is
closed as an accepted deviation: the tree is not rustfmt-clean, no formatting
gate is enforced, and a formatting-only rewrite of ~33k lines is not worth the
review risk. The accepted product and environment risks remain in section 6.

## 6. Accepted risks

- A compromised maintainer signing key can sign a malicious update; there is
  no revocation list, only rotation with a new public key.
- The official CLI lock is advisory; a non-cooperating process can bypass it.
- The zbus/accesskit stack has three unmaintained transitive crates with no
  known vulnerability; replacing them depends on an upstream release.
- Local malware with the user's privileges can read the sync folder and state.
- 50% interface size is very small on a 1080p display; it remains the approved
  lower bound and now carries an explicit readability warning.
- Changing the sync root archives local sync history; the save confirmation
  explains this before the change is applied.
- Symlinks inside the sync folder still fail the scan; the engine reports the
  offending path instead of guessing.

## 7. Program acceptance

This is a solo-maintainer program: the Director and council roles above are
review functions, not separate signatories. The coverage ledger, finding
register, release evidence, threat model and residual-risk list are the
acceptance record.

**Status:** accepted 2026-11-08. No external sign-off is required.
