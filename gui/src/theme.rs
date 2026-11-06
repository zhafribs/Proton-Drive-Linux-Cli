//! Clarity styling: neutral solid surfaces, Proton violet accent, flat status
//! discs, and a quiet light/dark pair. egui paints everything with rounded
//! rects and lines - no textures, shaders, or per-frame allocations.

use crate::prefs::Theme;
use eframe::egui;

pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x70, 0x51, 0xF0);
pub const ACCENT_LIGHT: egui::Color32 = egui::Color32::from_rgb(0xA7, 0x8B, 0xFF);
pub const CYAN: egui::Color32 = egui::Color32::from_rgb(0x3F, 0xD8, 0xE8);
pub const MAGENTA: egui::Color32 = egui::Color32::from_rgb(0xD2, 0x6B, 0xE8);
pub const SUCCESS: egui::Color32 = egui::Color32::from_rgb(0x16, 0xA3, 0x4A);
pub const WARNING: egui::Color32 = egui::Color32::from_rgb(0xD9, 0x77, 0x06);
/// Darker success/warning shades for glyph text: the bright dots pass the
/// 3:1 non-text bar but not 4.5:1 as text, so glyphs use these instead.
pub const SUCCESS_TEXT: egui::Color32 = egui::Color32::from_rgb(0x0B, 0x7A, 0x38);
pub const WARNING_TEXT: egui::Color32 = egui::Color32::from_rgb(0x9A, 0x5B, 0x00);
pub const ERROR: egui::Color32 = egui::Color32::from_rgb(0xDC, 0x26, 0x26);
/// Darker red for danger *text* on the light page (AA on PAGE, not just SURFACE).
pub const ERROR_TEXT: egui::Color32 = egui::Color32::from_rgb(0xC8, 0x1E, 0x1E);
/// Lighter red for danger *text* on dark surfaces (AA at 13px).
pub const DANGER_TEXT: egui::Color32 = egui::Color32::from_rgb(0xF8, 0x71, 0x71);
pub const INFO: egui::Color32 = egui::Color32::from_rgb(0x1D, 0x4E, 0xD8);
/// Neutral grey used for dots and glyphs; #6B7280 keeps at least 3:1 on
/// both the light and dark surfaces (WCAG non-text contrast).
pub const MUTED: egui::Color32 = egui::Color32::from_rgb(0x6B, 0x72, 0x80);

/// Theme-aware muted text colour with WCAG AA contrast on its own surface.
pub fn muted(visuals: &egui::Visuals) -> egui::Color32 {
    if visuals.dark_mode {
        DARK_SECONDARY
    } else {
        SECONDARY
    }
}

// --- light theme: frosted daylight ------------------------------------------
/// White surface: cards, sidebar, buttons, inputs.
pub const SURFACE: egui::Color32 = egui::Color32::from_rgb(0xFF, 0xFF, 0xFF);
/// Page background behind the cards (neutral, not lavender).
pub const PAGE: egui::Color32 = egui::Color32::from_rgb(0xF6, 0xF6, 0xFA);
/// Headings (deep navy).
pub const NAVY: egui::Color32 = egui::Color32::from_rgb(0x1B, 0x19, 0x30);
/// Body text.
pub const INK: egui::Color32 = egui::Color32::from_rgb(0x33, 0x30, 0x4D);
/// Secondary text.
pub const SECONDARY: egui::Color32 = egui::Color32::from_rgb(0x6E, 0x6B, 0x84);
/// Hairline border for cards, inputs and buttons.
pub const BORDER: egui::Color32 = egui::Color32::from_rgb(0xE4, 0xE2, 0xEE);
/// Soft purple: active nav pill / pressed controls.
pub const ACCENT_SOFT: egui::Color32 = egui::Color32::from_rgb(0xEE, 0xE9, 0xFF);
/// Darker violet for accent *text* on the light accent-soft pill (AA).
pub const ACCENT_TEXT: egui::Color32 = egui::Color32::from_rgb(0x5B, 0x3F, 0xD0);
/// Purple-tinted hover for buttons and rows.
pub const ACCENT_HOVER: egui::Color32 = egui::Color32::from_rgb(0xF1, 0xEE, 0xFF);

