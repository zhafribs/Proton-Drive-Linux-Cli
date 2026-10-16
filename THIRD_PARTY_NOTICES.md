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

## 3. Build tooling (build time only, never redistributed here)

* `appimagetool` — <https://github.com/AppImage/appimagetool> — downloaded by
  `build.sh` into `tools/` (git-ignored).
* AppImage runtime / AppImageKit — <https://github.com/AppImage/AppImageKit>
  and <https://github.com/AppImage/type2-runtime> — embedded as the AppImage
  stub at build time.
* See those projects' own repositories for their licenses.

## 4. What is *not* in this repository

* No Proton binary is committed (see `.gitignore`: `download/`, `dist/`,
  `tools/`, `build/` are excluded).
* No Proton source code is copied in.
* No credentials, tokens or account data are handled by the build.
