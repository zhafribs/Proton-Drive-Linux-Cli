//! Proton Drive event-stream listener.
//!
//! The official CLI has no watch command, but it keeps a per-volume cursor in
//! `~/.local/share/proton-drive-cli/events.json` and its auth session in the
//! Secret Service. We subscribe to the same Drive event stream with that
//! session (tokens stay in memory), share the cursor file atomically, and
//! record each changed node's parent folder. The sync loop then re-lists only
//! those folders instead of walking the whole tree.
//!
//! If any startup step fails the caller falls back to the fingerprint watcher
//! and the configured interval; this module never takes the engine down.

use crate::cli::{self, Cli};
use serde::Deserialize;
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Type, Value};

const API: &str = "https://drive-api.proton.me";
const SECRET_LABEL: &str = "ch.proton.drive/drive-sdk-cli/auth-session";
const IDLE_POLL: f64 = 1.0;

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct PendingEvent {
    pub parent: String,
    pub child: String,
    pub event_type: i32,
    pub trashed: bool,
}

pub struct EventWatch {
    pending: Arc<Mutex<BTreeSet<PendingEvent>>>,
    pub volume: String,
    pub root_uid: String,
}

impl EventWatch {
    /// Take the parent LinkIDs collected since the last cycle and clear the
    /// wake flag under the same lock, so an event cannot be lost between them.
    pub fn take(&self, wake: &AtomicBool) -> BTreeSet<PendingEvent> {
        let mut pending = self.pending.lock().unwrap();
        let out = std::mem::take(&mut *pending);
        wake.store(false, Ordering::Relaxed);
        out
    }
}

#[derive(Debug, Clone)]
struct Session {
    access_token: String,
    uid: String,
}

#[allow(dead_code)] // D-Bus tuple layout; only the value bytes are read
#[derive(Deserialize, Type)]
struct Secret(OwnedObjectPath, Vec<u8>, Vec<u8>, String);

/// Start the event listener. Returns None when the session, cursor or remote
/// info is unavailable; the caller then keeps the fingerprint/interval path.
pub fn start(
    cli: Cli,
    remote_root: String,
    wake: Arc<AtomicBool>,
    wake_event: Arc<Condvar>,
    wake_mutex: Arc<Mutex<()>>,
) -> Option<EventWatch> {
    if !cli::cfg_flag("PROTON_SYNC_PUSH", true) {
        return None;
    }
    let info = cli
        .run_json(&["fs", "info", &remote_root], true)
        .ok()?;
    let root_uid = info.get("uid")?.as_str()?.to_string();
    let volume = info
        .get("treeEventScopeId")
        .and_then(|v| v.as_str())
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .or_else(|| root_uid.split('~').next().map(str::to_string))?;
    let cursor = cursor_read(&volume)?;
    let session = read_cli_session()?;
    let pending = Arc::new(Mutex::new(BTreeSet::new()));

    {
        let pending = pending.clone();
        let wake = wake.clone();
        let wake_event = wake_event.clone();
        let wake_mutex = wake_mutex.clone();
        let volume = volume.clone();
        std::thread::spawn(move || {
            run(
                session,
                volume,
                cursor,
                pending,
                wake,
                wake_event,
                wake_mutex,
            );
        });
    }
    eprintln!("push: subscribed to Proton volume {volume}");
    Some(EventWatch {
        pending,
        volume,
        root_uid,
    })
}

/// What the loop needs to turn parent LinkIDs into folder rel-paths.
pub struct PushContext<'a> {
    pub pending: &'a BTreeSet<PendingEvent>,
    pub volume: &'a str,
    pub root_uid: &'a str,
}

