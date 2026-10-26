//! What colour scheme the desktop says it is.
//!
//! egui can only follow a "system" preference when the backend tells it what
//! that theme is. On Linux nothing does: winit hard-codes
//! `EventLoop::system_theme()` to `None`, and its X11 window theme is `None`
//! as well - which is the backend this GUI forces whenever XWayland exists.
//! `ThemePreference::System` therefore always resolved to egui's own fallback
//! (dark), never to the desktop. We ask the desktop ourselves instead, the
//! way the toolkits do:
//!
//! 1. XDG portal `org.freedesktop.appearance color-scheme` - what GNOME, KDE
//!    and the wlroots portals publish; toolkit- and backend-neutral.
//! 2. `gsettings org.gnome.desktop.interface color-scheme` (then `gtk-theme`)
//!    - GNOME, and the Hyprland setups that mirror it.
//! 3. GTK's `settings.ini` (`gtk-application-prefer-dark-theme`, theme name).
//! 4. Light - a conservative default when none of the above answers.
//!
//! [`watch`] keeps the answer current: the portal's `SettingChanged` signal
//! reports a flip as it happens, and a slow re-read covers portals that do
//! not emit it.

use crate::prefs::Theme;
use eframe::egui;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedValue, Value};

const PORTAL_SERVICE: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SETTINGS_IFACE: &str = "org.freedesktop.portal.Settings";
const APPEARANCE_NS: &str = "org.freedesktop.appearance";
const COLOR_SCHEME_KEY: &str = "color-scheme";

/// How often the watcher re-reads the setting when no signal arrives.
const POLL: Duration = Duration::from_secs(30);

/// The concrete theme to paint with: `System` takes the detected value.
pub fn resolve(pref: &Theme, detected: &Theme) -> Theme {
    match pref {
        Theme::System => detected.clone(),
        other => other.clone(),
    }
}

/// Read the desktop's colour scheme now. Blocking (one D-Bus round trip, or
/// two short subprocesses when the portal is absent), so it runs at startup
/// and on a theme change, not on every frame.
pub fn detect() -> Theme {
    portal()
        .or_else(gsettings)
        .or_else(gtk_settings)
        .unwrap_or(Theme::Light)
}

/// Report the desktop's colour scheme whenever it changes. The app reads it
/// once at startup itself; this covers flips while running. Blocking threads
/// only; the returned receiver is drained by the UI thread, and the context
/// argument is used to wake it up.
pub fn watch(ctx: egui::Context) -> mpsc::Receiver<Theme> {
    let (tx, rx) = mpsc::channel();

    let signal_tx = tx.clone();
    let signal_ctx = ctx.clone();
    std::thread::spawn(move || {
        let Some(conn) = session() else { return };
        let Ok(proxy) = Proxy::new(
            &conn,
            PORTAL_SERVICE,
            PORTAL_PATH,
            SETTINGS_IFACE,
        ) else {
            return;
        };
        let Ok(mut signals) = proxy.receive_signal("SettingChanged") else { return };
        for message in &mut signals {
            if !is_color_scheme_change(&message) {
                continue;
            }
            if signal_tx.send(detect()).is_err() {
                break; // the window is gone
            }
            signal_ctx.request_repaint();
        }
    });

    // The portal signal is the fast path; desktops whose portal never emits
    // SettingChanged (and the non-portal fallbacks above) still get noticed.
    std::thread::spawn(move || loop {
        std::thread::sleep(POLL);
        if tx.send(detect()).is_err() {
            break;
        }
        ctx.request_repaint();
    });

    rx
}

/// `org.freedesktop.portal.Settings.Read` returns a variant that may itself
/// contain a variant (`gdbus` prints `<<uint32 1>>`), so unwrap to the number.
/// 1 = prefer dark, 2 = prefer light, 0 = no preference.
fn color_scheme(value: &Value<'_>) -> Option<Theme> {
    match value {
        Value::Value(inner) => color_scheme(inner),
        Value::U32(1) => Some(Theme::Dark),
        Value::U32(2) => Some(Theme::Light),
        _ => None,
    }
}

fn is_color_scheme_change(message: &zbus::Message) -> bool {
    message
        .body()
        .deserialize::<(String, String, Value)>()
        .map(|(namespace, key, _)| {
            namespace == APPEARANCE_NS && key == COLOR_SCHEME_KEY
        })
        .unwrap_or(false)
}

fn portal() -> Option<Theme> {
    let conn = session()?;
    let proxy = Proxy::new(&conn, PORTAL_SERVICE, PORTAL_PATH, SETTINGS_IFACE).ok()?;
    let value: OwnedValue = proxy
        .call("Read", &(APPEARANCE_NS, COLOR_SCHEME_KEY))
        .ok()?;
    color_scheme(&value)
}

/// A session-bus connection with a short method timeout. Detection runs on
/// the startup path, so a wedged portal must not hold the window hostage.
fn session() -> Option<Connection> {
    zbus::blocking::connection::Builder::session()
        .ok()?
        .method_timeout(Duration::from_millis(500))
        .build()
        .ok()
}

