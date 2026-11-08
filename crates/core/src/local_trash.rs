//! The local trash manifest.
//!
//! Files deleted locally are moved to `<state>/trash/<relative path>` so a
//! restore can put them back without guessing. The manifest records the
//! original path, time, size and kind; a legacy scan still finds entries
//! written before the manifest existed. All writes are atomic, and a lock
//! serialises the engine (which records a trash) and the GUI (which restores
//! or permanently deletes one).

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const VERSION: u32 = 1;
const LOCK_TIMEOUT: Duration = Duration::from_secs(5);

/// One local deletion. `stored_rel` is the path under the trash directory;
/// `original_rel` is where the file came from, relative to the sync root.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub stored_rel: String,
    pub original_rel: String,
    pub trashed_at: u64,
    pub size: u64,
    /// `file` or `dir`.
    pub kind: String,
    /// True for an entry found on disk without manifest metadata. Its
    /// `original_rel` is the stored path, which may include a collision suffix.
    #[serde(default)]
    pub legacy: bool,
}

#[derive(Serialize, Deserialize)]
struct Manifest {
    version: u32,
    #[serde(default)]
    entries: Vec<Entry>,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            version: VERSION,
            entries: Vec::new(),
        }
    }
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn trash_dir(state_dir: &Path) -> PathBuf {
    state_dir.join("trash")
}

pub fn manifest_path(state_dir: &Path) -> PathBuf {
    state_dir.join("trash-manifest.json")
}

fn lock_path(state_dir: &Path) -> PathBuf {
    state_dir.join("trash-manifest.lock")
}

/// A cloud-supplied relative path is safe only if every component is a plain
/// name, and the same rule applies to a manifest written by an older build.
pub fn safe_rel(rel: &str) -> bool {
    !rel.is_empty()
        && !rel.starts_with('/')
        && !rel.contains('\0')
        && rel
            .split('/')
            .all(|c| !c.is_empty() && c != "." && c != "..")
}

fn is_busy(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::WouldBlock || e.raw_os_error() == Some(11)
}

fn lock_timeout(state_dir: &Path, timeout: Duration) -> Result<File, String> {
    let path = lock_path(state_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("creating {}: {e}", parent.display()))?;
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(&path)
        .map_err(|e| format!("opening {}: {e}", path.display()))?;
    let deadline = Instant::now() + timeout;
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(file),
            Err(e) if is_busy(&e) => {
                if Instant::now() >= deadline {
                    return Err("the trash manifest is busy; try again in a moment".into());
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => return Err(format!("locking the trash manifest: {e}")),
        }
    }
}

fn lock(state_dir: &Path) -> Result<File, String> {
    lock_timeout(state_dir, LOCK_TIMEOUT)
}

fn load_unlocked(state_dir: &Path) -> Result<Manifest, String> {
    let path = manifest_path(state_dir);
    match fs::read(&path) {
        Ok(bytes) => {
            let manifest: Manifest = serde_json::from_slice(&bytes)
                .map_err(|e| format!("reading {}: {e}", path.display()))?;
            if manifest.version != VERSION {
                return Err(format!(
                    "{} has unsupported version {}",
                    path.display(),
                    manifest.version
                ));
            }
            Ok(manifest)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Manifest::default()),
        Err(e) => Err(format!("reading {}: {e}", path.display())),
    }
}

