//! Clarity styling: neutral solid surfaces, Proton violet accent, flat status
//! discs, and a quiet light/dark pair. egui paints everything with rounded
//! rects and lines - no textures, shaders, or per-frame allocations.
//!
//! Two colour sources feed the same accessors: the Proton palette below, or
//! the desktop's generated GTK colours (see [`crate::system_colors`]) so the
//! window can follow the wallpaper like the shell does.

use crate::prefs::{ColorSource, Theme};
use crate::system_colors::SystemColors;
use eframe::egui;
use std::sync::{LazyLock, RwLock};

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

// --- file-type glyphs -------------------------------------------------------
// A separate namespace from the status colours above: these never appear on
// chrome, pills or surfaces. Light/dark pairs are contrast-tested on PAGE and
// SURFACE; amber and magenta status tokens are deliberately not reused.
pub const FILE_DOC_LIGHT: egui::Color32 = egui::Color32::from_rgb(0x1D, 0x4E, 0xD8);
pub const FILE_DOC_DARK: egui::Color32 = egui::Color32::from_rgb(0x7D, 0xA7, 0xFF);
pub const FILE_IMAGE_LIGHT: egui::Color32 = egui::Color32::from_rgb(0xD0, 0x5A, 0x86);
pub const FILE_IMAGE_DARK: egui::Color32 = egui::Color32::from_rgb(0xF0, 0xA3, 0xBF);
pub const FILE_MEDIA_LIGHT: egui::Color32 = egui::Color32::from_rgb(0x0F, 0x76, 0x6E);
pub const FILE_MEDIA_DARK: egui::Color32 = egui::Color32::from_rgb(0x2D, 0xD4, 0xBF);
pub const FILE_ARCHIVE_LIGHT: egui::Color32 = egui::Color32::from_rgb(0x6B, 0x6B, 0x1F);
pub const FILE_ARCHIVE_DARK: egui::Color32 = egui::Color32::from_rgb(0xC9, 0xC4, 0x6A);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FileKind {
    Document,
    Image,
    Media,
    Archive,
    Other,
}

/// Classify by extension without allocating: this runs per visible row.
pub fn file_kind(name: &str) -> FileKind {
    let Some((_, ext)) = name.rsplit_once('.') else {
        return FileKind::Other;
    };
    const DOCS: &[&str] = &[
        "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "md", "csv", "odt", "ods",
        "rtf",
    ];
    const IMAGES: &[&str] = &["jpg", "jpeg", "png", "gif", "webp", "heic", "svg", "bmp", "tif", "tiff"];
    const MEDIA: &[&str] = &["mp4", "mkv", "webm", "mov", "avi", "mp3", "wav", "flac", "m4a", "ogg"];
    const ARCHIVES: &[&str] = &["zip", "7z", "rar", "tar", "gz", "xz", "zst", "bz2"];
    if DOCS.iter().any(|e| ext.eq_ignore_ascii_case(e)) {
        FileKind::Document
    } else if IMAGES.iter().any(|e| ext.eq_ignore_ascii_case(e)) {
        FileKind::Image
    } else if MEDIA.iter().any(|e| ext.eq_ignore_ascii_case(e)) {
        FileKind::Media
    } else if ARCHIVES.iter().any(|e| ext.eq_ignore_ascii_case(e)) {
        FileKind::Archive
    } else {
        FileKind::Other
    }
}

/// Glyph colour for a file row. Folders keep the brand accent; selection is
/// handled by the caller so a selected row stays a solid accent band.
pub fn file_color(name: &str, visuals: &egui::Visuals) -> egui::Color32 {
    let dark = visuals.dark_mode;
    match file_kind(name) {
        FileKind::Document => {
            if dark { FILE_DOC_DARK } else { FILE_DOC_LIGHT }
        }
        FileKind::Image => {
            if dark { FILE_IMAGE_DARK } else { FILE_IMAGE_LIGHT }
        }
        FileKind::Media => {
            if dark { FILE_MEDIA_DARK } else { FILE_MEDIA_LIGHT }
        }
        FileKind::Archive => {
            if dark { FILE_ARCHIVE_DARK } else { FILE_ARCHIVE_LIGHT }
        }
        FileKind::Other => muted(visuals),
    }
}

