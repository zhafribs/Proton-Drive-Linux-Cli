//! Local tree scan: `{relpath: {type, mtime_ns, size}}`, like `scan_local`.
//!
//! Classification mirrors `os.walk(followlinks=False)`:
//! * a symlink to a directory is listed as a directory but not descended;
//! * a symlink to a file is skipped, because the Python code `lstat`s files
//!   and only accepts regular files;
//! * unreadable directories are skipped (Python's default `onerror`).

use crate::action::{LocalEntry, LocalTree};
use crate::filter::Filter;
use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

pub fn scan_local(root: &Path, filter: &Filter) -> LocalTree {
    let mut tree = LocalTree::new();
    walk(root, "", filter, &mut tree);
    tree
}

fn walk(dir: &Path, rel_base: &str, filter: &Filter, tree: &mut LocalTree) {
    let Ok(read) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = read.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = if rel_base.is_empty() {
            name.clone()
        } else {
            format!("{rel_base}/{name}")
        };
        let path = entry.path();
        let file_type = entry.file_type().ok();
        let followed = fs::metadata(&path).ok();
        let is_dir = followed.as_ref().is_some_and(|m| m.is_dir());

        if is_dir {
            if Filter::skip_name(&name) || filter.excluded(&rel) || !filter.allowed_dir(&rel) {
                continue;
            }
            tree.insert(rel.clone(), LocalEntry::Dir);
            if !file_type.is_some_and(|t| t.is_symlink()) {
                walk(&path, &rel, filter, tree);
            }
            continue;
        }

        if Filter::skip_name(&name) || filter.excluded(&rel) || !filter.allowed_file(&rel) {
            continue;
        }
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.file_type().is_file() {
            continue; // socket, fifo, symlink, ...
        }
        tree.insert(
            rel,
            LocalEntry::File {
                mtime_ns: mtime_ns(&meta),
                size: meta.len(),
            },
        );
    }
}

fn mtime_ns(meta: &fs::Metadata) -> i64 {
    let Ok(modified) = meta.modified() else {
        return 0;
    };
    match modified.duration_since(UNIX_EPOCH) {
        Ok(d) => i64::try_from(d.as_nanos()).unwrap_or(i64::MAX),
        Err(e) => -i64::try_from(e.duration().as_nanos()).unwrap_or(i64::MAX),
    }
}

/// Remote tree scan using the official CLI (`fs list --json`).
use crate::cli::{self, Cli};
use anyhow::Result;
use proton_core::snapshot::{RemoteEntry, RemoteTree};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// Result of a remote scan: the tree plus whether a full walk ran (false =
/// the cached walk was reused on the `fs size` fingerprint).
pub struct RemoteScan {
    pub tree: RemoteTree,
    pub full: bool,
}

/// `fs size` fingerprint of the remote root: `[size, numberOfDescendants]`.
/// One CLI call replaces a per-folder walk when it matches the last one.
fn remote_fp(cli: &Cli, remote_root: &str) -> Result<Vec<i64>> {
    let v = cli.run_json(&["fs", "size", remote_root], false)?;
    let size = v.get("size").and_then(|s| s.as_i64()).unwrap_or(-1);
    let descendants = v
        .get("numberOfDescendants")
        .and_then(|n| n.as_i64())
        .unwrap_or(-1);
    Ok(vec![size, descendants])
}

/// Pure decision for the cached-walk fast path: the fingerprint must match,
/// the walk must be fresh enough, and the skip budget must still have room.
/// Separated from I/O so the staleness rules are unit-testable.
fn cache_usable(snap: &proton_core::snapshot::Snapshot, sig: &str, fp: &[i64], now: f64, max_age: f64, max_skips: u32) -> bool {
    snap.sig == sig
        && !snap.tree.is_empty()
        && snap.fp == fp
        && now - snap.ts <= max_age
        && snap.skips < max_skips
}