fn write_unlocked(state_dir: &Path, manifest: &Manifest) -> Result<(), String> {
    let path = manifest_path(state_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("creating {}: {e}", parent.display()))?;
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = path.with_file_name(format!(
        ".trash-manifest.{}.{}.tmp",
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
        serde_json::to_writer_pretty(&mut file, manifest)
            .map_err(|e| format!("writing {}: {e}", tmp.display()))?;
        file.write_all(b"\n")
            .map_err(|e| format!("writing {}: {e}", tmp.display()))?;
        file.sync_all()
            .map_err(|e| format!("syncing {}: {e}", tmp.display()))?;
        drop(file);
        fs::rename(&tmp, &path).map_err(|e| format!("replacing {}: {e}", path.display()))?;
        if let Some(parent) = path.parent()
            && let Ok(dir) = File::open(parent)
        {
            let _ = dir.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Record one successfully moved local deletion. A missing manifest write is
/// not fatal for the moved data: the legacy scan will still see the file.
pub fn record(state_dir: &Path, entry: &Entry) -> Result<(), String> {
    if !safe_rel(&entry.stored_rel) || !safe_rel(&entry.original_rel) {
        return Err("trash entry has an unsafe path".into());
    }
    // The engine records a trash while the GUI may be scanning or
    // restoring. Waiting is not worth stalling a sync cycle: a missed
    // manifest entry is still visible through the legacy scan.
    let _lock = lock_timeout(state_dir, Duration::from_millis(200))?;
    let mut manifest = load_unlocked(state_dir)?;
    if let Some(existing) = manifest
        .entries
        .iter_mut()
        .find(|e| e.stored_rel == entry.stored_rel)
    {
        *existing = entry.clone();
    } else {
        manifest.entries.push(entry.clone());
    }
    write_unlocked(state_dir, &manifest)
}

fn scan_orphans(
    trash: &Path,
    rel: &str,
    seen: &BTreeSet<String>,
    out: &mut Vec<(String, fs::Metadata)>,
) {
    let dir = if rel.is_empty() {
        trash.to_path_buf()
    } else {
        trash.join(rel)
    };
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
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
        let Ok(meta) = fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if meta.is_dir() && !meta.file_type().is_symlink() {
            scan_orphans(trash, &child, seen, out);
        }
        if !seen.contains(&child) {
            out.push((child, meta));
        }
    }
}

/// Every restorable local deletion, newest first. Manifest entries are
/// authoritative; files written before the manifest are scanned from disk
/// and marked `legacy`. A trashed directory is one row, not one row per child.
pub fn entries(state_dir: &Path) -> Result<Vec<Entry>, String> {
    let _lock = lock(state_dir)?;
    let manifest = load_unlocked(state_dir)?;
    let trash = trash_dir(state_dir);
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    let mut dirs = BTreeSet::new();

    for entry in &manifest.entries {
        if !safe_rel(&entry.stored_rel) {
            continue;
        }
        let full = trash.join(&entry.stored_rel);
        if fs::symlink_metadata(&full).is_err() {
            continue; // removed outside the app
        }
        seen.insert(entry.stored_rel.clone());
        if entry.kind == "dir" {
            dirs.insert(entry.stored_rel.clone());
        }
        out.push(entry.clone());
    }

    let mut orphans = Vec::new();
    scan_orphans(&trash, "", &seen, &mut orphans);
    for (rel, meta) in &orphans {
        if meta.is_dir() && !meta.file_type().is_symlink() {
            dirs.insert(rel.clone());
        }
    }
    for (rel, meta) in orphans {
        if dirs
            .iter()
            .any(|d| rel != *d && rel.starts_with(&format!("{d}/")))
        {
            continue;
        }
        // An unrecorded parent directory that contains a manifest entry is
        // just the path scaffolding for that file, not a second deletion.
        if meta.is_dir()
            && seen
                .iter()
                .any(|s| s.starts_with(&format!("{rel}/")))
        {
            continue;
        }
        let kind = if meta.is_dir() { "dir" } else { "file" };
        out.push(Entry {
            stored_rel: rel.clone(),
            original_rel: rel,
            trashed_at: meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0),
            size: if meta.is_file() { meta.len() } else { 0 },
            kind: kind.to_string(),
            legacy: true,
        });
    }

    out.sort_by(|a, b| {
        b.trashed_at
            .cmp(&a.trashed_at)
            .then_with(|| a.original_rel.cmp(&b.original_rel))
    });
    Ok(out)
}

fn ensure_parent(root: &Path, rel: &str) -> Result<(), String> {
    let meta = fs::symlink_metadata(root)
        .map_err(|e| format!("reading sync folder {}: {e}", root.display()))?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err(format!("sync folder is not a directory: {}", root.display()));
    }
    let Some(pos) = rel.rfind('/') else {
        return Ok(());
    };
    let mut cur = root.to_path_buf();
    for part in rel[..pos].split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err("trash entry has an unsafe path".into());
        }
        cur.push(part);
        match fs::symlink_metadata(&cur) {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
            Ok(_) => return Err(format!("path component is not a folder: {}", cur.display())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&cur)
                    .map_err(|e| format!("creating {}: {e}", cur.display()))?;
            }
            Err(e) => return Err(format!("reading {}: {e}", cur.display())),
        }
    }
    Ok(())
}

