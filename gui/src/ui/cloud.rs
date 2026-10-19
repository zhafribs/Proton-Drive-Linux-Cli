//! Cloud: a browser for the remote side (breadcrumbs, back/forward, sorting,
//! right-click actions, uploads/downloads) - all through the official CLI.
//!
//! The page skeleton lives here; the full browser lands with its milestone
//! before this ships.

use super::App;
use eframe::egui;

pub fn draw(_app: &mut App, ui: &mut egui::Ui) {
    ui.label(egui::RichText::new("Cloud").strong().size(24.0));
    ui.add_space(12.0);
    ui.label(egui::RichText::new("Loading your cloud folder...").weak());
}
