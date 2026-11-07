//! About: product identity, versions, updates, details, credits and legal.

use super::{widgets, App};
use crate::paths::{RELEASE_PAGE, REPO_URL};
use crate::{GUI_VERSION, theme, util};
use eframe::egui;
use std::path::Path;

pub fn draw(app: &mut App, ui: &mut egui::Ui) {
    egui::ScrollArea::vertical()
        .id_salt("about-scroll")
        .show(ui, |ui| {
            page_head(app, ui);
            ui.add_space(14.0);
            hero(app, ui);
            ui.add_space(12.0);
            versions(app, ui);
            ui.add_space(12.0);

            if ui.available_width() >= 620.0 {
                let full = ui.available_width();
                let left = (full - 14.0) * 0.5;
                // Equal card bodies when they sit side by side, even though
                // the two lists have different natural heights. Measure the
                // wrapped text first, then give both cards the larger body.
                let min_body = Some(equal_body_height(ui, left));
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(left, 0.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| what_it_does(ui, min_body),
                    );
                    ui.add_space(14.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), 0.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| good_to_know(ui, min_body),
                    );
                });
            } else {
                what_it_does(ui, None);
                ui.add_space(12.0);
                good_to_know(ui, None);
            }

            ui.add_space(12.0);
            updates(app, ui);
            ui.add_space(12.0);
            details(app, ui);
            ui.add_space(12.0);
            credits(ui);
            ui.add_space(12.0);
            legal(ui);
        });
}

fn page_head(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        let title = ui.label(egui::RichText::new("About").strong().size(24.0));
        title.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Heading, true, "About")
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(270.0, 30.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    if widgets::secondary_button_hint(
                        ui,
                        "Keyboard shortcuts",
                        "F1 opens this list from any page.",
                    ) {
                        app.show_shortcuts();
                    }
                    if widgets::secondary_button_hint(
                        ui,
                        "GitHub repository",
                        "Open the project on GitHub.",
                    ) {
                        App::open_url(REPO_URL);
                    }
                },
            );
        });
    });
}

fn hero(app: &App, ui: &mut egui::Ui) {
    widgets::card(ui, None, |ui| {
        ui.horizontal_top(|ui| {
            if let Some(tex) = &app.icon_tex {
                ui.add(egui::Image::new((tex.id(), egui::vec2(72.0, 72.0))));
            } else {
                ui.allocate_exact_size(egui::vec2(72.0, 72.0), egui::Sense::hover());
            }
            ui.add_space(20.0);
            let text_w = (ui.available_width() - 150.0).max(150.0);
            ui.allocate_ui_with_layout(
                egui::vec2(text_w, 0.0),
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| {
                    ui.set_min_width(text_w);
                    ui.label(
                        egui::RichText::new("Proton Drive for Linux")
                            .strong()
                            .size(24.0),
                    );
                    ui.label(
                        egui::RichText::new(
                            "Community edition \u{b7} an unofficial desktop client",
                        )
                        .size(12.5)
                        .weak(),
                    );
                    ui.add_space(6.0);
                    tags(ui);
                },
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                update_pill(app, ui);
            });
        });
        ui.add_space(12.0);
        ui.separator();
        ui.add_space(10.0);
        ui.add(
            egui::Label::new(
                egui::RichText::new(
                    "A native Linux home for your Proton Drive: two-way folder sync, a \
                     cloud file manager and a photo timeline, built on Proton's official \
                     command-line tool. Encryption happens on your device.",
                )
                .size(12.5)
                .weak(),
            )
            .wrap(),
        );
    });
}

fn tags(ui: &mut egui::Ui) {
    ui.horizontal_wrapped(|ui| {
        for text in ["Rust", "egui", "AppImage", "MIT"] {
            egui::Frame::new()
                .fill(theme::accent_soft(ui.visuals()))
                .corner_radius(theme::pill())
                .inner_margin(egui::Margin::symmetric(8, 3))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(text)
                            .size(10.5)
                            .color(theme::accent_text(ui.visuals())),
                    );
                });
            ui.add_space(4.0);
        }
    });
}

fn update_state(app: &App) -> (&'static str, &'static str) {
    update_state_for(
        app.upd.checking,
        app.upd.available.is_some(),
        &app.upd.label,
    )
}

