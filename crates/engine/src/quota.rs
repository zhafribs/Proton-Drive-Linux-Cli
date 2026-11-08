//! Read-only account storage quota for the Home storage card.
//!
//! The official CLI has no quota command, but the engine already reads the
//! CLI's own session for the event stream. We use the same read-only session
//! to ask Proton's account endpoint for `UsedSpace`/`MaxSpace`; no writes,
//! no auth duplication, and no tokens are logged.

use crate::events::{API, read_cli_session};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Quota {
    /// Total account storage used, in bytes (shared across Proton apps).
    pub used: u64,
    /// Account storage limit, in bytes.
    pub total: u64,
    /// The Drive product's share of `used`, in bytes.
    pub drive_used: u64,
    /// ISO timestamp of the fetch.
    pub fetched_at: String,
}

pub fn path() -> PathBuf {
    crate::health::state_dir().join("quota.json")
}

pub fn start() {
    std::thread::spawn(|| loop {
        let delay = match fetch() {
            Some(quota) if write(&quota).is_ok() => Duration::from_secs(3600),
            _ => Duration::from_secs(300),
        };
        std::thread::sleep(delay);
    });
}

fn fetch() -> Option<Quota> {
    let session = read_cli_session()?;
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .build();
    let agent = ureq::Agent::new_with_config(agent);
    let res = agent
        .get(&format!("{API}/core/v4/users"))
        .header("Authorization", format!("Bearer {}", session.access_token))
        .header("x-pm-uid", &session.uid)
        .header("x-pm-appversion", "Other")
        .header("Accept", "application/json")
        .call()
        .ok()?;
    let body = res.into_body().read_to_string().ok()?;
    let value: Value = serde_json::from_str(&body).ok()?;
    parse(&value)
}

fn parse(value: &Value) -> Option<Quota> {
    let user = value.get("User").unwrap_or(value);
    let used = user.get("UsedSpace").and_then(Value::as_u64)?;
    let total = user.get("MaxSpace").and_then(Value::as_u64)?;
    let drive_used = user
        .get("ProductUsedSpace")
        .and_then(|p| p.get("Drive"))
        .and_then(Value::as_u64)
        .unwrap_or(used);
    Some(Quota {
        used,
        total,
        drive_used,
        fetched_at: chrono::Local::now()
            .format("%Y-%m-%dT%H:%M:%S%:z")
            .to_string(),
    })
}

fn write(quota: &Quota) -> io::Result<()> {
    let path = path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_vec(quota).map_err(io::Error::other)?;
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_account_response() {
        let value: Value = serde_json::from_str(
            r#"{"User":{"UsedSpace":1925769224,"MaxSpace":214748364800,
                "ProductUsedSpace":{"Drive":1917692050,"Mail":8077174}}}"#,
        )
        .unwrap();
        let quota = parse(&value).unwrap();
        assert_eq!(quota.used, 1_925_769_224);
        assert_eq!(quota.total, 214_748_364_800);
        assert_eq!(quota.drive_used, 1_917_692_050);
    }

    #[test]
    fn falls_back_to_total_used_when_drive_is_missing() {
        let value: Value =
            serde_json::from_str(r#"{"UsedSpace":10,"MaxSpace":100}"#).unwrap();
        let quota = parse(&value).unwrap();
        assert_eq!(quota.drive_used, 10);
    }
}
