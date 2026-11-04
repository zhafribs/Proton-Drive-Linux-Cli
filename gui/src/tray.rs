//! System tray (freedesktop StatusNotifierItem via ksni) - the port of the
//! PyQt app's QSystemTrayIcon: the app icon with a status dot, Qt's exact
//! menu entries, left-click window toggle, and "Recent files" from the
//! change journal. Menu actions hand messages back to the UI thread over
//! the existing `Back` channel (ksni calls them on its own service thread).

use crate::journal::ChangeRow;
use crate::ui::Back;
use crate::util::rel_time;
use std::sync::mpsc::Sender;

/// The dot's colour *is* the state. It comes from `theme::status_color` - the
/// same value the Home orb, the sidebar presence dot and the account menu
/// paint - so the panel cannot disagree with the window, with two exceptions
/// where the in-app hue would be illegible or ambiguous on the panel:
///
/// - the working states are `ACCENT` purple, the tray artwork's own colour, so
///   they take the palette's cyan (which the app already uses for transfers);
/// - `signed_out` shares blue with `starting` in the app, but "welcome, sign in"
///   and "engine booting" are opposites, so it gets the palette's magenta.
///
/// Ten states collapse to seven readable dots (busy, ok, booting, welcome,
/// attention, stopped, broken); the tooltip still spells out the exact state.
pub fn dot_color(status_key: &str) -> [u8; 3] {
    let c = match status_key {
        "syncing" | "signing_in" => crate::theme::CYAN,
        "signed_out" => crate::theme::MAGENTA,
        other => crate::theme::status_color(other),
    };
    let [r, g, b, _] = c.to_array();
    [r, g, b]
}

