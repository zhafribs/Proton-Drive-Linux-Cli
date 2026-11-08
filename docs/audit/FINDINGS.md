# Finding register

**Program:** Full product audit, 2026-11-08  
**Rule:** no milestone closes with an open P0 or P1. Every finding names a
file, the evidence, an owner, a fix and a verification step.

| Severity | Meaning | Release rule |
|---|---|---|
| P0 | Data loss, credential exposure, broken update trust, security hole | Stop; fix before any release |
| P1 | User-visible breakage, incorrect state, layout overlap at a supported size | Fix in the current milestone |
| P2 | Consistency, accessibility, performance, maintainability | Schedule in the program |
| P3 | Nice-to-have or future idea | Backlog; director decides |

## Open findings

| ID | Sev | Area | File | Evidence | Owner | Target | Status |
|---|---|---|---|---|---|---|---|
| F-004 | P2 | Tooling | workspace | `cargo fmt --check` is not part of CI and the current tree is not rustfmt-clean; adopting the default formatter would rewrite ~33k lines. | Engineering | G5 | Closed: accepted deviation; no formatting gate, no rewrite. |
| F-005 | P3 | Accessibility | `gui/src/ui/settings.rs` | 50% renders text at half size; it is the approved lower bound, but very small on a 1080p display. | Design | G5 | Accepted; 3.20.52 adds a readability warning at 50%. |
| F-006 | P3 | Layout | `gui/src/ui/settings.rs` | At the 720x400 logical minimum, the Appearance card needs vertical scrolling to reach the slider. No overlap or clipping occurs. | Design | G2 | Accepted: scrolling is expected at the minimum height. |
| F-007 | P2 | Tooling | workspace | `cargo-audit`, `cargo-deny`, `cargo-machete` and `cargo-udeps` are not installed locally; CI has the license check and clippy only. | Trust | G4 | Fixed: audit + machete run and recorded in `security-3.20.48.txt`. |
| F-008 | P2 | UX overlap | `gui/src/ui/settings.rs` | The same `PROTON_SYNC_WORKERS` value is edited by **Scan workers** (Performance) and **Parallel connections** (Fine-tuning), with different labels and help text. | Design/Engineering | G2 | Fixed in 3.20.46 (see F-023). |
| F-009 | P2 | Security | `crates/core/src/conf.rs` | `conf::write` creates `proton-sync.conf` with the default umask (usually 0644). The **Extra environment variables** card accepts proxy credentials, so the file can be readable by other local users. | Trust | G4 | Fixed in working tree for 3.20.48 (0600 + test). |
| F-010 | P3 | Consistency | `gui/src/ui/settings.rs` | The **Small** preset is 93%, but the slider steps in 5% increments, so 93% cannot be reached by dragging and the next drag jumps to 95%. | Design | G2 | Fixed in 3.20.46 (Small 95%, see F-024). |
| F-011 | P2 | Accessibility | `gui/src/ui/settings.rs` | In the selective-sync tree the folder name is a plain label; only the small checkbox glyph and the expand arrow are clickable. | Design | G2 | Fixed in 3.20.46 (see F-025). |
| F-012 | P3 | Migration | `gui/src/prefs.rs` | Configs written before 3.20.44 stored the window size in egui points. At a non-100% scale those values are now read as physical units, so the first launch after upgrading can open a smaller window; the scale-aware floor and auto-grow correct it. | Engineering | G5 | Accepted: one-time window-size change; no data loss. |

## Closed findings

