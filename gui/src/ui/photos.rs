//! Photos: the Proton photo timeline (My Photos) plus a local download folder.
//!
//! The official CLI owns both halves - `photo timeline -d --json` to list and
//! `photo download` to fetch - exactly like the files browser; this page lists,
//! groups by month and asks. There are no thumbnails: the CLI exposes no
//! thumbnail bytes, and decoding JPEG/HEIC would drag an image codec into the
//! binary for a nicety. The timeline is a dated list with name, resolution and
//! size instead.

use super::{widgets, App};
use crate::cli;
use crate::theme;
use eframe::egui;
use std::collections::BTreeMap;

#[derive(Default)]
pub struct PhotoState {
    pub items: Vec<cli::PhotoEntry>,
    /// A fetch has been started (the page only auto-fetches once).
    pub loaded: bool,
    pub loading: bool,
    /// A download is in flight.
    pub busy: bool,
    pub status: String,
    pub error: String,
    /// Selected photo uids.
    pub sel: Vec<String>,
    /// Decoded thumbnails: name -> (file stamp, texture).
    pub tex: std::collections::HashMap<String, (f64, egui::TextureHandle)>,
    /// Thumbnails being decoded right now.
    pub in_flight: std::collections::HashSet<String>,
    /// Names that failed to decode (unsupported format, broken file).
    pub failed: std::collections::HashSet<String>,
    /// "In sync · N in the cloud, M on disk" line from the engine's last cycle.
    pub last_sync: String,
    /// When to re-list automatically (after asking the engine to sync).
    pub refresh_at: Option<f64>,
}

/// Where downloaded photos land: `PROTON_SYNC_PHOTOS_LOCAL`, else
/// `~/ProtonPhotos`.
fn local_dir_from(conf: &BTreeMap<String, String>) -> std::path::PathBuf {
    match conf
        .get("PROTON_SYNC_PHOTOS_LOCAL")
        .map(String::as_str)
        .filter(|s| !s.trim().is_empty())
    {
        Some(v) => super::expand_user(v),
        None => dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("/"))
            .join("ProtonPhotos"),
    }
}

pub fn local_dir(app: &App) -> std::path::PathBuf {
    local_dir_from(&app.conf)
}

/// The page was shown (navigation): list the timeline the first time only.
pub fn on_shown(app: &mut App) {
    // The folder existing is what turns the engine's photo sync on, so create
    // it the first time the page is opened.
    let _ = std::fs::create_dir_all(local_dir(app));
    if !app.photos.loaded {
        refresh(app);
    }
}

/// What the engine's last photo cycle did (`photos-sync.json`).
#[derive(serde::Deserialize, Default)]
struct SyncSummary {
    ts: String,
    remote: u64,
    local: u64,
    uploaded: u64,
    downloaded: u64,
    conflicts: u64,
    error: String,
}

fn read_summary(app: &mut App) -> String {
    let path = app.paths.state_dir.join("photos-sync.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return "Not synced yet - the engine syncs this folder with /photos.".to_string();
    };
    let Ok(s) = serde_json::from_str::<SyncSummary>(&text) else {
        return String::new();
    };
    if !s.error.is_empty() {
        return format!("Sync failed: {}", s.error);
    }
    let when = chrono::DateTime::parse_from_rfc3339(&s.ts)
        .map(|t| crate::util::rel_time(Some(t)))
        .unwrap_or_default();
    let mut line = format!(
        "In sync \u{b7} {} in the cloud, {} on disk",
        s.remote, s.local
    );
    if s.uploaded > 0 || s.downloaded > 0 {
        line.push_str(&format!(
            " \u{b7} last pass {} up, {} down",
            s.uploaded, s.downloaded
        ));
    }
    if s.conflicts > 0 {
        line.push_str(&format!(" \u{b7} {} name clash(es) left alone", s.conflicts));
    }
    if !when.is_empty() {
        line.push_str(&format!(" \u{b7} {when}"));
    }
    line
}

