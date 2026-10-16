# Proton Drive CLI — AppImage (x86_64 + aarch64)

Unofficial AppImage packaging of the **official Proton Drive CLI** published by
Proton AG at <https://proton.me/download/drive/cli/index.html>.

Proton ships **no native Linux GUI** for Drive, and no Linux desktop app — only
this CLI. That is what this project packages: the unmodified upstream binary,
verified against Proton's published SHA-512 checksum before it is wrapped.

| Artifact | Arch | Size |
| --- | --- | --- |
| `dist/ProtonDrive-CLI-0.9.0-x86_64.AppImage` | x86_64 (Intel/AMD) | 40 MB |
| `dist/ProtonDrive-CLI-0.9.0-aarch64.AppImage` | aarch64 (ARM64) | 39 MB |
| `dist/SHA256SUMS` | checksums of the outputs | |

Upstream version: **0.9.0** (released 2026-10-05), `cli-drive@0.9.0+75612299`.

## Quick start

```bash
chmod +x dist/ProtonDrive-CLI-0.9.0-x86_64.AppImage
./dist/ProtonDrive-CLI-0.9.0-x86_64.AppImage --version
./dist/ProtonDrive-CLI-0.9.0-x86_64.AppImage --help
./dist/ProtonDrive-CLI-0.9.0-x86_64.AppImage auth login   # first run
```

Put it on your `PATH` as `proton-drive` (creates `~/.local/bin/proton-drive` → symlink):

```bash
./install-command.sh
```

Undo with `rm ~/.local/bin/proton-drive`.

Typical commands: `auth login`, `filesystem list /`, `filesystem upload <local> /`,
`filesystem download <remote> <local-folder>`, `sharing set-url ...`,
`photo timeline`, `takeout run <folder>`. Full list: `--help`.

## Folder sync (Google Drive-style)

Want your Drive as a normal folder in the file manager instead of typing
commands? The repo ships a two-way sync built **only on the official CLI** —
no AUR, no third-party binaries, nothing but this project's own MIT scripts:

```bash
./scripts/install-sync.sh     # installs a systemd *user* service (no sudo)
```

That keeps `~/ProtonDrive` in sync with the `/my-files` section of your Drive:

* **local edits upload within seconds** (inotify wakes the sync immediately),
  remote changes appear within 5 minutes (poll)
* runs at every login, restarts on failure, logs to the journal
  (`journalctl --user -u proton-sync -f`)
* dry run first: `python3 scripts/proton-sync.py --dry-run`

Safety rules baked in:

