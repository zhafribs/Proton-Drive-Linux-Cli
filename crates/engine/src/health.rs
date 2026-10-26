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
    let prev = proton_core::health::read(&health_path());
    let mut h = health.clone();
    h.ts = Some(chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%:z").to_string());
    if h.interval.is_none() {
        h.interval = prev.interval;
    }
    if h.next_poll.is_none() {
        h.next_poll = prev.next_poll;
    }
    let _ = fs::create_dir_all(state_dir());
    let tmp = health_path().with_extension("json.tmp");
    if let Ok(s) = serde_json::to_string_pretty(&h) {
        let _ = fs::write(&tmp, s);
        let _ = fs::rename(&tmp, health_path());
    }
}

/// Record the schedule after a cycle: the interval and when the next check is due.
pub fn write_schedule(interval: u64) {
    let mut h = proton_core::health::read(&health_path());
    h.interval = Some(interval);
    h.next_poll = (interval > 0).then(|| {
        (chrono::Local::now() + chrono::Duration::seconds(interval as i64))
            .format("%Y-%m-%dT%H:%M:%S%:z")
            .to_string()
    });
    write_health(&h);
}
