//! Aurora Glass styling: a deep-navy page under slowly drifting aurora
//! light, translucent glass surfaces, gradient violet→cyan call-to-action
//! buttons and a glowing status orb. Dark is the hero theme; the light
//! theme is the same language in frosted daylight. egui paints everything
//! with a handful of discs and rounded rects - no textures, no shaders, no
//! per-frame allocations.

use crate::prefs::Theme;
use eframe::egui;

pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x70, 0x51, 0xF0);
pub const ACCENT_LIGHT: egui::Color32 = egui::Color32::from_rgb(0xA7, 0x8B, 0xFF);
pub const CYAN: egui::Color32 = egui::Color32::from_rgb(0x3F, 0xD8, 0xE8);
pub const MAGENTA: egui::Color32 = egui::Color32::from_rgb(0xD2, 0x6B, 0xE8);
pub const TEAL: egui::Color32 = egui::Color32::from_rgb(0x35, 0xD0, 0xBA);
pub const SUCCESS: egui::Color32 = egui::Color32::from_rgb(0x16, 0xA3, 0x4A);
pub const WARNING: egui::Color32 = egui::Color32::from_rgb(0xD9, 0x77, 0x06);
/// Darker success/warning shades for glyph text: the bright dots pass the
/// 3:1 non-text bar but not 4.5:1 as text, so glyphs use these instead.
pub const SUCCESS_TEXT: egui::Color32 = egui::Color32::from_rgb(0x0B, 0x7A, 0x38);
pub const WARNING_TEXT: egui::Color32 = egui::Color32::from_rgb(0x9A, 0x5B, 0x00);
pub const ERROR: egui::Color32 = egui::Color32::from_rgb(0xDC, 0x26, 0x26);
pub const INFO: egui::Color32 = egui::Color32::from_rgb(0x25, 0x63, 0xEB);
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

/// `PROTON_UI_MOTION=0` freezes the aurora and every pulse for low-end
/// machines; the layout is identical either way.
pub fn motion_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("PROTON_UI_MOTION").ok().as_deref() != Some("0"))
}

// --- light theme: frosted daylight ------------------------------------------
/// White surface: cards, sidebar, buttons, inputs.
pub const SURFACE: egui::Color32 = egui::Color32::from_rgb(0xFF, 0xFF, 0xFF);
/// Page background behind the glass (pale lavender).
pub const PAGE: egui::Color32 = egui::Color32::from_rgb(0xED, 0xEB, 0xF8);
/// Headings (deep navy).
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

// --- dark theme: aurora night -------------------------------------------------
pub const DARK_PAGE: egui::Color32 = egui::Color32::from_rgb(0x0A, 0x0B, 0x1A);
pub const DARK_SURFACE: egui::Color32 = egui::Color32::from_rgb(0x16, 0x17, 0x2A);
pub const DARK_INPUT: egui::Color32 = egui::Color32::from_rgb(0x1D, 0x1E, 0x36);
pub const DARK_HOVER: egui::Color32 = egui::Color32::from_rgb(0x26, 0x27, 0x41);
pub const DARK_FAINT: egui::Color32 = egui::Color32::from_rgb(0x1B, 0x1C, 0x34);
pub const DARK_INK: egui::Color32 = egui::Color32::from_rgb(0xE2, 0xDF, 0xF2);
pub const DARK_STRONG: egui::Color32 = egui::Color32::from_rgb(0xF6, 0xF4, 0xFF);
pub const DARK_SECONDARY: egui::Color32 = egui::Color32::from_rgb(0xA6, 0xA2, 0xC6);
pub const DARK_BORDER: egui::Color32 = egui::Color32::from_rgb(0x34, 0x36, 0x5C);

/// Translucent glass fill that lets the aurora glow through.
pub fn glass_fill(visuals: &egui::Visuals) -> egui::Color32 {
    if visuals.dark_mode {
        egui::Color32::from_white_alpha(10)
    } else {
        egui::Color32::from_white_alpha(200)
    }
}

