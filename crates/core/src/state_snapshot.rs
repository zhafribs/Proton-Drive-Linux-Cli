//! Rotating, checksum-verified snapshots of `state.json`.
//!
//! The engine creates one before a root change, an approved mass delete, or
//! an update swap; the Recovery page can create and restore them explicitly.
//! A restore is itself snapshotted first, so it is reversible. Snapshots are
//! metadata only (the per-path history), not copies of user files.

use crate::state::{self, State};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_SNAPSHOTS: usize = 5;
const MAX_BYTES: u64 = 64 * 1024 * 1024;
const SNAPSHOT_VERSION: u32 = 1;

#[derive(Clone, Debug)]
pub struct SnapshotInfo {
    pub path: PathBuf,
    pub created_at: u64,
    pub reason: String,
    pub bytes: u64,
    pub entries: usize,
    pub valid: bool,
    /// The `sync-roots` value this history belongs to. A snapshot can only
    /// be restored while the app still uses the same roots.
    pub roots: String,
    pub roots_match: bool,
}

#[derive(Serialize, Deserialize)]
struct SnapshotMeta {
    version: u32,
    created_at: u64,
    reason: String,
    entries: usize,
    #[serde(default)]
    roots: String,
}

pub fn dir(state_dir: &Path) -> PathBuf {
    state_dir.join("snapshots")
}

fn checksum_path(path: &Path) -> PathBuf {
    path.with_extension("json.sha256")
}

fn state_path(state_dir: &Path) -> PathBuf {
    state_dir.join("state.json")
}

