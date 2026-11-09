//! Execute planned actions via the official CLI.
//! Mirrors Python's execute() behavior closely.

use crate::action::{Action, LocalEntry, LocalTree};
use crate::cli::{self, Cli};
use crate::filter::Filter;
use anyhow::{Context, Result};
use proton_core::snapshot::{RemoteEntry, RemoteTree};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::health::state_dir;

type PathSet = BTreeSet<String>;
type Fingerprints = BTreeMap<String, (i64, u64)>;

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

pub fn ensure_remote_dir(
    cli: &Cli,
    remote_root: &str,
    rel: &str,
    tree: &mut RemoteTree,
    created: &mut BTreeSet<String>,
) -> Result<()> {
    if rel.is_empty() || tree.contains_key(rel) {
        return Ok(());
    }
    let parent = parent_of(rel);
    if !parent.is_empty() {
        ensure_remote_dir(cli, remote_root, &parent, tree, created)?;
    }
    let name = name_of(rel);
    let parent_rpath = cli::remote_path(remote_root, &parent);
    let _ = cli.run(
        &["fs", "create-folder", &parent_rpath, &cli::escape(&name)],
        false,
        Some(120),
        true,
    );
    tree.insert(
        rel.to_string(),
        RemoteEntry {
            uid: None,
            kind: "dir".to_string(),
            mtime: None,
            size: Some(0),
            doc: false,
        },
    );
    created.insert(rel.to_string());
    eprintln!("MKDIR  remote {}", rel);
    Ok(())
}

fn trash_local_path(
    rel: &str,
    local_root: &Path,
    trash_dir: &Path,
) -> Result<proton_core::local_trash::Entry> {
    let full = local_root.join(rel);
    let meta = fs::symlink_metadata(&full)
        .with_context(|| format!("reading {}", full.display()))?;
    if meta.file_type().is_symlink() {
        anyhow::bail!("refusing to trash a symlink: {}", full.display());
    }
    let mut stored_rel = rel.to_string();
    let mut suffix = 1u32;
    while trash_dir.join(&stored_rel).exists() {
        suffix += 1;
        stored_rel = format!("{rel}.{suffix}");
    }
    let dst = trash_dir.join(&stored_rel);
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::rename(&full, &dst)
        .with_context(|| format!("moving {} to {}", full.display(), dst.display()))?;
    eprintln!("TRASH  local {}", rel);
    Ok(proton_core::local_trash::Entry {
        stored_rel,
        original_rel: rel.to_string(),
        trashed_at: proton_core::local_trash::now_secs(),
        size: if meta.is_file() { meta.len() } else { 0 },
        kind: if meta.is_dir() { "dir" } else { "file" }.to_string(),
        legacy: false,
    })
}

/// A cloud-supplied relative path is safe only if every component is a plain
/// name. Without this a malicious or malformed listing could escape the sync
/// root with `..` before any guard runs.
pub(crate) fn safe_rel(rel: &str) -> bool {
    !rel.is_empty()
        && rel
            .split('/')
            .all(|c| !c.is_empty() && c != "." && c != ".." && !c.contains('\0'))
}

