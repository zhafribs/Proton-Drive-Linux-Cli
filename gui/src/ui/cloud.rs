//! Cloud: a browser for the remote side (breadcrumbs, back/forward, sorting,
//! right-click actions, uploads/downloads) - all through the official CLI.
//!
//! Paint is browser-style: a folder appears instantly from this session's
//! cache, else the persisted cloud-cache.json, else the engine's last full
//! walk (remote-snapshot.json); a fresh listing always loads behind it, so
//! nothing ever waits on the network to be visible.

use super::{widgets, App, Back};
use crate::{cli, exec, notify, portal, theme};
use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

/// Cache key of the flat trash listing (a NUL can't appear in a rel path).
pub const TRASH_KEY: &str = "\u{0}trash";
const CLOUD_CACHE_MAX: usize = 1000;
/// How long a listing is trusted before it is re-fetched in the background.
/// Within this window, navigating never spawns a CLI process (and never waits
/// on the sync engine's lock), which is what made folder-to-folder browsing
/// feel slow.
const LIST_TTL: f64 = 30.0;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct DiskEntry {
    pub ts: f64,
    pub entries: Vec<cli::RemoteEntry>,
}

/// The "Move to..." folder picker (lazy, like the PyQt tree dialog).
#[derive(Default)]
pub struct Picker {
    /// parent rel -> folders (name, rel); "" is the root.
    pub children: HashMap<String, Vec<(String, String)>>,
    pub expanded: HashSet<String>,
    pub sel: Option<String>,
    /// expansion click waiting for the fetch worker
    pub expand_req: Option<String>,
}

#[derive(Default)]
pub struct CloudState {
    pub rel: String,
    pub hist: Vec<String>,
    pub fwd: Vec<String>,
    pub trash_view: bool,
    pub entries: Vec<cli::RemoteEntry>,
    pub session: HashMap<String, Vec<cli::RemoteEntry>>,
    pub disk: HashMap<String, DiskEntry>,
    /// epoch seconds of the last successful listing, per cache key
    pub fetched: HashMap<String, f64>,
    /// last time the disk cache was written (debounce)
    pub last_save: f64,
    pub disk_loaded: bool,
    pub opened: bool, // first render triggered the initial listing
    pub sort_col: usize, // 0 name, 1 size, 2 mtime
    pub sort_desc: bool,
    pub busy: bool,
    pub status: String,
    pub status_tip: String,
    pub sel: Vec<String>,       // selected rels, in click order
    pub anchor: Option<String>, // shift-range anchor
    pub serial: u64,            // listing request serial (drops stale results)
    pub picker: Option<Picker>,
    /// sidebar tree: expanded nodes and in-flight on-demand listings
    pub tree_open: HashSet<String>,
    pub tree_loading: HashSet<String>,
    /// the user has explicitly collapsed/expanded the root section
    pub tree_root_touched: bool,
}

pub fn remote_root(conf: &std::collections::BTreeMap<String, String>) -> String {
    match conf.get("PROTON_SYNC_REMOTE").map(String::as_str) {
        Some(r) if !r.is_empty() => r.to_string(),
        _ => "/my-files".to_string(),
    }
}

// ------------------------------------------------------------------ navigation

/// The page was shown (also on first visit): re-list, browser-style.
pub fn on_shown(app: &mut App) {
    app.cloud.opened = true;
    if !app.cloud.disk_loaded {
        app.cloud.disk_loaded = true;
        app.cloud.disk = load_disk_cache(&app.paths.cloud_cache_file);
    }
    if app.cloud.trash_view {
        load_trash(app, true);
    } else {
        let rel = app.cloud.rel.clone();
        load(app, rel, false, true, true);
    }
}

pub fn load(app: &mut App, rel: String, push: bool, announce: bool, paint: bool) {
    load_inner(app, rel, push, announce, paint, false);
}

/// Force a fresh listing, ignoring the cache TTL (Refresh, or after a
/// mutation). Used for both the files view and Trash.
pub fn reload(app: &mut App) {
    if app.cloud.trash_view {
        load_trash(app, false);
    } else {
        let rel = app.cloud.rel.clone();
        load_inner(app, rel, false, true, true, true);
    }
}

fn load_inner(app: &mut App, rel: String, push: bool, announce: bool, paint: bool, force: bool) {
    if push {
        // Navigation: reveal the folder in the sidebar tree. Nothing is
        // forced open afterwards, so every branch can still be collapsed.
        expand_path(app, &rel);
    }
    {
        let c = &mut app.cloud;
        if push && rel != c.rel {
            c.hist.push(std::mem::replace(&mut c.rel, rel.clone()));
            c.fwd.clear();
        } else {
            c.rel = rel.clone();
        }
        c.trash_view = false;
        c.serial += 1;
        if announce {
            c.status = "Opening...".to_string();
            c.status_tip.clear();
        }
    }

    // Paint instantly from whatever we already know: this session's listing,
    // the persisted disk cache, or the engine's last full walk.
    let cached = app
        .cloud
        .session
        .get(&rel)
        .cloned()
        .or_else(|| app.cloud.disk.get(&rel).map(|d| d.entries.clone()))
        .or_else(|| seed_entries(&app.paths.snapshot_file, &rel));
    let have_cached = cached.is_some();
    if paint && let Some(src) = cached {
        fill(app, src);
    }

    // Recently listed? Serve it and skip the CLI entirely. This is the normal
    // path while browsing, and it cannot queue behind the sync engine.
    let fresh = !force
        && have_cached
        && app
            .cloud
            .fetched
            .get(&rel)
            .is_some_and(|t| epoch_now() - *t < LIST_TTL);
    if fresh {
        if announce && (app.cloud.status == "Opening..." || app.cloud.status.is_empty()) {
            app.cloud.status = if app.cloud.entries.is_empty() {
                "This folder is empty.".to_string()
            } else {
                String::new()
            };
        }
        return;
    }

    let root = remote_root(&app.conf);
    let serial = app.cloud.serial;
    let tx = app.tx.clone();
    std::thread::spawn(move || {
        let result = cli::fetch_remote_entries(&root, &rel);
        let _ = tx.send(Back::CloudEntries { key: rel, serial, result });
    });
}

pub fn load_trash(app: &mut App, announce: bool) {
    {
        let c = &mut app.cloud;
        c.trash_view = true;
        c.serial += 1;
        if announce {
            c.status = "Opening...".to_string();
            c.status_tip.clear();
        }
    }
    let src = app
        .cloud
        .session
        .get(TRASH_KEY)
        .cloned()
        .or_else(|| app.cloud.disk.get(TRASH_KEY).map(|d| d.entries.clone()));
    if let Some(src) = src {
        fill(app, src);
    }
    let serial = app.cloud.serial;
    let tx = app.tx.clone();
    std::thread::spawn(move || {
        let result = cli::fetch_trash_entries();
        let _ = tx.send(Back::CloudEntries { key: TRASH_KEY.to_string(), serial, result });
    });
}

pub fn go_back(app: &mut App) {
    let Some(prev) = app.cloud.hist.pop() else { return };
    app.cloud.fwd.push(app.cloud.rel.clone());
    expand_path(app, &prev);
    load(app, prev, false, true, true);
}

pub fn go_fwd(app: &mut App) {
    let Some(next) = app.cloud.fwd.pop() else { return };
    app.cloud.hist.push(app.cloud.rel.clone());
    expand_path(app, &next);
    load(app, next, false, true, true);
}

pub fn trash_toggle(app: &mut App, on: bool) {
    if on {
        if !app.cloud.trash_view {
            app.cloud.hist.push(app.cloud.rel.clone());
            app.cloud.fwd.clear();
        }
        load_trash(app, true);
    } else if app.cloud.trash_view {
        let rel = app.cloud.rel.clone();
        load(app, rel, false, true, true);
    }
}

fn fill(app: &mut App, entries: Vec<cli::RemoteEntry>) {
    app.cloud.entries = entries;
    app.cloud
        .sel
        .retain(|r| app.cloud.entries.iter().any(|e| &e.rel == r));
}

/// The listing result landed (fresh paint behind whatever was shown).
pub fn on_entries(
    app: &mut App,
    key: String,
    serial: u64,
    result: Result<Vec<cli::RemoteEntry>, String>,
) {
    if serial != app.cloud.serial {
        return; // navigated away: never overwrite the visible folder
    }
    match result {
        Err(e) => {
            app.cloud.status = "Couldn't open this folder.".to_string();
            app.cloud.status_tip = e;
        }
        Ok(entries) => {
            let was = app.cloud.status.clone();
            app.cloud.session.insert(key.clone(), entries.clone());
            app.cloud.disk.insert(
                key.clone(),
                DiskEntry { ts: epoch_now(), entries: entries.clone() },
            );
            app.cloud.fetched.insert(key, epoch_now());
            persist_cache(app);
            fill(app, entries);
            // "Opening..." is cleared; a finished action's message stays.
            if was == "Opening..." || was.is_empty() {
                app.cloud.status = if app.cloud.entries.is_empty() {
                    if app.cloud.trash_view {
                        "Trash is empty.".to_string()
                    } else {
                        "This folder is empty.".to_string()
                    }
                } else {
                    String::new()
                };
            }
        }
    }
}

