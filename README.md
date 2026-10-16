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
./build.sh              # both architectures
./build.sh x86_64       # one architecture
VERSION=0.9.1 ./build.sh  # a newer upstream release (update the SHA-512 table first)
```

`build.sh` downloads `appimagetool` and the aarch64 runtime into `tools/` on
first run, fetches the upstream binaries, **aborts if the SHA-512 does not match
Proton's published checksum**, then builds and validates.

Dependencies (all non-root, nothing is installed system-wide):

* `wget`, `bash`, `coreutils` — likely already present
* `rsvg-convert` (`pacman -S librsvg` / `apt install librsvg2-bin` / `dnf install librsvg2-tools`) — renders the icon
* `desktop-file-validate` (`desktop-file-utils`) and `appstreamcli` (`appstream`) — optional, used for validation

No `sudo` and no `mksquashfs` needed: appimagetool ships its own squashfs builder.

## Project layout

```
build.sh          reproducible build script (download → verify → package → validate)
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

Proton ships **no Linux desktop app** — only this CLI. For a Linux GUI, **no
reverse engineering is needed**: Proton publishes the sources of its clients at
<https://github.com/ProtonDriveApps> — `windows-drive` (C#, GPLv3),
`mac-drive` (Swift, GPLv3), `android-drive` (Kotlin, GPLv3), plus the MIT
`sdk` (TypeScript) this CLI is built on. There is no Linux client repo, so a
Linux GUI means porting `windows-drive`/`sdk` yourself. REA
(`npx rea-agents setup`) is only useful for studying runtime behaviour you
cannot read in source.