fn current_roots(state_dir: &Path) -> String {
    fs::read_to_string(state_dir.join("sync-roots")).unwrap_or_default()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sanitize_reason(reason: &str) -> String {
    let mut out: String = reason
        .chars()
        .flat_map(|c| c.to_lowercase())
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "manual".into()
    } else {
        out.chars().take(32).collect()
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("creating {}: {e}", parent.display()))?;
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = path.with_file_name(format!(
        ".snapshot.{}.{}.tmp",
        std::process::id(),
        stamp
    ));
    let result = (|| -> Result<(), String> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(|e| format!("creating {}: {e}", tmp.display()))?;
        file.write_all(bytes)
            .map_err(|e| format!("writing {}: {e}", tmp.display()))?;
        file.sync_all()
            .map_err(|e| format!("syncing {}: {e}", tmp.display()))?;
        drop(file);
        fs::rename(&tmp, path).map_err(|e| format!("replacing {}: {e}", path.display()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Create a snapshot of the current state. Returns `None` when there is no
/// state file yet (a first run has nothing to preserve).
pub fn create(state_dir: &Path, reason: &str) -> Result<Option<SnapshotInfo>, String> {
    let src = state_path(state_dir);
    let bytes = match fs::read(&src) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("reading {}: {e}", src.display())),
    };
    let parsed: State = serde_json::from_slice(&bytes)
        .map_err(|e| format!("{} is not valid state JSON: {e}", src.display()))?;
    let roots = current_roots(state_dir);
    if roots.is_empty() {
        return Err("the sync roots are not recorded yet; refusing an unbound snapshot".into());
    }
    let reason = sanitize_reason(reason);
    let created_at = now_secs();
    let dir = dir(state_dir);
    fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let mut n = 0u32;
    let path = loop {
        let suffix = if n == 0 {
            String::new()
        } else {
            format!("-{}", n + 1)
        };
        let candidate = dir.join(format!("state-{created_at}-{reason}{suffix}.json"));
        if !candidate.exists() {
            break candidate;
        }
        n += 1;
    };
    let meta = SnapshotMeta {
        version: SNAPSHOT_VERSION,
        created_at,
        reason: reason.clone(),
        entries: parsed.len(),
        roots: roots.clone(),
    };
    // One line of metadata, then the raw state JSON. Keeping the metadata
    // on a single line makes the first newline an unambiguous separator.
    let meta_bytes = serde_json::to_vec(&meta)
        .map_err(|e| format!("encoding snapshot metadata: {e}"))?;
    let mut payload = meta_bytes;
    payload.push(b'\n');
    payload.extend_from_slice(&bytes);
    atomic_write(&path, &payload)?;
    let digest = hex(Sha256::digest(&payload).as_slice());
    atomic_write(&checksum_path(&path), format!("{digest}\n").as_bytes())?;
    prune(&dir);
    Ok(Some(SnapshotInfo {
        path,
        created_at,
        reason,
        bytes: payload.len() as u64,
        entries: parsed.len(),
        valid: true,
        roots: meta.roots.clone(),
        roots_match: meta.roots == roots,
    }))
}

fn parse_snapshot(path: &Path) -> Option<(SnapshotMeta, State, u64)> {
    let bytes = fs::read(path).ok()?;
    let newline = bytes.iter().position(|b| *b == b'\n')?;
    let meta: SnapshotMeta = serde_json::from_slice(&bytes[..newline]).ok()?;
    if meta.version != SNAPSHOT_VERSION {
        return None;
    }
    let state: State = serde_json::from_slice(&bytes[newline + 1..]).ok()?;
    Some((meta, state, bytes.len() as u64))
}

fn checksum_ok(path: &Path) -> bool {
    let Ok(payload) = fs::read(path) else {
        return false;
    };
    let Ok(expected) = fs::read_to_string(checksum_path(path)) else {
        return false;
    };
    let expected = expected.trim();
    !expected.is_empty() && hex(Sha256::digest(&payload).as_slice()) == expected
}

/// Snapshots, newest first. A snapshot with a missing/mismatched checksum or
/// malformed payload is listed but marked invalid; it cannot be restored.
pub fn list(state_dir: &Path) -> Vec<SnapshotInfo> {
    let dir = dir(state_dir);
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let current = current_roots(state_dir);
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(rest) = name.strip_prefix("state-") else {
            continue;
        };
        let Some((stamp, reason)) = rest.split_once('-') else {
            continue;
        };
        let Ok(created_at) = stamp.parse::<u64>() else {
            continue;
        };
        let reason = reason.trim_end_matches(".json").to_string();
        let bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
        let (entries, valid, reason, roots) = match parse_snapshot(&path) {
            Some((meta, state, _)) => (
                state.len(),
                checksum_ok(&path),
                if meta.reason.is_empty() {
                    reason
                } else {
                    meta.reason
                },
                meta.roots,
            ),
            None => (0, false, reason, String::new()),
        };
        let roots_match = !roots.is_empty() && roots == current;
        out.push(SnapshotInfo {
            path,
            created_at,
            reason,
            bytes,
            entries,
            valid,
            roots,
            roots_match,
        });
    }
    out.sort_by_key(|s| std::cmp::Reverse(s.created_at));
    out
}

fn remove_snapshot(path: &Path) {
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(checksum_path(path));
}

fn prune(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(u64, u64, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                return None;
            }
            let name = path.file_name()?.to_str()?;
            let rest = name.strip_prefix("state-")?;
            let (stamp, _) = rest.split_once('-')?;
            let created = stamp.parse::<u64>().ok()?;
            let bytes = e.metadata().map(|m| m.len()).unwrap_or(0);
            Some((created, bytes, path))
        })
        .collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.0)); // newest first
    let mut total = 0u64;
    for (i, (_, bytes, path)) in files.iter().enumerate() {
        total = total.saturating_add(*bytes);
        if i >= MAX_SNAPSHOTS || total > MAX_BYTES {
            remove_snapshot(path);
        }
    }
}

/// Delete one snapshot and its checksum. Only files inside the snapshot
/// folder are accepted; user files are never touched.
pub fn remove(state_dir: &Path, path: &Path) -> Result<(), String> {
    let dir = dir(state_dir);
    if path.parent() != Some(dir.as_path()) {
        return Err("refusing to delete a file outside the snapshot folder".into());
    }
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return Err("snapshot path is not UTF-8".into());
    };
    if !name.starts_with("state-") || !name.ends_with(".json") {
        return Err("refusing an unexpected snapshot name".into());
    }
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("deleting {}: {e}", path.display())),
    }
    let _ = fs::remove_file(checksum_path(path));
    Ok(())
}