fn smooth(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Tight alpha bounds `(x0, y0, x1, y1)` (exclusive max), ignoring the faint
/// antialiased fringe. The tray icon crops to this so padding does not make
/// the artwork look smaller than neighbouring full-bleed icons.
fn alpha_bounds(rgba: &[u8], w: usize, h: usize) -> Option<(usize, usize, usize, usize)> {
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return None;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if rgba[(y * w + x) * 4 + 3] > 8 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    (x1 > x0 && y1 > y0).then_some((x0, y0, x1, y1))
}

/// A 72x72 canvas: the cloud, cropped to its alpha bounds and scaled into the
/// top 72x66, plus a status dot in the strip below it.
///
/// The crop exists because the cloud PNG has ~19% transparent padding, which
/// made the tray icon look much smaller than a full-bleed icon like ZapFast.
///
/// Qt drew the dot as `drawEllipse(52, 52, 18, 18)` with a 4px white pen
/// (colour fill to r=7, ring 7..11). That pen is a trap: a panel renders this
/// at 16-22px, so a thick light ring ends up half the visible dot and dilutes
/// the hue - amber read as pink, cyan as light green. The badge is therefore
/// mostly solid colour (fill to r=16.5 of 18) with a thin dark edge, which
/// keeps the state readable at panel size and still defines the edge on light
/// panels while disappearing on dark ones.
///
/// Returns ARGB32 bytes, which is what `ksni::Icon` wants. `status_key` is
/// `health::UiStatus::key()`; the dot colour comes from [`dot_color`], so the
/// state is read from hue alone - nothing animates.
pub fn compose_icon(base_rgba: &[u8], base_w: usize, status_key: &str) -> Vec<u8> {
    const C: usize = 72;
    const ICON_W: usize = 72;
    const ICON_H: usize = 66;
    let mut canvas: Vec<[f32; 4]> = vec![[0.0; 4]; C * C]; // premultiplied RGBA

    let base_h = if base_w > 0 && base_rgba.len() >= base_w * base_w * 4 {
        base_rgba.len() / (base_w * 4)
    } else {
        0
    };
    if let Some((x0, y0, x1, y1)) = alpha_bounds(base_rgba, base_w, base_h) {
        let sample = |x: usize, y: usize| -> [f32; 4] {
            let i = (y * base_w + x) * 4;
            let a = base_rgba[i + 3] as f32 / 255.0;
            [
                base_rgba[i] as f32 / 255.0 * a,
                base_rgba[i + 1] as f32 / 255.0 * a,
                base_rgba[i + 2] as f32 / 255.0 * a,
                a,
            ]
        };
        let (cw, ch) = ((x1 - x0) as f32, (y1 - y0) as f32);
        for y in 0..ICON_H {
            for x in 0..ICON_W {
                let fx = x0 as f32 + (x as f32 + 0.5) * cw / ICON_W as f32 - 0.5;
                let fy = y0 as f32 + (y as f32 + 0.5) * ch / ICON_H as f32 - 0.5;
                let (x0f, y0f) = (fx.floor(), fy.floor());
                let (tx, ty) = (fx - x0f, fy - y0f);
                let xa = (x0f as isize).clamp(x0 as isize, x1 as isize - 1) as usize;
                let ya = (y0f as isize).clamp(y0 as isize, y1 as isize - 1) as usize;
                let xb = (xa + 1).min(x1 - 1);
                let yb = (ya + 1).min(y1 - 1);
                let a00 = sample(xa, ya);
                let a10 = sample(xb, ya);
                let a01 = sample(xa, yb);
                let a11 = sample(xb, yb);
                let mut px = [0.0f32; 4];
                for ch in 0..4 {
                    let top = a00[ch] * (1.0 - tx) + a10[ch] * tx;
                    let bot = a01[ch] * (1.0 - tx) + a11[ch] * tx;
                    px[ch] = top * (1.0 - ty) + bot * ty;
                }
                canvas[y * C + x] = px;
            }
        }
    }

    let mut over = |idx: usize, rgb: [u8; 3], a: f32| {
        if a <= 0.0 {
            return;
        }
        let dst = &mut canvas[idx];
        let ia = 1.0 - a;
        dst[0] = rgb[0] as f32 / 255.0 * a + dst[0] * ia;
        dst[1] = rgb[1] as f32 / 255.0 * a + dst[1] * ia;
        dst[2] = rgb[2] as f32 / 255.0 * a + dst[2] * ia;
        dst[3] = a + dst[3] * ia;
    };

    // Status dot: the state's colour, bottom-right, mostly solid fill.
    const BX: f32 = 54.0;
    const BY: f32 = 54.0;
    const BR: f32 = 18.0;
    const FILL: f32 = 16.5;
    let dot = dot_color(status_key);
    for y in 0..C {
        for x in 0..C {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let d = ((px - BX).powi(2) + (py - BY).powi(2)).sqrt();
            let idx = y * C + x;
            let edge = 1.0 - smooth(BR - 0.5, BR + 0.5, d);
            if edge > 0.0 {
                over(idx, [0x0a, 0x0b, 0x1a], edge);
            }
            let fill = 1.0 - smooth(FILL - 0.5, FILL + 0.5, d);
            if fill > 0.0 {
                over(idx, dot, fill);
            }
        }
    }

    // Unpremultiply into ARGB32.
    let mut out = vec![0u8; C * C * 4];
    for (i, px) in canvas.iter().enumerate() {
        let a = px[3];
        let (r, g, b) = (px[0] / a.max(1e-6), px[1] / a.max(1e-6), px[2] / a.max(1e-6));
        let o = i * 4;
        out[o] = (a * 255.0).round().clamp(0.0, 255.0) as u8;
        out[o + 1] = (r * 255.0).round().clamp(0.0, 255.0) as u8;
        out[o + 2] = (g * 255.0).round().clamp(0.0, 255.0) as u8;
        out[o + 3] = (b * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    out
}

/// "Recent files" entries: last 5 changes, newest first, Qt's label shape
/// `"{name[:44]}  \u{b7}  {when}"`, plus the parent folder to open on click.
pub fn recent_labels(changes: &[ChangeRow]) -> Vec<(String, String)> {
    changes
        .iter()
        .rev()
        .take(5)
        .map(|c| {
            let full = c.rel.trim_end_matches('/');
            let base = full.rsplit('/').next().unwrap_or(full);
            let parent = full[..full.len() - base.len()].trim_end_matches('/');
            let name: String = base.chars().take(44).collect();
            (format!("{name}  \u{b7}  {}", rel_time(c.ts)), parent.to_string())
        })
        .collect()
}

pub struct AppTray {
    status_key: String,
    title: String,
    active: bool,
    recent: Vec<(String, String)>,
    base_rgba: Vec<u8>,
    base_w: usize,
    tx: Sender<Back>,
}

impl AppTray {
    /// State push from the UI thread (only via `Handle::update`, which also
    /// tells the tray host to re-read icon/tooltip/menu properties).
    pub fn apply(
        &mut self,
        status_key: String,
        title: String,
        active: bool,
        recent: Vec<(String, String)>,
    ) {
        self.status_key = status_key;
        self.title = title;
        self.active = active;
        self.recent = recent;
    }
}

fn item(label: &str, tx: &Sender<Back>, msg: Back) -> ksni::menu::StandardItem<AppTray> {
    let tx = tx.clone();
    ksni::menu::StandardItem {
        label: label.to_string(),
        activate: Box::new(move |_| {
            let _ = tx.send(msg.clone());
        }),
        ..Default::default()
    }
}

impl ksni::Tray for AppTray {
    fn id(&self) -> String {
        "proton-drive-gui".into()
    }

    fn title(&self) -> String {
        "Proton Drive".into()
    }

    fn status(&self) -> ksni::Status {
        ksni::Status::Active
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        // Qt: tray.setToolTip(f"Proton Drive - {title}").
        ksni::ToolTip {
            title: format!("Proton Drive - {}", self.title),
            ..Default::default()
        }
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![ksni::Icon {
            width: 72,
            height: 72,
            data: compose_icon(&self.base_rgba, self.base_w, &self.status_key),
        }]
    }

    /// Left click toggles the window - the Qt Trigger handler.
    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.tx.send(Back::TrayToggle);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::{StandardItem, SubMenu};
        let tx = &self.tx;
        let mut m: Vec<ksni::MenuItem<Self>> = Vec::new();

        m.push(item("Open Proton Drive", tx, Back::TrayToggle).into());
        m.push(ksni::MenuItem::Separator);

        let mut sync = item("Sync now", tx, Back::TraySyncNow);
        sync.enabled = self.active;
        m.push(sync.into());
        m.push(item("Open local folder", tx, Back::TrayOpenFolder).into());
        m.push(ksni::MenuItem::Separator);

        // Qt: inactive -> "Resume sync" acts immediately; active -> the
        // three pause choices (identical strings).
        if self.active {
            m.push(
                SubMenu {
                    label: "Pause sync".into(),
                    submenu: vec![
                        item("Pause for 15 minutes", tx, Back::TrayPause(Some(900))).into(),
                        item("Pause for 1 hour", tx, Back::TrayPause(Some(3600))).into(),
                        item("Pause until I resume", tx, Back::TrayPause(None)).into(),
                    ],
                    ..Default::default()
                }
                .into(),
            );
        } else {
            m.push(item("Resume sync", tx, Back::TrayResume).into());
        }
        m.push(ksni::MenuItem::Separator);

        let recent: Vec<ksni::MenuItem<Self>> = if self.recent.is_empty() {
            vec![StandardItem {
                label: "Nothing yet".into(),
                enabled: false,
                ..Default::default()
            }
            .into()]
        } else {
            self.recent
                .iter()
                .map(|(label, parent)| item(label, tx, Back::TrayOpenPath(parent.clone())).into())
                .collect()
        };
        m.push(
            SubMenu {
                label: "Recent files".into(),
                submenu: recent,
                ..Default::default()
            }
            .into(),
        );
        m.push(ksni::MenuItem::Separator);
        m.push(item("Quit", tx, Back::TrayQuit).into());
        m
    }
}

/// A live tray handle. Every change arrives through `Handle::update`, which
/// is also what tells the host to re-read the icon - nothing ticks any more.
pub struct Tray {
    handle: ksni::blocking::Handle<AppTray>,
}

impl Tray {
    pub fn update<R>(&self, f: impl FnOnce(&mut AppTray) -> R) -> Option<R> {
        self.handle.update(f)
    }
}

/// Build and start the tray. Returns `None` when the desktop has no
/// StatusNotifierWatcher (Qt's `isSystemTrayAvailable() == False` - the app
/// keeps working, just without a tray).
pub fn spawn(
    tx: Sender<Back>,
    base_rgba: Vec<u8>,
    base_w: usize,
) -> Option<Tray> {
    use ksni::blocking::TrayMethods;
    let tray = AppTray {
        status_key: "unknown".into(),
        title: "Checking your files...".into(),
        active: false,
        recent: Vec::new(),
        base_rgba,
        base_w,
        tx,
    };
    let handle = match tray.spawn() {
        Ok(handle) => handle,
        Err(e) => {
            eprintln!("tray unavailable: {e}");
            return None;
        }
    };

    Some(Tray { handle })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every state `health::UiStatus::key()` can produce, all ten of them.
    const STATES: [&str; 10] = [
        "synced",
        "syncing",
        "paused",
        "needs_login",
        "signed_out",
        "signing_in",
        "engine_stopped",
        "starting",
        "error",
        "unknown",
    ];

    /// The tray dot follows the window's palette, except where that hue would
    /// be invisible or ambiguous on the icon: the working states take the cyan
    /// and the welcome screen the magenta.
    #[test]
    fn dot_colour_is_the_app_palette_for_every_state() {
        for key in STATES {
            let expected = match key {
                "syncing" | "signing_in" => crate::theme::CYAN,
                "signed_out" => crate::theme::MAGENTA,
                other => crate::theme::status_color(other),
            };
            let [r, g, b, a] = expected.to_array();
            assert_eq!(a, 0xff, "{key} palette colour must be opaque");
            assert_eq!(dot_color(key), [r, g, b], "{key}");
        }
        // No dot may be the cloud's own purple, or it would vanish on it.
        let [ar, ag, ab, _] = crate::theme::ACCENT.to_array();
        for key in STATES {
            assert_ne!(dot_color(key), [ar, ag, ab], "{key} must not be the cloud colour");
        }
        // The opposites must never look alike: welcome vs booting, or vs paused.
        assert_ne!(dot_color("signed_out"), dot_color("starting"));
        assert_ne!(dot_color("signed_out"), dot_color("paused"));
    }

    #[test]
    fn every_state_paints_its_dot() {
        for key in STATES {
            let argb = compose_icon(&[], 0, key);
            let o = (54 * 72 + 54) * 4;
            let [r, g, b] = dot_color(key);
            assert_eq!(&argb[o..o + 4], &[0xff, r, g, b], "{key} dot centre");
        }
    }

    /// The badge must stay the state's colour when a panel shrinks it: a thick
    /// white ring used to wash the hue out (amber read as pink, cyan as mint).
    #[test]
    fn the_dot_is_mostly_solid_colour() {
        let argb = compose_icon(&[], 0, "error");
        // Pure state colour almost to the edge.
        let o = ((54 - 14) * 72 + 54) * 4;
        assert_eq!(&argb[o..o + 4], &[0xff, 0xdc, 0x26, 0x26]);
        // A dark edge, not a white one, at the outer boundary.
        let o = ((54 - 18) * 72 + 54) * 4;
        assert!(argb[o] > 0xf0, "edge should be opaque: {}", argb[o]);
        assert!(
            argb[o + 1] < 0x30 && argb[o + 2] < 0x30 && argb[o + 3] < 0x30,
            "edge should be dark: {:?}",
            &argb[o..o + 4]
        );
        // Outside the badge the canvas is untouched.
        let o = ((54 - 20) * 72 + 54) * 4;
        assert_eq!(&argb[o..o + 4], &[0, 0, 0, 0]);
    }

    /// No animation: the same key always composes byte-identical pixels.
    #[test]
    fn the_icon_is_static() {
        let base = vec![255u8; 64 * 64 * 4];
        assert_eq!(
            compose_icon(&base, 64, "syncing"),
            compose_icon(&base, 64, "syncing")
        );
    }

    #[test]
    fn base_icon_fills_the_slot_and_crops_padding() {
        // A solid 64px base fills 72x66, leaving only the strip below for the
        // dot.
        let base = vec![255u8; 64 * 64 * 4];
        let argb = compose_icon(&base, 64, "synced");
        for (x, y) in [(0, 0), (71, 0), (0, 65), (71, 65)] {
            let o = (y * 72 + x) * 4;
            assert_eq!(&argb[o..o + 4], &[0xff, 0xff, 0xff, 0xff], "({x},{y})");
        }
        // Below the icon and clear of the dot.
        let o = (71 * 72) * 4;
        assert_eq!(&argb[o..o + 4], &[0, 0, 0, 0]);
    }

    #[test]
    fn transparent_padding_is_removed() {
        // A 16px square at (24,24) inside a 64px canvas must be scaled to the
        // full 72x66 slot: the corner is opaque now, not transparent padding.
        let mut base = vec![0u8; 64 * 64 * 4];
        for y in 24..40 {
            for x in 24..40 {
                let i = (y * 64 + x) * 4;
                base[i..i + 4].copy_from_slice(&[0xff, 0xff, 0xff, 0xff]);
            }
        }
        let argb = compose_icon(&base, 64, "synced");
        for (x, y) in [(0, 0), (71, 0), (0, 65), (71, 65)] {
            let o = (y * 72 + x) * 4;
            assert_eq!(&argb[o..o + 4], &[0xff, 0xff, 0xff, 0xff], "({x},{y})");
        }
    }

    #[test]
    fn recent_labels_qt_shape() {
        let rows = vec![
            ChangeRow {
                kind: crate::journal::ChangeKind::Uploaded,
                name: "a.md".into(),
                rel: "docs/a.md".into(),
                ts: None,
            },
            ChangeRow {
                kind: crate::journal::ChangeKind::Removed,
                name: "b".into(),
                rel: "b".into(),
                ts: None,
            },
        ];
        let recent = recent_labels(&rows);
        // Newest first (last row first, Qt's `[-5:][::-1]`); parent of
        // "docs/a.md" is "docs", a root-level item opens the root ("").
        assert_eq!(recent[0].0.split("  \u{b7}").next(), Some("b"));
        assert_eq!(recent[0].1, "");
        assert_eq!(recent[1].1, "docs");
        assert!(recent[1].0.starts_with("a.md  \u{b7}"));
    }
}