* **never deletes anything** (opt in with `PROTON_SYNC_DELETES=1`: local
  deletions go to Proton's trash, remote deletions to the desktop trash)
* deleting a file on one side keeps it on the other — no surprise data loss
* conflicts keep **both** versions (`name (conflict <timestamp>)`), and uploads
  use `create-new-revision`, so the old version stays in Proton's history
* Proton Docs/Sheets cannot be downloaded by the CLI; they are skipped

Knobs: `PROTON_SYNC_LOCAL`, `PROTON_SYNC_REMOTE`, `PROTON_SYNC_INTERVAL`
(seconds, default 300), `PROTON_SYNC_DELETES`, `PROTON_SYNC_WORKERS`,
`PROTON_SYNC_RETRIES`, `PROTON_SYNC_SUBFOLDERS` (comma-separated folders to
mirror instead of everything), `PROTON_SYNC_EXCLUDE` (comma-separated globs
like `*.tmp,.cache` that are never synced), `PROTON_SYNC_INOTIFY=0` (disable
the fast path). Uninstall:
`./scripts/install-sync.sh uninstall` (folder and state are kept).
Settings live in `~/.config/proton-sync.conf` - edit by hand or with the app.

## Desktop app (GUI)

```bash
./scripts/install-sync.sh    # the sync service first (see above)
./scripts/install-gui.sh     # then the app: menu entry + icon + launcher
proton-drive-gui             # or search "Proton Drive" in your app menu
```

A friendly Qt6 desktop app for the sync service (PyQt6 from the **official
repos** - no AUR, no bundled toolkits), designed for everyday users:

* **Home** - a big friendly status ("You're up to date" / "Syncing..." /
  "Sync is paused") with an animated status orb, one-click *Open my
  folder*, *Sync now* and *Pause* (15 minutes, 1 hour, or until you
  resume), your account, folder size and recently synced files
* **Activity** - what moved recently (uploaded / downloaded / removed /
  conflicts) with search and filters, plus a technical log tab
* **Settings** - plain-language options: which folder syncs, selective
  sync (browse the cloud and pick folders), never-sync patterns
  (`*.tmp`-style), instant upload, sync frequency, notifications with a
  test button, start at login, start-minimized-in-tray, light/dark/system
  theme - and an *Advanced* section for power users (connections, retries,
  extra environment variables, log access). Saving writes
  `~/.config/proton-sync.conf` and restarts syncing.
* **About** - what the app is, the good-to-know rules and the legal notes
* **tray icon** - status at a glance plus desktop notifications; closing
  the window keeps it running (right-click the tray icon to quit)

## Distro notes

Both upstream binaries require only **glibc ≥ 2.17**, so the same AppImage runs
on Debian/Ubuntu, Fedora, Arch/CachyOS and derivatives without bundling libs:

* **Arch / CachyOS** — works out of the box if `fuse2` is installed (this build
  was made and tested here, on CachyOS).
* **Debian / Ubuntu** — AppImages need the FUSE2 library:
  `sudo apt install libfuse2t64` (Debian 13+/Ubuntu 24.04+) or `libfuse2` (older).
* **Fedora** — `sudo dnf install fuse-libs` (provides `libfuse.so.2`).

No FUSE at all (containers, restricted systems, any distro):

```bash
APPIMAGE_EXTRACT_AND_RUN=1 ./dist/ProtonDrive-CLI-0.9.0-x86_64.AppImage --help
# or permanently:
./dist/ProtonDrive-CLI-0.9.0-x86_64.AppImage --appimage-extract
```

Desktop menu entry: install [AppImageLauncher](https://github.com/AppImageCommunity/AppImageLauncher)
or run `appimaged` — the AppImage ships a desktop file
(`Terminal=true`, so it opens in a terminal) plus 256/512 px icons and AppStream
metadata, all of which validate cleanly.

## Rebuilding / upgrading

```bash
./scripts/check-upstream.sh   # sync upstream.lock with Proton's latest release
./build.sh                    # both architectures
./build.sh x86_64             # one architecture
```

The version and its published checksums live in **`upstream.lock`** — you never
edit numbers by hand. `check-upstream.sh` parses Proton's download page,
rewrites the lock when a new release appears (or `--dry-run` to only report,
exit 3 = update available), and `build.sh` reads it and **aborts if the SHA-512
of the downloaded binary does not match**. A corrupt or incomplete lock fails
the build with a message telling you to run `check-upstream.sh`.

`build.sh` downloads `appimagetool` and the aarch64 runtime into `tools/` on
first run, fetches the upstream binaries, verifies them, then builds and
validates.

Dependencies (all non-root, nothing is installed system-wide):

* `wget` or `curl`, `bash`, `coreutils` — likely already present
* `rsvg-convert` (`pacman -S librsvg` / `apt install librsvg2-bin` / `dnf install librsvg2-tools`) — renders the icon
* `desktop-file-validate` (`desktop-file-utils`) and `appstreamcli` (`appstream`) — optional, used for validation

No `sudo` and no `mksquashfs` needed: appimagetool ships its own squashfs builder.

## Automatic updates (future-proofing)

The repository tracks upstream on its own — `.github/workflows/release.yml`:

| Trigger | What happens |
| --- | --- |
| **schedule** (every 6 h) | Runs `check-upstream.sh`. If Proton shipped a new CLI: rewrites `upstream.lock`, commits it with `[skip ci]`, builds both AppImages, renders notes from `release-notes.md.tmpl` and **publishes a release tagged `v<new version>`**. If nothing changed: validation build, no commit, no release churn. |
| **workflow_dispatch** (manual) | Same, plus a `force_rebuild` input that re-uploads assets and refreshes notes for the current version. |
| **push to main** (packaging changed) | Rebuilds and fills in any missing release assets. |
| **pull_request** | Validation only — builds and checks metadata, never commits or publishes. |

Design notes:

* **Idempotent**: the release step checks whether all three assets already
  exist and skips them, so a green run does not silently replace the AppImages
  people already downloaded (their hashes change on every rebuild).
* **Version-safe**: the tag always comes from `upstream.lock`, which only
  changes when Proton's page changes, so one release per upstream version.
* **Fails loudly**: if Proton restructures the page so nothing parses,
  `check-upstream.sh` exits 2 and the scheduled run turns red (GitHub emails
  you) instead of silently releasing a stale build.
* **Caching**: `tools/` and `download/` are cached, keyed on `upstream.lock`,
  so routine runs only pay for the squashfs build.
* **Caveat**: GitHub disables scheduled workflows after ~60 days without
  repository activity. A commit/release interaction resets that clock; if the
  badge ever says "workflow disabled", re-enable it under the Actions tab.

Adding a future platform (e.g. Proton's `linux-x64-musl` build) = add
`SHA512_LINUX_X64_MUSL=…` to `upstream.lock`, map it in `APPIMAGE_ARCH` in
`build.sh`, and add it to `REQUIRED_PLATFORMS` in
`scripts/check-upstream.sh` — the checker already warns about upstream
platforms it does not package, so new ones are visible in every run log.

Local equivalent of CI: `./scripts/check-upstream.sh && ./build.sh && ./scripts/release-notes.sh`.

## Project layout

```
build.sh          reproducible build script (download -> verify -> package -> validate)
upstream.lock     VERSION + Proton's published SHA-512s - written by check-upstream.sh only
scripts/check-upstream.sh   parse proton.me, refresh upstream.lock (CI runs this)
scripts/release-notes.sh    render release-notes.md.tmpl for the built artifacts
scripts/proton-sync.py      two-way folder sync on top of the official CLI (stdlib only)
scripts/install-sync.sh     install/uninstall the sync as a systemd user service
scripts/proton-sync-gui.py  Qt6 desktop app (dashboard, logs, settings, tray)
scripts/install-gui.sh      install the app: launcher + menu entry + icon
release-notes.md.tmpl       release notes template (hashes filled in at build time)
.github/workflows/release.yml  scheduled upstream check + build + publish
install-command.sh  symlink the built AppImage into ~/.local/bin
LICENSE           MIT, this project's own scripts
THIRD_PARTY_NOTICES.md  Proton's MIT payload license, trademarks, tooling credits
src/AppRun        AppImage entry point (pure argv pass-through)
src/io.github.zhafrisbs.ProtonDriveCLI.desktop     desktop entry
src/io.github.zhafrisbs.ProtonDriveCLI.appdata.xml AppStream metadata
src/licenses/Proton-Drive-SDK-MIT.txt  Proton's MIT notice (also inside every AppImage)
src/protondrive-icon.svg  Proton Drive glyph (simple-icons, CC0), tinted #6D4AFF
download/         upstream binaries (SHA-512-verified, git-ignored)
tools/            appimagetool + aarch64 runtime (auto-fetched, git-ignored)
build/            generated AppDir and validation logs (git-ignored)
dist/             final AppImages + SHA256SUMS (git-ignored, released via GitHub Releases)
```

Component id / file names share the id `io.github.zhafrisbs.ProtonDriveCLI`
because appimagetool requires the metainfo file to be named
`<desktop-basename>.appdata.xml` and appstreamcli requires that basename to
equal the component id. Change the string in `build.sh` (`DESKTOP_ID`,
`METAINFO_ID`), `src/*.desktop` and `src/*.appdata.xml` if you want your own id.

## License and legal

**Your safety rules, in short: source links + disclaimers + shipped notices.**

* **This repository's scripts** (`build.sh`, `install-command.sh`, `src/AppRun`,
  the desktop/AppStream metadata, the icon file) are **MIT** — see
  [`LICENSE`](LICENSE).
* **The payload** (the Proton Drive CLI binary inside each AppImage) is owned by
  Proton AG and is **MIT, `Copyright (c) 2025-2026 Proton AG`**. Its source is
  the SDK Proton publishes at <https://github.com/ProtonDriveApps/sdk>, which
  Proton states the CLI is built on. MIT's condition — keep the copyright and
  permission notice with all copies — is satisfied by
  [`src/licenses/Proton-Drive-SDK-MIT.txt`](src/licenses/Proton-Drive-SDK-MIT.txt),
  which `build.sh` installs into every AppImage at
  `usr/share/licenses/proton-drive/LICENSE`. Full inventory:
  [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).
* **Binary integrity**: payloads are byte-identical to Proton's downloads
  (`build.sh` aborts on SHA-512 mismatch) — nothing is patched, repacked or
  re-signed. x64 `3533025b…`, arm64 `c8d5a6b1…`.
* **No binaries in git**: `.gitignore` excludes `download/`, `dist/`, `tools/`
  and `build/`, so this repository contains no Proton code — only a script that
  fetches it from `proton.me` at build time.
* **Trademarks**: "Proton", "Proton Drive" and the Proton logo are trademarks of
  Proton AG, and their Terms of Service grant no trademark license. The name is
  used only to identify the product being packaged (nominative use), the glyph
  comes from simple-icons (CC0 file), and **this project is unofficial: not
  affiliated with, endorsed, sponsored or supported by Proton AG**. This
  disclaimer also appears in the AppStream metadata, the desktop entry comment
  and the release notes.
* Not legal advice — I am not a lawyer. If you want zero exposure, run
  `./build.sh` yourself instead of downloading anyone's AppImage; the script
  takes binaries straight from Proton.

## The GUI question

**Short answer for daily use: the folder sync** (see
[Folder sync](#folder-sync-google-drive-style) above) gives you a `~/ProtonDrive`
folder in Dolphin/Nautilus — drag & drop, double-click to open, just like a
local drive — on top of the official CLI only.

Proton ships **no Linux desktop app** — only this CLI. For a Linux GUI, **no
reverse engineering is needed**: Proton publishes the sources of its clients at
<https://github.com/ProtonDriveApps> — `windows-drive` (C#, GPLv3),
`mac-drive` (Swift, GPLv3), `android-drive` (Kotlin, GPLv3), plus the MIT
`sdk` (TypeScript) this CLI is built on. There is no Linux client repo, so a
Linux GUI means porting `windows-drive`/`sdk` yourself. REA
(`npx rea-agents setup`) is only useful for studying runtime behaviour you
cannot read in source.
