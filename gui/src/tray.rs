//! System tray (freedesktop StatusNotifierItem via ksni) - the port of the
//! PyQt app's QSystemTrayIcon: the app icon with a status dot, Qt's exact
//! menu entries, left-click window toggle, and "Recent files" from the
//! change journal. Menu actions hand messages back to the UI thread over
//! the existing `Back` channel (ksni calls them on its own service thread).

use crate::journal::ChangeRow;
use crate::ui::Back;
use crate::util::rel_time;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

/// What the tray icon should communicate.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IconMode {
    /// Synced: the plain app icon.
    Normal,
    /// Work in progress: fade the icon in and out.
    Pulse,
    /// Not syncing, paused, signed out or an error: app icon with an X.
    Error,
}

pub fn icon_mode(status_key: &str) -> IconMode {
    match status_key {
        "synced" => IconMode::Normal,
        // Starting, checking and signing in are still working states.
        "syncing" | "starting" | "signing_in" | "unknown" => IconMode::Pulse,
        _ => IconMode::Error,
    }
}

pub fn pulses(status_key: &str) -> bool {
    icon_mode(status_key) == IconMode::Pulse
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

/// Qt drew a 72x72 canvas: the 64px icon scaled to 66px at (2,2), then a
/// status dot `drawEllipse(52, 52, 18, 18)` with a 4px white pen (so the
/// color fill shows to r=7 and the white ring covers 7..11, center (61,61)).
///
/// The dot is deliberately larger than Qt's: at a 22px tray size the old
/// 14px fill became ~4px, making green (synced) and purple (syncing) hard to
/// tell apart.
///
/// The cloud PNG has ~19% transparent padding, which made the tray icon look
/// much smaller than a full-bleed icon like ZapFast. Crop to the alpha bounds
/// and scale that content to 72x66, so the cloud fills the slot; the badge
/// stays bottom-right in the remaining strip.
/// Returns ARGB32 bytes, which is what `ksni::Icon` wants.
pub fn compose_icon(base_rgba: &[u8], base_w: usize, status_key: &str, pulse: f32) -> Vec<u8> {
    const C: usize = 72;
    const ICON_W: usize = 72;
    const ICON_H: usize = 66;
    let mode = icon_mode(status_key);
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

    // Error badge: a red disc with a white X over the lower-right corner.
    // There is no status dot any more; this is the only badge.
    if mode == IconMode::Error {
        const BX: f32 = 54.0;
        const BY: f32 = 54.0;
        const BR: f32 = 17.0;
        const ARM: f32 = 7.5;
        for y in 0..C {
            for x in 0..C {
                let px = x as f32 + 0.5;
                let py = y as f32 + 0.5;
                let d = ((px - BX).powi(2) + (py - BY).powi(2)).sqrt();
                let idx = y * C + x;
                let disc = 1.0 - smooth(BR - 0.5, BR + 0.5, d);
                if disc > 0.0 {
                    over(idx, [0xdc, 0x26, 0x26], disc);
                }
                if d <= BR - 2.0 {
                    let a = segment_distance(px, py, BX - ARM, BY - ARM, BX + ARM, BY + ARM);
                    let b = segment_distance(px, py, BX + ARM, BY - ARM, BX - ARM, BY + ARM);
                    let x = 1.0 - smooth(2.1, 2.7, a.min(b));
                    if x > 0.0 {
                        over(idx, [0xff; 3], x);
                    }
                }
            }
        }
    }

    // Unpremultiply into ARGB32. The pulse dims the whole icon; the badge
    // only exists in Error mode, which never pulses.
    let dim = if mode == IconMode::Pulse {
        0.35 + 0.65 * (0.5 - 0.5 * (std::f32::consts::TAU * pulse.clamp(0.0, 1.0)).cos())
    } else {
        1.0
    };
    let mut out = vec![0u8; C * C * 4];
    for (i, px) in canvas.iter().enumerate() {
        let a = px[3];
        let (r, g, b) = (px[0] / a.max(1e-6), px[1] / a.max(1e-6), px[2] / a.max(1e-6));
        let o = i * 4;
        out[o] = (a * dim * 255.0).round().clamp(0.0, 255.0) as u8;
        out[o + 1] = (r * 255.0).round().clamp(0.0, 255.0) as u8;
        out[o + 2] = (g * 255.0).round().clamp(0.0, 255.0) as u8;
        out[o + 3] = (b * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    out
}

/// Shortest distance from a point to a line segment (for the badge X).
fn segment_distance(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = dx * dx + dy * dy;
    let t = if len2 <= f32::EPSILON {
        0.0
    } else {
        (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0)
    };
    ((px - (ax + t * dx)).powi(2) + (py - (ay + t * dy)).powi(2)).sqrt()
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
    pulse: f32,
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
            data: compose_icon(&self.base_rgba, self.base_w, &self.status_key, self.pulse),
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

/// A live tray handle plus the flag the pulse thread watches.
pub struct Tray {
    handle: ksni::blocking::Handle<AppTray>,
    pulsing: Arc<AtomicBool>,
}

impl Tray {
    pub fn update<R>(&self, f: impl FnOnce(&mut AppTray) -> R) -> Option<R> {
        self.handle.update(f)
    }

    pub fn set_pulsing(&self, on: bool) {
        self.pulsing.store(on, Ordering::Relaxed);
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
        pulse: 1.0,
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

    // The tray host only re-reads the icon on `update`, so the fade needs its
    // own clock. It runs only while the status is a pulsing one; otherwise
    // the loop just checks an atomic.
    let pulsing = Arc::new(AtomicBool::new(false));
    let pulse_handle = handle.clone();
    let pulse_flag = pulsing.clone();
    std::thread::spawn(move || {
        let start = std::time::Instant::now();
        loop {
            // Do not tick at animation rate while the icon is idle.
            if !pulse_flag.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_secs(1));
                continue;
            }
            std::thread::sleep(std::time::Duration::from_millis(120));
            let phase = (start.elapsed().as_secs_f32() / 2.0) % 1.0;
            if pulse_handle.update(|t| t.pulse = phase).is_none() {
                break;
            }
        }
    });

    Some(Tray { handle, pulsing })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_modes_match_the_sync_state() {
        assert_eq!(icon_mode("synced"), IconMode::Normal);
        assert_eq!(icon_mode("syncing"), IconMode::Pulse);
        assert_eq!(icon_mode("starting"), IconMode::Pulse);
        assert_eq!(icon_mode("signing_in"), IconMode::Pulse);
        for bad in ["error", "needs_login", "paused", "signed_out", "engine_stopped"] {
            assert_eq!(icon_mode(bad), IconMode::Error, "{bad}");
        }
    }

    #[test]
    fn synced_icon_has_no_badge() {
        let argb = compose_icon(&[], 0, "synced", 1.0);
        let o = (56 * 72 + 56) * 4;
        assert_eq!(&argb[o..o + 4], &[0, 0, 0, 0]);
    }

    #[test]
    fn error_icon_draws_a_white_x_on_red() {
        let argb = compose_icon(&[], 0, "error", 1.0);
        // The two strokes cross at the badge center: white.
        let o = (54 * 72 + 54) * 4;
        assert_eq!(&argb[o..o + 4], &[0xff, 0xff, 0xff, 0xff]);
        // Left of the arms, still on the red disc.
        let o = (54 * 72 + 40) * 4;
        assert_eq!(&argb[o..o + 4], &[0xff, 0xdc, 0x26, 0x26]);
    }

    #[test]
    fn pulse_fades_the_whole_icon() {
        let base = vec![255u8; 64 * 64 * 4];
        let full = compose_icon(&base, 64, "syncing", 0.5);
        let dim = compose_icon(&base, 64, "syncing", 0.0);
        let o = 0;
        assert_eq!(full[o], 0xff, "pulse peak should be opaque");
        assert!(dim[o] < 0x60, "pulse trough should be faded: {}", dim[o]);
        assert_eq!(&full[o + 1..o + 4], &[0xff, 0xff, 0xff]);
    }

    #[test]
    fn base_icon_fills_the_slot_and_crops_padding() {
        // A solid 64px base fills 72x66, leaving only the strip below for the
        // badge.
        let base = vec![255u8; 64 * 64 * 4];
        let argb = compose_icon(&base, 64, "synced", 1.0);
        for (x, y) in [(0, 0), (71, 0), (0, 65), (71, 65)] {
            let o = (y * 72 + x) * 4;
            assert_eq!(&argb[o..o + 4], &[0xff, 0xff, 0xff, 0xff], "({x},{y})");
        }
        // Below the icon and clear of the badge.
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
        let argb = compose_icon(&base, 64, "synced", 1.0);
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