/// Theme-aware muted text colour with WCAG AA contrast on its own surface.
pub fn muted(visuals: &egui::Visuals) -> egui::Color32 {
    mode(visuals).secondary
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
/// Hairline border for cards and panels.
pub const BORDER: egui::Color32 = egui::Color32::from_rgb(0xE4, 0xE2, 0xEE);
/// Stronger border for interactive controls (checkboxes, inputs, buttons):
/// the decorative hairline is only 1.28:1 on white, below the 3:1 non-text
/// contrast bar, so form controls need their own shade.
pub const CONTROL_BORDER: egui::Color32 = egui::Color32::from_rgb(0x8B, 0x88, 0x99);
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
/// Interactive-control border for the dark theme (3:1 on DARK_SURFACE).
pub const DARK_CONTROL_BORDER: egui::Color32 = egui::Color32::from_rgb(0x6B, 0x6A, 0x82);

// --- the live palette -------------------------------------------------------
// Every value the app paints comes from here. The constants above are the
// Proton source and the fallback when the desktop publishes no colours.

/// One resolved colour set for one of egui's two visual modes.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Mode {
    page: egui::Color32,
    surface: egui::Color32,
    input: egui::Color32,
    faint: egui::Color32,
    hover: egui::Color32,
    ink: egui::Color32,
    strong: egui::Color32,
    secondary: egui::Color32,
    border: egui::Color32,
    control_border: egui::Color32,
    accent: egui::Color32,
    accent_light: egui::Color32,
    accent_soft: egui::Color32,
    accent_text: egui::Color32,
    accent_hover: egui::Color32,
    on_accent: egui::Color32,
}

impl Mode {
    fn proton_light() -> Mode {
        Mode {
            page: PAGE,
            surface: SURFACE,
            input: SURFACE,
            faint: egui::Color32::from_rgb(0xF1, 0xEF, 0xF9),
            hover: ACCENT_HOVER,
            ink: INK,
            strong: NAVY,
            secondary: SECONDARY,
            border: BORDER,
            control_border: CONTROL_BORDER,
            accent: ACCENT,
            accent_light: ACCENT_LIGHT,
            accent_soft: ACCENT_SOFT,
            accent_text: ACCENT_TEXT,
            accent_hover: ACCENT_HOVER,
            on_accent: egui::Color32::WHITE,
        }
    }

    fn proton_dark() -> Mode {
        Mode {
            page: DARK_PAGE,
            surface: DARK_SURFACE,
            input: DARK_INPUT,
            faint: DARK_FAINT,
            hover: DARK_HOVER,
            ink: DARK_INK,
            strong: DARK_STRONG,
            secondary: DARK_SECONDARY,
            border: DARK_BORDER,
            control_border: DARK_CONTROL_BORDER,
            accent: ACCENT,
            accent_light: ACCENT_LIGHT,
            accent_soft: ACCENT.gamma_multiply(0.30),
            accent_text: ACCENT_LIGHT,
            accent_hover: DARK_HOVER,
            on_accent: egui::Color32::WHITE,
        }
    }

    /// Replace the neutrals with the desktop's page/card/text tokens. Shades
    /// are mixed from those three, so a wallpaper palette only has to publish
    /// the ones it actually has.
    fn with_surfaces(mut self, s: &SystemColors) -> Mode {
        self.page = s.page;
        self.surface = s.surface;
        self.ink = ensure_contrast(s.fg, self.surface, 4.5);
        self.strong = self.ink;
        self.secondary = ensure_contrast(mix(self.ink, self.surface, 0.40), self.surface, 4.5);
        self.input = mix(self.surface, self.ink, 0.06);
        self.faint = mix(self.surface, self.ink, 0.05);
        self.hover = mix(self.surface, self.ink, 0.09);
        self.border = mix(self.surface, self.ink, 0.16);
        self.control_border =
            ensure_contrast(mix(self.surface, self.ink, 0.40), self.surface, 3.0);
        self
    }

