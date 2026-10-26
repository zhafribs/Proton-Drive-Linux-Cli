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
}


/// Remote tree scan using the official CLI (`fs list --json`).
use crate::cli::{self, Cli};
use anyhow::Result;
use proton_core::snapshot::{RemoteEntry, RemoteTree};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone)]
struct RemoteConfig {
    remote_root: String,
    subfolders: String,
    exclude: String,
}

pub fn scan_remote(cli: &Cli, remote_root: &str, filter: &Filter) -> Result<RemoteTree> {
    let mut tree = RemoteTree::new();
    let start_rpath = cli::remote_path(remote_root, "");
    let mut rel_of: HashMap<String, String> = HashMap::new();
    rel_of.insert(start_rpath.clone(), "".to_string());
    let mut visited = std::collections::HashSet::new();
    let mut pending: Vec<String> = vec![start_rpath.clone()];
    let filter_arc = Arc::new(filter.clone());
    let config = Arc::new(RemoteConfig {
        remote_root: remote_root.to_string(),
        subfolders: String::new(),
        exclude: String::new(),
    });
    let _ = config;
    {
        let r = remote_root.to_string();
        let c = cli.clone();
        crate::cli::with_cli_lock(move || -> Result<()> {
            let _ = c.run(&["fs", "info", &r], true, Some(120), false);
            Ok(())
        })?;
    }
    // simple sequential for now (parallel later) - but need to respect lock semantics
    while !pending.is_empty() {
        let mut next_dirs: Vec<String> = Vec::new();
        for rpath in pending.drain(..).collect::<Vec<_>>() {
            if visited.contains(&rpath) { continue; }
            // list with lock held for session safety? python holds lock for whole batch
            let rpath_clone = rpath.clone();
            let c2 = cli.clone();
            let entries_val = crate::cli::with_cli_lock(move || {
                c2.run_json(&["fs", "list", &rpath_clone], false)
            })?;
            visited.insert(rpath.clone());
            let rel = rel_of.get(&rpath).cloned().unwrap_or_default();
            if let Value::Array(arr) = entries_val {
                for e in arr {
                    if !e.is_object() { continue; }
                    let obj = e.as_object().unwrap();
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
                    let is_dir = typ == "folder";
                    if is_dir {
                        if !filter.allowed_dir(&child_rel) { continue; }
                        tree.insert(child_rel.clone(), RemoteEntry {
                            uid: obj.get("uid").and_then(|v| v.as_str()).map(|s| s.to_string()),
                            kind: "dir".to_string(),
                            mtime: obj.get("modificationTime").and_then(|v| v.as_str()).map(|s| s.to_string()),
                            size: Some(0),
                            doc: false,
                        });
                        let child_rpath = format!("{}/{}", rpath, cli::escape(name));
                        if !visited.contains(&child_rpath) {
                            rel_of.insert(child_rpath.clone(), child_rel.clone());
                            next_dirs.push(child_rpath);
                        }
                    } else if typ == "file" {
                        if !filter.allowed_file(&child_rel) { continue; }
                        let doc = media.starts_with("application/vnd.proton");
                        tree.insert(child_rel.clone(), RemoteEntry {
                            uid: obj.get("uid").and_then(|v| v.as_str()).map(|s| s.to_string()),
                            kind: "file".to_string(),
                            mtime: obj.get("modificationTime").and_then(|v| v.as_str()).map(|s| s.to_string()),
                            size: Some(size),
                            doc,
                        });
                    }
                }
            }
        }
        pending = next_dirs;
    }
    Ok(tree)
}