| ID | Sev | Area | File | Finding | Fix | Verified |
|---|---|---|---|---|---|---|
| F-001 | P1 | Layout | `gui/src/ui/settings.rs` | At narrow widths the preset row and the slider shared one horizontal control row, pushing the percentage, Reset and part of the card past the right edge. | Presets and the slider now use separate rows inside a vertical control; the slider has a fixed 160 pt rail and a trailing fill. | 720x400 and 720x600 screenshots; Appearance card ends inside the page with the value and Reset visible. |
| F-002 | P1 | State | `gui/src/prefs.rs`, `gui/src/ui/mod.rs` | The window size was saved in egui points. Restarting at a larger interface scale restored a smaller physical window, and at 200% the layout fell below its minimum. | Window sizes are stored as physical pixels at the desktop scale (egui points × zoom). Loading clamps the saved size to a scale-aware floor. | `prefs::tests`; 200% launch from a 1024x720 seed grows to 1440x800. |
| F-003 | P1 | Layout | `gui/src/ui/mod.rs`, `gui/src/main.rs` | The viewport minimum was a fixed 720x540 physical points, so 200% left only 360 logical points and the three-column pages collided and clipped. | The minimum is 720x400 egui points, capped to the monitor; changing scale sends a scaled `MinInnerSize` and grows the window to the logical minimum. | 200% screenshots at 1440x800, 1440x900 and 1440x1000; all pages in `docs/audit/layout/`. |
| F-013 | P1 | Interaction | `gui/src/ui/settings.rs` | Dragging the new interface-size slider applied the zoom on every frame. The zoom changed the slider's geometry under the pointer, feeding back into the value; one short drag from 100% could jump to 50% or 200%. | The percentage still previews live, but the zoom is applied on release (or immediately for a click, wheel or arrow key). Added `scale_applies` and a unit test for the drag-stopped transition. | Virtual evdev pointer drag: 20 px right from 100% showed 120% while the UI stayed at 100% scale; on release the UI moved to 120%; a 40 px drag landed at 140%, not 200%. |

## Gate status

| Gate | Open P0 | Open P1 | Status |
|---|---:|---:|---|
| G0 Baseline | 0 | 0 | Evidence collection in progress |
| G1 Interface size | 0 | 0 | Shipped `v0.9.0-gui3.20.44`; patch `v0.9.0-gui3.20.45` fixes F-013; both verified |
| G2 UI/frontend | 0 | 0 | Shipped `v0.9.0-gui3.20.46`; UI P1s closed; G3/G4 carry-over tracked below |
| G3 Backend/contract | 0 | 0 | Shipped `v0.9.0-gui3.20.47`; engine timeout/lock/state P1s closed; UI logout/pause-race items carried |
| G4 Security/legal/release | 0 | 0 | Shipped `v0.9.0-gui3.20.48`; 0 cargo-audit vulnerabilities, action pins and 0600 config tests verified |
| G5 Consolidation | 0 | 0 | Shipped `v0.9.0-gui3.20.49`, patches `v0.9.0-gui3.20.50`, `v0.9.0-gui3.20.51` and maintenance `v0.9.0-gui3.20.52`; all implementation findings closed or accepted |

## G2 release — 3.20.46 (fixed)

