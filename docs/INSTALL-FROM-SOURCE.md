# Install / build from source

This project is Rust-only. The desktop GUI, the sync engine and the AppImage
`AppRun` launcher are all Rust; the bundled Proton Drive CLI is Proton's
official Rust CLI. There is no Python, no Qt and no bundled interpreter.

## Prerequisites

* Rust (stable) via [rustup](https://rustup.rs/)
* `wget` (or `curl`), `rsvg-convert` (`librsvg2-bin` / `librsvg`) for the icon
* Optional: `desktop-file-validate`, `appstreamcli` (AppStream validation)
* For running the AppImage: FUSE and a normal OpenGL desktop

Debian/Ubuntu:

```bash
sudo apt install build-essential wget librsvg2-bin file \
  desktop-file-utils appstream
```

Fedora/Arch users need the equivalent packages (`gcc`, `make`, `wget`,
`librsvg2-tools`/`librsvg`, `desktop-file-utils`, `appstream`).

## Build the AppImage

```bash
./build.sh x86_64
```

The script downloads and SHA-512-verifies the official Proton CLI, builds the
Rust GUI, engine and `AppRun` launcher, stages the AppDir, and packages it with
`appimagetool` into `dist/`.

## Run from a source checkout (development)

The GUI and the sync engine are separate processes.

```bash
# terminal 1 - desktop app
cargo run -p proton-drive-gui

# terminal 2 - sync engine
cargo run -p proton-engine -- --loop
```

The GUI/engine need the official `proton-drive` CLI. Either put it on `PATH`:

```bash
ln -s "$PWD/download/proton-drive-0.9.0-linux-x64" ~/.local/bin/proton-drive
```

or point the engine at it explicitly:

```bash
PROTON_DRIVE_BIN="$PWD/download/proton-drive-0.9.0-linux-x64" \
  cargo run -p proton-engine -- --loop
```

When you run from `cargo`, the app is **not** an AppImage, so it does not
install a systemd service or an autostart entry. Start the engine yourself as
above; the Home screen reports the engine state honestly.

## Build only the binaries

```bash
cargo build --release \
  -p proton-drive-gui -p proton-engine -p proton-drive-apprun
```

The release binaries land in `gui/target/release/` (the workspace target
directory is pinned in `.cargo/config.toml`).

## Tests

```bash
cargo test
cargo clippy --release --all-targets
```

## Files and folders

* Config: `~/.config/proton-sync.conf`
* State/cache: `~/.local/state/proton-sync/`
* Synced folder: `~/ProtonDrive` (configurable with `PROTON_SYNC_LOCAL`)

These are kept when you uninstall the app; remove them by hand only if you
want a completely fresh start.

## Uninstall a running install

```bash
systemctl --user disable --now proton-sync
rm -f ~/.config/systemd/user/proton-sync.service
rm -f ~/.config/autostart/proton-drive.desktop
rm -f ~/.local/share/applications/proton-drive.desktop
rm -f ~/.local/share/icons/hicolor/scalable/apps/proton-drive-gui.svg
systemctl --user daemon-reload
```

Your `~/ProtonDrive` folder and `~/.local/state/proton-sync/` are not touched.
