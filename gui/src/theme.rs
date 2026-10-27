//! Proton Drive web-app styling: the Proton-purple accent (#6D4AFF) on a
//! light-lavender page, white cards, navy headings and pill-shaped controls -
//! the visual language of the official Proton Drive web interface (this is an
//! independent egui implementation of that look, not Proton code). Both
//! themes get the treatment so an OS theme flip mid-session keeps the accent
//! and the rounded controls.

use crate::prefs::Theme;
use eframe::egui;

pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x6D, 0x4A, 0xFF);
pub const ACCENT_LIGHT: egui::Color32 = egui::Color32::from_rgb(0x8B, 0x6C, 0xFF);
pub const SUCCESS: egui::Color32 = egui::Color32::from_rgb(0x16, 0xA3, 0x4A);
pub const WARNING: egui::Color32 = egui::Color32::from_rgb(0xD9, 0x77, 0x06);
pub const ERROR: egui::Color32 = egui::Color32::from_rgb(0xDC, 0x26, 0x26);
pub const INFO: egui::Color32 = egui::Color32::from_rgb(0x25, 0x63, 0xEB);
pub const MUTED: egui::Color32 = egui::Color32::from_rgb(0x9C, 0xA3, 0xAF);

// --- light theme: the Proton Drive web look --------------------------------
/// White surface: cards, sidebar, buttons, inputs.
pub const SURFACE: egui::Color32 = egui::Color32::from_rgb(0xFF, 0xFF, 0xFF);
/// Page background behind the cards (light lavender).
pub const PAGE: egui::Color32 = egui::Color32::from_rgb(0xF7, 0xF6, 0xFD);
/// Headings (Proton navy).
pub const NAVY: egui::Color32 = egui::Color32::from_rgb(0x1B, 0x13, 0x40);
/// Body text.
pub const INK: egui::Color32 = egui::Color32::from_rgb(0x33, 0x30, 0x55);
/// Secondary text.
pub const SECONDARY: egui::Color32 = egui::Color32::from_rgb(0x6D, 0x6A, 0x85);
/// Hairline border for cards, inputs and buttons.
pub const BORDER: egui::Color32 = egui::Color32::from_rgb(0xE4, 0xE1, 0xF2);
/// Soft purple: active nav pill / pressed controls.
pub const ACCENT_SOFT: egui::Color32 = egui::Color32::from_rgb(0xEC, 0xE8, 0xFF);
/// Purple-tinted hover for buttons and rows.
pub const ACCENT_HOVER: egui::Color32 = egui::Color32::from_rgb(0xF1, 0xEE, 0xFF);

// --- dark theme ------------------------------------------------------------
pub const DARK_PAGE: egui::Color32 = egui::Color32::from_rgb(0x14, 0x12, 0x28);
pub const DARK_SURFACE: egui::Color32 = egui::Color32::from_rgb(0x1E, 0x1B, 0x3D);
pub const DARK_INPUT: egui::Color32 = egui::Color32::from_rgb(0x17, 0x15, 0x2F);
pub const DARK_HOVER: egui::Color32 = egui::Color32::from_rgb(0x2B, 0x27, 0x52);
pub const DARK_FAINT: egui::Color32 = egui::Color32::from_rgb(0x23, 0x1F, 0x46);
pub const DARK_INK: egui::Color32 = egui::Color32::from_rgb(0xDA, 0xD7, 0xF4);
pub const DARK_STRONG: egui::Color32 = egui::Color32::from_rgb(0xF2, 0xF0, 0xFF);
pub const DARK_SECONDARY: egui::Color32 = egui::Color32::from_rgb(0x99, 0x95, 0xBE);
pub const DARK_BORDER: egui::Color32 = egui::Color32::from_rgb(0x33, 0x2F, 0x5E);

/// Card/sidebar fill for the theme currently in use.
pub fn surface(visuals: &egui::Visuals) -> egui::Color32 {
    if visuals.dark_mode {
        DARK_SURFACE
    } else {
        SURFACE
    }
}

/// Accent that reads on the theme currently in use.
pub fn accent(visuals: &egui::Visuals) -> egui::Color32 {
    if visuals.dark_mode {
        ACCENT_LIGHT
    } else {
        ACCENT
    }
}

/// Soft purple pill (active nav, pressed control) for the current theme.
pub fn accent_soft(visuals: &egui::Visuals) -> egui::Color32 {
    if visuals.dark_mode {
        ACCENT.gamma_multiply(0.35)
    } else {
        ACCENT_SOFT
    }
}

