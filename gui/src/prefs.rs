//! App preferences - drop-in replacement for the PyQt app's QSettings file
//! (`~/.config/ProtonDriveCLI/ProtonDriveGUI.conf`, INI, `[General]`).
//!
//! Reading the Qt format in place means an upgrading user keeps their theme,
//! tray and notification choices. We write the same simple `key=value` shape
//! Qt reads back (booleans as `true`/`false`, window size as `WxH`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub enum Theme {
    System,
    Light,
    Dark,
}

impl Theme {
    pub fn as_str(&self) -> &'static str {
        match self {
            Theme::System => "system",
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }
    pub fn from_str(s: &str) -> Theme {
        match s {
            "light" => Theme::Light,
            "dark" => Theme::Dark,
            _ => Theme::System,
        }
    }
}

/// Interface scale: how big the whole UI (widgets *and* type) is drawn.
///
/// egui draws text at fixed point sizes with its own bundled font, so a
/// 15" laptop panel shows the same 14 pt labels as a 27" desktop - there is
/// no OS/browser text-size setting to follow. This is the user's knob for
/// that: it scales the UI as a whole, like a browser zoom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UiScale {
    Compact,
    Default,
    Large,
}

impl UiScale {
    pub fn as_str(&self) -> &'static str {
        match self {
            UiScale::Compact => "compact",
            UiScale::Default => "default",
            UiScale::Large => "large",
        }
    }
    pub fn from_str(s: &str) -> UiScale {
        match s {
            "compact" => UiScale::Compact,
            "large" => UiScale::Large,
            _ => UiScale::Default,
        }
    }
    /// egui zoom factor: 0.85 / 1.0 / 1.1.
    pub fn factor(&self) -> f32 {
        match self {
            UiScale::Compact => 0.85,
            UiScale::Default => 1.0,
            UiScale::Large => 1.1,
        }
    }
}

pub struct Prefs {
    pub file: PathBuf,
    pub values: BTreeMap<String, String>,
    pub window_w: u32,
    pub window_h: u32,
}

impl Prefs {
    pub fn load(file: &Path) -> Prefs {
        let mut values = BTreeMap::new();
        if let Ok(text) = std::fs::read_to_string(file) {
            for line in text.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('[') || line.starts_with('#') {
                    continue;
                }
                if let Some((k, v)) = line.split_once('=') {
                    values.insert(k.trim().to_string(), unquote(v.trim()));
                }
            }
        }
        let (window_w, window_h) = values
            .get("window_size")
            .and_then(|s| s.split_once('x'))
            .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
            // Qt: a remembered size counts from 700x480, then gets floored to
            // the no-scroll layout so old saves never come back clipped.
            .filter(|(w, h)| *w >= 700 && *h >= 480)
            .unwrap_or((940, 780));
        Prefs {
            file: file.to_path_buf(),
            values,
            window_w: window_w.max(920),
            window_h: window_h.max(780),
        }
    }

    fn bool_of(&self, key: &str, default: bool) -> bool {
        match self.values.get(key).map(String::as_str) {
            Some("true") => true,
            Some("false") => false,
            _ => default,
        }
    }

    pub fn theme(&self) -> Theme {
        Theme::from_str(self.values.get("theme").map(String::as_str).unwrap_or("system"))
    }
    pub fn ui_scale(&self) -> UiScale {
        UiScale::from_str(self.values.get("ui_scale").map(String::as_str).unwrap_or("default"))
    }
    pub fn start_in_tray(&self) -> bool {
        self.bool_of("start_in_tray", false)
    }
    pub fn close_to_tray(&self) -> bool {
        self.bool_of("close_to_tray", true)
    }
    pub fn always_on_top(&self) -> bool {
        self.bool_of("always_on_top", true)
    }
    pub fn updates_auto(&self) -> bool {
        self.bool_of("updates_auto", true)
    }
    pub fn notify_errors(&self) -> bool {
        self.bool_of("notify_errors", true)
    }
    pub fn notify_conflicts(&self) -> bool {
        self.bool_of("notify_conflicts", true)
    }
    pub fn notify_complete(&self) -> bool {
        self.bool_of("notify_complete", false)
    }

    pub fn set(&mut self, key: &str, value: impl Into<String>) {
        self.values.insert(key.to_string(), value.into());
        self.save();
    }
    pub fn set_bool(&mut self, key: &str, value: bool) {
        self.set(key, if value { "true" } else { "false" });
    }
    pub fn set_window_size(&mut self, w: u32, h: u32) {
        // Raw size, like Qt - restoring applies the floor.
        self.window_w = w.max(1);
        self.window_h = h.max(1);
        self.set("window_size", format!("{}x{}", self.window_w, self.window_h));
    }

    fn save(&self) {
        let mut out = String::from("[General]\n");
        for (k, v) in &self.values {
            out.push_str(&format!("{k}={v}\n"));
        }
        let _ = std::fs::create_dir_all(self.file.parent().unwrap_or(Path::new(".")));
        let _ = std::fs::write(&self.file, out);
    }
}

fn unquote(v: &str) -> String {
    let v = v.trim();
    v.strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(v)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qt_settings_format_roundtrips() {
        let p = std::env::temp_dir().join(format!("pdprefs-{}.conf", std::process::id()));
        // what a previous PyQt install left behind
        std::fs::write(
            &p,
            "[General]\ntheme=dark\nstart_in_tray=true\nclose_to_tray=false\nwindow_size=1000x700\n",
        )
        .unwrap();
        let mut pr = Prefs::load(&p);
        assert_eq!(pr.theme(), Theme::Dark);
        assert!(pr.start_in_tray());
        assert!(!pr.close_to_tray());
        // 1000x700 accepted, height floored to the no-scroll layout
        assert_eq!((pr.window_w, pr.window_h), (1000, 780));
        pr.set_bool("notify_complete", true);
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.starts_with("[General]\n"));
        assert!(text.contains("theme=dark")); // untouched keys survive
        assert!(text.contains("notify_complete=true"));
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn defaults_when_missing_or_below_floor() {
        let p = std::env::temp_dir().join(format!("pdprefs2-{}.conf", std::process::id()));
        std::fs::write(&p, "[General]\nwindow_size=100x50\n").unwrap();
        let pr = Prefs::load(&p);
        // below the 700x480 acceptance threshold -> the default size
        assert_eq!((pr.window_w, pr.window_h), (940, 780));
        assert!(pr.close_to_tray());
        assert_eq!(pr.theme(), Theme::System);
        // a small-but-accepted size gets floored to the no-scroll layout
        std::fs::write(&p, "[General]\nwindow_size=1000x700\n").unwrap();
        let pr = Prefs::load(&p);
        assert_eq!((pr.window_w, pr.window_h), (1000, 780));
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn interface_scale_prefs_roundtrip() {
        let p = std::env::temp_dir().join(format!("pdprefs3-{}.conf", std::process::id()));
        std::fs::write(&p, "[General]\ntheme=dark\n").unwrap();
        // no key yet -> the default size
        let pr = Prefs::load(&p);
        assert_eq!(pr.ui_scale(), UiScale::Default);
        assert_eq!(pr.ui_scale().factor(), 1.0);
        // a saved choice survives a reload, unknown values fall back
        let mut pr = pr;
        pr.set("ui_scale", "compact");
        assert_eq!(Prefs::load(&p).ui_scale(), UiScale::Compact);
        assert!(Prefs::load(&p).ui_scale().factor() < 1.0);
        pr.set("ui_scale", "gigantic");
        assert_eq!(Prefs::load(&p).ui_scale(), UiScale::Default);
        std::fs::remove_file(&p).ok();
    }
}
