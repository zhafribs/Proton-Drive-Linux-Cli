//! Conflict copies created by the sync engine.
//!
//! A conflict keeps the user's canonical file untouched and writes the remote
//! revision beside it as `<stem> (conflict <timestamp>[ n])<ext>`. This module
//! recognises exactly that engine-generated shape, so a user file that merely
//! contains the word “conflict” is never treated as one. Actions move data to
//! the local trash (recoverable), never delete it silently.

use crate::local_trash;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const ACK_VERSION: u32 = 1;
const MARKER: &str = " (conflict ";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conflict {
    pub conflict_rel: String,
    pub canonical_rel: String,
    pub size: u64,
    pub mtime_ns: i64,
    pub canonical_exists: bool,
    pub canonical_size: u64,
    pub canonical_mtime_ns: i64,
    pub stamp: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedName {
    pub canonical_rel: String,
    pub stamp: String,
}

fn split_parent(rel: &str) -> (&str, &str) {
    match rel.rfind('/') {
        Some(pos) => (&rel[..pos], &rel[pos + 1..]),
        None => ("", rel),
    }
}

fn valid_stamp(stamp: &str) -> bool {
    if stamp.len() < 19 || !stamp.is_char_boundary(19) {
        return false;
    }
    let ts = &stamp[..19];
    if chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%d %H-%M-%S").is_err() {
        return false;
    }
    if stamp.len() == 19 {
        return true;
    }
    let suffix = &stamp[19..];
    suffix
        .strip_prefix(' ')
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Parse an engine conflict name. Returns the canonical relative path and the
/// timestamp text. A name that does not exactly match the engine shape is
/// rejected.
pub fn parse_name(rel: &str) -> Option<ParsedName> {
    let (dir, name) = split_parent(rel);
    let idx = name.rfind(MARKER)?;
    let stem = &name[..idx];
    if stem.is_empty() {
        return None;
    }
    let rest = &name[idx + MARKER.len()..];
    let close = rest.find(')')?;
    let stamp = &rest[..close];
    if !valid_stamp(stamp) {
        return None;
    }
    let ext = &rest[close + 1..];
    if !ext.is_empty() && !ext.starts_with('.') {
        return None;
    }
    let canonical_name = format!("{stem}{ext}");
    if canonical_name.is_empty() {
        return None;
    }
    let canonical_rel = if dir.is_empty() {
        canonical_name
    } else {
        format!("{dir}/{canonical_name}")
    };
    Some(ParsedName {
        canonical_rel,
        stamp: stamp.to_string(),
    })
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn mtime_ns(meta: &fs::Metadata) -> i64 {
    meta.mtime()
        .saturating_mul(1_000_000_000)
        .saturating_add(meta.mtime_nsec())
}

fn walk(root: &Path, rel: &str, out: &mut Vec<Conflict>) -> Result<(), String> {
    let dir = if rel.is_empty() {
        root.to_path_buf()
    } else {
        root.join(rel)
    };
    let entries = fs::read_dir(&dir).map_err(|e| format!("reading {}: {e}", dir.display()))?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        let child = if rel.is_empty() {
            name.to_string()
        } else {
            format!("{rel}/{name}")
        };
        let meta = match fs::symlink_metadata(entry.path()) {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            // Engine-private folders never contain user conflicts.
            if name == ".proton-sync-staging" || name == ".proton-photos-trash" {
                continue;
            }
            walk(root, &child, out)?;
            continue;
        }
        let Some(parsed) = parse_name(&child) else {
            continue;
        };
        let canonical = root.join(&parsed.canonical_rel);
        let canonical_meta = fs::symlink_metadata(&canonical)
            .ok()
            .filter(|m| m.is_file() && !m.file_type().is_symlink());
        out.push(Conflict {
            conflict_rel: child,
            canonical_rel: parsed.canonical_rel,
            size: meta.len(),
            mtime_ns: mtime_ns(&meta),
            canonical_exists: canonical_meta.is_some(),
            canonical_size: canonical_meta.as_ref().map(|m| m.len()).unwrap_or(0),
            canonical_mtime_ns: canonical_meta.as_ref().map(mtime_ns).unwrap_or(0),
            stamp: parsed.stamp,
        });
    }
    Ok(())
}

/// All conflict copies under the sync root, sorted by canonical path.
pub fn scan(local_root: &Path) -> Result<Vec<Conflict>, String> {
    let meta = fs::symlink_metadata(local_root)
        .map_err(|e| format!("reading sync folder {}: {e}", local_root.display()))?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err(format!("sync folder is not a directory: {}", local_root.display()));
    }
    let mut out = Vec::new();
    walk(local_root, "", &mut out)?;
    out.sort_by(|a, b| {
        a.canonical_rel
            .cmp(&b.canonical_rel)
            .then_with(|| a.conflict_rel.cmp(&b.conflict_rel))
    });
    Ok(out)
}