// --- dark theme -------------------------------------------------------------
pub const DARK_PAGE: egui::Color32 = egui::Color32::from_rgb(0x0E, 0x0E, 0x16);
pub const DARK_SURFACE: egui::Color32 = egui::Color32::from_rgb(0x17, 0x17, 0x22);
pub const DARK_INPUT: egui::Color32 = egui::Color32::from_rgb(0x1F, 0x1F, 0x2E);
pub const DARK_HOVER: egui::Color32 = egui::Color32::from_rgb(0x26, 0x27, 0x38);
pub const DARK_FAINT: egui::Color32 = egui::Color32::from_rgb(0x1B, 0x1C, 0x2A);
pub const DARK_INK: egui::Color32 = egui::Color32::from_rgb(0xF2, 0xF1, 0xFA);
pub const DARK_STRONG: egui::Color32 = egui::Color32::from_rgb(0xF6, 0xF4, 0xFF);
pub const DARK_SECONDARY: egui::Color32 = egui::Color32::from_rgb(0xA6, 0xA3, 0xBC);
pub const DARK_BORDER: egui::Color32 = egui::Color32::from_rgb(0x2C, 0x2C, 0x3F);

/// Solid surface fill. The old glass/aurora layer is gone: cards and panels
/// are opaque so hierarchy comes from borders and spacing, not glow.
pub fn glass_fill(visuals: &egui::Visuals) -> egui::Color32 {
    if visuals.dark_mode {
        DARK_SURFACE
    } else {
        SURFACE
    }
}

/// Hairline edge for panels and cards.
pub fn glass_stroke(visuals: &egui::Visuals) -> egui::Stroke {
    egui::Stroke::new(
        1.0,
        if visuals.dark_mode {
            DARK_BORDER
        } else {
            BORDER
        },
    )
}

/// Danger text that stays AA on the current surface.
pub fn danger_text(visuals: &egui::Visuals) -> egui::Color32 {
    if visuals.dark_mode {
        DANGER_TEXT
    } else {
        ERROR_TEXT
    }
}

/// Warning text that stays AA on the current surface.
pub fn warning_text(visuals: &egui::Visuals) -> egui::Color32 {
    if visuals.dark_mode {
        WARNING
    } else {
        WARNING_TEXT
    }
}

/// Success text that stays readable on the current page.
pub fn success_text(visuals: &egui::Visuals) -> egui::Color32 {
    if visuals.dark_mode {
        SUCCESS
    } else {
        SUCCESS_TEXT
    }
}

/// Info blue for glyphs and dots: lighter on dark surfaces so the 3:1
/// non-text bar still holds.
pub fn info_text(visuals: &egui::Visuals) -> egui::Color32 {
    if visuals.dark_mode {
        egui::Color32::from_rgb(0x7D, 0xA7, 0xFF)
    } else {
        INFO
    }
}