/// A sidebar tree listing arrived: warm the caches only — never touch the
/// visible folder, its status, or its serial.
pub fn on_tree_entries(
    app: &mut App,
    key: String,
    result: Result<Vec<cli::RemoteEntry>, String>,
) {
    app.cloud.tree_loading.remove(&key);
    if let Ok(entries) = result {
        app.cloud.session.insert(key.clone(), entries.clone());
        app.cloud.fetched.insert(key.clone(), epoch_now());
        app.cloud.disk.insert(
            key,
            DiskEntry { ts: epoch_now(), entries: entries.clone() },
        );
        persist_cache(app);
    }
}

// ------------------------------------------------------------------ mutations

fn run_cli(args: &[String], timeout_secs: u64) -> Result<String, String> {
    let Some(cli) = cli::resolve_cli() else {
        return Err("proton-drive CLI not found".to_string());
    };
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let o = exec::run(
        &cli.to_string_lossy(),
        &refs,
        Duration::from_secs(timeout_secs),
    )?;
    if !o.ok() {
        let src = if o.stderr.trim().is_empty() { &o.stdout } else { &o.stderr };
        let s: String = src.trim().chars().take(200).collect();
        return Err(if s.is_empty() { "command failed".to_string() } else { s });
    }
    Ok(o.stdout.trim().chars().take(200).collect())
}

/// Run a CLI mutation on a worker (Qt `_cloud_run`): status "..." while it
/// runs, then done-message + refresh of the current folder.
pub fn mutate(
    app: &mut App,
    args: Vec<String>,
    status: &str,
    done: String,
    notify: Option<(String, String)>,
) {
    app.cloud.busy = true;
    app.cloud.status = status.to_string();
    app.cloud.status_tip.clear();
    let tx = app.tx.clone();
    std::thread::spawn(move || {
        let res = run_cli(&args, 600);
        let msg = match res {
            Ok(_) => Back::CloudMutated { ok: true, err: String::new(), done, notify },
            Err(e) => Back::CloudMutated { ok: false, err: e, done: String::new(), notify: None },
        };
        let _ = tx.send(msg);
    });
}

pub fn on_mutated(
    app: &mut App,
    ok: bool,
    err: String,
    done: String,
    notify: Option<(String, String)>,
) {
    app.cloud.busy = false;
    if !ok {
        app.cloud.status = "That didn't work.".to_string();
        app.cloud.status_tip = err;
        return;
    }
    if !done.is_empty() {
        app.cloud.status = done;
    }
    if let Some((t, b)) = notify {
        notify::send(&t, &b);
    }
    refresh_current(app);
}

/// After an action: drop this folder's listings everywhere (memory, disk) so
/// a just-renamed/deleted row can never be painted again; re-list without
/// painting so the action's message stays on screen.
fn refresh_current(app: &mut App) {
    let key = if app.cloud.trash_view {
        TRASH_KEY.to_string()
    } else {
        app.cloud.rel.clone()
    };
    app.cloud.session.remove(&key);
    app.cloud.disk.remove(&key);
    app.cloud.fetched.remove(&key);
    save_disk_cache(&app.paths.cloud_cache_file, &app.cloud.disk);
    if app.cloud.trash_view {
        app.cloud.serial += 1;
        let serial = app.cloud.serial;
        let tx = app.tx.clone();
        std::thread::spawn(move || {
            let result = cli::fetch_trash_entries();
            let _ = tx.send(Back::CloudEntries { key: TRASH_KEY.to_string(), serial, result });
        });
    } else {
        let rel = app.cloud.rel.clone();
        load_inner(app, rel, false, false, false, true);
    }
}

fn epoch_now() -> f64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

// ------------------------------------------------------------- single actions

pub fn action_rename(app: &mut App, old_rel: String, name: String) {
    let name = name.trim().to_string();
    let old_name = old_rel.rsplit('/').next().unwrap_or(&old_rel).to_string();
    if name.is_empty() || name == old_name || name.contains('/') {
        return;
    }
    let root = remote_root(&app.conf);
    mutate(
        app,
        vec!["fs".into(), "rename".into(), cli::rpath(&root, &old_rel), name.clone()],
        "Renaming...",
        format!("Renamed to {name}."),
        None,
    );
}

pub fn action_new_folder(app: &mut App, name: String) {
    let name = name.trim().to_string();
    if name.is_empty() || name.contains('/') {
        return;
    }
    let root = remote_root(&app.conf);
    mutate(
        app,
        vec![
            "fs".into(),
            "create-folder".into(),
            cli::rpath(&root, &app.cloud.rel),
            name.clone(),
        ],
        "Creating...",
        format!("Folder {name} created."),
        None,
    );
}

pub fn action_move(app: &mut App, target_rel: String) {
    let n = app.cloud.sel.len();
    if n == 0 {
        return;
    }
    let root = remote_root(&app.conf);
    let mut args = vec!["fs".to_string(), "move".to_string()];
    for s in &app.cloud.sel {
        args.push(cli::rpath(&root, s));
    }
    args.push(cli::rpath(&root, &target_rel));
    mutate(app, args, "Moving...", format!("Moved {n} item(s)."), None);
}

pub fn action_trash(app: &mut App) {
    let n = app.cloud.sel.len();
    if n == 0 {
        return;
    }
    let root = remote_root(&app.conf);
    let mut args = vec!["fs".to_string(), "trash".to_string()];
    for s in &app.cloud.sel {
        args.push(cli::rpath(&root, s));
    }
    mutate(
        app,
        args,
        "Moving to trash...",
        "Moved to trash.".to_string(),
        Some(("Moved to trash".into(), "Restore anytime from Trash".into())),
    );
}

pub fn action_restore(app: &mut App) {
    let n = app.cloud.sel.len();
    if n == 0 {
        return;
    }
    let mut args = vec!["fs".to_string(), "restore".to_string()];
    for s in &app.cloud.sel {
        args.push(format!("/trash/{s}"));
    }
    mutate(app, args, "Restoring...", format!("Restored {n} item(s)."), None);
}

pub fn action_delete_permanent(app: &mut App) {
    let n = app.cloud.sel.len();
    if n == 0 {
        return;
    }
    let mut args = vec!["fs".to_string(), "delete".to_string()];
    for s in &app.cloud.sel {
        args.push(format!("/trash/{s}"));
    }
    mutate(
        app,
        args,
        "Deleting...",
        format!("Deleted {n} item(s)."),
        None,
    );
}

pub fn action_empty_trash(app: &mut App) {
    mutate(
        app,
        vec!["fs".to_string(), "empty-trash".to_string()],
        "Emptying trash...",
        "Trash emptied.".to_string(),
        None,
    );
}

/// Duplicate the single selected item into its own folder as "<name> copy".
pub fn action_duplicate(app: &mut App) {
    let Some(rel) = app.cloud.sel.first().cloned() else {
        return;
    };
    let root = remote_root(&app.conf);
    let parent = rel.rsplit_once('/').map(|(p, _)| p.to_string()).unwrap_or_default();
    let name = rel.rsplit('/').next().unwrap_or(&rel).to_string();
    let copy_name = format!("{name} copy");
    mutate(
        app,
        vec![
            "fs".into(),
            "copy".into(),
            "-n".into(),
            copy_name.clone(),
            cli::rpath(&root, &rel),
            cli::rpath(&root, &parent),
        ],
        "Duplicating...",
        format!("Duplicated to {copy_name}."),
        None,
    );
}

// ------------------------------------------------------- uploads / downloads

pub fn pick_upload_files(app: &mut App) {
    let tx = app.tx.clone();
    std::thread::spawn(move || {
        let paths = portal::open_files();
        let picked = (!paths.is_empty()).then_some(super::Picked::UploadFiles(paths));
        let _ = tx.send(Back::CloudPicked(picked));
    });
}

pub fn pick_upload_folder(app: &mut App) {
    let tx = app.tx.clone();
    std::thread::spawn(move || {
        let picked = portal::open_dir("Upload folder").map(super::Picked::UploadFolder);
        let _ = tx.send(Back::CloudPicked(picked));
    });
}

pub fn pick_download(app: &mut App) {
    // Selections with only Proton documents can't be downloaded.
    let plain: Vec<String> = app
        .cloud
        .sel
        .iter()
        .filter(|r| {
            app.cloud
                .entries
                .iter()
                .find(|e| &e.rel == *r)
                .is_some_and(|e| !e.doc)
        })
        .cloned()
        .collect();
    if plain.is_empty() {
        return;
    }
    let root = remote_root(&app.conf);
    if plain.len() == 1 {
        let rel = plain[0].clone();
        let is_dir = app
            .cloud
            .entries
            .iter()
            .find(|e| e.rel == rel)
            .is_some_and(|e| e.dir);
        if !is_dir {
            let name = rel.rsplit('/').next().unwrap_or(&rel).to_string();
            let rpath = cli::rpath(&root, &rel);
            let tx = app.tx.clone();
            std::thread::spawn(move || {
                let picked = portal::save_file(&name)
                    .map(|dest| super::Picked::DownloadSave { rpath, dest });
                let _ = tx.send(Back::CloudPicked(picked));
            });
            return;
        }
    }
    let rpaths: Vec<String> = plain.iter().map(|r| cli::rpath(&root, r)).collect();
    let n = rpaths.len();
    let tx = app.tx.clone();
    std::thread::spawn(move || {
        let picked = portal::open_dir("Save to folder")
            .map(|dest| super::Picked::DownloadDir { rpaths, dest, n });
        let _ = tx.send(Back::CloudPicked(picked));
    });
}

