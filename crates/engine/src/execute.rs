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

fn local_root_default() -> std::path::PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| std::path::Path::new("/").to_path_buf());
    let config_dir = dirs::config_dir().unwrap_or_else(|| home.join(".config"));
    let cfg = proton_core::conf::read(&config_dir.join("proton-sync.conf"));
    if let Some(v) = cfg.get("PROTON_SYNC_LOCAL") {
        return std::path::PathBuf::from(v).expanduser();
    }
    home.join("ProtonDrive")
}

trait ExpandUser {
    fn expanduser(&self) -> std::path::PathBuf;
}
impl ExpandUser for std::path::PathBuf {
    fn expanduser(&self) -> std::path::PathBuf {
        let s = self.to_string_lossy();
        if s.starts_with("~/") {
            if let Some(home) = dirs::home_dir() {
                return home.join(&s[2..]);
            }
        }
        self.clone()
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
    local: &LocalTree,
    remote_root: &str,
    local_root: &Path,
    filter: &Filter,
) -> Result<(BTreeSet<String>, BTreeSet<String>)> {
    let cli = Cli::new()?;
    let mut tree: RemoteTree = remote.clone();
    let mut created_dirs: BTreeSet<String> = BTreeSet::new();
    let mut failed: BTreeSet<String> = BTreeSet::new();
    let mut downloaded: BTreeSet<String> = BTreeSet::new();

    for action in actions {
        match action {
            Action::Up { rel, node } => {
                if !node.is_file() { continue; }
                let parent = parent_of(rel);
                if !parent.is_empty() {
                    ensure_remote_dir(&cli, remote_root, &parent, &mut tree, &mut created_dirs)?;
                }
                let full = local_root.join(rel);
                let parent_rpath = cli::remote_path(remote_root, &parent);
                let res = cli.run(&["fs", "upload", &full.to_string_lossy(), &parent_rpath, "-f", "create-new-revision", "-d", "skip-identical"], false, Some(3600), true);
                if res.is_err() {
                    failed.insert(rel.clone());
                    continue;
                }
                eprintln!("UPLOAD {}", rel);
            }
            Action::Dl { rel, node } => {
                if node.kind == "dir" {
                    // download whole subtree - simplified
                    let parent_rpath = cli::remote_path(remote_root, rel);
                    let _ = local_root.join(rel).as_path();
                    let res = cli.run(&["fs", "download", &parent_rpath, &local_root.to_string_lossy(), "-f", "skip", "-d", "skip-identical"], false, Some(3600), true);
                    if res.is_err() {
                        failed.insert(rel.clone());
                        continue;
                    }
                    eprintln!("DOWNLOAD dir {} (whole subtree)", rel);
                    downloaded.insert(rel.clone());
                    continue;
                }
                let parent_rpath_parent = cli::remote_path(remote_root, &parent_of(rel));
                let res = cli.run(&["fs", "download", &cli::remote_path(remote_root, rel), &local_root.to_string_lossy(), "-f", "skip", "-d", "skip-identical"], false, Some(3600), true);
                if res.is_err() {
                    failed.insert(rel.clone());
                    continue;
                }
                eprintln!("DOWNLOAD {}", rel);
                downloaded.insert(rel.clone());
            }
            Action::TrashLocal { rel, .. } => {
                if let Err(_) = trash_local_path(rel, local_root) {
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
                let _ = cli.run(&["fs", "download", &cli::remote_path(remote_root, rel), &local_root.join(&dir).to_string_lossy(), "-f", "skip", "-d", "skip-identical"], false, Some(3600), true);
                // rename downloaded to conflict name (best effort)
                let _ = fs::rename(local_root.join(&dir).join(&base), local_root.join(&conflict_name));
                eprintln!("CONFLICT {} (kept remote copy as {})", rel, conflict_name);
                // upload local as canonical
                let parent_rpath = cli::remote_path(remote_root, &dir);
                let _ = cli.run(&["fs", "upload", &local_root.join(rel).to_string_lossy(), &parent_rpath, "-f", "create-new-revision", "-d", "skip-identical"], false, Some(3600), true);
            }
        }
    }
    Ok((failed, downloaded))
}