/// Group events by the snapshot path of their parent folder. Unknown
/// parents force a full/fallback scan because a new folder chain may not be
/// in the snapshot yet.
pub fn resolve_event_scopes(
    pending: &BTreeSet<PendingEvent>,
    tree: &proton_core::snapshot::RemoteTree,
    volume: &str,
    root_uid: &str,
) -> (Vec<(String, Vec<PendingEvent>)>, usize) {
    let root_link = root_uid.split('~').nth(1).unwrap_or("");
    let index: HashMap<&str, &str> = tree
        .iter()
        .filter_map(|(rel, node)| node.uid.as_deref().map(|uid| (uid, rel.as_str())))
        .collect();
    let mut groups: std::collections::BTreeMap<String, Vec<PendingEvent>> =
        std::collections::BTreeMap::new();
    let mut unknown = 0;
    for event in pending {
        let rel = if event.parent == root_link {
            Some(String::new())
        } else {
            let uid = format!("{volume}~{}", event.parent);
            index.get(uid.as_str()).map(|rel| (*rel).to_string())
        };
        match rel {
            Some(rel) => groups.entry(rel).or_default().push(event.clone()),
            None => unknown += 1,
        }
    }
    (groups.into_iter().collect(), unknown)
}

fn run(
    mut session: Session,
    volume: String,
    mut cursor: String,
    pending: Arc<Mutex<BTreeSet<PendingEvent>>>,
    wake: Arc<AtomicBool>,
    wake_event: Arc<Condvar>,
    wake_mutex: Arc<Mutex<()>>,
) {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .build();
    let agent = ureq::Agent::new_with_config(agent);
    let mut delay = 0.4f64;
    let mut auth_fails = 0u32;
    let mut stale_cursor = 0u32;
    let mut session_checked = Instant::now();

    loop {
        if session_checked.elapsed() > Duration::from_secs(60) {
            if let Some(fresh) = read_cli_session() {
                session = fresh;
            }
            session_checked = Instant::now();
        }

        let url = format!("{API}/drive/v2/volumes/{volume}/events/{cursor}");
        let started = Instant::now();
        let result = agent
            .get(&url)
            .header("Authorization", format!("Bearer {}", session.access_token))
            .header("x-pm-uid", &session.uid)
            .header("x-pm-appversion", "Other")
            .header("Accept", "application/json")
            .header("User-Agent", "proton-drive-sync")
            .call();

        let mut sleep_for = delay;
        match result {
            Ok(response) => match response.into_body().read_to_string() {
                Ok(body) => {
                    let value: serde_json::Value =
                        serde_json::from_str(&body).unwrap_or_default();
                    let events = value
                        .get("Events")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default();
                    if !events.is_empty() {
                        let pending_events: BTreeSet<PendingEvent> = events
                            .iter()
                            .filter_map(|event| {
                                let link = event.get("Link")?;
                                let parent = link
                                    .get("ParentLinkID")
                                    .and_then(|v| v.as_str())
                                    .filter(|v| !v.is_empty())?
                                    .to_string();
                                let child = link
                                    .get("LinkID")
                                    .and_then(|v| v.as_str())
                                    .filter(|v| !v.is_empty())?
                                    .to_string();
                                let event_type = event
                                    .get("EventType")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(-1) as i32;
                                let trashed = link
                                    .get("IsTrashed")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(false);
                                Some(PendingEvent { parent, child, event_type, trashed })
                            })
                            .collect();
                        if !pending_events.is_empty() {
                            {
                                let mut guard = pending.lock().unwrap();
                                guard.extend(pending_events);
                            }
                            wake.store(true, Ordering::Relaxed);
                            let _guard = wake_mutex.lock().unwrap();
                            wake_event.notify_all();
                        }
                        eprintln!("push: {} event(s) received", events.len());
                        delay = 0.4;
                    } else {
                        delay = (delay * 1.6).min(IDLE_POLL);
                    }
                    if let Some(next) = value.get("EventID").and_then(|v| v.as_str())
                        && !next.is_empty()
                        && next != cursor
                    {
                        cursor = next.to_string();
                        cursor_write(&volume, &cursor);
                        delay = 0.4;
                    }
                    sleep_for = delay.max(started.elapsed().as_secs_f64());
                }
                Err(_) => {
                    delay = (delay * 2.0).min(30.0);
                    sleep_for = delay;
                }
            },
            Err(ureq::Error::StatusCode(400)) => {
                let disk = cursor_read(&volume);
                if let Some(disk) = disk
                    && disk != cursor
                {
                    eprintln!("push: cursor consumed elsewhere - re-adopting");
                    cursor = disk;
                    delay = 0.4;
                    stale_cursor = 0;
                } else {
                    stale_cursor += 1;
                    delay = (delay.max(0.8) * 2.0).min(30.0);
                    if stale_cursor == 10 {
                        eprintln!("push: cursor rejected repeatedly; waiting for the CLI");
                    } else if stale_cursor.is_multiple_of(30) {
                        sleep_for = 30.0;
                    }
                }
            }
            Err(ureq::Error::StatusCode(401)) => {
                auth_fails += 1;
                if let Some(fresh) = read_cli_session() {
                    session = fresh;
                    auth_fails = 0;
                }
                delay = (delay * 2.0).min(15.0);
                if auth_fails >= 6 {
                    eprintln!("push: session rejected repeatedly - push disabled");
                    return;
                }
                sleep_for = delay;
            }
            Err(ureq::Error::StatusCode(429)) => {
                delay = (delay.max(5.0) * 2.0).min(120.0);
                eprintln!("push: rate limited - backing off {:.0}s", delay);
                sleep_for = delay;
            }
            Err(_) => {
                delay = (delay * 2.0).min(30.0);
                sleep_for = delay;
            }
        }
        std::thread::sleep(Duration::from_secs_f64(sleep_for.max(0.1)));
    }
}