pub fn on_picked(app: &mut App, picked: Option<super::Picked>) {
    use super::Picked;
    match picked {
        None => {} // cancelled: no state change
        Some(Picked::UploadFiles(paths)) => {
            let n = paths.len();
            let root = remote_root(&app.conf);
            let mut args = vec!["fs".to_string(), "upload".to_string()];
            for p in &paths {
                args.push(p.to_string_lossy().into_owned());
            }
            args.push(cli::rpath(&root, &app.cloud.rel));
            mutate(
                app,
                args,
                &format!("Uploading {n} file(s)..."),
                format!("Uploaded {n} file(s)."),
                Some(("Uploaded".into(), format!("{n} file(s)"))),
            );
        }
        Some(Picked::UploadFolder(dir)) => {
            let name = dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let root = remote_root(&app.conf);
            let body = name.clone();
            mutate(
                app,
                vec![
                    "fs".into(),
                    "upload".into(),
                    dir.to_string_lossy().into_owned(),
                    cli::rpath(&root, &app.cloud.rel),
                ],
                "Uploading folder...",
                format!("Uploaded {name}."),
                Some(("Uploaded".into(), body)),
            );
        }
        Some(Picked::DownloadSave { rpath, dest }) => {
            app.cloud.busy = true;
            app.cloud.status = "Downloading...".to_string();
            let tx = app.tx.clone();
            std::thread::spawn(move || {
                let res = fetch_one(&rpath, &dest);
                let _ = tx.send(Back::CloudDownload {
                    ok: res.is_ok(),
                    err: res.err().unwrap_or_default(),
                    name: dest
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    n: 1,
                });
            });
        }
        Some(Picked::DownloadDir { rpaths, dest, n }) => {
            app.cloud.busy = true;
            app.cloud.status = format!("Downloading {n} item(s)...");
            let tx = app.tx.clone();
            std::thread::spawn(move || {
                let (ok, got, err) = match fetch_all(&rpaths, &dest) {
                    Ok(n) => (true, n, String::new()),
                    Err(e) => (false, 0, e),
                };
                let _ = tx.send(Back::CloudDownload {
                    ok,
                    err,
                    name: String::new(),
                    n: got,
                });
            });
        }
    }
}

pub fn on_downloaded(app: &mut App, ok: bool, err: String, name: String, n: usize) {
    app.cloud.busy = false;
    if !ok {
        app.cloud.status = "Download failed.".to_string();
        app.cloud.status_tip = err;
        return;
    }
    app.cloud.status = "Saved.".to_string();
    if n <= 1 {
        notify::send("Downloaded", &name);
    } else {
        notify::send("Downloaded", &format!("{n} item(s)"));
    }
}

fn make_tmp_dir() -> std::io::Result<PathBuf> {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "proton-cloud-{}-{}",
        std::process::id(),
        N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Move without following symlinks; fall back to copy when tmp and the
/// destination are on different filesystems (Qt used shutil.move).
fn move_or_copy(from: &Path, to: &Path) -> std::io::Result<()> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(_) => {
            std::fs::copy(from, to)?;
            std::fs::remove_file(from)?;
            Ok(())
        }
    }
}

/// `fs download` to a temp dir, then place the item at `dest`
/// (Qt `_cloud_fetch`).
fn fetch_one(rpath: &str, dest: &Path) -> Result<(), String> {
    let tmp = make_tmp_dir().map_err(|e| e.to_string())?;
    run_cli(
        &["fs".into(), "download".into(), rpath.into(), tmp.to_string_lossy().into_owned()],
        600,
    )?;
    let mut produced = tmp.join(dest.file_name().unwrap_or_default());
    if !produced.exists() {
        let files: Vec<PathBuf> = std::fs::read_dir(&tmp)
            .map(|it| {
                it.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.is_file())
                    .collect()
            })
            .unwrap_or_default();
        if files.len() != 1 {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err("downloaded file not found".to_string());
        }
        produced = files[0].clone();
    }
    if let Some(parent) = dest.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let res = move_or_copy(&produced, dest);
    let _ = std::fs::remove_dir_all(&tmp);
    res.map_err(|e| e.to_string())
}

/// `fs download` of several items into a temp dir, then into `dest_dir`
/// (Qt `_cloud_fetch_all`). Returns the item count.
fn fetch_all(rpaths: &[String], dest_dir: &Path) -> Result<usize, String> {
    let tmp = make_tmp_dir().map_err(|e| e.to_string())?;
    let mut args = vec!["fs".to_string(), "download".to_string()];
    args.extend(rpaths.iter().cloned());
    args.push(tmp.to_string_lossy().into_owned());
    if let Err(e) = run_cli(&args, 3600) {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e);
    }
    let produced: Vec<PathBuf> = std::fs::read_dir(&tmp)
        .map(|it| it.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    if produced.is_empty() {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err("nothing was downloaded".to_string());
    }
    let _ = std::fs::create_dir_all(dest_dir);
    let mut ok = 0;
    for p in &produced {
        let name = p.file_name().unwrap_or_default();
        if move_or_copy(p, &dest_dir.join(name)).is_ok() {
            ok += 1;
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
    Ok(ok)
}

// ------------------------------------------------------------------ the picker

pub fn picker_expand(app: &mut App, rel: String) {
    let Some(picker) = app.cloud.picker.as_mut() else { return };
    if picker.expanded.contains(&rel) {
        picker.expanded.remove(&rel);
        return;
    }
    if picker.children.contains_key(&rel) {
        picker.expanded.insert(rel);
        return;
    }
    picker.expand_req = Some(rel);
    let Some(rel) = picker.expand_req.clone() else { return };
    let root = remote_root(&app.conf);
    let tx = app.tx.clone();
    std::thread::spawn(move || {
        let folders = cli::fetch_remote_folders(&root, &rel);
        let _ = tx.send(Back::PickerFolders { parent: rel, folders });
    });
}

pub fn on_picker_folders(app: &mut App, parent: String, folders: Vec<(String, String)>) {
    if let Some(picker) = app.cloud.picker.as_mut() {
        picker.expanded.insert(parent.clone());
        picker.children.insert(parent, folders);
    }
}

// --------------------------------------------------------- instant-paint seed

static SNAP: Mutex<(Option<SystemTime>, Option<Arc<serde_json::Value>>)> =
    Mutex::new((None, None));

/// The engine's walk as the tree object, parsed once per file change. Returned
/// behind an `Arc` so the (potentially large) tree is not deep-cloned for every
/// folder lookup or sidebar node on every frame.
fn snapshot_tree(path: &Path) -> Arc<serde_json::Value> {
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let mut guard = SNAP.lock().unwrap();
    if guard.0 != mtime {
        let tree = std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| v.get("tree").cloned())
            .filter(|v| v.is_object())
            .map(Arc::new);
        *guard = (mtime, tree);
    }
    guard
        .1
        .clone()
        .unwrap_or_else(|| Arc::new(serde_json::Value::Object(Default::default())))
}

/// A folder listing straight from the engine's walk - instant paint for any
/// folder, even ones never opened in this session (Qt `seed_entries`).
pub fn seed_entries(snapshot: &Path, rel: &str) -> Option<Vec<cli::RemoteEntry>> {
    let tree = snapshot_tree(snapshot);
    let obj = tree.as_object()?;
    let prefix = if rel.is_empty() {
        String::new()
    } else {
        format!("{rel}/")
    };
    let mut out: Vec<cli::RemoteEntry> = Vec::new();
    // A folder that exists in the walk but has no direct children is a known
    // empty folder: return an empty listing (not `None`) so the tree does not
    // re-fetch it just to discover it is empty.
    let known_empty = if rel.is_empty() {
        !obj.is_empty()
    } else {
        obj.contains_key(rel)
    };
    for (r, n) in obj {
        let rest = if rel.is_empty() {
            if r.contains('/') {
                continue;
            }
            r.as_str()
        } else {
            let Some(rest) = r.strip_prefix(&prefix) else { continue };
            if rest.contains('/') || rest.is_empty() {
                continue;
            }
            rest
        };
        let is_dir = n.get("type").and_then(|t| t.as_str()) == Some("dir");
        let mtime: String = n
            .get("mtime")
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .replace('T', " ")
            .chars()
            .take(16)
            .collect();
        out.push(cli::RemoteEntry {
            name: rest.to_string(),
            rel: r.clone(),
            dir: is_dir,
            size: n.get("size").and_then(|s| s.as_u64()).unwrap_or(0),
            mtime,
            doc: n.get("doc").and_then(|d| d.as_bool()).unwrap_or(false),
        });
    }
    if out.is_empty() {
        return known_empty.then_some(out);
    }
    out = sorted(&out, 0, false);
    Some(out)
}

// -------------------------------------------------------------- disk cache

pub fn load_disk_cache(path: &Path) -> HashMap<String, DiskEntry> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<HashMap<String, DiskEntry>>(&s).ok())
        .unwrap_or_default()
}