| ID | Sev | Area | Finding | Fix | Verified |
|---|---|---|---|---|---|
| F-014 | P1 | Files selection | Right-clicking a row built the menu from the previous `app.cloud.sel`; the empty-space menu appeared with no selection, and actions could target the wrong file. | A secondary click over the row selects it before the cell/row context menu is built. | Right-click Projects selected it (accent row) and showed Open/Rename/Duplicate/Details/Move to trash. |
| F-015 | P1 | Files selection | Shift-range used positions in the raw entry list but sliced the sorted list, ignoring the search filter. | The range now uses the same sorted+filtered vector the table draws. | Shift-click Photos→notes.md selected Photos, Projects, root, notes.md (4), not Documents. |
| F-016 | P1 | Photos layout | With a selection at 720x400 the hero and metric cards left almost no height for the grid. | The hero and metrics collapse while a selection is active. | Selection card and grid remain usable at 720x400. |
| F-017 | P1 | Photos layout | `grid_metrics` bounced 1→2→1 columns for widths 221–227 and could hang the UI. | One column returns at `MAX_TILE` instead of growing; regression test covers the edge. | `grid_metrics` test passes for 220/221/224/227/228/260. |
| F-018 | P1 | Keyboard focus | Tab into an off-screen About/Settings/Home/Photos control left the focus ring and control invisible. | `scroll_focused_into_view` scrolls a newly focused control into the page scroll area. | Tab to About "Check now" at 720x400 scrolled the Updates card into view. |
| F-019 | P2 | Contrast | Checkbox/input/button borders used the decorative card hairline (1.28:1 light, 1.30:1 dark). | Dedicated control borders at ≥3:1 for both themes; contrast test added. | `palette_meets_wcag_contrast` passes. |
| F-020 | P2 | Sidebar | Nav rows did not scroll into view on focus and accepted only Enter. | `scroll_to_me` on focus; Space activates like Enter; `my_files_row` follows. | 720x400 Tab to Settings leaves the focused row fully visible. |
| F-021 | P2 | Accessibility | Page titles and section labels were plain labels. | `page_title` and `section_label` expose `Role::Heading`. | Headings present on Home, Files, Photos, Activity, Settings, About. |
| F-022 | P2 | Accessibility | Account row only announced “Account menu”. | The accessible name and hover include the signed-in email. | Code review; `cargo clippy` clean. |
| F-023 | P2 | UX overlap | `PROTON_SYNC_WORKERS` had two controls with different labels. | Fine-tuning now only has Retries; Performance owns Scan workers. | Advanced tab at 720x400 shows one workers control. |
| F-024 | P3 | Consistency | Small preset 93% did not align with the 5% slider step. | Small is 95%. | Appearance shows 95% on selection. |
| F-025 | P2 | Accessibility | Selective-sync folder name was not clickable. | The name label toggles the checkbox and shows a pointing cursor. | Code review; no layout change. |
| F-026 | P2 | Files selection | Select all included files hidden by the search filter. | Selects the filtered set the table shows. | Code review. |
| F-027 | P2 | Service state | Saving settings restarted a paused engine, silently resuming deletes. | Save skips restart while the paused marker is present. | Code review; hint says "Saved. Syncing stays paused." |
| F-028 | P3 | Layout | About header allocated 270 pt for buttons needing ~282 pt. | Box is 300 pt. | Header fits at 720x400. |
| F-029 | P3 | Layout | Metric values could wrap; duplicate GitHub links. | Values truncate with a full-value hover; the Credits duplicate link is removed. | Code review. |

## Cross-gate carry-over (G3/G4)

| ID | Sev | Area | Finding | Target |
|---|---|---|---|---|
| F-044 | P1 | CLI session | The GUI CLI lock times out after 60 s and runs unprotected; `auth login` holds the same lock for up to 300 s, so a slow sign-in can race a `fs list` and lose the keyring session. | G3/G4 |
| F-045 | P1 | State paths | The service unit did not pin `XDG_STATE_HOME`/`XDG_CONFIG_HOME`; a GUI launched with custom XDG dirs could split health/wake/lock files from the engine. | G3 (fixed in working tree: unit pins both, relative XDG values rejected) |
| F-046 | P2 | Identity | `fetch_identity` maps every non-zero CLI result to signed-out, including busy locks and network failures. | G3 |
| F-047 | P2 | Logout | `logout` discards the CLI result; the UI clears identity even if sign-out failed. | G3 |
| F-048 | P2 | Pause | `pause_for` skips `stop()` when `systemd-run` is unavailable or fails. | G3 |
| F-049 | P2 | Service control | Pause/resume/save spawn independent workers with no serialization. | G3 |
| F-050 | P2 | Service unit | `unit_appimage` accepts non-executable files; unit writes are not atomic. | G3 |
| F-051 | P2 | Desktop | Autostart/menu helpers discard write/remove errors. | G3 |
| F-052 | P2 | Symlinks | `folder_stats` follows symlinks, unlike the engine scanner. | G3 |
| F-053 | P2 | CLI resolution | GUI resolver ignores `PROTON_DRIVE_BIN` and does not require an executable. | G3 |
| F-054 | P2 | Symlink write | `arm_photo_hold` and the audit-request write can follow a symlink. | G4 |
| F-055 | P2 | Files UI | Status/error line is hidden while a selection is active. | G3 |
| F-056 | P2 | Files UI | Context-menu-only actions lack a keyboard path; table roles are flat. | G3/G5 |
| F-057 | P2 | Photos | Duplicate-name photos are reported as “already on this computer” even when no local copy exists. | G3 |
| F-061 | P1 | Engine timeout | The engine `run_output` ignored its timeout parameter and could block the CLI slot forever on a hung command. | G3 (fixed in working tree: poll/kill + test) |
| F-062 | P2 | CLI lock | `with_cli_lock` claimed re-entrancy but never set the thread depth on the first lock. | G3 (fixed in working tree) |
| F-063 | P2 | Duplication | `state_dir()` was copied in four engine modules. | G3 (fixed in working tree: one `health::state_dir`) |