/// Reuse the cached remote walk when the fingerprint is unchanged, the walk
/// is fresh enough, and the skip budget still has room. On success the
/// snapshot's timestamp and skip counter are refreshed so the next cycle sees
/// this reuse; `None` means a full walk must run.
fn try_cached_tree(cli: &Cli, remote_root: &str, filter: &Filter) -> Result<Option<RemoteTree>> {
    if cli::cfg_int("PROTON_SYNC_FAST_REMOTE", 1, 1) == 0 || !filter.is_unfiltered() {
        return Ok(None);
    }
    let snap_path = crate::health::state_dir().join("remote-snapshot.json");
    let Some(snap) = proton_core::snapshot::read(&snap_path) else {
        return Ok(None);
    };
    let fp = remote_fp(cli, remote_root)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    let max_age = cli::cfg_int("PROTON_SYNC_FAST_REMOTE_MAX_AGE", 21600, 604800) as f64;
    let max_skips = cli::cfg_int("PROTON_SYNC_FAST_REMOTE_MAX_SKIPS", 6, 48) as u32;
    if !cache_usable(&snap, &filter.signature(remote_root), &fp, now, max_age, max_skips) {
        return Ok(None);
    }
    let mut updated = snap.clone();
    updated.ts = now;
    updated.skips += 1;
    let _ = proton_core::snapshot::write(&snap_path, &updated);
    Ok(Some(snap.view()))
}