/// Ask the engine for a cycle now (it syncs the folder), then re-list.
pub fn sync_now(app: &mut App) {
    let dir = local_dir(app);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        app.photos.error = format!("cannot create {}: {e}", dir.display());
        return;
    }
    let _ = std::fs::write(&app.paths.wake_file, b"");
    app.photos.error.clear();
    app.photos.status = "Syncing photos with the cloud...".to_string();
    app.photos.refresh_at = Some(app.now() + 10.0);
}

pub fn refresh(app: &mut App) {
    if app.photos.loading || app.photos.busy {
        return;
    }
    app.photos.loaded = true;
    app.photos.loading = true;
    app.photos.error.clear();
    app.photos.status = "Loading the photo timeline...".to_string();
    let tx = app.tx.clone();
    std::thread::spawn(move || {
        let _ = tx.send(super::Back::Photos(cli::fetch_photos()));
    });
}

pub fn on_fetched(app: &mut App, r: Result<Vec<cli::PhotoEntry>, String>) {
    app.photos.loading = false;
    app.photos.last_sync = read_summary(app);
    match r {
        Ok(items) => {
            app.photos.status.clear();
            // Drop selections for photos that are gone.
            let live: Vec<&str> = items.iter().map(|p| p.uid.as_str()).collect();
            app.photos.sel.retain(|u| live.contains(&u.as_str()));
            app.photos.items = items;
        }
        Err(e) => {
            app.photos.status.clear();
            app.photos.error = e;
        }
    }
}

pub fn download(app: &mut App, uids: Vec<String>) {
    if uids.is_empty() || app.photos.busy {
        return;
    }
    let dir = local_dir(app);
    app.photos.busy = true;
    app.photos.error.clear();
    app.photos.status = format!(
        "Downloading {} photo(s) to {}...",
        uids.len(),
        dir.display()
    );
    let tx = app.tx.clone();
    std::thread::spawn(move || {
        let r = cli::download_photos(&uids, &dir);
        let _ = tx.send(super::Back::PhotosDownloaded(r));
    });
}

pub fn on_downloaded(app: &mut App, r: Result<usize, String>) {
    app.photos.busy = false;
    match r {
        Ok(n) => {
            app.photos.error.clear();
            app.photos.status.clear();
            let dir = local_dir(app);
            app.toast(format!("Saved {n} photo(s) to {}", dir.display()));
        }
        Err(e) => {
            app.photos.status.clear();
            app.photos.error = e;
        }
    }
}

/// "2026-08-21 12:55" -> "August 2026", for the month headers.
fn month_of(capture: &str) -> String {
    chrono::NaiveDateTime::parse_from_str(capture, "%Y-%m-%d %H:%M")
        .map(|dt| dt.format("%B %Y").to_string())
        .unwrap_or_else(|_| "Undated".to_string())
}

/// "2026-08-21 12:55" -> "21 Aug 2026, 12:55" (the row's date column).
fn pretty_date(capture: &str) -> String {
    chrono::NaiveDateTime::parse_from_str(capture, "%Y-%m-%d %H:%M")
        .map(|dt| dt.format("%d %b %Y, %H:%M").to_string())
        .unwrap_or_else(|_| capture.to_string())
}

/// Timeline order (newest first) grouped into months, newest month first.
/// Cloned so rows can be drawn while the app borrows mutably.
fn months(items: &[cli::PhotoEntry]) -> Vec<(String, Vec<cli::PhotoEntry>)> {
    let mut out: Vec<(String, Vec<cli::PhotoEntry>)> = Vec::new();
    for p in items {
        let m = month_of(&p.capture);
        match out.last_mut() {
            Some((name, group)) if *name == m => group.push(p.clone()),
            _ => out.push((m, vec![p.clone()])),
        }
    }
    out
}

