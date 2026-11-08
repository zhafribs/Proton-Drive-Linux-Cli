# File and surface coverage ledger

**Program:** Full product audit, 2026-11-08  
**Rule:** every Rust file is read by a council; high-risk files get a second
independent read. A file is not marked *Read* until the whole file has been
reviewed against its callers and tests.  
**Inventory:** 60 Rust files, 33,244 lines, excluding
`target/` and vendored build output.  
**Status:** G0 inventory complete; reads begin at G2. G1 touched the
interface-size path and marked it *Partial*.

Legend: **H** = high risk (credentials, destructive file operations,
subprocesses, update trust, paths, state/locks, DBus). **M** = UI or shared
behaviour. **L** = small/leaf module. *Double-read* is required for every H
file.

## Rust files

| # | File | Lines | Risk | Owner | First read | Double-read | Findings / notes |
|---:|---|---:|---|---|---|---|---|
| 1 | `crates/apprun/src/main.rs` | 86 | M | Engineering | Pending | No |  |
| 2 | `crates/core/src/conf.rs` | 200 | L | Engineering | Pending | No |  |
| 3 | `crates/core/src/delete_approval.rs` | 139 | H | Trust | Pending | Yes |  |
| 4 | `crates/core/src/health.rs` | 190 | L | Engineering | Pending | No |  |
| 5 | `crates/core/src/lib.rs` | 12 | L | Engineering | Pending | No |  |
| 6 | `crates/core/src/snapshot.rs` | 214 | L | Engineering | Pending | No |  |
| 7 | `crates/core/src/state.rs` | 187 | H | Trust | Pending | Yes |  |
| 8 | `crates/core/src/time.rs` | 28 | L | Engineering | Pending | No |  |
| 9 | `crates/engine/src/action.rs` | 363 | L | Engineering | Pending | No |  |
| 10 | `crates/engine/src/cli.rs` | 270 | H | Trust | Pending | Yes |  |
| 11 | `crates/engine/src/events.rs` | 513 | H | Trust | Partial (G1) | Yes | Interface-size path read; full-file review still pending. |
| 12 | `crates/engine/src/execute.rs` | 789 | H | Trust | Pending | Yes |  |
| 13 | `crates/engine/src/filter.rs` | 168 | L | Engineering | Pending | No |  |
| 14 | `crates/engine/src/firstsync.rs` | 278 | L | Engineering | Pending | No |  |
| 15 | `crates/engine/src/health.rs` | 68 | L | Engineering | Pending | No |  |
| 16 | `crates/engine/src/lib.rs` | 21 | L | Engineering | Pending | No |  |
| 17 | `crates/engine/src/loop_.rs` | 1220 | H | Trust | Pending | Yes |  |
| 18 | `crates/engine/src/main.rs` | 222 | M | Engineering | Pending | No |  |
| 19 | `crates/engine/src/photos.rs` | 2716 | L | Engineering | Pending | No |  |
| 20 | `crates/engine/src/plan.rs` | 1152 | L | Engineering | Pending | No |  |
| 21 | `crates/engine/src/policy.rs` | 99 | H | Trust | Pending | Yes |  |
| 22 | `crates/engine/src/push.rs` | 72 | L | Engineering | Pending | No |  |
| 23 | `crates/engine/src/quota.rs` | 114 | H | Trust | Partial (G1) | Yes | Interface-size path read; full-file review still pending. |
| 24 | `crates/engine/src/scan.rs` | 598 | L | Engineering | Pending | No |  |
| 25 | `crates/engine/src/state_update.rs` | 312 | H | Trust | Pending | Yes |  |
| 26 | `crates/engine/tests/fake_cli.rs` | 439 | L | Secretariat | Pending | No |  |
| 27 | `gui/src/clilock.rs` | 98 | H | Trust | Read (G2) | Yes | Whole-file read during the G2 UI/security review. |
| 28 | `gui/src/cli.rs` | 799 | H | Trust | Partial (G1) | Yes | Interface-size path read; full-file review still pending. |
| 29 | `gui/src/conf.rs` | 6 | L | Engineering | Pending | No |  |
| 30 | `gui/src/desktop.rs` | 331 | L | Engineering | Pending | No |  |
| 31 | `gui/src/exec.rs` | 85 | H | Trust | Read (G2) | Yes | Whole-file read during the G2 UI/security review. |
| 32 | `gui/src/health.rs` | 498 | L | Engineering | Pending | No |  |
| 33 | `gui/src/journal.rs` | 258 | L | Engineering | Pending | No |  |
| 34 | `gui/src/main.rs` | 189 | H | Trust | Partial (G1) | Yes | Interface-size path read; full-file review still pending. |
| 35 | `gui/src/notify.rs` | 52 | L | Engineering | Pending | No |  |
| 36 | `gui/src/paths.rs` | 103 | H | Trust | Pending | Yes |  |
| 37 | `gui/src/portal.rs` | 228 | H | Trust | Pending | Yes |  |
| 38 | `gui/src/prefs.rs` | 319 | L | Engineering | Read (G2 start) | No | Whole file read with the interface-size migration. |
| 39 | `gui/src/release.rs` | 1549 | H | Trust | Read (G2 start) | Yes | Whole-file read; double-read still required.
| 40 | `gui/src/service.rs` | 377 | H | Trust | Pending | Yes |  |
| 41 | `gui/src/statewatch.rs` | 155 | L | Engineering | Pending | No |  |
| 42 | `gui/src/system_theme.rs` | 481 | L | Design | Pending | No |  |
| 43 | `gui/src/theme.rs` | 440 | L | Design | Pending | No |  |
| 44 | `gui/src/tray.rs` | 516 | H | Trust | Pending | Yes |  |
| 45 | `gui/src/ui/about.rs` | 611 | M | Design | Pending | No |  |
| 46 | `gui/src/ui/activity.rs` | 207 | M | Design | Read (G2) | No | Whole-file read during the G2 UI/security review. |
| 47 | `gui/src/ui/cloud.rs` | 2884 | H | Trust | Pending | Yes |  |
| 48 | `gui/src/ui/home.rs` | 442 | M | Design | Read (G2) | No | Whole-file read during the G2 UI/security review. |
| 49 | `gui/src/ui/mod.rs` | 3140 | H | Trust | Partial (G1) | Yes | Interface-size path read; full-file review still pending. |
| 50 | `gui/src/ui/photos.rs` | 1724 | M | Design | Pending | No |  |
| 51 | `gui/src/ui/settings.rs` | 1396 | H | Trust | Read (G2 start) | Yes | Whole-file read; double-read still required.
| 52 | `gui/src/ui/widgets.rs` | 948 | M | Design | Pending | No |  |
| 53 | `gui/src/util.rs` | 124 | L | Engineering | Pending | No |  |
| 54 | `vendor/ksni/src/blocking.rs` | 240 | M | Engineering | Pending | No |  |
| 55 | `vendor/ksni/src/compat.rs` | 314 | M | Engineering | Pending | No |  |
| 56 | `vendor/ksni/src/dbus_interface.rs` | 501 | H | Trust | Pending | Yes |  |
| 57 | `vendor/ksni/src/lib.rs` | 630 | M | Engineering | Pending | No |  |
| 58 | `vendor/ksni/src/menu.rs` | 1130 | M | Engineering | Pending | No |  |
| 59 | `vendor/ksni/src/service.rs` | 1843 | M | Engineering | Pending | No |  |
| 60 | `vendor/ksni/src/tray.rs` | 156 | H | Trust | Pending | Yes |  |