fn update_state_for(
    checking: bool,
    has_update: bool,
    label: &str,
) -> (&'static str, &'static str) {
    let lower = label.to_ascii_lowercase();
    let failed = lower.contains("failed")
        || lower.contains("could not")
        || lower.contains("error")
        || lower.contains("rate limit");
    if checking {
        ("starting", "Checking")
    } else if failed {
        ("error", "Update failed")
    } else if has_update {
        ("paused", "Update available")
    } else if !label.is_empty() {
        ("synced", "Up to date")
    } else {
        ("engine_stopped", "Not checked yet")
    }
}

fn update_pill(app: &App, ui: &mut egui::Ui) {
    let (key, label) = update_state(app);
    widgets::status_pill(ui, key, label);
}

fn cli_version(app: &App) -> String {
    cli_version_from(
        app.identity
            .as_ref()
            .map(|i| i.version.as_str())
            .unwrap_or(""),
    )
}

/// The CLI reports names such as `cli-drive@0.9.0` or `proton-drive 0.9.0`.
/// Show only the version, without the package prefix or build suffix.
fn cli_version_from(raw: &str) -> String {
    let token = raw
        .split_whitespace()
        .rev()
        .find_map(|part| {
            let candidate = part.rsplit_once('@').map(|(_, v)| v).unwrap_or(part);
            let candidate = candidate.trim_matches(|c: char| !c.is_ascii_digit() && c != '.');
            (candidate.starts_with(|c: char| c.is_ascii_digit())
                && candidate.contains('.'))
            .then_some(candidate)
        })
        .unwrap_or("");
    let version = util::strip_build_suffix(token);
    if version.is_empty() || version == "?" {
        "\u{2014}".to_string()
    } else {
        version
    }
}

fn versions(app: &App, ui: &mut egui::Ui) {
    let cli = cli_version(app);
    ui.columns(3, |cols| {
        widgets::metric(&mut cols[0], "App version", GUI_VERSION, "GUI build");
        widgets::metric(&mut cols[1], "Proton CLI", &cli, "Official CLI");
        widgets::metric(
            &mut cols[2],
            "Platform",
            std::env::consts::ARCH,
            std::env::consts::OS,
        );
    });
}

const FEATURES: [(&str, &str); 3] = [
    (
        "Two-way folder sync",
        "Keeps a local folder equal to Proton Drive in both directions.",
    ),
    (
        "Cloud file manager",
        "Upload, download, rename, move and search from one list.",
    ),
    (
        "Photos timeline",
        "Review the timeline and keep local copies in step.",
    ),
];

const GOOD_TO_KNOW: [&str; 6] = [
    "Deletes go to trash by default; permanent delete is an explicit action.",
    "Conflicts keep both versions, so nothing is lost.",
    "Proton Docs and Sheets stay online only.",
    "Sign-in persists until you sign out.",
    "Your password is handled by Proton's official CLI, not this app.",
    "Updates are signed and verified before they run.",
];

/// The taller of the two card bodies, measured with the same fonts and wrap
/// width the cards will use. A fixed minimum would be a guess; this grows the
/// shorter card to match the longer one.
fn equal_body_height(ui: &egui::Ui, column_w: f32) -> f32 {
    let inner = (column_w - 40.0).max(80.0);
    let feature_w = (inner - 42.0).max(40.0);
    let bullet_w = (inner - 14.0).max(40.0);
    let color = ui.visuals().text_color();
    let line_gap = ui.spacing().item_spacing.y;
    let feature_h: f32 = FEATURES
        .iter()
        .map(|(title, body)| {
            let title_h = ui
                .painter()
                .layout_no_wrap(
                    title.to_string(),
                    egui::FontId::proportional(13.5),
                    color,
                )
                .size()
                .y;
            let body_h = ui
                .painter()
                .layout(
                    body.to_string(),
                    egui::FontId::proportional(12.0),
                    color,
                    feature_w,
                )
                .size()
                .y;
            32.0f32.max(title_h + line_gap + body_h) + 10.0
        })
        .sum();
    let bullet_h: f32 = GOOD_TO_KNOW
        .iter()
        .map(|text| {
            let text_h = ui
                .painter()
                .layout(
                    text.to_string(),
                    egui::FontId::proportional(12.0),
                    color,
                    bullet_w,
                )
                .size()
                .y;
            6.0f32.max(text_h) + 6.0
        })
        .sum();
    feature_h.max(bullet_h) + 24.0
}