/// Pill radius. egui clamps a corner radius to half the smallest side, so
/// 255 renders as a full pill on buttons, inputs and segmented controls.
pub fn pill() -> egui::CornerRadius {
    egui::CornerRadius::same(255)
}

/// Soft card drop shadow: a whisper of depth rather than a hard edge, so the
/// white cards float on the lavender page like the 2026 web look.
pub fn card_shadow(visuals: &egui::Visuals) -> egui::Shadow {
    if visuals.dark_mode {
        egui::Shadow {
            offset: [0, 3],
            blur: 20,
            spread: 0,
            color: egui::Color32::from_black_alpha(70),
        }
    } else {
        egui::Shadow {
            offset: [0, 2],
            blur: 16,
            spread: 0,
            color: egui::Color32::from_black_alpha(12),
        }
    }
}

/// The shared card frame: white surface, hairline border, soft shadow and a
/// generous radius. `card()` and the files table both hang off this.
pub fn card_frame(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::new()
        .fill(surface(ui.visuals()))
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(egui::CornerRadius::same(14))
        .shadow(card_shadow(ui.visuals()))
        .inner_margin(egui::Margin::symmetric(20, 18))
}

/// Orb/dot color per status key.
pub fn status_color(key: &str) -> egui::Color32 {
    match key {
        "synced" => SUCCESS,
        "syncing" => ACCENT,
        "paused" => WARNING,
        // A lost session is recoverable - warn, do not alarm.
        "needs_login" => WARNING,
        // No account yet: calm, not an error.
        "signed_out" => INFO,
        "signing_in" => ACCENT,
        // The engine is not running - neutral grey, not a failure.
        "engine_stopped" => MUTED,
        "starting" => INFO,
        "error" => ERROR,
        _ => MUTED,
    }
}

/// Pick the active egui theme. The app resolves `System` through
/// [`crate::system_theme`] before calling this, because Linux never reports a
/// system theme to egui; the `System` arm is only the defensive path (egui
/// then uses its own dark fallback). `apply` always styles both visual sets,
/// so this only flips which one egui paints with - without it the Appearance
/// combo stored the choice without ever showing it.
pub fn force(ctx: &egui::Context, theme: &Theme) {
    match theme {
        Theme::System => ctx.set_theme(egui::ThemePreference::System),
        Theme::Light => ctx.set_theme(egui::Theme::Light),
        Theme::Dark => ctx.set_theme(egui::Theme::Dark),
    }
}

