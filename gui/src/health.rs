//! `health.json` - the engine's periodic snapshot - and the status mapping
//! that drives the Home orb, the tray dot and the notifications.

use crate::util::rel_time;
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct Health {
    pub status: String,
    pub error: Option<String>,
    pub ts: Option<String>,
    pub next_poll: Option<String>,
    pub uploaded: u64,
    pub downloaded: u64,
    pub actions: u64,
    pub holds: u64,
    pub mode: Option<String>,
    pub remote_entries: Option<u64>,
    pub local_entries: Option<u64>,
    pub state_records: Option<u64>,
    pub interval: Option<u64>,
}

pub fn read(path: &Path) -> Health {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

impl Health {
    pub fn ts_dt(&self) -> Option<chrono::DateTime<chrono::FixedOffset>> {
        self.ts.as_deref().and_then(crate::util::parse_iso)
    }

    pub fn next_poll_dt(&self) -> Option<chrono::DateTime<chrono::FixedOffset>> {
        self.next_poll.as_deref().and_then(crate::util::parse_iso)
    }
}

/// The states the Home orb cycles through.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UiStatus {
    Synced,
    Syncing,
    Paused,
    /// The stored session is gone - only a fresh sign-in brings syncing back.
    NeedsLogin,
    Error,
    Unknown,
}

impl UiStatus {
    pub fn key(self) -> &'static str {
        match self {
            UiStatus::Synced => "synced",
            UiStatus::Syncing => "syncing",
            UiStatus::Paused => "paused",
            UiStatus::NeedsLogin => "needs_login",
            UiStatus::Error => "error",
            UiStatus::Unknown => "unknown",
        }
    }
}

/// Service active + engine-reported status -> one UI status.
pub fn map_status(active: bool, hstatus: &str) -> UiStatus {
    if !active {
        UiStatus::Paused
    } else {
        match hstatus {
            "ok" => UiStatus::Synced,
            "syncing" => UiStatus::Syncing,
            "needs_login" => UiStatus::NeedsLogin,
            "error" => UiStatus::Error,
            _ => UiStatus::Unknown,
        }
    }
}

/// (title, subtitle-template) per status - verbatim PyQt strings.
pub fn status_text(status: UiStatus) -> (&'static str, &'static str) {
    match status {
        UiStatus::Synced => ("You're up to date", ""),
        UiStatus::Syncing => ("Syncing your files...", "Just a moment..."),
        UiStatus::Paused => ("Sync is paused", "Nothing is syncing. Your files are safe."),
        UiStatus::NeedsLogin => ("Your session expired", "Sign in again to resume syncing."),
        UiStatus::Error => ("Couldn't sync", "{error}"),
        UiStatus::Unknown => ("Checking your files...", ""),
    }
}

/// "Last checked just now · Next check in 42s" - any half available is fine.
pub fn synced_line(h: &Health) -> String {
    let last = rel_time(h.ts_dt());
    // interval 0 means no scheduled checks, so there is no countdown to show -
    // just the last check, exactly like a normal cycle with no next time.
    let nxt = if h.interval == Some(0) {
        None
    } else {
        // Prefer the engine's own next_poll timestamp; fall back to the
        // interval. A stale/past next_poll clamps to 0 rather than panicking.
        h.next_poll_dt()
            .map(|dt| {
                let secs = (dt - chrono::Utc::now().fixed_offset()).num_seconds();
                secs.max(0) as u64
            })
            .or(h.interval)
    };
    match (last.is_empty(), nxt) {
        (false, Some(n)) => format!("Last checked {last} · Next check in {n}s"),
        (false, None) => format!("Last checked {last}"),
        (true, Some(n)) => format!("Next check in {n}s"),
        (true, None) => "Waiting for the first check".to_string(),
    }
}

/// The subtitle under the hero title, per status.
pub fn hero_sub(status: UiStatus, h: &Health) -> String {
    let (_, sub) = status_text(status);
    match status {
        UiStatus::Synced => synced_line(h),
        UiStatus::Error => h
            .error
            .clone()
            .unwrap_or_else(|| "See the Activity page for details.".into()),
        _ => sub.replace("{error}", h.error.as_deref().unwrap_or("")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verbatim engine output shape - a single field type mismatch used to
    /// silently fail the whole parse and blank the Home orb.
    #[test]
    fn parses_live_health_json() {
        let json = r#"{
            "status": "ok",
            "started": "2026-10-20T00:29:25+08:00",
            "ts": "2026-10-20T00:30:02+08:00",
            "cycle_seconds": 36.5,
            "actions": 0,
            "holds": 270,
            "failed": 0,
            "uploaded": 0,
            "downloaded": 0,
            "conflicts": 0,
            "remote_entries": 270,
            "local_entries": 270,
            "state_records": 270,
            "interval": 60,
            "error": "",
            "next_poll": "2026-10-20T00:31:02+08:00",
            "mode": "full"
        }"#;
        let h: Health = serde_json::from_str(json).unwrap();
        assert_eq!(h.status, "ok");
        assert_eq!(h.holds, 270);
        assert_eq!(h.interval, Some(60));
        assert!(h.next_poll_dt().is_some());
        assert!(h.ts_dt().is_some());
        assert_eq!(map_status(true, &h.status), UiStatus::Synced);
    }

    /// Interval 0 is "Off": no scheduled check, so no countdown - but the
    /// last-check time still shows normally.
    #[test]
    fn manual_interval_has_no_countdown() {
        let h: Health = serde_json::from_str(
            r#"{"status":"ok","ts":"2026-10-20T00:30:02+08:00","interval":0}"#,
        )
        .unwrap();
        let line = synced_line(&h);
        assert!(line.starts_with("Last checked"));
        assert!(!line.contains("Next check"));
        let empty: Health = serde_json::from_str(r#"{"interval":0}"#).unwrap();
        assert_eq!(synced_line(&empty), "Waiting for the first check");
    }

    /// A lost keyring session is its own state: the engine is running fine,
    /// but nothing can be synced until the user signs in again.
    #[test]
    fn needs_login_is_its_own_status() {
        assert_eq!(map_status(true, "needs_login"), UiStatus::NeedsLogin);
        // a stopped service still reads as paused, not as a lost session
        assert_eq!(map_status(false, "needs_login"), UiStatus::Paused);
        let (title, sub) = status_text(UiStatus::NeedsLogin);
        assert_eq!(title, "Your session expired");
        assert!(sub.contains("Sign in again"));
        assert_eq!(UiStatus::NeedsLogin.key(), "needs_login");
    }
}