/// Hairline edge for glass surfaces.
pub fn glass_stroke(visuals: &egui::Visuals) -> egui::Stroke {
    if visuals.dark_mode {
        egui::Stroke::new(1.0, egui::Color32::from_white_alpha(36))
    } else {
        egui::Stroke::new(1.0, BORDER)
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

/// Soft card drop shadow: glass floats on the aurora with a deep halo.
pub fn card_shadow(visuals: &egui::Visuals) -> egui::Shadow {
    if visuals.dark_mode {
        egui::Shadow {
            offset: [0, 6],
            blur: 30,
            spread: 0,
            color: egui::Color32::from_black_alpha(110),
        }
    } else {
        egui::Shadow {
            offset: [0, 4],
            blur: 22,
            spread: 0,
            color: egui::Color32::from_black_alpha(22),
        }
    }
}

/// The shared glass card frame. `card()` and the files table hang off this.
pub fn card_frame(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::new()
        .fill(glass_fill(ui.visuals()))
        .stroke(glass_stroke(ui.visuals()))
        .corner_radius(egui::CornerRadius::same(18))
        .shadow(card_shadow(ui.visuals()))
        .inner_margin(egui::Margin::symmetric(20, 18))
}

/// Paint the aurora backdrop across `rect` (window coordinates). Five large
/// translucent discs drift on slow sine paths; with `PROTON_UI_MOTION=0`
/// the time term is zero and the layout is a fixed, still frame.
pub fn paint_aurora(ui: &egui::Ui, rect: egui::Rect) {
    let t = if motion_enabled() {
        ui.ctx().input(|i| i.time) as f32
    } else {
        0.0
    };
    let dark = ui.visuals().dark_mode;
    let palette: [egui::Color32; 5] = if dark {
        [ACCENT, CYAN, MAGENTA, TEAL, ACCENT_LIGHT]
    } else {
        [
            egui::Color32::from_rgb(0xE7, 0xE1, 0xFF),
            egui::Color32::from_rgb(0xD5, 0xF3, 0xF8),
            egui::Color32::from_rgb(0xF6, 0xDE, 0xF8),
            egui::Color32::from_rgb(0xD8, 0xF6, 0xF1),
            egui::Color32::from_rgb(0xE0, 0xE4, 0xFF),
        ]
    };
    // (centre x, centre y, x speed, y speed) - fractions of the rect.
    const BLUBS: [(f32, f32, f32, f32); 5] = [
        (0.24, 0.28, 0.022, 0.017),
        (0.76, 0.22, -0.026, 0.021),
        (0.50, 0.62, 0.018, -0.024),
        (0.14, 0.82, -0.020, -0.018),
        (0.86, 0.78, 0.024, 0.016),
    ];
    let w = rect.width().max(1.0);
    let h = rect.height().max(1.0);
    let base_r = w.max(h) * 0.42;
    for (i, (cx, cy, sx, sy)) in BLUBS.iter().enumerate() {
        let x = rect.left() + w * (cx + 0.24 * (t * sx).sin());
        let y = rect.top() + h * (cy + 0.20 * (t * sy).sin());
        let r = base_r * (0.80 + 0.15 * (t * 0.05 + i as f32).sin());
        let alpha = if dark { 0.055 } else { 0.16 };
        ui.painter()
            .circle_filled(egui::pos2(x, y), r, palette[i].gamma_multiply(alpha));
    }
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
    dark.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, CYAN.gamma_multiply(0.5));
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
            (INFO, SURFACE, 4.5),
            (ERROR, SURFACE, 4.5),
            (DARK_INK, DARK_PAGE, 4.5),
            (DARK_STRONG, DARK_PAGE, 4.5),
            (DARK_SECONDARY, DARK_PAGE, 4.5),
            (MUTED, SURFACE, 3.0),
            (MUTED, DARK_PAGE, 3.0),
            (SUCCESS, SURFACE, 3.0),
            (WARNING, SURFACE, 3.0),
            (SUCCESS, DARK_PAGE, 3.0),
            (WARNING, DARK_PAGE, 3.0),
            (egui::Color32::WHITE, ACCENT, 4.5),
        ] {
            let r = ratio(fg, bg);
            assert!(r >= min, "ratio {r:.2} below {min}: {fg:?} on {bg:?}");
        }
    }
}
