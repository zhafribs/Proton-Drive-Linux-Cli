//! Home: hero status orb + actions, account card, folder card, recent files.

use super::{widgets, App, Dialog};
use crate::{health, util};
use eframe::egui;

pub fn draw(app: &mut App, ui: &mut egui::Ui) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        let status = health::map_status(app.active, &app.health.status);
        let (title, _) = health::status_text(status);
        let key = status.key();
        let logged_in = app.logged_in();
        // The engine reports a lost keyring session; that is not "not signed
        // in" but "signed in and now locked out" - it needs its own wording
        // and its own way out.
        let needs_login = status == health::UiStatus::NeedsLogin;

        // --- hero ---------------------------------------------------------
        ui.horizontal(|ui| {
            widgets::status_orb(ui, key);
            ui.add_space(26.0);
            ui.vertical(|ui| {
                let t = if logged_in || needs_login {
                    title
                } else {
                    "Welcome to Proton Drive"
                };
                ui.label(egui::RichText::new(t).strong().size(27.0));
                ui.add_space(4.0);

                let sub = if needs_login {
                    health::hero_sub(status, &app.health)
                } else if !logged_in {
                    "Sign in to start - your files will appear here.".to_string()
                } else if let Some((text, at)) = &app.sub_override {
                    if app.now() < *at {
                        text.clone()
                    } else {
                        health::hero_sub(status, &app.health)
                    }
                } else {
                    health::hero_sub(status, &app.health)
                };
                ui.label(egui::RichText::new(sub).size(14.0).weak());
                if app.health.audit_pending == Some(true) {
                    let note = if app.health.audit_phase.as_deref() == Some("error") {
                        "Background check failed - deletes and existing-file edits stay held for safety."
                    } else {
                        "Deletes and edits to existing files are held until the background check finishes."
                    };
                    ui.label(egui::RichText::new(note).size(12.0).weak());
                }
                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    if needs_login {
                        if widgets::primary_button(ui, "  Sign in again") {
                            app.action_login();
                        }
                    } else {
                        if widgets::primary_button(ui, "  Open my folder") {
                            app.action_open_folder();
                        }
                        if widgets::secondary_button(ui, "  Sync now") {
                            app.action_sync_now();
                        }
                    }
                    let pause_label = if app.active { "Pause" } else { "Resume" };
                    if widgets::secondary_button(ui, pause_label) {
                        if app.active {
                            app.dialog = Some(Dialog::PauseMenu);
                        } else {
                            app.action_resume();
                        }
                    }
                });
            });
        });

        ui.add_space(18.0);

        // --- account ------------------------------------------------------
        widgets::card(ui, None, |ui| {
            ui.horizontal(|ui| {
                let email = app.identity.as_ref().and_then(|i| i.email.clone());
                widgets::avatar(ui, email.as_deref());
                ui.add_space(14.0);
                ui.vertical(|ui| {
                    let (value, badge) = match (&app.identity, &email) {
                        (None, _) => ("Checking your account...", ""),
                        (Some(_), Some(e)) => (e.as_str(), "Signed in"),
                        (Some(_), None) => ("You're not signed in yet", "Not signed in"),
                    };
                    ui.label(egui::RichText::new(value).strong().size(15.0));
                    if !badge.is_empty() {
                        ui.label(egui::RichText::new(badge).size(12.0).weak());
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if logged_in {
                        if widgets::danger_ghost_button(ui, "Sign out") {
                            app.dialog = Some(Dialog::ConfirmLogout);
                        }
                    } else if widgets::secondary_button(ui, "Sign in") {
                        app.action_login();
                    }
                });
            });
        });

        ui.add_space(14.0);

        // --- folder -------------------------------------------------------
        let local = app
            .conf
            .get("PROTON_SYNC_LOCAL")
            .cloned()
            .unwrap_or_default();
        let folder_name = local
            .rsplit('/')
            .find(|s| !s.is_empty())
            .unwrap_or("ProtonDrive")
            .to_string();
        widgets::card(ui, Some("Your folder on this computer"), |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("\u{1f4c1}").size(26.0));
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new(folder_name).strong().size(15.0));
                    ui.label(egui::RichText::new(&local).size(12.0).weak());
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some((files, bytes)) = app.stats {
                        ui.label(
                            egui::RichText::new(format!(
                                "{} files \u{b7} {}",
                                files,
                                service::human_size(bytes)
                            ))
                            .weak(),
                        );
                    }
                });
            });
        });

        ui.add_space(14.0);

        // --- recent -------------------------------------------------------
        widgets::card(ui, Some("Recently synced"), |ui| {
            if app.changes.is_empty() {
                ui.label(egui::RichText::new("Files you change will show up here.").weak());
            } else {
                let rows: Vec<crate::journal::ChangeRow> = app
                    .changes
                    .iter()
                    .rev()
                    .take(6)
                    .cloned()
                    .collect();
                for row in &rows {
                    let when = util::rel_time(row.ts);
                    widgets::move_row(ui, row, "", &when);
                }
            }
        });
    });
}

use crate::service;
