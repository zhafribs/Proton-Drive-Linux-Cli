# "Match my system" actually follows the desktop — short design (2026-10-26)

## Intent

Settings → Appearance offers "Match my system", but picking it never changed
the theme: after choosing Light, selecting it left the app light - even on a
dark desktop. There are two independent reasons.

1. `theme::force()` mapped `Theme::System` to `{}`. The preference was stored
   but never applied, so the app kept whichever of Light/Dark was forced last
   (at startup: the default, light).
2. Even a correct `ThemePreference::System` cannot work on Linux. egui resolves
   `System` from `RawInput.system_theme`, and winit never supplies one:
   `EventLoop::system_theme()` is hard-coded to `None`
   (`winit-0.30.13/src/platform_impl/linux/mod.rs:909`) and the X11 window's
   `theme()` returns `None` too (`x11/window.rs:1905`). This GUI deliberately
   forces X11 whenever XWayland exists, so that is the backend in use. egui
   therefore falls back to its own default (dark), which ignores the desktop
   just as completely - it only looked right on a dark system by coincidence.

The desktop does expose the answer. On the affected machine
(Hyprland/Wayland) `org.freedesktop.appearance color-scheme` is `1` (dark),
matching `gsettings` (`color-scheme='prefer-dark'`,
`gtk-theme='adw-gtk3-dark'`).

## Change

**New `gui/src/system_theme.rs`**

- `detect() -> Theme` asks the desktop, in the same order the toolkits do:
  1. XDG portal `org.freedesktop.portal.Settings.Read`
     (`org.freedesktop.appearance`, `color-scheme`) through the `zbus` the GUI
     already uses for file dialogs;
  2. `gsettings get org.gnome.desktop.interface color-scheme`, then
     `gtk-theme`;
  3. GTK's `settings.ini` (`gtk-application-prefer-dark-theme`, theme name);
  4. light - a conservative default when none of the above answers.
- `watch(ctx) -> Receiver<Theme>` reports changes: the portal's
  `SettingChanged` signal gives an instant flip, and a 30 s re-read is the
  safety net for portals that do not emit it.
- `resolve(pref, detected)` maps `System` to the detected theme, so the
  decision is one testable function.

**`gui/src/theme.rs`**

- `force()` stops pretending to handle `System`; it receives the theme that
  was already resolved (Light/Dark). The doc comment explains why egui cannot
  do this itself.

**`gui/src/ui/mod.rs`**

- `App` keeps `system_theme` (the last detected value) plus the watcher's
  receiver, and paints with `resolved_theme()`.
- Startup detects once, before the first frame, so a dark desktop never
  flashes light.
- Selecting a theme re-detects synchronously when the preference is `System`,
  so the switch is immediate rather than waiting for the next poll.
- Watcher updates replace the field and repaint only while the preference is
  `System`.

**`gui/src/main.rs`**

- `--smoke` prints the detected system theme next to the preference, so the
  packaged AppImage can be verified with one command.

## Alternatives considered

- **Just set `ThemePreference::System`** — looks right, has no effect on Linux
  for the reason above.
- **Stop forcing X11** — unrelated (winit reports no theme on either backend)
  and undoes a deliberate XWayland choice.
- **Add the `dark-light` crate** — solves the same problem, but the GUI
  already has a hand-rolled blocking `zbus` portal client; a second portal
  library and its async runtime is more surface than the ~150 lines here.
- **Poll only** — a 30 s lag on an OS flip. The signal gives an instant switch;
  the poll is only the fallback.

## Consequences

- Light and Dark behave exactly as before.
- "Match my system" follows the desktop, including a flip while running.
- A desktop exposing none of the four sources gets light: a deliberate,
  visible default (egui's own fallback is dark).
- Cost: one D-Bus call at startup and on each OS change, plus a cheap re-read
  every 30 s in the watcher; no per-frame work.