fn what_it_does(ui: &mut egui::Ui, min_body: Option<f32>) {
    widgets::card(ui, Some("What it does"), |ui| {
        if let Some(h) = min_body {
            ui.set_min_height(h);
        }
        for (icon, (title, body)) in [
            widgets::NavIcon::Home,
            widgets::NavIcon::Folder,
            widgets::NavIcon::Photos,
        ]
        .into_iter()
        .zip(FEATURES)
        {
            feature(ui, icon, title, body);
        }
    });
}

fn feature(ui: &mut egui::Ui, icon: widgets::NavIcon, title: &str, body: &str) {
    ui.horizontal_top(|ui| {
        let accent = theme::accent(ui.visuals());
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(32.0, 32.0), egui::Sense::hover());
        ui.painter()
            .circle_filled(rect.center(), 16.0, accent.gamma_multiply(0.10));
        widgets::nav_icon(
            ui.painter(),
            egui::Rect::from_center_size(rect.center(), egui::vec2(17.0, 17.0)),
            icon,
            accent,
        );
        ui.add_space(10.0);
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(title).strong().size(13.5));
            ui.add(egui::Label::new(egui::RichText::new(body).size(12.0).weak()).wrap());
        });
    });
    ui.add_space(10.0);
}

fn good_to_know(ui: &mut egui::Ui, min_body: Option<f32>) {
    widgets::card(ui, Some("Good to know"), |ui| {
        if let Some(h) = min_body {
            ui.set_min_height(h);
        }
        for text in GOOD_TO_KNOW {
            bullet(ui, text);
        }
    });
}

fn bullet(ui: &mut egui::Ui, text: &str) {
    ui.horizontal_top(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(6.0, 6.0), egui::Sense::hover());
        ui.painter()
            .circle_filled(rect.center(), 3.0, theme::accent(ui.visuals()));
        ui.add_space(8.0);
        ui.add(egui::Label::new(egui::RichText::new(text).size(12.0).weak()).wrap());
    });
    ui.add_space(6.0);
}

fn updates(app: &mut App, ui: &mut egui::Ui) {
    widgets::card(ui, Some("Updates"), |ui| {
        let release = app.upd.available.clone();
        let checking = app.upd.checking;
        let downloading = app.upd.downloading;
        let status = app.upd.label.clone();
        let has_update = release.is_some() && app.paths.appimage.is_some();

        if !status.is_empty() {
            let short = status.split(" (").next().unwrap_or(&status).to_string();
            let resp = ui.label(egui::RichText::new(short).size(12.0).weak());
            resp.on_hover_text(&status);
        } else {
            ui.label(egui::RichText::new("No check has run yet.").size(12.0).weak());
        }
        if let Some(rel) = &release {
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(format!("GUI {} is available.", rel.gui))
                    .size(11.5)
                    .weak(),
            );
        }
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(!checking, egui::Button::new("Check now"))
                .clicked()
            {
                app.action_check_updates();
            }
            if has_update
                && ui
                    .add_enabled(
                        !downloading,
                        egui::Button::new("Download and restart"),
                    )
                    .clicked()
            {
                app.action_download_update();
            }
            if ui.button("Open download page").clicked() {
                App::open_url(RELEASE_PAGE);
            }
            if ui.button("Update settings").clicked() {
                app.page = super::Page::Settings;
                app.settings_tab = 5;
            }
        });
    });
}

fn details(app: &mut App, ui: &mut egui::Ui) {
    widgets::card(ui, None, |ui| {
        egui::CollapsingHeader::new(
            egui::RichText::new("Technical details").strong().size(13.5),
        )
        .id_salt("about-details")
        .show(ui, |ui| {
            let cli = cli_version(app);
            let engine = app
                .health
                .build
                .clone()
                .unwrap_or_else(|| GUI_VERSION.to_string());
            let install = if app.paths.appimage.is_some() {
                "AppImage"
            } else {
                "Development build"
            };
            let conf = short_home(&app.paths.conf_file);
            let conf_full = app.paths.conf_file.display().to_string();
            let state = short_home(&app.paths.state_dir);
            let state_full = app.paths.state_dir.display().to_string();

            detail_row(ui, "GUI", GUI_VERSION, None);
            detail_row(ui, "Proton CLI", &cli, None);
            detail_row(ui, "Engine", &engine, None);
            detail_row(ui, "Install", install, None);
            detail_row(ui, "Configuration", &conf, Some(&conf_full));
            detail_row(ui, "State", &state, Some(&state_full));
            ui.add_space(8.0);
            if ui.button("Copy diagnostics").clicked() {
                let text = format!(
                    "Proton Drive for Linux {GUI_VERSION}\n\
                     Proton CLI {cli}\n\
                     Engine {engine}\n\
                     Platform {} {}\n\
                     Install {install}\n\
                     Config {conf_full}\n\
                     State {state_full}",
                    std::env::consts::OS,
                    std::env::consts::ARCH,
                );
                ui.ctx().copy_text(text);
                app.toast("Diagnostics copied.");
            }
        });
    });
}