fn staging_root(local_root: &Path, parent: &str) -> Result<std::path::PathBuf> {
    let base = if parent.is_empty() {
        local_root.to_path_buf()
    } else {
        local_root.join(parent)
    };
    fs::create_dir_all(&base)?;
    let root = base.join(".proton-sync-staging");
    match fs::symlink_metadata(&root) {
        Ok(meta) if meta.file_type().is_symlink() => {
            anyhow::bail!(
                "refusing a staging directory that is a symlink: {}",
                root.display()
            )
        }
        Ok(meta) if !meta.is_dir() => {
            anyhow::bail!("staging path is not a directory: {}", root.display())
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => fs::create_dir_all(&root)?,
        Err(e) => return Err(e.into()),
    }
    Ok(root)
}

static STAGE_SEQ: AtomicU64 = AtomicU64::new(0);

fn new_stage(local_root: &Path, parent: &str) -> Result<std::path::PathBuf> {
    let seq = STAGE_SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = staging_root(local_root, parent)?
        .join(format!("{}-{seq}-{nanos}", std::process::id()));
    // `create_dir`, not `create_dir_all`: a stale or planted directory with
    // this name must never be reused as a download target.
    fs::create_dir(&path)?;
    Ok(path)
}

/// Create the parent chain for `dest` one component at a time, refusing any
/// existing symlink. The scanner already rejects links, but a link planted
/// between the scan and the write must not redirect the file outside the root.
fn ensure_local_parent(local_root: &Path, dest: &Path) -> Result<()> {
    let parent = dest.parent().unwrap_or(local_root);
    let rel = parent
        .strip_prefix(local_root)
        .map_err(|_| anyhow::anyhow!("destination outside the sync root: {}", dest.display()))?;
    let mut cur = local_root.to_path_buf();
    for comp in rel.components() {
        cur.push(comp);
        match fs::symlink_metadata(&cur) {
            Ok(meta) if meta.file_type().is_symlink() => {
                anyhow::bail!("refusing symlinked path component: {}", cur.display())
            }
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => anyhow::bail!("path component is not a directory: {}", cur.display()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&cur)
                    .with_context(|| format!("creating {}", cur.display()))?;
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

fn ensure_local_dir(local_root: &Path, rel: &str) -> Result<()> {
    let dest = local_root.join(rel);
    ensure_local_parent(local_root, &dest.join(".proton-dir-probe"))
}

/// A planned local file should still have the fingerprint that was scanned
/// before a destructive local action.
fn planned_file_matches(local_root: &Path, rel: &str, planned: Option<&LocalEntry>) -> bool {
    let Some(LocalEntry::File { mtime_ns, size }) = planned else {
        return true; // directories are judged by their plan-time children
    };
    fs::symlink_metadata(local_root.join(rel))
        .map(|m| m.is_file() && crate::scan::mtime_ns(&m) == *mtime_ns && m.len() == *size)
        .unwrap_or(false)
}

/// Place a verified download without replacing content that appeared after
/// the plan was made. `None` means the path did not exist at plan time.
fn place_download(
    source: &Path,
    dest: &Path,
    planned: Option<&LocalEntry>,
    backup_dir: &Path,
) -> Result<(i64, u64)> {
    let source_fp = fs::symlink_metadata(source)
        .map(|m| (crate::scan::mtime_ns(&m), m.len()))
        .with_context(|| format!("reading staged {}", source.display()))?;
    let placed_fp = |dest: &Path| -> Result<(i64, u64)> {
        let meta = fs::symlink_metadata(dest)
            .with_context(|| format!("reading placed {}", dest.display()))?;
        let fp = (crate::scan::mtime_ns(&meta), meta.len());
        if fp != source_fp {
            anyhow::bail!("local file changed during the download: {}", dest.display());
        }
        Ok(fp)
    };
    match planned {
        Some(LocalEntry::File { mtime_ns, size }) => {
            let before = fs::symlink_metadata(dest)
                .with_context(|| format!("reading {}", dest.display()))?;
            if !before.is_file()
                || crate::scan::mtime_ns(&before) != *mtime_ns
                || before.len() != *size
            {
                anyhow::bail!("local file changed during the download: {}", dest.display());
            }
            // Copy the old bytes aside, replace atomically, then verify the
            // file we moved aside is still the planned one. A user edit in
            // the tiny window is restored instead of being lost.
            let backup = backup_dir.join(format!(".backup-{}", name_of(&dest.to_string_lossy())));
            // A hard link preserves the old inode's mtime/size, so a user edit
            // in the replace window is detectable after the atomic rename.
            let linked = fs::hard_link(dest, &backup).is_ok();
            if !linked {
                fs::copy(dest, &backup)
                    .with_context(|| format!("backing up {}", dest.display()))?;
            }
            if let Err(e) = fs::rename(source, dest) {
                let _ = fs::remove_file(&backup);
                return Err(e.into());
            }
            let old = fs::symlink_metadata(&backup)?;
            let matches = old.is_file()
                && old.len() == *size
                && (!linked || crate::scan::mtime_ns(&old) == *mtime_ns);
            if matches {
                let _ = fs::remove_file(&backup);
                placed_fp(dest)
            } else {
                let _ = fs::remove_file(dest);
                let _ = fs::rename(&backup, dest);
                anyhow::bail!("local file changed during the download: {}", dest.display());
            }
        }
        _ => {
            if fs::symlink_metadata(dest).is_ok() {
                anyhow::bail!("destination appeared during the download: {}", dest.display());
            }
            match fs::hard_link(source, dest) {
                Ok(()) => {
                    let _ = fs::remove_file(source);
                    placed_fp(dest)
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    anyhow::bail!("destination appeared during the download: {}", dest.display())
                }
                Err(_) => {
                    // Filesystems without hard links: check once and rename.
                    if fs::symlink_metadata(dest).is_ok() {
                        anyhow::bail!(
                            "destination appeared during the download: {}",
                            dest.display()
                        );
                    }
                    fs::rename(source, dest)?;
                    placed_fp(dest)
                }
            }
        }
    }
}

pub(crate) struct Downloads {
    pub failed: PathSet,
    pub placed: Fingerprints,
}

/// Download `rels` into a private staging directory, verify each file's size,
/// then move only the verified files into place. A missing or short download
/// never touches the final path, so it cannot become the local truth.
///
/// The caller already holds the CLI lock.
pub(crate) fn staged_download(
    cli: &Cli,
    remote_root: &str,
    local_root: &Path,
    planned_local: &LocalTree,
    parent: &str,
    rels: &[(String, u64)],
    retries: usize,
) -> Downloads {
    let mut failed = BTreeSet::new();
    let mut placed = BTreeMap::new();
    if rels.is_empty() {
        return Downloads { failed, placed };
    }
    for (rel, _) in rels {
        if !safe_rel(rel) {
            eprintln!("REFUSE unsafe path from the cloud: {rel:?}");
            failed.insert(rel.clone());
        }
    }
    if !failed.is_empty() {
        failed.extend(rels.iter().map(|(rel, _)| rel.clone()));
        return Downloads { failed, placed };
    }

    let stage = match new_stage(local_root, parent) {
        Ok(path) => path,
        Err(e) => {
            eprintln!("FAILED staging directory: {e:#}");
            failed.extend(rels.iter().map(|(rel, _)| rel.clone()));
            return Downloads { failed, placed };
        }
    };
    let rpaths: Vec<String> = rels
        .iter()
        .map(|(rel, _)| cli::remote_path(remote_root, rel))
        .collect();
    let mut args: Vec<&str> = vec!["fs", "download"];
    args.extend(rpaths.iter().map(String::as_str));
    args.extend(["-f", "remove", "-d", "merge"]);
    let stage_s = stage.to_string_lossy().into_owned();
    args.push(&stage_s);

    let mut res = cli.run(&args, false, Some(3600), false);
    for attempt in 1..retries.max(1) {
        if res.is_ok() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(2 * attempt as u64));
        res = cli.run(&args, false, Some(3600), false);
    }

    match res {
        Ok(_) => {
            for (rel, expected) in rels {
                let source = stage.join(name_of(rel));
                let verified = fs::metadata(&source)
                    .is_ok_and(|m| m.is_file() && m.len() == *expected);
                if !verified {
                    eprintln!("FAILED verify {} (staged size is not {expected})", rel);
                    failed.insert(rel.clone());
                    continue;
                }
                if let Err(e) = fs::File::open(&source).and_then(|f| f.sync_all()) {
                    eprintln!("FAILED sync {}: {e}", rel);
                    failed.insert(rel.clone());
                    continue;
                }
                let dest = local_root.join(rel);
                if let Err(e) = ensure_local_parent(local_root, &dest) {
                    eprintln!("FAILED prepare {}: {e:#}", rel);
                    failed.insert(rel.clone());
                    continue;
                }
                match place_download(&source, &dest, planned_local.get(rel), &stage) {
                    Ok(fp) => {
                        placed.insert(rel.clone(), fp);
                    }
                    Err(e) => {
                        eprintln!("FAILED place {}: {e:#}", rel);
                        failed.insert(rel.clone());
                    }
                }
            }
        }
        Err(e) => {
            eprintln!("FAILED download batch ({} path(s)): {e}", rels.len());
            failed.extend(rels.iter().map(|(rel, _)| rel.clone()));
        }
    }
    let _ = fs::remove_dir_all(&stage);
    Downloads { failed, placed }
}

/// Download one file, verify it, then rename it to `dest`.
fn staged_download_one(
    cli: &Cli,
    remote_root: &str,
    local_root: &Path,
    rel: &str,
    expected: u64,
    dest: &Path,
) -> Result<()> {
    if !safe_rel(rel) {
        anyhow::bail!("unsafe path from the cloud: {rel:?}");
    }
    let stage = new_stage(local_root, &parent_of(rel))?;
    let rpath = cli::remote_path(remote_root, rel);
    let stage_s = stage.to_string_lossy().into_owned();
    let res = cli.run(
        &[
            "fs",
            "download",
            &rpath,
            "-f",
            "remove",
            "-d",
            "merge",
            &stage_s,
        ],
        false,
        Some(3600),
        true,
    );
    let outcome = (|| -> Result<()> {
        res?;
        let source = stage.join(name_of(rel));
        let meta = fs::metadata(&source)?;
        if !meta.is_file() || meta.len() != expected {
            anyhow::bail!(
                "downloaded {} bytes, expected {expected}",
                meta.len()
            );
        }
        fs::File::open(&source)?.sync_all()?;
        ensure_local_parent(local_root, dest)?;
        let _ = place_download(&source, dest, None, &stage)?;
        Ok(())
    })();
    let _ = fs::remove_dir_all(&stage);
    outcome
}

/// A conflict filename that does not exist yet, so two conflicts in the same
/// second cannot overwrite each other.
fn conflict_rel(rel: &str, ts: &str, local_root: &Path) -> String {
    let dir = parent_of(rel);
    let base = name_of(rel);
    let (stem, ext) = match base.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s, format!(".{e}")),
        _ => (base.as_str(), String::new()),
    };
    for n in 0u32.. {
        let suffix = if n == 0 {
            String::new()
        } else {
            format!(" {}", n + 1)
        };
        let name = format!("{stem} (conflict {ts}{suffix}){ext}");
        let candidate = if dir.is_empty() {
            name
        } else {
            format!("{dir}/{name}")
        };
        if !local_root.join(&candidate).exists() {
            return candidate;
        }
    }
    unreachable!()
}

pub fn execute(
    actions: &[Action],
    remote: &RemoteTree,
    local: &LocalTree,
    remote_root: &str,
    local_root: &Path,
    _filter: &Filter,
) -> Result<(PathSet, PathSet, Fingerprints)> {
    let cli = Cli::new()?;
    let state = state_dir();
    let trash_dir = state.join("trash");
    let mut tree: RemoteTree = remote.clone();
    let mut created_dirs: BTreeSet<String> = BTreeSet::new();
    let mut failed: BTreeSet<String> = BTreeSet::new();
    let mut downloaded: BTreeSet<String> = BTreeSet::new();
    let mut downloaded_fp: BTreeMap<String, (i64, u64)> = BTreeMap::new();
    let mut deferred_up: Vec<String> = Vec::new();
    let mut deferred_dl: Vec<(String, u64)> = Vec::new();

    for action in actions {
        // One gate before any join or CLI call: a cached snapshot, a crafted
        // listing or a stale state row must never escape the sync root.
        let bad = match action {
            Action::MvRemote { old, new } | Action::MvLocal { old, new } => {
                !safe_rel(old) || !safe_rel(new)
            }
            other => !safe_rel(other.rel()),
        };
        if bad {
            eprintln!("REFUSE unsafe path from the cloud: {:?}", action.rel());
            failed.insert(action.rel().to_string());
            continue;
        }
        match action {
            Action::Up { rel, node } => {
                if !node.is_file() {
                    continue;
                }
                deferred_up.push(rel.clone());
            }
            Action::Dl { rel, node } if node.kind != "dir" => {
                let Some(size) = node.size else {
                    eprintln!("FAILED download {}: the cloud listing has no size", rel);
                    failed.insert(rel.clone());
                    continue;
                };
                deferred_dl.push((rel.clone(), size));
            }
            Action::Dl { rel, node } => {
                if node.kind == "dir" {
                    // Only the folder itself: its files are planned and
                    // downloaded one by one, so a subtree download here would
                    // fetch every file again for each folder above it.
                    if let Err(e) = ensure_local_dir(local_root, rel) {
                        eprintln!("FAILED folder {}: {e:#}", rel);
                        failed.insert(rel.clone());
                    }
                    continue;
                }
            }
            Action::TrashLocal { rel, .. } => {
                if !planned_file_matches(local_root, rel, local.get(rel)) {
                    eprintln!("FAILED trash {}: local file changed after the scan", rel);
                    failed.insert(rel.clone());
                } else {
                    match trash_local_path(rel, local_root, &trash_dir) {
                        Ok(entry) => {
                            if let Err(e) = proton_core::local_trash::record(&state, &entry) {
                                eprintln!("WARN trash manifest {}: {e}", rel);
                            }
                        }
                        Err(e) => {
                            eprintln!("FAILED trash {}: {e:#}", rel);
                            failed.insert(rel.clone());
                        }
                    }
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
                let res = cli.run(
                    &["fs", "move", &old_rpath, &parent_rpath, &cli::escape(&name)],
                    false,
                    Some(300),
                    true,
                );
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
                if !planned_file_matches(local_root, old, local.get(old)) {
                    eprintln!("FAILED move {}: local file changed after the scan", old);
                    failed.insert(old.clone());
                    failed.insert(new.clone());
                    continue;
                }
                let res = ensure_local_parent(local_root, &new_full)
                    .and_then(|()| {
                        if fs::symlink_metadata(&new_full).is_ok() {
                            anyhow::bail!(
                                "destination appeared before the move: {}",
                                new_full.display()
                            );
                        }
                        fs::rename(&old_full, &new_full).with_context(|| {
                            format!("moving {} to {}", old_full.display(), new_full.display())
                        })
                    });
                if res.is_err() {
                    failed.insert(old.clone());
                    failed.insert(new.clone());
                } else {
                    eprintln!("MOVE   local {} -> {}", old, new);
                }
            }
            Action::Conflict { rel, remote, .. } => {
                // Copy the remote revision to a conflict path first. The
                // canonical local file is never written, so the user's edit
                // survives even if every later step fails.
                let ts = chrono::Utc::now().format("%Y-%m-%d %H-%M-%S").to_string();
                let conflict_name = conflict_rel(rel, &ts, local_root);
                let Some(expected) = remote.size else {
                    eprintln!("FAILED conflict copy for {}: no cloud size", rel);
                    failed.insert(rel.clone());
                    continue;
                };
                let dest = local_root.join(&conflict_name);
                if let Err(e) = staged_download_one(
                    &cli,
                    remote_root,
                    local_root,
                    rel,
                    expected,
                    &dest,
                ) {
                    eprintln!("FAILED conflict copy for {}: {e:#}", rel);
                    failed.insert(rel.clone());
                    continue;
                }
                eprintln!("CONFLICT {} (kept remote copy as {})", rel, conflict_name);
                proton_core::conflict::invalidate_cache(&state);
                // Only now upload the untouched local revision. `-d merge`
                // keeps the conflict copy; the canonical name is updated.
                let parent_rpath = cli::remote_path(remote_root, &parent_of(rel));
                if let Err(e) = cli.run(
                    &[
                        "fs",
                        "upload",
                        &local_root.join(rel).to_string_lossy(),
                        &parent_rpath,
                        "-f",
                        "create-new-revision",
                        "-d",
                        "merge",
                    ],
                    false,
                    Some(3600),
                    true,
                ) {
                    eprintln!("FAILED conflict upload for {}: {e:#}", rel);
                    failed.insert(rel.clone());
                }
            }
        }
    }
    let transfers = cli::cfg_int("PROTON_SYNC_TRANSFERS", 4, 10);
    let mut batches: Vec<Batch> = Vec::new();
    let mut up_by_parent: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for rel in &deferred_up {
        up_by_parent
            .entry(parent_of(rel))
            .or_default()
            .push(rel.clone());
    }
    for (parent, rels) in &up_by_parent {
        if !parent.is_empty() {
            ensure_remote_dir(&cli, remote_root, parent, &mut tree, &mut created_dirs)?;
        }
        for chunk in rels.chunks(MAX_BATCH) {
            batches.push(Batch {
                upload: true,
                parent: parent.clone(),
                rels: chunk.to_vec(),
                sizes: Vec::new(),
            });
        }
    }
    let mut dl_by_parent: BTreeMap<String, Vec<(String, u64)>> = BTreeMap::new();
    for (rel, size) in &deferred_dl {
        dl_by_parent
            .entry(parent_of(rel))
            .or_default()
            .push((rel.clone(), *size));
    }
    for (parent, rels) in &dl_by_parent {
        fs::create_dir_all(local_root.join(parent))?;
        for chunk in rels.chunks(MAX_BATCH) {
            let (names, sizes) = chunk.iter().cloned().unzip();
            batches.push(Batch {
                upload: false,
                parent: parent.clone(),
                rels: names,
                sizes,
            });
        }
    }

    let total_files: u64 = batches.iter().map(|b| b.rels.len() as u64).sum();
    let done_files = std::sync::atomic::AtomicU64::new(0);
    let retries = cli::cfg_int("PROTON_SYNC_RETRIES", 3, 10);
    report_transfer(0, total_files, &[]);
    let outcomes = cli::with_cli_lock(|| {
        Ok(cli::run_parallel(batches, transfers, |b| {
            let n = b.rels.len() as u64;
            let res: Result<Downloads> = if b.upload {
                let locals: Vec<String> = b
                    .rels
                    .iter()
                    .map(|r| local_root.join(r).to_string_lossy().into_owned())
                    .collect();
                let parent_rpath = cli::remote_path(remote_root, &b.parent);
                let mut args: Vec<&str> =
                    vec!["fs", "upload", "-f", "create-new-revision", "-d", "merge"];
                args.extend(locals.iter().map(|s| s.as_str()));
                args.push(&parent_rpath);
                cli.run(&args, false, Some(3600), false)
                    .map(|_| Downloads {
                        failed: BTreeSet::new(),
                        placed: BTreeMap::new(),
                    })
            } else {
                let pairs: Vec<(String, u64)> = b
                    .rels
                    .iter()
                    .cloned()
                    .zip(b.sizes.iter().cloned())
                    .collect();
                Ok(staged_download(
                    &cli,
                    remote_root,
                    local_root,
                    local,
                    &b.parent,
                    &pairs,
                    retries,
                ))
            };
            let done = done_files.fetch_add(n, std::sync::atomic::Ordering::Relaxed) + n;
            let names: Vec<String> = b
                .rels
                .iter()
                .map(|r| {
                    if b.upload {
                        format!("up:{r}")
                    } else {
                        format!("down:{r}")
                    }
                })
                .collect();
            report_transfer(done, total_files, &names);
            (b, res)
        }))
    })?;
    report_transfer(total_files, total_files, &[]);

    for (b, res) in outcomes {
        match res {
            Ok(dl) => {
                for rel in &b.rels {
                    if dl.failed.contains(rel) {
                        failed.insert(rel.clone());
                    } else if b.upload {
                        eprintln!("UPLOAD {}", rel);
                    } else {
                        eprintln!("DOWNLOAD {}", rel);
                        downloaded.insert(rel.clone());
                        if let Some(fp) = dl.placed.get(rel) {
                            downloaded_fp.insert(rel.clone(), *fp);
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("FAILED transfer batch ({} path(s)): {}", b.rels.len(), e);
                failed.extend(b.rels.iter().cloned());
            }
        }
    }
    Ok((failed, downloaded, downloaded_fp))
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

fn report_transfer(done: u64, total: u64, names: &[String]) {
    let path = crate::health::health_path();
    let mut h = proton_core::health::read(&path);
    h.status = "syncing".to_string();
    h.transfer_active = Some(done < total);
    h.transfer_done = Some(done);
    h.transfer_total = Some(total);
    if names.is_empty() {
        h.queue.clear();
    } else {
        h.queue = names.iter().take(20).cloned().collect();
    }
    crate::health::write_health(&h);
}

struct Batch {
    upload: bool,
    parent: String,
    rels: Vec<String>,
    sizes: Vec<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("pd-trash-{}-{name}", std::process::id()))
    }

    #[test]
    fn local_trash_keeps_paths_and_same_names_distinct() {
        let root = temp("root");
        let trash = temp("trash");
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&trash);
        fs::create_dir_all(root.join("a")).unwrap();
        fs::create_dir_all(root.join("b")).unwrap();
        fs::write(root.join("a/same.txt"), b"first").unwrap();
        fs::write(root.join("b/same.txt"), b"second").unwrap();

        let first = trash_local_path("a/same.txt", &root, &trash).unwrap();
        let second = trash_local_path("b/same.txt", &root, &trash).unwrap();

        assert_eq!(first.stored_rel, "a/same.txt");
        assert_eq!(first.original_rel, "a/same.txt");
        assert_eq!(first.kind, "file");
        assert_eq!(first.size, 5);
        assert_eq!(second.stored_rel, "b/same.txt");
        assert_eq!(fs::read(trash.join("a/same.txt")).unwrap(), b"first");
        assert_eq!(fs::read(trash.join("b/same.txt")).unwrap(), b"second");
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(trash);
    }

    #[test]
    fn local_trash_does_not_reuse_an_existing_entry() {
        let root = temp("root2");
        let trash = temp("trash2");
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&trash);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("same.txt"), b"new").unwrap();
        fs::create_dir_all(&trash).unwrap();
        fs::write(trash.join("same.txt"), b"old").unwrap();

        trash_local_path("same.txt", &root, &trash).unwrap();

        assert_eq!(fs::read(trash.join("same.txt")).unwrap(), b"old");
        assert_eq!(fs::read(trash.join("same.txt.2")).unwrap(), b"new");
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(trash);
    }

    #[test]
    fn failed_local_trash_returns_an_error() {
        let root = temp("missing-root");
        let trash = temp("missing-trash");
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&trash);
        assert!(trash_local_path("gone.txt", &root, &trash).is_err());
    }

    fn fake_cli(dir: &std::path::Path, body: &str) -> Cli {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("fake-proton-drive");
        fs::write(&path, body).unwrap();
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).unwrap();
        Cli::for_test(path)
    }

    /// The download script's last argument is the target directory.
    const DOWNLOAD_SCRIPT: &str = "#!/bin/sh\ntarget=\"\"\nfor a in \"$@\"; do target=\"$a\"; done\nprintf 'hello' > \"$target/a.txt\"\n";

    #[test]
    fn staged_download_verifies_then_moves() {
        let root = temp("stage-ok");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let cli = fake_cli(&root, DOWNLOAD_SCRIPT);
        let failed = staged_download(
            &cli,
            "/my-files",
            &root,
            &LocalTree::new(),
            "",
            &[("a.txt".to_string(), 5)],
            1,
        ).failed;
        assert!(failed.is_empty());
        assert_eq!(fs::read(root.join("a.txt")).unwrap(), b"hello");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn staged_download_does_not_overwrite_a_file_edited_after_the_plan() {
        let root = temp("stage-edit-race");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let cli = fake_cli(&root, DOWNLOAD_SCRIPT);
        let path = root.join("a.txt");
        fs::write(&path, b"old").unwrap();
        let meta = fs::symlink_metadata(&path).unwrap();
        let mut plan = LocalTree::new();
        plan.insert(
            "a.txt".into(),
            LocalEntry::File {
                mtime_ns: crate::scan::mtime_ns(&meta),
                size: meta.len(),
            },
        );
        // The user edits after the scan but before the bytes land.
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&path, b"user-edit").unwrap();
        let failed = staged_download(
            &cli,
            "/my-files",
            &root,
            &plan,
            "",
            &[("a.txt".to_string(), 5)],
            1,
        ).failed;
        assert!(failed.contains("a.txt"));
        assert_eq!(fs::read(&path).unwrap(), b"user-edit");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn staged_download_replaces_an_unchanged_file() {
        let root = temp("stage-unchanged");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let cli = fake_cli(&root, DOWNLOAD_SCRIPT);
        let path = root.join("a.txt");
        fs::write(&path, b"old").unwrap();
        let meta = fs::symlink_metadata(&path).unwrap();
        let mut plan = LocalTree::new();
        plan.insert(
            "a.txt".into(),
            LocalEntry::File {
                mtime_ns: crate::scan::mtime_ns(&meta),
                size: meta.len(),
            },
        );
        let failed = staged_download(
            &cli,
            "/my-files",
            &root,
            &plan,
            "",
            &[("a.txt".to_string(), 5)],
            1,
        ).failed;
        assert!(failed.is_empty());
        assert_eq!(fs::read(&path).unwrap(), b"hello");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn short_download_never_replaces_the_local_file() {
        let root = temp("stage-short");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("a.txt"), b"old").unwrap();
        let cli = fake_cli(&root, DOWNLOAD_SCRIPT);
        let failed = staged_download(
            &cli,
            "/my-files",
            &root,
            &LocalTree::new(),
            "",
            &[("a.txt".to_string(), 9)],
            1,
        ).failed;
        assert!(failed.contains("a.txt"));
        assert_eq!(fs::read(root.join("a.txt")).unwrap(), b"old");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn unsafe_rel_fails_the_whole_batch() {
        let root = temp("stage-unsafe");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let cli = fake_cli(&root, DOWNLOAD_SCRIPT);
        let failed = staged_download(
            &cli,
            "/my-files",
            &root,
            &LocalTree::new(),
            "",
            &[("../escape.txt".to_string(), 5), ("a.txt".to_string(), 5)],
            1,
        ).failed;
        assert!(failed.contains("../escape.txt"));
        assert!(failed.contains("a.txt"));
        assert!(!root.join("a.txt").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn conflict_copy_keeps_local_bytes_and_uses_a_unique_name() {
        let root = temp("conflict");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("dir")).unwrap();
        fs::write(root.join("dir/photo.jpg"), b"local").unwrap();
        let cli = fake_cli(
            &root,
            "#!/bin/sh\ntarget=\"\"\nfor a in \"$@\"; do target=\"$a\"; done\nprintf 'cloud' > \"$target/photo.jpg\"\n",
        );
        let copy = conflict_rel("dir/photo.jpg", "2026-01-01 00-00-00", &root);
        assert_eq!(copy, "dir/photo (conflict 2026-01-01 00-00-00).jpg");
        let dest = root.join(&copy);
        staged_download_one(&cli, "/my-files", &root, "dir/photo.jpg", 5, &dest).unwrap();
        assert_eq!(fs::read(root.join("dir/photo.jpg")).unwrap(), b"local");
        assert_eq!(fs::read(&dest).unwrap(), b"cloud");

        // The second conflict in the same second must not overwrite the first.
        let again = conflict_rel("dir/photo.jpg", "2026-01-01 00-00-00", &root);
        assert_eq!(again, "dir/photo (conflict 2026-01-01 00-00-00 2).jpg");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn hidden_files_keep_their_whole_name_in_a_conflict() {
        assert_eq!(
            conflict_rel(".bashrc", "2026-01-01 00-00-00", Path::new("/nonexistent")),
            ".bashrc (conflict 2026-01-01 00-00-00)"
        );
    }

    #[test]
    fn staging_directory_is_never_scanned() {
        assert!(Filter::skip_name(".proton-sync-staging"));
        assert!(!Filter::skip_name("photos"));
    }
}
