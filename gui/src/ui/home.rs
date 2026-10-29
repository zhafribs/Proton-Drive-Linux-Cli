//! Home: the status hero plus the activity feed (the old Activity tab).

use super::{activity, widgets, App, Dialog};
use crate::{health, theme, util};
use eframe::egui;

pub fn draw(app: &mut App, ui: &mut egui::Ui) {
    let status = app.ui_status();
    let (title, _) = health::status_text(status);
    let key = status.key();
    let logged_in = app.logged_in();
    let needs_login = status == health::UiStatus::NeedsLogin;
    let signed_out = status == health::UiStatus::SignedOut;
    let signing_in = status == health::UiStatus::SigningIn;

    // Soft accent glow behind the hero: layered translucent discs so the
    // status block reads as the page's focal point instead of floating on
    // flat colour. Painted first, under every widget.
    {
        let c = ui.cursor().min + egui::vec2(52.0, 52.0);
        let glow = theme::accent(ui.visuals());
        let cyan = theme::CYAN;
        for (r, a, col) in [
            (260.0, 0.05, glow),
            (360.0, 0.030, cyan),
            (460.0, 0.02, glow),
        ] {
            ui.painter()
                .circle_filled(c, r, col.gamma_multiply(a));
        }
    }

    // --- hero -------------------------------------------------------------
    ui.horizontal(|ui| {
        widgets::status_orb(ui, key);
        ui.add_space(26.0);
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(title).strong().size(27.0));
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
                let done = app.health.transfer_done.unwrap_or(0) as f32;
                let total = app.health.transfer_total.unwrap_or(0) as f32;
                ui.label(
                    egui::RichText::new(format!(
                        "Transferring {} / {} files...",
                        app.health.transfer_done.unwrap_or(0),
                        app.health.transfer_total.unwrap_or(0)
                    ))
                    .size(12.0)
                    .weak(),
                );
                ui.add(
                    egui::ProgressBar::new(done / total.max(1.0))
                        .desired_width(220.0)
                        .show_percentage(),
                );
                for q in app.health.queue.iter().take(8) {
                    let (glyph, rel) = match q.split_once(':') {
                        Some(("up", r)) => ("\u{2191}", r),
                        Some(("down", r)) => ("\u{2193}", r),
                        _ => ("\u{b7}", q.as_str()),
                    };
                    let name = rel.rsplit('/').next().unwrap_or(rel);
                    ui.label(
                        egui::RichText::new(format!("{glyph} {name}"))
                            .size(11.5)
                            .weak(),
                    );
                }
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

            // Under the subtitle, where it has always been. Compact padding
            // lets all three fit on one line even at the minimum width.
            if !signed_out && !signing_in && !needs_login {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    ui.spacing_mut().button_padding.x = 10.0;
                    if widgets::primary_button_hint(
                        ui,
                        "  Open local folder",
                        "Open your Proton Drive folder in the file manager.",
                    ) {
                        app.action_open_folder();
                    }
                    if app.active
                        && widgets::secondary_button_hint(
                            ui,
                            "  Sync now",
                            "Run a full check now.",
                        )
                    {
                        app.action_sync_now();
                    }
                    let pause_label = if app.active {
                        "Pause"
                    } else if app.paused_by_user {
                        "Resume"
                    } else {
                        "Start syncing"
                    };
                    let pause_hint = if app.active {
                        "Pause syncing: pick 15 minutes, 1 hour, or until you resume."
                    } else if app.paused_by_user {
                        "Resume syncing."
                    } else {
                        "Start the sync engine."
                    };
                    if widgets::secondary_button_hint(ui, pause_label, pause_hint) {
                        if app.active {
                            app.dialog = Some(Dialog::PauseMenu);
                        } else {
                            app.action_resume();
                        }
                    }
                });
            }
        });
    });

    ui.add_space(16.0);
    activity::draw(app, ui, "Recent activity");
}
