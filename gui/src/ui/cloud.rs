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
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

/// Cache key of the flat trash listing (a NUL can't appear in a rel path).
pub const TRASH_KEY: &str = "\u{0}trash";
const CLOUD_CACHE_MAX: usize = 300;

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
    if paint {
        let src = app
            .cloud
            .session
            .get(&rel)
            .cloned()
            .or_else(|| app.cloud.disk.get(&rel).map(|d| d.entries.clone()))
            .or_else(|| seed_entries(&app.paths.snapshot_file, &rel));
        if let Some(src) = src {
            fill(app, src);
        }
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
    load(app, prev, false, true, true);
}

pub fn go_fwd(app: &mut App) {
    let Some(next) = app.cloud.fwd.pop() else { return };
    app.cloud.hist.push(app.cloud.rel.clone());
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
                key,
                DiskEntry { ts: epoch_now(), entries: entries.clone() },
            );
            save_disk_cache(&app.paths.cloud_cache_file, &app.cloud.disk);
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
        app.cloud.disk.insert(
            key,
            DiskEntry { ts: epoch_now(), entries: entries.clone() },
        );
        save_disk_cache(&app.paths.cloud_cache_file, &app.cloud.disk);
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
        load(app, rel, false, false, false);
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

static SNAP: Mutex<(Option<SystemTime>, Option<serde_json::Value>)> =
    Mutex::new((None, None));

/// The engine's walk as the tree object, parsed once per file change.
fn snapshot_tree(path: &Path) -> serde_json::Value {
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let mut guard = SNAP.lock().unwrap();
    if guard.0 != mtime {
        let tree = std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| v.get("tree").cloned())
            .filter(|v| v.is_object());
        *guard = (mtime, tree);
    }
    guard.1.clone().unwrap_or(serde_json::Value::Object(Default::default()))
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
        return None;
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
    let mut cache = cache.clone();
    if cache.len() > CLOUD_CACHE_MAX {
        let mut keys: Vec<(String, f64)> =
            cache.iter().map(|(k, v)| (k.clone(), v.ts)).collect();
        keys.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        let drop_n = cache.len() - CLOUD_CACHE_MAX;
        for (k, _) in keys.iter().take(drop_n) {
            cache.remove(k);
        }
    }
    if let Ok(json) = serde_json::to_string(&cache) {
        let tmp = path.with_file_name(format!(
            "{}.{}.tmp",
            path.file_name().unwrap_or_default().to_string_lossy(),
            std::process::id()
        ));
        let _ = std::fs::write(&tmp, &json);
        let _ = std::fs::rename(&tmp, path);
    }
}

// ------------------------------------------------------------------ sorting

/// Qt's `_cloud_fill` stable sort: name as tie-break, then the chosen
/// column, then folders first (so folders stay on top even descending).
pub fn sorted(rows: &[cli::RemoteEntry], col: usize, desc: bool) -> Vec<cli::RemoteEntry> {
    use std::cmp::Ordering;
    let rev = |o: Ordering| if desc { o.reverse() } else { o };
    let lower = |s: &str| s.to_lowercase();
    let mut rows: Vec<cli::RemoteEntry> = rows.to_vec();
    rows.sort_by(|a, b| lower(&a.name).cmp(&lower(&b.name)));
    match col {
        1 => rows.sort_by(|a, b| rev(a.size.cmp(&b.size))),
        2 => rows.sort_by(|a, b| rev(a.mtime.cmp(&b.mtime))),
        _ => rows.sort_by(|a, b| rev(lower(&a.name).cmp(&lower(&b.name)))),
    }
    rows.sort_by(|a, b| b.dir.cmp(&a.dir)); // folders first (stable: keeps column order)
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

fn top_row(app: &mut App, ui: &mut egui::Ui) {
    let (has_back, has_fwd, busy) = {
        let c = &app.cloud;
        (!c.hist.is_empty(), !c.fwd.is_empty(), c.busy)
    };
    ui.horizontal(|ui| {
        if ui
            .add_enabled(has_back && !busy, egui::Button::new("\u{2190}").min_size(egui::vec2(44.0, 0.0)))
            .on_hover_text("Go back")
            .clicked()
        {
            go_back(app);
        }
        if ui
            .add_enabled(has_fwd && !busy, egui::Button::new("\u{2192}").min_size(egui::vec2(44.0, 0.0)))
            .on_hover_text("Go forward")
            .clicked()
        {
            go_fwd(app);
        }

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
        // Room for Refresh + a gap, reserved up front.
        let crumb_w = (ui.available_width() - 86.0).max(120.0);
        let est = |parts: &[String]| -> f32 {
            parts
                .iter()
                .map(|s| widgets::elide(s, 34).len() as f32 * 6.9 + 8.0)
                .sum::<f32>()
                + 12.0
        };
        let mut segs = segs;
        while segs.len() > 2 && est(&segs) > crumb_w && !segs.iter().any(|s| s == "...") {
            segs.insert(segs.len() / 2, "...".to_string());
        }
        ui.allocate_ui_with_layout(
            egui::vec2(crumb_w, 26.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if trash_view {
                    ui.label(egui::RichText::new("Trash").strong());
                    return;
                }
                for (i, seg) in segs.iter().enumerate() {
                    if i > 0 {
                        ui.label(egui::RichText::new("\u{203a}").weak().size(12.0));
                    }
                    if seg == "..." {
                        ui.add_enabled(false, egui::Button::new("...").frame_when_inactive(false));
                        continue;
                    }
                    let parts: Vec<&str> = segs[1..=i]
                        .iter()
                        .map(String::as_str)
                        .filter(|s| *s != "...")
                        .collect();
                    let prefix = parts.join("/");
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
                            load(app, prefix, true, true, true);
                        }
                    }
                }
            },
        );

        // Refresh: placed after the bounded crumb area, so the address can
        // never slide underneath it.
        ui.add_space(6.0);
        if ui.button("Refresh").clicked() {
            let rel = app.cloud.rel.clone();
            load(app, rel, false, true, true);
        }
    });
}

