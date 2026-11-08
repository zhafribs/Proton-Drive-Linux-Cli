# Full product audit — final report

**Program:** Proton Drive for Linux full-product audit  
**Period:** 2026-11-08  
**Director:** approved scope, 50–200% interface size, file-coverage model,
soft freeze until G5, one signed release per milestone  
**Status:** G0–G4 complete; G5 release `v0.9.0-gui3.20.49` verified; director
sign-off line below remains to be signed.

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
| G0 Baseline | — | Inventory, ledgers, baseline tests/clippy/gate |
| G1 Interface size | `v0.9.0-gui3.20.44` | Five presets, 50–200% slider, migration, scale-aware window |
| G1 patch | `v0.9.0-gui3.20.45` | Slider zoom on release, not mid-drag |
| G2 UI/frontend | `v0.9.0-gui3.20.46` | Keyboard/contrast/selection/layout audit |
| G3 Backend/contract | `v0.9.0-gui3.20.47` | CLI timeouts, state paths, service safety |
| G4 Security/legal/release | `v0.9.0-gui3.20.48` | Advisory updates, 0600 config, action pins, threat model/licences |
| G5 Consolidation | `v0.9.0-gui3.20.49` | Data-safety P0/P1 fixes, dead deps, final screenshots |

All seven releases have a signed `SHA256SUMS`, an Ed25519 signature, a smoke
test and a verified GLIBC 2.28 floor. The releases page is never pruned.

## 3. Verification

| Check | Result |
|---|---|
| `cargo test --workspace` | 279 passed, 4 ignored |
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

## 5. Open P2/P3 backlog

No P0 or P1 is open. The remaining findings are scheduled for the next
maintenance window:

- P2: unkeyed 64-bit deletion-approval digest; bind it to roots and use
  SHA-256.
- P2: keyring lookup accepts a non-exact Proton auth-session label; prefer the
  exact label and reject ambiguous matches.
- P2: a post-download edit before the state rescan can be stamped as synced;
  carry the placed fingerprint into `state_update`.
- P2: upload destination is captured when the file picker closes; carry the
  target folder from the opening click.
- P2: context-menu-only file actions still need a full keyboard path beyond
  Ctrl+A.
- P2: the Files status/error line is hidden while a selection is active.
- P2: pause/resume workers are not serialized; a fast pause→resume can leave
  the service in the opposite state.
- P2: the folder-tree label can clip at the 150 pt sidebar; width-aware
  elision and a hover are pending.
- P3: non-UTF-8 file-picker paths, portal cancel-vs-unavailable refinements,
  the second-launch quit race, and a few copy/elision polish items.
- P3: formatter decision — the tree is not rustfmt-clean; adopting rustfmt
  would rewrite 33k lines. No fmt gate is enforced. The director can choose a
  formatting-only release.

## 6. Accepted risks

- A compromised maintainer signing key can sign a malicious update; there is
  no revocation list, only rotation with a new public key.
- The official CLI lock is advisory; a non-cooperating process can bypass it.
- The zbus/accesskit stack has three unmaintained transitive crates with no
  known vulnerability; replacing them depends on an upstream release.
- Local malware with the user's privileges can read the sync folder and state.
- 50% interface size is very small on a 1080p display; it is the approved
  lower bound and remains a product decision.

## 7. Director sign-off

I have reviewed the coverage ledger, the finding register, the release
evidence and the threat model, and I accept the residual risks listed above.

Director: __________________________  Date: ______________