fn copy_file_no_replace(src: &Path, dest: &Path, meta: &fs::Metadata) -> Result<(), String> {
    let mut created = false;
    let result = (|| -> Result<(), String> {
        let mut input =
            File::open(src).map_err(|e| format!("reading {}: {e}", src.display()))?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(meta.permissions().mode())
            .open(dest)
            .map_err(|e| format!("creating {}: {e}", dest.display()))?;
        created = true;
        io::copy(&mut input, &mut output)
            .map_err(|e| format!("copying to {}: {e}", dest.display()))?;
        output
            .sync_all()
            .map_err(|e| format!("syncing {}: {e}", dest.display()))?;
        drop(output);
        fs::set_permissions(dest, meta.permissions())
            .map_err(|e| format!("setting permissions on {}: {e}", dest.display()))?;
        Ok(())
    })();
    if result.is_err() && created {
        let _ = fs::remove_file(dest);
    }
    result
}

fn copy_dir_no_replace(src: &Path, dest: &Path, meta: &fs::Metadata) -> Result<(), String> {
    let mut created = false;
    let result = (|| -> Result<(), String> {
        fs::create_dir(dest).map_err(|e| format!("creating {}: {e}", dest.display()))?;
        created = true;
        let children = fs::read_dir(src).map_err(|e| format!("reading {}: {e}", src.display()))?;
        for child in children {
            let child = child.map_err(|e| format!("reading {}: {e}", src.display()))?;
            let name = child.file_name();
            let from = child.path();
            let to = dest.join(&name);
            let child_meta = fs::symlink_metadata(&from)
                .map_err(|e| format!("reading {}: {e}", from.display()))?;
            if child_meta.file_type().is_symlink() {
                return Err(format!("refusing a symlink in the trash: {}", from.display()));
            }
            if child_meta.is_dir() {
                copy_dir_no_replace(&from, &to, &child_meta)?;
            } else {
                copy_file_no_replace(&from, &to, &child_meta)?;
            }
        }
        fs::set_permissions(dest, meta.permissions())
            .map_err(|e| format!("setting permissions on {}: {e}", dest.display()))?;
        Ok(())
    })();
    if result.is_err() && created {
        let _ = fs::remove_dir_all(dest);
    }
    result
}

/// Move out of the trash without ever replacing a destination that appeared
/// after the caller's existence check. Files try a hard link first (atomic
/// no-replace), then a create-new copy for filesystems without links. A
/// directory is created with `create_dir`, which also fails on an existing
/// path, then its children are copied.
fn move_out(src: &Path, dest: &Path, meta: &fs::Metadata) -> Result<(), String> {
    if meta.is_dir() {
        copy_dir_no_replace(src, dest, meta)?;
        fs::remove_dir_all(src).map_err(|e| format!("removing {}: {e}", src.display()))?;
        return Ok(());
    }
    if fs::hard_link(src, dest).is_err() {
        copy_file_no_replace(src, dest, meta)?;
    }
    fs::remove_file(src).map_err(|e| format!("removing {}: {e}", src.display()))?;
    Ok(())
}

fn prune_empty(trash: &Path, from: Option<&Path>) {
    let mut cur = from;
    while let Some(dir) = cur {
        if dir == trash {
            break;
        }
        if fs::remove_dir(dir).is_err() {
            break;
        }
        cur = dir.parent();
    }
}

fn remove_manifest_entry(manifest: &mut Manifest, stored_rel: &str) {
    let prefix = format!("{stored_rel}/");
    manifest
        .entries
        .retain(|e| e.stored_rel != stored_rel && !e.stored_rel.starts_with(&prefix));
}

