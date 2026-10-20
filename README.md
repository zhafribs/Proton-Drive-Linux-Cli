# Proton Drive for Linux — AppImage

![Latest release](https://img.shields.io/github/v/release/zhafribs/Proton-Drive-Linux-Cli)
![License: MIT](https://img.shields.io/badge/license-MIT-blue)
![Arch: x86_64](https://img.shields.io/badge/arch-x86_64-lightgrey)
![Proton Drive CLI: 0.9.0](https://img.shields.io/badge/Proton%20Drive%20CLI-0.9.0-6D4AFF)

One double-clickable file that gives Linux what Proton doesn't ship: an
**unofficial desktop app** for Proton Drive — two-way folder sync, a cloud file
manager, tray, notifications and in-app updates — built on Proton's
**official, unmodified Drive CLI**. Nothing to install: no AUR, no Python
setup, no Qt — the app and its sync runtime travel inside the AppImage.

> Unofficial. Not affiliated with, endorsed by, sponsored or supported by
> Proton AG. "Proton Drive" is a trademark of Proton AG.

**Get it:** [`ProtonDrive-CLI-0.9.0-GUI-3.16.3-x86_64.AppImage`](https://github.com/zhafribs/Proton-Drive-Linux-Cli/releases/latest)
(≈80 MB · x86_64 · [all releases](https://github.com/zhafribs/Proton-Drive-Linux-Cli/releases))

| Inside the file | Version |
| --- | --- |
| Proton Drive CLI — official, unmodified, SHA-512-verified at build | **0.9.0** (2026-10-05) |
| Desktop app + folder-sync engine (this repo) | **3.16.3** |

## Start here

1. Download the AppImage and make it executable once:
   ```bash
   chmod +x ProtonDrive-CLI-0.9.0-GUI-3.16.3-x86_64.AppImage
   ```
2. Double-click it. Click **Sign in** — Proton's official login opens in a
   terminal (that's how their CLI does 2FA); the app picks the session up.
3. Settings → pick your sync folder → **Save**. The first save installs the
   background sync service; from then on `~/ProtonDrive` mirrors your Drive
   and the tray icon watches it.

Works on any x86_64 desktop with **glibc ≥ 2.28** (Debian 10 / Ubuntu 20.04 /
Fedora 32 era or newer). If it won't start you're missing FUSE — Arch
`pacman -S fuse2` · Debian/Ubuntu `apt install libfuse2t64` · Fedora
`dnf install fuse-libs` — or launch with `APPIMAGE_EXTRACT_AND_RUN=1`.

The upstream CLI is untouched — arguments pass straight through:

```bash
./ProtonDrive-CLI-0.9.0-GUI-3.16.3-x86_64.AppImage --help
./ProtonDrive-CLI-0.9.0-GUI-3.16.3-x86_64.AppImage auth login   # the app's sign-in
./ProtonDrive-CLI-0.9.0-GUI-3.16.3-x86_64.AppImage filesystem list /
```

`auth login`, `filesystem …`, `sharing …`, `photo timeline`, `takeout run …` —
full list in `--help`. Put it on your PATH as `proton-drive` with
`./install-command.sh` (undo: `rm ~/.local/bin/proton-drive`).

## Screenshots

| Home | My files | Activity | Settings |
|:---:|:---:|:---:|:---:|
| ![Home](docs/screenshots/home.png) | ![My files](docs/screenshots/cloud.png) | ![Activity](docs/screenshots/activity.png) | ![Settings](docs/screenshots/settings.png) |

*Light theme shown; dark and system themes are built in. Compact floating
window (940×780 default, no page ever scrolls sideways, size remembered).*

## What the app does

* **Folder sync** — `~/ProtonDrive` mirrors the `/my-files` section of your
  Drive both ways: local edits upload within seconds (inotify), cloud changes
  appear in ~10 s (Proton's own change events), and renames/moves are
  followed **server-side without re-uploading**. Install:
  `./scripts/install-sync.sh` (systemd *user* service, no sudo); log:
  `journalctl --user -u proton-sync -f`; try it first with
  `python3 scripts/proton-sync.py --dry-run`.
* **My files** — a full cloud file manager: upload files/folders, download,
  rename, move, multi-select, sortable columns, back/forward + breadcrumbs,
  and a **Trash** view where restore is one click.
* **Home** — one big plain-language status with the sync orb, *Open my
  folder* / *Sync now* / *Pause* (15 min, 1 h, until resumed), folder size
  and recently synced files.
* **Activity** — what moved recently (uploaded / downloaded / removed /
  conflicts) with search and filters, plus a raw log tab.
* **Settings** — selective sync (tick which cloud folders mirror), never-sync
  patterns (`*.tmp`), instant upload, cloud checks as fast as every 30 s,
  notifications with a test button, light/dark/system theme, tray and
  start-at-login behaviour, one-click menu install, and **Updates** that
  check GitHub releases and *Download and restart* (the download is verified
  against the published SHA-256 before it replaces the file).

**Sync safety rules, baked in**

* Deletions follow you **both ways — but only ever to trash**, never
  permanent (off with `PROTON_SYNC_DELETES=0`): a local delete goes to
  Proton's trash, a cloud delete to your desktop trash — both restorable.
* A sick scan can never mass-delete: if one side comes back empty while the
  other has files (moved disk, broken login, wrong path), every deletion is
  held for that cycle and logged.
* Conflicts keep **both** versions (`name (conflict <timestamp>)`), and
  uploads are new revisions — the old version stays in Proton's history.
* Proton Docs/Sheets can't be downloaded by the CLI; they are skipped.

Tunables: `PROTON_SYNC_LOCAL/REMOTE/INTERVAL/FULL_EVERY/DELETES/WORKERS/
SUBFOLDERS/EXCLUDE/INOTIFY/…` — settings live in
`~/.config/proton-sync.conf` (edit by hand or in the app). Uninstall:
`./scripts/install-sync.sh uninstall` (folder and state kept).

### Good-to-know settings

* **Keep this window above other windows** — the app floats over your apps;
  on Wayland it runs through X11/XWayland because Wayland has no
  keep-above request.
* **Keep running when I close the window** — hides to the tray, syncing
  never stops (turn off to fully quit on close).
* **Start syncing when I log in** — enables the systemd user service at
  login; **Open Proton Drive when I log in** starts the app itself.
* **Also remove files I delete** — the both-ways-to-trash rule above.
* **tray icon** — status at a glance, notifications, right-click to quit.

## Optional: menu entry

```bash
./scripts/install-sync.sh    # the sync service (above)
./scripts/install-gui.sh     # app menu entry + icon for the current user
proton-drive-gui             # or search "Proton Drive" in your menu
```

No admin rights; works with the AppImage on Debian, Fedora and Arch. Inside
the AppImage the app uses the runtime bundled in the file; running these
repo scripts directly uses your distro's `python3` for the sync engine.

## Build it yourself

```bash
./scripts/check-upstream.sh   # refresh upstream.lock from proton.me (--dry-run to only report)
./build.sh                    # download -> SHA-512 verify -> build GUI -> package -> validate
./scripts/release-notes.sh    # render the release notes for the built artifacts
```

* **`upstream.lock`** is the single source of truth — CLI version + Proton's
  published SHA-512s, written only by `check-upstream.sh`. `build.sh`
  **aborts** if the downloaded binary doesn't match, so you never trust a
  number typed by hand.
* Build deps (nothing installed system-wide, no `sudo`): `bash`, `curl`/`wget`,
  `python3` with pip (used only to *download* wheels), `rsvg-convert`
  (`librsvg2-bin` / `librsvg`), optionally `desktop-file-validate` and
  `appstreamcli`. First run fetches `appimagetool` into `tools/`.
* GUI glibc floor is pinned at build time via `cargo-zigbuild` (glibc 2.28);
  a plain `cargo build` is the local-dev fallback.

### Release automation

`.github/workflows/release.yml` runs on a schedule and on push:

| Trigger | What happens |
| --- | --- |
| **every 6 h** | Checks Proton's page. New CLI → rewrites `upstream.lock`, commits with `[skip ci]`, builds, publishes a release tagged `v<cli>-gui<gui>`. Nothing new → validation only, no churn. |
| **push to main** (packaging/`gui/**` changed) | Rebuilds; a new `GUI_VERSION` publishes that pair's brand-new release. |
| **manual dispatch** | Same, plus `force_rebuild` to re-upload assets for the current version. |
| **pull request** | Validation build only — never commits or publishes. |

Releases are **never overwritten** (asset-existence check), older builds stay
downloadable, and a parsing failure on Proton's page turns the scheduled run
red instead of shipping a stale build. GitHub disables scheduled workflows
after ~60 idle days — re-enable under the Actions tab if the badge ever says
so. Local equivalent:
`./scripts/check-upstream.sh && ./build.sh && ./scripts/release-notes.sh`.

## Project layout

```
build.sh               download -> verify -> build GUI -> package -> validate
upstream.lock          CLI version + Proton's SHA-512s (written by check-upstream.sh only)
scripts/check-upstream.sh   refresh upstream.lock from proton.me (CI runs this)
scripts/release-notes.sh    render release-notes.md.tmpl for the built artifacts
scripts/proton-sync.py      two-way sync engine (stdlib only, runs on python3)
scripts/install-sync.sh     install/uninstall the sync as a systemd user service
scripts/install-gui.sh      menu entry + icon for a source checkout
scripts/fetch-desktop-deps.sh  pinned CPython + jeepney wheels for the AppImage
scripts/gen-licenses.sh     regenerate aggregated crate licence texts (cargo-about)
gui/                   Rust desktop app (egui): Home, My files, Activity, tray, updater
src/AppRun             AppImage entry point (argv pass-through)
src/*.desktop, src/*.appdata.xml   desktop entry + AppStream metadata
src/licenses/          Proton's MIT notice + bundled runtime licences (ship inside every build)
release-notes.md.tmpl  release notes template (hashes filled in at build time)
.github/workflows/release.yml  scheduled upstream check + build + publish
install-command.sh     symlink the AppImage into ~/.local/bin as proton-drive
LICENSE                MIT (this project's own code)
THIRD_PARTY_NOTICES.md full inventory of bundled components + trademarks
```

No binaries live in git: `download/`, `dist/`, `tools/`, `build/` are
git-ignored — the repo contains only scripts that fetch upstream bits from
`proton.me` at build time.

## License and legal

* **This repo's own code** (Rust GUI, build scripts, sync engine, metadata,
  icon) is **MIT** — see [`LICENSE`](LICENSE).
* **The payload** (the Proton Drive CLI binary inside each AppImage) belongs
  to Proton AG and is **MIT, © 2025-2026 Proton AG**; its source is the SDK
  Proton publishes at <https://github.com/ProtonDriveApps/sdk>. MIT's notice
  condition is satisfied by
  [`src/licenses/Proton-Drive-SDK-MIT.txt`](src/licenses/Proton-Drive-SDK-MIT.txt),
  which `build.sh` installs into every AppImage at
  `usr/share/licenses/proton-drive/LICENSE`.
* **Bundled runtime** — portable CPython (**PSF-2.0**) for the sync engine
  and jeepney (**MIT**), plus the GUI binary's crate graph (~450 crates,
  permissive licences) with every licence text generated from
  `gui/Cargo.lock` (`scripts/gen-licenses.sh`). All texts ship inside every
  AppImage at `usr/share/licenses/`; inventory:
  [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md). No copyleft: since
  GUI 3.14.0 the AppImage carries no Qt, no PyQt6 and no GPL-3.0-only
  component.
* **Binary integrity** — payloads are byte-identical to Proton's downloads
  (`build.sh` aborts on SHA-512 mismatch); nothing is patched or re-signed.
* **Trademarks** — "Proton", "Proton Drive" and the Proton logo are
  trademarks of Proton AG, used here only to identify the packaged product
  (nominative use); the glyph is from simple-icons (CC0). **Unofficial:
  not affiliated with, endorsed, sponsored or supported by Proton AG** —
  this disclaimer also ships in the AppStream metadata, desktop entry and
  release notes.
* Not legal advice — for zero exposure, run `./build.sh` yourself: the
  script takes the binaries straight from Proton.

## Why unofficial?

Proton ships **no Linux desktop client** — only this CLI. Their other
clients' sources are public at <https://github.com/ProtonDriveApps>
(`windows-drive` C#/GPLv3, `mac-drive` Swift/GPLv3, `android-drive`
Kotlin/GPLv3, plus the MIT `sdk` this CLI is built on) — but there is no
Linux client to build, and no reverse engineering is needed to package the
official CLI and put an original, permissively-licensed app on top of it.
For daily use you don't even need the window: see
[folder sync above](#what-the-app-does).
