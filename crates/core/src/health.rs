//! `health.json` - the engine's periodic telemetry snapshot.
//!
//! The engine merge-updates this file, so unknown keys are kept in `extra`
//! rather than being dropped on a read/write round-trip. Every field is
//! optional-tolerant: a partial file must never blank the GUI's status orb.

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct Health {
    pub status: String,
    pub started: Option<String>,
    pub error: Option<String>,
    pub ts: Option<String>,
    pub next_poll: Option<String>,
    pub phase: Option<String>,
    pub phase_seconds: Option<f64>,
    pub cycle_seconds: Option<f64>,
    pub cycle_reason: Option<String>,
    pub mode: Option<String>,
    pub uploaded: u64,
    pub downloaded: u64,
    pub actions: u64,
    pub holds: u64,
    pub failed: u64,
    pub conflicts: u64,
    pub remote_entries: Option<u64>,
    pub local_entries: Option<u64>,
    pub state_records: Option<u64>,
    pub interval: Option<u64>,
    pub cache_age: Option<u64>,
    pub audit_pending: Option<bool>,
    pub audit_done: Option<u64>,
    pub audit_total: Option<u64>,
    pub audit_phase: Option<String>,
    pub audit_seconds: Option<f64>,
    pub audit_error: Option<String>,
    pub last_audit: Option<String>,
    pub effective_workers: Option<u64>,
    pub full_scan_seconds: Option<f64>,
    pub transfer_active: Option<bool>,
    pub transfer_done: Option<u64>,
    pub transfer_total: Option<u64>,
    pub transfer_bytes: Option<u64>,
    /// Recently reported transfer paths, `"up:rel"` / `"down:rel"`, newest
    /// first. The GUI shows them under the progress bar while active.
    pub queue: Vec<String>,

    /// AppImage/GUI build the engine was launched from (`PROTON_GUI_VERSION`).
    /// The GUI uses this to notice a stale engine after an in-app update.
    pub build: Option<String>,

    /// Fields a newer engine wrote that this build does not know yet.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

/// Parse a health file, returning defaults for a missing or malformed one.
pub fn read(path: &Path) -> Health {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// CLI errors that mean the keyring session is gone or locked, rather than a
/// transient network/filesystem failure. Used by the engine (to write
/// `needs_login`) and the GUI (to show the padlock instead of the welcome).
pub fn is_login_error(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    m.contains("you need to login")
        || m.contains("login first")
        || m.contains("unauthorized")
        || m.contains("401")
        || m.contains("keyring")
}

impl Health {
    pub fn ts_dt(&self) -> Option<DateTime<FixedOffset>> {
        self.ts.as_deref().and_then(crate::time::parse_iso)
    }

    pub fn next_poll_dt(&self) -> Option<DateTime<FixedOffset>> {
        self.next_poll.as_deref().and_then(crate::time::parse_iso)
    }

    pub fn last_audit_dt(&self) -> Option<DateTime<FixedOffset>> {
        self.last_audit.as_deref().and_then(crate::time::parse_iso)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real shape written by the Python engine, including nulls and the
    /// telemetry fields the GUI does not display.
    #[test]
    fn parses_live_telemetry() {
        let json = r#"{
            "status": "ok",
            "started": "2026-10-26T20:57:25+08:00",
            "ts": "2026-10-26T20:57:25+08:00",
            "phase": "plan",
            "audit_done": 218,
            "audit_total": 218,
            "audit_pending": false,
            "cycle_reason": "local",
            "full_scan_seconds": 106.4,
            "phase_seconds": 0.0,
            "cycle_seconds": 0.0,
            "actions": 0,
            "holds": 1257,
            "failed": 0,
            "uploaded": 0,
            "downloaded": 0,
            "conflicts": 0,
            "remote_entries": 1257,
            "local_entries": 1257,
            "state_records": 1257,
            "interval": 0,
            "error": "",
            "mode": "fast",
            "audit_phase": "idle",
            "audit_error": "",
            "cache_age": null,
            "transfer_active": false,
            "effective_workers": 8,
            "next_poll": null,
            "transfer_done": 1,
            "transfer_total": 1,
            "transfer_bytes": 11803,
            "audit_seconds": 160.9,
            "last_audit": "2026-10-26T20:57:25+08:00"
        }"#;
        let h: Health = serde_json::from_str(json).unwrap();
        assert_eq!(h.status, "ok");
        assert_eq!(h.holds, 1257);
        assert_eq!(h.audit_pending, Some(false));
        assert_eq!(h.interval, Some(0));
        assert_eq!(h.mode.as_deref(), Some("fast"));
        assert_eq!(h.transfer_bytes, Some(11803));
        assert!(h.cache_age.is_none());
        assert!(h.ts_dt().is_some());
        assert!(h.next_poll_dt().is_none());
        assert!(h.last_audit_dt().is_some());
        assert!(h.extra.is_empty());
    }

    /// A newer engine's extra fields must survive a read/write round-trip.
    #[test]
    fn preserves_unknown_fields() {
        let h: Health =
            serde_json::from_str(r#"{"status":"ok","future_metric":7,"nested":{"a":1}}"#).unwrap();
        assert_eq!(h.extra["future_metric"], serde_json::json!(7));
        let back = serde_json::to_value(&h).unwrap();
        assert_eq!(back["future_metric"], serde_json::json!(7));
        assert_eq!(back["nested"]["a"], serde_json::json!(1));
    }

    #[test]
    fn missing_file_is_default() {
        let path = std::env::temp_dir().join(format!("pdhealth-missing-{}", std::process::id()));
        std::fs::remove_file(&path).ok();
        let h = read(&path);
        assert_eq!(h.status, "");
        assert_eq!(h.holds, 0);
    }

    #[test]
    fn recognises_lost_session_errors() {
        assert!(is_login_error("You need to login first"));
        assert!(is_login_error("keyring unlock failed"));
        assert!(is_login_error("HTTP 401 Unauthorized"));
        assert!(!is_login_error("network is unreachable"));
    }
}