/// Text colour for a status pill: the dot uses [`status_color`], the label
/// uses this so 11–12 px text keeps AA contrast on its tinted background.
pub fn status_text_color(key: &str, visuals: &egui::Visuals) -> egui::Color32 {
    match key {
        "synced" => success_text(visuals),
        "paused" | "deletes_held" | "needs_login" => warning_text(visuals),
        "error" => danger_text(visuals),
        "signed_out" | "starting" => {
            if visuals.dark_mode {
                ACCENT_LIGHT
            } else {
                INFO
            }
        }
        "signing_in" | "syncing" => accent(visuals),
        "engine_stopped" => muted(visuals),
        _ => muted(visuals),
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

/// Accent colour for text/icons on `accent_soft`; the bright accent is only
/// 4.26:1 on the light pill, so light mode uses the darker shade.
pub fn accent_text(visuals: &egui::Visuals) -> egui::Color32 {
    if visuals.dark_mode {
        ACCENT_LIGHT
    } else {
        ACCENT_TEXT
    }
}

/// Soft purple pill (active nav, pressed control) for the current theme.
pub fn accent_soft(visuals: &egui::Visuals) -> egui::Color32 {
    if visuals.dark_mode {
        ACCENT.gamma_multiply(0.30)
    } else {
        ACCENT_SOFT
    }
}

/// Pill radius. egui clamps a corner radius to half the smallest side, so
/// 255 renders as a full pill on buttons, inputs and segmented controls.
pub fn pill() -> egui::CornerRadius {
    egui::CornerRadius::same(255)
}

/// Soft card drop shadow: enough to separate a surface from the page, never
/// a glow or halo.
pub fn card_shadow(visuals: &egui::Visuals) -> egui::Shadow {
    if visuals.dark_mode {
        egui::Shadow {
            offset: [0, 4],
            blur: 18,
            spread: 0,
            color: egui::Color32::from_black_alpha(70),
        }
    } else {
        egui::Shadow {
            offset: [0, 2],
            blur: 12,
            spread: 0,
            color: egui::Color32::from_black_alpha(18),
        }
    }
}

/// The shared card frame. `card()` and the files table hang off this.
pub fn card_frame(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::new()
        .fill(glass_fill(ui.visuals()))
        .stroke(glass_stroke(ui.visuals()))
        .corner_radius(egui::CornerRadius::same(12))
        .shadow(card_shadow(ui.visuals()))
        .inner_margin(egui::Margin::symmetric(20, 18))
}


/// Orb/dot color per status key.
pub fn status_color(key: &str) -> egui::Color32 {
    match key {
        "synced" => SUCCESS,
        "syncing" => ACCENT,
        "paused" | "deletes_held" => WARNING,
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
pub fn apply(ctx: &egui::Context, _theme: &Theme, animate: bool) {
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
    dark.window_corner_radius = egui::CornerRadius::same(14);
    dark.menu_corner_radius = round10;
    dark.window_shadow.blur = 30;
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
    dark.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, DARK_BORDER);
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
        style.animation_time = if animate {
            egui::Style::default().animation_time
        } else {
            0.0
        };
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

    #[test]
    fn motion_off_disables_widget_animation() {
        let ctx = egui::Context::default();
        force(&ctx, &Theme::Dark);
        apply(&ctx, &Theme::Dark, false);
        assert_eq!(ctx.style_of(egui::Theme::Dark).animation_time, 0.0);
        apply(&ctx, &Theme::Dark, true);
        assert!(ctx.style_of(egui::Theme::Dark).animation_time > 0.0);
        apply(&ctx, &Theme::Light, false);
        assert_eq!(ctx.style_of(egui::Theme::Light).animation_time, 0.0);
        apply(&ctx, &Theme::Light, true);
        assert!(ctx.style_of(egui::Theme::Light).animation_time > 0.0);
    }

    fn luminance(c: egui::Color32) -> f64 {
        let lin = |v: u8| {
            let s = v as f64 / 255.0;
            if s <= 0.04045 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
    }

    fn ratio(a: egui::Color32, b: egui::Color32) -> f64 {
        let (l1, l2) = (luminance(a), luminance(b));
        let (hi, lo) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
        (hi + 0.05) / (lo + 0.05)
    }

    /// WCAG contrast gate for the palette: text pairs meet AA (4.5:1) and
    /// dot/indicator colours meet the non-text bar (3:1) on their surfaces.
    #[test]
    fn palette_meets_wcag_contrast() {
        for (fg, bg, min) in [
            (INK, SURFACE, 4.5),
            (NAVY, SURFACE, 4.5),
            (SECONDARY, SURFACE, 4.5),
            (SUCCESS_TEXT, SURFACE, 4.5),
            (WARNING_TEXT, SURFACE, 4.5),
            (WARNING_TEXT, PAGE, 4.5),
            (INFO, SURFACE, 4.5),
            (ACCENT_TEXT, ACCENT_SOFT, 4.5),
            (ERROR, SURFACE, 4.5),
            (ERROR_TEXT, PAGE, 4.5),
            (ERROR, DARK_PAGE, 3.0),
            (DANGER_TEXT, DARK_SURFACE, 4.5),
            (DANGER_TEXT, DARK_PAGE, 4.5),
            (DARK_INK, DARK_PAGE, 4.5),
            (DARK_STRONG, DARK_PAGE, 4.5),
            (DARK_SECONDARY, DARK_PAGE, 4.5),
            (MUTED, SURFACE, 3.0),
            (MUTED, DARK_PAGE, 3.0),
            (SUCCESS, SURFACE, 3.0),
            (SUCCESS, DARK_SURFACE, 3.0),
            (WARNING, SURFACE, 3.0),
            (SUCCESS, DARK_PAGE, 3.0),
            (WARNING, DARK_PAGE, 4.5),
            (WARNING, DARK_SURFACE, 4.5),
            (info_text(&egui::Visuals::dark()), DARK_SURFACE, 3.0),
            (info_text(&egui::Visuals::light()), SURFACE, 4.5),
            (egui::Color32::WHITE, ACCENT, 4.5),
        ] {
            let r = ratio(fg, bg);
            assert!(r >= min, "ratio {r:.2} below {min}: {fg:?} on {bg:?}");
        }
    }
}
