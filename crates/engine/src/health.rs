//! Health file updates for GUI compatibility.

use proton_core::health::Health;
use std::fs;

pub fn state_dir() -> std::path::PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| std::path::Path::new("/").to_path_buf());
    std::env::var_os("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".local/state"))
        .join("proton-sync")
}

pub fn health_path() -> std::path::PathBuf {
    state_dir().join("health.json")
}

pub fn write_health(health: &Health) {
    let prev = proton_core::health::read(&health_path());
    let mut h = health.clone();
    h.ts = Some(chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%:z").to_string());
    // Stamp the build this engine was launched from (set by the unit), so the
    // GUI can restart a service that is still exec'ing an older AppImage.
    if h.build.is_none() {
        h.build = std::env::var("PROTON_GUI_VERSION").ok();
    }
    if h.started.is_none() {
        h.started = prev.started;
    }
    if h.interval.is_none() {
        h.interval = prev.interval;
    }
    if h.next_poll.is_none() && h.interval != Some(0) {
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
/// Schedule the next check at a short retry interval (used after a
/// login/keyring error at boot so the engine recovers as soon as the keyring
/// is unlocked instead of waiting the configured interval).
pub fn write_schedule_retry(interval: u64, retry_secs: u64) {
    let mut h = proton_core::health::read(&health_path());
    h.interval = Some(interval);
    h.next_poll = Some(
        (chrono::Local::now() + chrono::Duration::seconds(retry_secs as i64))
            .format("%Y-%m-%dT%H:%M:%S%:z")
            .to_string(),
    );
    write_health(&h);
}

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
