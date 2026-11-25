//! The desktop's own colours, as the theme generator writes them.
//!
//! GTK colour schemes are plain `@define-color name #rrggbb;` files. Whatever
//! generated them - Noctalia, matugen, pywal, Gradience, caelestia, end-4's
//! templates, ML4W - they land in `~/.config/gtk-4.0` or `~/.config/gtk-3.0`
//! under a name we do not have to know: every `*.css` there is read, newest
//! first, and the first complete palette wins. Reading those tokens is how
//! "match my wallpaper" works without decoding the picture ourselves: the
//! desktop already did it, and the shell is using the result.

use eframe::egui::Color32;
use inotify::{EventMask, Inotify, WatchMask};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

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

/// Where palette tools write, in preference order.
const DIRS: [&str; 2] = ["gtk-4.0", "gtk-3.0"];

fn config_base() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
}

/// The desktop's generated palette, or `None` when nothing published one.
pub fn read() -> Option<SystemColors> {
    read_at(&config_base()?)
}

/// Newest complete palette in the GTK config directories.
///
/// Filenames differ per tool (`gtk.css`, `noctalia.css`, `colors.css`, ...),
/// and a stock `gtk.css` is often just an `@import` stub, so parse every CSS
/// file and take the newest one that carries the tokens. GTK 4 wins over GTK 3
/// when both publish one.
fn read_at(base: &Path) -> Option<SystemColors> {
    DIRS.iter().find_map(|dir| {
        let mut files = css_files(&base.join(dir));
        // Newest first: the file the desktop just generated is the palette in
        // use; an older leftover from a previous tool must not win.
        files.sort_by_key(|(time, _)| std::cmp::Reverse(*time));
        files
            .into_iter()
            .find_map(|(_, path)| parse(&std::fs::read_to_string(path).ok()?))
    })
}

/// `(modified, path)` for every CSS file in `dir`; subdirectories are ignored.
fn css_files(dir: &Path) -> Vec<(std::time::SystemTime, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| is_css_path(path))
        .filter_map(|path| Some((std::fs::metadata(&path).ok()?.modified().ok()?, path)))
        .collect()
}

fn is_css_path(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("css"))
}

/// Fallback poll when inotify cannot watch the GTK directories.
const POLL: Duration = Duration::from_secs(1);

/// Report when the desktop rewrites its generated colours.
///
/// A wallpaper change is not a colour-scheme change, so the portal's
/// `SettingChanged` signal never fires for it and the 30-second theme poll
/// left the window on the old palette for half a minute. Palette tools write
/// their CSS in place or through a rename, which inotify reports at once,
/// whatever the file is called; one `()` is sent per batch of writes and the
/// UI thread re-reads.
pub fn watch(ctx: eframe::egui::Context) -> mpsc::Receiver<()> {
    let (tx, rx) = mpsc::channel();
    let Some(base) = config_base() else {
        return rx;
    };
    let _ = std::thread::Builder::new()
        .name("system-colors".into())
        .spawn(move || run(base, tx, ctx));
    rx
}

