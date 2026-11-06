//! Home: the status hero, three honest metrics, a compact activity preview,
//! and the actions people actually use.

use super::{App, Dialog, activity, widgets};
use crate::{health, service, theme, util};
use eframe::egui;

pub fn draw(app: &mut App, ui: &mut egui::Ui) {
    egui::ScrollArea::vertical()
        .id_salt("home-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| draw_inner(app, ui));
}

fn draw_inner(app: &mut App, ui: &mut egui::Ui) {
    let status = app.ui_status();
    let (title, _) = health::status_text(status);
    let key = status.key();
    let logged_in = app.logged_in();
    let needs_login = status == health::UiStatus::NeedsLogin;
    let signed_out = status == health::UiStatus::SignedOut;
    let signing_in = status == health::UiStatus::SigningIn;

    page_head(
        app,
        ui,
        logged_in,
        needs_login,
        signed_out,
        signing_in,
    );

    let animate = app.prefs.motion_enabled();
    ui.add_space(14.0);
    widgets::card(ui, None, |ui| {
        ui.horizontal(|ui| {
            widgets::status_orb(ui, key, animate);
            ui.add_space(18.0);
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(title).strong().size(20.0));
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
                ui.label(egui::RichText::new(sub).size(12.5).weak());

                if (signed_out || needs_login) && !signing_in {
                    let local = app
                        .conf
                        .get("PROTON_SYNC_LOCAL")
                        .filter(|s| !s.is_empty())
                        .cloned()
                        .unwrap_or_else(|| "~/ProtonDrive".to_string());
                    ui.add_space(5.0);
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("Files will sync to")
                                .size(11.5)
                                .weak(),
                        );
                        ui.label(
                            egui::RichText::new(widgets::elide_middle(&local, 48))
                                .size(11.5)
                                .strong(),
                        )
                        .on_hover_text(&local);
                        let change = ui
                            .small_button("Change")
                            .on_hover_text("Opens Settings > General.");
                        change.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::Role::Button,
                                true,
                                "Change sync folder",
                            )
                        });
                        if change.clicked() {
                            app.page = super::Page::Settings;
                            app.settings_tab = 0;
                        }
                    });
                }

                if app.health.audit_pending == Some(true) {
                    let note = if app.health.audit_phase.as_deref() == Some("error") {
                        "Background check failed - deletes and existing-file edits stay held for safety."
                    } else {
                        "Deletes and edits to existing files are held until the background check finishes."
                    };
                    ui.label(egui::RichText::new(note).size(11.5).weak());
                }
                if status == health::UiStatus::DeletesHeld {                    if app.health.deletes_wipe_guard {
                        ui.label(
                            egui::RichText::new(
                                "Nothing will be deleted while one side looks empty. Restore the \
                                 folder or fix the sync path, then check again.",
                            )
                            .size(11.5)
                            .weak(),
                        );
                    } else if widgets::secondary_button_hint(
                        ui,
                        "Review and allow once",
                        "Review the files that would be removed, then approve this batch only.",
                    ) {
                        app.dialog = Some(Dialog::ConfirmHeldDeletes {
                            local: app.health.deletes_held_local,
                            cloud: app.health.deletes_held_cloud,
                            local_digest: app.health.deletes_held_local_digest.clone().unwrap_or_default(),
                            cloud_digest: app.health.deletes_held_cloud_digest.clone().unwrap_or_default(),
                            sample: app.health.deletes_held_sample.clone(),
                        });
                    }
                }
                if app.health.transfer_active == Some(true)
                    && app.health.transfer_total.unwrap_or(0) > 0
                {
                    let done = app.health.transfer_done.unwrap_or(0) as f32;
                    let total = app.health.transfer_total.unwrap_or(0) as f32;
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(format!(
                            "Transferring {} / {} files",
                            app.health.transfer_done.unwrap_or(0),
                            app.health.transfer_total.unwrap_or(0)
                        ))
                        .size(11.5)
                        .weak(),
                    );
                    ui.add(
                        egui::ProgressBar::new(done / total.max(1.0))
                            .desired_width(220.0)
                            .show_percentage(),
                    );
                    for q in app.health.queue.iter().take(5) {
                        let (glyph, rel) = match q.split_once(':') {
                            Some(("up", r)) => ("\u{2191}", r),
                            Some(("down", r)) => ("\u{2193}", r),
                            _ => ("\u{b7}", q.as_str()),
                        };
                        let name = rel.rsplit('/').next().unwrap_or(rel);
                        ui.label(
                            egui::RichText::new(format!("{glyph} {name}"))
                                .size(11.0)
                                .weak(),
                        );
                    }
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                status_pill(ui, key, health::short_label(key));
            });
        });
    });

    ui.add_space(12.0);
    if logged_in {
        metrics(app, ui);
        ui.add_space(12.0);
    }

    let full = ui.available_width();
    if full < 640.0 {
        activity::draw_preview(app, ui, "Recent activity", 4);
        ui.add_space(12.0);
        sync_details(app, ui);
    } else {
        ui.horizontal_top(|ui| {
            let left = (full * 0.60).max(340.0);
            let right = (full - left - 14.0).max(230.0);
            ui.allocate_ui_with_layout(
                egui::vec2(left, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| activity::draw_preview(app, ui, "Recent activity", 4),
            );
            ui.add_space(14.0);
            ui.allocate_ui_with_layout(
                egui::vec2(right, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| sync_details(app, ui),
            );
        });
    }
}

fn page_head(
    app: &mut App,
    ui: &mut egui::Ui,
    logged_in: bool,
    needs_login: bool,
    signed_out: bool,
    signing_in: bool,
) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Home").strong().size(24.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if signing_in {
                ui.label(egui::RichText::new("Signing in\u{2026}").weak());
            } else if signed_out || needs_login {
                let label = if needs_login { "  Sign in again" } else { "  Sign in" };
                if widgets::primary_button_hint(
                    ui,
                    label,
                    "Sign in with your Proton account to start syncing.",
                ) {
                    app.action_login();
                }
            } else if logged_in {
                if app.active
                    && widgets::primary_button_hint(
                        ui,
                        "  Sync now",
                        "Run a full check now.",
                    )
                {
                    app.action_sync_now();
                }
                if widgets::secondary_button_hint(
                    ui,
                    "  Open local folder",
                    "Open your Proton Drive folder in the file manager.",
                ) {
                    app.action_open_folder();
                }
                let pause_label = if app.active {
                    "  Pause"
                } else if app.paused_by_user {
                    "  Resume"
                } else {
                    "  Start syncing"
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
            }
        });
    });
}