fn detail_row(ui: &mut egui::Ui, label: &str, value: &str, full: Option<&str>) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(label).size(11.5).weak());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let shown = widgets::elide_middle(value, 56);
            let response = ui.label(egui::RichText::new(shown).size(11.5).monospace());
            if let Some(full) = full {
                response.on_hover_text(full);
            }
        });
    });
    ui.add_space(4.0);
}

fn short_home(path: &Path) -> String {
    if let Some(home) = dirs::home_dir()
        && let Ok(rest) = path.strip_prefix(&home)
    {
        let rest = rest.display().to_string();
        if rest.is_empty() {
            return "~".to_string();
        }
        return format!("~/{rest}");
    }
    path.display().to_string()
}

fn credits(ui: &mut egui::Ui) {
    widgets::card(ui, Some("Credits"), |ui| {
        ui.add(
            egui::Label::new(
                egui::RichText::new(
                    "Built by the community on top of Proton's official command-line tool.",
                )
                .size(12.0)
                .weak(),
            )
            .wrap(),
        );
        ui.add_space(8.0);
        let licence = format!("{REPO_URL}/blob/main/LICENSE");
        let notices = format!("{REPO_URL}/blob/main/THIRD_PARTY_NOTICES.md");
        ui.horizontal_wrapped(|ui| {
            link(ui, "Project repository", REPO_URL);
            link(ui, "Licence (MIT)", &licence);
            link(ui, "Third-party notices", &notices);
            link(ui, "Proton Drive CLI", "https://github.com/ProtonDriveApps/sdk");
        });
    });
}

fn link(ui: &mut egui::Ui, text: &str, url: &str) {
    if ui.link(egui::RichText::new(text).size(12.0)).clicked() {
        App::open_url(url);
    }
}

fn legal(ui: &mut egui::Ui) {
    widgets::card(ui, None, |ui| {
        egui::CollapsingHeader::new(
            egui::RichText::new("Legal and trademarks").strong().size(13.5),
        )
        .id_salt("about-legal")
        .show(ui, |ui| {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(
                        "Unofficial. Not affiliated with, endorsed, sponsored or supported by \
                         Proton AG, Google LLC or Microsoft Corporation. This app uses the official \
                         Proton Drive CLI (payload MIT, Copyright (c) 2025-2026 Proton AG; source: \
                         github.com/ProtonDriveApps/sdk). All scripts in this repository, including \
                         this app, are MIT-licensed (see LICENSE). 'Proton' and 'Proton Drive' are \
                         trademarks of Proton AG, used here only to identify the product being \
                         packaged. Mentions of other products are feature comparisons only - no code, \
                         assets or configurations of those products are used. Not legal advice.",
                    )
                    .size(11.5)
                    .weak(),
                )
                .wrap(),
            );
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_version_drops_package_prefixes() {
        assert_eq!(cli_version_from("cli-drive@0.9.0"), "0.9.0");
        assert_eq!(cli_version_from("proton-drive 0.9.0"), "0.9.0");
        assert_eq!(cli_version_from("0.9.0+official"), "0.9.0");
        assert_eq!(cli_version_from("proton-drive 0.9.0 (build 7)"), "0.9.0");
        assert_eq!(cli_version_from("?"), "\u{2014}");
    }

    #[test]
    fn update_state_never_reports_failure_as_up_to_date() {
        assert_eq!(
            update_state_for(false, false, "Could not check for updates: 403"),
            ("error", "Update failed")
        );
        assert_eq!(
            update_state_for(false, true, "Update failed: download"),
            ("error", "Update failed")
        );
        assert_eq!(
            update_state_for(true, true, "Checking for updates..."),
            ("starting", "Checking")
        );
        assert_eq!(
            update_state_for(false, true, "Update available"),
            ("paused", "Update available")
        );
        assert_eq!(
            update_state_for(false, false, "You're up to date (latest release: v1)"),
            ("synced", "Up to date")
        );
        assert_eq!(
            update_state_for(false, false, ""),
            ("engine_stopped", "Not checked yet")
        );
    }
}