fn size_label(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b >= KB * KB * KB {
        format!("{:.1} GB", b / (KB * KB * KB))
    } else if b >= KB * KB {
        format!("{:.1} MB", b / (KB * KB))
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

/// Longest edge of a cached thumbnail, in pixels.
const THUMB: usize = 256;

/// Decode a photo file into a downscaled RGBA thumbnail.
///
/// PNG goes through the crate the app already ships; JPEG through `zune-jpeg`
/// (pure Rust, no C). Anything else (HEIC from an iPhone, say) is reported as
/// unsupported so the tile can show a placeholder instead of a broken image.
fn decode_thumb(bytes: &[u8]) -> Result<(usize, usize, Vec<u8>), String> {
    let (w, h, rgba) = if bytes.starts_with(&[0xFF, 0xD8]) {
        use zune_jpeg::zune_core::{colorspace::ColorSpace, options::DecoderOptions};
        let opts = DecoderOptions::new_fast().jpeg_set_out_colorspace(ColorSpace::RGBA);
        let mut dec =
            zune_jpeg::JpegDecoder::new_with_options(std::io::Cursor::new(bytes), opts);
        let data = dec.decode().map_err(|e| format!("jpeg: {e:?}"))?;
        let info = dec.info().ok_or("jpeg: no header")?;
        (info.width as usize, info.height as usize, data)
    } else if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        super::decode_png(bytes).ok_or("png: decode failed")?
    } else {
        return Err("unsupported image format".to_string());
    };
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return Err("decoded image is empty".to_string());
    }
    Ok(downscale(&rgba, w, h, THUMB))
}

/// Box-filter downscale to at most `max` px on the long edge. Decoding a
/// 4000px photo only to hand the GPU a 150px tile wastes upload bandwidth, and
/// averaging beats nearest-neighbour for thumbnails.
fn downscale(rgba: &[u8], w: usize, h: usize, max: usize) -> (usize, usize, Vec<u8>) {
    let step = w.max(h).div_ceil(max).max(1);
    if step == 1 {
        return (w, h, rgba.to_vec());
    }
    let (ow, oh) = (w.div_ceil(step), h.div_ceil(step));
    let mut out = vec![0u8; ow * oh * 4];
    for oy in 0..oh {
        for ox in 0..ow {
            let x0 = ox * step;
            let y0 = oy * step;
            let (mut r, mut g, mut b, mut a, mut n) = (0u32, 0u32, 0u32, 0u32, 0u32);
            for sy in y0..(y0 + step).min(h) {
                for sx in x0..(x0 + step).min(w) {
                    let i = (sy * w + sx) * 4;
                    r += rgba[i] as u32;
                    g += rgba[i + 1] as u32;
                    b += rgba[i + 2] as u32;
                    a += rgba[i + 3] as u32;
                    n += 1;
                }
            }
            let o = (oy * ow + ox) * 4;
            let n = n.max(1);
            out[o] = (r / n) as u8;
            out[o + 1] = (g / n) as u8;
            out[o + 2] = (b / n) as u8;
            out[o + 3] = (a / n) as u8;
        }
    }
    (ow, oh, out)
}

/// The file's mtime in seconds, as the cache stamp (0 when unreadable).
fn file_stamp(path: &std::path::Path) -> f64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Decode a photo's thumbnail in the background (max 4 at a time).
fn request_thumb(app: &mut App, name: &str, path: std::path::PathBuf, stamp: f64) {
    if app.photos.in_flight.len() >= 4 || app.photos.in_flight.contains(name) {
        return;
    }
    app.photos.in_flight.insert(name.to_string());
    let tx = app.tx.clone();
    let name = name.to_string();
    std::thread::spawn(move || {
        let result = std::fs::read(&path)
            .map_err(|e| format!("read: {e}"))
            .and_then(|bytes| decode_thumb(&bytes));
        let _ = tx.send(super::Back::PhotoThumb { name, stamp, result });
    });
}

