//! Execute planned actions via the official CLI.
//! Mirrors Python's execute() behavior closely.

use crate::action::{Action, LocalTree};
use crate::cli::{self, Cli};
use crate::filter::Filter;
use anyhow::Result;
use proton_core::snapshot::{RemoteEntry, RemoteTree};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

fn state_dir() -> std::path::PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_STATE_HOME") {
        std::path::PathBuf::from(xdg).join("proton-sync")
    } else {
        dirs::home_dir().unwrap_or_else(|| std::path::Path::new("/").to_path_buf()).join(".local/state/proton-sync")
    }
}

fn parent_of(rel: &str) -> String {
    if let Some(pos) = rel.rfind('/') {
        rel[..pos].to_string()
    } else {
        String::new()
    }
}

fn name_of(rel: &str) -> String {
    if let Some(pos) = rel.rfind('/') {
        rel[pos + 1..].to_string()
    } else {
        rel.to_string()
    }
}

pub fn ensure_remote_dir(cli: &Cli, remote_root: &str, rel: &str, tree: &mut RemoteTree, created: &mut BTreeSet<String>) -> Result<()> {
    if rel.is_empty() || tree.contains_key(rel) {
        return Ok(());
    }
    let parent = parent_of(rel);
    if !parent.is_empty() {
        ensure_remote_dir(cli, remote_root, &parent, tree, created)?;
    }
    let name = name_of(rel);
    let parent_rpath = cli::remote_path(remote_root, &parent);
    let _ = cli.run(&["fs", "create-folder", &parent_rpath, &cli::escape(&name)], false, Some(120), true);
    tree.insert(rel.to_string(), RemoteEntry {
        uid: None,
        kind: "dir".to_string(),
        mtime: None,
        size: Some(0),
        doc: false,
    });
    created.insert(rel.to_string());
    eprintln!("MKDIR  remote {}", rel);
    Ok(())
}

fn trash_local_path(rel: &str, local_root: &Path) -> Result<()> {
    let full = local_root.join(rel);
    // move to .Trash-<pid> like simple approach? python uses gio/trash; for now move to trash dir
    let trash_dir = state_dir().join("trash");
    fs::create_dir_all(&trash_dir)?;
    let dst = trash_dir.join(name_of(rel));
    let _ = fs::rename(&full, &dst);
    eprintln!("TRASH  local {}", rel);
    Ok(())
}

