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
use proton_core::state_snapshot::{self, SnapshotInfo};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ConflictChoice {
    KeepCurrent,
    UseConflict,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CheckResult {
    pub ts: u64,
    pub ok: bool,
    pub summary: String,
    pub details: Vec<String>,
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
    // State snapshots.
    pub snapshots: Vec<SnapshotInfo>,
    pub snapshots_loaded: bool,
    pub snapshots_loading: bool,
    pub snapshot_busy: bool,
    pub snapshot_error: Option<String>,
    pub snapshot_status: String,
    pub confirm_snapshot: Option<PathBuf>,
    // Last read-only recovery check.
    pub last_check: Option<CheckResult>,
    pub check_loaded: bool,
    pub check_busy: bool,
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
    if !app.recovery.snapshots_loaded
        && !app.recovery.snapshots_loading
        && app.recovery.snapshot_error.is_none()
    {
        refresh_snapshots(app);
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
                        || app.recovery.conflicts_loading
                        || app.recovery.snapshot_busy
                        || app.recovery.snapshots_loading;
                    if ui
                        .add_enabled(!busy, egui::Button::new("Refresh"))
                        .on_hover_text("Re-read conflict copies and the local trash")
                        .clicked()
                    {
                        refresh_trash(app);
                        refresh_conflicts(app);
                        refresh_snapshots(app);
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
            draw_snapshots(app, ui);
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

fn draw_snapshots(app: &mut App, ui: &mut egui::Ui) {
    widgets::section_label(ui, "State snapshots");
    ui.add_space(3.0);
    ui.label(
        egui::RichText::new(
            "A snapshot preserves the last-synced history, not your files. One is taken automatically before a root change, an approved mass delete, or an update.",
        )
        .size(12.0)
        .weak(),
    );
    ui.add_space(6.0);

    if !app.recovery.snapshot_status.is_empty() {
        ui.label(
            egui::RichText::new(&app.recovery.snapshot_status)
                .size(12.5)
                .color(crate::theme::success_text(ui.visuals())),
        );
        ui.add_space(6.0);
    }

    if let Some(error) = app.recovery.snapshot_error.clone() {
        let load_failed = app.recovery.snapshots.is_empty();
        widgets::card(
            ui,
            Some(if load_failed {
                "Snapshots could not be read"
            } else {
                "Snapshot action failed"
            }),
            |ui| {
                ui.label(egui::RichText::new(error).size(12.5));
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .button(if load_failed { "Try again" } else { "Dismiss" })
                        .clicked()
                    {
                        app.recovery.snapshot_error = None;
                        app.recovery.snapshots_loaded = false;
                        if load_failed {
                            refresh_snapshots(app);
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

    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(
                !app.recovery.snapshot_busy,
                egui::Button::new("Create snapshot"),
            )
            .on_hover_text("Save the current sync history as a restore point.")
            .clicked()
        {
            start_snapshot_create(app);
        }
        if ui.button("Open snapshot folder").clicked() {
            open_snapshot_folder(app);
        }
        if ui
            .add_enabled(
                !app.recovery.check_busy,
                egui::Button::new("Run recovery check"),
            )
            .on_hover_text(
                "Read the sync history, snapshots, local trash and conflict copies; write the result to the state folder.",
            )
            .clicked()
        {
            start_check(app);
        }
        let count = app.recovery.snapshots.len();
        ui.label(
            egui::RichText::new(if count == 1 {
                "1 snapshot".to_string()
            } else {
                format!("{count} snapshots")
            })
            .size(12.0)
            .weak(),
        );
    });
    ui.add_space(6.0);

    if app.recovery.snapshots_loading {
        ui.horizontal(|ui| {
            widgets::busy_spinner(ui, app.prefs.motion_enabled());
            ui.label(egui::RichText::new("Loading snapshots\u{2026}").weak());
        });
        return;
    }

    let snapshots = app.recovery.snapshots.clone();
    if snapshots.is_empty() {
        widgets::card(ui, None, |ui| {
            ui.label(
                egui::RichText::new(
                    "No snapshots yet. Create one before a risky change, or wait for the automatic snapshot before a root change, mass delete or update.",
                )
                .size(12.5)
                .weak(),
            );
        });
        return;
    }

    for snapshot in &snapshots {
        snapshot_card(app, ui, snapshot);
        ui.add_space(8.0);
    }
}

fn snapshot_card(app: &mut App, ui: &mut egui::Ui, snap: &SnapshotInfo) {
    egui::Frame::new()
        .fill(crate::theme::glass_fill(ui.visuals()))
        .stroke(crate::theme::glass_stroke(ui.visuals()))
        .corner_radius(egui::CornerRadius::same(12))
        .inner_margin(egui::Margin::symmetric(14, 11))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let when = chrono::DateTime::<chrono::Utc>::from_timestamp(snap.created_at as i64, 0)
                .map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string())
                .unwrap_or_else(|| "unknown time".into());
            let reason = snap.reason.replace('-', " ");
            ui.add(
                egui::Label::new(egui::RichText::new(when).strong().size(13.5)).truncate(),
            );
            let size = super::cloud::fmt_size(snap.bytes);
            let mut meta = format!(
                "{} entries \u{b7} {}{} \u{b7} {}",
                snap.entries,
                if size.is_empty() { "0 B".to_string() } else { size },
                if snap.valid { "" } else { " \u{b7} checksum mismatch" },
                reason
            );
            if !snap.valid || !snap.roots_match {
                meta.push_str(" \u{b7} not restorable");
            }
            ui.add(egui::Label::new(egui::RichText::new(meta).size(11.5).weak()).truncate());
            if !snap.roots_match {
                ui.label(
                    egui::RichText::new(
                        "This history belongs to different sync roots. Switch back to those roots before restoring it.",
                    )
                    .size(11.5)
                    .color(crate::theme::warning_text(ui.visuals())),
                );
            }
            ui.add_space(7.0);

            if app.recovery.confirm_snapshot.as_deref() == Some(snap.path.as_path()) {
                ui.label(
                    egui::RichText::new(
                        "Restore this snapshot? The current state is snapshotted first, then the engine re-plans before any transfer.",
                    )
                    .size(12.0),
                );
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            !app.recovery.snapshot_busy,
                            egui::Button::new("Restore this snapshot"),
                        )
                        .clicked()
                    {
                        app.recovery.confirm_snapshot = None;
                        start_snapshot_restore(app, snap.clone());
                    }
                    if ui.button("Cancel").clicked() {
                        app.recovery.confirm_snapshot = None;
                    }
                });
                return;
            }

            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(
                        snap.valid && snap.roots_match && !app.recovery.snapshot_busy,
                        egui::Button::new("Restore"),
                    )
                    .on_hover_text(
                        "Put this sync history back, then let the engine plan against the current files.",
                    )
                    .clicked()
                {
                    app.recovery.confirm_snapshot = Some(snap.path.clone());
                }
                if ui
                    .add_enabled(
                        !app.recovery.snapshot_busy,
                        egui::Button::new("Delete"),
                    )
                    .on_hover_text("Delete this restore point. Your files are not touched.")
                    .clicked()
                {
                    start_snapshot_delete(app, snap.path.clone());
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

fn check_path(state_dir: &std::path::Path) -> PathBuf {
    state_dir.join("recovery-drill.json")
}

pub fn load_check(state_dir: &std::path::Path) -> Option<CheckResult> {
    let bytes = std::fs::read(check_path(state_dir)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn run_check(state_dir: &std::path::Path, root: Option<&std::path::Path>) -> CheckResult {
    let mut details = Vec::new();
    let mut problems = 0u32;
    let state_file = state_dir.join("state.json");
    match proton_core::state::read_checked(&state_file) {
        Ok(Some(_)) => details.push("sync history: readable".into()),
        Ok(None) => details.push("sync history: not created yet".into()),
        Err(e) => {
            problems += 1;
            details.push(format!("sync history: unreadable ({e})"));
        }
    }
    let roots = std::fs::read_to_string(state_dir.join("sync-roots")).unwrap_or_default();
    if state_file.exists() && roots.is_empty() {
        problems += 1;
        details.push("sync roots: not recorded".into());
    } else if !roots.is_empty() {
        details.push("sync roots: recorded".into());
    }
    let snapshots = state_snapshot::list(state_dir);
    let usable = snapshots
        .iter()
        .filter(|s| s.valid && s.roots_match)
        .count();
    if state_file.exists() && snapshots.is_empty() {
        details.push("snapshots: none yet".into());
    } else {
        details.push(format!(
            "snapshots: {} total, {usable} restorable",
            snapshots.len()
        ));
    }
    match local_trash::entries(state_dir) {
        Ok(entries) => details.push(format!("local trash: {} entries", entries.len())),
        Err(e) => {
            problems += 1;
            details.push(format!("local trash: unreadable ({e})"));
        }
    }
    if let Some(root) = root {
        match conflict::scan(root) {
            Ok(conflicts) => {
                details.push(format!("conflict copies: {} found", conflicts.len()))
            }
            Err(e) => {
                problems += 1;
                details.push(format!("conflict scan: failed ({e})"));
            }
        }
    } else {
        details.push("conflict scan: no sync folder".into());
    }
    let ok = problems == 0;
    CheckResult {
        ts: local_trash::now_secs(),
        ok,
        summary: if ok {
            "All recovery checks passed.".into()
        } else {
            format!("{problems} recovery check(s) need attention.")
        },
        details,
    }
}

pub(crate) fn start_check(app: &mut App) {
    let state_dir = app.paths.state_dir.clone();
    let root = sync_root(app);
    let tx = app.tx.clone();
    app.recovery.check_busy = true;
    std::thread::spawn(move || {
        let result = run_check(&state_dir, root.as_deref());
        let path = check_path(&state_dir);
        let save = serde_json::to_vec_pretty(&result)
            .map_err(|e| e.to_string())
            .and_then(|bytes| std::fs::write(&path, bytes).map_err(|e| e.to_string()));
        match save {
            Ok(()) => {
                let _ = tx.send(Back::RecoveryCheckDone(Ok(result)));
            }
            Err(e) => {
                let _ = tx.send(Back::RecoveryCheckDone(Err(format!(
                    "recovery check ran but could not be saved: {e}"
                ))));
            }
        }
    });
}

pub fn on_check_done(app: &mut App, result: Result<CheckResult, String>) {
    app.recovery.check_busy = false;
    app.recovery.check_loaded = true;
    match result {
        Ok(result) => {
            let summary = result.summary.clone();
            app.recovery.last_check = Some(result);
            app.toast(summary);
        }
        Err(e) => app.toast_error(e),
    }
}

fn refresh_snapshots(app: &mut App) {
    app.recovery.snapshots_loading = true;
    app.recovery.snapshot_error = None;
    let state_dir = app.paths.state_dir.clone();
    let tx = app.tx.clone();
    std::thread::spawn(move || {
        let result = Ok(state_snapshot::list(&state_dir));
        let _ = tx.send(Back::SnapshotsLoaded(result));
    });
}

fn start_snapshot_create(app: &mut App) {
    let state_dir = app.paths.state_dir.clone();
    let tx = app.tx.clone();
    app.recovery.snapshot_busy = true;
    app.recovery.snapshot_error = None;
    std::thread::spawn(move || {
        let result = state_snapshot::create(&state_dir, "manual");
        let (ok, message) = match result {
            Ok(Some(snapshot)) => (
                true,
                format!("Snapshot created with {} entries.", snapshot.entries),
            ),
            Ok(None) => (true, "There is no sync history to snapshot yet.".to_string()),
            Err(error) => (false, error),
        };
        let _ = tx.send(Back::SnapshotResult { ok, message });
    });
}

fn start_snapshot_restore(app: &mut App, snap: SnapshotInfo) {
    let state_dir = app.paths.state_dir.clone();
    let tx = app.tx.clone();
    let wake = app.paths.wake_file.clone();
    app.recovery.snapshot_busy = true;
    app.recovery.snapshot_error = None;
    std::thread::spawn(move || {
        let result = state_snapshot::restore(&state_dir, &snap);
        let (ok, message) = match result {
            Ok(()) => {
                let _ = std::fs::write(&wake, b"");
                (true, "State restored. The engine will re-plan against the current files.".to_string())
            }
            Err(error) => (false, error),
        };
        let _ = tx.send(Back::SnapshotResult { ok, message });
    });
}

fn start_snapshot_delete(app: &mut App, path: PathBuf) {
    let state_dir = app.paths.state_dir.clone();
    let tx = app.tx.clone();
    app.recovery.snapshot_busy = true;
    app.recovery.snapshot_error = None;
    std::thread::spawn(move || {
        let result = state_snapshot::remove(&state_dir, &path);
        let (ok, message) = match result {
            Ok(()) => (true, "Snapshot deleted.".to_string()),
            Err(error) => (false, error),
        };
        let _ = tx.send(Back::SnapshotResult { ok, message });
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

pub fn on_snapshots_loaded(
    app: &mut App,
    result: Result<Vec<SnapshotInfo>, String>,
) {
    app.recovery.snapshots_loading = false;
    app.recovery.snapshots_loaded = true;
    match result {
        Ok(snapshots) => {
            app.recovery.snapshots = snapshots;
            app.recovery.snapshot_error = None;
        }
        Err(error) => {
            app.recovery.snapshots.clear();
            app.recovery.snapshot_error = Some(error);
        }
    }
}

pub fn on_snapshot_result(app: &mut App, ok: bool, message: String) {
    app.recovery.snapshot_busy = false;
    if ok {
        app.recovery.snapshot_status = message;
        app.recovery.snapshots_loaded = false;
        app.recovery.snapshot_error = None;
    } else {
        app.recovery.snapshot_status.clear();
        app.recovery.snapshot_error = Some(message);
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

fn open_snapshot_folder(app: &App) {
    let path = app.paths.state_dir.join("snapshots");
    if std::fs::create_dir_all(&path).is_ok() {
        let _ = std::process::Command::new("xdg-open").arg(&path).spawn();
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
