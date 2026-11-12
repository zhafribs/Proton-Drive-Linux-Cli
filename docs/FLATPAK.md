# Flatpak

The release includes a single-file Flatpak bundle for x86_64:

```bash
flatpak install --user ./proton-drive_0.9.0+gui3.31.0_x86_64.flatpak
flatpak run io.github.zhafribs.protondrivecli
```

The runtime is `org.freedesktop.Platform//25.08`; Flatpak offers to install it
from Flathub if it is missing.

## Permissions

- `--filesystem=home` — the engine syncs a real folder and needs inotify,
  rename, and trash moves, not a portal-chosen single file.
- `--share=network` — the official CLI talks to Proton.
- `--talk-name=org.freedesktop.secrets` — the CLI session lives in the desktop
  keyring.
- Wayland/X11, DRM, notifications and the StatusNotifier tray.

## Sign-in

The sandbox has no host terminal, so **Sign in** shows the command to run:

```bash
flatpak run --command=proton-drive io.github.zhafribs.protondrivecli auth login
```

Finish the browser flow in that terminal. The app polls the session and
continues onboarding when it appears.

## Updates and startup

- Flatpak owns updates. The app can still tell you a newer release exists; it
  does not download or swap files inside the sandbox.
- Sync starts when the window opens. Host autostart/menu toggles are hidden
  because they cannot write outside the sandbox.
- `--diagnose` works inside the sandbox and reports the bundled CLI path,
  session probe, network and log tail.

## What is not packaged

- **aarch64 Flatpak.** The manifest and bundle are x86_64 only for now; the
  aarch64 AppImage and `.deb` remain available.
- **Flathub submission.** The bundle is self-hosted. A Flathub listing would
  need a source build and review.
