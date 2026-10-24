# Install from source (tar.gz)

This guide builds Proton Drive for Linux from the source archive on GitHub.
You do **not** need the prebuilt AppImage to start — you can compile this
project yourself.

There are two ways to do it:

* **A. Build the AppImage** — recommended. You get the same self-contained app
  as the release, including its bundled Python runtime.
* **B. Install natively** — no AppImage and no FUSE. This builds the Rust GUI
  and uses your system Python for the sync service.

Both paths compile this repository's code. The only prebuilt part of the app
payload is Proton's **official Drive CLI**: the build downloads it from
`proton.me` and verifies it against the SHA-512 checksums in
[`upstream.lock`](../upstream.lock). Proton's CLI is not built from this
repository. The AppImage path also downloads a pinned portable CPython and
`appimagetool`; those are build/runtime dependencies, not Proton code.

> The screenshots and app name refer to Proton Drive. This is an unofficial
> client; it is not affiliated with Proton AG.

## 1. What you need

* An **x86_64** Linux desktop (releases are x86_64-only).
* **glibc 2.28 or newer** for the portable release-style AppImage (Debian 10 /
  Ubuntu 20.04 / Fedora 32 era or newer). A plain local `cargo build` without
  `cargo-zigbuild` inherits your build machine's glibc instead.
* About **3 GB of free disk space** for the downloads and Rust build.
* An internet connection for GitHub, `proton.me`, crates.io, and the portable
  Python runtime.

The build itself needs no root. Installing the packages below does.

### Build packages

Choose your distribution:

**Debian / Ubuntu**

```bash
sudo apt update
sudo apt install -y build-essential wget curl python3 python3-pip \
  librsvg2-bin file desktop-file-utils appstream
```

To run the finished AppImage you also need FUSE:

```bash
sudo apt install -y libfuse2t64   # Ubuntu 24.04+
# or: sudo apt install -y libfuse2   # older Debian/Ubuntu
```

If FUSE is not available, launch with `APPIMAGE_EXTRACT_AND_RUN=1`.

**Fedora**

```bash
sudo dnf install -y gcc gcc-c++ make wget curl python3 python3-pip \
  librsvg2-tools file desktop-file-utils appstream fuse-libs
```

**Arch Linux**

```bash
sudo pacman -S --needed base-devel wget curl python python-pip \
  librsvg file desktop-file-utils appstream fuse2
```

`desktop-file-utils` and `appstream` are optional — they only add validation
during the build. `librsvg2-bin` / `librsvg2-tools` / `librsvg` provides
`rsvg-convert`, which the build uses for the icon.

### Rust

