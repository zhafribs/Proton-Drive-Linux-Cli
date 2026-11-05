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
    /// Remote uids whose local copy is a 0-byte placeholder to replace.
    pub repair: Vec<String>,
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

/// A repair that failed: the cloud size it was meant to restore, how many
/// attempts failed, and when the last one ran. Persisted so the 0-byte file
/// is not retried on every wake, and so the cloud copy is never trashed just
/// because its local placeholder is gone.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RepairFail {
    pub size: u64,
    pub tries: u32,
    pub last_ts: u64,
}

/// The engine's memory of this folder: which photos are in sync, and which
/// name clashes it already reported (so one is not reported every cycle).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PhotoState {
    pub synced: BTreeMap<String, Synced>,
    pub handled: BTreeMap<String, (u64, u64)>,
    pub repair_failed: BTreeMap<String, RepairFail>,
    /// A previous empty-folder or short-timeline hold. Deletions stay off
    /// until every known remote photo is present locally again, so a partial
    /// GUI download cannot re-arm the empty-folder guard.
    #[serde(default)]
    pub hold: bool,
}

fn remote_by_name<'a>(remote: &'a [RemotePhoto], name: &str) -> Option<&'a RemotePhoto> {
    remote.iter().find(|r| r.name == name)
}
fn local_by_name<'a>(local: &'a [LocalPhoto], name: &str) -> Option<&'a LocalPhoto> {
    local.iter().find(|l| l.name == name)
}

/// Name indexes for the plan: one pass instead of O(n^2) scans on big
/// libraries. Duplicate names keep the first entry, matching the old lookups.
fn index_local(local: &[LocalPhoto]) -> BTreeMap<&str, &LocalPhoto> {
    let mut out = BTreeMap::new();
    for l in local {
        out.entry(l.name.as_str()).or_insert(l);
    }
    out
}
fn index_remote(remote: &[RemotePhoto]) -> BTreeMap<&str, &RemotePhoto> {
    let mut out = BTreeMap::new();
    for r in remote {
        out.entry(r.name.as_str()).or_insert(r);
    }
    out
}

/// Names the timeline holds more than once. Their identity is ambiguous while
/// state is keyed by name, so nothing for them is transferred or trashed.
fn shared_names(remote: &[RemotePhoto]) -> std::collections::BTreeSet<&str> {
    let mut seen = std::collections::BTreeSet::new();
    let mut shared = std::collections::BTreeSet::new();
    for r in remote {
        if !seen.insert(r.name.as_str()) {
            shared.insert(r.name.as_str());
        }
    }
    shared
}

/// Is the initial timeline credible enough to delete by? A first run has no
/// memory and is trusted. After that, skipped entries, an empty timeline, or
/// losing more than half of the known library holds the whole cycle. The
/// post-cycle check uses the known executed-deletion count instead.
fn remote_looks_present(remote: &[RemotePhoto], state: &PhotoState, skipped: usize) -> bool {
    if state.synced.is_empty() {
        // A first run has no baseline to protect and will not delete anything.
        // Unreadable entries are skipped and retried next cycle.
        return true;
    }
    if skipped > 0 {
        return false;
    }
    !remote.is_empty() && remote.len() * 2 >= state.synced.len()
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
    let shared = shared_names(remote);
    let local_index = index_local(local);
    let remote_index = index_remote(remote);

    for l in local {
        let Some(r) = remote_index.get(l.name.as_str()) else {
            // A 0-byte image is a failed write, not a photo: never upload it.
            if l.size == 0 {
                continue;
            }
            match state.synced.get(&l.name) {
                Some(_) => p.remove_local.push(l.name.clone()),
                None => p.upload.push(l.name.clone()),
            }
            continue;
        };
        if shared.contains(l.name.as_str()) || r.size == 0 {
            p.conflicts.push(l.name.clone());
        } else if l.size == 0 {
            // 0-byte placeholder: replace it from the verified cloud copy.
            p.repair.push(r.uid.clone());
        } else if r.size != l.size && state.handled.get(&l.name) != Some(&(l.size, r.size)) {
            p.conflicts.push(l.name.clone());
        }
    }
    for r in remote {
        if local_index.contains_key(r.name.as_str()) {
            continue; // handled above (in sync, repair, conflict)
        }
        if shared.contains(r.name.as_str()) {
            p.conflicts.push(r.name.clone());
            continue;
        }
        match state.synced.get(&r.name) {
            // A failed repair vetoes trashing: the cloud copy may be the only
            // good copy left.
            Some(s) if s.uid == r.uid && state.repair_failed.contains_key(&r.uid) => {
                p.conflicts.push(r.name.clone());
            }
            // We had it synced and the file is gone: the user deleted it here.
            Some(s) if s.uid == r.uid => p.trash_remote.push((r.name.clone(), r.uid.clone())),
            // Never seen before: download it.
            _ => p.download.push(r.uid.clone()),
        }
    }
    p.upload.sort();
    p.download.sort();
    p.download.dedup();
    p.repair.sort();
    p.repair.dedup();
    p.conflicts.sort();
    p.conflicts.dedup();
    p.trash_remote.sort();
    p.remove_local.sort();
    if !allow_deletes
        || !folder_looks_present
        || !remote_looks_present(remote, state, 0)
    {
        // Either the switch is off, or the listing smells like an unmounted
        // disk / bad API response: do not destroy anything, just report what
        // would have happened.
        p.trash_remote.clear();
        p.remove_local.clear();
    }
    p
}

/// Exponential backoff for a failed repair, capped at six hours.
fn backoff_secs(tries: u32) -> u64 {
    (600u64 << tries.saturating_sub(1).min(6)).min(6 * 3600)
}

