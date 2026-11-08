//! Recovery: conflict copies created by the engine and the local trash that
//! receives local deletions. Both are reviewed here, and every destructive
//! action moves data to the local trash first.
//!
//! Listings and mutations run on worker threads. The core manifest takes a
//! short lock, so the engine can record a new trash while the GUI is reading.

use super::{widgets, App, Back};
use eframe::egui;
use proton_core::conflict::{self, Conflict};
use proton_core::local_trash::{self, Entry};
use proton_core::state;
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ConflictChoice {
    KeepCurrent,
    UseConflict,
}

#[derive(Clone)]
pub struct ConflictRow {
    pub conflict: Conflict,
    pub remote_mtime: Option<String>,
}

#[derive(Default)]
pub struct RecoveryState {
    // Local trash.
    pub entries: Vec<Entry>,
    pub loaded: bool,
    pub loading: bool,
    pub busy: bool,
    pub error: Option<String>,
    pub status: String,
    /// `stored_rel` of the entry whose permanent-delete button is armed.
    pub confirm_delete: Option<String>,
    // Conflict copies.
    pub conflicts: Vec<ConflictRow>,
    pub conflicts_loaded: bool,
    pub conflicts_loading: bool,
    pub conflict_busy: bool,
    pub conflict_error: Option<String>,
    pub conflict_status: String,
    /// `(conflict_rel, choice)` waiting for a confirmation click.
    pub confirm_conflict: Option<(String, ConflictChoice)>,
}

pub fn draw(app: &mut App, ui: &mut egui::Ui) {
    if !app.recovery.loaded && !app.recovery.loading && app.recovery.error.is_none() {
        refresh_trash(app);
    }
    if !app.recovery.conflicts_loaded
        && !app.recovery.conflicts_loading
        && app.recovery.conflict_error.is_none()
    {
        refresh_conflicts(app);
    }

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                widgets::page_title(ui, "Recovery");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let busy = app.recovery.busy
                        || app.recovery.loading
                        || app.recovery.conflict_busy
                        || app.recovery.conflicts_loading;
                    if ui
                        .add_enabled(!busy, egui::Button::new("Refresh"))
                        .on_hover_text("Re-read conflict copies and the local trash")
                        .clicked()
                    {
                        refresh_trash(app);
                        refresh_conflicts(app);
                    }
                });
            });
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(
                    "Review conflict copies and restore files that were deleted locally.",
                )
                .size(12.5)
                .weak(),
            );
            ui.add_space(14.0);

            draw_conflicts(app, ui);
            ui.add_space(16.0);
            draw_trash(app, ui);
        });
}

fn draw_conflicts(app: &mut App, ui: &mut egui::Ui) {
    widgets::section_label(ui, "Conflict copies");
    ui.add_space(3.0);
    ui.label(
        egui::RichText::new(
            "The local edit is at the canonical name. The conflict copy is the cloud revision kept beside it.",
        )
        .size(12.0)
        .weak(),
    );
    ui.add_space(6.0);

    if !app.recovery.conflict_status.is_empty() {
        ui.label(
            egui::RichText::new(&app.recovery.conflict_status)
                .size(12.5)
                .color(crate::theme::success_text(ui.visuals())),
        );
        ui.add_space(6.0);
    }

    if let Some(error) = app.recovery.conflict_error.clone() {
        let load_failed = app.recovery.conflicts.is_empty();
        widgets::card(
            ui,
            Some(if load_failed {
                "Conflicts could not be read"
            } else {
                "Conflict action failed"
            }),
            |ui| {
                ui.label(egui::RichText::new(error).size(12.5));
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .button(if load_failed { "Try again" } else { "Dismiss" })
                        .clicked()
                    {
                        app.recovery.conflict_error = None;
                        app.recovery.conflicts_loaded = false;
                        if load_failed {
                            refresh_conflicts(app);
                        }
                    }
                });
            },
        );
        ui.add_space(10.0);
        if load_failed {
            return;
        }
    }

    if app.recovery.conflicts_loading {
        ui.horizontal(|ui| {
            widgets::busy_spinner(ui, app.prefs.motion_enabled());
            ui.label(egui::RichText::new("Scanning for conflict copies\u{2026}").weak());
        });
        return;
    }

    let rows = app.recovery.conflicts.clone();
    if rows.is_empty() {
        widgets::card(ui, Some("No conflict copies"), |ui| {
            ui.label(
                egui::RichText::new(
                    "When both sides change, the engine keeps the cloud revision here as a separate file and syncs the local edit normally. Nothing was overwritten.",
                )
                .size(12.5)
                .weak(),
            );
        });
        return;
    }

    ui.label(
        egui::RichText::new(if rows.len() == 1 {
            "1 conflict to review".to_string()
        } else {
            format!("{} conflicts to review", rows.len())
        })
        .size(12.0)
        .weak(),
    );
    ui.add_space(6.0);
    for row in &rows {
        conflict_card(app, ui, row);
        ui.add_space(8.0);
    }
}