pub fn save_disk_cache(path: &Path, cache: &HashMap<String, DiskEntry>) {
    let json = if cache.len() > CLOUD_CACHE_MAX {
        let mut pruned = cache.clone();
        let mut keys: Vec<(String, f64)> =
            pruned.iter().map(|(k, v)| (k.clone(), v.ts)).collect();
        keys.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        let drop_n = pruned.len() - CLOUD_CACHE_MAX;
        for (k, _) in keys.iter().take(drop_n) {
            pruned.remove(k);
        }
        serde_json::to_string(&pruned)
    } else {
        serde_json::to_string(cache)
    };
    if let Ok(json) = json {
        let tmp = path.with_file_name(format!(
            "{}.{}.tmp",
            path.file_name().unwrap_or_default().to_string_lossy(),
            std::process::id()
        ));
        let _ = std::fs::write(&tmp, &json);
        let _ = std::fs::rename(&tmp, path);
    }
}

/// Persist the disk cache, debounced so rapid browsing does not re-serialise
/// the whole cache (up to 300 folders) on the UI thread for every listing.
fn persist_cache(app: &mut App) {
    let now = epoch_now();
    if now - app.cloud.last_save < 1.0 {
        return;
    }
    app.cloud.last_save = now;
    save_disk_cache(&app.paths.cloud_cache_file, &app.cloud.disk);
}

// ------------------------------------------------------------------ sorting

/// Qt's `_cloud_fill` stable sort: name as tie-break, then the chosen
/// column, then folders first (so folders stay on top even descending).
pub fn sorted(rows: &[cli::RemoteEntry], col: usize, desc: bool) -> Vec<cli::RemoteEntry> {
    use std::cmp::Ordering;
    let rev = |o: Ordering| if desc { o.reverse() } else { o };
    let lower = |s: &str| s.to_lowercase();
    let mut rows: Vec<cli::RemoteEntry> = rows.to_vec();
    rows.sort_by_key(|a| lower(&a.name));
    match col {
        1 => rows.sort_by(|a, b| rev(a.size.cmp(&b.size))),
        2 => rows.sort_by(|a, b| rev(a.mtime.cmp(&b.mtime))),
        _ => rows.sort_by(|a, b| rev(lower(&a.name).cmp(&lower(&b.name)))),
    }
    rows.sort_by_key(|a| !a.dir); // folders first (stable: keeps column order)
    rows
}

pub fn fmt_size(n: u64) -> String {
    if n == 0 {
        return String::new();
    }
    if n < 1024 {
        return format!("{n} B");
    }
    if n < 1024 * 1024 {
        return format!("{:.0} KB", n as f64 / 1024.0);
    }
    if n < 1024 * 1024 * 1024 {
        return format!("{:.1} MB", n as f64 / (1024.0 * 1024.0));
    }
    format!("{:.1} GB", n as f64 / (1024.0 * 1024.0 * 1024.0))
}

// ------------------------------------------------------------------ drawing

pub fn draw(app: &mut App, ui: &mut egui::Ui) {
    top_row(app, ui);
    ui.add_space(8.0);
    table(app, ui);
    ui.add_space(6.0);
    bottom_row(app, ui);
}

/// A square icon button for the files toolbar: the glyph is painted onto an
/// empty framed button, so hover/press affordances stay native.
fn top_icon_button(ui: &mut egui::Ui, kind: widgets::TopIcon, enabled: bool, hover: &str) -> bool {
    let resp = ui
        .add_enabled(
            enabled,
            egui::Button::new(egui::RichText::new("")).min_size(egui::vec2(30.0, 30.0)),
        )
        .on_hover_text(hover);
    let color = if enabled {
        ui.visuals().text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    widgets::top_icon(ui.painter(), resp.rect, kind, color);
    resp.clicked()
}

/// Collapse the middle of the breadcrumb until the row fits `crumb_w`:
/// a single "..." replaces segments (first and last crumbs are always
/// kept), preferring to drop the ones after the "...". Each remaining
/// crumb keeps the label and full path it arrived with.
///
/// The row must never outgrow `crumb_w` - anything wider here widens the
/// pane for every widget below it (the card would be laid out on that
/// inflated width, so header and rows end up on different bases). The
/// width estimate is rough, hence the 16px of slack.
fn collapse_segments(segs: &mut Vec<(String, String)>, crumb_w: f32) {
    let est = |parts: &[(String, String)]| -> f32 {
        parts
            .iter()
            .map(|(s, _)| widgets::elide(s, 34).len() as f32 * 6.9 + 8.0)
            .sum::<f32>()
            + 12.0
    };
    while segs.len() > 2 && est(segs) > crumb_w - 16.0 {
        match segs.iter().position(|(s, _)| s == "...") {
            Some(p) if p + 1 < segs.len() - 1 => {
                segs.remove(p + 1);
            }
            Some(p) if p > 1 => {
                segs.remove(p - 1);
            }
            Some(_) => break, // down to [first, "...", last]
            None => {
                segs.insert(segs.len() / 2, ("...".to_string(), String::new()));
            }
        }
    }
    // Extreme narrow (minimum window with the widest sidebar): even
    // [first, "...", last] can overshoot. Shave the last label - it keeps
    // its full path - until the estimate fits.
    while est(segs) > crumb_w - 16.0 {
        let Some((label, _)) = segs.last_mut() else { break };
        let has_ellipsis = label.ends_with('\u{2026}');
        let n = label.chars().count();
        if n <= if has_ellipsis { 6 } else { 5 } {
            break;
        }
        // With an ellipsis already there, drop it plus one more character -
        // otherwise take(n - 1) would remove the ellipsis and re-add it,
        // leaving the string unchanged.
        let keep = if has_ellipsis { n - 2 } else { n - 1 };
        let mut s: String = label.chars().take(keep).collect();
        s.push('\u{2026}');
        *label = s;
    }
}

fn top_row(app: &mut App, ui: &mut egui::Ui) {
    let (has_back, has_fwd, busy) = {
        let c = &app.cloud;
        (!c.hist.is_empty(), !c.fwd.is_empty(), c.busy)
    };
    ui.horizontal(|ui| {
        // Icon-only toolbar: Refresh first, then back/forward - all drawn
        // in the nav-icon line style, the words live on hover.
        if top_icon_button(ui, widgets::TopIcon::Refresh, true, "Refresh") {
            reload(app);
        }
        if top_icon_button(ui, widgets::TopIcon::ArrowLeft, has_back && !busy, "Go back") {
            go_back(app);
        }
        if top_icon_button(ui, widgets::TopIcon::ArrowRight, has_fwd && !busy, "Go forward") {
            go_fwd(app);
        }
        ui.add_space(6.0);

        // Breadcrumbs: "My Files > a > b", compact - unframed segments,
        // tight chevrons - collapsing the middle when the row would
        // overflow ("..." stays inert). The crumb area is bounded so it
        // can never stretch into the Refresh button's room.
        let (segs, trash_view) = {
            let c = &app.cloud;
            let mut segs: Vec<String> = vec!["My Files".to_string()];
            if !c.rel.is_empty() {
                segs.extend(c.rel.split('/').map(str::to_string));
            }
            (segs, c.trash_view)
        };
        // The row's full remainder: nothing sits to the right of the crumbs
        // anymore, so long paths get maximum room before the middle
        // collapses to "...".
        let crumb_w = ui.available_width().max(120.0);
        // Each crumb carries its path from the uncollapsed list, so hiding
        // middle segments never breaks where a visible crumb points.
        let mut segs: Vec<(String, String)> = segs
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let prefix = segs[1..=i]
                    .iter()
                    .map(String::as_str)
                    .filter(|x| *x != "...")
                    .collect::<Vec<_>>()
                    .join("/");
                (s.clone(), prefix)
            })
            .collect();
        collapse_segments(&mut segs, crumb_w);
        ui.allocate_ui_with_layout(
            egui::vec2(crumb_w, 26.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if trash_view {
                    ui.label(egui::RichText::new("Trash").strong());
                    return;
                }
                for (i, (seg, prefix)) in segs.iter().enumerate() {
                    if i > 0 {
                        ui.label(egui::RichText::new("\u{203a}").weak().size(12.0));
                    }
                    if seg == "..." {
                        // A plain label keeps the same metrics as the crumbs
                        // around it - a disabled Button sits a few px lower
                        // and reads as a stumble in the row.
                        ui.label(egui::RichText::new("\u{2026}").weak().size(13.5));
                        continue;
                    }
                    let current = i == segs.len() - 1;
                    let label = widgets::elide(seg, 34);
                    if current {
                        // Proton's breadcrumb: the open folder reads as bold
                        // navy; ancestors are quiet, clickable text.
                        ui.label(egui::RichText::new(&label).strong().size(13.5))
                            .on_hover_text(format!("/{prefix}"));
                    } else {
                        let resp = ui
                            .add(
                                egui::Label::new(egui::RichText::new(&label).size(13.5))
                                    .sense(egui::Sense::click()),
                            )
                            .on_hover_text(format!("/{prefix}"));
                        if resp.clicked() {
                            load(app, prefix.clone(), true, true, true);
                        }
                    }
                }
            },
        );

    });
}

const HEAD_H: f32 = 26.0;
const ROW_H: f32 = 34.0;

