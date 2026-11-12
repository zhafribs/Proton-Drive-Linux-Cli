# Linux and desktop compatibility

This is the honest support matrix for the AppImage and .deb packages, not a
marketing claim. Releases cover x86_64 and aarch64 Linux.

## What works

| Area | Status |
|---|---|
| ABI | glibc 2.28+ on x86_64 and aarch64. Release builds are zig-pinned; the gate checks the ELF floor. |
| AppImage mount | Type-2 runtime with FUSE2 `fusermount`; recent runtimes also try `fusermount3`, and FUSE3-only systems can set `FUSERMOUNT_PROG=fusermount3`. |
| No FUSE | With `APPIMAGE_EXTRACT_AND_RUN=1` the GUI runs. Background sync still needs a mountable AppImage: the systemd unit starts the AppImage without that variable, so treat extract-and-run as GUI-only. |
| Display | X11; Wayland through XWayland; native Wayland with `PROTON_SYNC_WAYLAND=1`. |
| Desktop | GNOME, KDE Plasma, XFCE, Cinnamon, MATE, Hyprland and wlroots desktops through the XDG portal and GTK settings. |
| Theme | XDG portal, then desktop settings (`kdeglobals`, XFCE XSettings, gsettings/dconf for GNOME/Cinnamon/MATE with the active schema first), then `GTK_THEME`, then GTK 3/4 settings. |
| File pickers | XDG portal first, then `zenity`, then `kdialog`. |
| Tray | StatusNotifierItem where the desktop provides a host; AppIndicator extension required on GNOME. |
| Autostart | XDG desktop entry. Background sync uses a systemd user unit. |
| Notifications | The desktop notification daemon over D-Bus (notify-rust). |

## Needs a package or setting

| Situation | Requirement |
|---|---|
| FUSE3-only system | `FUSERMOUNT_PROG=fusermount3` or extract-and-run. |
| GNOME tray icon | AppIndicator/KStatusNotifierItem extension. |
| No portal | Install `zenity` or `kdialog` for file pickers. |
| Native Wayland | `PROTON_SYNC_WAYLAND=1`; close-to-tray still minimizes on some compositors. |
| Sign-in terminal | `$TERMINAL` first, then konsole, kitty, alacritty, xfce4-terminal, gnome-terminal, ptyxis, kgx, foot, wezterm, terminator, urxvt, st, xterm. |
| OpenGL | Working EGL/OpenGL; llvmpipe is fine. |

## Not supported

- **musl-only distributions** (Alpine): the release is glibc-linked.
- **32-bit x86**: releases are 64-bit only.
- **Flatpak**: no sandboxed package yet.
- **Non-systemd init systems** for background sync. The GUI can browse, but
  start/pause/enable-at-login and the engine service require a systemd user
  session. This is the largest remaining portability gap.
- **Symlinks inside the sync folder** are not followed. A symlink, special
  file, unreadable directory or malformed cloud listing stops the cycle with
  an error instead of being mistaken for a deletion. Remove or exclude the
  link to continue.
- **Changing the local or remote sync root** archives the previous state
  history (`state.json`, snapshot, approvals) and starts a fresh scan. Signing
  out clears the account's state and cloud caches.
- **Quota and event stream**: the official CLI has no quota command, so the
  engine reads `UsedSpace`/`MaxSpace` and the event stream through the CLI's
  own keyring session. Both are read-only; no token is written or logged.
- A dark **wallpaper** does not imply a dark desktop theme. “Match my system”
  follows the desktop's color-scheme setting; the Appearance tab shows
  `System theme: Dark/Light` so this is visible.

## Release checks

- CI builds with `REQUIRE_GLIBC_FLOOR=1` and `cargo-zigbuild` at glibc 2.28.
- The release gate verifies both AppImages, both `.deb` packages, signatures
  and smoke startup on the build architecture.
- Local development builds without zig are not release artifacts; they inherit
  the host glibc and must not be published.
