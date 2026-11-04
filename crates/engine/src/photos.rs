//! Two-way sync between the cloud photo timeline (`/photos`) and a local
//! folder (default `~/ProtonPhotos`).
//!
//! Deliberately additive. A photo that exists only locally is uploaded, one
//! that exists only in the cloud is downloaded, and nothing is ever deleted or
//! overwritten: the official CLI has no photo-trash command, and a photo is the
//! one file type where guessing wrong costs something irreplaceable. A name
//! that exists on both sides with different sizes is reported and left alone.

use crate::cli::Cli;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Extensions the timeline accepts; anything else in the folder is ignored.
const IMAGE_EXTS: [&str; 9] = [
    "jpg", "jpeg", "png", "heic", "heif", "webp", "gif", "tif", "tiff",
];

/// How many paths go into one CLI call (uploads and downloads both).
const BATCH: usize = 25;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RemotePhoto {
    /// Node uid: `photo download` wants `/photos/<uid>`.
    pub uid: String,
    pub name: String,
    /// Stored size in the cloud (what the timeline reports).
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
}

/// Pairs already left alone, with the sizes seen then. Without this the same
/// conflict would be reported (and reconsidered) on every cycle; a changed size
/// on either side clears the entry so the pair is looked at again.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Handled(BTreeMap<String, (u64, u64)>);

/// The plan for one cycle - pure, so the rules are unit-tested.
pub fn plan(local: &[LocalPhoto], remote: &[RemotePhoto], handled: &Handled) -> PhotoPlan {
    let mut p = PhotoPlan::default();
    let remote_of = |name: &str| remote.iter().find(|r| r.name == name);
    for l in local {
        match remote_of(&l.name) {
            // New on this machine: upload it.
            None => p.upload.push(l.name.clone()),
            // Same name, same stored size: assume in sync.
            Some(r) if r.size == l.size => {}
            // Same name, different bytes: never overwrite either copy.
            Some(r) => {
                if handled.0.get(&l.name) != Some(&(l.size, r.size)) {
                    p.conflicts.push(l.name.clone());
                }
            }
        }
    }
    for r in remote {
        if !local.iter().any(|l| l.name == r.name) {
            p.download.push(r.uid.clone());
        }
    }
    p.upload.sort();
    p.download.sort();
    p.download.dedup();
    p.conflicts.sort();
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
        // The size to compare with the file on disk: `claimedSize` is the
        // original file size; `totalStorageSize` is the encrypted size and
        // would make every photo look like a conflict.
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

fn read_handled(state_dir: &Path) -> Handled {
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


/// One sync cycle: list both sides, move what is missing, never delete.
pub fn sync(cli: &Cli, dir: &Path, state_dir: &Path) -> Result<Summary> {
    let remote = timeline(cli)?;
    let local = scan_local(dir);
    let mut handled = read_handled(state_dir);
    let p = plan(&local, &remote, &handled);

    let mut summary = Summary {
        ts: chrono::Utc::now().to_rfc3339(),
        remote: remote.len(),
        local: local.len(),
        conflicts: p.conflicts.len(),
        ..Default::default()
    };
    for name in &p.conflicts {
        // Remember the pair so the next cycle does not report it again.
        if let (Some(l), Some(r)) = (
            local.iter().find(|l| &l.name == name),
            remote.iter().find(|r| &r.name == name),
        ) {
            handled.0.insert(name.clone(), (l.size, r.size));
            eprintln!(
                "photos: {name} differs (local {} B, cloud {} B); keeping both",
                l.size, r.size
            );
        }
    }
    // Forget entries whose sizes changed: that pair needs a fresh look.
    handled.0.retain(|name, (ls, rs)| {
        let l = local.iter().find(|l| &l.name == name).map(|l| l.size);
        let r = remote.iter().find(|r| &r.name == name).map(|r| r.size);
        l == Some(*ls) && r == Some(*rs)
    });

    summary.uploaded = upload(cli, dir, &p.upload)?;
    summary.downloaded = download(cli, dir, &p.download)?;

    write_json(&state_path(state_dir), &handled);
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

    #[test]
    fn new_files_go_both_ways() {
        let l = vec![local("mine.jpg", 10), local("both.jpg", 20)];
        let r = vec![remote("u1", "both.jpg", 20), remote("u2", "theirs.jpg", 30)];
        let p = plan(&l, &r, &Handled::default());
        assert_eq!(p.upload, vec!["mine.jpg"]);
        assert_eq!(p.download, vec!["u2"]);
        assert!(p.conflicts.is_empty());
    }

    /// Same name, different bytes: never overwrite, never loop.
    #[test]
    fn size_mismatch_keeps_both_and_reports_once() {
        let l = vec![local("x.jpg", 10)];
        let r = vec![remote("u1", "x.jpg", 99)];
        let mut handled = Handled::default();
        let first = plan(&l, &r, &handled);
        assert_eq!(first.conflicts, vec!["x.jpg"]);
        assert!(first.upload.is_empty() && first.download.is_empty());
        handled.0.insert("x.jpg".into(), (10, 99));
        let again = plan(&l, &r, &handled);
        assert!(again.conflicts.is_empty(), "reported once, not every cycle");
        // A revised local file asks for a fresh look.
        let revised = plan(&[local("x.jpg", 11)], &r, &handled);
        assert_eq!(revised.conflicts, vec!["x.jpg"]);
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
        conf.insert("PROTON_SYNC_PHOTOS".into(), "0".into());
        assert!(!enabled(&conf));
    }
}