fn cache_path(state_dir: &Path) -> PathBuf {
    state_dir.join("conflicts-cache.json")
}

#[derive(Serialize, Deserialize)]
struct Cache {
    version: u32,
    root: String,
    scanned_at: u64,
    conflicts: Vec<Conflict>,
}

const CACHE_VERSION: u32 = 1;
const CACHE_TTL: u64 = 300;

fn cached(state_dir: &Path, local_root: &Path) -> Option<Vec<Conflict>> {
    let bytes = fs::read(cache_path(state_dir)).ok()?;
    let cache: Cache = serde_json::from_slice(&bytes).ok()?;
    if cache.version != CACHE_VERSION
        || cache.root != local_root.to_string_lossy()
        || now_secs().saturating_sub(cache.scanned_at) > CACHE_TTL
    {
        return None;
    }
    if cache.conflicts.iter().all(|c| verify(local_root, c).is_ok()) {
        Some(cache.conflicts)
    } else {
        None
    }
}

fn save_cache(state_dir: &Path, local_root: &Path, conflicts: &[Conflict]) {
    let cache = Cache {
        version: CACHE_VERSION,
        root: local_root.to_string_lossy().into_owned(),
        scanned_at: now_secs(),
        conflicts: conflicts.to_vec(),
    };
    let Ok(bytes) = serde_json::to_vec_pretty(&cache) else {
        return;
    };
    let path = cache_path(state_dir);
    let _ = fs::create_dir_all(state_dir);
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    if fs::write(&tmp, bytes).is_ok() {
        let _ = fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        let _ = fs::rename(&tmp, &path);
    }
}

/// Remove the scan cache when the engine creates a conflict, so the next
/// Recovery view cannot miss the new file.
pub fn invalidate_cache(state_dir: &Path) {
    let _ = fs::remove_file(cache_path(state_dir));
}

/// Cached scan. The cache is only trusted when it is recent, belongs to the
/// same root, and every cached file still matches its fingerprint.
pub fn scan_cached(state_dir: &Path, local_root: &Path) -> Result<Vec<Conflict>, String> {
    if let Some(list) = cached(state_dir, local_root) {
        return Ok(list);
    }
    let list = scan(local_root)?;
    save_cache(state_dir, local_root, &list);
    Ok(list)
}

fn ack_path(state_dir: &Path) -> PathBuf {
    state_dir.join("conflicts-ack.json")
}

#[derive(Serialize, Deserialize, Default)]
struct Acks {
    version: u32,
    #[serde(default)]
    entries: BTreeMap<String, i64>,
}

fn read_acks(state_dir: &Path) -> Acks {
    let path = ack_path(state_dir);
    let Ok(text) = fs::read(&path) else {
        return Acks {
            version: ACK_VERSION,
            entries: BTreeMap::new(),
        };
    };
    let mut acks: Acks = serde_json::from_slice(&text).unwrap_or_default();
    if acks.version != ACK_VERSION {
        acks = Acks {
            version: ACK_VERSION,
            entries: BTreeMap::new(),
        };
    }
    acks
}

