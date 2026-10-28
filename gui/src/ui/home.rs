//! Home: hero status orb + actions, account card, folder card, recent files.

use super::{widgets, App, Dialog};
use crate::{health, util};
use eframe::egui;

/// What the account card shows. The identity probe can wait up to a minute
/// behind the engine's CLI lock, so never render a "checking" line: use the
/// cached email, or a neutral "Signed in"/"Account" state.
fn account_text(
    identity: Option<&crate::cli::Identity>,
    last_email: Option<&str>,
    status: health::UiStatus,
) -> (String, &'static str) {
    let id_email = identity.and_then(|i| i.email.as_deref());
    match (identity, id_email) {
        // Identity probe still in flight.
        (None, _) => {
            if let Some(email) = last_email {
                let badge = match status {
                    health::UiStatus::NeedsLogin => "Session expired",
                    health::UiStatus::SigningIn => "Signing in...",
                    _ => "Signed in",
                };
                (email.to_string(), badge)
            } else if status == health::UiStatus::SigningIn {
                ("Signing in...".to_string(), "")
            } else if matches!(
                status,
                health::UiStatus::Syncing | health::UiStatus::Synced
            ) {
                // The engine is already working; the address arrives with the
                // identity probe, but the account is clearly signed in.
                ("Signed in".to_string(), "")
            } else if matches!(
                status,
                health::UiStatus::SignedOut | health::UiStatus::NeedsLogin
            ) {
                ("You're not signed in yet".to_string(), "Not signed in")
            } else {
                // Starting / engine stopped / paused / error: neutral until
                // the probe answers.
                ("Account".to_string(), "")
            }
        }
        (Some(_), Some(email)) => (email.to_string(), "Signed in"),
        (Some(_), None) => {
            if let Some(email) = last_email {
                let badge = match status {
                    health::UiStatus::NeedsLogin => "Session expired",
                    health::UiStatus::SigningIn => "Signing in...",
                    _ => "Not signed in",
                };
                (email.to_string(), badge)
            } else if status == health::UiStatus::SigningIn {
                ("Signing in...".to_string(), "")
            } else {
                ("You're not signed in yet".to_string(), "Not signed in")
            }
        }
    }
}

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

                // The status text already covers signed-out, session-expired,
                // signing-in, engine-stopped and error states. Never show a
                // separate "Checking your account..." here: the identity probe
                // can wait up to a minute behind the engine's CLI lock.
                let sub = if logged_in
                    && let Some((text, at)) = &app.sub_override
                {
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

        // On a wide window, put the account and folder cards side by side
        // (TMOG's panels use the same horizontal packing); a single column
        // leaves half the page empty on a large screen.
        if ui.available_width() >= 640.0 {
            ui.columns(2, |cols| {
                account_card(app, &mut cols[0], status, logged_in);
                folder_card(app, &mut cols[1]);
            });
        } else {
            account_card(app, ui, status, logged_in);
            ui.add_space(14.0);
            folder_card(app, ui);
        }

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

fn account_card(
    app: &mut App,
    ui: &mut egui::Ui,
    status: health::UiStatus,
    logged_in: bool,
) {
    widgets::card(ui, None, |ui| {
        ui.horizontal(|ui| {
            let id_email = app.identity.as_ref().and_then(|i| i.email.as_deref());
            let email = id_email.or(app.last_email.as_deref());
            widgets::avatar(ui, email);
            ui.add_space(14.0);
            ui.vertical(|ui| {
                let (value, badge) =
                    account_text(app.identity.as_ref(), app.last_email.as_deref(), status);
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
}

fn folder_card(app: &mut App, ui: &mut egui::Ui) {
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
}

use crate::service;

#[cfg(test)]
mod tests {
    use super::account_text;
    use crate::cli::Identity;
    use crate::health::UiStatus;

    fn identity(email: Option<&str>) -> Identity {
        Identity {
            version: "0.9.0".into(),
            email: email.map(str::to_string),
            logged_in: email.is_some(),
            error: None,
        }
    }

    #[test]
    fn pending_probe_never_says_checking() {
        assert_eq!(
            account_text(None, None, UiStatus::Syncing),
            ("Signed in".into(), "")
        );
        assert_eq!(
            account_text(None, None, UiStatus::Starting),
            ("Account".into(), "")
        );
        assert_eq!(
            account_text(None, None, UiStatus::SigningIn),
            ("Signing in...".into(), "")
        );
    }

    #[test]
    fn cached_email_is_used_immediately() {
        assert_eq!(
            account_text(None, Some("a@b.c"), UiStatus::Syncing),
            ("a@b.c".into(), "Signed in")
        );
        assert_eq!(
            account_text(None, Some("a@b.c"), UiStatus::NeedsLogin),
            ("a@b.c".into(), "Session expired")
        );
    }

    #[test]
    fn signed_out_and_signed_in_states() {
        assert_eq!(
            account_text(None, None, UiStatus::SignedOut),
            ("You're not signed in yet".into(), "Not signed in")
        );
        let id = identity(Some("a@b.c"));
        assert_eq!(
            account_text(Some(&id), None, UiStatus::Synced),
            ("a@b.c".into(), "Signed in")
        );
    }
}