## UI surfaces

Every surface is tested at 50%, 100%, 150% and 200% where the platform can
display it, and at 720x540, 1024x720, 1280x800, 1440x900 and 1920x1080.

- [ ] Home: signed out, starting, synced, syncing, paused, needs login,
      deletes held, engine stopped, error.
- [ ] Files: My files, folder tree, search, trash view, empty/loading/error,
      context menus, multi-select, drag-and-drop.
- [ ] Photos: empty, timeline, selection, download progress, errors.
- [ ] Activity: Files moved, Technical log, filters, empty state.
- [ ] Settings: General, Sync, Notifications, Appearance, Advanced,
      App & updates; validation errors and save/discard.
- [ ] About: hero, update state, version cards, details, credits, legal.
- [ ] Dialogs: Shortcuts, Info, Confirm save, Confirm trash/delete/empty,
      held deletions, sign-out, folder picker, rename/new folder.
- [ ] Menus: account, folder tree, file row, tray.
- [ ] Toasts, notifications, window states (tray, hidden, always-on-top).

## CLI and engine commands

| Surface | Command(s) | Owner |
|---|---|---|
| Identity/version | `--version`, `fs list /my-files --json` | GUI |
| Root metadata | `fs info <remote>` | engine |
| Folder walk | `fs list <path> [--json]` | engine |
| Change fingerprint | `fs size <remote>` | engine |
| Upload | `fs upload -f create-new-revision -d merge` | engine |
| Download | `fs download <remote> <local>` | engine |
| Create folder | `fs create-folder <parent> <name>` | engine |
| Rename | `fs rename <path> <name>` | cloud UI |
| Move | `fs move <old> <parent> <name>` | engine/cloud UI |
| Trash | `fs trash <path>` | engine/cloud UI |
| Permanent delete | `fs delete <path>` | cloud UI (confirmed) |
| Sign out | `auth logout` | GUI |
| Photos timeline | `photo timeline -d [--json]` | engine |
| Photos download | `photo download -c rename` | engine/GUI |