    /// Replace the accent family, keeping the rest of this mode's surfaces.
    fn with_accent(mut self, s: &SystemColors) -> Mode {
        let accent = s.accent;
        let dark = luminance(self.surface) < 0.5;
        self.accent = accent;
        self.accent_light = ensure_contrast(accent, self.surface, 4.5);
        self.accent_soft = mix(accent, self.surface, if dark { 0.72 } else { 0.86 });
        self.accent_hover = mix(accent, self.surface, if dark { 0.85 } else { 0.93 });
        self.accent_text = if dark {
            self.accent_light
        } else {
            ensure_contrast(accent, self.accent_soft, 4.5)
        };
        // The desktop ships a matching foreground for its accent; only trust
        // it when it actually reads on that fill.
        self.on_accent = s
            .on_accent
            .filter(|c| contrast(*c, accent) >= 4.5)
            .unwrap_or_else(|| {
                let target = if contrast(egui::Color32::WHITE, accent)
                    >= contrast(egui::Color32::BLACK, accent)
                {
                    egui::Color32::WHITE
                } else {
                    egui::Color32::BLACK
                };
                ensure_contrast(target, accent, 4.5)
            });
        self
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
struct Palette {
    light: Mode,
    dark: Mode,
    /// True when the colours came from the desktop, not from Proton.
    system: bool,
}

impl Palette {
    fn proton() -> Palette {
        Palette {
            light: Mode::proton_light(),
            dark: Mode::proton_dark(),
            system: false,
        }
    }

    /// The desktop's accent on both modes; its surfaces replace the neutrals
    /// of whichever mode it was generated for. A dark wallpaper palette must
    /// not repaint the light theme dark, so the other mode keeps the Proton
    /// surfaces and takes only the accent.
    fn from_system(s: &SystemColors) -> Palette {
        let mut light = Mode::proton_light();
        let mut dark = Mode::proton_dark();
        if s.is_dark() {
            dark = dark.with_surfaces(s);
        } else {
            light = light.with_surfaces(s);
        }
        light = light.with_accent(s);
        dark = dark.with_accent(s);
        Palette {
            light,
            dark,
            system: true,
        }
    }
}

static ACTIVE: LazyLock<RwLock<Palette>> = LazyLock::new(|| RwLock::new(Palette::proton()));

fn active() -> Palette {
    *ACTIVE.read().unwrap_or_else(|e| e.into_inner())
}

/// The palette in use for the current visual mode.
fn mode(visuals: &egui::Visuals) -> Mode {
    if visuals.dark_mode {
        active().dark
    } else {
        active().light
    }
}

/// Whether "My system" found desktop colours to use.
pub fn system_colors_active() -> bool {
    active().system
}

pub(crate) fn luminance(c: egui::Color32) -> f64 {
    let lin = |v: u8| {
        let s = f64::from(v) / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
}

pub(crate) fn contrast(a: egui::Color32, b: egui::Color32) -> f64 {
    let (l1, l2) = (luminance(a), luminance(b));
    let (hi, lo) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
    (hi + 0.05) / (lo + 0.05)
}

fn mix(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0);
    let ch = |x: u8, y: u8| (f32::from(x) * (1.0 - t) + f32::from(y) * t).round() as u8;
    egui::Color32::from_rgb(ch(a.r(), b.r()), ch(a.g(), b.g()), ch(a.b(), b.b()))
}

/// Nudge `c` toward white or black until it reads on `bg`. A wallpaper palette
/// is not required to be accessible; the app still is.
fn ensure_contrast(c: egui::Color32, bg: egui::Color32, min: f64) -> egui::Color32 {
    if contrast(c, bg) >= min {
        return c;
    }
    let target = if luminance(bg) < 0.5 {
        egui::Color32::WHITE
    } else {
        egui::Color32::BLACK
    };
    let mut out = c;
    for step in 1..=50 {
        out = mix(c, target, step as f32 / 50.0);
        if contrast(out, bg) >= min {
            break;
        }
    }
    out
}

/// Solid surface fill. The old glass/aurora layer is gone: cards and panels
/// are opaque so hierarchy comes from borders and spacing, not glow.
pub fn glass_fill(visuals: &egui::Visuals) -> egui::Color32 {
    mode(visuals).surface
}

/// The page behind the cards.
pub fn page(visuals: &egui::Visuals) -> egui::Color32 {
    mode(visuals).page
}

/// Text/icon colour that reads on an accent-filled control (primary button,
/// avatar, selection).
pub fn on_accent(visuals: &egui::Visuals) -> egui::Color32 {
    mode(visuals).on_accent
}

/// Hairline edge for panels and cards.
pub fn glass_stroke(visuals: &egui::Visuals) -> egui::Stroke {
    egui::Stroke::new(1.0, mode(visuals).border)
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
                mode(visuals).accent_light
            } else {
                INFO
            }
        }
        "signing_in" | "syncing" => accent_text(visuals),
        "engine_stopped" => muted(visuals),
        _ => muted(visuals),
    }
}