fn write_acks(state_dir: &Path, acks: &Acks) -> Result<(), String> {
    let path = ack_path(state_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("creating {}: {e}", parent.display()))?;
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = path.with_file_name(format!(
        ".conflicts-ack.{}.{}.tmp",
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
        serde_json::to_writer_pretty(&mut file, acks)
            .map_err(|e| format!("writing {}: {e}", tmp.display()))?;
        file.write_all(b"\n")
            .map_err(|e| format!("writing {}: {e}", tmp.display()))?;
        file.sync_all()
            .map_err(|e| format!("syncing {}: {e}", tmp.display()))?;
        drop(file);
        fs::rename(&tmp, &path).map_err(|e| format!("replacing {}: {e}", path.display()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Conflict paths the user explicitly chose to keep (both versions). A later
/// edit changes the mtime and makes the conflict visible again.
pub fn acked(state_dir: &Path) -> BTreeMap<String, i64> {
    read_acks(state_dir).entries
}

pub fn ack(state_dir: &Path, conflict_rel: &str, mtime_ns: i64) -> Result<(), String> {
    let mut acks = read_acks(state_dir);
    acks.entries.insert(conflict_rel.to_string(), mtime_ns);
    write_acks(state_dir, &acks)
}

pub fn unack(state_dir: &Path, conflict_rel: &str) -> Result<(), String> {
    let mut acks = read_acks(state_dir);
    if acks.entries.remove(conflict_rel).is_some() {
        write_acks(state_dir, &acks)?;
    }
    Ok(())
}

fn verify(local_root: &Path, conflict: &Conflict) -> Result<(), String> {
    let path = local_root.join(&conflict.conflict_rel);
    let meta = fs::symlink_metadata(&path)
        .map_err(|e| format!("reading {}: {e}", path.display()))?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err(format!("conflict copy is no longer a file: {}", path.display()));
    }
    if meta.len() != conflict.size || mtime_ns(&meta) != conflict.mtime_ns {
        return Err(format!(
            "conflict copy changed since the list was read: {}",
            path.display()
        ));
    }
    let canonical = local_root.join(&conflict.canonical_rel);
    if conflict.canonical_exists {
        let cmeta = fs::symlink_metadata(&canonical)
            .map_err(|e| format!("reading {}: {e}", canonical.display()))?;
        if !cmeta.is_file()
            || cmeta.len() != conflict.canonical_size
            || mtime_ns(&cmeta) != conflict.canonical_mtime_ns
        {
            return Err(format!(
                "the canonical file changed since the list was read: {}",
                canonical.display()
            ));
        }
    } else if fs::symlink_metadata(&canonical).is_ok() {
        return Err(format!(
            "a file appeared at the canonical path: {}",
            canonical.display()
        ));
    }
    Ok(())
}

/// Keep the canonical file and move the conflict copy to the local trash.
pub fn keep_current(
    state_dir: &Path,
    local_root: &Path,
    conflict: &Conflict,
) -> Result<(), String> {
    verify(local_root, conflict)?;
    local_trash::trash_local(state_dir, local_root, &conflict.conflict_rel)?;
    let _ = unack(state_dir, &conflict.conflict_rel);
    Ok(())
}

/// Promote the conflict copy to the canonical name. The current canonical file
/// is moved to the local trash first, so the action is reversible even if the
/// rename fails.
pub fn use_conflict(
    state_dir: &Path,
    local_root: &Path,
    conflict: &Conflict,
) -> Result<(), String> {
    verify(local_root, conflict)?;
    let conflict_path = local_root.join(&conflict.conflict_rel);
    let canonical_path = local_root.join(&conflict.canonical_rel);
    let backup = if conflict.canonical_exists {
        Some(local_trash::trash_local(
            state_dir,
            local_root,
            &conflict.canonical_rel,
        )?)
    } else {
        None
    };
    if fs::symlink_metadata(&canonical_path).is_ok() {
        if let Some(backup) = &backup {
            let _ = local_trash::restore(state_dir, local_root, backup);
        }
        return Err(format!(
            "the canonical path changed before the swap: {}",
            canonical_path.display()
        ));
    }
    if let Err(e) = fs::rename(&conflict_path, &canonical_path) {
        if let Some(backup) = &backup {
            let _ = local_trash::restore(state_dir, local_root, backup);
        }
        return Err(format!(
            "moving {} to {}: {e}",
            conflict_path.display(),
            canonical_path.display()
        ));
    }
    let _ = unack(state_dir, &conflict.conflict_rel);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "pd-conflict-{}-{}-{name}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ))
    }

    #[test]
    fn parses_only_the_engine_shape() {
        let p = parse_name("dir/photo (conflict 2026-01-02 03-04-05).jpg").unwrap();
        assert_eq!(p.canonical_rel, "dir/photo.jpg");
        let p = parse_name(".bashrc (conflict 2026-01-02 03-04-05)").unwrap();
        assert_eq!(p.canonical_rel, ".bashrc");
        let p = parse_name("a (conflict 2026-01-02 03-04-05 2).txt").unwrap();
        assert_eq!(p.canonical_rel, "a.txt");
        assert!(parse_name("notes about conflict.txt").is_none());
        assert!(parse_name("x (conflict yesterday).txt").is_none());
        assert!(parse_name("x (conflict 2026-01-02 03-04-05).txt").is_some());
    }

    #[test]
    fn scan_and_use_conflict_back_up_the_canonical_file() {
        let state = temp("state");
        let root = temp("root");
        fs::create_dir_all(root.join("dir")).unwrap();
        fs::write(root.join("dir/photo.jpg"), b"local").unwrap();
        fs::write(
            root.join("dir/photo (conflict 2026-01-02 03-04-05).jpg"),
            b"remote",
        )
        .unwrap();
        let list = scan(&root).unwrap();
        assert_eq!(list.len(), 1);
        assert!(list[0].canonical_exists);
        use_conflict(&state, &root, &list[0]).unwrap();
        assert_eq!(fs::read(root.join("dir/photo.jpg")).unwrap(), b"remote");
        let trash = local_trash::entries(&state).unwrap();
        assert_eq!(trash.len(), 1);
        assert_eq!(trash[0].original_rel, "dir/photo.jpg");
        let _ = fs::remove_dir_all(&state);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn keep_current_moves_the_copy_to_trash() {
        let state = temp("state-keep");
        let root = temp("root-keep");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("a.txt"), b"local").unwrap();
        fs::write(root.join("a (conflict 2026-01-02 03-04-05).txt"), b"remote").unwrap();
        let list = scan(&root).unwrap();
        keep_current(&state, &root, &list[0]).unwrap();
        assert_eq!(fs::read(root.join("a.txt")).unwrap(), b"local");
        assert!(!root.join("a (conflict 2026-01-02 03-04-05).txt").exists());
        assert_eq!(local_trash::entries(&state).unwrap().len(), 1);
        let _ = fs::remove_dir_all(&state);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn changed_fingerprint_refuses_the_action() {
        let state = temp("state-race");
        let root = temp("root-race");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("a.txt"), b"local").unwrap();
        fs::write(root.join("a (conflict 2026-01-02 03-04-05).txt"), b"remote").unwrap();
        let list = scan(&root).unwrap();
        fs::write(root.join("a (conflict 2026-01-02 03-04-05).txt"), b"changed").unwrap();
        assert!(keep_current(&state, &root, &list[0]).is_err());
        let _ = fs::remove_dir_all(&state);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn cache_reuses_unchanged_entries_and_invalidates_on_change() {
        let state = temp("state-cache");
        let root = temp("root-cache");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("a.txt"), b"local").unwrap();
        fs::write(root.join("a (conflict 2026-01-02 03-04-05).txt"), b"remote").unwrap();
        let first = scan_cached(&state, &root).unwrap();
        assert_eq!(first.len(), 1);
        assert!(cache_path(&state).is_file());
        let cached = scan_cached(&state, &root).unwrap();
        assert_eq!(cached[0].mtime_ns, first[0].mtime_ns);
        std::thread::sleep(std::time::Duration::from_millis(2));
        fs::write(root.join("a (conflict 2026-01-02 03-04-05).txt"), b"changed").unwrap();
        let after = scan_cached(&state, &root).unwrap();
        assert_ne!(after[0].mtime_ns, first[0].mtime_ns);
        invalidate_cache(&state);
        assert!(!cache_path(&state).exists());
        let _ = fs::remove_dir_all(&state);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn ack_round_trip_tracks_the_mtime() {
        let state = temp("state-ack");
        fs::create_dir_all(&state).unwrap();
        ack(&state, "a (conflict 2026-01-02 03-04-05).txt", 1).unwrap();
        assert_eq!(
            acked(&state).get("a (conflict 2026-01-02 03-04-05).txt"),
            Some(&1)
        );
        unack(&state, "a (conflict 2026-01-02 03-04-05).txt").unwrap();
        assert!(acked(&state).is_empty());
        let _ = fs::remove_dir_all(&state);
    }
}
