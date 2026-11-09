# Milestone M2 — Portable Sync

**Status:** complete — released as `v0.9.0-gui3.22.0`  
**Target release:** `v0.9.0-gui3.22.0` (one signed release)  
**Theme:** sync reliably without a systemd user session, and never run two
engines.

## Why

The systemd user unit is the right default on mainstream desktops, but a
systemd-less session left the GUI able to install configuration and never start
the engine. The engine also had no supervisor outside systemd, so a crash or a
GUI restart could leave sync stopped or spawn a duplicate.

## Scope

- **Supervisor fallback:** when `systemctl --user` is unavailable, the GUI
  spawns `proton-engine --loop` (or the AppImage's `__sync-engine` route) and
  watches it.
- **Crash recovery:** a monitor restarts the engine after an unexpected exit,
  with the engine's `sync.lock` as the final duplicate guard.
- **Adoption:** `supervisor.pid` lets a new GUI attach to an engine that
  survived a GUI restart; pause/restart still reach the process.
- **Logging:** `supervisor.log` captures engine output; the Activity journal
  falls back to it when `journalctl` is unavailable.
- **Pause/resume:** pause timers use an in-process timer in fallback mode;
  the persisted pause marker still wins at startup.
- **Autostart:** the existing XDG autostart entry starts the GUI, which then
  starts the supervised engine.
- **Recovery polish from 3.21.1:** refresh on entry, acknowledged-conflict
  filtering, trash usage, snapshot delete confirmation.

## Acceptance

| Check | Result |
|---|---|
| Engine starts without systemd | Manual: `PROTON_SYNC_SUPERVISOR=1`, PID + `health.json` present |
| Kill -9 engine | Manual: monitor restarted it with a new PID |
| GUI restart | Manual: adopted the same PID, no duplicate |
| Single-engine lock | Engine `sync.lock` still enforced |
| Systemd path | Unchanged; `PROTON_SYNC_SUPERVISOR=0` forces the unit path |
| Tests / clippy / gate | 297 tests, clippy clean, gate 12/12 |
| Release | Signed AppImage, smoke, GLIBC 2.28 |

## Lifecycle

Closing to the tray keeps the GUI and the supervised engine running. Choosing
Quit (tray or Ctrl+Q) stops the supervised engine and removes
`supervisor.pid`; the systemd service is independent and is not stopped.

A detached watchdog for the case where the GUI is killed abruptly remains a
candidate for a later milestone.

## Non-goals

- Replacing the systemd unit on systems that have one.
- A system-wide daemon or root service.
- aarch64 packaging.

## Release verification

- Tag: `v0.9.0-gui3.22.0` (29 releases retained).
- CI `42594134761`: verify and release succeeded.
- Checksum, Ed25519 signature, `--smoke`, GLIBC 2.28 verified.
- Manual: supervisor started the engine with `PROTON_SYNC_SUPERVISOR=1`,
  restarted it after `kill -9`, and a new GUI adopted the running PID.
- Evidence: `docs/audit/evidence/release-3.22.0.txt`.
- Patch `v0.9.0-gui3.24.1`: choosing Quit (tray or Ctrl+Q) now stops the
  supervised engine and removes `supervisor.pid`; closing to the tray keeps it
  running. Evidence: `docs/audit/evidence/release-3.24.1.txt`.

## Consolidation

`v0.9.0-gui3.25.0` hides the misleading systemd-only "Start syncing when I log
in" toggle in supervisor mode and points the user at the app's login entry.
