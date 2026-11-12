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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UiScale {
    percent: u16,
}

impl UiScale {
    pub const MIN: u16 = 50;
    pub const MAX: u16 = 200;
    pub const DEFAULT: u16 = 100;

    pub fn from_percent(percent: u16) -> Self {
        Self {
            percent: percent.clamp(Self::MIN, Self::MAX),
        }
    }

    pub fn percent(&self) -> u16 {
        self.percent
    }

    pub fn as_str(&self) -> String {
        self.percent.to_string()
    }

    pub fn from_str(s: &str) -> UiScale {
        match s.trim() {
            "compact" => Self::from_percent(85),
            "large" => Self::from_percent(110),
            "default" => Self::from_percent(Self::DEFAULT),
            other => other
                .parse::<u16>()
                .map(Self::from_percent)
                .unwrap_or_else(|_| Self::from_percent(Self::DEFAULT)),
        }
    }

    /// egui zoom factor: 0.5..2.0.
    pub fn factor(&self) -> f32 {
        f32::from(self.percent) / 100.0
    }
}

impl Default for UiScale {
    fn default() -> Self {
        Self::from_percent(Self::DEFAULT)
    }
}

#[derive(Clone)]
pub struct Prefs {
    pub file: PathBuf,
    pub values: BTreeMap<String, String>,
    pub window_w: u32,
    pub window_h: u32,
    /// Sidebar width in points, remembered across restarts (150–400).
    pub nav_width: f32,
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
        // Window sizes are stored in physical pixels at the desktop scale,
        // i.e. the egui size multiplied by the interface zoom. Restoring them
        // unchanged keeps the same physical window when the zoom changes;
        // storing raw egui points would shrink the window on every restart at
        // a larger scale.
        let scale = UiScale::from_str(values.get("ui_scale").map(String::as_str).unwrap_or("default"));
        let factor = scale.factor();
        let min_w = (720.0 * factor).round() as u32;
        let min_h = (400.0 * factor).round() as u32;
        let (window_w, window_h) = values
            .get("window_size")
            .and_then(|s| s.split_once('x'))
            .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
            .map(|(w, h)| (w.max(min_w), h.max(min_h)))
            .unwrap_or((940.max(min_w), 780.max(min_h)));
        let nav = Self::nav_width(values.get("nav_width"));
        Prefs {
            file: file.to_path_buf(),
            values,
            window_w,
            window_h,
            nav_width: nav,
        }
    }

    pub fn set_nav_width(&mut self, w: f32) {
        self.nav_width = w.clamp(150.0, 400.0);
        self.set("nav_width", format!("{:.0}", self.nav_width));
    }

    fn bool_of(&self, key: &str, default: bool) -> bool {
        match self.values.get(key).map(String::as_str) {
            Some("true") => true,
            Some("false") => false,
            _ => default,
        }
    }

    fn nav_width(raw: Option<&String>) -> f32 {
        raw.and_then(|v| v.trim().parse::<f32>().ok())
            .unwrap_or(200.0)
            .clamp(150.0, 400.0)
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
    pub fn onboarding_done(&self) -> bool {
        self.bool_of("onboarding_done", false)
    }
    pub fn language(&self) -> crate::i18n::Language {
        crate::i18n::Language::from_str(
            self.values.get("language").map(String::as_str).unwrap_or("en"),
        )
    }

    /// Animate status changes / request continuous repaints.
    pub fn motion_enabled(&self) -> bool {
        self.bool_of("motion", true)
            && std::env::var("PROTON_UI_MOTION").ok().as_deref() != Some("0")
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

    pub fn save(&self) {
        let mut out = String::from("[General]\n");
        for (k, v) in &self.values {
            out.push_str(&format!("{k}={v}\n"));
        }
        let _ = std::fs::create_dir_all(self.file.parent().unwrap_or(Path::new(".")));
        let _ = std::fs::write(&self.file, out);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(
                &self.file,
                std::fs::Permissions::from_mode(0o600),
            );
        }
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
        // 1000x700 is accepted as-is; it is above the 720x540 floor.
        assert_eq!((pr.window_w, pr.window_h), (1000, 700));
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
        // a bogus tiny save is clamped to the layout floor, not discarded
        assert_eq!((pr.window_w, pr.window_h), (720, 400));
        assert!(pr.close_to_tray());
        assert_eq!(pr.theme(), Theme::System);
        // a small-but-accepted size is kept, down to the layout floor
        std::fs::write(&p, "[General]\nwindow_size=1000x700\n").unwrap();
        let pr = Prefs::load(&p);
        assert_eq!((pr.window_w, pr.window_h), (1000, 700));
        std::fs::write(&p, "[General]\nwindow_size=720x540\n").unwrap();
        let pr = Prefs::load(&p);
        assert_eq!((pr.window_w, pr.window_h), (720, 540));
        // Below the floor still clamps up, so old tiny saves stay usable.
        std::fs::write(&p, "[General]\nwindow_size=710x500\n").unwrap();
        let pr = Prefs::load(&p);
        assert_eq!((pr.window_w, pr.window_h), (720, 500));
        // The floor follows the interface scale: 200% needs twice the pixels.
        std::fs::write(&p, "[General]\nui_scale=200\nwindow_size=700x500\n").unwrap();
        let pr = Prefs::load(&p);
        assert_eq!((pr.window_w, pr.window_h), (1440, 800));
        // 50% keeps a deliberately small physical window instead of rejecting it.
        std::fs::write(&p, "[General]\nui_scale=50\nwindow_size=360x200\n").unwrap();
        let pr = Prefs::load(&p);
        assert_eq!((pr.window_w, pr.window_h), (360, 200));
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn interface_scale_prefs_roundtrip() {
        let p = std::env::temp_dir().join(format!("pdprefs3-{}.conf", std::process::id()));
        std::fs::write(&p, "[General]\ntheme=dark\n").unwrap();
        // no key yet -> the default size
        let pr = Prefs::load(&p);
        assert_eq!(pr.ui_scale(), UiScale::default());
        assert_eq!(pr.ui_scale().percent(), 100);
        assert_eq!(pr.ui_scale().factor(), 1.0);
        // Saved percentages survive; legacy names migrate.
        let mut pr = pr;
        pr.set("ui_scale", "compact");
        assert_eq!(Prefs::load(&p).ui_scale().percent(), 85);
        pr.set("ui_scale", "112");
        assert_eq!(Prefs::load(&p).ui_scale().percent(), 112);
        assert!((Prefs::load(&p).ui_scale().factor() - 1.12).abs() < f32::EPSILON);
        pr.set("ui_scale", "10");
        assert_eq!(Prefs::load(&p).ui_scale().percent(), UiScale::MIN);
        pr.set("ui_scale", "999");
        assert_eq!(Prefs::load(&p).ui_scale().percent(), UiScale::MAX);
        pr.set("ui_scale", "gigantic");
        assert_eq!(Prefs::load(&p).ui_scale(), UiScale::default());
        std::fs::remove_file(&p).ok();
    }
    #[test]
    fn nav_width_is_parsed_and_clamped() {
        let p = std::env::temp_dir().join(format!("pdprefs-nw-{}.conf", std::process::id()));
        std::fs::write(&p, "[General]\nnav_width=300\n").unwrap();
        assert_eq!(Prefs::load(&p).nav_width, 300.0);
        std::fs::write(&p, "[General]\nnav_width=900\n").unwrap();
        assert_eq!(Prefs::load(&p).nav_width, 400.0);
        std::fs::write(&p, "[General]\nnav_width=abc\n").unwrap();
        assert_eq!(Prefs::load(&p).nav_width, 200.0);
        std::fs::write(&p, "[General]\n").unwrap();
        assert_eq!(Prefs::load(&p).nav_width, 200.0);
        std::fs::remove_file(&p).ok();
    }

}