pub fn draw(app: &mut App, ui: &mut egui::Ui) {
    if let Some(t) = app.photos.refresh_at
        && app.now() >= t
    {
        app.photos.refresh_at = None;
        app.photos.status.clear();
        refresh(app);
    }
    let dir = local_dir(app);
    egui::ScrollArea::vertical()
        .id_salt("photos-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Photos").strong().size(24.0));
                if !app.photos.items.is_empty() {
                    let total: u64 = app.photos.items.iter().map(|p| p.size).sum();
                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new(format!(
                            "{} photo(s)  \u{b7}  {}",
                            app.photos.items.len(),
                            size_label(total)
                        ))
                        .weak(),
                    );
                }
            });
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new(format!("Two-way sync with /photos \u{b7} {}", dir.display()))
                    .weak()
                    .size(12.0),
            );
            if !app.photos.last_sync.is_empty() {
                ui.label(
                    egui::RichText::new(&app.photos.last_sync)
                        .weak()
                        .size(12.0),
                );
            }
            ui.add_space(10.0);

            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(!app.photos.busy, egui::Button::new("Sync now"))
                    .on_hover_text(
                        "Upload new files from this folder and download new cloud photos",
                    )
                    .clicked()
                {
                    sync_now(app);
                }
                if ui
                    .add_enabled(
                        !app.photos.loading && !app.photos.busy,
                        egui::Button::new("Refresh"),
                    )
                    .clicked()
                {
                    refresh(app);
                }
                if app.photos.sel.is_empty() {
                    if ui
                        .add_enabled(
                            !app.photos.busy && !app.photos.items.is_empty(),
                            egui::Button::new("Download all"),
                        )
                        .clicked()
                    {
                        let all: Vec<String> =
                            app.photos.items.iter().map(|p| p.uid.clone()).collect();
                        download(app, all);
                    }
                } else {
                    let sel = app.photos.sel.clone();
                    if ui
                        .add_enabled(
                            !app.photos.busy,
                            egui::Button::new(format!("Download {} selected", sel.len())),
                        )
                        .clicked()
                    {
                        download(app, sel);
                    }
                    if ui.button("Clear selection").clicked() {
                        app.photos.sel.clear();
                    }
                }
                if ui.button("Open folder").clicked() {
                    let _ = std::fs::create_dir_all(&dir);
                    App::open_url(&dir.to_string_lossy());
                }
            });

            if !app.photos.error.is_empty() {
                ui.add_space(10.0);
                ui.label(egui::RichText::new(&app.photos.error).color(theme::ERROR));
            }
            if !app.photos.status.is_empty() {
                ui.add_space(10.0);
                ui.label(egui::RichText::new(&app.photos.status).weak());
            }
            if app.photos.items.is_empty() {
                if !app.photos.loading && app.photos.error.is_empty() {
                    ui.add_space(18.0);
                    ui.label(
                        egui::RichText::new("No photos in your timeline yet.")
                            .weak()
                            .size(13.5),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("Photos you upload from the Proton apps appear here.")
                            .weak()
                            .size(12.0),
                    );
                }
                return;
            }

            for (month, group) in months(&app.photos.items) {
                ui.add_space(10.0);
                widgets::card(ui, Some(&month), |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for p in &group {
                            tile(ui, app, p, &dir);
                        }
                    });
                });
            }
            ui.add_space(10.0);
        });
}

/// One tile of the grid: the picture when it is on disk, a placeholder while it
/// is not (with a Download button), name and date underneath.
const TILE: f32 = 148.0;

