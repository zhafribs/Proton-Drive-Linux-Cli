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
use std::collections::{HashMap, HashSet};

/// Result of a remote scan: the tree plus whether a full walk ran (false =
/// the cached walk was reused on the `fs size` fingerprint).
pub struct RemoteScan {
    pub tree: RemoteTree,
    pub full: bool,
}

/// `fs size` fingerprint of the remote root: `[size, numberOfDescendants]`.
/// One CLI call replaces a per-folder walk when it matches the last one.
pub fn remote_fingerprint(cli: &Cli, remote_root: &str) -> Result<Vec<i64>> {
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
    if !cli::cfg_flag("PROTON_SYNC_FAST_REMOTE", true) || !filter.is_unfiltered() {
        return Ok(None);
    }
    let snap_path = crate::health::state_dir().join("remote-snapshot.json");
    let Some(snap) = proton_core::snapshot::read(&snap_path) else {
        return Ok(None);
    };
    let fp = remote_fingerprint(cli, remote_root)?;
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

fn parse_listing(
    value: &serde_json::Value,
    rel: &str,
    filter: &Filter,
) -> Vec<(String, RemoteEntry)> {
    let serde_json::Value::Array(arr) = value else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in arr {
        let Some(obj) = e.as_object() else { continue };
        if !obj.contains_key("type") {
            continue;
        }
        let typ = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let name_obj = obj.get("name");
        let ok = name_obj
            .and_then(|n| n.get("ok"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let name = name_obj
            .and_then(|n| n.get("value"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !ok || name.is_empty() || Filter::skip_name(name) {
            continue;
        }
        let child_rel = if rel.is_empty() {
            name.to_string()
        } else {
            format!("{rel}/{name}")
        };
        if filter.excluded(&child_rel) {
            continue;
        }
        let media = obj.get("mediaType").and_then(|v| v.as_str()).unwrap_or("");
        let size = obj
            .get("activeRevision")
            .and_then(|r| r.get("claimedSize"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let uid = obj.get("uid").and_then(|v| v.as_str()).map(str::to_string);
        let mtime = obj
            .get("modificationTime")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        if typ == "folder" {
            if !filter.allowed_dir(&child_rel) {
                continue;
            }
            out.push((
                child_rel,
                RemoteEntry { uid, kind: "dir".to_string(), mtime, size: Some(0), doc: false },
            ));
        } else if typ == "file" {
            if !filter.allowed_file(&child_rel) {
                continue;
            }
            out.push((
                child_rel,
                RemoteEntry {
                    uid,
                    kind: "file".to_string(),
                    mtime,
                    size: Some(size),
                    doc: media.starts_with("application/vnd.proton"),
                },
            ));
        }
    }
    out
}

/// Direct children of one remote folder, already filtered and sorted by the
/// CLI.
pub fn list_children(
    cli: &Cli,
    remote_root: &str,
    rel: &str,
    filter: &Filter,
) -> Result<Vec<(String, RemoteEntry)>> {
    let rpath = cli::remote_path(remote_root, rel);
    // Caller holds the app-wide CLI lock (see `scoped_remote`).
    let value = cli.run_json(&["fs", "list", &rpath], false)?;
    Ok(parse_listing(&value, rel, filter))
}

/// Recursively list a folder subtree (used for a folder that just appeared or
/// was replaced). Returns descendants only; the caller inserts the folder node.
pub fn scan_scope_tree(
    cli: &Cli,
    remote_root: &str,
    rel: &str,
    filter: &Filter,
) -> Result<RemoteTree> {
    let mut tree = RemoteTree::new();
    let mut level = vec![rel.to_string()];
    while !level.is_empty() {
        let mut next = Vec::new();
        for parent in level {
            for (child_rel, entry) in list_children(cli, remote_root, &parent, filter)? {
                if entry.kind == "dir" {
                    next.push(child_rel.clone());
                }
                tree.insert(child_rel, entry);
            }
        }
        level = next;
    }
    Ok(tree)
}

fn remove_subtree(tree: &mut RemoteTree, key: &str) {
    let prefix = format!("{key}/");
    let keys: Vec<String> = tree
        .range(key.to_string()..)
        .take_while(|(k, _)| *k == key || k.starts_with(&prefix))
        .map(|(k, _)| k.clone())
        .collect();
    for k in keys {
        tree.remove(&k);
    }
}

fn move_subtree(tree: &mut RemoteTree, old: &str, new: &str) {
    let prefix = format!("{old}/");
    let keys: Vec<String> = tree
        .range(old.to_string()..)
        .take_while(|(k, _)| *k == old || k.starts_with(&prefix))
        .map(|(k, _)| k.clone())
        .collect();
    for k in keys {
        let Some(value) = tree.remove(&k) else { continue };
        let new_key = if k == old {
            new.to_string()
        } else {
            format!("{new}{}", &k[old.len()..])
        };
        tree.insert(new_key, value);
    }
}

/// Refresh one changed folder in place, using its direct listing plus a scan
/// of any newly appeared subfolders. Unchanged subtrees are reused. Returns
/// an error so the caller can fall back to a full walk rather than ship a
/// partial snapshot.
pub fn apply_scope(
    cli: &Cli,
    remote_root: &str,
    filter: &Filter,
    tree: &mut RemoteTree,
    rel: &str,
) -> Result<()> {
    let fresh = list_children(cli, remote_root, rel, filter)?;
    let prefix = if rel.is_empty() {
        String::new()
    } else {
        format!("{rel}/")
    };

    let mut old_children: std::collections::HashMap<String, (String, Option<String>)> =
        std::collections::HashMap::new();
    for (key, node) in tree.iter() {
        if let Some(rest) = key.strip_prefix(&prefix)
            && !rest.is_empty()
            && !rest.contains('/')
        {
            old_children.insert(rest.to_string(), (key.clone(), node.uid.clone()));
        }
    }
    let old_by_uid: std::collections::HashMap<String, String> = old_children
        .iter()
        .filter_map(|(name, (_, uid))| uid.as_ref().map(|u| (u.clone(), name.clone())))
        .collect();
    let fresh_names: std::collections::HashSet<String> = fresh
        .iter()
        .map(|(r, _)| r.rsplit('/').next().unwrap_or(r).to_string())
        .collect();
    let fresh_uids: std::collections::HashSet<String> = fresh
        .iter()
        .filter_map(|(_, n)| n.uid.clone())
        .collect();

    for (child_rel, node) in &fresh {
        let name = child_rel.rsplit('/').next().unwrap_or(child_rel).to_string();
        let uid = node.uid.clone();
        let prev_name = uid.as_ref().and_then(|u| old_by_uid.get(u)).cloned();
        let same_name = old_children.get(&name).cloned();

        if let Some(prev) = prev_name
            && prev != name
        {
            move_subtree(tree, &format!("{prefix}{prev}"), child_rel);
        } else if let Some((old_key, old_uid)) = same_name
            && uid.is_some()
            && old_uid.is_some()
            && uid != old_uid
        {
            remove_subtree(tree, &old_key);
        }

        let existed = tree.contains_key(child_rel);
        tree.insert(child_rel.clone(), node.clone());
        if node.kind == "dir" && !existed {
            let subtree = scan_scope_tree(cli, remote_root, child_rel, filter)?;
            tree.extend(subtree);
        }
    }

    for (name, (key, uid)) in &old_children {
        let present = fresh_names.contains(name)
            || uid.as_ref().is_some_and(|u| fresh_uids.contains(u));
        if !present {
            remove_subtree(tree, key);
        }
    }
    for (child_rel, _) in fresh {
        if !tree.contains_key(&child_rel) {
            anyhow::bail!("scope merge incomplete for {child_rel}");
        }
    }
    Ok(())
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
                let entries = entries?;
                for (child_rel, entry) in parse_listing(&entries, &rel, filter) {
                    let is_dir = entry.kind == "dir";
                    let name = child_rel.rsplit('/').next().unwrap_or(&child_rel).to_string();
                    tree.insert(child_rel.clone(), entry);
                    if is_dir {
                        let child_rpath = format!("{}/{}", rpath, cli::escape(&name));
                        rel_of.insert(child_rpath.clone(), child_rel);
                        next.push(child_rpath);
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

#[cfg(test)]
mod scope_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn entry(uid: &str, kind: &str) -> RemoteEntry {
        RemoteEntry {
            uid: Some(uid.into()),
            kind: kind.into(),
            mtime: Some("2026-10-31T00:00:00Z".into()),
            size: Some(0),
            doc: false,
        }
    }

    #[test]
    fn scope_merge_adds_renames_and_drops_children() {
        let dir = std::env::temp_dir().join(format!("pd-scope-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("proton-drive");
        std::fs::write(
            &script,
            r#"#!/bin/sh
case "$3" in
  /my-files/Docs)
    cat <<'JSON'
[
 {"type":"folder","name":{"ok":true,"value":"Sub"},"uid":"vol~sub","modificationTime":"2026-10-31T00:00:00Z"},
 {"type":"folder","name":{"ok":true,"value":"NewDir"},"uid":"vol~newdir","modificationTime":"2026-10-31T00:00:00Z"},
 {"type":"file","name":{"ok":true,"value":"renamed.txt"},"uid":"vol~old","activeRevision":{"claimedSize":12},"modificationTime":"2026-10-31T00:00:00Z"},
 {"type":"file","name":{"ok":true,"value":"new.txt"},"uid":"vol~new","activeRevision":{"claimedSize":7},"modificationTime":"2026-10-31T00:00:00Z"}
]
JSON
    ;;
  /my-files/Docs/NewDir)
    echo '[{"type":"file","name":{"ok":true,"value":"inside.txt"},"uid":"vol~inside","activeRevision":{"claimedSize":3},"modificationTime":"2026-10-31T00:00:00Z"}]'
    ;;
  *) echo '[]' ;;
esac
"#,
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let mut tree = RemoteTree::new();
        tree.insert("Docs".into(), entry("vol~docs", "dir"));
        tree.insert("Docs/old.txt".into(), entry("vol~old", "file"));
        tree.insert("Docs/Sub".into(), entry("vol~sub", "dir"));
        tree.insert("Docs/Sub/a.txt".into(), entry("vol~a", "file"));
        tree.insert("Docs/gone.txt".into(), entry("vol~gone", "file"));

        let cli = Cli::for_test(script);
        let filter = Filter::new("", "");
        apply_scope(&cli, "/my-files", &filter, &mut tree, "Docs").unwrap();

        assert_eq!(tree.get("Docs/renamed.txt").unwrap().uid.as_deref(), Some("vol~old"));
        assert!(!tree.contains_key("Docs/old.txt"));
        assert!(tree.contains_key("Docs/new.txt"));
        assert!(tree.contains_key("Docs/Sub/a.txt"));
        assert!(!tree.contains_key("Docs/gone.txt"));
        assert!(tree.contains_key("Docs/NewDir/inside.txt"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
