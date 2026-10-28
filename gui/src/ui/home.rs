//! Home: hero status orb + actions, account card, folder card, recent files.

use super::{widgets, App, Dialog};
use crate::{health, util};
use eframe::egui;

pub fn draw(app: &mut App, ui: &mut egui::Ui) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        let status = app.ui_status();
        let (title, _) = health::status_text(status);
        let key = status.key();
        let logged_in = app.logged_in();
        // The engine reports a lost keyring session; that is not "not signed
        // in" but "signed in and now locked out" - it needs its own wording
        // and its own way out.
        let needs_login = status == health::UiStatus::NeedsLogin;
        let signed_out = status == health::UiStatus::SignedOut;
        let signing_in = status == health::UiStatus::SigningIn;

        // --- hero ---------------------------------------------------------
        ui.horizontal(|ui| {
            widgets::status_orb(ui, key);
            ui.add_space(26.0);
            ui.vertical(|ui| {
                let t = title;
                ui.label(egui::RichText::new(t).strong().size(27.0));
                ui.add_space(4.0);

                let sub = if needs_login {
                    health::hero_sub(status, &app.health)
                } else if signing_in {
                    "Finish signing in in the terminal window.".to_string()
                } else if matches!(
                    status,
                    health::UiStatus::Syncing
                        | health::UiStatus::Synced
                        | health::UiStatus::Error
                        | health::UiStatus::Unknown
                ) {
                    // The engine's real state is more useful than the identity
                    // probe, which may still be waiting on the CLI lock.
                    health::hero_sub(status, &app.health)
                } else if app.identity.is_none() {
                    "Checking your account...".to_string()
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
                if app.health.transfer_active == Some(true)
                    && app.health.transfer_total.unwrap_or(0) > 0
                {
                    ui.label(
                        egui::RichText::new(format!(
                            "Transferring {} / {} files...",
                            app.health.transfer_done.unwrap_or(0),
                            app.health.transfer_total.unwrap_or(0)
                        ))
                        .size(12.0)
                        .weak(),
                    );
                }
                if let Some(dt) = app.health.last_audit_dt() {
                    ui.label(
                        egui::RichText::new(format!(
                            "Last full check {}",
                            util::rel_time(Some(dt))
                        ))
                        .size(12.0)
                        .weak(),
                    );
                }
                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    if needs_login {
                        if widgets::primary_button(ui, "  Sign in again") {
                            app.action_login();
                        }
                    } else if signed_out {
                        if widgets::primary_button(ui, "  Sign in") {
                            app.action_login();
                        }
                    } else if signing_in {
                        ui.label(
                            egui::RichText::new("Waiting for the terminal sign-in...")
                                .size(12.0)
                                .weak(),
                        );
                    } else {
                        if widgets::primary_button(ui, "  Open my folder") {
                            app.action_open_folder();
                        }
                        if app.active && widgets::secondary_button(ui, "  Sync now") {
                            app.action_sync_now();
                        }
                    }
                    // Pause/resume only makes sense with an account. When the
                    // engine simply is not running, the same button offers to
                    // start it instead.
                    if !signed_out && !signing_in && !needs_login {
                        let pause_label = if app.active {
                            "Pause"
                        } else if app.paused_by_user {
                            "Resume"
                        } else {
                            "Start syncing"
                        };
                        if widgets::secondary_button(ui, pause_label) {
                            if app.active {
                                app.dialog = Some(Dialog::PauseMenu);
                            } else {
                                app.action_resume();
                            }
                        }
                    }
                });
            });
        });

        ui.add_space(18.0);

        // --- account ------------------------------------------------------
        widgets::card(ui, None, |ui| {
            ui.horizontal(|ui| {
                let id_email = app.identity.as_ref().and_then(|i| i.email.clone());
                let email = id_email.clone().or_else(|| app.last_email.clone());
                widgets::avatar(ui, email.as_deref());
                ui.add_space(14.0);
                ui.vertical(|ui| {
                    let (value, badge) = match (&app.identity, &id_email) {
                        (None, _) => (
                            app.last_email.as_deref().unwrap_or("Checking your account..."),
                            if app.last_email.is_some() { "Checking..." } else { "" },
                        ),
                        (Some(_), Some(e)) => (e.as_str(), "Signed in"),
                        (Some(_), None) => (
                            app.last_email.as_deref().unwrap_or("You're not signed in yet"),
                            if status == health::UiStatus::NeedsLogin {
                                "Session expired"
                            } else {
                                "Not signed in"
                            },
                        ),
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