/// Move a local path into the trash and record it. Used by the Recovery page
/// when a conflict action needs to back up the canonical file. A manifest
/// write failure is not fatal: the file is already in the trash, and the
/// legacy scan still finds it.
pub fn trash_local(state_dir: &Path, local_root: &Path, rel: &str) -> Result<Entry, String> {
    if !safe_rel(rel) {
        return Err("refusing to trash an unsafe path".into());
    }
    let full = local_root.join(rel);
    let meta = fs::symlink_metadata(&full)
        .map_err(|e| format!("reading {}: {e}", full.display()))?;
    if meta.file_type().is_symlink() {
        return Err(format!("refusing to trash a symlink: {}", full.display()));
    }
    let trash = trash_dir(state_dir);
    let mut stored_rel = rel.to_string();
    let mut suffix = 1u32;
    while trash.join(&stored_rel).exists() {
        suffix += 1;
        stored_rel = format!("{rel}.{suffix}");
    }
    let dst = trash.join(&stored_rel);
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("creating {}: {e}", parent.display()))?;
    }
    move_out(&full, &dst, &meta)?;
    let entry = Entry {
        stored_rel,
        original_rel: rel.to_string(),
        trashed_at: now_secs(),
        size: if meta.is_file() { meta.len() } else { 0 },
        kind: if meta.is_dir() { "dir" } else { "file" }.to_string(),
        legacy: false,
    };
    if let Err(e) = record(state_dir, &entry) {
        eprintln!("WARN trashed {rel} but could not update the manifest: {e}");
    }
    Ok(entry)
}

/// Restore one entry to its original relative path. The destination must not
/// exist, parent components must not be symlinks, and the move is done with
/// no-replace semantics even across filesystems.
pub fn restore(state_dir: &Path, local_root: &Path, entry: &Entry) -> Result<(), String> {
    if !safe_rel(&entry.stored_rel) || !safe_rel(&entry.original_rel) {
        return Err("trash entry has an unsafe path".into());
    }
    let _lock = lock(state_dir)?;
    let trash = trash_dir(state_dir);
    let src = trash.join(&entry.stored_rel);
    let meta = fs::symlink_metadata(&src)
        .map_err(|e| format!("reading {}: {e}", src.display()))?;
    if meta.file_type().is_symlink() {
        return Err(format!(
            "refusing to restore a symlink from the trash: {}",
            src.display()
        ));
    }
    ensure_parent(local_root, &entry.original_rel)?;
    let dest = local_root.join(&entry.original_rel);
    if fs::symlink_metadata(&dest).is_ok() {
        return Err(format!(
            "not restored: {} already exists",
            dest.display()
        ));
    }
    move_out(&src, &dest, &meta)?;
    let mut manifest = load_unlocked(state_dir)?;
    remove_manifest_entry(&mut manifest, &entry.stored_rel);
    if let Err(e) = write_unlocked(state_dir, &manifest) {
        eprintln!("WARN restored {} but could not update the trash manifest: {e}", entry.original_rel);
    }
    prune_empty(&trash, src.parent());
    Ok(())
}

