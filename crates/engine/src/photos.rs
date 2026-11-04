//! Two-way sync between the cloud photo timeline (`/photos`) and a local
//! folder (default `~/ProtonPhotos`).
//!
//! Files only on disk are uploaded, photos only in the cloud are downloaded,
//! and deletions propagate both ways - but only for photos the engine has seen
//! in sync, recorded in `photos-state.json`. That record is what makes the
//! destructive half safe: a photo that was never synced is never deleted just
//! because it is missing on one side.

use crate::cli::Cli;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Extensions the timeline accepts; anything else in the folder is ignored.
const IMAGE_EXTS: [&str; 9] = [
    "jpg", "jpeg", "png", "heic", "heif", "webp", "gif", "tif", "tiff",
];

/// How many paths go into one CLI call (uploads, downloads, trash).
const BATCH: usize = 25;

/// Most deletions one cycle will perform. A mass delete (a moved folder, a
/// half-mounted disk) then crawls instead of wiping the library in one go.
const MAX_DELETES: usize = 25;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RemotePhoto {
    /// Node uid: `photo download` and `fs trash` want `/photos/<uid>`.
    pub uid: String,
    pub name: String,
    /// Original size (`claimedSize`), which is what the file on disk measures.
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LocalPhoto {
    pub name: String,
    pub size: u64,
}

/// What one cycle moves.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PhotoPlan {
    /// Local file names to upload.
    pub upload: Vec<String>,
    /// Remote uids to download.
    pub download: Vec<String>,
    /// Names that differ in size on both sides: left alone, reported once.
    pub conflicts: Vec<String>,
    /// Photos deleted on disk: (name, uid) to send to the cloud trash.
    pub trash_remote: Vec<(String, String)>,
    /// Photos deleted in the cloud: local file names to remove.
    pub remove_local: Vec<String>,
}

/// A photo known to exist on both sides.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Synced {
    pub uid: String,
    pub size: u64,
}

/// The engine's memory of this folder: which photos are in sync, and which
/// name clashes it already reported (so one is not reported every cycle).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PhotoState {
    pub synced: BTreeMap<String, Synced>,
    pub handled: BTreeMap<String, (u64, u64)>,
}

fn remote_by_name<'a>(remote: &'a [RemotePhoto], name: &str) -> Option<&'a RemotePhoto> {
    remote.iter().find(|r| r.name == name)
}
fn local_by_name<'a>(local: &'a [LocalPhoto], name: &str) -> Option<&'a LocalPhoto> {
    local.iter().find(|l| l.name == name)
}

/// The plan for one cycle - pure, so every rule is unit-tested.
///
/// `allow_deletes` is the user's switch; `folder_looks_present` guards against
/// an empty mount point being read as "everything was deleted".
pub fn plan(
    local: &[LocalPhoto],
    remote: &[RemotePhoto],
    state: &PhotoState,
    allow_deletes: bool,
    folder_looks_present: bool,
) -> PhotoPlan {
    let mut p = PhotoPlan::default();
    for l in local {
        match remote_by_name(remote, &l.name) {
            // Same name, same size: in sync (the caller records it).
            Some(r) if r.size == l.size => {}
            // Same name, different bytes: never overwrite either copy.
            Some(r) => {
                if state.handled.get(&l.name) != Some(&(l.size, r.size)) {
                    p.conflicts.push(l.name.clone());
                }
            }
            // Not in the cloud: new locally, or the cloud copy was deleted.
            None => match state.synced.get(&l.name) {
                Some(_) => p.remove_local.push(l.name.clone()),
                None => p.upload.push(l.name.clone()),
            },
        }
    }
    for r in remote {
        if local_by_name(local, &r.name).is_some() {
            continue; // handled above (in sync or conflict)
        }
        match state.synced.get(&r.name) {
            // We had it synced and the file is gone: the user deleted it here.
            Some(s) if s.uid == r.uid => p.trash_remote.push((r.name.clone(), r.uid.clone())),
            // Never seen before: download it.
            _ => p.download.push(r.uid.clone()),
        }
    }
    p.upload.sort();
    p.download.sort();
    p.download.dedup();
    p.conflicts.sort();
    p.trash_remote.sort();
    p.remove_local.sort();
    if !allow_deletes || !folder_looks_present {
        // Either the switch is off, or the folder smells like an unmounted
        // disk: do not destroy anything, just report what would have happened.
        p.trash_remote.clear();
        p.remove_local.clear();
    }
    p
}

/// Result of one cycle, mirrored into `photos-sync.json` for the GUI.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Summary {
    pub ts: String,
    pub remote: usize,
    pub local: usize,
    pub uploaded: usize,
    pub downloaded: usize,
    pub conflicts: usize,
    #[serde(default)]
    pub trashed: usize,
    #[serde(default)]
    pub removed: usize,
    pub error: String,
}