fn events_file() -> PathBuf {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".local/share")))
        .unwrap_or_else(|| PathBuf::from(".local/share"));
    data.join("proton-drive-cli/events.json")
}

fn cursor_read(volume: &str) -> Option<String> {
    let text = std::fs::read_to_string(events_file()).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value
        .get("drive")?
        .get(volume)?
        .get("lastEventId")?
        .as_str()
        .map(str::to_string)
}

/// Merge our advanced cursor into the CLI's file with an atomic replace, so
/// the CLI continues the same event chain.
fn cursor_write(volume: &str, id: &str) -> bool {
    let path = events_file();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return false;
    };
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return false;
    };
    let Some(drive) = value.get_mut("drive").and_then(|v| v.as_object_mut()) else {
        return false;
    };
    let entry = drive
        .entry(volume.to_string())
        .or_insert_with(|| serde_json::json!({}));
    let Some(entry) = entry.as_object_mut() else {
        return false;
    };
    entry.insert(
        "lastEventId".to_string(),
        serde_json::Value::String(id.to_string()),
    );
    let tmp = path.with_file_name("events.json.tmp");
    if std::fs::write(&tmp, serde_json::to_string_pretty(&value).unwrap_or_default()).is_err() {
        return false;
    }
    std::fs::rename(&tmp, &path).is_ok()
}