fn run(base: PathBuf, tx: mpsc::Sender<()>, ctx: eframe::egui::Context) {
    match watcher(&base) {
        Some(mut inotify) => {
            let mut buf = [0u8; 4096];
            loop {
                let events = match inotify.read_events_blocking(&mut buf) {
                    Ok(events) => events,
                    Err(_) => return,
                };
                let mut wake = false;
                for event in events {
                    if event.mask.contains(EventMask::Q_OVERFLOW) {
                        // Events were dropped: re-read rather than miss a write.
                        wake = true;
                    } else if event.mask.contains(EventMask::IGNORED)
                        || event.mask.contains(EventMask::DELETE_SELF)
                    {
                        return;
                    } else if is_css_name(event.name) {
                        wake = true;
                    }
                }
                if wake {
                    if tx.send(()).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
            }
        }
        None => {
            let mut last = read_at(&base);
            loop {
                std::thread::sleep(POLL);
                let now = read_at(&base);
                if now != last {
                    last = now;
                    if tx.send(()).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
            }
        }
    }
}

/// Watch both GTK config directories (the file may be created, rewritten in
/// place, or replaced by a rename). `None` when neither directory exists yet
/// or inotify is unavailable, so the caller polls instead.
fn watcher(base: &Path) -> Option<Inotify> {
    let mut inotify = Inotify::init().ok()?;
    let mask = WatchMask::CREATE
        | WatchMask::CLOSE_WRITE
        | WatchMask::MOVED_TO
        | WatchMask::DELETE
        | WatchMask::DELETE_SELF
        | WatchMask::MOVE_SELF;
    let watched = ["gtk-4.0", "gtk-3.0"]
        .iter()
        .filter(|dir| inotify.add_watch(base.join(dir), mask).is_ok())
        .count();
    (watched > 0).then_some(inotify)
}

/// Any `*.css` write in the watched directories matters: the palette file's
/// name depends on the tool that generated it.
fn is_css_name(name: Option<&std::ffi::OsStr>) -> bool {
    name.and_then(|n| n.to_str())
        .is_some_and(|n| n.to_ascii_lowercase().ends_with(".css"))
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
    use std::time::UNIX_EPOCH;

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

    /// A blue palette, to tell "newest wins" apart from "first name wins".
    const BLUE: &str = "\
@define-color accent_color #7aa2f7;
@define-color accent_bg_color #7aa2f7;
@define-color accent_fg_color #16161e;
@define-color window_bg_color #16161e;
@define-color window_fg_color #c0caf5;
@define-color card_bg_color #1f2335;
";

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pd-colors-{tag}-{}-{}",
            std::process::id(),
            UNIX_EPOCH.elapsed().unwrap().as_nanos()
        ));
        std::fs::create_dir_all(dir.join("gtk-4.0")).unwrap();
        dir
    }

    #[test]
    fn any_css_write_wakes_the_ui() {
        use std::ffi::OsStr;
        assert!(is_css_name(Some(OsStr::new("noctalia.css"))));
        assert!(is_css_name(Some(OsStr::new("gtk.css"))));
        assert!(is_css_name(Some(OsStr::new("end4.css"))));
        assert!(is_css_name(Some(OsStr::new("Colors.CSS"))));
        // Palette tools write through a temporary name and rename it.
        assert!(!is_css_name(Some(OsStr::new("gtk.css.tmp"))));
        assert!(!is_css_name(Some(OsStr::new("settings.ini"))));
        assert!(!is_css_name(None));
    }

    /// No filename is special: an import stub is skipped and any name that
    /// carries the tokens is read.
    #[test]
    fn finds_a_palette_whatever_the_tool_named_it() {
        let dir = temp_dir("named");
        std::fs::write(dir.join("gtk-4.0/gtk.css"), "@import url(\"end4.css\");\n").unwrap();
        std::fs::write(dir.join("gtk-4.0/end4.css"), TEAL).unwrap();
        assert_eq!(
            read_at(&dir).map(|c| c.accent),
            Some(Color32::from_rgb(0x84, 0xD2, 0xE6))
        );

        // GTK 3 is read when GTK 4 publishes no complete palette.
        std::fs::remove_file(dir.join("gtk-4.0/end4.css")).unwrap();
        std::fs::create_dir_all(dir.join("gtk-3.0")).unwrap();
        std::fs::write(dir.join("gtk-3.0/colors.css"), BLUE).unwrap();
        assert_eq!(
            read_at(&dir).map(|c| c.accent),
            Some(Color32::from_rgb(0x7A, 0xA2, 0xF7))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Switching palette tools leaves the old file behind; the one just
    /// generated must win.
    #[test]
    fn the_newest_palette_wins() {
        let dir = temp_dir("newest");
        let old = dir.join("gtk-4.0/noctalia.css");
        let new = dir.join("gtk-4.0/caelestia.css");
        std::fs::write(&old, TEAL).unwrap();
        std::fs::write(&new, BLUE).unwrap();
        std::fs::File::open(&old)
            .unwrap()
            .set_modified(UNIX_EPOCH + Duration::from_secs(1_000))
            .unwrap();
        std::fs::File::open(&new)
            .unwrap()
            .set_modified(UNIX_EPOCH + Duration::from_secs(2_000))
            .unwrap();
        assert_eq!(
            read_at(&dir).map(|c| c.accent),
            Some(Color32::from_rgb(0x7A, 0xA2, 0xF7))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The wallpaper path end to end: the desktop rewrites its CSS under
    /// whatever name it chose, and the watcher wakes the UI instead of
    /// waiting for the theme poll.
    #[test]
    fn a_wallpaper_write_wakes_the_watcher() {
        let dir = temp_dir("watch");
        let (tx, rx) = mpsc::channel();
        let watched = dir.clone();
        std::thread::spawn(move || run(watched, tx, eframe::egui::Context::default()));

        let file = dir.join("gtk-4.0/end4.css");
        // Retry: the watch may still be installing on the first write.
        for _ in 0..20 {
            std::fs::write(&file, TEAL).unwrap();
            if rx.recv_timeout(Duration::from_millis(250)).is_ok() {
                let _ = std::fs::remove_dir_all(&dir);
                return;
            }
        }
        panic!("rewriting a palette CSS should wake the watcher");
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
