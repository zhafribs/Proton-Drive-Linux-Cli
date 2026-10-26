//! Health file updates for GUI compatibility.

use proton_core::health::Health;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn state_dir() -> std::path::PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_STATE_HOME") {
        std::path::PathBuf::from(xdg).join("proton-sync")
    } else {
        dirs::home_dir().unwrap_or_else(|| std::path::Path::new("/").to_path_buf()).join(".local/state/proton-sync")
    }
}

pub fn health_path() -> std::path::PathBuf {
    state_dir().join("health.json")
}

pub fn write_health(health: &Health) {
    let _ = fs::create_dir_all(state_dir());
    let tmp = health_path().with_extension("json.tmp");
    if let Ok(s) = serde_json::to_string_pretty(health) {
        let _ = fs::write(&tmp, s);
        let _ = fs::rename(&tmp, health_path());
    }
}
