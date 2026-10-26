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
    let tree: Arc<std::sync::Mutex<RemoteTree>> = Arc::new(std::sync::Mutex::new(RemoteTree::new()));
    let start_rpath = cli::remote_path(remote_root, "");
    let rel_of: Arc<std::sync::Mutex<HashMap<String, String>>> = Arc::new(std::sync::Mutex::new(HashMap::new()));
    rel_of.lock().unwrap().insert(start_rpath.clone(), "".to_string());
    let visited: Arc<std::sync::Mutex<std::collections::HashSet<String>>> = Arc::new(std::sync::Mutex::new(std::collections::HashSet::new()));
    let pending: Arc<std::sync::Mutex<Vec<String>>> = Arc::new(std::sync::Mutex::new(vec![start_rpath.clone()]));
    
    let tree_c = tree.clone();
    let rel_of_c = rel_of.clone();
    let visited_c = visited.clone();
    let pending_c = pending.clone();
    let cli_c = cli.clone();
    let filter_c = filter.clone();
    let remote_root_c = remote_root.to_string();
    crate::cli::with_cli_lock(move || -> Result<()> {
        // Probe session first under lock like Python does
        {
            let r = remote_root_c.clone();
            let c = cli_c.clone();
            let _ = c.run(&["fs", "info", &r], true, Some(120), false);
        }
        let workers = 8;
        loop {
            let to_scan: Vec<String> = {
                let mut p = pending.lock().unwrap();
                if p.is_empty() { break; }
                let v = visited.lock().unwrap();
                p.drain(..).filter(|x| !v.contains(x)).collect()
            };
            if to_scan.is_empty() { break; }
            
            use std::sync::mpsc;
            use std::thread;
            let (tx, rx) = mpsc::channel();
            let mut handles = vec![];
            for chunk in to_scan.chunks((to_scan.len() + workers - 1) / workers) {
                let chunk_vec: Vec<String> = chunk.to_vec();
                let tx_clone = tx.clone();
                let cli_clone = cli.clone();
                let rel_of_clone = rel_of.clone();
                handles.push(thread::spawn(move || {
                    for rpath in chunk_vec {
                        let rel_of_lock = rel_of_clone.lock().unwrap();
                        let rel = rel_of_lock.get(&rpath).cloned().unwrap_or_default();
                        drop(rel_of_lock);
                        let rpath_clone = rpath.clone();
                        let c2 = cli_clone.clone();
                        let entries_res = c2.run_json(&["fs", "list", &rpath_clone], false);
                        if let Ok(entries_val) = entries_res {
                            let _ = tx_clone.send((rpath, rel, entries_val));
                        }
                    }
                }));
            }
            drop(tx);
            
            let mut next_dirs: Vec<String> = Vec::new();
            for (rpath, rel, entries_val) in rx {
                visited_c.lock().unwrap().insert(rpath.clone());
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
                            if !filter_c.allowed_dir(&child_rel) { continue; }
                            tree_c.lock().unwrap().insert(child_rel.clone(), RemoteEntry {
                                uid: obj.get("uid").and_then(|v| v.as_str()).map(|s| s.to_string()),
                                kind: "dir".to_string(),
                                mtime: obj.get("modificationTime").and_then(|v| v.as_str()).map(|s| s.to_string()),
                                size: Some(0),
                                doc: false,
                            });
                            let child_rpath = format!("{}/{}", rpath, crate::cli::escape(name));
                            if !visited.lock().unwrap().contains(&child_rpath) {
                                rel_of.lock().unwrap().insert(child_rpath.clone(), child_rel.clone());
                                next_dirs.push(child_rpath);
                            }
                        } else if typ == "file" {
                            if !filter.allowed_file(&child_rel) { continue; }
                            let doc = media.starts_with("application/vnd.proton");
                            tree_c.lock().unwrap().insert(child_rel.clone(), RemoteEntry {
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
            for h in handles { let _ = h.join(); }
            if !next_dirs.is_empty() {
                pending_c.lock().unwrap().extend(next_dirs);
            }
        }
        Ok(())
    })?;
    Ok(std::sync::Arc::try_unwrap(tree).unwrap().into_inner().unwrap())
}
