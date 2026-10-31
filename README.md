# Proton Drive for Linux — AppImage

![Latest release](https://img.shields.io/github/v/release/zhafribs/Proton-Drive-Linux-Cli)
![License: MIT](https://img.shields.io/badge/license-MIT-blue)
![Rust only](https://img.shields.io/badge/Rust-only-000?logo=rust)
![Arch: x86_64](https://img.shields.io/badge/arch-x86_64-lightgrey)
![Proton Drive CLI: 0.9.0](https://img.shields.io/badge/Proton%20Drive%20CLI-0.9.0-6D4AFF)

**An unofficial Proton Drive desktop app for Linux** — two-way folder sync, a
cloud file manager, tray, notifications and in-app updates, built on Proton's
**official, unmodified Drive CLI**.

The GUI, sync engine and AppImage launcher are **Rust**; the bundled CLI is
Proton's own Rust binary. Nothing is installed on the host — no Python, no Qt,
no AUR.

> Unofficial. Not affiliated with, endorsed, sponsored or supported by Proton
> AG. "Proton Drive" is a trademark of Proton AG.

**[Download the latest AppImage →](https://github.com/zhafribs/Proton-Drive-Linux-Cli/releases/latest)**

The releases page keeps the **seven most recent builds**; older ones are
removed automatically.

## Quick start

1. Download and make it executable:
   ```bash
   chmod +x ProtonDrive-CLI-*-x86_64.AppImage
   ```
2. Run it and click **Sign in** in the bottom-left account pill — Proton's CLI
   opens a terminal for 2FA.
3. **Settings → pick your sync folder → Save.** The app installs a systemd
   *user* service; from then on `~/ProtonDrive` mirrors your Drive.

Works on x86_64 Linux with **glibc ≥ 2.28** and FUSE. If it won't start:
install `libfuse2` / `fuse2` / `fuse-libs`, or run with
`APPIMAGE_EXTRACT_AND_RUN=1`.

## Screenshots

| Home | My files |
|:---:|:---:|
| ![Home](docs/screenshots/home.png) | ![My files](docs/screenshots/cloud.png) |

*Dark theme shown; light and system themes are built in.*

## Features

- **Folder sync** — `~/ProtonDrive` mirrors `/my-files` both ways: local edits
  upload within seconds (inotify), cloud changes are watched every 3 s with
  the check interval as fallback (**Instant** = 30 s; **Off** = manual), and
  renames/moves are followed server-side without re-uploading.
- **My files** — upload files/folders (or drag-and-drop them in), download,
  rename, move, multi-select, search, sortable columns, back/forward,
  breadcrumbs and a restorable **Trash**.
- **Home** — one plain-language status orb with pause (15 min, 1 h, until
  resumed), file count and disk usage in the sidebar, a live transfer progress
  bar and queue, and the activity feed (uploads, downloads, removals,
  conflicts) with search and a raw log tab.
- **Settings** — selective sync, never-sync patterns, instant upload, check
  intervals, notifications, light/dark/system theme, interface size, tray,
  start-at-login, menu install and **in-app updates** (SHA-256 verified).
- **Tray + notifications** — a plain icon when synced, a pulse while starting,
  syncing or signing in, and a red X when paused, signed out, stopped or in
  error. Click it to reopen the window (it floats on Hyprland). Closing the
  window keeps syncing in the background.
- **Account** — the bottom-left pill shows your email and live status; sign in
  or sign out from its menu.

**Safety built in:** deletes follow both ways but **only to trash** (never
permanent); a wipe guard holds mass deletions; conflicts keep both versions;
one app-wide lock protects the CLI's keyring session.

## Use the CLI directly

The upstream CLI is untouched — arguments pass straight through:

```bash
./ProtonDrive-CLI-*-x86_64.AppImage --help
./ProtonDrive-CLI-*-x86_64.AppImage auth login
./ProtonDrive-CLI-*-x86_64.AppImage filesystem list /my-files
```

Put it on your `PATH` as `proton-drive` with `./install-command.sh`.

## Build from source

Install Rust with [rustup](https://rustup.rs/), then `wget` and
`rsvg-convert` (optional: `desktop-file-validate`, `appstreamcli`):

```bash
./build.sh x86_64
```

`build.sh` downloads and **SHA-512-verifies** Proton's CLI, builds the Rust
GUI, sync engine and `AppRun` launcher, and packages the AppImage. No binaries
are committed. Full guide:
[docs/INSTALL-FROM-SOURCE.md](docs/INSTALL-FROM-SOURCE.md).

## Project layout

```
gui/                 egui desktop app: Home, My files, Settings, tray, updater
crates/core/         shared config / state / health / snapshot types
crates/engine/       two-way Rust sync engine
crates/apprun/       Rust AppImage entry point
build.sh             download -> verify -> build -> package -> validate
upstream.lock        Proton CLI version + SHA-512s (written by check-upstream.sh)
scripts/             upstream check, release notes, licence texts, dev install
```

## License

This project's own code is **MIT** — see [`LICENSE`](LICENSE). The bundled
Proton CLI is **MIT, © Proton AG**; its notice ships in every AppImage.
Third-party crate licences are listed in
[`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md) and inside the AppImage.
No Qt, PyQt6, CPython or GPL-only components.