fn header_cell(ui: &mut egui::Ui, label: &str, col: usize, width: f32, app: &mut App) {
    let active = app.cloud.sort_col == col;
    let arrow = if active {
        if app.cloud.sort_desc { " \u{25bc}" } else { " \u{25b2}" }
    } else {
        ""
    };
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(width, 20.0), egui::Sense::click());
    let text = format!("{label}{arrow}");
    let color = if active {
        ui.visuals().strong_text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    ui.painter().text(
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        text,
        egui::FontId::proportional(13.0),
        color,
    );
    if resp.hovered() {
        ui.painter().hline(
            rect.x_range(),
            rect.bottom() - 1.0,
            ui.visuals().widgets.noninteractive.fg_stroke,
        );
    }
    if resp.clicked() {
        // Qt: same column flips the order, new column starts ascending.
        if app.cloud.sort_col == col {
            app.cloud.sort_desc = !app.cloud.sort_desc;
        } else {
            app.cloud.sort_col = col;
            app.cloud.sort_desc = false;
        }
    }
}

fn table(app: &mut App, ui: &mut egui::Ui) {
    // The list sits in a white card with a quiet header, like Proton Drive's
    // file list; the action/status row below gets its space reserved so the
    // card can never push it out of the panel.
    let fill = theme::surface(ui.visuals());
    egui::Frame::group(ui.style())
        .fill(fill)
        .corner_radius(egui::CornerRadius::same(12))
        .inner_margin(egui::Margin::symmetric(14, 10))
        .show(ui, |ui| {
            let col_size_w = 96.0f32;
            let col_mtime_w = 132.0f32;
            let full_w = ui.available_width();
            let name_w = (full_w - col_size_w - col_mtime_w).max(140.0);

            ui.horizontal(|ui| {
                header_cell(ui, "Name", 0, name_w, app);
                header_cell(ui, "Size", 1, col_size_w, app);
                header_cell(ui, "Modified", 2, col_mtime_w, app);
            });
            ui.separator();

            let (trash_view, busy, sort_col, sort_desc) = {
                let c = &app.cloud;
                (c.trash_view, c.busy, c.sort_col, c.sort_desc)
            };
            let rows = sorted(&app.cloud.entries, sort_col, sort_desc);
            // Frame margin (10) + gap (6) + the action row (~36).
            let max_h = (ui.available_height() - 52.0).max(120.0);

            let mut open_rel: Option<String> = None;
            let mut menu_item: Option<&'static str> = None;

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .max_height(max_h)
                .show(ui, |ui| {
                    for e in &rows {
                        row(app, ui, e, name_w, col_size_w, &mut open_rel, &mut menu_item);
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

fn row(
    app: &mut App,
    ui: &mut egui::Ui,
    e: &cli::RemoteEntry,
    name_w: f32,
    size_w: f32,
    open_rel: &mut Option<String>,
    menu_item: &mut Option<&'static str>,
) {
    let height = 28.0;
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
    } else {
        visuals.text_color()
    };
    let (ix, iy) = (rect.left() + 8.0, cy - 5.0);
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
    ui.painter().text(
        egui::pos2(rect.left() + 28.0, cy),
        egui::Align2::LEFT_CENTER,
        widgets::elide(&e.name, 80),
        egui::FontId::proportional(14.0),
        if selected { visuals.selection.stroke.color } else { visuals.text_color() },
    );
    if !e.dir {
        ui.painter().text(
            egui::pos2(rect.left() + name_w + size_w - 8.0, cy),
            egui::Align2::RIGHT_CENTER,
            fmt_size(e.size),
            egui::FontId::proportional(12.5),
            visuals.weak_text_color(),
        );
    }
    ui.painter().text(
        egui::pos2(rect.right() - 8.0, cy),
        egui::Align2::RIGHT_CENTER,
        e.mtime.clone(),
        egui::FontId::proportional(12.5),
        visuals.weak_text_color(),
    );

    if resp.hovered() {
        resp.clone().on_hover_text(e.name.clone());
    }

    // Selection: plain click replaces, ctrl toggles, shift ranges; Qt also
    // moved the current item when right-clicking outside the selection.
    if resp.clicked() {
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

    if resp.double_clicked() {
        *open_rel = Some(e.rel.clone());
    }

    resp.context_menu(|ui| {
        if app.cloud.trash_view {
            if ui.button("Restore").clicked() {
                ui.close();
                *menu_item = Some("restore");
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
    });
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
        Some("restore") => action_restore(app),
        Some("uploadfiles") => pick_upload_files(app),
        Some("uploadfolder") => pick_upload_folder(app),
        Some("refresh") => {
            let rel = app.cloud.rel.clone();
            load(app, rel, false, true, true);
        }
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
            if ui
                .add_enabled(!trash_view && !busy, egui::Button::new("\u{2191} Upload"))
                .on_hover_text("Upload files or a folder")
                .clicked()
            {
                pick_upload_files(app);
            }
            if ui
                .add_enabled(!trash_view && !busy, egui::Button::new("New folder"))
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

/// Is `node` a folder on the path down to `rel`? (auto-expand the open route)
fn under_path(node: &str, rel: &str) -> bool {
    if node.is_empty() {
        return !rel.is_empty();
    }
    rel.strip_prefix(node).is_some_and(|rest| rest.starts_with('/'))
}

/// The pane's "+ New" pill (Proton's sidebar button): a menu for creating
/// and uploading into the files view.
pub fn new_button(app: &mut App, ui: &mut egui::Ui) {
    let resp = ui.add_sized(
        egui::vec2(ui.available_width(), 32.0),
        egui::Button::new(egui::RichText::new("+ New").strong()),
    );
    let _ = egui::Popup::menu(&resp).show(|ui| {
        if ui.button("New folder...").clicked() {
            ui.close();
            ensure_files_view(app);
            apply_menu_item(app, Some("newfolder"));
        }
        if ui.button("Upload files...").clicked() {
            ui.close();
            ensure_files_view(app);
            apply_menu_item(app, Some("uploadfiles"));
        }
        if ui.button("Upload folder...").clicked() {
            ui.close();
            ensure_files_view(app);
            apply_menu_item(app, Some("uploadfolder"));
        }
    });
}

/// The pane's actions may be used while another page (or the trash) is
/// showing: land in the files view first, like the empty-area menu does.
fn ensure_files_view(app: &mut App) {
    app.page = super::Page::Cloud;
    let was_trash = app.cloud.trash_view;
    app.cloud.trash_view = false;
    if !app.cloud.opened {
        on_shown(app);
    } else if was_trash {
        let rel = app.cloud.rel.clone();
        load(app, rel, false, true, true);
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
    let in_files = app.page == super::Page::Cloud && !app.cloud.trash_view;
    // Proton's sidebar shows "My files" open by default once its folders are
    // known; an explicit toggle wins, and the open route always expands.
    let root_open = app.cloud.tree_open.contains("")
        || (!app.cloud.tree_root_touched && children(app, "").is_some())
        || (in_files && !app.cloud.rel.is_empty());
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
    widgets::nav_icon(&ui.painter(), icon_rect, widgets::NavIcon::Folder, color);
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
    if resp.clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            if pos.x >= rect.right() - 26.0 {
                toggle_tree(app, "");
            } else {
                go(app, String::new());
            }
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
    dirs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    let in_files = app.page == super::Page::Cloud && !app.cloud.trash_view;
    for e in &dirs {
        let r = e.rel.trim_end_matches('/').to_string();
        let forced = in_files && under_path(&r, &app.cloud.rel);
        let open = app.cloud.tree_open.contains(&r) || forced;
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
        widgets::nav_icon(&ui.painter(), icon_rect, widgets::NavIcon::Folder, color);
        ui.painter().text(
            rect.left_center() + egui::vec2(indent + 36.0, 0.0),
            egui::Align2::LEFT_CENTER,
            widgets::elide(&e.name, 26),
            egui::FontId::proportional(13.5),
            color,
        );
        if resp.clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                if pos.x < rect.left() + indent + 16.0 {
                    toggle_tree(app, &r);
                } else {
                    go(app, r.clone());
                }
            }
        }
        if open {
            if forced && children(app, &r).is_none() {
                ensure_children(app, &r); // the auto-open route fetches its data once
            }
            tree_rows(app, ui, &r, depth + 1);
        }
    }
}
