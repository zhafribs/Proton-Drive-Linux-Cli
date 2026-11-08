//! Recovery: the local trash that the engine fills when a synced file is
//! deleted locally. Restore puts a file back at its original relative path;
//! permanent delete removes it from the trash subtree.
//!
//! The listing and every mutation run on a worker thread. The core manifest
//! takes a short lock, so the engine can record a new trash while the GUI is
//! looking at the list.

use super::{widgets, App, Back};
use eframe::egui;
use proton_core::local_trash::{self, Entry};
use std::path::PathBuf;

#[derive(Default)]
pub struct RecoveryState {
    pub entries: Vec<Entry>,
    pub loaded: bool,
    pub loading: bool,
    pub busy: bool,
    pub error: Option<String>,
    pub status: String,
    /// `stored_rel` of the entry whose permanent-delete button is armed.
    pub confirm_delete: Option<String>,
}

pub fn draw(app: &mut App, ui: &mut egui::Ui) {
    if !app.recovery.loaded && !app.recovery.loading && app.recovery.error.is_none() {
        refresh(app);
    }

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                widgets::page_title(ui, "Recovery");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(
                            !app.recovery.busy && !app.recovery.loading,
                            egui::Button::new("Refresh"),
                        )
                        .on_hover_text("Re-read the local trash manifest and folder")
                        .clicked()
                    {
                        refresh(app);
                    }
                });
            });
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(
                    "Files deleted locally are kept here until you restore or permanently delete them.",
                )
                .size(12.5)
                .weak(),
            );
            ui.add_space(12.0);

            if !app.recovery.status.is_empty() {
                ui.label(
                    egui::RichText::new(&app.recovery.status)
                        .size(12.5)
                        .color(crate::theme::success_text(ui.visuals())),
                );
                ui.add_space(6.0);
            }

            if let Some(error) = app.recovery.error.clone() {
                let load_failed = app.recovery.entries.is_empty();
                widgets::card(
                    ui,
                    Some(if load_failed {
                        "Local trash could not be read"
                    } else {
                        "Recovery action failed"
                    }),
                    |ui| {
                        ui.label(egui::RichText::new(error).size(12.5));
                        ui.add_space(8.0);
                        ui.horizontal_wrapped(|ui| {
                            let retry = if load_failed { "Try again" } else { "Dismiss" };
                            if ui.button(retry).clicked() {
                                app.recovery.error = None;
                                app.recovery.loaded = false;
                                if load_failed {
                                    refresh(app);
                                }
                            }
                            if ui.button("Open trash folder").clicked() {
                                open_trash_folder(app);
                            }
                        });
                    },
                );
                ui.add_space(10.0);
                if load_failed {
                    return;
                }
            }

            if app.recovery.loading {
                ui.horizontal(|ui| {
                    widgets::busy_spinner(ui, app.prefs.motion_enabled());
                    ui.label(egui::RichText::new("Loading local trash\u{2026}").weak());
                });
                return;
            }

            let entries = app.recovery.entries.clone();
            if entries.is_empty() {
                widgets::card(ui, Some("No local trash"), |ui| {
                    ui.label(
                        egui::RichText::new(
                            "Files deleted locally will appear here and can be restored to their original folder.",
                        )
                        .size(12.5)
                        .weak(),
                    );
                    ui.add_space(8.0);
                    if ui.button("Open trash folder").clicked() {
                        open_trash_folder(app);
                    }
                });
                return;
            }

            let count = entries.len();
            ui.label(
                egui::RichText::new(if count == 1 {
                    "1 deleted item".to_string()
                } else {
                    format!("{count} deleted items")
                })
                .size(12.0)
                .weak(),
            );
            ui.add_space(6.0);

            for entry in &entries {
                entry_card(app, ui, entry);
                ui.add_space(8.0);
            }

            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                if ui.button("Open trash folder").clicked() {
                    open_trash_folder(app);
                }
                ui.label(
                    egui::RichText::new(format!(
                        "Manifest: {}",
                        app.paths
                            .state_dir
                            .join("trash-manifest.json")
                            .display()
                    ))
                    .size(11.5)
                    .weak(),
                );
            });
        });
}