pub fn execute(
    actions: &[Action],
    remote: &RemoteTree,
    _local: &LocalTree,
    remote_root: &str,
    local_root: &Path,
    _filter: &Filter,
) -> Result<(BTreeSet<String>, BTreeSet<String>)> {
    let cli = Cli::new()?;
    let mut tree: RemoteTree = remote.clone();
    let mut created_dirs: BTreeSet<String> = BTreeSet::new();
    let mut failed: BTreeSet<String> = BTreeSet::new();
    let mut downloaded: BTreeSet<String> = BTreeSet::new();
    let mut deferred_up: Vec<String> = Vec::new();
    let mut deferred_dl: Vec<String> = Vec::new();

    for action in actions {
        match action {
            Action::Up { rel, node } => {
                if !node.is_file() { continue; }
                deferred_up.push(rel.clone());
            }
            Action::Dl { rel, node } if node.kind != "dir" => {
                deferred_dl.push(rel.clone());
            }
            Action::Dl { rel, node } => {
                if node.kind == "dir" {
                    // Only the folder itself: its files are planned and
                    // downloaded one by one, so a subtree download here would
                    // fetch every file again for each folder above it.
                    if let Err(e) = fs::create_dir_all(local_root.join(rel)) {
                        eprintln!("FAILED folder {}: {}", rel, e);
                        failed.insert(rel.clone());
                    }
                    continue;
                }
            }
            Action::TrashLocal { rel, .. } => {
                if trash_local_path(rel, local_root).is_err() {
                    failed.insert(rel.clone());
                }
            }
            Action::TrashRemote { rel, .. } => {
                let rpath = cli::remote_path(remote_root, rel);
                let res = cli.run(&["fs", "trash", &rpath], false, Some(300), true);
                if res.is_err() {
                    failed.insert(rel.clone());
                } else {
                    eprintln!("TRASH  remote {}", rel);
                }
            }
            Action::MvRemote { old, new } => {
                let old_rpath = cli::remote_path(remote_root, old);
                let parent_rpath = cli::remote_path(remote_root, &parent_of(new));
                let name = name_of(new);
                let res = cli.run(&["fs", "move", &old_rpath, &parent_rpath, &cli::escape(&name)], false, Some(300), true);
                if res.is_err() {
                    failed.insert(old.clone());
                    failed.insert(new.clone());
                } else {
                    eprintln!("MOVE   remote {} -> {}", old, new);
                }
            }
            Action::MvLocal { old, new } => {
                let old_full = local_root.join(old);
                let new_full = local_root.join(new);
                if let Some(p) = new_full.parent() {
                    let _ = fs::create_dir_all(p);
                }
                let res = fs::rename(&old_full, &new_full);
                if res.is_err() {
                    failed.insert(old.clone());
                    failed.insert(new.clone());
                } else {
                    eprintln!("MOVE   local {} -> {}", old, new);
                }
            }
            Action::Conflict { rel, .. } => {
                // save remote as conflict, upload local
                let ts = chrono::Utc::now().format("%Y-%m-%d %H-%M-%S").to_string();
                let base = name_of(rel);
                let parts: Vec<&str> = base.rsplitn(2, '.').collect();
                let (stem, ext) = if parts.len() == 2 && !parts[0].is_empty() { (parts[1], format!(".{}", parts[0])) } else { (base.as_str(), String::new()) };
                let dir = parent_of(rel);
                let conflict_name = if dir.is_empty() { format!("{} (conflict {}){}", stem, ts, ext) } else { format!("{}/{} (conflict {}){}", dir, stem, ts, ext) };
                // download remote version to conflict path
                let _ = cli.run(&["fs", "download", &cli::remote_path(remote_root, rel), &local_root.join(&dir).to_string_lossy(), "-f", "remove", "-d", "merge"], false, Some(3600), true);
                // rename downloaded to conflict name (best effort)
                let _ = fs::rename(local_root.join(&dir).join(&base), local_root.join(&conflict_name));
                eprintln!("CONFLICT {} (kept remote copy as {})", rel, conflict_name);
                // upload local as canonical
                let parent_rpath = cli::remote_path(remote_root, &dir);
                let _ = cli.run(&["fs", "upload", &local_root.join(rel).to_string_lossy(), &parent_rpath, "-f", "create-new-revision", "-d", "merge"], false, Some(3600), true);
            }
        }
    }
    let transfers = cli::cfg_int("PROTON_SYNC_TRANSFERS", 4, 10);
    let mut batches: Vec<Batch> = Vec::new();
    let mut up_by_parent: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
    for rel in &deferred_up {
        up_by_parent.entry(parent_of(rel)).or_default().push(rel.clone());
    }
    for (parent, rels) in &up_by_parent {
        if !parent.is_empty() {
            ensure_remote_dir(&cli, remote_root, parent, &mut tree, &mut created_dirs)?;
        }
        for chunk in rels.chunks(MAX_BATCH) {
            batches.push(Batch { upload: true, parent: parent.clone(), rels: chunk.to_vec() });
        }
    }
    let mut dl_by_parent: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
    for rel in &deferred_dl {
        dl_by_parent.entry(parent_of(rel)).or_default().push(rel.clone());
    }
    for (parent, rels) in &dl_by_parent {
        fs::create_dir_all(local_root.join(parent))?;
        for chunk in rels.chunks(MAX_BATCH) {
            batches.push(Batch { upload: false, parent: parent.clone(), rels: chunk.to_vec() });
        }
    }

    let total_files: u64 = batches.iter().map(|b| b.rels.len() as u64).sum();
    let done_files = std::sync::atomic::AtomicU64::new(0);
    report_transfer(0, total_files);
    let outcomes = cli::with_cli_lock(|| {
        let _ = cli.run(&["fs", "info", remote_root], true, Some(120), false);
        Ok(cli::run_parallel(batches, transfers, |b| {
            let n = b.rels.len() as u64;
            let res = if b.upload {
                let locals: Vec<String> = b.rels.iter().map(|r| local_root.join(r).to_string_lossy().into_owned()).collect();
                let parent_rpath = cli::remote_path(remote_root, &b.parent);
                let mut args: Vec<&str> = vec!["fs", "upload", "-f", "create-new-revision", "-d", "merge"];
                args.extend(locals.iter().map(|s| s.as_str()));
                args.push(&parent_rpath);
                cli.run(&args, false, Some(3600), false)
            } else {
                let rpaths: Vec<String> = b.rels.iter().map(|r| cli::remote_path(remote_root, r)).collect();
                let target = local_root.join(&b.parent).to_string_lossy().into_owned();
                let mut args: Vec<&str> = vec!["fs", "download"];
                args.extend(rpaths.iter().map(|s| s.as_str()));
                args.extend(["-f", "remove", "-d", "merge"]);
                args.push(&target);
                cli.run(&args, false, Some(3600), false)
            };
            let done = done_files.fetch_add(n, std::sync::atomic::Ordering::Relaxed) + n;
            report_transfer(done, total_files);
            (b, res)
        }))
    })?;
    report_transfer(total_files, total_files);

    for (b, res) in outcomes {
        match res {
            Ok(_) => {
                for rel in &b.rels {
                    if b.upload {
                        eprintln!("UPLOAD {}", rel);
                    } else {
                        eprintln!("DOWNLOAD {}", rel);
                        downloaded.insert(rel.clone());
                    }
                }
            }
            Err(e) => {
                eprintln!("FAILED transfer batch ({} path(s)): {}", b.rels.len(), e);
                failed.extend(b.rels.iter().cloned());
            }
        }
    }
    Ok((failed, downloaded))
}

const MAX_BATCH: usize = 50;

/// Progress for a transfer whose total is not known yet (the streaming first
/// download). Keeps the total the file already carries.
pub fn report_progress(done: u64) {
    let path = crate::health::health_path();
    let mut h = proton_core::health::read(&path);
    h.status = "syncing".to_string();
    h.transfer_active = Some(true);
    h.transfer_done = Some(done);
    crate::health::write_health(&h);
}

fn report_transfer(done: u64, total: u64) {
    let path = crate::health::health_path();
    let mut h = proton_core::health::read(&path);
    h.status = "syncing".to_string();
    h.transfer_active = Some(done < total);
    h.transfer_done = Some(done);
    h.transfer_total = Some(total);
    crate::health::write_health(&h);
}

struct Batch {
    upload: bool,
    parent: String,
    rels: Vec<String>,
}