## G3 release — 3.20.47 (fixed)

| ID | Sev | Area | Finding | Fix | Verified |
|---|---|---|---|---|---|
| F-061 | P1 | Engine | `run_output` ignored its timeout and could block the CLI slot forever on a hung command. | Poll/kill with a deadline; metadata calls bounded at 180 s, transfers at 3600 s. | `a_hung_command_is_killed_at_the_deadline` (1 s). |
| F-062 | P2 | CLI lock | `with_cli_lock` never set the thread depth on the first lock, so the advertised re-entrant path was unreachable. | Depth is set to 1 before the closure. | Code review; nested calls now take the inner path. |
| F-063 | P2 | Duplication | `state_dir()` was copied in four engine modules. | One `health::state_dir()` shared by cli/execute/loop. | `cargo test --workspace`. |
| F-045 | P1 | State paths | The unit did not pin XDG dirs; a custom-XDG GUI could split state from the engine. | Unit pins `XDG_STATE_HOME`/`XDG_CONFIG_HOME`; relative/empty XDG values are rejected. | Unit template test and smoke. |
| F-044 | P1 | CLI session | `auth login` could time out of its lock and the sign-in probe could run unprotected after 60 s. | Login never runs without the lock; the sign-in identity probe waits 330 s. | Login-script test; code review. Remaining fallback for non-identity calls is tracked for G4. |
| F-046 | P2 | Identity | Any non-zero CLI result looked like signed-out. | Transient errors are kept as errors; the UI treats them as unknown instead of welcome. | Code review. |
| F-048 | P2 | Pause | `pause_for` skipped `stop()` when the autoresume timer failed. | It always stops; the timer failure only loses auto-resume. | Code review. |
| F-050 | P2 | Service unit | A non-executable file could become `ExecStart`; unit writes were non-atomic. | Executable-only AppImage selection; temp+rename unit write; reload failure is reported. | Service tests. |
| F-052 | P2 | Symlinks | `folder_stats` followed symlinks outside the sync root. | Uses `symlink_metadata`, skips symlinks. | `folder_stats_skips_symlinks...` test. |
| F-053 | P2 | CLI resolution | GUI ignored `PROTON_DRIVE_BIN` and accepted non-executables; engine accepted non-executables. | Both resolvers prefer `PROTON_DRIVE_BIN` and require an executable file. | Code review. |
| F-054 | P2 | Symlink write | `arm_photo_hold` could truncate a symlink target. | A symlink marker is removed before writing. | Code review. |
| F-059 | P3 | Menu paths | Menu entries hardcoded `~/.local/share`. | Uses `dirs::data_dir()` with an absolute-path fallback. | Code review. |

### Remaining open after G3

F-047 (logout ignores errors), F-049 (pause/resume worker race), F-051 (autostart/menu errors), F-055 (status hidden with selection), F-056 (context-menu keyboard path), F-057 (duplicate-name photo copy), F-058 (non-UTF8 paths), F-060 (selective folder error). F-004/F-005/F-006/F-007 remain for G4/G5.

## G4 release — 3.20.48 (fixed)