fn is_image(name: &str) -> bool {
    name.rsplit_once('.')
        .map(|(_, ext)| IMAGE_EXTS.contains(&ext.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Every image file directly inside `dir` (the timeline is flat).
pub fn scan_local(dir: &Path) -> Vec<LocalPhoto> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Ok(meta) = e.metadata() else { continue };
        if !meta.is_file() || !is_image(&name) {
            continue;
        }
        out.push(LocalPhoto { name, size: meta.len() });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// The cloud timeline (`photo timeline -d --json`).
pub fn timeline(cli: &Cli) -> Result<Vec<RemotePhoto>> {
    let v = cli.run_json(&["photo", "timeline", "-d"], true)?;
    let arr = v.as_array().cloned().unwrap_or_default();
    let mut out = Vec::new();
    for e in &arr {
        let uid = e.get("uid").and_then(|x| x.as_str()).unwrap_or("");
        let name = e
            .get("name")
            .and_then(|n| n.get("value"))
            .and_then(|n| n.as_str())
            .unwrap_or("");
        if uid.is_empty() || name.is_empty() {
            continue;
        }
        // `claimedSize` is the original file size; `totalStorageSize` is the
        // encrypted size and would make every photo look like a conflict.
        let size = e
            .get("activeRevision")
            .and_then(|r| r.get("claimedSize"))
            .and_then(|x| x.as_u64())
            .or_else(|| e.get("totalStorageSize").and_then(|x| x.as_u64()))
            .unwrap_or(0);
        out.push(RemotePhoto {
            uid: uid.to_string(),
            name: name.to_string(),
            size,
        });
    }
    Ok(out)
}

fn state_path(state_dir: &Path) -> PathBuf {
    state_dir.join("photos-state.json")
}

fn summary_path(state_dir: &Path) -> PathBuf {
    state_dir.join("photos-sync.json")
}

fn read_state(state_dir: &Path) -> PhotoState {
    std::fs::read_to_string(state_path(state_dir))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn write_json<T: Serialize>(path: &Path, v: &T) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(text) = serde_json::to_string(v) else { return };
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

/// Upload the given file names (batched), from `dir`.
fn upload(cli: &Cli, dir: &Path, names: &[String]) -> Result<usize> {
    let mut done = 0;
    for chunk in names.chunks(BATCH) {
        let mut args: Vec<String> = ["photo", "upload", "-c", "rename"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        args.extend(chunk.iter().map(|n| dir.join(n).to_string_lossy().into_owned()));
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        cli.run(&refs, false, None, true)
            .with_context(|| format!("uploading {} photo(s)", chunk.len()))?;
        done += chunk.len();
    }
    Ok(done)
}

/// Download the given uids (batched) into `dir`.
fn download(cli: &Cli, dir: &Path, uids: &[String]) -> Result<usize> {
    let mut done = 0;
    for chunk in uids.chunks(BATCH) {
        let mut args: Vec<String> = ["photo", "download", "-c", "rename"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        args.extend(chunk.iter().map(|u| format!("/photos/{u}")));
        args.push(dir.to_string_lossy().into_owned());
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        cli.run(&refs, false, None, true)
            .with_context(|| format!("downloading {} photo(s)", chunk.len()))?;
        done += chunk.len();
    }
    Ok(done)
}

/// Trash the given uids in the cloud (batched).
fn trash_remote(cli: &Cli, photos: &[(String, String)]) -> Result<usize> {
    let mut done = 0;
    for chunk in photos.chunks(BATCH) {
        let mut args: Vec<String> = ["fs", "trash"].iter().map(|s| s.to_string()).collect();
        args.extend(chunk.iter().map(|(_, uid)| format!("/photos/{uid}")));
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        cli.run(&refs, false, None, true)
            .with_context(|| format!("trashing {} photo(s)", chunk.len()))?;
        done += chunk.len();
    }
    Ok(done)
}

/// One sync cycle: list both sides, move what is missing, propagate deletions.
pub fn sync(cli: &Cli, dir: &Path, state_dir: &Path, allow_deletes: bool) -> Result<Summary> {
    let remote = timeline(cli)?;
    let local = scan_local(dir);
    let mut state = read_state(state_dir);

    // An empty folder with a populated state is the signature of an unmounted
    // disk, a moved folder or a failed mount - never a library the user just
    // wiped. Refuse to delete anything in that case.
    let folder_looks_present = !(local.is_empty() && !state.synced.is_empty());
    if !folder_looks_present {
        eprintln!(
            "photos: {} is empty but {} photo(s) are known to be synced; skipping deletions",
            dir.display(),
            state.synced.len()
        );
    }

    let mut p = plan(
        &local,
        &remote,
        &state,
        allow_deletes,
        folder_looks_present,
    );
    // A mass delete crawls: whatever is left over is picked up next cycle.
    let mut capped = 0;
    if p.trash_remote.len() > MAX_DELETES {
        capped += p.trash_remote.len() - MAX_DELETES;
        p.trash_remote.truncate(MAX_DELETES);
    }
    if p.remove_local.len() > MAX_DELETES {
        capped += p.remove_local.len() - MAX_DELETES;
        p.remove_local.truncate(MAX_DELETES);
    }
    if capped > 0 {
        eprintln!("photos: {capped} more deletion(s) wait for the next cycle");
    }

    let mut summary = Summary {
        ts: chrono::Utc::now().to_rfc3339(),
        remote: remote.len(),
        local: local.len(),
        conflicts: p.conflicts.len(),
        ..Default::default()
    };
    for name in &p.conflicts {
        if let (Some(l), Some(r)) = (
            local_by_name(&local, name),
            remote_by_name(&remote, name),
        ) {
            state.handled.insert(name.clone(), (l.size, r.size));
            eprintln!(
                "photos: {name} differs (local {} B, cloud {} B); keeping both",
                l.size, r.size
            );
        }
    }
    state.handled.retain(|name, (ls, rs)| {
        local_by_name(&local, name).map(|l| l.size) == Some(*ls)
            && remote_by_name(&remote, name).map(|r| r.size) == Some(*rs)
    });

    summary.uploaded = upload(cli, dir, &p.upload)?;
    summary.downloaded = download(cli, dir, &p.download)?;
    // Deletions last: a failed transfer above must not cost anything.
    summary.trashed = trash_remote(cli, &p.trash_remote)?;
    for name in &p.remove_local {
        match std::fs::remove_file(dir.join(name)) {
            Ok(()) => summary.removed += 1,
            Err(e) => eprintln!("photos: could not remove {}: {e}", dir.join(name).display()),
        }
    }

    // Remember what is in sync now: names the next cycle may delete safely.
    // Transfers above changed both sides, so re-read them: a photo uploaded in
    // this cycle is only in sync once its uid is known, and without that the
    // next cycle would re-download it instead of trashing a local delete.
    let moved = !p.upload.is_empty() || !p.download.is_empty() || !p.trash_remote.is_empty();
    let final_remote = if moved {
        timeline(cli).unwrap_or(remote)
    } else {
        remote
    };
    let final_local = if moved { scan_local(dir) } else { local };
    summary.remote = final_remote.len();
    summary.local = final_local.len();

    state.synced.clear();
    for r in &final_remote {
        if local_by_name(&final_local, &r.name).is_some_and(|l| l.size == r.size) {
            state.synced.insert(
                r.name.clone(),
                Synced {
                    uid: r.uid.clone(),
                    size: r.size,
                },
            );
        }
    }
    // Photos just trashed still need their state entry dropped by name.
    for (name, _) in &p.trash_remote {
        state.synced.remove(name);
    }

    write_json(&state_path(state_dir), &state);
    write_json(&summary_path(state_dir), &summary);
    Ok(summary)
}

/// The photo folder: `PROTON_SYNC_PHOTOS_LOCAL`, else `~/ProtonPhotos`.
pub fn local_dir(values: &BTreeMap<String, String>) -> Option<PathBuf> {
    let raw = values
        .get("PROTON_SYNC_PHOTOS_LOCAL")
        .map(String::as_str)
        .filter(|s| !s.trim().is_empty());
    match raw {
        Some(v) => Some(expand_home(v)),
        None => dirs::home_dir().map(|h| h.join("ProtonPhotos")),
    }
}

fn expand_home(s: &str) -> PathBuf {
    let s = s.trim();
    if s == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    }
    match s.strip_prefix("~/") {
        Some(rest) => dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/"))
            .join(rest),
        None => PathBuf::from(s),
    }
}

/// Is photo sync on? `PROTON_SYNC_PHOTOS=0` turns it off; the folder must
/// exist, which is what the Photos page creates the first time it is opened.
pub fn enabled(values: &BTreeMap<String, String>) -> bool {
    values
        .get("PROTON_SYNC_PHOTOS")
        .map(|v| v != "0")
        .unwrap_or(true)
}

/// Do deletions travel between the two sides? On unless switched off.
pub fn deletes_enabled(values: &BTreeMap<String, String>) -> bool {
    values
        .get("PROTON_SYNC_PHOTOS_DELETES")
        .map(|v| v != "0")
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(name: &str, size: u64) -> LocalPhoto {
        LocalPhoto {
            name: name.to_string(),
            size,
        }
    }

    fn remote(uid: &str, name: &str, size: u64) -> RemotePhoto {
        RemotePhoto {
            uid: uid.to_string(),
            name: name.to_string(),
            size,
        }
    }

    fn synced(name: &str, uid: &str, size: u64) -> (String, Synced) {
        (
            name.to_string(),
            Synced {
                uid: uid.to_string(),
                size,
            },
        )
    }

    #[test]
    fn new_files_go_both_ways() {
        let l = vec![local("mine.jpg", 10), local("both.jpg", 20)];
        let r = vec![remote("u1", "both.jpg", 20), remote("u2", "theirs.jpg", 30)];
        let p = plan(&l, &r, &PhotoState::default(), true, true);
        assert_eq!(p.upload, vec!["mine.jpg"]);
        assert_eq!(p.download, vec!["u2"]);
        assert!(p.conflicts.is_empty());
        assert!(p.trash_remote.is_empty() && p.remove_local.is_empty());
    }

    /// A photo deleted on disk is trashed in the cloud - but only when the
    /// engine knew it was in sync, so a first run never trashes anything.
    #[test]
    fn local_delete_trashes_the_cloud_copy() {
        let r = vec![remote("u1", "gone.jpg", 10)];
        let state = PhotoState {
            synced: [synced("gone.jpg", "u1", 10)].into(),
            ..Default::default()
        };
        let p = plan(&[], &r, &state, true, true);
        assert_eq!(p.trash_remote, vec![("gone.jpg".to_string(), "u1".to_string())]);

        // Unknown to the state: a download, never a delete.
        let fresh = plan(&[], &r, &PhotoState::default(), true, true);
        assert_eq!(fresh.download, vec!["u1"]);
        assert!(fresh.trash_remote.is_empty());
    }

    /// A photo trashed in the cloud removes the local copy - same rule.
    #[test]
    fn cloud_delete_removes_the_local_copy() {
        let l = vec![local("gone.jpg", 10)];
        let state = PhotoState {
            synced: [synced("gone.jpg", "u1", 10)].into(),
            ..Default::default()
        };
        let p = plan(&l, &[], &state, true, true);
        assert_eq!(p.remove_local, vec!["gone.jpg"]);
        assert!(p.upload.is_empty());

        // A file that was never synced is uploaded instead.
        let fresh = plan(&l, &[], &PhotoState::default(), true, true);
        assert_eq!(fresh.upload, vec!["gone.jpg"]);
        assert!(fresh.remove_local.is_empty());
    }

    /// The guards: the switch, and an empty folder that looks unmounted.
    #[test]
    fn deletions_need_the_switch_and_a_live_folder() {
        let l = vec![local("gone.jpg", 10)];
        let state = PhotoState {
            synced: [synced("gone.jpg", "u1", 10)].into(),
            ..Default::default()
        };
        assert!(plan(&l, &[], &state, false, true).remove_local.is_empty());
        assert!(plan(&l, &[], &state, true, false).remove_local.is_empty());
        assert_eq!(plan(&l, &[], &state, true, true).remove_local.len(), 1);
    }

    /// Same name, different bytes: never overwrite, never loop.
    #[test]
    fn size_mismatch_keeps_both_and_reports_once() {
        let l = vec![local("x.jpg", 10)];
        let r = vec![remote("u1", "x.jpg", 99)];
        let mut state = PhotoState::default();
        let first = plan(&l, &r, &state, true, true);
        assert_eq!(first.conflicts, vec!["x.jpg"]);
        assert!(first.upload.is_empty() && first.download.is_empty());
        state.handled.insert("x.jpg".into(), (10, 99));
        assert!(plan(&l, &r, &state, true, true).conflicts.is_empty());
        assert_eq!(plan(&[local("x.jpg", 11)], &r, &state, true, true).conflicts, vec!["x.jpg"]);
    }

    #[test]
    fn only_image_files_are_synced() {
        assert!(is_image("a.JPG"));
        assert!(is_image("b.heic"));
        assert!(is_image("c.png"));
        assert!(!is_image("notes.txt"));
        assert!(!is_image("no-extension"));
    }

    #[test]
    fn the_photo_folder_follows_conf_or_home() {
        let mut conf = BTreeMap::new();
        assert!(local_dir(&conf).unwrap().ends_with("ProtonPhotos"));
        conf.insert("PROTON_SYNC_PHOTOS_LOCAL".into(), "~/Pics".into());
        assert!(local_dir(&conf).unwrap().ends_with("Pics"));
        assert!(enabled(&conf));
        assert!(deletes_enabled(&conf), "deletions are on by default");
        conf.insert("PROTON_SYNC_PHOTOS".into(), "0".into());
        conf.insert("PROTON_SYNC_PHOTOS_DELETES".into(), "0".into());
        assert!(!enabled(&conf));
        assert!(!deletes_enabled(&conf));
    }
}

