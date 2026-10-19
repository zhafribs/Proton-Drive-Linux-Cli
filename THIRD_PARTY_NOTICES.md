# Third-party notices

This repository packages third-party software it does not own. This file lists
what is included, who owns it, and under what terms it is used.

## 1. Proton Drive CLI binary (the payload inside the AppImage)

* Owner: **Proton AG**, Route de la Galaise 32, 1228 Plan-les-Ouates, Geneva,
  Switzerland.
* Copyright: `Copyright (c) 2025-2026 Proton AG`
* License: **MIT** (verbatim text in
  [`src/licenses/Proton-Drive-SDK-MIT.txt`](src/licenses/Proton-Drive-SDK-MIT.txt),
  also installed inside every AppImage at
  `usr/share/licenses/proton-drive/LICENSE`).
* Upstream source: <https://github.com/ProtonDriveApps/sdk> (the TypeScript SDK
  the CLI is built on). Related official clients:
  <https://github.com/ProtonDriveApps> — `windows-drive` (C#, GPL-3.0-or-later),
  `mac-drive` (Swift, GPL-3.0-or-later), `android-drive` (Kotlin, GPL-3.0-or-later),
  `ios-drive`, `dotnet-crypto` (MIT), `dotnet-security`.
* Downloaded from: `https://proton.me/download/drive/cli/<version>/<platform>/proton-drive`
* Integrity: `build.sh` refuses to package a binary whose SHA-512 differs from
  the checksum Proton publishes at
  <https://proton.me/download/drive/cli/index.html>.
* Modification: **none**. The binary is redistributed verbatim inside the
  AppImage squashfs, exactly as Proton ships it.

MIT conditions are satisfied by shipping the text above inside every AppImage
and in this repository.

## 2. Proton trademarks

"Proton", "Proton Drive", "Proton Mail" and the Proton logos are trademarks or
registered trademarks of Proton AG. Proton's Terms of Service grant no license
to use them. This project uses the name **Proton Drive** only to identify the
upstream product it packages (nominative use).

**This project is unofficial. It is not created by, affiliated with, endorsed,
sponsored or otherwise supported by Proton AG.** Proton AG does not support,
verify or assume responsibility for this packaging, any resulting AppImage, or
any download from this repository.

The app icon is the Proton Drive glyph from
[simple-icons](https://simpleicons.org/) (CC0-1.0 file license), tinted by this
project. Trademark rights in the glyph remain with Proton AG; it is used here
solely to identify the packaged product. If Proton AG asks you to remove it,
remove `src/protondrive-icon.svg` and rebuild.

## 3. Bundled desktop stack (inside every release AppImage)

Release AppImages are self-contained: instead of borrowing Python and Qt from
the host, `build.sh` assembles this stack into `usr/lib/bundled-python`.
Everything below is redistributed unmodified (or, for CPython, the published
"install_only" archive unmodified apart from trimming modules the app never
loads); the licence texts ship inside every AppImage under
`usr/share/licenses/` and inside each component's own directory.

* **CPython 3.12** (portable build from
  [astral-sh/python-build-standalone](https://github.com/astral-sh/python-build-standalone)) —
  *Owner: Python Software Foundation*. Licence: **PSF-2.0** (text in
  [`src/licenses/PSF-2.0.txt`](src/licenses/PSF-2.0.txt), installed at
  `usr/share/licenses/PSF-2.0.txt`). Version and SHA-256 are pinned in
  [`scripts/fetch-desktop-deps.sh`](scripts/fetch-desktop-deps.sh) against
  upstream's `SHA256SUMS`; the download is verified on every fetch.
* **Qt 6.12 libraries and plugins** (from the `PyQt6-Qt6` wheel) — *Owner:
  The Qt Company Ltd / originally Digia*. Licence: **LGPL-3.0-only** with the
  Qt Company FOSS exception (text in
  [`src/licenses/LGPL-3.0-only.txt`](src/licenses/LGPL-3.0-only.txt),
  installed at `usr/share/licenses/LGPL-3.0-only.txt`). Used as a dynamically
  linked library, unmodified, in keeping with the LGPL's terms; the wheel's
  own `PyQt6/Qt6/pyqt6_qt6-*.dist-info/LICENSE` travels alongside it.
* **PyQt6 6.11 bindings** (from the `PyQt6` wheel) — *Owner: Riverbank
  Computing Ltd*. Licence: **GPL-3.0-only** (the Riverbank SIP licence; text
  in [`src/licenses/GPL-3.0-only.txt`](src/licenses/GPL-3.0-only.txt),
  installed at `usr/share/licenses/GPL-3.0-only.txt`; the wheel's
  `PyQt6/pyqt6-*.dist-info/licenses/LICENSE` travels alongside it). This is
  what makes the AppImage as a whole a GPL-3.0-only distribution: this
  repository's own scripts remain MIT, but the packaged combination is
  distributed under GPL-3.0-only, and its complete corresponding source is
  this public repository (build recipe, AppRun, GUI and sync engine under
  `src/` and `scripts/`, plus the pinned third-party versions above).
* **PyQt6-sip 13.13** — *Owner: Riverbank Computing Ltd*. Licence:
  **GPL-3.0-only or commercial** (terms travel inside the wheel's
  `pyqt6_sip-*.dist-info/`); same distribution terms as PyQt6 above.
* **jeepney 0.9.0** (from the `jeepney` wheel) — *Owner: Thomas Kluyver and
  contributors*. Licence: **MIT** (terms travel inside the wheel's
  `jeepney-0.9.0.dist-info/`). Pure-Python D-Bus client; the sync engine's
  real-time listener uses it to read the official CLI's session from the OS
  Secret Service - which is what keeps that feature free of any host package.
* Downloaded by `scripts/fetch-desktop-deps.sh` for **both** architectures
  (x86_64 wheels require glibc ≥ 2.34, aarch64 wheels ≥ 2.39), never
  executed at build time, never modified: wheels are extracted verbatim into
  `usr/lib/bundled-python/lib/python3.12/site-packages/`. The binaries live
  only inside `download/` and the AppImage - never committed (see
  `.gitignore`).

## 4. Build tooling (build time only, never redistributed here)

* `appimagetool` — <https://github.com/AppImage/appimagetool> — downloaded by
  `build.sh` into `tools/` (git-ignored).
* AppImage runtime / AppImageKit — <https://github.com/AppImage/AppImageKit>
  and <https://github.com/AppImage/type2-runtime> — embedded as the AppImage
  stub at build time.
* See those projects' own repositories for their licenses.

## 5. What is *not* in this repository

* No Proton binary is committed (see `.gitignore`: `download/`, `dist/`,
  `tools/`, `build/` are excluded - the bundled Python and PyQt6 wheels land
  in `download/bundle/` too).
* No Proton source code is copied in.
* No third-party binary (CPython, Qt6, PyQt6) is committed either - only the
  licence texts and the checksum-pinned fetch script.
* No credentials, tokens or account data are handled by the build.