Install the current stable Rust with [rustup](https://rustup.rs/):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
  | sh -s -- -y --profile minimal
. "$HOME/.cargo/env"
rustc --version
```

The GUI uses the Rust 2024 edition, so an old distro `cargo` may be too old.
If `rustc --version` is below 1.85, run `rustup update stable`.

### Optional: portable AppImage build

`build.sh` uses `cargo-zigbuild` when it is available, which pins the GUI to
glibc 2.28. Without it, the build falls back to a normal `cargo build` and the
binary inherits your machine's glibc — fine for installing on this machine,
but less portable if you share the AppImage.

```bash
cargo install cargo-zigbuild --locked
# Also install Zig 0.15.x from https://ziglang.org/download/ and put it on PATH.
```

This step is optional. If you only want to run the app locally, skip it.

## 2. Get the source

Open the [latest release](https://github.com/zhafribs/Proton-Drive-Linux-Cli/releases/latest)
and download **Source code (tar.gz)** from the Assets section. Then extract it:

```bash
TAG=v0.9.0-gui3.16.16   # replace with the tag you downloaded
mkdir -p ~/src/proton-drive-src
cd ~/src
wget -O proton-drive-src.tar.gz \
  "https://github.com/zhafribs/Proton-Drive-Linux-Cli/archive/refs/tags/${TAG}.tar.gz"
tar -xzf proton-drive-src.tar.gz \
  -C proton-drive-src --strip-components=1
cd proton-drive-src
```

`--strip-components=1` removes GitHub's top-level archive directory, so the
repository files land directly in `~/src/proton-drive-src`. Check that the
build script is there:

```bash
ls build.sh scripts/proton-sync.py
```

> GitHub's generated source archive is not signed. The important integrity
> check happens during the build: the official Proton CLI is verified against
> Proton's published SHA-512 before it is packaged.

## 3. Path A — build the AppImage (recommended)

From the extracted source directory:

```bash
./build.sh
```

Run it as your normal user — it writes only inside the checkout. On the first
run it:

1. downloads `appimagetool` into `tools/`;
2. downloads Proton's official CLI and verifies its SHA-512 against
   `upstream.lock`;
3. builds the Rust GUI in `gui/`;
4. downloads a pinned portable CPython and the `jeepney` wheel;
5. bundles the sync engine and those runtime pieces;
6. writes the AppImage and `dist/SHA256SUMS`.

It can take 5–20 minutes depending on the machine and network. A normal run
ends with a path like:

```text
dist/ProtonDrive-CLI-0.9.0-GUI-3.16.16-x86_64.AppImage
```

The exact version comes from `upstream.lock` and `gui/src/main.rs`. Find the
newest artifact with:

```bash
ls -1t dist/*.AppImage | head -1
```

### Run it

```bash
chmod +x dist/*.AppImage
./dist/ProtonDrive-CLI-0.9.0-GUI-3.16.16-x86_64.AppImage
```

If your desktop lacks FUSE, use:

```bash
APPIMAGE_EXTRACT_AND_RUN=1 \
  ./dist/ProtonDrive-CLI-0.9.0-GUI-3.16.16-x86_64.AppImage
```

### Optional: put the CLI on your PATH

```bash
./install-command.sh
proton-drive --version
```

This creates `~/.local/bin/proton-drive` as a symlink to the built AppImage.
You can also pass a path explicitly:

```bash
./install-command.sh ~/Applications/ProtonDrive-CLI-0.9.0-GUI-3.16.16-x86_64.AppImage
```

The AppImage passes CLI arguments straight through, so `proton-drive filesystem
list /` and Proton's other CLI commands work. The app's **Sign in** button uses
the same CLI.

### Menu entry and updates

Keep the AppImage somewhere stable (for example `~/Applications`) and launch
it. In the app, **Settings → Add to my app menu** adds a desktop entry. The
normal in-app updater works for AppImages, but it downloads official release
builds; if you want to keep using your own source build, download the newer
source archive and rebuild instead.

## 4. Path B — native install (no AppImage)

This path is useful if you do not want an AppImage/FUSE, or you want to run the
GUI from `~/.local/bin`. The trade-off is that the sync service points at this
source checkout, so keep the checkout in place.

### 4.1 Build the GUI

```bash
cd gui
cargo build --release
cd ..
```

The binary is `gui/target/release/proton-drive-gui`. A first Rust build can
take several minutes and downloads the crate graph from crates.io.

### 4.2 Install the GUI

```bash
./scripts/install-gui.sh
```

This installs:

* `~/.local/bin/proton-drive-gui`
* a menu entry (`~/.local/share/applications/proton-drive-gui.desktop`)
* the icon under `~/.local/share/icons`

Make sure `~/.local/bin` is on your `PATH`. Most desktops already include it;
if not, add this to your shell startup file:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

Then launch `proton-drive-gui` from a terminal or your application menu.

### 4.3 Install the official Proton CLI

The GUI and the sync engine both invoke `proton-drive`. Download the official
Linux binary and verify it against the checksum that this repository pins:

```bash
BASE=$(sed -n 's/^DOWNLOAD_BASE=//p' upstream.lock)
VER=$(sed -n 's/^VERSION=//p' upstream.lock)
SHA=$(sed -n 's/^SHA512_LINUX_X64=//p' upstream.lock)
TMP=$(mktemp)
wget -O "$TMP" "$BASE/$VER/linux-x64/proton-drive"
echo "$SHA  $TMP" | sha512sum -c - \
  && mkdir -p "$HOME/.local/bin" \
  && install -m 0755 "$TMP" "$HOME/.local/bin/proton-drive" \
  && rm -f "$TMP" \
  && proton-drive --version
```

`sha512sum -c -` must print `OK`; the `&&` chain stops before installing on a
mismatch. Run `./scripts/check-upstream.sh` to refresh `upstream.lock` if
Proton has published a new version, then repeat the download.

### 4.4 Optional: enable real-time cloud push

The AppImage bundles `jeepney`, the pure-Python client the sync engine uses to
read the CLI session from your desktop keyring. A native install uses your
system Python, which may not have it.

Install it with your package manager when possible:

```bash
# Debian / Ubuntu
sudo apt install python3-jeepney

# Fedora
sudo dnf install python3-jeepney

# Arch
sudo pacman -S python-jeepney
```

If your distribution does not package it, skip this step — the sync engine
still works without it, just with interval polling instead of real-time push.

Without `jeepney` the sync still works, but real-time cloud push is disabled
and the engine falls back to interval polling. Local edits still upload
immediately through inotify.

### 4.5 First launch and sign-in

```bash
proton-drive-gui
```

Click **Sign in**, complete Proton's login/2FA in the terminal window that
opens, then return to the app and confirm it can see your account.

### 4.6 Install the sync service

Run this from the source checkout after signing in:

```bash
./scripts/install-sync.sh
```

This creates the systemd **user** service
`~/.config/systemd/user/proton-sync.service`, creates a default
`~/.config/proton-sync.conf` if needed, and starts/enables the service.

Important: the unit runs the engine from **this checkout**:

```text
ExecStart=/usr/bin/python3 /home/you/src/proton-drive-src/scripts/proton-sync.py --loop
```

Keep the checkout where it is. If you move it, rerun `install-sync.sh` from
the new location so the unit is rewritten. The sync folder
(`~/ProtonDrive`) and the state/snapshot live outside the checkout and are
safe.

Check it:

```bash
systemctl --user status proton-sync.service
journalctl --user -u proton-sync -f
```

The GUI can start, stop, pause and resume the service from **Settings** or
**Home**. A native build has no AppImage to replace itself, so **in-app
updates are not available** — see the updating section below.

## 5. First run (both paths)

1. Launch the app and click **Sign in**. Proton's official login opens in a
   terminal; that is how their CLI handles 2FA. The app picks up the session
   when the CLI finishes.
2. Open **Settings → Sync folder**, choose the folder to mirror, and save.
   The first save installs/starts the background sync service for an AppImage
   build. For a native build, `install-sync.sh` has already done it.
3. The Home screen shows the current state. **Sync now** forces a check; the
   interval can be set from *Instant* to *Off*. *Off* disables scheduled
   checks only — cloud push and local changes still sync, and **Sync now**
   still works.

The default local folder is `~/ProtonDrive`. Your Drive's `/my-files` section
is mirrored into it.

## 6. Updating a source install

### AppImage build

Download the newer source archive, extract it, and run `./build.sh` again.
Replace the old AppImage and keep your existing config. You can also use the
app's normal updater, but it installs the official release AppImage rather
than rebuilding your checkout.

### Native install

Download and extract the newer source archive, then rebuild and reinstall:

```bash
cd gui && cargo build --release && cd ..
./scripts/install-gui.sh
```

If `upstream.lock` changed, repeat the CLI download/verify step from section
4.3. If you extracted the new checkout to a different directory, rerun
`./scripts/install-sync.sh` from the new checkout so the systemd unit points
there.

Your settings (`~/.config/proton-sync.conf`), snapshot/state
(`~/.local/state/proton-sync/`), and sync folder are kept across rebuilds.

## 7. Uninstall

Stop and remove the sync service from the checkout:

```bash
./scripts/install-sync.sh uninstall
```

This keeps your sync folder and state. Then remove the app pieces:

**AppImage**

If you added a menu entry, remove it first with **Settings → Remove from app
menu**, then delete the file and the PATH symlink:

```bash
rm -f ~/.local/bin/proton-drive
rm -f ~/Applications/ProtonDrive-CLI-*-x86_64.AppImage   # or wherever it lives
```

**Native**

```bash
./scripts/install-gui.sh remove
rm -f ~/.local/bin/proton-drive
```

You can then remove the source checkout. If you also want to remove the sync
configuration and local snapshot:

```bash
rm -f ~/.config/proton-sync.conf
rm -rf ~/.local/state/proton-sync
```

Do **not** delete `~/ProtonDrive` unless you also want to remove the local
copy of your files. Deletions made by the app go to Proton's trash or your
desktop trash, never permanently.

## 8. Troubleshooting

**`wget: command not found`** — install `wget`. `build.sh` uses it to fetch
Proton's CLI and the portable runtime.

**`rsvg-convert: command not found`** — install `librsvg2-bin` (Debian/Ubuntu),
`librsvg2-tools` (Fedora), or `librsvg` (Arch).

**`cargo: command not found`** — rustup installed Rust but your shell has not
loaded it yet. Run `. "$HOME/.cargo/env"` or open a new terminal.

**Rust is too old / edition 2024 error** — run `rustup update stable`.

**`Permission denied` running `build.sh`** — if the archive lost its
executable bits, run `chmod +x build.sh scripts/*.sh`.

**`cargo zigbuild` fails because Zig is missing** — install Zig 0.15.x from
<https://ziglang.org/download/>, or remove `cargo-zigbuild` from `PATH` so
`build.sh` falls back to the normal `cargo build`.

**The build stops with a SHA-512 mismatch** — do not bypass it. Proton may
have published a new CLI; run `./scripts/check-upstream.sh` to refresh
`upstream.lock`, then build again. If the download is being altered by a proxy
or mirror, fix the network path.

**The AppImage will not start / FUSE error** — install FUSE (`libfuse2t64`,
`libfuse2`, `fuse-libs`, or `fuse2`), or run it with
`APPIMAGE_EXTRACT_AND_RUN=1`.

**`install-sync.sh` says the official CLI was not found** — check that
`~/.local/bin/proton-drive` exists and is executable, or point the installer
at your copy:

```bash
PROTON_DRIVE_BIN=/path/to/proton-drive ./scripts/install-sync.sh
```

**The service fails after I moved the source folder** — rerun
`./scripts/install-sync.sh` from the new checkout. The unit contains an
absolute path to `scripts/proton-sync.py`.

**Cloud changes are slow, but local edits sync instantly** — check whether
`jeepney` is available to the system Python (section 4.4). The log says
`jeepney not bundled - interval polling only` when push is unavailable. You
can also lower the interval in Settings.

**`proton-drive-gui: command not found`** — add `~/.local/bin` to `PATH`
(section 4.2), or run `gui/target/release/proton-drive-gui` directly from the
checkout.

**Sign-in did not stick** — the official CLI stores its session in your
desktop keyring. Make sure the keyring is unlocked, then click **Sign in**
again. The engine detects a locked keyring and retries; it cannot unlock a
keyring that never prompts.

## 9. What the build is doing

For reference, `./build.sh`:

* reads `upstream.lock` for the Proton CLI version and SHA-512;
* downloads `proton-drive` and refuses to package a checksum mismatch;
* builds `gui/` with `cargo zigbuild` or plain `cargo`;
* renders the icon with `rsvg-convert`;
* downloads the pinned portable CPython and `jeepney`;
* copies `scripts/proton-sync.py`, the GUI, the CLI, the icon and licences
  into an AppDir;
* runs `appimagetool` and writes `dist/*.AppImage` plus `dist/SHA256SUMS`.

The native path skips the AppDir packaging: it installs the Rust GUI and the
official CLI into `~/.local/bin`, and runs the Python sync engine directly
from the checkout through a systemd user service.

## Need the prebuilt version instead?

Download the AppImage from the
[latest release](https://github.com/zhafribs/Proton-Drive-Linux-Cli/releases/latest).
The main [README](../README.md) covers the normal install, first-run flow, and
the app's features.