/// Accent that reads on the theme currently in use.
pub fn accent(visuals: &egui::Visuals) -> egui::Color32 {
    mode(visuals).accent
}

/// Accent colour for text/icons on `accent_soft`; the bright accent is only
/// 4.26:1 on the light pill, so light mode uses the darker shade.
pub fn accent_text(visuals: &egui::Visuals) -> egui::Color32 {
    mode(visuals).accent_text
}

/// Soft accent pill (active nav, pressed control) for the current theme.
pub fn accent_soft(visuals: &egui::Visuals) -> egui::Color32 {
    mode(visuals).accent_soft
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
pub fn status_color(key: &str, visuals: &egui::Visuals) -> egui::Color32 {
    match key {
        "synced" => SUCCESS,
        "syncing" => accent(visuals),
        "paused" | "deletes_held" => WARNING,
        // A lost session is recoverable - warn, do not alarm.
        "needs_login" => WARNING,
        // No account yet: calm, not an error.
        "signed_out" => INFO,
        "signing_in" => accent(visuals),
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
pub fn apply(ctx: &egui::Context, colors: ColorSource, animate: bool) {
    let palette = match colors {
        ColorSource::System => crate::system_colors::read()
            .map(|s| Palette::from_system(&s))
            .unwrap_or_else(Palette::proton),
        ColorSource::Proton => Palette::proton(),
    };
    *ACTIVE.write().unwrap_or_else(|e| e.into_inner()) = palette;
    let (dm, lm) = (palette.dark, palette.light);
    let mut base = ctx.style_of(egui::Theme::Dark).as_ref().clone();
    base.spacing.item_spacing = egui::vec2(10.0, 8.0);
    base.spacing.button_padding = egui::vec2(14.0, 7.0);

    let round12 = egui::CornerRadius::same(12);
    let round10 = egui::CornerRadius::same(10);
    let pill = pill();

    // --- dark ---------------------------------------------------------------
    let mut dark = egui::Visuals::dark();
    dark.panel_fill = dm.page;
    dark.window_fill = dm.surface;
    dark.extreme_bg_color = dm.input; // TextEdit background
    dark.faint_bg_color = dm.faint;
    dark.weak_text_color = Some(dm.secondary);
    dark.hyperlink_color = dm.accent_light;
    dark.selection.bg_fill = dm.accent;
    dark.selection.stroke = egui::Stroke::new(1.0, dm.on_accent);
    dark.button_frame = true;
    dark.window_corner_radius = egui::CornerRadius::same(14);
    dark.menu_corner_radius = round10;
    dark.window_shadow.blur = 30;
    dark.widgets.noninteractive.bg_fill = dm.surface; // cards
    dark.widgets.noninteractive.weak_bg_fill = dm.surface;
    dark.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, dm.border);
    dark.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, dm.ink); // body text
    dark.widgets.noninteractive.corner_radius = round12;
    dark.widgets.inactive.weak_bg_fill = dm.surface; // buttons
    dark.widgets.inactive.bg_fill = dm.surface; // checkbox box
    dark.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, dm.control_border);
    dark.widgets.inactive.corner_radius = pill;
    dark.widgets.hovered.bg_fill = dm.hover;
    dark.widgets.hovered.weak_bg_fill = dm.hover;
    dark.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, dm.control_border);
    dark.widgets.hovered.fg_stroke = egui::Stroke::new(1.5, dm.strong);
    dark.widgets.hovered.corner_radius = pill;
    dark.widgets.active.bg_fill = dm.accent.gamma_multiply(0.6);
    dark.widgets.active.weak_bg_fill = dm.accent.gamma_multiply(0.6);
    dark.widgets.active.bg_stroke = egui::Stroke::new(1.0, dm.accent_light);
    dark.widgets.active.fg_stroke = egui::Stroke::new(1.5, dm.strong); // strong text
    dark.widgets.active.corner_radius = pill;
    dark.widgets.open.bg_fill = dm.hover;
    dark.widgets.open.weak_bg_fill = dm.hover;
    dark.widgets.open.bg_stroke = egui::Stroke::new(1.0, dm.control_border);
    dark.widgets.open.fg_stroke = egui::Stroke::new(1.0, dm.strong);
    dark.widgets.open.corner_radius = pill;

    // --- light (the Proton Drive web look) ----------------------------------
    let mut light = egui::Visuals::light();
    light.panel_fill = lm.page;
    light.window_fill = lm.surface;
    light.extreme_bg_color = lm.input; // TextEdit background
    light.faint_bg_color = lm.faint;
    light.weak_text_color = Some(lm.secondary);
    light.hyperlink_color = lm.accent;
    light.selection.bg_fill = lm.accent;
    light.selection.stroke = egui::Stroke::new(1.0, lm.on_accent);
    light.button_frame = true;
    light.window_corner_radius = round12;
    light.menu_corner_radius = round10;
    light.window_shadow.blur = 24;
    light.widgets.noninteractive.bg_fill = lm.surface; // cards
    light.widgets.noninteractive.weak_bg_fill = lm.surface;
    light.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, lm.border);
    light.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, lm.ink); // body text
    light.widgets.noninteractive.corner_radius = round12;
    light.widgets.inactive.weak_bg_fill = lm.surface; // buttons
    light.widgets.inactive.bg_fill = lm.surface; // checkbox box
    light.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, lm.control_border);
    light.widgets.inactive.corner_radius = pill;
    light.widgets.hovered.bg_fill = lm.accent_hover;
    light.widgets.hovered.weak_bg_fill = lm.accent_hover;
    light.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, lm.control_border);
    light.widgets.hovered.fg_stroke = egui::Stroke::new(1.5, lm.strong);
    light.widgets.hovered.corner_radius = pill;
    light.widgets.active.bg_fill = lm.accent_soft;
    light.widgets.active.weak_bg_fill = lm.accent_soft;
    light.widgets.active.bg_stroke = egui::Stroke::new(1.0, lm.accent);
    light.widgets.active.fg_stroke = egui::Stroke::new(1.5, lm.strong); // strong text
    light.widgets.active.corner_radius = pill;
    light.widgets.open.bg_fill = lm.accent_hover;
    light.widgets.open.weak_bg_fill = lm.accent_hover;
    light.widgets.open.bg_stroke = egui::Stroke::new(1.0, lm.control_border);
    light.widgets.open.fg_stroke = egui::Stroke::new(1.0, lm.strong);
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