fn tile(ui: &mut egui::Ui, app: &mut App, p: &cli::PhotoEntry, dir: &std::path::Path) {
    let path = dir.join(&p.name);
    let local = path.is_file();
    let stamp = if local { file_stamp(&path) } else { 0.0 };
    if local
        && !app.photos.failed.contains(&p.name)
        && app.photos.tex.get(&p.name).is_none_or(|(s, _)| (*s - stamp).abs() >= 0.5)
    {
        request_thumb(app, &p.name, path.clone(), stamp);
    }
    let tex = app.photos.tex.get(&p.name).map(|(_, t)| t.clone());
    let selected = app.photos.sel.iter().any(|u| u == &p.uid);

    ui.allocate_ui_with_layout(
        egui::vec2(TILE, TILE + 40.0),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            let (rect, resp) =
                ui.allocate_exact_size(egui::vec2(TILE, TILE), egui::Sense::click());
            ui.painter().rect_filled(
                rect,
                egui::CornerRadius::same(10),
                theme::glass_fill(ui.visuals()),
            );
            match &tex {
                Some(t) => {
                    let [tw, th] = t.size();
                    let (tw, th) = (tw.max(1) as f32, th.max(1) as f32);
                    let scale = (rect.width() / tw).min(rect.height() / th);
                    let fitted = egui::Rect::from_center_size(
                        rect.center(),
                        egui::vec2(tw * scale, th * scale),
                    );
                    ui.painter().image(
                        t.id(),
                        fitted,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                }
                None => {
                    // Not on disk yet, still decoding, or an unsupported format.
                    let icon = egui::Rect::from_center_size(
                        rect.center() - egui::vec2(0.0, 8.0),
                        egui::vec2(30.0, 30.0),
                    );
                    widgets::nav_icon(
                        ui.painter(),
                        icon,
                        widgets::NavIcon::Photos,
                        ui.visuals().weak_text_color(),
                    );
                }
            }
            if selected {
                ui.painter().rect_stroke(
                    rect,
                    egui::CornerRadius::same(10),
                    egui::Stroke::new(2.0, theme::accent(ui.visuals())),
                    egui::StrokeKind::Inside,
                );
            }
            if resp.clicked() {
                if selected {
                    app.photos.sel.retain(|u| u != &p.uid);
                } else {
                    app.photos.sel.push(p.uid.clone());
                }
            }
            let when = if p.capture.is_empty() {
                "No capture date".to_string()
            } else {
                pretty_date(&p.capture)
            };
            resp.on_hover_text(format!("{when}\n{}", p.name));

            let label = widgets::elide(&p.name, 17);
            ui.label(egui::RichText::new(label).size(11.0));
            if local {
                ui.label(
                    egui::RichText::new(when)
                        .weak()
                        .size(10.0),
                );
            } else {
                if ui
                    .add_enabled(!app.photos.busy, egui::Button::new("Download"))
                    .clicked()
                {
                    download(app, vec![p.uid.clone()]);
                }
            }
        },
    );
}



#[cfg(test)]
mod tests {
    use super::*;

    fn photo(name: &str, capture: &str) -> cli::PhotoEntry {
        cli::PhotoEntry {
            uid: format!("uid-{name}"),
            name: name.to_string(),
            size: 1024,
            capture: capture.to_string(),
            width: 100,
            height: 50,
        }
    }

    /// The timeline is grouped into month cards, newest month first.
    #[test]
    fn timeline_groups_by_month() {
        let items = vec![
            photo("c.png", "2026-08-21 12:55"),
            photo("b.png", "2026-08-02 09:00"),
            photo("a.jpg", "2026-07-31 23:59"),
            photo("old.jpg", ""),
        ];
        let groups = months(&items);
        let names: Vec<&str> = groups.iter().map(|(m, _)| m.as_str()).collect();
        assert_eq!(names, vec!["August 2026", "July 2026", "Undated"]);
        assert_eq!(groups[0].1.len(), 2);
        assert_eq!(groups[0].1[0].name, "c.png");
        assert_eq!(groups[2].1[0].name, "old.jpg");
    }

    #[test]
    fn dates_and_sizes_read_like_a_gallery() {
        assert_eq!(pretty_date("2026-08-21 12:55"), "21 Aug 2026, 12:55");
        assert_eq!(pretty_date("garbage"), "garbage");
        assert_eq!(size_label(900), "900 B");
        assert_eq!(size_label(2 * 1024 * 1024), "2.0 MB");
        assert_eq!(size_label(3 * 1024), "3 KB");
    }

    /// Photos land in ~/ProtonPhotos unless the conf says otherwise.
    #[test]
    fn photos_land_in_home_protonphotos() {
        let empty = BTreeMap::new();
        assert!(local_dir_from(&empty).ends_with("ProtonPhotos"));
        let mut conf = BTreeMap::new();
        conf.insert(
            "PROTON_SYNC_PHOTOS_LOCAL".to_string(),
            "~/Pictures/Proton".to_string(),
        );
        assert!(local_dir_from(&conf).ends_with("Pictures/Proton"));
        conf.insert("PROTON_SYNC_PHOTOS_LOCAL".to_string(), "   ".to_string());
        assert!(
            local_dir_from(&conf).ends_with("ProtonPhotos"),
            "a blank key falls back to the default"
        );
    }
}