/// Apply the palettes + widget styling to the context, for both themes (so
/// an OS theme flip mid-session keeps the accent and the pill controls).
///
/// egui derives most widgets from `Visuals::widgets`: buttons use
/// `weak_bg_fill` + `corner_radius`, TextEdits use `extreme_bg_color` +
/// the same corner radius, checkboxes cap their rounding themselves, and
/// `strong_text_color()` resolves to `widgets.active.fg_stroke` - so the
/// whole look cascades from these two blocks.
pub fn apply(ctx: &egui::Context, _theme: &Theme) {
    let mut base = ctx.style_of(egui::Theme::Dark).as_ref().clone();
    base.spacing.item_spacing = egui::vec2(10.0, 8.0);
    base.spacing.button_padding = egui::vec2(14.0, 7.0);

    let round12 = egui::CornerRadius::same(12);
    let round10 = egui::CornerRadius::same(10);
    let pill = pill();

    // --- dark ---------------------------------------------------------------
    let mut dark = egui::Visuals::dark();
    dark.panel_fill = DARK_PAGE;
    dark.window_fill = DARK_SURFACE;
    dark.extreme_bg_color = DARK_INPUT; // TextEdit background
    dark.faint_bg_color = DARK_FAINT;
    dark.weak_text_color = Some(DARK_SECONDARY);
    dark.hyperlink_color = ACCENT_LIGHT;
    dark.selection.bg_fill = ACCENT;
    dark.selection.stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);
    dark.button_frame = true;
    dark.window_corner_radius = round12;
    dark.menu_corner_radius = round10;
    dark.window_shadow.blur = 24;
    dark.widgets.noninteractive.bg_fill = DARK_SURFACE; // cards
    dark.widgets.noninteractive.weak_bg_fill = DARK_SURFACE;
    dark.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, DARK_BORDER);
    dark.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, DARK_INK); // body text
    dark.widgets.noninteractive.corner_radius = round12;
    dark.widgets.inactive.weak_bg_fill = DARK_SURFACE; // buttons
    dark.widgets.inactive.bg_fill = DARK_SURFACE; // checkbox box
    dark.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, DARK_BORDER);
    dark.widgets.inactive.corner_radius = pill;
    dark.widgets.hovered.bg_fill = DARK_HOVER;
    dark.widgets.hovered.weak_bg_fill = DARK_HOVER;
    dark.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, ACCENT.gamma_multiply(0.6));
    dark.widgets.hovered.fg_stroke = egui::Stroke::new(1.5, DARK_STRONG);
    dark.widgets.hovered.corner_radius = pill;
    dark.widgets.active.bg_fill = ACCENT.gamma_multiply(0.6);
    dark.widgets.active.weak_bg_fill = ACCENT.gamma_multiply(0.6);
    dark.widgets.active.bg_stroke = egui::Stroke::new(1.0, ACCENT_LIGHT);
    dark.widgets.active.fg_stroke = egui::Stroke::new(1.5, DARK_STRONG); // strong text
    dark.widgets.active.corner_radius = pill;
    dark.widgets.open.bg_fill = DARK_HOVER;
    dark.widgets.open.weak_bg_fill = DARK_HOVER;
    dark.widgets.open.bg_stroke = egui::Stroke::new(1.0, DARK_BORDER);
    dark.widgets.open.fg_stroke = egui::Stroke::new(1.0, DARK_STRONG);
    dark.widgets.open.corner_radius = pill;

    // --- light (the Proton Drive web look) ----------------------------------
    let mut light = egui::Visuals::light();
    light.panel_fill = PAGE;
    light.window_fill = SURFACE;
    light.extreme_bg_color = SURFACE; // TextEdit background
    light.faint_bg_color = egui::Color32::from_rgb(0xF1, 0xEF, 0xF9);
    light.weak_text_color = Some(SECONDARY);
    light.hyperlink_color = ACCENT;
    light.selection.bg_fill = ACCENT;
    light.selection.stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);
    light.button_frame = true;
    light.window_corner_radius = round12;
    light.menu_corner_radius = round10;
    light.window_shadow.blur = 24;
    light.widgets.noninteractive.bg_fill = SURFACE; // cards
    light.widgets.noninteractive.weak_bg_fill = SURFACE;
    light.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, BORDER);
    light.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, INK); // body text
    light.widgets.noninteractive.corner_radius = round12;
    light.widgets.inactive.weak_bg_fill = SURFACE; // buttons
    light.widgets.inactive.bg_fill = SURFACE; // checkbox box
    light.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, BORDER);
    light.widgets.inactive.corner_radius = pill;
    light.widgets.hovered.bg_fill = ACCENT_HOVER;
    light.widgets.hovered.weak_bg_fill = ACCENT_HOVER;
    light.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, BORDER);
    light.widgets.hovered.fg_stroke = egui::Stroke::new(1.5, NAVY);
    light.widgets.hovered.corner_radius = pill;
    light.widgets.active.bg_fill = ACCENT_SOFT;
    light.widgets.active.weak_bg_fill = ACCENT_SOFT;
    light.widgets.active.bg_stroke = egui::Stroke::new(1.0, ACCENT);
    light.widgets.active.fg_stroke = egui::Stroke::new(1.5, NAVY); // strong text
    light.widgets.active.corner_radius = pill;
    light.widgets.open.bg_fill = ACCENT_HOVER;
    light.widgets.open.weak_bg_fill = ACCENT_HOVER;
    light.widgets.open.bg_stroke = egui::Stroke::new(1.0, BORDER);
    light.widgets.open.fg_stroke = egui::Stroke::new(1.0, NAVY);
    light.widgets.open.corner_radius = pill;

    for (t, visuals) in [(egui::Theme::Light, light), (egui::Theme::Dark, dark)] {
        let mut style = base.clone();
        style.visuals = visuals;
        ctx.set_style_of(t, style);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn force_flips_the_active_theme() {
        let ctx = egui::Context::default();
        force(&ctx, &Theme::Dark);
        assert_eq!(ctx.theme(), egui::Theme::Dark);
        force(&ctx, &Theme::Light);
        assert_eq!(ctx.theme(), egui::Theme::Light);
        // The original bug: `System` was a no-op, so the forced Light stayed.
        force(&ctx, &Theme::System);
        assert_eq!(
            ctx.options(|o| o.theme_preference),
            egui::ThemePreference::System
        );
    }
}
