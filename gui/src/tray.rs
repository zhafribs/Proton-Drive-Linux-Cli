//! System tray (freedesktop StatusNotifierItem via ksni) - the port of the
//! PyQt app's QSystemTrayIcon: the app icon with a status dot, Qt's exact
//! menu entries, left-click window toggle, and "Recent files" from the
//! change journal. Menu actions hand messages back to the UI thread over
//! the existing `Back` channel (ksni calls them on its own service thread).

use crate::journal::ChangeRow;
use crate::ui::Back;
use crate::util::rel_time;
use std::sync::mpsc::Sender;

/// Qt's tray dot palette (ACCENT / SUCCESS / WARNING / ERROR / MUTED_GRAY).
fn dot_color(status_key: &str) -> [u8; 3] {
    match status_key {
        "synced" => [0x16, 0xa3, 0x4a],
        "syncing" => [0x6d, 0x4a, 0xff],
        "paused" => [0xd9, 0x77, 0x06],
        // A lost session is recoverable - warn, do not alarm.
        "needs_login" => [0xd9, 0x77, 0x06],
        "error" => [0xdc, 0x26, 0x26],
        _ => [0x9c, 0xa3, 0xaf],
    }
}

fn smooth(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Qt drew a 72x72 canvas: the 64px icon scaled to 66px at (2,2), then a
/// status dot `drawEllipse(52, 52, 18, 18)` with a 4px white pen (so the
/// color fill shows to r=7 and the white ring covers 7..11, center (61,61)).
/// Returns ARGB32 bytes, which is what `ksni::Icon` wants.
pub fn compose_icon(base_rgba: &[u8], base_w: usize, status_key: &str) -> Vec<u8> {
    const C: usize = 72;
    let color = dot_color(status_key);
    let mut canvas: Vec<[f32; 4]> = vec![[0.0; 4]; C * C]; // premultiplied RGBA

    // Base icon: bilinear scale base_w -> 66 at offset (2,2), premultiplied.
    let base_h = if base_w > 0 && base_rgba.len() >= base_w * base_w * 4 {
        base_rgba.len() / (base_w * 4)
    } else {
        0
    };
    if base_h > 0 {
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
        for y in 0..66usize {
            for x in 0..66usize {
                let fx = (x as f32 + 0.5) * base_w as f32 / 66.0 - 0.5;
                let fy = (y as f32 + 0.5) * base_h as f32 / 66.0 - 0.5;
                let (x0f, y0f) = (fx.floor(), fy.floor());
                let (tx, ty) = (fx - x0f, fy - y0f);
                let x0 = (x0f as isize).clamp(0, base_w as isize - 1) as usize;
                let y0 = (y0f as isize).clamp(0, base_h as isize - 1) as usize;
                let x1 = (x0 + 1).min(base_w - 1);
                let y1 = (y0 + 1).min(base_h - 1);
                let a00 = sample(x0, y0);
                let a10 = sample(x1, y0);
                let a01 = sample(x0, y1);
                let a11 = sample(x1, y1);
                let mut px = [0.0f32; 4];
                for ch in 0..4 {
                    let top = a00[ch] * (1.0 - tx) + a10[ch] * tx;
                    let bot = a01[ch] * (1.0 - tx) + a11[ch] * tx;
                    px[ch] = top * (1.0 - ty) + bot * ty;
                }
                // Handle the sub-pixel edge (negative fx) by falling back to
                // the clamped edge tap - close enough at 64 -> 66.
                canvas[(y + 2) * C + (x + 2)] = px;
            }
        }
    }

    // Status dot over the base.
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
    for y in 0..C {
        for x in 0..C {
            let d = ((x as f32 + 0.5 - 61.0).powi(2) + (y as f32 + 0.5 - 61.0).powi(2)).sqrt();
            if d > 12.0 {
                continue;
            }
            let idx = y * C + x;
            // Brush fill first (visible to r=7), then the white pen ring.
            over(idx, color, 1.0 - smooth(6.5, 7.5, d));
            over(idx, [0xff; 3], (1.0 - smooth(10.5, 11.5, d)) * smooth(6.5, 7.5, d));
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
        m.push(item("Open my folder", tx, Back::TrayOpenFolder).into());
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

/// Build and start the tray. Returns `None` when the desktop has no
/// StatusNotifierWatcher (Qt's `isSystemTrayAvailable() == False` - the app
/// keeps working, just without a tray).
pub fn spawn(
    tx: Sender<Back>,
    base_rgba: Vec<u8>,
    base_w: usize,
) -> Option<ksni::blocking::Handle<AppTray>> {
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
    match tray.spawn() {
        Ok(handle) => Some(handle),
        Err(e) => {
            eprintln!("tray unavailable: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dot_center_is_status_color_in_argb() {
        let argb = compose_icon(&[], 0, "synced");
        assert_eq!(argb.len(), 72 * 72 * 4);
        // Pixel (61,61) sits on the dot center: opaque green, A then RGB.
        let o = (61 * 72 + 61) * 4;
        assert_eq!(&argb[o..o + 4], &[0xff, 0x16, 0xa3, 0x4a]);
    }

    #[test]
    fn white_ring_and_clear_corner() {
        let argb = compose_icon(&[], 0, "syncing");
        // 9px out from center: the white pen ring.
        let o = (61 * 72 + 70) * 4;
        assert_eq!(&argb[o..o + 4], &[0xff, 0xff, 0xff, 0xff]);
        // Far corner: beyond dot and (empty) base -> fully transparent.
        let o = (71 * 72 + 71) * 4;
        assert_eq!(&argb[o..o + 4], &[0, 0, 0, 0]);
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
