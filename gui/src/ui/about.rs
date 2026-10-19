//! About: version, plain-language explainer, good-to-know list, legal block.

use super::{widgets, App};
use crate::paths::REPO_URL;
use eframe::egui;

pub fn draw(app: &mut App, ui: &mut egui::Ui) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.label(egui::RichText::new("About").strong().size(24.0));
        ui.add_space(12.0);

        // --- hero ---------------------------------------------------------
        widgets::card(ui, None, |ui| {
            ui.horizontal(|ui| {
                if let Some(tex) = &app.icon_tex {
                    ui.add(egui::Image::new((tex.id(), egui::vec2(72.0, 72.0))));
                }
                ui.add_space(20.0);
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new("Proton Drive for Linux").strong().size(24.0));
                    let version_line = match &app.identity {
                        None => format!("Version {}", crate::GUI_VERSION),
                        Some(id) => format!(
                            "Version {}  \u{b7}  Engine {}",
                            crate::GUI_VERSION,
                            crate::util::strip_build_suffix(&id.version)
                        ),
                    };
                    let raw = app.identity.as_ref().map(|i| i.version.clone());
                    let lbl = ui.label(egui::RichText::new(version_line).weak());
                    if let Some(r) = raw {
                        lbl.on_hover_text(r);
                    }
                    ui.add_space(8.0);
                    if ui.button("Visit the website").clicked() {
                        App::open_url(REPO_URL);
                    }
                });
            });
        });

        ui.add_space(14.0);

        // --- what is this --------------------------------------------------
        widgets::card(ui, Some("What is this?"), |ui| {
            ui.label(
                egui::RichText::new(
                    "Keeps a folder on this computer equal to your Proton Drive, \
                     automatically. Built by the community on top of Proton's official \
                     command-line tool. Encryption happens only on your device.",
                )
                .weak(),
            );
        });

        ui.add_space(14.0);

        // --- good to know --------------------------------------------------
        widgets::card(ui, Some("Good to know"), |ui| {
            for text in [
                "Deleted files stay safe unless you enable removal syncing.",
                "Conflicts keep both versions - nothing is lost.",
                "Proton Docs and Sheets stay online only.",
                "Your sign-in stays until you sign out.",
            ] {
                ui.label(egui::RichText::new(format!("\u{2022}  {text}")).weak());
                ui.add_space(4.0);
            }
        });

        ui.add_space(14.0);

        // --- legal ---------------------------------------------------------
        widgets::card(ui, Some("Legal"), |ui| {
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
                    .size(12.0)
                    .weak(),
                )
                .wrap(),
            );
        });
    });
}