## State files and locks

| Path | Writer | Reader | Purpose |
|---|---|---|---|
| `$XDG_STATE_HOME/proton-sync/gui.lock` | GUI | GUI | single GUI instance |
| `.../cli.lock` | engine/GUI | engine/GUI | serialize official CLI calls |
| `.../state.json` | engine | engine | local/remote history |
| `.../remote-snapshot.json` | engine | GUI/engine | remote tree cache |
| `.../deletes-approval.json` | GUI | engine | approved destructive batch |
| `.../trash/` | engine | engine | local trash staging |
| `.../health.json` | engine | GUI | status and cycle telemetry |
| `.../quota.json` | engine | GUI | account used/free quota |
| `.../photos-state.json`, `photos-sync.json`, `timeline.json` | engine | engine/GUI | photo sync state |
| `.../wake`, `audit-request`, `photos-request` | GUI | engine | one-shot requests |
| `.../paused`, `last-email` | GUI | GUI | pause marker and identity hint |
| `$XDG_CONFIG_HOME/proton-sync.conf` | GUI | GUI/engine service | sync settings |
| `$XDG_CONFIG_HOME/ProtonDriveCLI/ProtonDriveGUI.conf` | GUI | GUI | prefs (theme, scale, window) |
| `$XDG_CONFIG_HOME/systemd/user/proton-sync.service` | GUI | systemd | user service |

## Coverage summary

| Area | Files | Read | Double-read | Open findings |
|---|---:|---:|---:|---:|
| crates/core | 7 | 0 | 0 | 1 |
| crates/engine | 20 | 0 | 0 | 2 |
| gui | 27 | 7 | 0 | 8 |
| vendor/ksni | 7 | 0 | 0 | 0 |
| **Total** | **60** | **7** | **0** | **11** |

Fully read so far: `gui/src/prefs.rs`, `gui/src/ui/settings.rs`,
`gui/src/ui/home.rs`, `gui/src/ui/activity.rs`, `gui/src/release.rs`,
`gui/src/exec.rs`, `gui/src/clilock.rs`. Partial G1/G2/G3 reads:
`gui/src/ui/mod.rs`, `gui/src/ui/cloud.rs`, `gui/src/ui/photos.rs`,
`gui/src/ui/about.rs`, `gui/src/ui/widgets.rs`, `gui/src/theme.rs`,
`gui/src/main.rs`, `gui/src/cli.rs`, `crates/engine/src/cli.rs`,
`crates/engine/src/events.rs` and `crates/engine/src/quota.rs`.