fn conflict_card(app: &mut App, ui: &mut egui::Ui, row: &ConflictRow) {
    let c = &row.conflict;
    egui::Frame::new()
        .fill(crate::theme::glass_fill(ui.visuals()))
        .stroke(crate::theme::glass_stroke(ui.visuals()))
        .corner_radius(egui::CornerRadius::same(12))
        .inner_margin(egui::Margin::symmetric(14, 11))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.add(
                egui::Label::new(egui::RichText::new(&c.canonical_rel).strong().size(13.5))
                    .truncate(),
            );
            ui.add(
                egui::Label::new(
                    egui::RichText::new(format!("Conflict copy: {}", c.conflict_rel))
                        .size(11.5)
                        .weak(),
                )
                .truncate(),
            );
            let local = fs_time(c.mtime_ns);
            let size = super::cloud::fmt_size(c.size);
            let mut meta = if size.is_empty() {
                format!("conflict kept {local}")
            } else {
                format!("{size} \u{b7} conflict kept {local}")
            };
            if let Some(remote) = &row.remote_mtime {
                meta.push_str(&format!(" \u{b7} last synced cloud time {remote}"));
            }
            ui.add(egui::Label::new(egui::RichText::new(meta).size(11.5).weak()).truncate());
            if !c.canonical_exists {
                ui.label(
                    egui::RichText::new(
                        "The canonical file is missing; using the conflict copy will restore it.",
                    )
                    .size(11.5)
                    .color(crate::theme::warning_text(ui.visuals())),
                );
            }
            ui.add_space(7.0);

            let armed = app
                .recovery
                .confirm_conflict
                .as_ref()
                .is_some_and(|(rel, _)| rel == &c.conflict_rel);
            if armed {
                let choice = app.recovery.confirm_conflict.as_ref().unwrap().1;
                let (question, confirm_label) = match choice {
                    ConflictChoice::UseConflict => (
                        "Replace the canonical file with the conflict copy? The current file moves to local trash first.",
                        "Use conflict copy",
                    ),
                    ConflictChoice::KeepCurrent => (
                        "Move the conflict copy to local trash? The canonical file stays.",
                        "Keep current",
                    ),
                };
                ui.label(egui::RichText::new(question).size(12.0));
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(!app.recovery.conflict_busy, egui::Button::new(confirm_label))
                        .clicked()
                    {
                        app.recovery.confirm_conflict = None;
                        start_conflict_action(app, row, choice);
                    }
                    if ui.button("Cancel").clicked() {
                        app.recovery.confirm_conflict = None;
                    }
                });
                return;
            }

            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        !app.recovery.conflict_busy,
                        egui::Button::new("Use conflict copy"),
                    )
                    .on_hover_text("Put the cloud revision at the canonical name; the current file moves to local trash.")
                    .clicked()
                {
                    app.recovery.confirm_conflict =
                        Some((c.conflict_rel.clone(), ConflictChoice::UseConflict));
                }
                if ui
                    .add_enabled(
                        c.canonical_exists && !app.recovery.conflict_busy,
                        egui::Button::new("Keep current"),
                    )
                    .on_hover_text("Keep the canonical file; move the conflict copy to local trash.")
                    .clicked()
                {
                    app.recovery.confirm_conflict =
                        Some((c.conflict_rel.clone(), ConflictChoice::KeepCurrent));
                }
                if ui
                    .add_enabled(
                        !app.recovery.conflict_busy,
                        egui::Button::new("Keep both"),
                    )
                    .on_hover_text("Leave both files in place and hide this conflict until one changes.")
                    .clicked()
                {
                    start_keep_both(app, row);
                }
                if ui.button("Open folder").clicked() {
                    open_parent(app, &c.canonical_rel);
                }
            });
        });
}