/// One sortable column header. Rendered with egui's own label layout (not
/// hand-anchored text) so it is vertically centred and stays flush with the
/// rows below. The sort direction is a small caret beside the label - the
/// active column's title is accent-coloured, inactive ones stay muted.
fn header_cell(app: &mut App, ui: &mut egui::Ui, w: f32, label: &str, col: usize, right: bool) {
    let active = app.cloud.sort_col == col;
    let accent = theme::accent(ui.visuals());
    let color = if active {
        accent
    } else {
        ui.visuals().weak_text_color()
    };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, HEAD_H), egui::Sense::click());

    let layout = if right {
        egui::Layout::right_to_left(egui::Align::Center)
    } else {
        egui::Layout::left_to_right(egui::Align::Center)
    };
    {
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(layout));
        child.spacing_mut().item_spacing.x = 4.0;
        if right {
            child.add_space(8.0);
            child.label(
                egui::RichText::new(label.to_uppercase())
                    .size(11.5)
                    .strong()
                    .color(color),
            );
            if active {
                let c = if app.cloud.sort_desc { "\u{25be}" } else { "\u{25b4}" };
                child.label(egui::RichText::new(c).size(10.0).color(accent));
            }
        } else {
            child.add_space(32.0);
            child.label(
                egui::RichText::new(label.to_uppercase())
                    .size(11.5)
                    .strong()
                    .color(color),
            );
            if active {
                let c = if app.cloud.sort_desc { "\u{25be}" } else { "\u{25b4}" };
                child.label(egui::RichText::new(c).size(10.0).color(accent));
            }
        }
    }

    if resp.hovered() {
        ui.painter().hline(
            rect.x_range(),
            rect.bottom() - 1.0,
            ui.visuals().widgets.noninteractive.fg_stroke,
        );
    }
    if resp.clicked() {
        // Same column flips the order, a new column starts ascending.
        if app.cloud.sort_col == col {
            app.cloud.sort_desc = !app.cloud.sort_desc;
        } else {
            app.cloud.sort_col = col;
            app.cloud.sort_desc = false;
        }
    }
}

const COL_SIZE_W: f32 = 84.0;
const COL_MTIME_W: f32 = 124.0;
const NAME_MIN_W: f32 = 132.0;

/// Column widths for the file list, degrading Modified then Size only when
/// the compositor forces the pane below the layout floor.
fn column_layout(full_w: f32, gap: f32) -> (f32, bool, bool) {
    let show_size = full_w >= NAME_MIN_W + gap + COL_SIZE_W;
    let show_mtime =
        show_size && full_w >= NAME_MIN_W + COL_SIZE_W + COL_MTIME_W + 2.0 * gap;
    let mut name_w = full_w;
    if show_size {
        name_w -= gap + COL_SIZE_W;
    }
    if show_mtime {
        name_w -= gap + COL_MTIME_W;
    }
    (name_w.max(NAME_MIN_W), show_size, show_mtime)
}

fn table(app: &mut App, ui: &mut egui::Ui) {
    // The list sits in a white card with a quiet header, like Proton Drive's
    // file list; the action/status row below gets its space reserved so the
    // card can never push it out of the panel.
    egui::Frame::new()
        .fill(theme::surface(ui.visuals()))
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(egui::CornerRadius::same(14))
        .shadow(theme::card_shadow(ui.visuals()))
        .inner_margin(egui::Margin::symmetric(16, 12))
        .show(ui, |ui| {
            let full_w = ui.available_width();
            // The header row is three cells separated by item_spacing; that
            // spacing comes out of the flexible Name column so the row adds
            // up to exactly full_w. A row wider than the card would widen
            // the pane for the rows below - header and rows would end up on
            // different bases.
            let gap = ui.spacing().item_spacing.x;
            let (name_w, show_size, show_mtime) = column_layout(full_w, gap);

            ui.horizontal(|ui| {
                header_cell(app, ui, name_w, "Name", 0, false);
                if show_size {
                    header_cell(app, ui, COL_SIZE_W, "Size", 1, true);
                }
                if show_mtime {
                    header_cell(app, ui, COL_MTIME_W, "Modified", 2, true);
                }
            });
            ui.separator();

            let (trash_view, busy, sort_col, sort_desc) = {
                let c = &app.cloud;
                (c.trash_view, c.busy, c.sort_col, c.sort_desc)
            };
            let rows = sorted(&app.cloud.entries, sort_col, sort_desc);
            // Frame margin (24) + gap (6) + the action row (~36).
            let max_h = (ui.available_height() - 66.0).max(120.0);

            let mut open_rel: Option<String> = None;
            let mut menu_item: Option<&'static str> = None;

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .max_height(max_h)
                .show(ui, |ui| {
                    for e in &rows {
                        row(
                            app,
                            ui,
                            e,
                            name_w,
                            COL_SIZE_W,
                            COL_MTIME_W,
                            show_size,
                            show_mtime,
                            &mut open_rel,
                            &mut menu_item,
                        );
                    }
                    // A folder with nothing in it gets a friendly, centred
                    // empty state instead of a blank card.
                    if rows.is_empty() && !busy {
                        ui.add_space(26.0);
                        ui.vertical_centered(|ui| {
                            // Folder glyph on a soft accent disc - a designed
                            // empty state instead of a lone emoji.
                            let (r, _) = ui.allocate_exact_size(
                                egui::vec2(64.0, 64.0),
                                egui::Sense::hover(),
                            );
                            ui.painter().circle_filled(
                                r.center(),
                                32.0,
                                theme::accent(ui.visuals()).gamma_multiply(0.12),
                            );
                            ui.painter().text(
                                r.center(),
                                egui::Align2::CENTER_CENTER,
                                "\u{1f4c1}",
                                egui::FontId::proportional(28.0),
                                egui::Color32::WHITE,
                            );
                            ui.add_space(10.0);
                            let msg = if trash_view {
                                "Your trash is empty"
                            } else {
                                "This folder is empty"
                            };
                            ui.label(egui::RichText::new(msg).strong().size(15.0));
                            ui.add_space(2.0);
                            if !trash_view {
                                ui.label(
                                    egui::RichText::new("Drop files here or use \u{201c}+ New\u{201d}")
                                        .size(12.5)
                                        .weak(),
                                );
                            }
                        });
                        ui.add_space(18.0);
                    }
                    // Empty area: click clears the selection, right-click offers the
                    // folder-level menu (Qt's empty-item context menu).
                    let h = (ui.available_height()).max(24.0);
                    let (rect, resp) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), h),
                        egui::Sense::click(),
                    );
                    if resp.clicked() && !app.cloud.sel.is_empty() {
                        app.cloud.sel.clear();
                    }
                    resp.context_menu(|ui| {
                        if trash_view {
                            if ui.button("Empty trash").clicked() {
                                ui.close();
                                menu_item = Some("emptytrash");
                            }
                            return;
                        }
                        if ui.button("New folder...").clicked() {
                            ui.close();
                            menu_item = Some("newfolder");
                        }
                        if ui.button("Upload files...").clicked() {
                            ui.close();
                            menu_item = Some("uploadfiles");
                        }
                        if ui.button("Upload folder...").clicked() {
                            ui.close();
                            menu_item = Some("uploadfolder");
                        }
                        ui.separator();
                        if ui.button("Select all").clicked() {
                            ui.close();
                            menu_item = Some("selectall");
                        }
                        if ui.button("Refresh").clicked() {
                            ui.close();
                            menu_item = Some("refresh");
                        }
                    });
                    let _ = rect;
                });

            if let Some(rel) = open_rel {
                open_item(app, &rel);
            }
            apply_menu_item(app, menu_item);

            // Delete key = move to trash (Qt event filter).
            let del = ui.input(|i| i.key_pressed(egui::Key::Delete));
            if del && !trash_view && !busy && !app.cloud.sel.is_empty() {
                app.dialog = Some(super::Dialog::ConfirmTrash { n: app.cloud.sel.len() });
            }
        });
}

/// Shorten `text` with an ellipsis until it fits `max_w` at `font`.
///
/// Unlike the character-count elide used for the tree and breadcrumb, file
/// names must never paint over the Size/Modified columns, so this measures
/// the real galley width and binary-searches the cut point.
fn elide_fit(ui: &egui::Ui, text: &str, font: egui::FontId, max_w: f32) -> String {
    let fits = |s: &str| {
        ui.painter()
            .layout_no_wrap(s.to_owned(), font.clone(), egui::Color32::WHITE)
            .size()
            .x
            <= max_w
    };
    if fits(text) {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0usize, chars.len().saturating_sub(1));
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let mut s: String = chars[..mid].iter().collect();
        s.push('\u{2026}');
        if fits(&s) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let mut s: String = chars[..lo].iter().collect();
    s.push('\u{2026}');
    s
}

