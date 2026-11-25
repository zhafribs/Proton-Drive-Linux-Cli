//! The desktop's own colours, as the theme generator writes them.
//!
//! GTK colour schemes are plain `@define-color name #rrggbb;` files, and every
//! wallpaper-colour tool (Noctalia, matugen, pywal's GTK template, Gradience,
//! libadwaita's accent) writes the same names to the same two places. Reading
//! those tokens is how "match my wallpaper" works without decoding the picture
//! ourselves: the desktop already did it, and the shell is using the result.

use eframe::egui::Color32;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The tokens the app needs. Everything else is derived from them so a
/// wallpaper palette does not have to carry twenty hand-tuned shades.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SystemColors {
    pub accent: Color32,
    pub on_accent: Option<Color32>,
    pub page: Color32,
    pub surface: Color32,
    pub fg: Color32,
}

impl SystemColors {
    pub fn is_dark(&self) -> bool {
        crate::theme::luminance(self.page) < 0.5
    }
}

/// The desktop's generated palette, or `None` when nothing published one.
pub fn read() -> Option<SystemColors> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    [
        // The active desktop's generated file first, then the stock GTK one.
        "gtk-4.0/noctalia.css",
        "gtk-3.0/noctalia.css",
        "gtk-4.0/gtk.css",
        "gtk-3.0/gtk.css",
    ]
    .iter()
    .find_map(|rel| parse(&std::fs::read_to_string(base.join(rel)).ok()?))
}

fn parse(text: &str) -> Option<SystemColors> {
    let mut tokens = BTreeMap::new();
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("@define-color ") else {
            continue;
        };
        let Some((name, value)) = rest.split_once(char::is_whitespace) else {
            continue;
        };
        tokens.insert(name.trim().to_owned(), value.trim().trim_end_matches(';').to_owned());
    }
    let get = |names: &[&str]| {
        names
            .iter()
            .find_map(|n| tokens.get(*n))
            .and_then(|v| resolve(v, &tokens, 0))
    };
    Some(SystemColors {
        accent: get(&["accent_bg_color", "accent_color", "theme_selected_bg_color"])?,
        on_accent: get(&["accent_fg_color"]),
        page: get(&["window_bg_color", "view_bg_color"])?,
        surface: get(&["card_bg_color", "popover_bg_color"])
            .or_else(|| get(&["window_bg_color"]))?,
        fg: get(&["window_fg_color", "view_fg_color"])?,
    })
}

/// Values may point at another token (`@window_bg_color`).
fn resolve(value: &str, tokens: &BTreeMap<String, String>, depth: u8) -> Option<Color32> {
    if let Some(name) = value.strip_prefix('@') {
        if depth > 4 {
            return None;
        }
        return tokens
            .get(name)
            .and_then(|v| resolve(v, tokens, depth + 1));
    }
    parse_color(value)
}

fn parse_color(text: &str) -> Option<Color32> {
    let text = text.trim();
    if let Some(hex) = text.strip_prefix('#') {
        let nib = |i: usize| u8::from_str_radix(hex.get(i..i + 1)?, 16).ok();
        let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
        return match hex.len() {
            3 => Some(Color32::from_rgb(nib(0)? * 17, nib(1)? * 17, nib(2)? * 17)),
            6 | 8 => Some(Color32::from_rgb(byte(0)?, byte(2)?, byte(4)?)),
            _ => None,
        };
    }
    let inner = text
        .strip_prefix("rgba(")
        .or_else(|| text.strip_prefix("rgb("))?
        .trim_end_matches(')');
    let mut parts = inner.split(',').map(|p| p.trim().parse::<f32>().ok());
    let r = parts.next()??;
    let g = parts.next()??;
    let b = parts.next()??;
    Some(Color32::from_rgb(
        r.clamp(0.0, 255.0) as u8,
        g.clamp(0.0, 255.0) as u8,
        b.clamp(0.0, 255.0) as u8,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape Noctalia writes for a teal wallpaper.
    const TEAL: &str = "\
@define-color accent_color #84d2e6;
@define-color accent_bg_color #84d2e6;
@define-color accent_fg_color #003640;
@define-color window_bg_color #0f1416;
@define-color window_fg_color #dee3e5;
@define-color view_bg_color #0f1416;
@define-color card_bg_color #1b2122;
@define-color popover_bg_color #1b2122;
@define-color theme_selected_bg_color @accent_bg_color;
";

    #[test]
    fn reads_the_generated_tokens() {
        let c = parse(TEAL).expect("palette");
        assert_eq!(c.accent, Color32::from_rgb(0x84, 0xD2, 0xE6));
        assert_eq!(c.on_accent, Some(Color32::from_rgb(0x00, 0x36, 0x40)));
        assert_eq!(c.page, Color32::from_rgb(0x0F, 0x14, 0x16));
        assert_eq!(c.surface, Color32::from_rgb(0x1B, 0x21, 0x22));
        assert_eq!(c.fg, Color32::from_rgb(0xDE, 0xE3, 0xE5));
        assert!(c.is_dark());
    }

    #[test]
    fn falls_back_to_short_hex_and_rgb() {
        assert_eq!(parse_color("#abc"), Some(Color32::from_rgb(0xAA, 0xBB, 0xCC)));
        assert_eq!(parse_color("#11223344"), Some(Color32::from_rgb(0x11, 0x22, 0x33)));
        assert_eq!(parse_color("rgb(1, 2, 3)"), Some(Color32::from_rgb(1, 2, 3)));
        assert_eq!(parse_color("rgba(1,2,3,0.5)"), Some(Color32::from_rgb(1, 2, 3)));
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("red"), None);
    }

    #[test]
    fn ignores_a_file_without_the_needed_tokens() {
        // A stock gtk.css that only tweaks fonts, or an @import stub.
        assert_eq!(parse("@import url(\"noctalia.css\");\n"), None);
        assert_eq!(parse("@define-color window_bg_color #fff;\n"), None);
    }

    #[test]
    fn token_indirection_terminates() {
        let mut tokens = BTreeMap::new();
        tokens.insert("a".to_string(), "@b".to_string());
        tokens.insert("b".to_string(), "#102030".to_string());
        assert_eq!(resolve("@a", &tokens, 0), Some(Color32::from_rgb(0x10, 0x20, 0x30)));
        tokens.insert("b".to_string(), "@a".to_string());
        assert_eq!(resolve("@a", &tokens, 0), None);
    }
}