/// Should this 0-byte file be repaired again? `force` is a manual request for
/// this exact uid and bypasses the backoff once.
pub fn repair_due(
    failed: &BTreeMap<String, RepairFail>,
    uid: &str,
    size: u64,
    now: u64,
    force: bool,
) -> bool {
    if force {
        return true;
    }
    let Some(fail) = failed.get(uid) else {
        return true;
    };
    if fail.size != size {
        return true; // the cloud copy changed; verify it afresh
    }
    if fail.tries >= 6 {
        return false; // automatic retries exhausted; the tile can force one
    }
    now.saturating_sub(fail.last_ts) >= backoff_secs(fail.tries)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
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
    /// Photos that are not fully in sync for a reason the user can act on:
    /// incomplete download, size difference, duplicate name, or a failed
    /// repair that is waiting to retry.
    #[serde(default)]
    pub attention: usize,
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
        out.push(LocalPhoto {
            name,
            size: meta.len(),
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// The cloud timeline (`photo timeline -d --json`).
///
/// Returns the readable photos and how many entries were skipped. A non-array
/// response, or an array whose entries are all unreadable, is an error: turning
/// that into "empty" is how a bad API response becomes a local deletion.
pub fn timeline(cli: &Cli) -> Result<(Vec<RemotePhoto>, usize)> {
    let v = cli.run_json(&["photo", "timeline", "-d"], true)?;
    let arr = v
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("photo timeline: expected a JSON array"))?;
    let mut out = Vec::new();
    let mut skipped = 0usize;
    for e in arr {
        let uid = e.get("uid").and_then(|x| x.as_str()).unwrap_or("");
        let name_obj = e.get("name");
        let ok = name_obj
            .and_then(|n| n.get("ok"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let name = name_obj
            .and_then(|n| n.get("value"))
            .and_then(|n| n.as_str())
            .unwrap_or("");
        if !ok
            || !crate::filter::valid_remote_name(uid)
            || !crate::filter::valid_remote_name(name)
        {
            skipped += 1;
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
    if out.is_empty() && !arr.is_empty() {
        anyhow::bail!(
            "photo timeline: {} entries had no readable name/uid",
            arr.len()
        );
    }
    Ok((out, skipped))
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
    let Ok(text) = serde_json::to_string(v) else {
        return;
    };
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
        args.extend(
            chunk
                .iter()
                .map(|n| dir.join(n).to_string_lossy().into_owned()),
        );
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

/// Persist the state and report whether the write landed. Repair bookkeeping
/// must be durable before a download starts, or the trash veto can be lost.
fn write_state(state_dir: &Path, state: &PhotoState) -> bool {
    let path = state_path(state_dir);
    let Ok(text) = serde_json::to_string(state) else {
        return false;
    };
    let tmp = path.with_extension("json.tmp");
    (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&tmp)?;
        std::io::Write::write_all(&mut file, text.as_bytes())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, &path)?;
        if let Some(parent) = path.parent()
            && let Ok(dir) = std::fs::File::open(parent)
        {
            let _ = dir.sync_all();
        }
        Ok(())
    })()
    .is_ok()
}

/// Local photo removals go beside the folder, on the same filesystem. Renaming
/// into `$XDG_STATE_HOME` fails with EXDEV on the external drives people often
/// use for photos.
fn trash_local_photo(dir: &Path, name: &str) -> Result<()> {
    let trash = dir.join(".proton-photos-trash");
    std::fs::create_dir_all(&trash)?;
    let mut dst = trash.join(name);
    let mut suffix = 1u32;
    while dst.exists() {
        suffix += 1;
        dst = trash.join(format!("{name}.{suffix}"));
    }
    std::fs::rename(dir.join(name), &dst)
        .with_context(|| format!("moving {} to {}", dir.join(name).display(), dst.display()))?;
    Ok(())
}

/// Names whose two copies differ, counted every cycle (not just the first).
fn mismatch_names(
    local: &[LocalPhoto],
    remote: &[RemotePhoto],
    shared: &std::collections::BTreeSet<&str>,
) -> std::collections::BTreeSet<String> {
    let mut out: std::collections::BTreeSet<String> =
        shared.iter().map(|s| (*s).to_string()).collect();
    for l in local {
        let Some(r) = remote_by_name(remote, &l.name) else {
            continue;
        };
        if shared.contains(l.name.as_str()) || (l.size > 0 && r.size > 0 && l.size != r.size) {
            out.insert(l.name.clone());
        }
    }
    out
}

/// Photos the page should warn about: incomplete, differing, duplicate, or a
/// failed repair still waiting. Missing and unpreviewable are normal states.
fn attention_count(local: &[LocalPhoto], remote: &[RemotePhoto], state: &PhotoState) -> usize {
    let shared = shared_names(remote);
    let mut names = mismatch_names(local, remote, &shared);
    for r in remote {
        if let Some(l) = local_by_name(local, &r.name)
            && l.size == 0
            && r.size > 0
        {
            names.insert(r.name.clone());
        }
        if state.repair_failed.contains_key(&r.uid) {
            names.insert(r.name.clone());
        }
    }
    names.len()
}

/// Replace a 0-byte local file with a verified download. The new copy lands in
/// a temp dir first; the placeholder is touched only after its size checks out.
fn repair_one(cli: &Cli, dir: &Path, remote: &RemotePhoto) -> Result<bool> {
    let target = dir.join(&remote.name);
    if let Ok(meta) = std::fs::metadata(&target)
        && (!meta.is_file() || meta.len() != 0)
    {
        return Ok(false); // no longer a placeholder
    }
    if remote.size == 0 {
        return Ok(false); // nothing to verify against
    }
    let tmp_root = dir.join(".proton-repair");
    let _ = std::fs::remove_dir_all(&tmp_root);
    std::fs::create_dir_all(&tmp_root)?;
    let outcome = (|| -> Result<bool> {
        let rpath = format!("/photos/{}", remote.uid);
        let dest = tmp_root.to_string_lossy().into_owned();
        let args = [
            "photo",
            "download",
            "-c",
            "rename",
            rpath.as_str(),
            dest.as_str(),
        ];
        cli.run(&args, false, None, true)?;
        let files: Vec<PathBuf> = std::fs::read_dir(&tmp_root)?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .collect();
        if files.len() != 1 {
            return Ok(false);
        }
        let produced = &files[0];
        if std::fs::metadata(produced)?.len() != remote.size {
            return Ok(false);
        }
        // The user may have written a real file during the download.
        if let Ok(meta) = std::fs::metadata(&target)
            && meta.len() != 0
        {
            return Ok(false);
        }
        std::fs::rename(produced, &target)?;
        Ok(true)
    })();
    let _ = std::fs::remove_dir_all(&tmp_root);
    outcome
}

/// One sync cycle: list both sides, move what is missing, repair 0-byte
/// downloads, and propagate deletions. `force_uids` are manual retries from
/// the Photos page; only those bypass the repair backoff.
pub fn sync(
    cli: &Cli,
    dir: &Path,
    state_dir: &Path,
    allow_deletes: bool,
    force_uids: &[String],
) -> Result<Summary> {
    let (remote, skipped) = match timeline(cli) {
        Ok(v) => v,
        Err(e) => {
            let summary = Summary {
                ts: chrono::Utc::now().to_rfc3339(),
                error: format!(
                    "Photo timeline could not be read: {e:#}. Nothing was changed."
                ),
                attention: 1,
                ..Default::default()
            };
            eprintln!("photos: {}", summary.error);
            write_json(&summary_path(state_dir), &summary);
            return Ok(summary);
        }
    };
    let local = scan_local(dir);
    let mut state = read_state(state_dir);
    let hold_marker = dir.join(".proton-photos-hold");

    // The GUI leaves this marker before downloading into a folder that is
    // missing known photos, so the hold exists even if no engine pass has run
    // since the folder was emptied.
    if hold_marker.is_file() {
        if state.synced.is_empty() {
            let _ = std::fs::remove_file(&hold_marker);
        } else if !state.hold {
            state.hold = true;
            let _ = write_state(state_dir, &state);
        }
    }

    // A previous empty-folder/short-timeline hold is released only when every
    // known photo still in the cloud is present locally. This is what stops a
    // single GUI download from making the folder non-empty and re-arming the
    // guard for the rest of the library.
    if state.hold {
        let known: Vec<&RemotePhoto> = remote
            .iter()
            .filter(|r| state.synced.contains_key(&r.name))
            .collect();
        let missing = known
            .iter()
            .filter(|r| local_by_name(&local, &r.name).is_none())
            .count();
        if missing > 0 {
            let summary = Summary {
                ts: chrono::Utc::now().to_rfc3339(),
                remote: remote.len(),
                local: local.len(),
                error: format!(
                    "Photo folder is recovering: {} of {} known photo(s) are local. Nothing was changed; finish restoring the folder first.",
                    known.len() - missing,
                    known.len()
                ),
                attention: 1,
                ..Default::default()
            };
            eprintln!("photos: {}", summary.error);
            write_json(&summary_path(state_dir), &summary);
            return Ok(summary);
        }
        state.hold = false;
        let _ = write_state(state_dir, &state);
        let _ = std::fs::remove_file(&hold_marker);
    }

    // A short or unreadable timeline is far more likely to be an API hiccup
    // than a mass deletion. Hold the whole cycle, keep the previous state and
    // tell the user instead of deleting 25 photos per cycle.
    if !remote_looks_present(&remote, &state, skipped) {
        state.hold = true;
        let _ = write_state(state_dir, &state);
        let _ = std::fs::write(&hold_marker, b"");
        let summary = Summary {
            ts: chrono::Utc::now().to_rfc3339(),
            remote: remote.len(),
            local: local.len(),
            error: if skipped > 0 {
                format!(
                    "Photo timeline: {skipped} entries could not be read. Nothing was changed; retry later or fix those cloud entries."
                )
            } else {
                format!(
                    "Photo timeline looks incomplete ({} readable, {} known). Nothing was changed. If you deleted these photos in Proton, move the stale local copies out of {} and remove {} to reset the baseline.",
                    remote.len(),
                    state.synced.len(),
                    dir.display(),
                    state_path(state_dir).display()
                )
            },
            attention: 1,
            ..Default::default()
        };
        eprintln!("photos: {}", summary.error);
        write_json(&summary_path(state_dir), &summary);
        return Ok(summary);
    }

    // An empty folder with a populated state is the signature of an unmounted
    // disk, a moved folder or a failed mount - never a library the user just
    // wiped. Hold the whole cycle: downloading a new cloud photo here would
    // make the folder non-empty and defeat the guard next cycle.
    let folder_looks_present = !(local.is_empty() && !state.synced.is_empty());
    if !folder_looks_present {
        state.hold = true;
        let _ = write_state(state_dir, &state);
        let _ = std::fs::write(&hold_marker, b"");
        let summary = Summary {
            ts: chrono::Utc::now().to_rfc3339(),
            remote: remote.len(),
            local: local.len(),
            error: format!(
                "Photo folder {} is empty but {} photo(s) are known to be synced. Nothing was changed. Restore the folder or fix the sync path, or remove {} if you deleted them intentionally.",
                dir.display(),
                state.synced.len(),
                state_path(state_dir).display()
            ),
            attention: 1,
            ..Default::default()
        };
        eprintln!("photos: {}", summary.error);
        write_json(&summary_path(state_dir), &summary);
        return Ok(summary);
    }

    let mut p = plan(&local, &remote, &state, allow_deletes, folder_looks_present);
    // A mass delete crawls: whatever is left over is picked up next cycle, and
    // its state entry is kept so it is not mistaken for a new photo.
    let mut deferred_trash: Vec<(String, String)> = Vec::new();
    let mut deferred_remove: Vec<String> = Vec::new();
    let mut capped = 0;
    if p.trash_remote.len() > MAX_DELETES {
        capped += p.trash_remote.len() - MAX_DELETES;
        deferred_trash = p.trash_remote.split_off(MAX_DELETES);
    }
    if p.remove_local.len() > MAX_DELETES {
        capped += p.remove_local.len() - MAX_DELETES;
        deferred_remove = p.remove_local.split_off(MAX_DELETES);
    }
    if capped > 0 {
        eprintln!("photos: {capped} more deletion(s) wait for the next cycle");
    }

    let now = now_secs();
    let mut summary = Summary {
        ts: chrono::Utc::now().to_rfc3339(),
        remote: remote.len(),
        local: local.len(),
        conflicts: mismatch_names(&local, &remote, &shared_names(&remote)).len(),
        ..Default::default()
    };
    for name in &p.conflicts {
        if let (Some(l), Some(r)) = (local_by_name(&local, name), remote_by_name(&remote, name)) {
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

    // Transfers. A failed batch is remembered, not fatal: the same cycle still
    // tries to repair any 0-byte result it left behind.
    let mut hard: Option<anyhow::Error> = None;
    if let Err(e) = upload(cli, dir, &p.upload) {
        hard = Some(e);
    } else {
        summary.uploaded = p.upload.len();
    }
    if hard.is_none() {
        match download(cli, dir, &p.download) {
            Ok(n) => summary.downloaded = n,
            Err(e) => hard = Some(e),
        }
    }

    let login_error = hard
        .as_ref()
        .is_some_and(|e| proton_core::health::is_login_error(&format!("{e:#}")));
    let mut repaired = false;
    if !login_error {
        let after = scan_local(dir);
        let mut repairs = p.repair.clone();
        for uid in &p.download {
            if let Some(r) = remote.iter().find(|r| &r.uid == uid)
                && let Some(l) = local_by_name(&after, &r.name)
                && l.size == 0
                && r.size > 0
            {
                repairs.push(uid.clone());
            }
        }
        repairs.sort();
        repairs.dedup();
        for uid in repairs {
            let Some(remote) = remote.iter().find(|r| r.uid == uid) else {
                continue;
            };
            let placeholder = std::fs::metadata(dir.join(&remote.name))
                .map(|m| m.is_file() && m.len() == 0)
                .unwrap_or(false);
            if !placeholder {
                continue;
            }
            let force = force_uids.iter().any(|u| u == &uid);
            if !repair_due(&state.repair_failed, &uid, remote.size, now, force) {
                continue;
            }
            // Persist the attempt before downloading: this is what stops a
            // lost 0-byte file from being read as a local delete next cycle.
            let entry = state.repair_failed.entry(uid.clone()).or_default();
            entry.size = remote.size;
            entry.tries = entry.tries.saturating_add(1);
            entry.last_ts = now;
            if !write_state(state_dir, &state) {
                eprintln!("photos: cannot persist repair attempt for {}", remote.name);
                continue;
            }
            match repair_one(cli, dir, remote) {
                Ok(true) => {
                    state.repair_failed.remove(&uid);
                    let _ = write_state(state_dir, &state);
                    summary.downloaded += 1;
                    repaired = true;
                }
                Ok(false) => eprintln!("photos: repair incomplete for {}", remote.name),
                Err(e) => eprintln!("photos: repair failed for {}: {e:#}", remote.name),
            }
        }
    }

    // Deletions last: a failed transfer above must not cost anything.
    if hard.is_none() {
        match trash_remote(cli, &p.trash_remote) {
            Ok(n) => summary.trashed = n,
            Err(e) => hard = Some(e),
        }
        for name in &p.remove_local {
            match trash_local_photo(dir, name) {
                Ok(()) => summary.removed += 1,
                Err(e) => eprintln!("photos: could not trash {}: {e:#}", dir.join(name).display()),
            }
        }
    }

    // Remember what is in sync now. A failed cycle keeps the previous memory
    // rather than rebuilding it from a doubtful scan.
    let moved = !p.upload.is_empty()
        || !p.download.is_empty()
        || !p.trash_remote.is_empty()
        || !p.remove_local.is_empty()
        || repaired;
    // Re-list only when something moved. The post-cycle check may account for
    // deletions this cycle actually executed, but must still see every photo
    // The post-cycle list must still contain every known photo that should
    // remain after the remote deletions this cycle executed.
    let trashed: std::collections::BTreeSet<&str> = p
        .trash_remote
        .iter()
        .map(|(name, _)| name.as_str())
        .collect();
    let expected: Vec<&String> = state
        .synced
        .keys()
        .filter(|name| {
            remote_by_name(&remote, name).is_some() && !trashed.contains(name.as_str())
        })
        .collect();
    let (final_remote, final_trusted) = if moved {
        match timeline(cli) {
            Ok((photos, skipped))
                if skipped == 0
                    && expected
                        .iter()
                        .all(|name| remote_by_name(&photos, name).is_some()) =>
            {
                (photos, true)
            }
            Ok(_) => {
                eprintln!(
                    "photos: post-cycle timeline looks incomplete; keeping the previous state"
                );
                (remote, false)
            }
            Err(e) => {
                eprintln!(
                    "photos: post-cycle timeline failed: {e:#}; keeping the previous state"
                );
                (remote, false)
            }
        }
    } else {
        (remote, true)
    };
    let final_local = if moved { scan_local(dir) } else { local };
    summary.remote = final_remote.len();
    summary.local = final_local.len();
    let shared = shared_names(&final_remote);

    if hard.is_none() && final_trusted && folder_looks_present {
        let old_synced = state.synced.clone();
        state.synced.clear();
        for r in &final_remote {
            if shared.contains(r.name.as_str()) || r.size == 0 {
                continue; // ambiguous, or no verified size to compare
            }
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
        // Photos just trashed must not stay in state as if they were synced.
        for (name, _) in &p.trash_remote {
            state.synced.remove(name);
        }
        // Deferred past the per-cycle cap still exist on both sides; keep
        // their old entry so the next cycle does not treat them as new.
        for (name, _) in &deferred_trash {
            if let Some(old) = old_synced.get(name) {
                state.synced.insert(name.clone(), old.clone());
            }
        }
        for name in &deferred_remove {
            if let Some(old) = old_synced.get(name) {
                state.synced.insert(name.clone(), old.clone());
            }
        }
        // Keep a failed-repair entry while the local copy is still broken or
        // gone: it is the veto that stops the cloud copy from being trashed.
        state.repair_failed.retain(|uid, fail| {
            let Some(r) = final_remote.iter().find(|r| &r.uid == uid) else {
                return false;
            };
            r.size == fail.size
                && !local_by_name(&final_local, &r.name).is_some_and(|l| l.size == r.size)
        });
    }
    summary.conflicts = mismatch_names(&final_local, &final_remote, &shared).len();
    summary.attention = attention_count(&final_local, &final_remote, &state);
    let _ = write_state(state_dir, &state);
    if let Some(e) = hard {
        summary.error = format!("{e:#}");
        write_json(&summary_path(state_dir), &summary);
        return Err(e);
    }
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
        assert_eq!(
            p.trash_remote,
            vec![("gone.jpg".to_string(), "u1".to_string())]
        );

        // Unknown to the state: a download, never a delete.
        let fresh = plan(&[], &r, &PhotoState::default(), true, true);
        assert_eq!(fresh.download, vec!["u1"]);
        assert!(fresh.trash_remote.is_empty());
    }

    /// A photo trashed in the cloud removes the local copy - same rule.
    #[test]
    fn cloud_delete_removes_the_local_copy() {
        let l = vec![local("gone.jpg", 10), local("stay.jpg", 20)];
        let r = vec![remote("u2", "stay.jpg", 20)];
        let state = PhotoState {
            synced: [
                synced("gone.jpg", "u1", 10),
                synced("stay.jpg", "u2", 20),
            ]
            .into(),
            ..Default::default()
        };
        let p = plan(&l, &r, &state, true, true);
        assert_eq!(p.remove_local, vec!["gone.jpg"]);
        assert!(p.upload.is_empty());

        // A file that was never synced is uploaded instead.
        let fresh = plan(&l, &[], &PhotoState::default(), true, true);
        assert_eq!(fresh.upload.len(), 2);
        assert!(fresh.remove_local.is_empty());
    }

    /// The guards: the switch, an empty folder that looks unmounted, and a
    /// timeline that lost most of the known library.
    #[test]
    fn deletions_need_the_switch_and_a_live_folder() {
        let l = vec![local("gone.jpg", 10)];
        let r = vec![remote("u2", "other.jpg", 20)];
        let state = PhotoState {
            synced: [synced("gone.jpg", "u1", 10)].into(),
            ..Default::default()
        };
        assert!(plan(&l, &r, &state, false, true).remove_local.is_empty());
        assert!(plan(&l, &r, &state, true, false).remove_local.is_empty());
        assert_eq!(plan(&l, &r, &state, true, true).remove_local.len(), 1);
    }

    #[test]
    fn empty_or_short_timeline_holds_deletions() {
        let locals = (0..10)
            .map(|i| local(&format!("p{i}.jpg"), 10))
            .collect::<Vec<_>>();
        let state = PhotoState {
            synced: (0..10)
                .map(|i| synced(&format!("p{i}.jpg"), &format!("u{i}"), 10))
                .collect(),
            ..Default::default()
        };
        // Empty remote: never delete.
        let empty = plan(&locals, &[], &state, true, true);
        assert!(empty.remove_local.is_empty());
        // Three of ten known photos: too short, hold.
        let short = (0..3)
            .map(|i| remote(&format!("u{i}"), &format!("p{i}.jpg"), 10))
            .collect::<Vec<_>>();
        assert!(plan(&locals, &short, &state, true, true).remove_local.is_empty());
        // Half the library is the line; five of ten is allowed.
        let half = (0..5)
            .map(|i| remote(&format!("u{i}"), &format!("p{i}.jpg"), 10))
            .collect::<Vec<_>>();
        assert_eq!(
            plan(&locals, &half, &state, true, true).remove_local.len(),
            5
        );
        // A short timeline plus a partial local scan must not trash remote
        // photos that are present in the short listing but absent locally.
        let partial_local = vec![local("p0.jpg", 10)];
        let short_remote = (0..3)
            .map(|i| remote(&format!("u{i}"), &format!("p{i}.jpg"), 10))
            .collect::<Vec<_>>();
        assert!(
            plan(&partial_local, &short_remote, &state, true, true)
                .trash_remote
                .is_empty()
        );
    }

    #[test]
    fn remote_presence_guard_counts_unreadable_entries() {
        let state = PhotoState {
            synced: [synced("a.jpg", "u1", 10)].into(),
            ..Default::default()
        };
        let r = vec![remote("u1", "a.jpg", 10)];
        assert!(remote_looks_present(&r, &state, 0));
        assert!(!remote_looks_present(&r, &state, 1));
        assert!(remote_looks_present(&[], &PhotoState::default(), 0));
        // A first run has no baseline to protect: skipped rows are retried,
        // not a reason to freeze the whole photo pass.
        assert!(remote_looks_present(&[], &PhotoState::default(), 1));
        assert!(!remote_looks_present(&[], &state, 0));
    }

    #[test]
    fn local_photo_removals_go_to_trash() {
        let dir = std::env::temp_dir().join(format!("pd-photo-trash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.jpg"), b"one").unwrap();
        std::fs::write(dir.join("b.jpg"), b"two").unwrap();
        trash_local_photo(&dir, "a.jpg").unwrap();
        // A second file with the same name must not overwrite the first.
        std::fs::write(dir.join("a.jpg"), b"again").unwrap();
        trash_local_photo(&dir, "a.jpg").unwrap();
        assert_eq!(std::fs::read(dir.join(".proton-photos-trash/a.jpg")).unwrap(), b"one");
        assert_eq!(
            std::fs::read(dir.join(".proton-photos-trash/a.jpg.2")).unwrap(),
            b"again"
        );
        assert!(!dir.join("a.jpg").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unreadable_timeline_writes_a_visible_summary() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("pd-photo-timeline-sum-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let photos = root.join("photos");
        let state_dir = root.join("state");
        std::fs::create_dir_all(&photos).unwrap();
        std::fs::create_dir_all(&state_dir).unwrap();
        std::fs::write(photos.join("a.jpg"), b"12345").unwrap();
        let state = PhotoState {
            synced: [synced("a.jpg", "u1", 5)].into(),
            ..Default::default()
        };
        assert!(write_state(&state_dir, &state));
        let fake = root.join("fake-proton-drive");
        std::fs::write(&fake, "#!/bin/sh\necho '{}'\n").unwrap();
        let mut perms = std::fs::metadata(&fake).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake, perms).unwrap();
        let cli = Cli::for_test(fake);

        let summary = sync(&cli, &photos, &state_dir, true, &[]).unwrap();
        assert!(summary.error.contains("could not be read"));
        assert_eq!(summary.attention, 1);
        assert_eq!(std::fs::read(photos.join("a.jpg")).unwrap(), b"12345");
        assert_eq!(read_state(&state_dir).synced.len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn unreadable_timeline_is_an_error_not_an_empty_library() {        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("pd-photo-timeline-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("fake-proton-drive");
        std::fs::write(&fake, "#!/bin/sh\necho '{}'\n").unwrap();
        let mut perms = std::fs::metadata(&fake).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake, perms).unwrap();
        let cli = Cli::for_test(fake.clone());

        assert!(timeline(&cli).is_err());
        std::fs::write(&fake, "#!/bin/sh\necho '[]'\n").unwrap();
        assert_eq!(timeline(&cli).unwrap().0.len(), 0);
        std::fs::write(
            &fake,
            "#!/bin/sh\necho '[{\"uid\":\"../x\",\"name\":{\"ok\":true,\"value\":\"a.jpg\"}}]'\n",
        )
        .unwrap();
        assert!(timeline(&cli).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_empty_timeline_holds_the_whole_cycle_and_keeps_state() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("pd-photo-hold-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let photos = root.join("photos");
        let state_dir = root.join("state");
        std::fs::create_dir_all(&photos).unwrap();
        std::fs::create_dir_all(&state_dir).unwrap();
        std::fs::write(photos.join("a.jpg"), b"local").unwrap();
        let state = PhotoState {
            synced: [synced("a.jpg", "u1", 5)].into(),
            ..Default::default()
        };
        assert!(write_state(&state_dir, &state));
        let fake = root.join("fake-proton-drive");
        std::fs::write(&fake, "#!/bin/sh\necho '[]'\n").unwrap();
        let mut perms = std::fs::metadata(&fake).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake, perms).unwrap();
        let cli = Cli::for_test(fake);

        let summary = sync(&cli, &photos, &state_dir, true, &[]).unwrap();
        assert!(summary.error.contains("incomplete"));
        assert_eq!(summary.removed, 0);
        assert_eq!(std::fs::read(photos.join("a.jpg")).unwrap(), b"local");
        assert_eq!(read_state(&state_dir).synced.len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn engine_side_bulk_delete_does_not_lock_the_next_cycle() {        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("pd-photo-bulk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let photos = root.join("photos");
        let state_dir = root.join("state");
        std::fs::create_dir_all(&photos).unwrap();
        std::fs::create_dir_all(&state_dir).unwrap();
        let mut state = PhotoState::default();
        for i in 0..30 {
            let name = format!("p{i}.jpg");
            if i < 10 {
                std::fs::write(photos.join(&name), b"12345").unwrap();
            }
            state.synced.insert(
                name.clone(),
                Synced {
                    uid: format!("u{i}"),
                    size: 5,
                },
            );
        }
        assert!(write_state(&state_dir, &state));
        let json = |range: std::ops::Range<usize>| {
            let rows: Vec<String> = range
                .map(|i| {
                    format!(
                        "{{\"uid\":\"u{i}\",\"name\":{{\"ok\":true,\"value\":\"p{i}.jpg\"}},\"activeRevision\":{{\"claimedSize\":5}}}}"
                    )
                })
                .collect();
            format!("[{}]", rows.join(","))
        };
        std::fs::write(root.join("t1.json"), json(0..30)).unwrap();
        std::fs::write(root.join("t2.json"), json(0..10)).unwrap();
        let fake = root.join("fake-proton-drive");
        std::fs::write(
            &fake,
            concat!(
                "#!/bin/sh\n",
                "dir=$(dirname \"$0\")\n",
                "if [ \"$1\" = photo ] && [ \"$2\" = timeline ]; then\n",
                "  n=$(cat \"$dir/count\" 2>/dev/null || echo 0)\n",
                "  if [ \"$n\" = 0 ]; then cat \"$dir/t1.json\"; echo 1 > \"$dir/count\"; else cat \"$dir/t2.json\"; fi\n",
                "else\n",
                "  echo '[]'\n",
                "fi\n",
            ),
        )
        .unwrap();
        let mut perms = std::fs::metadata(&fake).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake, perms).unwrap();
        let cli = Cli::for_test(fake);

        let summary = sync(&cli, &photos, &state_dir, true, &[]).unwrap();
        assert_eq!(summary.trashed, 20);
        assert_eq!(read_state(&state_dir).synced.len(), 10);
        for i in 0..10 {
            assert!(photos.join(format!("p{i}.jpg")).exists());
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// More deletions than the per-cycle cap must crawl across cycles without
    /// re-downloading the deferred photos as "new".
    #[test]
    fn bulk_delete_over_the_cap_crawls_without_re_download() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("pd-photo-cap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let photos = root.join("photos");
        let state_dir = root.join("state");
        std::fs::create_dir_all(&photos).unwrap();
        std::fs::create_dir_all(&state_dir).unwrap();
        std::fs::write(photos.join("p00.jpg"), b"12345").unwrap();
        let mut state = PhotoState::default();
        for i in 0..30 {
            state.synced.insert(
                format!("p{i:02}.jpg"),
                Synced {
                    uid: format!("u{i}"),
                    size: 5,
                },
            );
        }
        assert!(write_state(&state_dir, &state));
        let json = |ids: &[usize]| {
            let rows: Vec<String> = ids
                .iter()
                .map(|i| {
                    format!(
                        "{{\"uid\":\"u{i}\",\"name\":{{\"ok\":true,\"value\":\"p{i:02}.jpg\"}},\"activeRevision\":{{\"claimedSize\":5}}}}"
                    )
                })
                .collect();
            format!("[{}]", rows.join(","))
        };
        std::fs::write(root.join("t1.json"), json(&(0..30).collect::<Vec<_>>())).unwrap();
        std::fs::write(root.join("t2.json"), json(&[0, 26, 27, 28, 29])).unwrap();
        std::fs::write(root.join("t3.json"), json(&[0])).unwrap();
        let fake = root.join("fake-proton-drive");
        std::fs::write(
            &fake,
            concat!(
                "#!/bin/sh\n",
                "dir=$(dirname \"$0\")\n",
                "if [ \"$1\" = photo ] && [ \"$2\" = timeline ]; then\n",
                "  n=$(cat \"$dir/count\" 2>/dev/null || echo 0)\n",
                "  case \"$n\" in\n",
                "    0) cat \"$dir/t1.json\"; echo 1 > \"$dir/count\" ;;\n",
                "    1|2) cat \"$dir/t2.json\"; echo $((n+1)) > \"$dir/count\" ;;\n",
                "    *) cat \"$dir/t3.json\" ;;\n",
                "  esac\n",
                "else\n",
                "  echo '[]'\n",
                "fi\n",
            ),
        )
        .unwrap();
        let mut perms = std::fs::metadata(&fake).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake, perms).unwrap();
        let cli = Cli::for_test(fake);

        let first = sync(&cli, &photos, &state_dir, true, &[]).unwrap();
        assert_eq!(first.trashed, 25);
        assert_eq!(read_state(&state_dir).synced.len(), 5);
        assert!(photos.join("p00.jpg").exists());

        let second = sync(&cli, &photos, &state_dir, true, &[]).unwrap();
        assert_eq!(second.trashed, 4);
        assert_eq!(read_state(&state_dir).synced.len(), 1);
        assert!(photos.join("p00.jpg").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn empty_folder_holds_the_whole_cycle() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("pd-photo-empty-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let photos = root.join("photos");
        let state_dir = root.join("state");
        std::fs::create_dir_all(&photos).unwrap();
        std::fs::create_dir_all(&state_dir).unwrap();
        let state = PhotoState {
            synced: (0..4)
                .map(|i| synced(&format!("p{i}.jpg"), &format!("u{i}"), 5))
                .collect(),
            ..Default::default()
        };
        assert!(write_state(&state_dir, &state));
        let fake = root.join("fake-proton-drive");
        std::fs::write(
            &fake,
            "#!/bin/sh\ndir=$(dirname \"$0\")\necho \"$@\" >> \"$dir/calls.log\"\nif [ \"$1\" = photo ] && [ \"$2\" = timeline ]; then echo '[{\"uid\":\"u0\",\"name\":{\"ok\":true,\"value\":\"p0.jpg\"},\"activeRevision\":{\"claimedSize\":5}},{\"uid\":\"u1\",\"name\":{\"ok\":true,\"value\":\"p1.jpg\"},\"activeRevision\":{\"claimedSize\":5}},{\"uid\":\"u2\",\"name\":{\"ok\":true,\"value\":\"p2.jpg\"},\"activeRevision\":{\"claimedSize\":5}},{\"uid\":\"u3\",\"name\":{\"ok\":true,\"value\":\"p3.jpg\"},\"activeRevision\":{\"claimedSize\":5}}]'; else echo '[]'; fi\n",
        )
        .unwrap();
        let mut perms = std::fs::metadata(&fake).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake, perms).unwrap();
        let cli = Cli::for_test(fake);
        let images = || {
            std::fs::read_dir(&photos)
                .unwrap()
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().ends_with(".jpg"))
                .count()
        };

        let first = sync(&cli, &photos, &state_dir, true, &[]).unwrap();
        assert!(first.error.contains("empty"));
        assert!(read_state(&state_dir).hold);
        assert!(photos.join(".proton-photos-hold").is_file());
        assert_eq!(read_state(&state_dir).synced.len(), 4);
        assert_eq!(images(), 0);

        // The GUI can download one tile while the hold is active. That must
        // not re-arm the empty-folder guard and trash the rest.
        std::fs::write(photos.join("p0.jpg"), b"12345").unwrap();
        let partial = sync(&cli, &photos, &state_dir, true, &[]).unwrap();
        assert!(partial.error.contains("recovering"));
        assert!(read_state(&state_dir).hold);
        assert_eq!(read_state(&state_dir).synced.len(), 4);
        assert_eq!(images(), 1);

        // Once the known library is back, the hold releases without deleting.
        for i in 1..4 {
            std::fs::write(photos.join(format!("p{i}.jpg")), b"12345").unwrap();
        }
        let recovered = sync(&cli, &photos, &state_dir, true, &[]).unwrap();
        assert!(recovered.error.is_empty());
        assert!(!read_state(&state_dir).hold);
        assert!(!photos.join(".proton-photos-hold").exists());
        assert_eq!(read_state(&state_dir).synced.len(), 4);
        assert_eq!(images(), 4);

        let calls = std::fs::read_to_string(root.join("calls.log")).unwrap();
        assert_eq!(calls.lines().count(), 3);
        assert!(calls.lines().all(|l| l.starts_with("photo timeline")));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The GUI can download before the engine has seen the empty folder. Its
    /// marker must arm the hold on the very first pass.
    #[test]
    fn gui_hold_marker_arms_before_the_first_sync() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("pd-photo-gui-race-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let photos = root.join("photos");
        let state_dir = root.join("state");
        std::fs::create_dir_all(&photos).unwrap();
        std::fs::create_dir_all(&state_dir).unwrap();
        let state = PhotoState {
            synced: (0..4)
                .map(|i| synced(&format!("p{i}.jpg"), &format!("u{i}"), 5))
                .collect(),
            ..Default::default()
        };
        assert!(write_state(&state_dir, &state));
        // Simulate the GUI: one tile downloaded, marker written, no engine pass.
        std::fs::write(photos.join("p0.jpg"), b"12345").unwrap();
        std::fs::write(photos.join(".proton-photos-hold"), b"").unwrap();
        let fake = root.join("fake-proton-drive");
        std::fs::write(
            &fake,
            "#!/bin/sh\ndir=$(dirname \"$0\")\necho \"$@\" >> \"$dir/calls.log\"\nif [ \"$1\" = photo ] && [ \"$2\" = timeline ]; then echo '[{\"uid\":\"u0\",\"name\":{\"ok\":true,\"value\":\"p0.jpg\"},\"activeRevision\":{\"claimedSize\":5}},{\"uid\":\"u1\",\"name\":{\"ok\":true,\"value\":\"p1.jpg\"},\"activeRevision\":{\"claimedSize\":5}},{\"uid\":\"u2\",\"name\":{\"ok\":true,\"value\":\"p2.jpg\"},\"activeRevision\":{\"claimedSize\":5}},{\"uid\":\"u3\",\"name\":{\"ok\":true,\"value\":\"p3.jpg\"},\"activeRevision\":{\"claimedSize\":5}}]'; else echo '[]'; fi\n",
        )
        .unwrap();
        let mut perms = std::fs::metadata(&fake).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake, perms).unwrap();
        let cli = Cli::for_test(fake);

        let held = sync(&cli, &photos, &state_dir, true, &[]).unwrap();
        assert!(held.error.contains("recovering"));
        assert!(read_state(&state_dir).hold);
        assert_eq!(read_state(&state_dir).synced.len(), 4);

        for i in 1..4 {
            std::fs::write(photos.join(format!("p{i}.jpg")), b"12345").unwrap();
        }
        let recovered = sync(&cli, &photos, &state_dir, true, &[]).unwrap();
        assert!(recovered.error.is_empty());
        assert!(!read_state(&state_dir).hold);
        assert!(!photos.join(".proton-photos-hold").exists());

        let calls = std::fs::read_to_string(root.join("calls.log")).unwrap();
        assert_eq!(calls.lines().count(), 2);
        assert!(calls.lines().all(|l| l.starts_with("photo timeline")));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A cycle can hit the 25-per-cycle cap in both directions at once. The
    /// post-cycle baseline must count only remote deletions, or the next cycle
    /// holds forever.
    #[test]
    fn mixed_over_cap_deletes_crawl_without_locking() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("pd-photo-mixed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let photos = root.join("photos");
        let state_dir = root.join("state");
        std::fs::create_dir_all(&photos).unwrap();
        std::fs::create_dir_all(&state_dir).unwrap();
        let mut state = PhotoState::default();
        for i in 0..60 {
            let name = format!("p{i:02}.jpg");
            if i >= 30 {
                std::fs::write(photos.join(&name), b"12345").unwrap();
            }
            state.synced.insert(
                name.clone(),
                Synced {
                    uid: format!("u{i}"),
                    size: 5,
                },
            );
        }
        assert!(write_state(&state_dir, &state));
        let json = |ids: &[usize]| {
            let rows: Vec<String> = ids
                .iter()
                .map(|i| {
                    format!(
                        "{{\"uid\":\"u{i}\",\"name\":{{\"ok\":true,\"value\":\"p{i:02}.jpg\"}},\"activeRevision\":{{\"claimedSize\":5}}}}"
                    )
                })
                .collect();
            format!("[{}]", rows.join(","))
        };
        std::fs::write(root.join("t1.json"), json(&(0..30).collect::<Vec<_>>())).unwrap();
        std::fs::write(root.join("t2.json"), json(&[25, 26, 27, 28, 29])).unwrap();
        std::fs::write(root.join("t3.json"), json(&[])).unwrap();
        let fake = root.join("fake-proton-drive");
        std::fs::write(
            &fake,
            concat!(
                "#!/bin/sh\n",
                "dir=$(dirname \"$0\")\n",
                "if [ \"$1\" = photo ] && [ \"$2\" = timeline ]; then\n",
                "  n=$(cat \"$dir/count\" 2>/dev/null || echo 0)\n",
                "  case \"$n\" in\n",
                "    0) cat \"$dir/t1.json\"; echo 1 > \"$dir/count\" ;;\n",
                "    1|2) cat \"$dir/t2.json\"; echo $((n+1)) > \"$dir/count\" ;;\n",
                "    *) cat \"$dir/t3.json\" ;;\n",
                "  esac\n",
                "else\n",
                "  echo '[]'\n",
                "fi\n",
            ),
        )
        .unwrap();
        let mut perms = std::fs::metadata(&fake).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake, perms).unwrap();
        let cli = Cli::for_test(fake);

        let first = sync(&cli, &photos, &state_dir, true, &[]).unwrap();
        assert_eq!(first.trashed, 25);
        assert_eq!(first.removed, 25);
        assert_eq!(read_state(&state_dir).synced.len(), 10);

        let second = sync(&cli, &photos, &state_dir, true, &[]).unwrap();
        assert_eq!(second.trashed, 5);
        assert_eq!(second.removed, 5);
        assert_eq!(read_state(&state_dir).synced.len(), 0);
        assert_eq!(std::fs::read_dir(&photos).unwrap().count(), 1); // the trash dir
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_short_timeline_holds_the_whole_cycle() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("pd-photo-short-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let photos = root.join("photos");
        let state_dir = root.join("state");
        std::fs::create_dir_all(&photos).unwrap();
        std::fs::create_dir_all(&state_dir).unwrap();
        let mut state = PhotoState::default();
        for i in 0..10 {
            let name = format!("p{i}.jpg");
            std::fs::write(photos.join(&name), b"12345").unwrap();
            state.synced.insert(
                name.clone(),
                Synced {
                    uid: format!("u{i}"),
                    size: 5,
                },
            );
        }
        assert!(write_state(&state_dir, &state));
        let fake = root.join("fake-proton-drive");
        std::fs::write(
            &fake,
            "#!/bin/sh\necho '[{\"uid\":\"u0\",\"name\":{\"ok\":true,\"value\":\"p0.jpg\"},\"activeRevision\":{\"claimedSize\":5}},{\"uid\":\"u1\",\"name\":{\"ok\":true,\"value\":\"p1.jpg\"},\"activeRevision\":{\"claimedSize\":5}},{\"uid\":\"u2\",\"name\":{\"ok\":true,\"value\":\"p2.jpg\"},\"activeRevision\":{\"claimedSize\":5}}]'\n",
        )
        .unwrap();
        let mut perms = std::fs::metadata(&fake).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake, perms).unwrap();
        let cli = Cli::for_test(fake);

        let summary = sync(&cli, &photos, &state_dir, true, &[]).unwrap();
        assert!(summary.error.contains("incomplete"));
        assert_eq!(read_state(&state_dir).synced.len(), 10);
        for i in 0..10 {
            assert!(photos.join(format!("p{i}.jpg")).exists());
        }
        let _ = std::fs::remove_dir_all(&root);
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
        assert_eq!(
            plan(&[local("x.jpg", 11)], &r, &state, true, true).conflicts,
            vec!["x.jpg"]
        );
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
    fn zero_byte_local_copy_is_a_repair_not_a_conflict() {
        let l = vec![local("broken.jpg", 0)];
        let r = vec![remote("u1", "broken.jpg", 99)];
        let mut state = PhotoState::default();
        // An old conflict memory must not hide the repair.
        state.handled.insert("broken.jpg".into(), (0, 99));
        let p = plan(&l, &r, &state, true, true);
        assert_eq!(p.repair, vec!["u1"]);
        assert!(p.conflicts.is_empty());
        assert!(p.download.is_empty() && p.upload.is_empty() && p.trash_remote.is_empty());
    }

    #[test]
    fn zero_byte_local_with_no_cloud_copy_is_never_uploaded() {
        let p = plan(
            &[local("empty.png", 0)],
            &[],
            &PhotoState::default(),
            true,
            true,
        );
        assert!(p.upload.is_empty());
        assert!(p.download.is_empty() && p.remove_local.is_empty());
    }

    #[test]
    fn shared_cloud_names_transfer_nothing_and_never_trash() {
        let r = vec![remote("u1", "same.jpg", 10), remote("u2", "same.jpg", 20)];
        let state = PhotoState {
            synced: [synced("same.jpg", "u1", 10)].into(),
            ..Default::default()
        };
        let p = plan(&[], &r, &state, true, true);
        assert!(p.download.is_empty() && p.trash_remote.is_empty());
        assert_eq!(p.conflicts, vec!["same.jpg"]);

        let p2 = plan(&[local("same.jpg", 10)], &r, &state, true, true);
        assert!(p2.upload.is_empty() && p2.download.is_empty() && p2.trash_remote.is_empty());
    }

    #[test]
    fn a_failed_repair_vetoes_trashing_the_cloud_copy() {
        let r = vec![remote("u1", "gone.jpg", 10)];
        let mut state = PhotoState {
            synced: [synced("gone.jpg", "u1", 10)].into(),
            ..Default::default()
        };
        state.repair_failed.insert(
            "u1".into(),
            RepairFail {
                size: 10,
                tries: 1,
                last_ts: 0,
            },
        );
        let p = plan(&[], &r, &state, true, true);
        assert!(p.trash_remote.is_empty());
        assert_eq!(p.conflicts, vec!["gone.jpg"]);
    }

    #[test]
    fn repair_backoff_is_bounded_and_resets_on_size_change() {
        let mut failed = BTreeMap::new();
        assert!(repair_due(&failed, "u1", 10, 100, false));
        failed.insert(
            "u1".into(),
            RepairFail {
                size: 10,
                tries: 1,
                last_ts: 100,
            },
        );
        assert!(!repair_due(&failed, "u1", 10, 699, false));
        assert!(repair_due(&failed, "u1", 10, 700, false));
        assert!(repair_due(&failed, "u1", 11, 100, false));
        failed.get_mut("u1").unwrap().tries = 6;
        assert!(!repair_due(&failed, "u1", 10, u64::MAX, false));
        assert!(
            repair_due(&failed, "u1", 10, u64::MAX, true),
            "manual retry"
        );
    }

    #[test]
    fn attention_counts_each_name_once() {
        let l = vec![local("a.jpg", 10), local("b.jpg", 0), local("ok.jpg", 5)];
        let r = vec![
            remote("u1", "a.jpg", 99),
            remote("u2", "b.jpg", 20),
            remote("u3", "ok.jpg", 5),
            remote("u4", "missing.jpg", 1),
        ];
        let shared = shared_names(&r);
        assert_eq!(mismatch_names(&l, &r, &shared).len(), 1);
        assert_eq!(attention_count(&l, &r, &PhotoState::default()), 2);
    }

    fn fake_cli(root: &std::path::Path, body: &str) -> Cli {
        use std::os::unix::fs::PermissionsExt;
        let path = root.join("fake-cli");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        Cli::for_test(path)
    }

    fn test_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("pd-photos-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("photos")).unwrap();
        std::fs::create_dir_all(root.join("state")).unwrap();
        root
    }

    const TIMELINE: &str = r#"[{"uid":"u1","name":{"ok":true,"value":"a.png"},"activeRevision":{"claimedSize":100}},{"uid":"u2","name":{"ok":true,"value":"b.png"},"activeRevision":{"claimedSize":200}}]"#;

    /// A failed repair is on disk before the download starts, so a later
    /// deleted placeholder can never trash the only good copy.
    #[test]
    fn failed_repair_is_persisted_and_vetoes_trash_next_cycle() {
        let root = test_root("veto");
        let photos = root.join("photos");
        let state_dir = root.join("state");
        let marker = root.join("trashed");
        let mut state = PhotoState::default();
        state.synced.insert(
            "a.png".into(),
            Synced {
                uid: "u1".into(),
                size: 100,
            },
        );
        write_json(&state_path(&state_dir), &state);
        std::fs::write(photos.join("a.png"), b"").unwrap();
        let script = format!(
            "case \"$*\" in\n\
             *timeline*) printf '%s' '{TIMELINE}' ;;\n\
             *trash*) : > '{}' ;;\n\
             *download*) echo 'download failed' >&2; exit 1 ;;\n\
             esac\n",
            marker.display()
        );
        let cli = fake_cli(&root, &script);

        assert!(sync(&cli, &photos, &state_dir, true, &[]).is_err());
        let persisted = read_state(&state_dir);
        assert!(
            persisted.repair_failed.contains_key("u1"),
            "attempt is durable"
        );

        std::fs::remove_file(photos.join("a.png")).unwrap();
        let _ = sync(&cli, &photos, &state_dir, true, &[]);
        assert!(!marker.exists(), "must not trash the cloud copy");
        assert!(read_state(&state_dir).repair_failed.contains_key("u1"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A verified temp download replaces the 0-byte placeholder and leaves the
    /// repair bookkeeping clean.
    #[test]
    fn a_verified_repair_replaces_the_zero_byte_file() {
        let root = test_root("repair-ok");
        let photos = root.join("photos");
        let state_dir = root.join("state");
        std::fs::write(photos.join("a.png"), b"").unwrap();
        let script = format!(
            "case \"$*\" in\n\
             *timeline*) printf '%s' '{TIMELINE}' ;;\n\
             *download*) mkdir -p \"$6\"\n\
               case \"$6\" in\n\
                 *.proton-repair*) head -c 100 /dev/zero > \"$6/a.png\" ;;\n\
                 *) head -c 200 /dev/zero > \"$6/b.png\" ;;\n\
               esac ;;\n\
             esac\n"
        );
        let cli = fake_cli(&root, &script);
        let out = sync(&cli, &photos, &state_dir, true, &[]).unwrap();
        assert_eq!(std::fs::metadata(photos.join("a.png")).unwrap().len(), 100);
        assert!(read_state(&state_dir).repair_failed.is_empty());
        assert!(out.downloaded >= 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// If the user writes a real file while the repair downloads, the verified
    /// temp copy is discarded instead of overwriting it.
    #[test]
    fn repair_never_overwrites_a_file_that_appeared_meanwhile() {
        let root = test_root("repair-race");
        let photos = root.join("photos");
        let state_dir = root.join("state");
        let target = photos.join("a.png");
        std::fs::write(&target, b"").unwrap();
        let script = format!(
            "case \"$*\" in\n\
             *timeline*) printf '%s' '{TIMELINE}' ;;\n\
             *download*) mkdir -p \"$6\"; head -c 100 /dev/zero > \"$6/a.png\"; printf real > '{}' ;;\n\
             esac\n",
            target.display()
        );
        let cli = fake_cli(&root, &script);
        let _ = sync(&cli, &photos, &state_dir, true, &[]);
        assert_eq!(std::fs::read(&target).unwrap(), b"real");
        let _ = std::fs::remove_dir_all(&root);
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