/// One file-list row. The column widths, selection state and menu plumbing are
/// all call-site locals; grouping them would scatter the layout math, so the
/// parameter count is the honest shape here.
#[allow(clippy::too_many_arguments)]
fn row(
    app: &mut App,
    ui: &mut egui::Ui,
    e: &cli::RemoteEntry,
    name_w: f32,
    size_w: f32,
    mtime_w: f32,
    show_size: bool,
    show_mtime: bool,
    open_rel: &mut Option<String>,
    menu_item: &mut Option<&'static str>,
) {
    let height = ROW_H;
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::click_and_drag(),
    );
    let selected = app.cloud.sel.contains(&e.rel);
    let visuals = ui.visuals();
    if selected {
        ui.painter().rect_filled(rect, 2.0, visuals.selection.bg_fill);
    } else if resp.hovered() {
        ui.painter().rect_filled(
            rect,
            2.0,
            visuals.widgets.hovered.bg_fill.gamma_multiply(0.45),
        );
    }
    let cy = rect.center().y;
    let icon_color = if selected {
        visuals.selection.stroke.color
    } else if e.dir {
        // Folders wear the Proton purple, files stay a quiet neutral - the
        // one thing that makes a file list look "Proton Drive" at a glance.
        theme::accent(ui.visuals())
    } else {
        visuals.weak_text_color()
    };
    let (ix, iy) = (rect.left() + 10.0, cy - 5.0);
    if e.dir {
        // Folder: body + tab.
        ui.painter().rect_filled(
            egui::Rect::from_min_size(egui::pos2(ix, iy + 2.0), egui::vec2(13.0, 9.0)),
            2.0,
            icon_color,
        );
        ui.painter().rect_filled(
            egui::Rect::from_min_size(egui::pos2(ix, iy), egui::vec2(6.0, 4.0)),
            1.0,
            icon_color,
        );
    } else {
        // File: page with a folded corner.
        ui.painter().rect_filled(
            egui::Rect::from_min_size(egui::pos2(ix + 1.0, iy), egui::vec2(11.0, 12.0)),
            2.0,
            icon_color.gamma_multiply(0.75),
        );
    }
    // The three cells render through egui labels at the exact column rects the
    // header uses, so header and values line up with no hand-tuned offsets.
    let name_color = if selected {
        visuals.selection.stroke.color
    } else {
        visuals.text_color()
    };
    let weak = visuals.weak_text_color();
    let gap = ui.spacing().item_spacing.x;

    // Name (starts after the icon, stops before the Size column).
    let name_rect = egui::Rect::from_min_size(
        egui::pos2(rect.left() + 32.0, rect.top()),
        egui::vec2((name_w - 40.0).max(24.0), height),
    );
    let name_text = elide_fit(ui, &e.name, egui::FontId::proportional(14.0), name_rect.width());
    let mut name_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(name_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let name_resp =
        name_ui.label(egui::RichText::new(name_text).size(14.0).color(name_color));
    name_resp.context_menu(|ui| row_menu_body(app, ui, menu_item));

    // Size (right-aligned, files only).
    if show_size && !e.dir {
        let size_rect = egui::Rect::from_min_size(
            egui::pos2(rect.left() + name_w + gap, rect.top()),
            egui::vec2(size_w, height),
        );
        let mut size_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(size_rect)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        size_ui.add_space(8.0);
        let size_resp =
            size_ui.label(egui::RichText::new(fmt_size(e.size)).size(12.5).color(weak));
        size_resp.context_menu(|ui| row_menu_body(app, ui, menu_item));
    }

    // Modified (right-aligned).
    if show_mtime {
        let mtime_rect = egui::Rect::from_min_size(
            egui::pos2(rect.right() - mtime_w, rect.top()),
            egui::vec2(mtime_w, height),
        );
        let mut mtime_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(mtime_rect)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        mtime_ui.add_space(8.0);
        let mtime_resp =
            mtime_ui.label(egui::RichText::new(e.mtime.clone()).size(12.5).color(weak));
        mtime_resp.context_menu(|ui| row_menu_body(app, ui, menu_item));
    }

    if resp.hovered() {
        resp.clone().on_hover_text(e.name.clone());
    }

    // The name/size/date cells are selectable labels, and egui hands the click
    // to the topmost widget - the label - so a click on the text never reached
    // the row. Detect a raw primary click over the row so clicking a name
    // selects the item, while drag-to-select-text still works.
    let pointer_over_row = ui.rect_contains_pointer(rect);
    let raw_click = pointer_over_row && ui.input(|i| i.pointer.primary_clicked());
    let raw_double = pointer_over_row
        && ui.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary));

    // Selection: plain click replaces, ctrl toggles, shift ranges; Qt also
    // moved the current item when right-clicking outside the selection.
    if resp.clicked() || raw_click {
        let mods = ui.input(|i| i.modifiers);
        let sel = &mut app.cloud.sel;
        if mods.ctrl || mods.command {
            if let Some(pos) = sel.iter().position(|r| r == &e.rel) {
                sel.remove(pos);
            } else {
                sel.push(e.rel.clone());
            }
            app.cloud.anchor = Some(e.rel.clone());
        } else if mods.shift {
            if let Some(anchor) = app.cloud.anchor.clone() {
                let a = rows_pos(&app.cloud.entries, &anchor);
                let b = rows_pos(&app.cloud.entries, &e.rel);
                if let (Some(a), Some(b)) = (a, b) {
                    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                    let ordered: Vec<String> = sorted(
                        &app.cloud.entries,
                        app.cloud.sort_col,
                        app.cloud.sort_desc,
                    )
                    .iter()
                    .skip(lo)
                    .take(hi - lo + 1)
                    .map(|x| x.rel.clone())
                    .collect();
                    *sel = ordered;
                }
            } else {
                sel.push(e.rel.clone());
                app.cloud.anchor = Some(e.rel.clone());
            }
        } else {
            sel.clear();
            sel.push(e.rel.clone());
            app.cloud.anchor = Some(e.rel.clone());
        }
    }

    if resp.double_clicked() || raw_double {
        *open_rel = Some(e.rel.clone());
    }

    resp.context_menu(|ui| row_menu_body(app, ui, menu_item));

    // Keep the pointer cursor over rows. egui's selectable labels otherwise
    // flip to an I-beam on hover, which made the list feel like a text box.
    // Drag-to-select still works - this only changes the cursor glyph. It is
    // set after the labels so it wins over the cursor they set while drawing.
    if ui.rect_contains_pointer(rect) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
}

/// The context menu shared by a row and each of its cells, so right-clicking
/// the name (or the size/date text) opens the same menu as the row background.
fn row_menu_body(app: &mut App, ui: &mut egui::Ui, menu_item: &mut Option<&'static str>) {
    if app.cloud.trash_view {
        if ui.button("Restore").clicked() {
            ui.close();
            *menu_item = Some("restore");
        }
        ui.separator();
        if ui.button("Delete permanently").clicked() {
            ui.close();
            *menu_item = Some("delete");
        }
        return;
    }
    let sel = &app.cloud.sel;
    if sel.is_empty() {
        if ui.button("New folder...").clicked() {
            ui.close();
            *menu_item = Some("newfolder");
        }
        if ui.button("Upload files...").clicked() {
            ui.close();
            *menu_item = Some("uploadfiles");
        }
        if ui.button("Upload folder...").clicked() {
            ui.close();
            *menu_item = Some("uploadfolder");
        }
        ui.separator();
        if ui.button("Select all").clicked() {
            ui.close();
            *menu_item = Some("selectall");
        }
        if ui.button("Refresh").clicked() {
            ui.close();
            *menu_item = Some("refresh");
        }
    } else if sel.len() == 1 {
        let entry = app
            .cloud
            .entries
            .iter()
            .find(|x| x.rel == sel[0])
            .cloned();
        if let Some(it) = entry {
            if it.dir && ui.button("Open").clicked() {
                ui.close();
                *menu_item = Some("open");
            } else if !it.dir && !it.doc && ui.button("Download").clicked() {
                ui.close();
                *menu_item = Some("download");
            }
            if ui.button("Rename...").clicked() {
                ui.close();
                *menu_item = Some("rename");
            }
            if ui.button("Duplicate").clicked() {
                ui.close();
                *menu_item = Some("duplicate");
            }
            ui.separator();
            if ui.button("Details").clicked() {
                ui.close();
                *menu_item = Some("details");
            }
        }
        ui.separator();
        if ui.button("Move to...").clicked() {
            ui.close();
            *menu_item = Some("move");
        }
        if ui.button("Move to trash").clicked() {
            ui.close();
            *menu_item = Some("trash");
        }
    } else {
        if ui.button(format!("Download ({} items)", sel.len())).clicked() {
            ui.close();
            *menu_item = Some("download");
        }
        ui.separator();
        if ui.button("Move to...").clicked() {
            ui.close();
            *menu_item = Some("move");
        }
        if ui.button("Move to trash").clicked() {
            ui.close();
            *menu_item = Some("trash");
        }
    }
}

fn rows_pos(rows: &[cli::RemoteEntry], rel: &str) -> Option<usize> {
    rows.iter().position(|e| e.rel == rel)
}

fn open_item(app: &mut App, rel: &str) {
    if app.cloud.trash_view {
        return; // Qt: opening items in Trash does nothing
    }
    let is_dir = app
        .cloud
        .entries
        .iter()
        .find(|e| e.rel == rel)
        .is_some_and(|e| e.dir);
    if is_dir {
        load(app, rel.to_string(), true, true, true);
    } else {
        app.cloud.sel = vec![rel.to_string()];
        pick_download(app);
    }
}