pub fn scan_remote(cli: &Cli, remote_root: &str, filter: &Filter) -> Result<RemoteScan> {
    let start_rpath = cli::remote_path(remote_root, "");
    let workers = cli::cfg_int("PROTON_SYNC_WORKERS", 12, 32);
    let retries = cli::cfg_int("PROTON_SYNC_RETRIES", 3, 10);

    cli::with_cli_lock(|| {
        // Fast path: one `fs size` call is the change signal for the whole
        // tree. When `[size, numberOfDescendants]` still matches the last
        // full walk and that walk is recent enough (few skipped cycles), the
        // cached tree is authoritative and no per-folder listing runs. The
        // periodic forced full walk bounds same-size renames the totals
        // cannot see.
        if let Some(tree) = try_cached_tree(cli, remote_root, filter)? {
            return Ok(RemoteScan { tree, full: false });
        }

        let _ = cli.run(&["fs", "info", remote_root], true, Some(120), false);

        let mut tree = RemoteTree::new();
        let mut visited: HashSet<String> = HashSet::new();
        let mut rel_of: HashMap<String, String> = HashMap::new();
        rel_of.insert(start_rpath.clone(), String::new());
        let mut level = vec![start_rpath.clone()];

        while !level.is_empty() {
            level.retain(|p| visited.insert(p.clone()));
            let items: Vec<(String, String)> = level
                .iter()
                .map(|p| (p.clone(), rel_of.get(p).cloned().unwrap_or_default()))
                .collect();
            let listed = cli::run_parallel(items, workers, |(rpath, rel)| {
                let mut entries = cli.run_json(&["fs", "list", &rpath], false);
                for attempt in 1..retries {
                    if entries.is_ok() {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_secs(2 * attempt as u64));
                    entries = cli.run_json(&["fs", "list", &rpath], false);
                }
                (rpath, rel, entries)
            });

            let mut next: Vec<String> = Vec::new();
            for (rpath, rel, entries) in listed {
                let Value::Array(arr) = entries? else { continue };
                for e in arr {
                    let Some(obj) = e.as_object() else { continue };
                    if !obj.contains_key("type") { continue; }
                    let typ = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    let name_obj = obj.get("name");
                    let ok = name_obj.and_then(|n| n.get("ok")).and_then(|v| v.as_bool()).unwrap_or(true);
                    let name = name_obj.and_then(|n| n.get("value")).and_then(|v| v.as_str()).unwrap_or("");
                    if !ok || name.is_empty() { continue; }
                    if Filter::skip_name(name) { continue; }
                    let child_rel = if rel.is_empty() { name.to_string() } else { format!("{}/{}", rel, name) };
                    if filter.excluded(&child_rel) { continue; }
                    let media = obj.get("mediaType").and_then(|v| v.as_str()).unwrap_or("");
                    let size = obj.get("activeRevision")
                        .and_then(|r| r.get("claimedSize"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    if typ == "folder" {
                        if !filter.allowed_dir(&child_rel) { continue; }
                        tree.insert(child_rel.clone(), RemoteEntry {
                            uid: obj.get("uid").and_then(|v| v.as_str()).map(|s| s.to_string()),
                            kind: "dir".to_string(),
                            mtime: obj.get("modificationTime").and_then(|v| v.as_str()).map(|s| s.to_string()),
                            size: Some(0),
                            doc: false,
                        });
                        let child_rpath = format!("{}/{}", rpath, cli::escape(name));
                        rel_of.insert(child_rpath.clone(), child_rel);
                        next.push(child_rpath);
                    } else if typ == "file" {
                        if !filter.allowed_file(&child_rel) { continue; }
                        let doc = media.starts_with("application/vnd.proton");
                        tree.insert(child_rel, RemoteEntry {
                            uid: obj.get("uid").and_then(|v| v.as_str()).map(|s| s.to_string()),
                            kind: "file".to_string(),
                            mtime: obj.get("modificationTime").and_then(|v| v.as_str()).map(|s| s.to_string()),
                            size: Some(size),
                            doc,
                        });
                    }
                }
            }
            level = next;
        }
        Ok(RemoteScan { tree, full: true })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn tmp(name: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("pdscan-{}-{name}", std::process::id()));
        fs::remove_dir_all(&p).ok();
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn scans_files_dirs_and_applies_filters() {
        let root = tmp("basic");
        fs::create_dir_all(root.join("Docs/Sub")).unwrap();
        fs::create_dir_all(root.join("Videos")).unwrap();
        fs::write(root.join("Docs/a.txt"), b"abc").unwrap();
        fs::write(root.join("Docs/Sub/b.tmp"), b"x").unwrap();
        fs::write(root.join("Videos/c.mp4"), b"x").unwrap();
        fs::write(root.join(".DS_Store"), b"x").unwrap();

        let all = scan_local(&root, &Filter::new("", ""));
        assert_eq!(all["Docs"], LocalEntry::Dir);
        assert_eq!(all["Docs/Sub"], LocalEntry::Dir);
        assert_eq!(all["Docs/a.txt"].size(), Some(3));
        assert!(all.contains_key("Videos/c.mp4"));
        assert!(!all.contains_key(".DS_Store"));

        let filtered = scan_local(&root, &Filter::new("Docs", "*.tmp"));
        assert!(filtered.contains_key("Docs/a.txt"));
        assert!(!filtered.contains_key("Docs/Sub/b.tmp"));
        assert!(!filtered.contains_key("Videos"));
        assert!(!filtered.contains_key("Videos/c.mp4"));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn symlinked_dirs_are_listed_but_not_descended_and_file_links_skipped() {
        let root = tmp("links");
        let outside = tmp("outside");
        fs::write(outside.join("inside.txt"), b"x").unwrap();
        symlink(&outside, root.join("linked-dir")).unwrap();
        symlink(outside.join("inside.txt"), root.join("linked-file")).unwrap();

        let tree = scan_local(&root, &Filter::new("", ""));
        assert_eq!(tree["linked-dir"], LocalEntry::Dir);
        assert!(!tree.contains_key("linked-dir/inside.txt"));
        assert!(!tree.contains_key("linked-file"));
        fs::remove_dir_all(&root).ok();
        fs::remove_dir_all(&outside).ok();
    }

    fn snap() -> proton_core::snapshot::Snapshot {
        let mut tree = RemoteTree::new();
        tree.insert(
            "a.txt".into(),
            RemoteEntry {
                uid: Some("uid-1".into()),
                kind: "file".into(),
                mtime: None,
                size: Some(12),
                doc: false,
            },
        );
        proton_core::snapshot::Snapshot {
            sig: "/my-files||".into(),
            fp: vec![100, 10],
            ts: 1000.0,
            skips: 0,
            tree,
        }
    }

    #[test]
    fn cached_walk_is_reused_only_when_fresh_matching_and_within_budget() {
        // Fresh, matching fingerprint, budget left: reuse.
        assert!(cache_usable(&snap(), "/my-files||", &[100, 10], 2000.0, 21600.0, 6));
        // Fingerprint changed: full walk.
        assert!(!cache_usable(&snap(), "/my-files||", &[99, 10], 2000.0, 21600.0, 6));
        // Too old: full walk.
        assert!(!cache_usable(&snap(), "/my-files||", &[100, 10], 1000.0 + 21601.0, 21600.0, 6));
        // Skip budget exhausted: forced full walk.
        let mut s = snap();
        s.skips = 6;
        assert!(!cache_usable(&s, "/my-files||", &[100, 10], 2000.0, 21600.0, 6));
        // Empty tree or wrong signature: never reused.
        let mut e = snap();
        e.tree.clear();
        assert!(!cache_usable(&e, "/my-files||", &[100, 10], 2000.0, 21600.0, 6));
        assert!(!cache_usable(&snap(), "/other||", &[100, 10], 2000.0, 21600.0, 6));
    }
}