fn metrics(app: &App, ui: &mut egui::Ui) {
    let files = app
        .stats
        .map(|(f, _)| util::thousands(f))
        .unwrap_or_else(|| "\u{2014}".to_string());
    let bytes = app
        .stats
        .map(|(_, b)| service::human_size(b))
        .unwrap_or_else(|| "\u{2014}".to_string());
    let last = app
        .health
        .last_audit_dt()
        .map(|dt| util::rel_time(Some(dt)))
        .unwrap_or_else(|| "Not yet".to_string());

    ui.columns(3, |cols| {
        metric(&mut cols[0], "Files", &files, "In your sync folder");
        metric(&mut cols[1], "On disk", &bytes, "Local copy");
        metric(&mut cols[2], "Last check", &last, "Background audit");
    });
}

fn metric(ui: &mut egui::Ui, label: &str, value: &str, sub: &str) {
    widgets::card(ui, None, |ui| {
        ui.label(egui::RichText::new(label).size(11.5).weak());
        ui.add_space(2.0);
        ui.label(egui::RichText::new(value).strong().size(23.0));
        ui.label(egui::RichText::new(sub).size(11.0).weak());
    });
}

fn sync_details(app: &mut App, ui: &mut egui::Ui) {
    let next = app
        .health
        .next_poll_dt()
        .map(next_check_label)
        .unwrap_or_else(|| "Not scheduled".to_string());
    let engine = app
        .health
        .build
        .clone()
        .unwrap_or_else(|| crate::GUI_VERSION.to_string());
    let remote = if app.draft.remote.is_empty() {
        "\u{2014}".to_string()
    } else {
        app.draft.remote.clone()
    };
    let interval = app
        .health
        .interval
        .filter(|s| *s > 0)
        .map(interval_label)
        .unwrap_or_else(|| "\u{2014}".to_string());

    widgets::card(ui, Some("Sync details"), |ui| {
        detail_row(ui, "Next check", &next);
        detail_row(ui, "App build", &engine);
        detail_row(ui, "Remote folder", &remote);
        detail_row(ui, "Check interval", &interval);
    });
}

fn detail_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(label).size(11.5).weak());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(
                egui::Label::new(egui::RichText::new(value).size(12.0).strong()).truncate(),
            );
        });
    });
    ui.add_space(7.0);
}

fn interval_label(secs: u64) -> String {
    if secs.is_multiple_of(3600) {
        format!("Every {} h", secs / 3600)
    } else if secs.is_multiple_of(60) {
        format!("Every {} min", secs / 60)
    } else {
        format!("Every {secs} s")
    }
}

fn next_check_label(dt: chrono::DateTime<chrono::FixedOffset>) -> String {
    let secs = (dt - chrono::Utc::now().fixed_offset()).num_seconds();
    next_check_text(secs)
}

fn next_check_text(secs: i64) -> String {
    if secs <= 0 {
        "Due now".to_string()
    } else if secs < 60 {
        format!("in {secs}s")
    } else if secs < 3600 {
        format!("in {}m", secs / 60)
    } else if secs < 86_400 {
        format!("in {}h {}m", secs / 3600, (secs % 3600) / 60)
    } else {
        format!("in {}d {}h", secs / 86_400, (secs % 86_400) / 3600)
    }
}

fn status_pill(ui: &mut egui::Ui, key: &str, label: &str) {
    let color = theme::status_color(key);
    let text = theme::status_text_color(key, ui.visuals());
    egui::Frame::new()
        .fill(color.gamma_multiply(0.10))
        .corner_radius(theme::pill())
        .inner_margin(egui::Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(7.0, 7.0), egui::Sense::hover());
                ui.painter().circle_filled(rect.center(), 3.5, color);
                ui.label(egui::RichText::new(label).size(11.5).color(text));
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_check_reads_as_a_countdown() {
        assert_eq!(next_check_text(-1), "Due now");
        assert_eq!(next_check_text(0), "Due now");
        assert_eq!(next_check_text(30), "in 30s");
        assert_eq!(next_check_text(300), "in 5m");
        assert_eq!(next_check_text(3900), "in 1h 5m");
        assert_eq!(next_check_text(90_000), "in 1d 1h");
    }
}