fn apply_menu_item(app: &mut App, item: Option<&'static str>) {
    match item {
        None => {}
        Some("open") => {
            if let Some(rel) = app.cloud.sel.first().cloned() {
                open_item(app, &rel);
            }
        }
        Some("download") => pick_download(app),
        Some("newfolder") => {
            app.input_buf.clear();
            app.dialog = Some(super::Dialog::Input {
                title: "New folder".to_string(),
                label: "Folder name:".to_string(),
                intent: super::InputIntent::CloudNewFolder,
            });
        }
        Some("rename") => {
            if let Some(rel) = app.cloud.sel.first().cloned() {
                let name = rel.rsplit('/').next().unwrap_or(&rel).to_string();
                app.input_buf = name.clone();
                app.dialog = Some(super::Dialog::Input {
                    title: "Rename".to_string(),
                    label: "New name:".to_string(),
                    intent: super::InputIntent::CloudRename(rel),
                });
            }
        }
        Some("move") => {
            app.cloud.picker = Some(Picker::default());
            app.dialog = Some(super::Dialog::CloudMove);
            let rel = String::new();
            picker_expand(app, rel);
        }
        Some("trash") => {
            app.dialog = Some(super::Dialog::ConfirmTrash { n: app.cloud.sel.len() });
        }
        Some("delete") => {
            app.dialog = Some(super::Dialog::ConfirmDelete { n: app.cloud.sel.len() });
        }
        Some("emptytrash") => {
            app.dialog = Some(super::Dialog::ConfirmEmptyTrash);
        }
        Some("duplicate") => action_duplicate(app),
        Some("selectall") => {
            app.cloud.sel = app
                .cloud
                .entries
                .iter()
                .map(|e| e.rel.clone())
                .collect();
        }
        Some("details") => {
            let rel = app.cloud.sel.first().cloned();
            let it = rel.and_then(|r| app.cloud.entries.iter().find(|e| e.rel == r).cloned());
            if let Some(it) = it {
                let kind = if it.dir { "Folder" } else { "File" };
                let size = if it.dir { "-".to_string() } else { fmt_size(it.size) };
                let body = format!(
                    "Name: {}\nType: {}\nSize: {}\nModified: {}\nLocation: /{}",
                    it.name, kind, size, it.mtime, it.rel
                );
                app.dialog = Some(super::Dialog::Info {
                    title: it.name.clone(),
                    body,
                });
            }
        }
        Some("restore") => action_restore(app),
        Some("uploadfiles") => pick_upload_files(app),
        Some("uploadfolder") => pick_upload_folder(app),
        Some("refresh") => reload(app),
        _ => {}
    }
}

fn bottom_row(app: &mut App, ui: &mut egui::Ui) {
    let (status, status_tip, busy, trash_view) = {
        let c = &app.cloud;
        (c.status.clone(), c.status_tip.clone(), c.busy, c.trash_view)
    };
    let sel: Vec<cli::RemoteEntry> = app
        .cloud
        .sel
        .iter()
        .filter_map(|r| app.cloud.entries.iter().find(|e| &e.rel == r))
        .cloned()
        .collect();
    let docs_only = !sel.is_empty() && sel.iter().all(|e| e.doc);
    let dl_enabled = !sel.is_empty() && !docs_only && !trash_view && !busy;

    ui.horizontal(|ui| {
        // Status on the left, never justified: a justified truncate-label
        // takes the whole row and clips the actions (the label's galley is
        // laid to the full available width in a justified layout). Eliding
        // also keeps a long done-message from crowding the buttons.
        let short = widgets::elide(&status, 44);
        let resp = ui
            .add(egui::Label::new(egui::RichText::new(short).size(12.5).weak()).truncate());
        if resp.hovered() {
            let full = if status_tip.is_empty() { status } else { status_tip };
            resp.clone().on_hover_text(full);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // right_to_left: the first item sits at the row's right edge.
            if trash_view {
                // Trash only ever needs clean-up actions - no upload, no new
                // folder, no download.
                if ui
                    .add_enabled(!busy, egui::Button::new("Empty trash"))
                    .on_hover_text("Permanently delete everything in the trash")
                    .clicked()
                {
                    app.dialog = Some(super::Dialog::ConfirmEmptyTrash);
                }
                if !sel.is_empty() {
                    if widgets::danger_ghost_button(ui, "Delete permanently") {
                        app.dialog = Some(super::Dialog::ConfirmDelete { n: sel.len() });
                    }
                    if ui.button("Restore").clicked() {
                        action_restore(app);
                    }
                }
                return;
            }
            if ui
                .add_enabled(!busy, egui::Button::new("\u{2191} Upload"))
                .on_hover_text("Upload files or a folder")
                .clicked()
            {
                pick_upload_files(app);
            }
            if ui
                .add_enabled(!busy, egui::Button::new("New folder"))
                .clicked()
            {
                app.input_buf.clear();
                app.dialog = Some(super::Dialog::Input {
                    title: "New folder".to_string(),
                    label: "Folder name:".to_string(),
                    intent: super::InputIntent::CloudNewFolder,
                });
            }
            if ui
                .add_enabled(dl_enabled, egui::Button::new("\u{2193} Download"))
                .clicked()
            {
                pick_download(app);
            }
        });
    });
}

// ------------------------------------------------------------- sidebar tree
//
// The left pane follows Proton Drive's own sidebar (DonnieDice's layout):
// "+ New", an expandable "My files" tree, and "Trash"; the app's other pages
// live behind the account row at the bottom. The tree fills lazily from the
// session/disk/seed caches and, on demand, with the same read-only listing
// the browser uses; a tree fetch never touches the visible folder.

/// The folders under `rel`, from warm caches only (rendering stays cheap and
/// side-effect free).
fn children(app: &App, rel: &str) -> Option<Vec<cli::RemoteEntry>> {
    app.cloud
        .session
        .get(rel)
        .cloned()
        .or_else(|| app.cloud.disk.get(rel).map(|d| d.entries.clone()))
        .or_else(|| seed_entries(&app.paths.snapshot_file, rel))
}

/// Kick off a background listing for `rel` if we don't have it yet.
fn ensure_children(app: &mut App, rel: &str) {
    if app.cloud.tree_loading.contains(rel) || children(app, rel).is_some() {
        return;
    }
    app.cloud.tree_loading.insert(rel.to_string());
    let root = remote_root(&app.conf);
    let key = rel.to_string();
    let tx = app.tx.clone();
    std::thread::spawn(move || {
        let result = cli::fetch_remote_entries(&root, &key);
        let _ = tx.send(Back::TreeEntries { key, result });
    });
}

fn toggle_tree(app: &mut App, rel: &str) {
    if rel.is_empty() {
        app.cloud.tree_root_touched = true; // remember an explicit root toggle
    }
    if app.cloud.tree_open.contains(rel) {
        app.cloud.tree_open.remove(rel);
    } else {
        app.cloud.tree_open.insert(rel.to_string());
        ensure_children(app, rel);
    }
}

/// Expand the tree down to `rel`: the root section and every ancestor go
/// into `tree_open`, so navigation lands with the path visible. Nothing is
/// forced open later - any branch can still be collapsed by its caret.
fn expand_path(app: &mut App, rel: &str) {
    app.cloud.tree_open.insert(String::new());
    let mut prefix = String::new();
    for part in rel.split('/').filter(|p| !p.is_empty()) {
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(part);
        app.cloud.tree_open.insert(prefix.clone());
        ensure_children(app, &prefix);
    }
}

/// Browser-style navigation from the pane: history push + fresh listing.
fn go(app: &mut App, rel: String) {
    app.page = super::Page::Cloud;
    app.cloud.opened = true;
    load(app, rel, true, true, true);
}

fn go_trash(app: &mut App) {
    app.page = super::Page::Cloud;
    app.cloud.opened = true;
    trash_toggle(app, true);
}

/// The pane's nav list, in the product's left-rail order: Home, Activity,
/// "My files" with its lazy tree, Settings, Trash, About (the "+ New" pill
/// is drawn by the caller above it).
pub fn nav_ui(app: &mut App, ui: &mut egui::Ui) {
    if !app.cloud.disk_loaded {
        app.cloud.disk_loaded = true;
        app.cloud.disk = load_disk_cache(&app.paths.cloud_cache_file);
    }
    if widgets::nav_row(ui, widgets::NavIcon::Home, "Home", app.page == super::Page::Home) {
        app.page = super::Page::Home;
    }
    ui.add_space(6.0);
    if widgets::nav_row(
        ui,
        widgets::NavIcon::Activity,
        "Activity",
        app.page == super::Page::Activity,
    ) {
        app.page = super::Page::Activity;
    }
    ui.add_space(8.0);
    // Proton's sidebar shows "My files" open by default once its folders are
    // known; an explicit toggle wins. Navigation inserts itself into
    // tree_open, so nothing needs to be forced open while browsing.
    let root_open = app.cloud.tree_open.contains("")
        || (!app.cloud.tree_root_touched && children(app, "").is_some());
    my_files_row(app, ui, root_open);
    if root_open {
        tree_rows(app, ui, "", 0);
    }
    ui.add_space(8.0);
    if widgets::nav_row(
        ui,
        widgets::NavIcon::Settings,
        "Settings",
        app.page == super::Page::Settings,
    ) {
        app.page = super::Page::Settings;
    }
    ui.add_space(6.0);
    let trash_active = app.page == super::Page::Cloud && app.cloud.trash_view;
    if widgets::nav_row(ui, widgets::NavIcon::Trash, "Trash", trash_active) && !app.cloud.busy {
        go_trash(app);
    }
    ui.add_space(6.0);
    if widgets::nav_row(ui, widgets::NavIcon::About, "About", app.page == super::Page::About) {
        app.page = super::Page::About;
    }
}