/// The official CLI's session from the Secret Service. Tokens are kept in
/// memory only and never logged.
fn read_cli_session() -> Option<Session> {
    let conn = Connection::session().ok()?;
    let service = Proxy::new(
        &conn,
        "org.freedesktop.secrets",
        "/org/freedesktop/secrets",
        "org.freedesktop.Secret.Service",
    )
    .ok()?;

    let (paths, _): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) = service
        .call("SearchItems", &HashMap::<String, String>::new())
        .ok()?;
    let mut item = None;
    for path in paths {
        let label: String = Proxy::new(
            &conn,
            "org.freedesktop.secrets",
            path.as_str(),
            "org.freedesktop.Secret.Item",
        )
        .ok()?
        .get_property("Label")
        .ok()?;
        if label == SECRET_LABEL
            || (label.contains("proton") && label.ends_with("/auth-session"))
        {
            item = Some(path);
            break;
        }
    }
    let item = item?;
    let (_, session_path): (OwnedValue, OwnedObjectPath) = service
        .call("OpenSession", &("plain", Value::from("")))
        .ok()?;
    let item_proxy = Proxy::new(
        &conn,
        "org.freedesktop.secrets",
        item.as_str(),
        "org.freedesktop.Secret.Item",
    )
    .ok()?;
    let (secret,): (Secret,) = item_proxy.call("GetSecret", &(session_path,)).ok()?;
    let wrapper: serde_json::Value = serde_json::from_slice(&secret.2).ok()?;
    let inner = wrapper.get("session")?;
    let inner: serde_json::Value = match inner.as_str() {
        Some(text) => serde_json::from_str(text).ok()?,
        None => inner.clone(),
    };
    Some(Session {
        access_token: inner.get("accessToken")?.as_str()?.to_string(),
        uid: inner.get("uid")?.as_str()?.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_file_shape_is_a_drive_map() {
        let value: serde_json::Value =
            serde_json::from_str(r#"{"drive":{"vol":{"lastEventId":"abc"}}}"#).unwrap();
        assert_eq!(
            value
                .get("drive")
                .and_then(|v| v.get("vol"))
                .and_then(|v| v.get("lastEventId"))
                .and_then(|v| v.as_str()),
            Some("abc")
        );
    }

    #[test]
    fn event_parents_group_by_relative_path() {
        use proton_core::snapshot::{RemoteEntry, RemoteTree};
        let mut tree = RemoteTree::new();
        tree.insert(
            "Docs".into(),
            RemoteEntry {
                uid: Some("vol~a".into()),
                kind: "dir".into(),
                ..Default::default()
            },
        );
        tree.insert(
            "Docs/Sub".into(),
            RemoteEntry {
                uid: Some("vol~b".into()),
                kind: "dir".into(),
                ..Default::default()
            },
        );
        let pending: BTreeSet<PendingEvent> = [
            PendingEvent { parent: "a".into(), child: "f1".into(), event_type: 2, trashed: false },
            PendingEvent { parent: "a".into(), child: "f2".into(), event_type: 2, trashed: false },
            PendingEvent { parent: "b".into(), child: "f3".into(), event_type: 2, trashed: false },
        ]
        .into();
        let (groups, unknown) = resolve_event_scopes(&pending, &tree, "vol", "vol~root");
        assert_eq!(unknown, 0);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].0, "Docs");
        assert_eq!(groups[0].1.len(), 2);
        assert_eq!(groups[1].0, "Docs/Sub");
        let root: BTreeSet<PendingEvent> = [PendingEvent {
            parent: "root".into(),
            child: "f".into(),
            event_type: 1,
            trashed: false,
        }]
        .into();
        assert_eq!(resolve_event_scopes(&root, &tree, "vol", "vol~root").0[0].0, "");
        let missing: BTreeSet<PendingEvent> = [PendingEvent {
            parent: "zzz".into(),
            child: "f".into(),
            event_type: 2,
            trashed: false,
        }]
        .into();
        assert_eq!(resolve_event_scopes(&missing, &tree, "vol", "vol~root").1, 1);
    }

    #[test]
    #[ignore = "starts the live event listener against the Proton API"]
    fn live_event_listener_starts() {
        let wake = Arc::new(AtomicBool::new(false));
        let event = Arc::new(Condvar::new());
        let mutex = Arc::new(Mutex::new(()));
        let cli = Cli::new().expect("CLI");
        let watch = start(
            cli,
            "/my-files".to_string(),
            wake,
            event,
            mutex,
        );
        assert!(watch.is_some(), "event listener did not start");
        std::thread::sleep(Duration::from_secs(1));
    }

    /// Live check against the user's Secret Service and CLI cursor. Ignored in
    /// CI; run with `cargo test -p proton-engine -- --ignored live_push_...`.
    #[test]
    #[ignore = "reads the live Secret Service and Proton CLI state"]
    fn live_push_session_and_cursor_are_readable() {
        let session = read_cli_session();
        eprintln!("secret service session present: {}", session.is_some());
        assert!(session.is_some(), "CLI session not found in Secret Service");
        let cli = Cli::new().expect("CLI");
        let info = cli
            .run_json(&["fs", "info", "/my-files"], true)
            .expect("fs info");
        let volume = info
            .get("treeEventScopeId")
            .and_then(|v| v.as_str())
            .expect("treeEventScopeId");
        eprintln!("volume {volume} cursor present: {}", cursor_read(volume).is_some());
        assert!(cursor_read(volume).is_some(), "event cursor missing");
    }
}