| ID | Sev | Area | Finding | Fix | Verified |
|---|---|---|---|---|---|
| F-009 | P2 | Security | `proton-sync.conf`/prefs were created with the default umask and can hold proxy credentials. | Unix writes set mode 0600; core test checks the mode. | `config_file_is_private`. |
| F-064 | P2 | CI supply chain | Release workflow used movable action tags (`@v7`, `@v6`). | Checkout, cache and artifact actions pinned to commit SHAs. | CI run 42477805162 passed all jobs with the pins. |
| F-065 | P1 | Dependencies | `cargo audit` found rustls-webpki 0.103.15 (RUSTSEC-2026-0382) and webpki-roots 1.0.9 (RUSTSEC-2026-0381). | Updated to 0.103.16 and 1.0.10. | `cargo audit`: 0 vulnerabilities. |
| F-066 | P3 | Dependencies | Three zbus-family crates are flagged unmaintained. | No known vulnerability; documented with an upgrade path. | Accepted in `LICENSE-REVIEW.md`. |

## Remaining G5 backlog

F-047 (logout result, fixed in working tree for 3.20.49), F-049 (pause/resume
worker race), F-051 (autostart/menu errors, fixed in working tree), F-055
(status hidden during selection), F-056 (context-menu keyboard path, Ctrl+A
partial), F-057 (duplicate-name photo copy), F-058 (non-UTF8 paths), F-060
(selective-folder error). F-004 is the formatter decision; F-005/F-006 are
accepted product decisions.

## G5 release — 3.20.49 (fixed)

| ID | Sev | Area | Finding | Fix | Verified |
|---|---|---|---|---|---|
| F-070 | P0 | Data safety | A planned download could overwrite a local edit made after the scan. | Re-check the plan-time fingerprint before placement; back up/replace/restore the file; no-replace hard links for new paths; `MvLocal` refuses an appeared destination. | `staged_download_does_not_overwrite...`, `staged_download_replaces_an_unchanged_file`. |
| F-071 | P1 | Deletions | Delete propagation ignored an edit on the surviving side. | Trash only when the surviving side still matches state; otherwise restore/upload it. | `plan` tests; code review. |
| F-072 | P1 | Scan | Read errors, symlinks, special files and malformed listings looked like deletions. | Scan fails closed; listings fail on non-arrays/undecryptable/unknown nodes. | `scan` tests; fake CLI suite. |
| F-073 | P1 | Wipe guard | Libraries under five files were not protected by the empty-side guard. | Guard arms on any non-empty history (`state_local > 0`). | `wipe_guard_protects_a_small_library_too`. |
| F-074 | P1 | Approval | Folder trashes counted only state-known children, so the approval could show 0 while trashing a subtree. | Every descendant is included in `candidate_cloud`. | Plan tests. |
| F-075 | P1 | Moves | A cloud rename was paired with a locally edited survivor. | Pair only when every state-known local fingerprint is unchanged. | Plan tests. |
| F-076 | P1 | Fast path | `FastEdit` wrote downloads/state without the planner's conflict checks. | Removed; event updates use the normal scoped listing + planner. | `loop_` tests. |
| F-077 | P1 | State scope | State and cloud caches were not scoped to roots/account. | `sync-roots` archives history on a root change; sign-out clears state and cloud caches; root change resets the GUI. | Root guard test; code review. |
| F-078 | P1 | CLI lock | Cloud mutations bypassed the shared CLI lock; the timeout fallback ran unprotected. | `cli::run_owned` holds the lock and fails closed; Cloud uses it. | Code review; fake CLI tests. |
| F-079 | P1 | Downloads | Temp dirs were predictable/0755 and left behind; moves followed symlinks/overwrote. | 0700 unique RAII temp dir, recursive collision-safe placement, temp-file rename. | Code review; cloud tests. |
| F-080 | P1 | Trash | Trash view reused live rows/selection; duplicate names shared one key. | Views clear on switch; trash rows use node UIDs. | Code review. |
| F-081 | P1 | Sidebar | A failed tree listing spawned a CLI process every frame and stale results overwrote newer data. | Failure cooldown + request timestamp guard. | Code review. |
| F-082 | P1 | Snapshot | `publish_snapshot` called the CLI outside the lock. | `run_json(..., true)`. | Code review; tests. |
| F-083 | P1 | Journal/health | systemd `+0800` timestamps failed, notification verbs matched file names, and failed transfers still said “up to date”. | Precise timestamps/normalization, leading-verb classification, engine writes `error` on failure. | Journal tests; loop tests. |
| F-084 | P2 | Paths | Engine still accepted relative/empty XDG values and raw data dirs. | One absolute resolver shared by GUI/engine; `dirs::data_dir()`. | Code review. |
| F-085 | P2 | UI/desktop | Theme detection order, tray “Open”, update recovery before the lock, startup tray notification, portal cancel, icon self-copy. | Targeted fixes. | Tests/code review. |
| F-093 | P2 | AppRun | A non-UTF-8 first operand was mistaken for “no argument” and sent to the GUI, whose `args()` aborts on non-Unicode. | Dispatch on `OsStr`; non-UTF-8 goes to the CLI; exec errors use 126 vs 127. | `non_utf8_operands_go_to_the_cli`. |
| F-094 | P2 | Tray | A same-session peer owning the predictable StatusNotifierItem bus name triggered an upstream assert; release builds abort. | Vendored ksni returns `NameTaken` as `Error::Dbus`; tray becomes unavailable. | Code review; vendor patch documented. |