/// "My files": click the row to go to the root, click the right-hand caret
/// to open the folder tree — DonnieDice/Proton's section row.
fn my_files_row(app: &mut App, ui: &mut egui::Ui, root_open: bool) {
    let active = app.page == super::Page::Cloud && !app.cloud.trash_view;
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 36.0),
        egui::Sense::click(),
    );
    let fill = if active {
        theme::accent_soft(ui.visuals())
    } else if resp.hovered() {
        ui.visuals().widgets.hovered.bg_fill
    } else {
        egui::Color32::TRANSPARENT
    };
    if fill != egui::Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, egui::CornerRadius::same(9), fill);
    }
    let color = if active {
        theme::accent(ui.visuals())
    } else {
        ui.visuals().weak_text_color()
    };
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 22.0, rect.center().y),
        egui::vec2(17.0, 17.0),
    );
    widgets::nav_icon(ui.painter(), icon_rect, widgets::NavIcon::Folder, color);
    ui.painter().text(
        rect.left_center() + egui::vec2(42.0, 0.0),
        egui::Align2::LEFT_CENTER,
        "My files",
        egui::FontId::proportional(14.5),
        color,
    );
    let open = root_open;
    let caret = if open { "\u{25be}" } else { "\u{25b8}" };
    ui.painter().text(
        rect.right_center() + egui::vec2(-16.0, 0.0),
        egui::Align2::RIGHT_CENTER,
        caret,
        egui::FontId::proportional(11.0),
        color,
    );
    if resp.clicked()
        && let Some(pos) = resp.interact_pointer_pos() {
            if pos.x >= rect.right() - 26.0 {
                toggle_tree(app, "");
            } else {
                go(app, String::new());
            }
        }
}

/// One level of the folder tree: indented rows with a left caret; the open
/// folder wears the accent pill (Proton highlights where you are).
fn tree_rows(app: &mut App, ui: &mut egui::Ui, parent: &str, depth: usize) {
    let Some(kids) = children(app, parent) else {
        if app.cloud.tree_loading.contains(parent) {
            let indent = 16.0 + depth as f32 * 14.0;
            ui.add_space(indent);
            ui.label(egui::RichText::new("Loading...").size(12.0).weak());
        }
        return;
    };
    let mut dirs: Vec<cli::RemoteEntry> = kids.into_iter().filter(|e| e.dir).collect();
    dirs.sort_by_key(|a| a.name.to_lowercase());
    let in_files = app.page == super::Page::Cloud && !app.cloud.trash_view;
    for e in &dirs {
        let r = e.rel.trim_end_matches('/').to_string();
        let open = app.cloud.tree_open.contains(&r);
        let active = in_files && app.cloud.rel == r;
        let indent = 16.0 + depth as f32 * 14.0;
        let (rect, resp) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), 26.0),
            egui::Sense::click(),
        );
        let fill = if active {
            theme::accent_soft(ui.visuals())
        } else if resp.hovered() {
            ui.visuals().widgets.hovered.bg_fill
        } else {
            egui::Color32::TRANSPARENT
        };
        if fill != egui::Color32::TRANSPARENT {
            ui.painter().rect_filled(rect, egui::CornerRadius::same(7), fill);
        }
        let color = if active {
            theme::accent(ui.visuals())
        } else {
            ui.visuals().weak_text_color()
        };
        let caret = if open { "\u{25be}" } else { "\u{25b8}" };
        ui.painter().text(
            rect.left_center() + egui::vec2(indent, 0.0),
            egui::Align2::LEFT_CENTER,
            caret,
            egui::FontId::proportional(10.0),
            color,
        );
        let icon_rect = egui::Rect::from_center_size(
            egui::pos2(rect.left() + indent + 24.0, rect.center().y),
            egui::vec2(14.0, 14.0),
        );
        widgets::nav_icon(ui.painter(), icon_rect, widgets::NavIcon::Folder, color);
        ui.painter().text(
            rect.left_center() + egui::vec2(indent + 36.0, 0.0),
            egui::Align2::LEFT_CENTER,
            widgets::elide(&e.name, 26),
            egui::FontId::proportional(13.5),
            color,
        );
        if resp.clicked()
            && let Some(pos) = resp.interact_pointer_pos() {
                if pos.x < rect.left() + indent + 16.0 {
                    toggle_tree(app, &r);
                } else {
                    go(app, r.clone());
                }
            }
        if open {
            if children(app, &r).is_none() {
                ensure_children(app, &r); // expanded node: fetch its listing once
            }
            tree_rows(app, ui, &r, depth + 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{NAME_MIN_W, collapse_segments, column_layout};

    /// The engine publishes `remote-snapshot.json`; the GUI's instant tree must
    /// read exactly that shape (`type`, `mtime` with a `T`, null folder size).
    #[test]
    fn seed_entries_matches_the_engine_snapshot_format() {
        use super::seed_entries;
        let path = std::env::temp_dir().join(format!(
            "pd-snapshot-test-{}.json",
            std::process::id()
        ));
        let json = r#"{"sig":"/my-files||","fp":[],"ts":0.0,"tree":{
            "A":{"uid":"u1","type":"dir","mtime":"2026-01-02T03:04:05.000Z","size":null,"doc":false},
            "A/b.txt":{"uid":"u2","type":"file","mtime":"2026-01-02T03:04:05.000Z","size":12,"doc":false},
            "A/Empty":{"uid":"u3","type":"dir","mtime":"2026-01-02T03:04:05.000Z","size":null,"doc":false}
        }}"#;
        std::fs::write(&path, json).unwrap();

        let root = seed_entries(&path, "").expect("root listing");
        assert_eq!(root.len(), 1);
        assert_eq!(root[0].name, "A");
        assert!(root[0].dir);

        let a = seed_entries(&path, "A").expect("A listing");
        let rows: Vec<_> = a.iter().map(|e| (e.name.as_str(), e.dir, e.size)).collect();
        assert!(rows.contains(&("b.txt", false, 12)));
        assert!(rows.contains(&("Empty", true, 0)));

        // Present but childless means known-empty, not "unknown": the tree
        // must not fetch it again just to discover that.
        assert_eq!(seed_entries(&path, "A/Empty").map(|v| v.len()), Some(0));
        assert!(seed_entries(&path, "missing").is_none());

        let _ = std::fs::remove_file(&path);
    }

    /// What top_row feeds the collapse: (label, full path to that crumb).
    fn crumbs() -> Vec<(String, String)> {
        [
            "My Files",
            "UUM-Bungalow",
            "Zhafri",
            "Worker Document",
            "E&Z WB Venture (Airconditioner)",
            "Ibrani",
        ]
        .iter()
        .map(|s| (s.to_string(), s.to_string()))
        .collect()
    }

    #[test]
    fn wide_enough_keeps_every_crumb() {
        let mut segs = crumbs();
        collapse_segments(&mut segs, 4000.0);
        assert_eq!(segs.len(), 6);
        assert!(!segs.iter().any(|(s, _)| s == "..."));
    }

    #[test]
    fn narrow_collapses_but_keeps_both_ends() {
        let mut segs = crumbs();
        let first = segs[0].clone();
        let last = segs[segs.len() - 1].clone();
        collapse_segments(&mut segs, 400.0);
        // Exactly one ellipsis, and the ends of the path are untouched.
        assert_eq!(segs.iter().filter(|(s, _)| s == "...").count(), 1);
        assert_eq!(segs.first(), Some(&first));
        assert_eq!(segs.last(), Some(&last));
        // Never below [first, "...", last].
        assert!(segs.len() >= 3);
    }

    #[test]
    fn survivors_keep_their_original_paths() {
        let mut segs: Vec<(String, String)> = (1..=8)
            .map(|i| (format!("segment-{i:02}"), format!("UUM-Bungalow/folder/{i}")))
            .collect();
        let original: Vec<(String, String)> = segs.clone();
        collapse_segments(&mut segs, 300.0);
        assert!(segs.len() < original.len());
        // Every surviving crumb still carries the path it arrived with, so
        // hiding middle segments never breaks navigation.
        for (s, p) in &segs {
            if s == "..." {
                continue;
            }
            assert!(original.contains(&(s.clone(), p.clone())));
        }
    }

    #[test]
    fn narrow_floor_keeps_the_modified_column() {
        // 388 is the content width at the 720px window floor with the sidebar
        // at its maximum; all three columns must still fit.
        let (name_w, show_size, show_mtime) = column_layout(388.0, 6.0);
        assert!(show_size && show_mtime);
        assert!(name_w >= NAME_MIN_W);
        // Below the floor, Modified goes first, then Size.
        let (_, show_size, show_mtime) = column_layout(340.0, 6.0);
        assert!(show_size && !show_mtime);
        let (name_w, show_size, show_mtime) = column_layout(200.0, 6.0);
        assert!(!show_size && !show_mtime);
        assert!(name_w >= NAME_MIN_W);
    }

    #[test]
    fn extreme_narrow_shaves_the_last_crumb_but_keeps_its_path() {
        let mut segs: Vec<(String, String)> = vec![
            ("My Files".into(), String::new()),
            (
                "Muhammad Al-Mu_izz Bin Safie".into(),
                "UUM-Bungalow/Muhammad Al-Mu_izz Bin Safie".into(),
            ),
        ];
        let last_path = segs.last().unwrap().1.clone();
        collapse_segments(&mut segs, 240.0);
        let (label, path) = segs.last().unwrap();
        // The label was shortened with an ellipsis...
        assert!(label.ends_with('\u{2026}'));
        assert!(label.chars().count() < "Muhammad Al-Mu_izz Bin Safie".chars().count());
        // ...but it still points at the full folder.
        assert_eq!(path, &last_path);
    }
}
