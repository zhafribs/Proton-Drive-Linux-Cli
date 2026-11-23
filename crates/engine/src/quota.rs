//! Read-only account storage quota for the Home storage card.
//!
//! The official CLI has no quota command, but the engine already reads the
//! CLI's own session for the event stream. We use the same read-only session
//! to ask Proton's account endpoint for `UsedSpace`/`MaxSpace`; no writes,
//! no auth duplication, and no tokens are logged.

use crate::events::{API, Session, read_cli_session};
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
    /// The Drive file volume's used space, in bytes.
    pub drive_used: u64,
    /// The Photos volume's used space, in bytes (part of the Drive product).
    #[serde(default)]
    pub photos_used: u64,
    /// ISO timestamp of the fetch.
    pub fetched_at: String,
}

pub fn path() -> PathBuf {
    crate::health::state_dir().join("quota.json")
}

pub fn start() {
    std::thread::spawn(|| loop {
        let delay = match fetch() {
            // The web refreshes storage on every visit; ten minutes keeps the
            // Home card close enough without polling the account endpoint.
            Some(quota) if write(&quota).is_ok() => Duration::from_secs(600),
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
    let users = get_json(&agent, &session, "/core/v4/users")?;
    let mut quota = parse(&users)?;
    // Proton keeps Photos in a volume of its own next to the file volume, but
    // `ProductUsedSpace.Drive` is their sum. Splitting them makes the card
    // tally with what the Drive folder and the Photos tab each hold.
    if let Some(volumes) = get_json(&agent, &session, "/drive/volumes") {
        split(&mut quota, &users, &volumes);
    }
    Some(quota)
}

fn get_json(agent: &ureq::Agent, session: &Session, path: &str) -> Option<Value> {
    let res = agent
        .get(&format!("{API}{path}"))
        .header("Authorization", format!("Bearer {}", session.access_token))
        .header("x-pm-uid", &session.uid)
        .header("x-pm-appversion", "Other")
        .header("Accept", "application/json")
        .call()
        .ok()?;
    let body = res.into_body().read_to_string().ok()?;
    serde_json::from_str(&body).ok()
}

fn parse(value: &Value) -> Option<Quota> {
    let user = value.get("User").unwrap_or(value);
    let used = user.get("UsedSpace").and_then(Value::as_u64)?;
    let total = user.get("MaxSpace").and_then(Value::as_u64)?;
    let drive_used = product_drive(value).unwrap_or(used);
    Some(Quota {
        used,
        total,
        drive_used,
        photos_used: 0,
        fetched_at: chrono::Local::now()
            .format("%Y-%m-%dT%H:%M:%S%:z")
            .to_string(),
    })
}

/// The Drive product total across its volumes, in bytes.
fn product_drive(value: &Value) -> Option<u64> {
    value
        .get("User")
        .unwrap_or(value)
        .get("ProductUsedSpace")
        .and_then(|p| p.get("Drive"))
        .and_then(Value::as_u64)
}

/// The file volume (type 1). Photos is a second volume (type 2) listed only
/// to web clients, so it is derived as the product total minus this one.
fn volume_used(value: &Value) -> Option<u64> {
    value
        .get("Volumes")
        .and_then(Value::as_array)?
        .iter()
        .find(|v| v.get("Type").and_then(Value::as_u64) == Some(1))?
        .get("UsedSpace")
        .and_then(Value::as_u64)
}

/// Split the product total into the file volume and the Photos remainder.
/// Called only when `ProductUsedSpace.Drive` exists, so Mail is never counted
/// as photos.
fn split(quota: &mut Quota, users: &Value, volumes: &Value) {
    let (Some(product), Some(volume)) = (product_drive(users), volume_used(volumes)) else {
        return;
    };
    quota.photos_used = product.saturating_sub(volume);
    quota.drive_used = volume;
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
        assert_eq!(quota.photos_used, 0);
    }

    #[test]
    fn splits_the_file_volume_from_photos() {
        let users: Value = serde_json::from_str(
            r#"{"User":{"UsedSpace":2795127543,"MaxSpace":214748364800,
                "ProductUsedSpace":{"Drive":2786323210,"Mail":8804333}}}"#,
        )
        .unwrap();
        // `Other` clients see only the file volume; photos is the remainder.
        let volumes: Value = serde_json::from_str(
            r#"{"Volumes":[{"VolumeID":"a","Type":1,"UsedSpace":1401268671}]}"#,
        )
        .unwrap();
        let mut quota = parse(&users).unwrap();
        split(&mut quota, &users, &volumes);
        assert_eq!(quota.drive_used, 1_401_268_671);
        assert_eq!(quota.photos_used, 1_385_054_539);
        assert_eq!(quota.drive_used + quota.photos_used, 2_786_323_210);
    }

    #[test]
    fn a_missing_volume_keeps_the_product_total() {
        let users: Value = serde_json::from_str(
            r#"{"User":{"UsedSpace":10,"MaxSpace":100,"ProductUsedSpace":{"Drive":8}}}"#,
        )
        .unwrap();
        let mut quota = parse(&users).unwrap();
        split(&mut quota, &users, &serde_json::json!({"Volumes": []}));
        assert_eq!(quota.drive_used, 8);
        assert_eq!(quota.photos_used, 0);
    }

    #[test]
    fn falls_back_to_total_used_when_drive_is_missing() {
        let value: Value =
            serde_json::from_str(r#"{"UsedSpace":10,"MaxSpace":100}"#).unwrap();
        let quota = parse(&value).unwrap();
        assert_eq!(quota.drive_used, 10);
    }
}