fn draw_trash(app: &mut App, ui: &mut egui::Ui) {
    widgets::section_label(ui, "Local trash");
    ui.add_space(3.0);
    ui.label(
        egui::RichText::new(
            "Files deleted locally are kept here until you restore or permanently delete them.",
        )
        .size(12.0)
        .weak(),
    );
    ui.add_space(6.0);

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
                    if ui
                        .button(if load_failed { "Try again" } else { "Dismiss" })
                        .clicked()
                    {
                        app.recovery.error = None;
                        app.recovery.loaded = false;
                        if load_failed {
                            refresh_trash(app);
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
        widgets::card(ui, None, |ui| {
            ui.label(
                egui::RichText::new(
                    "No local trash. Files deleted locally will appear here and can be restored to their original folder.",
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

    ui.label(
        egui::RichText::new(if entries.len() == 1 {
            "1 deleted item".to_string()
        } else {
            format!("{} deleted items", entries.len())
        })
        .size(12.0)
        .weak(),
    );
    ui.add_space(6.0);
    for entry in &entries {
        entry_card(app, ui, entry);
        ui.add_space(8.0);
    }
    ui.add_space(2.0);
    ui.horizontal_wrapped(|ui| {
        if ui.button("Open trash folder").clicked() {
            open_trash_folder(app);
        }
        ui.label(
            egui::RichText::new(format!(
                "Manifest: {}",
                app.paths.state_dir.join("trash-manifest.json").display()
            ))
            .size(11.5)
            .weak(),
        );
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
                    .add_enabled(!app.recovery.busy, egui::Button::new("Restore"))
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

fn refresh_trash(app: &mut App) {
    app.recovery.loading = true;
    app.recovery.error = None;
    let state_dir = app.paths.state_dir.clone();
    let tx = app.tx.clone();
    std::thread::spawn(move || {
        let result = local_trash::entries(&state_dir);
        let _ = tx.send(Back::RecoveryLoaded(result));
    });
}

fn refresh_conflicts(app: &mut App) {
    let Some(root) = sync_root(app) else {
        app.recovery.conflicts_loaded = true;
        app.recovery.conflicts_loading = false;
        app.recovery.conflict_error =
            Some("Choose a sync folder before reviewing conflicts.".into());
        return;
    };
    app.recovery.conflicts_loading = true;
    app.recovery.conflict_error = None;
    let state_dir = app.paths.state_dir.clone();
    let tx = app.tx.clone();
    std::thread::spawn(move || {
        let result = (|| -> Result<Vec<ConflictRow>, String> {
            let acked = conflict::acked(&state_dir);
            let mut list = conflict::scan(&root)?;
            list.retain(|c| acked.get(&c.conflict_rel) != Some(&c.mtime_ns));
            let history = state::read(&state_dir.join("state.json"));
            Ok(list
                .into_iter()
                .map(|conflict| {
                    let remote_mtime = history
                        .get(&conflict.canonical_rel)
                        .and_then(|e| e.r_mtime.clone());
                    ConflictRow {
                        conflict,
                        remote_mtime,
                    }
                })
                .collect())
        })();
        let _ = tx.send(Back::ConflictsLoaded(result));
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

pub fn on_conflicts_loaded(app: &mut App, result: Result<Vec<ConflictRow>, String>) {
    app.recovery.conflicts_loading = false;
    app.recovery.conflicts_loaded = true;
    match result {
        Ok(rows) => {
            app.recovery.conflicts = rows;
            app.recovery.conflict_error = None;
        }
        Err(error) => {
            app.recovery.conflicts.clear();
            app.recovery.conflict_error = Some(error);
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

pub fn on_conflict_result(app: &mut App, ok: bool, message: String) {
    app.recovery.conflict_busy = false;
    if ok {
        app.recovery.conflict_status = message;
        app.recovery.conflicts_loaded = false;
        app.recovery.conflict_error = None;
    } else {
        app.recovery.conflict_status.clear();
        app.recovery.conflict_error = Some(message);
    }
}

fn start_restore(app: &mut App, entry: Entry) {
    let Some(root) = sync_root(app) else {
        app.toast_error("Choose a sync folder before restoring a file.");
        return;
    };
    let state_dir = app.paths.state_dir.clone();
    let tx = app.tx.clone();
    let name = entry.original_rel.clone();
    app.recovery.busy = true;
    app.recovery.error = None;
    std::thread::spawn(move || {
        let result = local_trash::restore(&state_dir, &root, &entry);
        let ok = result.is_ok();
        let message = match result {
            Ok(()) => format!("Restored {name}."),
            Err(error) => error,
        };
        let _ = tx.send(Back::RecoveryResult { ok, message });
    });
}

fn start_delete(app: &mut App, entry: Entry) {
    let state_dir = app.paths.state_dir.clone();
    let tx = app.tx.clone();
    let name = entry.original_rel.clone();
    app.recovery.busy = true;
    app.recovery.error = None;
    std::thread::spawn(move || {
        let result = local_trash::delete(&state_dir, &entry);
        let ok = result.is_ok();
        let message = match result {
            Ok(()) => format!("Permanently deleted {name}."),
            Err(error) => error,
        };
        let _ = tx.send(Back::RecoveryResult { ok, message });
    });
}

fn start_conflict_action(app: &mut App, row: &ConflictRow, choice: ConflictChoice) {
    let Some(root) = sync_root(app) else {
        app.toast_error("Choose a sync folder before resolving a conflict.");
        return;
    };
    let state_dir = app.paths.state_dir.clone();
    let tx = app.tx.clone();
    let conflict = row.conflict.clone();
    let name = conflict.canonical_rel.clone();
    let had_canonical = conflict.canonical_exists;
    app.recovery.conflict_busy = true;
    app.recovery.conflict_error = None;
    std::thread::spawn(move || {
        let result = match choice {
            ConflictChoice::KeepCurrent => {
                conflict::keep_current(&state_dir, &root, &conflict)
            }
            ConflictChoice::UseConflict => {
                conflict::use_conflict(&state_dir, &root, &conflict)
            }
        };
        let ok = result.is_ok();
        let message = match result {
            Ok(()) => match choice {
                ConflictChoice::KeepCurrent => {
                    format!("Kept the current {name}; the conflict copy is in local trash.")
                }
                ConflictChoice::UseConflict if had_canonical => {
                    format!("Using the conflict copy as {name}; the old file is in local trash.")
                }
                ConflictChoice::UseConflict => {
                    format!("Restored {name} from the conflict copy.")
                }
            },
            Err(error) => error,
        };
        let _ = tx.send(Back::ConflictResult { ok, message });
    });
}

fn start_keep_both(app: &mut App, row: &ConflictRow) {
    let state_dir = app.paths.state_dir.clone();
    let tx = app.tx.clone();
    let conflict = row.conflict.clone();
    app.recovery.conflict_busy = true;
    app.recovery.conflict_error = None;
    std::thread::spawn(move || {
        let result = conflict::ack(&state_dir, &conflict.conflict_rel, conflict.mtime_ns);
        let ok = result.is_ok();
        let message = match result {
            Ok(()) => "Both versions kept; the conflict is hidden until one changes.".to_string(),
            Err(error) => error,
        };
        let _ = tx.send(Back::ConflictResult { ok, message });
    });
}

fn sync_root(app: &App) -> Option<PathBuf> {
    app.conf
        .get("PROTON_SYNC_LOCAL")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
}

fn open_parent(app: &App, rel: &str) {
    if let Some(root) = sync_root(app) {
        let path = root.join(rel);
        let dir = path.parent().unwrap_or(&root);
        let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
    }
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

fn fs_time(mtime_ns: i64) -> String {
    if mtime_ns <= 0 {
        return "modified earlier".into();
    }
    let secs = mtime_ns / 1_000_000_000;
    let dt = chrono::DateTime::<chrono::Utc>::from_timestamp(secs, 0).map(|d| d.fixed_offset());
    format!("modified {}", crate::util::rel_time(dt))
}