fn gsettings() -> Option<Theme> {
    let color_scheme = gsettings_get("color-scheme")?;
    parse_color_scheme(&color_scheme).or_else(|| {
        let theme = gsettings_get("gtk-theme")?;
        parse_gtk_theme(&theme)
    })
}

fn gsettings_get(key: &str) -> Option<String> {
    let out = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", key])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `gsettings get ... color-scheme` prints `'prefer-dark'`, `'prefer-light'`
/// or `'default'`. `default` is not a light preference - fall through to the
/// theme name.
fn parse_color_scheme(text: &str) -> Option<Theme> {
    match text.trim().trim_matches('\'') {
        "prefer-dark" => Some(Theme::Dark),
        "prefer-light" => Some(Theme::Light),
        _ => None,
    }
}

/// A GTK theme name: anything carrying a `dark` marker is dark, everything
/// else is that theme's light variant.
fn parse_gtk_theme(text: &str) -> Option<Theme> {
    let name = text.trim().trim_matches('\'');
    (!name.is_empty()).then(|| {
        if name.to_ascii_lowercase().contains("dark") {
            Theme::Dark
        } else {
            Theme::Light
        }
    })
}

fn gtk_settings() -> Option<Theme> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"))
        })?;
    ["gtk-4.0", "gtk-3.0"]
        .iter()
        .find_map(|v| parse_gtk_settings_file(&base.join(v).join("settings.ini")))
}

fn parse_gtk_settings_file(path: &Path) -> Option<Theme> {
    parse_gtk_settings(&std::fs::read_to_string(path).ok()?)
}

/// Both GTK 3 and 4 keep this in `[Settings]`. An explicit
/// `gtk-application-prefer-dark-theme=1` wins; otherwise the theme name
/// decides. `=0` is not a light request by itself.
fn parse_gtk_settings(text: &str) -> Option<Theme> {
    let mut theme_name = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("gtk-application-prefer-dark-theme=") {
            match value.trim() {
                "1" | "true" => return Some(Theme::Dark),
                "0" | "false" => {}
                _ => {}
            }
        }
        if let Some(value) = line.strip_prefix("gtk-theme-name=") {
            theme_name = Some(value.trim().to_owned());
        }
    }
    theme_name.as_deref().and_then(parse_gtk_theme)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portal_numbers() {
        assert_eq!(color_scheme(&Value::U32(1)), Some(Theme::Dark));
        assert_eq!(color_scheme(&Value::U32(2)), Some(Theme::Light));
        assert_eq!(color_scheme(&Value::U32(0)), None);
        assert_eq!(color_scheme(&Value::U32(3)), None);
        assert_eq!(color_scheme(&Value::Bool(true)), None);
    }

    #[test]
    fn portal_reply_unwraps_variants() {
        // What this desktop actually returns: `v` containing another `v`.
        let nested = Value::Value(Box::new(Value::Value(Box::new(Value::U32(1)))));
        assert_eq!(color_scheme(&nested), Some(Theme::Dark));
    }

    #[test]
    fn gsettings_color_scheme() {
        assert_eq!(parse_color_scheme("'prefer-dark'\n"), Some(Theme::Dark));
        assert_eq!(parse_color_scheme("'prefer-light'\n"), Some(Theme::Light));
        assert_eq!(parse_color_scheme("'default'\n"), None);
        assert_eq!(parse_color_scheme(""), None);
    }

    #[test]
    fn gsettings_gtk_theme() {
        assert_eq!(parse_gtk_theme("'adw-gtk3-dark'\n"), Some(Theme::Dark));
        assert_eq!(parse_gtk_theme("'Adwaita'\n"), Some(Theme::Light));
        assert_eq!(parse_gtk_theme("''\n"), None);
    }

    #[test]
    fn gtk_settings_ini() {
        assert_eq!(
            parse_gtk_settings("[Settings]\ngtk-application-prefer-dark-theme=1\n"),
            Some(Theme::Dark)
        );
        assert_eq!(
            parse_gtk_settings("[Settings]\ngtk-theme-name=adw-gtk3-dark\n"),
            Some(Theme::Dark)
        );
        assert_eq!(
            parse_gtk_settings("[Settings]\ngtk-theme-name=Adwaita\n"),
            Some(Theme::Light)
        );
        assert_eq!(parse_gtk_settings("[Settings]\n"), None);
    }

    #[test]
    fn gtk_prefer_dark_zero_still_uses_the_theme_name() {
        assert_eq!(
            parse_gtk_settings(
                "[Settings]\ngtk-application-prefer-dark-theme=0\ngtk-theme-name=adw-gtk3-dark\n"
            ),
            Some(Theme::Dark)
        );
    }

    #[test]
    fn system_resolves_to_the_detected_theme() {
        assert_eq!(resolve(&Theme::System, &Theme::Dark), Theme::Dark);
        assert_eq!(resolve(&Theme::System, &Theme::Light), Theme::Light);
        assert_eq!(resolve(&Theme::Light, &Theme::Dark), Theme::Light);
        assert_eq!(resolve(&Theme::Dark, &Theme::Light), Theme::Dark);
    }
}