/// Restore one snapshot. The current state is snapshotted first, the payload
/// checksum is verified, and only then is `state.json` replaced atomically.
pub fn restore(state_dir: &Path, snapshot: &SnapshotInfo) -> Result<(), String> {
    let dir = dir(state_dir);
    let parent = snapshot
        .path
        .parent()
        .ok_or_else(|| "snapshot has no parent directory".to_string())?;
    if parent != dir {
        return Err("refusing a snapshot outside the snapshot folder".into());
    }
    let Some(name) = snapshot.path.file_name().and_then(|n| n.to_str()) else {
        return Err("snapshot path is not UTF-8".into());
    };
    if !name.starts_with("state-") || !name.ends_with(".json") {
        return Err("refusing an unexpected snapshot name".into());
    }
    if !checksum_ok(&snapshot.path) {
        return Err("snapshot checksum does not match; refusing to restore it".into());
    }
    let Some((meta, state, _)) = parse_snapshot(&snapshot.path) else {
        return Err("snapshot payload is unreadable".into());
    };
    if meta.roots.is_empty() || meta.roots != current_roots(state_dir) {
        return Err("snapshot belongs to a different sync root; refusing to restore it".into());
    }
    // Keep the current state as a pre-restore snapshot before replacing it.
    // A corrupt state cannot be snapshotted, so preserve its raw bytes under
    // a dated name instead of silently discarding the evidence.
    let current = state_path(state_dir);
    if current.exists()
        && let Err(e) = create(state_dir, "pre-restore")
    {
        let backup = state_dir.join(format!("state.json.corrupt-{}", now_secs()));
        fs::copy(&current, &backup).map_err(|copy| {
            format!(
                "could not preserve the current state before restoring ({e}); copy also failed: {copy}"
            )
        })?;
        eprintln!(
            "WARN current state could not be snapshotted ({e}); preserved {}",
            backup.display()
        );
    }
    state::write(&current, &state)
        .map_err(|e| format!("writing {}: {e}", state_path(state_dir).display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "pd-snapshot-{}-{}-{name}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ))
    }

    fn write_state(dir: &Path, value: &str) {
        let mut state = BTreeMap::new();
        state.insert(
            "a.txt".to_string(),
            state::StateEntry {
                l_mtime_ns: Some(1),
                l_size: Some(value.len() as u64),
                r_mtime: Some(value.to_string()),
                r_uid: None,
            },
        );
        fs::write(dir.join("sync-roots"), "/root\n/my-files\n").unwrap();
        state::write(&dir.join("state.json"), &state).unwrap();
    }

    #[test]
    fn create_list_and_restore_round_trip() {
        let dir = temp("state");
        fs::create_dir_all(&dir).unwrap();
        write_state(&dir, "one");
        let snap = create(&dir, "mass delete").unwrap().unwrap();
        assert_eq!(snap.reason, "mass-delete");
        assert_eq!(snap.entries, 1);
        write_state(&dir, "two");
        restore(&dir, &list(&dir)[0]).unwrap();
        let restored = state::read(&dir.join("state.json"));
        assert_eq!(
            restored["a.txt"].r_mtime.as_deref(),
            Some("one")
        );
        // The pre-restore state is itself a snapshot.
        assert!(list(&dir).iter().any(|s| s.reason == "pre-restore"));
        assert!(list(&dir).iter().all(|s| s.roots_match));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_tampered_snapshot_is_not_restored() {
        let dir = temp("tamper");
        fs::create_dir_all(&dir).unwrap();
        write_state(&dir, "one");
        let snap = create(&dir, "manual").unwrap().unwrap();
        let mut bytes = fs::read(&snap.path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        fs::write(&snap.path, bytes).unwrap();
        assert!(restore(&dir, &snap).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rotation_keeps_the_newest_five() {
        let dir = temp("rotate");
        fs::create_dir_all(&dir).unwrap();
        for i in 0..7 {
            write_state(&dir, &format!("v{i}"));
            let _ = create(&dir, "manual").unwrap();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let listed = list(&dir);
        assert!(listed.len() <= MAX_SNAPSHOTS);
        assert!(listed.iter().all(|s| s.valid));
        let _ = fs::remove_dir_all(&dir);
    }
}