## G5 residual patch — 3.20.51 (fixed)

| ID | Sev | Area | Finding | Fix | Verified |
|---|---|---|---|---|---|
| F-086 | P2 | Approval | The deletion-approval digest was unkeyed 64-bit FNV and not bound to the sync roots. | SHA-256 over the side and exact path set; `local_root`/`remote_root` are stored in the approval and matched before it can be consumed; missing or different roots fail closed. | `delete_approval::tests`; `approval_must_match_the_reviewed_paths_and_never_a_wipe`. |
| F-087 | P2 | Keyring | Any Proton auth-session label was accepted, so two sessions could route quota/event calls to the wrong account. | The exact CLI item is preferred; one renamed auth-session is tolerated; two ambiguous sessions fail closed. | Code review; clippy clean. |
| F-088 | P2 | State | A post-download edit before the state rescan could be stamped as synced. | The fingerprint captured immediately after placement is carried into `state_update` and stored instead of the later rescan value. | `downloaded_fingerprint_is_not_replaced_by_a_later_rescan_edit`; execute tests. |
| F-091 | P2 | Service | Pause/resume workers were not serialized; a fast pause→resume could leave the engine in the opposite state. | One service-control mutex plus a desired-state bit: the last click wins and the worker re-checks the bit under the lock. | Code review; `service::tests`; clippy clean. |

## Post-audit maintenance — 3.20.52 (fixed)

| ID | Sev | Area | Finding | Fix | Verified |
|---|---|---|---|---|---|
| F-089 | P2 | Uploads | The upload target was read when the picker closed, so a late listing or navigation could retarget it. | `UploadFiles`/`UploadFolder` carry the target captured when the picker opens; drag-and-drop still uses the drop-time folder. | Code review; `cargo test --workspace`; smoke. |
| F-090 | P2 | Keyboard/status | Context-menu-only actions had no keyboard path, and the status/error line disappeared while a selection was active. | The selection bar has a keyboard-reachable **More** menu (Open, Rename, Duplicate, Details, Move), **F2** rename, Ctrl+A in the shortcut list, and a persistent status/error label. | Code review; 720x400 layout reserve; clippy. |
| F-092 | P3 | Pickers/sidebar/launch | Non-UTF-8 paths were lossy, picker unavailability looked like cancel, a second launch during quit could lose the app, and tree labels used a fixed character count. | Non-UTF-8 paths are refused visibly; `PickResult` distinguishes cancel from unavailable; the second launch waits for the lock; tree labels elide to width with a full-name hover. | Portal tests; code review; clippy. |
| F-095 | P3 | Settings | Changing a sync root archived history without warning; 50% had no readability hint. | The save confirmation explains the archive/cache reset; Appearance warns at 50%. | Code review; 720x400/50% screenshots. |

## Remaining findings

No P0, P1, P2 or P3 implementation finding is open. F-004 (formatter) is an
accepted deviation; F-005/F-006/F-012 are accepted product/migration notes.
The residual risks are listed in `FINAL-REPORT.md` section 6.