/// Permanently delete one entry. A directory removes its subtree.
pub fn delete(state_dir: &Path, entry: &Entry) -> Result<(), String> {
    if !safe_rel(&entry.stored_rel) {
        return Err("trash entry has an unsafe path".into());
    }
    let _lock = lock(state_dir)?;
    let trash = trash_dir(state_dir);
    let src = trash.join(&entry.stored_rel);
    let meta = fs::symlink_metadata(&src)
        .map_err(|e| format!("reading {}: {e}", src.display()))?;
    if meta.is_dir() && !meta.file_type().is_symlink() {
        fs::remove_dir_all(&src).map_err(|e| format!("deleting {}: {e}", src.display()))?;
    } else {
        fs::remove_file(&src).map_err(|e| format!("deleting {}: {e}", src.display()))?;
    }
    let mut manifest = load_unlocked(state_dir)?;
    remove_manifest_entry(&mut manifest, &entry.stored_rel);
    write_unlocked(state_dir, &manifest)?;
    prune_empty(&trash, src.parent());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "pd-local-trash-{}-{}-{name}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ))
    }

    fn entry(stored: &str, original: &str, kind: &str) -> Entry {
        Entry {
            stored_rel: stored.into(),
            original_rel: original.into(),
            trashed_at: now_secs(),
            size: 3,
            kind: kind.into(),
            legacy: false,
        }
    }

    #[test]
    fn manifest_round_trip_and_restore() {
        let state = temp("state");
        let root = temp("root");
        fs::create_dir_all(trash_dir(&state).join("dir")).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(trash_dir(&state).join("dir/a.txt"), b"hey").unwrap();
        record(&state, &entry("dir/a.txt", "dir/a.txt", "file")).unwrap();

        let listed = entries(&state).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].original_rel, "dir/a.txt");

        restore(&state, &root, &listed[0]).unwrap();
        assert_eq!(fs::read(root.join("dir/a.txt")).unwrap(), b"hey");
        assert!(entries(&state).unwrap().is_empty());
        let _ = fs::remove_dir_all(&state);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn restore_refuses_to_overwrite() {
        let state = temp("state-overwrite");
        let root = temp("root-overwrite");
        fs::create_dir_all(trash_dir(&state)).unwrap();
        fs::create_dir_all(&root).unwrap();
        fs::write(trash_dir(&state).join("a.txt"), b"old").unwrap();
        fs::write(root.join("a.txt"), b"new").unwrap();
        let e = entry("a.txt", "a.txt", "file");
        assert!(restore(&state, &root, &e).is_err());
        assert_eq!(fs::read(root.join("a.txt")).unwrap(), b"new");
        assert_eq!(fs::read(trash_dir(&state).join("a.txt")).unwrap(), b"old");
        let _ = fs::remove_dir_all(&state);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn legacy_scan_lists_a_directory_once_and_delete_removes_it() {
        let state = temp("state-legacy");
        fs::create_dir_all(trash_dir(&state).join("old/sub")).unwrap();
        fs::write(trash_dir(&state).join("old/sub/a.txt"), b"x").unwrap();
        let listed = entries(&state).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].stored_rel, "old");
        assert!(listed[0].legacy);
        delete(&state, &listed[0]).unwrap();
        assert!(!trash_dir(&state).join("old").exists());
        let _ = fs::remove_dir_all(&state);
    }

    #[test]
    fn copy_fallback_is_no_replace_for_files_and_folders() {
        let base = temp("copy-fallback");
        let src = base.join("src");
        let dest = base.join("dest");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("sub/a.txt"), b"abc").unwrap();
        let meta = fs::symlink_metadata(&src).unwrap();
        copy_dir_no_replace(&src, &dest, &meta).unwrap();
        assert_eq!(fs::read(dest.join("sub/a.txt")).unwrap(), b"abc");
        assert!(copy_dir_no_replace(&src, &dest, &meta).is_err());

        let file = dest.join("sub/a.txt");
        let file_meta = fs::symlink_metadata(&file).unwrap();
        let copy = base.join("copy.txt");
        copy_file_no_replace(&file, &copy, &file_meta).unwrap();
        assert_eq!(fs::read(&copy).unwrap(), b"abc");
        assert!(copy_file_no_replace(&file, &copy, &file_meta).is_err());
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn trash_local_records_and_restores() {
        let state = temp("trash-local-state");
        let root = temp("trash-local-root");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("a.txt"), b"abc").unwrap();
        let entry = trash_local(&state, &root, "a.txt").unwrap();
        assert!(!root.join("a.txt").exists());
        assert!(trash_dir(&state).join("a.txt").exists());
        let listed = entries(&state).unwrap();
        assert_eq!(listed.len(), 1);
        restore(&state, &root, &listed[0]).unwrap();
        assert_eq!(fs::read(root.join("a.txt")).unwrap(), b"abc");
        let _ = entry;
        let _ = fs::remove_dir_all(&state);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn unsafe_manifest_paths_are_refused() {
        assert!(!safe_rel("../escape"));
        assert!(!safe_rel("/abs"));
        assert!(!safe_rel("a/../../b"));
        assert!(safe_rel("a/b"));
    }
}