/// Re-read the desktop colours (a wallpaper change rewrites them) and restyle
/// only when they moved. Returns whether the palette changed.
pub fn refresh(ctx: &egui::Context, colors: ColorSource, animate: bool) -> bool {
    let before = active();
    apply(ctx, colors, animate);
    active() != before
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
        apply(&ctx, ColorSource::Proton, false);
        assert_eq!(ctx.style_of(egui::Theme::Dark).animation_time, 0.0);
        apply(&ctx, ColorSource::Proton, true);
        assert!(ctx.style_of(egui::Theme::Dark).animation_time > 0.0);
        apply(&ctx, ColorSource::Proton, false);
        assert_eq!(ctx.style_of(egui::Theme::Light).animation_time, 0.0);
        apply(&ctx, ColorSource::Proton, true);
        assert!(ctx.style_of(egui::Theme::Light).animation_time > 0.0);
    }

    fn ratio(a: egui::Color32, b: egui::Color32) -> f64 {
        contrast(a, b)
    }

    /// WCAG contrast gate for the palette: text pairs meet AA (4.5:1) and
    /// dot/indicator colours meet the non-text bar (3:1) on their surfaces.
    #[test]
    fn file_extensions_map_to_type_glyphs() {
        assert_eq!(file_kind("report.PDF"), FileKind::Document);
        assert_eq!(file_kind("photo.jpeg"), FileKind::Image);
        assert_eq!(file_kind("clip.MP4"), FileKind::Media);
        assert_eq!(file_kind("backup.tar.gz"), FileKind::Archive);
        assert_eq!(file_kind("README"), FileKind::Other);
        assert_eq!(file_kind("no-extension"), FileKind::Other);
    }

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
            (CONTROL_BORDER, SURFACE, 3.0),
            (DARK_CONTROL_BORDER, DARK_SURFACE, 3.0),
            (SUCCESS, SURFACE, 3.0),
            (SUCCESS, DARK_SURFACE, 3.0),
            (WARNING, SURFACE, 3.0),
            (SUCCESS, DARK_PAGE, 3.0),
            (WARNING, DARK_PAGE, 4.5),
            (WARNING, DARK_SURFACE, 4.5),
            (info_text(&egui::Visuals::dark()), DARK_SURFACE, 3.0),
            (info_text(&egui::Visuals::light()), SURFACE, 4.5),
            (egui::Color32::WHITE, ACCENT, 4.5),
            (FILE_DOC_LIGHT, SURFACE, 3.0),
            (FILE_DOC_LIGHT, PAGE, 3.0),
            (FILE_DOC_DARK, DARK_SURFACE, 3.0),
            (FILE_DOC_DARK, DARK_PAGE, 3.0),
            (FILE_IMAGE_LIGHT, SURFACE, 3.0),
            (FILE_IMAGE_LIGHT, PAGE, 3.0),
            (FILE_IMAGE_DARK, DARK_SURFACE, 3.0),
            (FILE_IMAGE_DARK, DARK_PAGE, 3.0),
            (FILE_MEDIA_LIGHT, SURFACE, 3.0),
            (FILE_MEDIA_LIGHT, PAGE, 3.0),
            (FILE_MEDIA_DARK, DARK_SURFACE, 3.0),
            (FILE_MEDIA_DARK, DARK_PAGE, 3.0),
            (FILE_ARCHIVE_LIGHT, SURFACE, 3.0),
            (FILE_ARCHIVE_LIGHT, PAGE, 3.0),
            (FILE_ARCHIVE_DARK, DARK_SURFACE, 3.0),
            (FILE_ARCHIVE_DARK, DARK_PAGE, 3.0),
        ] {
            let r = ratio(fg, bg);
            assert!(r >= min, "ratio {r:.2} below {min}: {fg:?} on {bg:?}");
        }
    }

    /// The Proton palette is what shipped: moving every accessor onto the live
    /// palette must not change a single default pixel.
    #[test]
    fn proton_palette_is_the_original_theme() {
        let light = Mode::proton_light();
        assert_eq!(light.page, PAGE);
        assert_eq!(light.surface, SURFACE);
        assert_eq!(light.ink, INK);
        assert_eq!(light.secondary, SECONDARY);
        assert_eq!(light.accent, ACCENT);
        assert_eq!(light.accent_soft, ACCENT_SOFT);
        assert_eq!(light.accent_text, ACCENT_TEXT);
        assert_eq!(light.accent_hover, ACCENT_HOVER);
        assert_eq!(light.border, BORDER);
        assert_eq!(light.control_border, CONTROL_BORDER);
        let dark = Mode::proton_dark();
        assert_eq!(dark.page, DARK_PAGE);
        assert_eq!(dark.surface, DARK_SURFACE);
        assert_eq!(dark.input, DARK_INPUT);
        assert_eq!(dark.ink, DARK_INK);
        assert_eq!(dark.secondary, DARK_SECONDARY);
        assert_eq!(dark.accent_soft, ACCENT.gamma_multiply(0.30));
        assert_eq!(dark.accent_text, ACCENT_LIGHT);
    }

    fn wallpaper(is_dark: bool) -> SystemColors {
        if is_dark {
            SystemColors {
                accent: egui::Color32::from_rgb(0x84, 0xD2, 0xE6),
                on_accent: Some(egui::Color32::from_rgb(0x00, 0x36, 0x40)),
                page: egui::Color32::from_rgb(0x0F, 0x14, 0x16),
                surface: egui::Color32::from_rgb(0x1B, 0x21, 0x22),
                fg: egui::Color32::from_rgb(0xDE, 0xE3, 0xE5),
            }
        } else {
            SystemColors {
                accent: egui::Color32::from_rgb(0x4F, 0x6B, 0x8A),
                on_accent: None,
                page: egui::Color32::from_rgb(0xFA, 0xF8, 0xF5),
                surface: egui::Color32::from_rgb(0xFF, 0xFF, 0xFF),
                fg: egui::Color32::from_rgb(0x22, 0x1E, 0x1B),
            }
        }
    }

    /// A wallpaper palette is arbitrary, so the derived shades must still meet
    /// the same contrast bars the Proton palette is tested against.
    #[test]
    fn system_palette_stays_readable() {
        for dark in [true, false] {
            let p = Palette::from_system(&wallpaper(dark));
            let (m, other) = if dark { (p.dark, p.light) } else { (p.light, p.dark) };
            assert!(p.system);
            assert_eq!(m.page, wallpaper(dark).page);
            // The mode the wallpaper did not describe keeps the Proton look.
            assert_eq!(other.page, if dark { PAGE } else { DARK_PAGE });
            for (fg, bg, min) in [
                (m.ink, m.surface, 4.5),
                (m.secondary, m.surface, 4.5),
                (m.accent_text, m.accent_soft, 4.5),
                (m.accent_light, m.surface, 4.5),
                (m.control_border, m.surface, 3.0),
                (m.on_accent, m.accent, 4.5),
                (m.ink, m.input, 4.5),
            ] {
                let r = ratio(fg, bg);
                assert!(r >= min, "dark={dark} ratio {r:.2} below {min}");
            }
        }
    }

    /// A washed-out wallpaper accent (yellow) must not leave white labels on
    /// the primary button.
    #[test]
    fn system_accent_gets_a_readable_foreground() {
        let mut s = wallpaper(true);
        s.accent = egui::Color32::from_rgb(0xF2, 0xE0, 0x6B);
        s.on_accent = Some(egui::Color32::WHITE); // the desktop guessed wrong
        let m = Palette::from_system(&s).dark;
        assert!(ratio(m.on_accent, m.accent) >= 4.5);
    }
}