fn entry_card(app: &mut App, ui: &mut egui::Ui, entry: &Entry) {
    egui::Frame::new()
        .fill(crate::theme::glass_fill(ui.visuals()))
        .stroke(crate::theme::glass_stroke(ui.visuals()))
        .corner_radius(egui::CornerRadius::same(12))
        .inner_margin(egui::Margin::symmetric(14, 11))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.add(
                egui::Label::new(
                    egui::RichText::new(&entry.original_rel)
                        .strong()
                        .size(13.5),
                )
                .truncate(),
            );
            let size = if entry.kind == "dir" {
                "folder".to_string()
            } else {
                super::cloud::fmt_size(entry.size)
            };
            let mut meta = if size.is_empty() {
                deleted_at(entry.trashed_at)
            } else {
                format!("{size} \u{b7} {}", deleted_at(entry.trashed_at))
            };
            if entry.legacy {
                meta.push_str(" \u{b7} legacy entry");
            }
            ui.add(
                egui::Label::new(egui::RichText::new(meta).size(11.5).weak())
                    .truncate(),
            );
            if entry.legacy {
                ui.label(
                    egui::RichText::new(
                        "The original name was not recorded; the stored path is shown.",
                    )
                    .size(11.5)
                    .weak(),
                );
            }
            ui.add_space(7.0);
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        !app.recovery.busy,
                        egui::Button::new("Restore"),
                    )
                    .on_hover_text(
                        "Put the item back at its original path. Existing files are never overwritten.",
                    )
                    .clicked()
                {
                    start_restore(app, entry.clone());
                }
                if app.recovery.confirm_delete.as_deref() == Some(&entry.stored_rel) {
                    let delete = ui
                        .add_enabled_ui(!app.recovery.busy, |ui| {
                            widgets::danger_ghost_button(ui, "Confirm permanent delete")
                        })
                        .inner;
                    if delete {
                        app.recovery.confirm_delete = None;
                        start_delete(app, entry.clone());
                    }
                    if ui.button("Cancel").clicked() {
                        app.recovery.confirm_delete = None;
                    }
                } else if ui
                    .add_enabled_ui(!app.recovery.busy, |ui| {
                        widgets::danger_ghost_button(ui, "Delete permanently")
                    })
                    .inner
                {
                    app.recovery.confirm_delete = Some(entry.stored_rel.clone());
                }
            });
        });
}

fn refresh(app: &mut App) {
    app.recovery.loading = true;
    app.recovery.error = None;
    let state = app.paths.state_dir.clone();
    let tx = app.tx.clone();
    std::thread::spawn(move || {
        let result = local_trash::entries(&state);
        let _ = tx.send(Back::RecoveryLoaded(result));
    });
}

pub fn on_loaded(app: &mut App, result: Result<Vec<Entry>, String>) {
    app.recovery.loading = false;
    app.recovery.loaded = true;
    match result {
        Ok(entries) => {
            app.recovery.entries = entries;
            app.recovery.error = None;
        }
        Err(error) => {
            app.recovery.entries.clear();
            app.recovery.error = Some(error);
        }
    }
}

pub fn on_result(app: &mut App, ok: bool, message: String) {
    app.recovery.busy = false;
    if ok {
        app.recovery.status = message;
        app.recovery.loaded = false;
        app.recovery.error = None;
    } else {
        app.recovery.status.clear();
        app.recovery.error = Some(message);
    }
}

fn start_restore(app: &mut App, entry: Entry) {
    let Some(root) = sync_root(app) else {
        app.toast_error("Choose a sync folder before restoring a file.");
        return;
    };
    let state = app.paths.state_dir.clone();
    let tx = app.tx.clone();
    let name = entry.original_rel.clone();
    app.recovery.busy = true;
    app.recovery.error = None;
    std::thread::spawn(move || {
        let result = local_trash::restore(&state, &root, &entry);
        let ok = result.is_ok();
        let message = match result {
            Ok(()) => format!("Restored {name}."),
            Err(error) => error,
        };
        let _ = tx.send(Back::RecoveryResult { ok, message });
    });
}

fn start_delete(app: &mut App, entry: Entry) {
    let state = app.paths.state_dir.clone();
    let tx = app.tx.clone();
    let name = entry.original_rel.clone();
    app.recovery.busy = true;
    app.recovery.error = None;
    std::thread::spawn(move || {
        let result = local_trash::delete(&state, &entry);
        let ok = result.is_ok();
        let message = match result {
            Ok(()) => format!("Permanently deleted {name}."),
            Err(error) => error,
        };
        let _ = tx.send(Back::RecoveryResult { ok, message });
    });
}

fn sync_root(app: &App) -> Option<PathBuf> {
    app.conf
        .get("PROTON_SYNC_LOCAL")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
}

fn open_trash_folder(app: &App) {
    let path = app.paths.state_dir.join("trash");
    if std::fs::create_dir_all(&path).is_ok() {
        let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
    }
}

fn deleted_at(secs: u64) -> String {
    if secs == 0 {
        return "deleted earlier".into();
    }
    let dt = chrono::DateTime::<chrono::Utc>::from_timestamp(secs as i64, 0)
        .map(|d| d.fixed_offset());
    format!("deleted {}", crate::util::rel_time(dt))
}
