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
    if !app.photos.loaded {
        refresh(app);
    }
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

pub fn draw(app: &mut App, ui: &mut egui::Ui) {
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
                egui::RichText::new(format!("Downloads go to {}", dir.display()))
                    .weak()
                    .size(12.0),
            );
            ui.add_space(10.0);

            ui.horizontal_wrapped(|ui| {
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
                    for p in &group {
                        row(ui, app, p, &dir);
                    }
                });
            }
            ui.add_space(10.0);
        });
}



fn row(ui: &mut egui::Ui, app: &mut App, p: &cli::PhotoEntry, dir: &std::path::Path) {
    let selected = app.photos.sel.iter().any(|u| u == &p.uid);
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
        widgets::nav_icon(
            ui.painter(),
            rect.shrink(2.0),
            widgets::NavIcon::Photos,
            if selected {
                theme::accent(ui.visuals())
            } else {
                ui.visuals().weak_text_color()
            },
        );
        let when = if p.capture.is_empty() {
            "No capture date".to_string()
        } else {
            pretty_date(&p.capture)
        };
        if ui
            .selectable_label(selected, egui::RichText::new(&p.name).size(13.5))
            .on_hover_text(&when)
            .clicked()
        {
            if selected {
                app.photos.sel.retain(|u| u != &p.uid);
            } else {
                app.photos.sel.push(p.uid.clone());
            }
        }
        let mut meta: Vec<String> = Vec::new();
        if p.width > 0 && p.height > 0 {
            meta.push(format!("{}x{}", p.width, p.height));
        }
        if p.size > 0 {
            meta.push(size_label(p.size));
        }
        meta.push(if p.capture.is_empty() {
            "no date".to_string()
        } else {
            pretty_date(&p.capture)
        });
        if dir.join(&p.name).is_file() {
            meta.push("saved".to_string());
        }
        ui.label(
            egui::RichText::new(meta.join("  \u{b7}  "))
                .weak()
                .size(11.5),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled(!app.photos.busy, egui::Button::new("Download"))
                .clicked()
            {
                download(app, vec![p.uid.clone()]);
            }
        });
    });
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

