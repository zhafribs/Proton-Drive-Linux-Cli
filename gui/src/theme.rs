//! Light/dark palettes with the Proton-purple accent (#6D4AFF), matching the
//! PyQt app's look closely enough to feel like the same product.

use crate::prefs::Theme;
use eframe::egui;

pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x6D, 0x4A, 0xFF);
pub const ACCENT_LIGHT: egui::Color32 = egui::Color32::from_rgb(0x8B, 0x6C, 0xFF);
pub const SUCCESS: egui::Color32 = egui::Color32::from_rgb(0x16, 0xA3, 0x4A);
pub const WARNING: egui::Color32 = egui::Color32::from_rgb(0xD9, 0x77, 0x06);
pub const ERROR: egui::Color32 = egui::Color32::from_rgb(0xDC, 0x26, 0x26);
pub const INFO: egui::Color32 = egui::Color32::from_rgb(0x25, 0x63, 0xEB);
pub const MUTED: egui::Color32 = egui::Color32::from_rgb(0x9C, 0xA3, 0xAF);

/// Orb/dot color per status key.
pub fn status_color(key: &str) -> egui::Color32 {
    match key {
        "synced" => SUCCESS,
        "syncing" => ACCENT,
        "paused" => WARNING,
        "error" => ERROR,
        _ => MUTED,
    }
}

/// Apply the palettes + widget rounding to the context, for both themes (so
/// an OS theme flip mid-session keeps the accent and rounded corners).
pub fn apply(ctx: &egui::Context, _theme: &Theme) {
    let mut base = ctx.style_of(egui::Theme::Dark).as_ref().clone();
    base.spacing.item_spacing = egui::vec2(10.0, 8.0);
    base.spacing.button_padding = egui::vec2(12.0, 6.0);
    base.visuals.window_shadow.blur = 24;

    let mut dark = egui::Visuals::dark();
    dark.selection.bg_fill = ACCENT;
    dark.hyperlink_color = ACCENT_LIGHT;
    dark.widgets.inactive.weak_bg_fill = dark.panel_fill.linear_multiply(0.94);
    dark.window_corner_radius = egui::CornerRadius::same(12);
    dark.menu_corner_radius = egui::CornerRadius::same(10);

    let mut light = egui::Visuals::light();
    light.selection.bg_fill = ACCENT;
    light.hyperlink_color = ACCENT;
    light.window_corner_radius = egui::CornerRadius::same(12);
    light.menu_corner_radius = egui::CornerRadius::same(10);

    for (t, visuals) in [(egui::Theme::Light, light), (egui::Theme::Dark, dark)] {
        let mut style = base.clone();
        style.visuals = visuals;
        ctx.set_style_of(t, style);
    }
}
