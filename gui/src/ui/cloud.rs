//! Cloud: a browser for the remote side (breadcrumbs, back/forward, sorting,
//! right-click actions, uploads/downloads) - all through the official CLI.
//!
//! Paint is browser-style: a folder appears instantly from this session's
//! cache, else the persisted cloud-cache.json, else the engine's last full
//! walk (remote-snapshot.json); a fresh listing always loads behind it, so
//! nothing ever waits on the network to be visible.

use super::{AppCtx, Back, Dialog, Page, Toast, widgets};
use crate::{cli, health, notify, portal, theme};
use eframe::egui;
use std::collections::{HashMap, HashSet, VecDeque};
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
    /// Client-side filter over the current listing (name substring).
    pub query: String,
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
    /// Last failed tree listing and when it failed, to avoid a spawn per frame.
    pub tree_failed: HashMap<String, f64>,
    /// the user has explicitly collapsed/expanded the root section
    pub tree_root_touched: bool,
}

/// The narrow view of the app the Files browser receives.
pub struct CloudView<'a> {
    pub cloud: &'a mut CloudState,
    pub ctx: &'a AppCtx,
    pub page: &'a mut Page,
    pub status: health::UiStatus,
    pub can_transfer: bool,
    pub dialog: &'a mut Option<Dialog>,
    pub input_buf: &'a mut String,
    pub input_focus: &'a mut bool,
    pub search_focus: &'a mut bool,
    pub toasts: &'a mut VecDeque<Toast>,
    pub login_requested: &'a mut bool,
    pub now: f64,
}

impl CloudView<'_> {
    fn ui_status(&self) -> health::UiStatus {
        self.status
    }

    fn can_transfer(&self) -> bool {
        self.can_transfer
    }

    fn toast_error(&mut self, msg: impl Into<String>) {
        self.push_toast(msg.into(), true);
    }

    fn push_toast(&mut self, msg: String, error: bool) {
        if self.toasts.len() >= 4 {
            let idx = self.toasts.iter().position(|t| !t.error).unwrap_or(0);
            self.toasts.remove(idx);
        }
        self.toasts.push_back(Toast {
            msg,
            at: self.now,
            error,
            shown_at: None,
        });
    }

    fn action_login(&mut self) {
        *self.login_requested = true;
    }
}

pub fn remote_root(conf: &std::collections::BTreeMap<String, String>) -> String {
    match conf.get("PROTON_SYNC_REMOTE").map(String::as_str) {
        Some(r) if !r.is_empty() => r.to_string(),
        _ => "/my-files".to_string(),
    }
}

/// Drop every account/root-scoped listing and selection. Used on sign-out and
/// when the sync roots change, so a new account never sees the old filenames
/// or acts on stale rows.
pub fn reset(cloud: &mut CloudState, paths: &crate::paths::Paths) {
    *cloud = CloudState::default();
    cloud.opened = false;
    let _ = std::fs::remove_file(&paths.cloud_cache_file);
}

// ------------------------------------------------------------------ navigation

/// The page was shown (also on first visit): re-list, browser-style.
pub fn on_shown(app: &mut CloudView) {
    app.cloud.opened = true;
    if !app.cloud.disk_loaded {
        app.cloud.disk_loaded = true;
        app.cloud.disk = load_disk_cache(&app.ctx.paths.cloud_cache_file);
    }
    if app.cloud.trash_view {
        load_trash(app, true);
    } else {
        let rel = app.cloud.rel.clone();
        load(app, rel, false, true, true);
    }
}

pub fn load(app: &mut CloudView, rel: String, push: bool, announce: bool, paint: bool) {
    load_inner(app, rel, push, announce, paint, false);
}

/// Force a fresh listing, ignoring the cache TTL (Refresh, or after a
/// mutation). Used for both the files view and Trash.
pub fn reload(app: &mut CloudView) {
    app.cloud.status = "Opening...".to_string();
    app.cloud.status_tip.clear();
    if app.cloud.trash_view {
        load_trash(app, false);
    } else {
        let rel = app.cloud.rel.clone();
        load_inner(app, rel, false, true, true, true);
    }
}

fn load_inner(app: &mut CloudView, rel: String, push: bool, announce: bool, paint: bool, force: bool) {
    if push {
        // Navigation: reveal the folder in the sidebar tree. Nothing is
        // forced open afterwards, so every branch can still be collapsed.
        expand_path(app, &rel);
    }
    {
        let c = &mut app.cloud;
        // Never paint the previous folder's rows under this folder (or under
        // the Trash header). The cache/listing below replaces them.
        if rel != c.rel || c.trash_view {
            c.entries.clear();
            c.sel.clear();
            c.anchor = None;
            c.query.clear();
        }
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
        .or_else(|| seed_entries(&app.ctx.paths.snapshot_file, &rel));
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
            app.cloud.status = if !app.can_transfer() {
                "Sign in to load your files.".to_string()
            } else if app.cloud.entries.is_empty() {
                "This folder is empty.".to_string()
            } else {
                String::new()
            };
        }
        return;
    }

    if !app.can_transfer() {
        if announce && app.cloud.status == "Opening..." {
            app.cloud.status = if matches!(
                app.ui_status(),
                crate::health::UiStatus::SigningIn
            ) {
                "Signing in…".to_string()
            } else {
                "Sign in to load your files.".to_string()
            };
            app.cloud.status_tip
                .push_str("Cached names stay visible; the remote listing needs a session.");
        }
        return;
    }

    let root = remote_root(&app.ctx.conf);
    let serial = app.cloud.serial;
    let tx = app.ctx.tx.clone();
    std::thread::spawn(move || {
        let result = cli::fetch_remote_entries(&root, &rel);
        let _ = tx.send(Back::CloudEntries { key: rel, serial, result });
    });
}

pub fn load_trash(app: &mut CloudView, announce: bool) {
    {
        let c = &mut app.cloud;
        if !c.trash_view {
            c.entries.clear();
            c.sel.clear();
            c.anchor = None;
            c.query.clear();
        }
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
    if !app.can_transfer() {
        if announce && app.cloud.status == "Opening..." {
            app.cloud.status = if matches!(
                app.ui_status(),
                crate::health::UiStatus::SigningIn
            ) {
                "Signing in…".to_string()
            } else {
                "Sign in to load your trash.".to_string()
            };
            app.cloud.status_tip
                .push_str("Cached names stay visible; the remote listing needs a session.");
        }
        return;
    }
    let serial = app.cloud.serial;
    let tx = app.ctx.tx.clone();
    std::thread::spawn(move || {
        let result = cli::fetch_trash_entries();
        let _ = tx.send(Back::CloudEntries { key: TRASH_KEY.to_string(), serial, result });
    });
}

pub fn go_back(app: &mut CloudView) {
    let Some(prev) = app.cloud.hist.pop() else { return };
    app.cloud.fwd.push(app.cloud.rel.clone());
    expand_path(app, &prev);
    load(app, prev, false, true, true);
}

pub fn go_fwd(app: &mut CloudView) {
    let Some(next) = app.cloud.fwd.pop() else { return };
    app.cloud.hist.push(app.cloud.rel.clone());
    expand_path(app, &next);
    load(app, next, false, true, true);
}

pub fn trash_toggle(app: &mut CloudView, on: bool) {
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

fn fill(app: &mut CloudView, entries: Vec<cli::RemoteEntry>) {
    app.cloud.entries = entries;
    app.cloud
        .sel
        .retain(|r| app.cloud.entries.iter().any(|e| &e.rel == r));
}

/// The listing result landed (fresh paint behind whatever was shown).
pub fn on_entries(
    app: &mut CloudView,
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
    app: &mut CloudView,
    key: String,
    at: f64,
    result: Result<Vec<cli::RemoteEntry>, String>,
) {
    app.cloud.tree_loading.remove(&key);
    match result {
        Ok(entries) => {
            app.cloud.tree_failed.remove(&key);
            // Do not let a tree request started before a newer folder fetch
            // overwrite the newer listing.
            if app.cloud.fetched.get(&key).is_none_or(|t| *t <= at) {
                app.cloud.session.insert(key.clone(), entries.clone());
                app.cloud.fetched.insert(key.clone(), at);
                app.cloud.disk.insert(
                    key,
                    DiskEntry { ts: at, entries: entries.clone() },
                );
                persist_cache(app);
            }
        }
        Err(_) => {
            app.cloud.tree_failed.insert(key, epoch_now());
        }
    }
}

// ------------------------------------------------------------------ mutations

fn run_cli(args: &[String], timeout_secs: u64) -> Result<String, String> {
    cli::run_owned(args, Duration::from_secs(timeout_secs))
}

/// Run a CLI mutation on a worker (Qt `_cloud_run`): status "..." while it
/// runs, then done-message + refresh of the current folder.
pub fn mutate(
    app: &mut CloudView,
    args: Vec<String>,
    status: &str,
    done: String,
    notify: Option<(String, String)>,
) {
    app.cloud.busy = true;
    app.cloud.status = status.to_string();
    app.cloud.status_tip.clear();
    let tx = app.ctx.tx.clone();
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
    app: &mut CloudView,
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
fn refresh_current(app: &mut CloudView) {
    let key = if app.cloud.trash_view {
        TRASH_KEY.to_string()
    } else {
        app.cloud.rel.clone()
    };
    app.cloud.session.remove(&key);
    app.cloud.disk.remove(&key);
    app.cloud.fetched.remove(&key);
    save_disk_cache(&app.ctx.paths.cloud_cache_file, &app.cloud.disk);
    if app.cloud.trash_view {
        app.cloud.serial += 1;
        let serial = app.cloud.serial;
        let tx = app.ctx.tx.clone();
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

/// A relative remote path whose every component is safe to join.
fn safe_rel(rel: &str) -> bool {
    !rel.is_empty() && rel.split('/').all(cli::valid_remote_name)
}

fn selection_safe(app: &CloudView) -> bool {
    app.cloud.sel.iter().all(|s| safe_rel(s))
}

fn refuse_unsafe(app: &mut CloudView) -> bool {
    if selection_safe(app) {
        false
    } else {
        app.toast_error("Refusing an unsafe remote path.");
        true
    }
}

// ------------------------------------------------------------- single actions

pub fn action_rename(app: &mut CloudView, old_rel: String, name: String) {
    let name = name.trim().to_string();
    let old_name = old_rel.rsplit('/').next().unwrap_or(&old_rel).to_string();
    if name.is_empty() || name == old_name || name.contains('/') {
        return;
    }
    if !safe_rel(&old_rel) {
        app.toast_error("Refusing an unsafe remote path.");
        return;
    }
    let root = remote_root(&app.ctx.conf);
    mutate(
        app,
        vec!["fs".into(), "rename".into(), cli::rpath(&root, &old_rel), name.clone()],
        "Renaming...",
        format!("Renamed to {name}."),
        None,
    );
}

pub fn action_new_folder(app: &mut CloudView, name: String) {
    let name = name.trim().to_string();
    if name.is_empty() || name.contains('/') {
        return;
    }
    let root = remote_root(&app.ctx.conf);
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

pub fn action_move(app: &mut CloudView, target_rel: String) {
    let n = app.cloud.sel.len();
    if n == 0 || refuse_unsafe(app) {
        return;
    }
    if !safe_rel(&target_rel) {
        app.toast_error("Refusing an unsafe destination path.");
        return;
    }
    let root = remote_root(&app.ctx.conf);
    let mut args = vec!["fs".to_string(), "move".to_string()];
    for s in &app.cloud.sel {
        args.push(cli::rpath(&root, s));
    }
    args.push(cli::rpath(&root, &target_rel));
    mutate(app, args, "Moving...", format!("Moved {n} item(s)."), None);
}

pub fn action_trash(app: &mut CloudView) {
    let n = app.cloud.sel.len();
    if n == 0 || refuse_unsafe(app) {
        return;
    }
    let root = remote_root(&app.ctx.conf);
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

pub fn action_restore(app: &mut CloudView) {
    let n = app.cloud.sel.len();
    if n == 0 || refuse_unsafe(app) {
        return;
    }
    let mut args = vec!["fs".to_string(), "restore".to_string()];
    for s in &app.cloud.sel {
        args.push(format!("/trash/{s}"));
    }
    mutate(app, args, "Restoring...", format!("Restored {n} item(s)."), None);
}

pub fn action_delete_permanent(app: &mut CloudView) {
    let n = app.cloud.sel.len();
    if n == 0 || refuse_unsafe(app) {
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

pub fn action_empty_trash(app: &mut CloudView) {
    mutate(
        app,
        vec!["fs".to_string(), "empty-trash".to_string()],
        "Emptying trash...",
        "Trash emptied.".to_string(),
        None,
    );
}

/// Duplicate the single selected item into its own folder as "<name> copy".
pub fn action_duplicate(app: &mut CloudView) {
    let Some(rel) = app.cloud.sel.first().cloned() else {
        return;
    };
    let root = remote_root(&app.ctx.conf);
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

pub fn pick_upload_files(app: &mut CloudView) {
    if !app.can_transfer() {
        app.toast_error("Sign in to upload files.");
        return;
    }
    let target = app.cloud.rel.clone();
    let tx = app.ctx.tx.clone();
    std::thread::spawn(move || {
        let picked = match portal::open_files() {
            portal::PickResult::Done(paths) => {
                Some(super::Picked::UploadFiles { paths, target })
            }
            portal::PickResult::Cancelled => None,
            portal::PickResult::Unavailable => {
                let _ = tx.send(Back::CloudPickFailed(
                    "No file dialog is available. Install xdg-desktop-portal, zenity or kdialog."
                        .into(),
                ));
                return;
            }
        };
        let _ = tx.send(Back::CloudPicked(picked));
    });
}

pub fn pick_upload_folder(app: &mut CloudView) {
    if !app.can_transfer() {
        app.toast_error("Sign in to upload folders.");
        return;
    }
    let target = app.cloud.rel.clone();
    let tx = app.ctx.tx.clone();
    std::thread::spawn(move || {
        let picked = match portal::open_dir("Upload folder") {
            portal::PickResult::Done(dir) => {
                Some(super::Picked::UploadFolder { dir, target })
            }
            portal::PickResult::Cancelled => None,
            portal::PickResult::Unavailable => {
                let _ = tx.send(Back::CloudPickFailed(
                    "No folder dialog is available. Install xdg-desktop-portal, zenity or kdialog."
                        .into(),
                ));
                return;
            }
        };
        let _ = tx.send(Back::CloudPicked(picked));
    });
}

pub fn pick_download(app: &mut CloudView) {
    if !app.can_transfer() {
        app.toast_error("Sign in to download files.");
        return;
    }
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
    let root = remote_root(&app.ctx.conf);
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
            let tx = app.ctx.tx.clone();
            std::thread::spawn(move || {
                let picked = match portal::save_file(&name) {
                    portal::PickResult::Done(dest) => {
                        Some(super::Picked::DownloadSave { rpath, dest })
                    }
                    portal::PickResult::Cancelled => None,
                    portal::PickResult::Unavailable => {
                        let _ = tx.send(Back::CloudPickFailed(
                            "No save dialog is available. Install xdg-desktop-portal, zenity or kdialog."
                                .into(),
                        ));
                        return;
                    }
                };
                let _ = tx.send(Back::CloudPicked(picked));
            });
            return;
        }
    }
    let rpaths: Vec<String> = plain.iter().map(|r| cli::rpath(&root, r)).collect();
    let n = rpaths.len();
    let tx = app.ctx.tx.clone();
    std::thread::spawn(move || {
        let picked = match portal::open_dir("Save to folder") {
            portal::PickResult::Done(dest) => {
                Some(super::Picked::DownloadDir { rpaths, dest, n })
            }
            portal::PickResult::Cancelled => None,
            portal::PickResult::Unavailable => {
                let _ = tx.send(Back::CloudPickFailed(
                    "No folder dialog is available. Install xdg-desktop-portal, zenity or kdialog."
                        .into(),
                ));
                return;
            }
        };
        let _ = tx.send(Back::CloudPicked(picked));
    });
}

pub fn on_picked(app: &mut CloudView, picked: Option<super::Picked>) {
    use super::Picked;
    match picked {
        None => {} // cancelled: no state change
        Some(Picked::UploadFiles { paths, target }) => {
            let n = paths.len();
            let root = remote_root(&app.ctx.conf);
            let mut args = vec!["fs".to_string(), "upload".to_string()];
            for p in &paths {
                match p.to_str() {
                    Some(p) => args.push(p.to_string()),
                    None => {
                        app.toast_error(
                            "A selected path contains characters this build cannot pass to the CLI.",
                        );
                        return;
                    }
                }
            }
            args.push(cli::rpath(&root, &target));
            mutate(
                app,
                args,
                &format!("Uploading {n} file(s)..."),
                format!("Uploaded {n} file(s)."),
                Some(("Uploaded".into(), format!("{n} file(s)"))),
            );
        }
        Some(Picked::UploadFolder { dir, target }) => {
            let name = dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let Some(dir) = dir.to_str() else {
                app.toast_error(
                    "The selected folder path contains characters this build cannot pass to the CLI.",
                );
                return;
            };
            let root = remote_root(&app.ctx.conf);
            let body = name.clone();
            mutate(
                app,
                vec![
                    "fs".into(),
                    "upload".into(),
                    dir.to_string(),
                    cli::rpath(&root, &target),
                ],
                "Uploading folder...",
                format!("Uploaded {name}."),
                Some(("Uploaded".into(), body)),
            );
        }
        Some(Picked::DownloadSave { rpath, dest }) => {
            app.cloud.busy = true;
            app.cloud.status = "Downloading...".to_string();
            let tx = app.ctx.tx.clone();
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
            let tx = app.ctx.tx.clone();
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

pub fn on_downloaded(app: &mut CloudView, ok: bool, err: String, name: String, n: usize) {
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

/// A private temp directory for one download. Created with mode 0700 and a
/// unique name, and removed on every exit path (including errors).
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> std::io::Result<Self> {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        for _ in 0..32 {
            let path = std::env::temp_dir().join(format!(
                "proton-cloud-{}-{}-{}",
                std::process::id(),
                nanos,
                N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            match builder.create(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "could not create a private download directory",
        ))
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// A destination that does not overwrite an existing file or folder.
fn unique_dest(dir: &Path, name: &std::ffi::OsStr) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let text = name.to_string_lossy();
    let (stem, ext) = match text.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (text.into_owned(), String::new()),
    };
    for i in 2..1000 {
        let candidate = dir.join(format!("{stem} ({i}){ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{stem} (copy){ext}"))
}

/// Copy a file to `to` through a temp file in the same directory, so a
/// planted symlink is replaced rather than followed and a cross-filesystem
/// source still lands atomically.
fn place_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Ok(meta) = std::fs::symlink_metadata(to)
        && (meta.file_type().is_symlink() || meta.is_dir())
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("destination is not a regular file: {}", to.display()),
        ));
    }
    let parent = to.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let nanos = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = parent.join(format!(
        ".proton-place-{}-{nanos}",
        std::process::id()
    ));
    std::fs::copy(from, &tmp)?;
    if let Err(e) = std::fs::rename(&tmp, to) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    let _ = std::fs::remove_file(from);
    Ok(())
}

fn place_item(from: &Path, to: &Path) -> std::io::Result<()> {
    if from.is_dir() {
        std::fs::create_dir(to)?;
        for child in std::fs::read_dir(from)?.flatten() {
            place_item(&child.path(), &to.join(child.file_name()))?;
        }
        let _ = std::fs::remove_dir(from);
        Ok(())
    } else {
        place_file(from, to)
    }
}

/// `fs download` to a temp dir, then place the item at `dest`
/// (Qt `_cloud_fetch`).
fn fetch_one(rpath: &str, dest: &Path) -> Result<(), String> {
    let tmp = TempDir::new().map_err(|e| e.to_string())?;
    run_cli(
        &["fs".into(), "download".into(), rpath.into(), tmp.path().to_string_lossy().into_owned()],
        600,
    )?;
    let mut produced = tmp.path().join(dest.file_name().unwrap_or_default());
    if !produced.exists() {
        let files: Vec<PathBuf> = std::fs::read_dir(tmp.path())
            .map(|it| it.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        if files.len() != 1 {
            return Err("downloaded file not found".to_string());
        }
        produced = files[0].clone();
    }
    let dest = if dest.exists() {
        unique_dest(dest.parent().unwrap_or(Path::new(".")), dest.file_name().unwrap_or_default())
    } else {
        dest.to_path_buf()
    };
    place_item(&produced, &dest).map_err(|e| e.to_string())
}

/// `fs download` of several items into a temp dir, then into `dest_dir`
/// (Qt `_cloud_fetch_all`). Returns the item count.
fn fetch_all(rpaths: &[String], dest_dir: &Path) -> Result<usize, String> {
    let tmp = TempDir::new().map_err(|e| e.to_string())?;
    let mut args = vec!["fs".to_string(), "download".to_string()];
    args.extend(rpaths.iter().cloned());
    args.push(tmp.path().to_string_lossy().into_owned());
    run_cli(&args, 3600)?;
    let produced: Vec<PathBuf> = std::fs::read_dir(tmp.path())
        .map(|it| it.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    if produced.is_empty() {
        return Err("nothing was downloaded".to_string());
    }
    std::fs::create_dir_all(dest_dir).map_err(|e| e.to_string())?;
    let mut ok = 0;
    let mut failed = 0;
    for p in &produced {
        let name = p.file_name().unwrap_or_default();
        let dest = unique_dest(dest_dir, name);
        match place_item(p, &dest) {
            Ok(()) => ok += 1,
            Err(_) => failed += 1,
        }
    }
    if failed > 0 {
        return Err(format!(
            "{failed} of {} downloaded item(s) could not be saved",
            produced.len()
        ));
    }
    Ok(ok)
}

// ------------------------------------------------------------------ the picker

pub fn picker_expand(app: &mut CloudView, rel: String) {
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
    let root = remote_root(&app.ctx.conf);
    let tx = app.ctx.tx.clone();
    std::thread::spawn(move || {
        let folders = cli::fetch_remote_folders(&root, &rel);
        let _ = tx.send(Back::PickerFolders { parent: rel, folders });
    });
}

pub fn on_picker_folders(app: &mut CloudView, parent: String, folders: Vec<(String, String)>) {
    if let Some(picker) = app.cloud.picker.as_mut() {
        picker.expanded.insert(parent.clone());
        picker.children.insert(parent, folders);
    }
}

// --------------------------------------------------------- instant-paint seed

static SNAP: Mutex<(Option<SystemTime>, Option<Arc<serde_json::Value>>)> =
    Mutex::new((None, None));

type CacheKey = (SystemTime, std::path::PathBuf);
type SnapshotUsage = (u64, u64, f64);

static USAGE: Mutex<(Option<CacheKey>, Option<SnapshotUsage>)> =
    Mutex::new((None, None));

/// Cloud storage used + entry count from the engine's `fs size` fingerprint,
/// cached until the snapshot file changes. `fp` is `[size, descendants]`.
pub fn snapshot_usage(path: &Path) -> Option<(u64, u64, f64)> {
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
    let key = (mtime, path.to_path_buf());
    let mut guard = USAGE.lock().unwrap();
    if guard.0.as_ref() == Some(&key) {
        return guard.1;
    }
    let usage = proton_core::snapshot::read(path).and_then(|snapshot| {
        let size = snapshot.fp.first().copied().filter(|v| *v >= 0)? as u64;
        let entries = snapshot
            .fp
            .get(1)
            .copied()
            .filter(|v| *v >= 0)
            .unwrap_or(0) as u64;
        Some((size, entries, snapshot.ts))
    });
    *guard = (Some(key), usage);
    usage
}

#[derive(Clone, serde::Deserialize)]
struct QuotaFile {
    used: u64,
    total: u64,
    drive_used: u64,
    fetched_at: String,
}

static QUOTA: Mutex<(Option<CacheKey>, Option<QuotaFile>)> = Mutex::new((None, None));

/// Account storage quota written by the engine, cached until the file changes.
pub fn quota_usage(path: &Path) -> Option<(u64, u64, u64, String)> {
    let mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
    let key = (mtime, path.to_path_buf());
    let mut guard = QUOTA.lock().unwrap();
    if guard.0.as_ref() != Some(&key) {
        let quota = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<QuotaFile>(&text).ok());
        *guard = (Some(key), quota);
    }
    guard
        .1
        .as_ref()
        .map(|q| (q.used, q.total, q.drive_used, q.fetched_at.clone()))
}

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
            uid: n
                .get("uid")
                .and_then(|u| u.as_str())
                .unwrap_or("")
                .to_string(),
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
    let mut cache = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<HashMap<String, DiskEntry>>(&s).ok())
        .unwrap_or_default();
    // The cache is a local file; never feed an unsafe path back into an
    // action or a join just because it predates the parser validation.
    for entry in cache.values_mut() {
        entry.entries.retain(|e| {
            cli::valid_remote_name(&e.name)
                && safe_rel(&e.rel)
                && (e.rel == e.name
                    || e.rel.ends_with(&format!("/{}", e.name))
                    || (!e.uid.is_empty() && e.rel == e.uid))
        });
    }
    cache
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
fn persist_cache(app: &mut CloudView) {
    let now = epoch_now();
    if now - app.cloud.last_save < 1.0 {
        return;
    }
    app.cloud.last_save = now;
    save_disk_cache(&app.ctx.paths.cloud_cache_file, &app.cloud.disk);
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

pub fn draw(app: &mut CloudView, ui: &mut egui::Ui) {
    page_head(app, ui);
    ui.add_space(10.0);
    top_row(app, ui);
    ui.add_space(8.0);
    table(app, ui);
    ui.add_space(6.0);
    bottom_row(app, ui);
}

fn page_head(app: &mut CloudView, ui: &mut egui::Ui) {
    let (trash_view, busy, can_transfer) =
        (app.cloud.trash_view, app.cloud.busy, app.can_transfer());
    ui.horizontal(|ui| {
        widgets::page_title(ui, if trash_view { "Trash" } else { "Files" });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if trash_view {
                if can_transfer
                    && ui
                        .add_enabled(!busy, egui::Button::new("Empty trash"))
                        .on_hover_text("Permanently delete everything in the trash")
                        .clicked()
                {
                    *app.dialog = Some(super::Dialog::ConfirmEmptyTrash);
                }
            } else if can_transfer {
                let upload = ui
                    .add_enabled_ui(!busy, |ui| {
                        widgets::primary_button_hint(
                            ui,
                            "  Upload",
                            "Upload files from this computer.",
                        )
                    })
                    .inner;
                if upload {
                    pick_upload_files(app);
                }
                let new_folder = ui
                    .add_enabled_ui(!busy, |ui| ui.button("New folder"))
                    .inner
                    .clicked();
                if new_folder {
                    app.input_buf.clear();
                    *app.input_focus = true;
                    *app.dialog = Some(super::Dialog::Input {
                        title: "New folder".to_string(),
                        label: "Folder name:".to_string(),
                        intent: super::InputIntent::CloudNewFolder,
                    });
                }
            }
        });
    });
    if !can_transfer {
        sign_in_banner(app, ui);
    }
}

/// A signed-out browser is still useful for cached names, but every action
/// would fail against the CLI. Say why and offer the one action that works.
fn sign_in_banner(app: &mut CloudView, ui: &mut egui::Ui) {
    let signing_in = matches!(app.ui_status(), crate::health::UiStatus::SigningIn);
    let message = if app.cloud.trash_view {
        "Sign in to restore or permanently delete items."
    } else {
        "Sign in to upload, download and sync."
    };
    egui::Frame::new()
        .fill(theme::accent_soft(ui.visuals()))
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(message)
                        .size(13.0),
                );
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if signing_in {
                            ui.label(
                                egui::RichText::new("Signing in\u{2026}")
                                    .size(12.5)
                                    .weak(),
                            );
                        } else if widgets::primary_button_hint(
                            ui,
                            "Sign in",
                            "Open the Proton sign-in terminal.",
                        ) {
                            app.action_login();
                        }
                    },
                );
            });
        });
    ui.add_space(8.0);
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
    resp.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Button, enabled, hover)
    });
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

fn top_row(app: &mut CloudView, ui: &mut egui::Ui) {
    let (has_back, has_fwd, busy, trash_view) = {
        let c = &app.cloud;
        (!c.hist.is_empty(), !c.fwd.is_empty(), c.busy, c.trash_view)
    };
    ui.horizontal(|ui| {
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

        let search_w = (ui.available_width() * 0.30).clamp(150.0, 240.0);
        if !trash_view {
            let crumb_w = (ui.available_width() - search_w - 12.0).max(120.0);
            breadcrumb(app, ui, crumb_w);
        }
        ui.add_space(6.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let search = ui.add_sized(
                egui::vec2(search_w, 28.0),
                egui::TextEdit::singleline(&mut app.cloud.query).hint_text("Search files..."),
            );
            if *app.search_focus {
                search.request_focus();
                *app.search_focus = false;
            }
        });
    });
}

/// "My files / a / b" with clickable ancestors and a collapsed middle when
/// the row is too narrow. The full path is always on the hover text.
fn breadcrumb(app: &mut CloudView, ui: &mut egui::Ui, crumb_w: f32) {
    let segs = {
        let c = &app.cloud;
        let mut segs: Vec<String> = vec!["My files".to_string()];
        if !c.rel.is_empty() {
            segs.extend(c.rel.split('/').map(str::to_string));
        }
        segs
    };
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
            for (i, (seg, prefix)) in segs.iter().enumerate() {
                if i > 0 {
                    ui.label(egui::RichText::new("\u{203a}").weak().size(12.0));
                }
                if seg == "..." {
                    ui.label(egui::RichText::new("\u{2026}").weak().size(13.5));
                    continue;
                }
                let current = i == segs.len() - 1;
                let label = widgets::elide(seg, 34);
                if current {
                    ui.label(egui::RichText::new(&label).strong().size(13.5))
                        .on_hover_text(format!("/{prefix}"));
                } else {
                    let resp = ui
                        .add(
                            egui::Label::new(egui::RichText::new(&label).size(13.5))
                                .sense(egui::Sense::click()),
                        )
                        .on_hover_text(format!("/{prefix}"));
                    resp.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::Role::Button,
                            ui.is_enabled(),
                            format!("Open {}", seg),
                        )
                    });
                    if resp.has_focus() {
                        ui.painter().rect_stroke(
                            resp.rect.expand(2.0),
                            egui::CornerRadius::same(4),
                            egui::Stroke::new(1.5, theme::accent(ui.visuals())),
                            egui::StrokeKind::Inside,
                        );
                    }
                    let keyboard = resp.has_focus()
                        && ui.input(|i| {
                            i.key_pressed(egui::Key::Enter)
                                || i.key_pressed(egui::Key::Space)
                        });
                    if resp.clicked() || keyboard {
                        load(app, prefix.clone(), true, true, true);
                    }
                }
            }
        },
    );
}

const HEAD_H: f32 = 26.0;
const ROW_H: f32 = 34.0;

/// One sortable column header. Rendered with egui's own label layout (not
/// hand-anchored text) so it is vertically centred and stays flush with the
/// rows below. The sort direction is a small caret beside the label - the
/// active column's title is accent-coloured, inactive ones stay muted.
fn header_cell(app: &mut CloudView, ui: &mut egui::Ui, w: f32, label: &str, col: usize, right: bool) {
    let active = app.cloud.sort_col == col;
    let accent = theme::accent(ui.visuals());
    let color = if active {
        accent
    } else {
        ui.visuals().weak_text_color()
    };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, HEAD_H), egui::Sense::click());
    let descending = app.cloud.sort_desc;
    resp.widget_info(|| {
        let name = if active {
            format!(
                "Sort by {label}, {}",
                if descending { "descending" } else { "ascending" }
            )
        } else {
            format!("Sort by {label}")
        };
        egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), name)
    });
    if active {
        let direction = if descending {
            egui::accesskit::SortDirection::Descending
        } else {
            egui::accesskit::SortDirection::Ascending
        };
        ui.ctx()
            .accesskit_node_builder(resp.id, |n| n.set_sort_direction(direction));
    }

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
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        ui.painter().hline(
            rect.x_range(),
            rect.bottom() - 1.0,
            ui.visuals().widgets.noninteractive.fg_stroke,
        );
    }
    if resp.has_focus() {
        ui.painter().rect_stroke(
            rect.shrink(2.0),
            egui::CornerRadius::same(6),
            egui::Stroke::new(1.5, accent),
            egui::StrokeKind::Inside,
        );
    }
    let keyboard = resp.has_focus()
        && ui.input(|i| {
            i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Space)
        });
    if resp.clicked() || keyboard {
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

fn table(app: &mut CloudView, ui: &mut egui::Ui) {
    // The list sits in a white card with a quiet header, like Proton Drive's
    // file list; the action/status row below gets its space reserved so the
    // card can never push it out of the panel.
    egui::Frame::new()
        .fill(theme::glass_fill(ui.visuals()))
        .stroke(theme::glass_stroke(ui.visuals()))
        .corner_radius(egui::CornerRadius::same(12))
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

            let (trash_view, sort_col, sort_desc) = {
                let c = &app.cloud;
                (c.trash_view, c.sort_col, c.sort_desc)
            };
            let rows = sorted(&app.cloud.entries, sort_col, sort_desc);
            let query = app.cloud.query.trim().to_lowercase();
            let loading = app.cloud.status == "Opening...";
            let load_failed = app.cloud.entries.is_empty()
                && !loading
                && app.cloud.status.starts_with("Couldn't open");
            // Frame margin (24) + gap (6) + the action row. With a selection
            // the row can wrap to two lines (actions plus the status label),
            // so reserve the taller height instead of clipping it.
            let reserve = if app.cloud.sel.is_empty() { 66.0 } else { 92.0 };
            let max_h = (ui.available_height() - reserve).max(120.0);

            let mut open_rel: Option<String> = None;
            let mut menu_item: Option<&'static str> = None;

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .max_height(max_h)
                .show(ui, |ui| {
                    let filtered: Vec<&cli::RemoteEntry> = rows
                        .iter()
                        .filter(|e| query.is_empty() || e.name.to_lowercase().contains(&query))
                        .collect();
                    for e in filtered.iter().copied() {
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
                    if filtered.is_empty() {
                        let query_active = !query.is_empty();
                        ui.add_space(26.0);
                        ui.vertical_centered(|ui| {
                            if loading {
                                widgets::busy_spinner(ui, app.ctx.prefs.motion_enabled());
                                ui.add_space(8.0);
                                ui.label(egui::RichText::new("Loading files\u{2026}").weak());
                            } else if query_active {
                                ui.label(
                                    egui::RichText::new("No files match")
                                        .strong()
                                        .size(15.0),
                                );
                                ui.add_space(2.0);
                                ui.label(
                                    egui::RichText::new(format!(
                                        "\u{201c}{}\u{201d}",
                                        app.cloud.query.trim()
                                    ))
                                    .size(12.5)
                                    .weak(),
                                );
                                ui.add_space(8.0);
                                if ui.button("Clear search").clicked() {
                                    app.cloud.query.clear();
                                }
                            } else if load_failed && app.can_transfer() {
                                ui.label(
                                    egui::RichText::new("Couldn't open this folder")
                                        .strong()
                                        .size(15.0),
                                );
                                ui.add_space(2.0);
                                ui.label(
                                    egui::RichText::new("Check your connection and try again.")
                                        .size(12.5)
                                        .weak(),
                                );
                                ui.add_space(8.0);
                                if ui.button("Retry").clicked() {
                                    reload(app);
                                }
                            } else if !app.can_transfer() {
                                let (r, _) = ui.allocate_exact_size(
                                    egui::vec2(64.0, 64.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter().circle_filled(
                                    r.center(),
                                    32.0,
                                    theme::accent(ui.visuals()).gamma_multiply(0.10),
                                );
                                widgets::nav_icon(
                                    ui.painter(),
                                    egui::Rect::from_center_size(
                                        r.center(),
                                        egui::vec2(24.0, 24.0),
                                    ),
                                    widgets::NavIcon::Folder,
                                    theme::accent(ui.visuals()),
                                );
                                ui.add_space(10.0);
                                if trash_view {
                                    ui.label(
                                        egui::RichText::new("Sign in to see your trash")
                                            .strong()
                                            .size(15.0),
                                    );
                                    ui.add_space(2.0);
                                    ui.label(
                                        egui::RichText::new(
                                            "Deleted files stay here until you sign in.",
                                        )
                                        .size(12.5)
                                        .weak(),
                                    );
                                } else {
                                    ui.label(
                                        egui::RichText::new("Sign in to see your files")
                                            .strong()
                                            .size(15.0),
                                    );
                                    ui.add_space(2.0);
                                    ui.label(
                                        egui::RichText::new(
                                            "Your files will appear here after you sign in.",
                                        )
                                        .size(12.5)
                                        .weak(),
                                    );
                                }
                                ui.add_space(8.0);
                                let signing_in = matches!(
                                    app.ui_status(),
                                    crate::health::UiStatus::SigningIn
                                );
                                if signing_in {
                                    ui.label(
                                        egui::RichText::new("Signing in…")
                                            .size(12.5)
                                            .weak(),
                                    );
                                } else if widgets::primary_button_hint(ui, "Sign in", "") {
                                    app.action_login();
                                }
                            } else {
                                // A drawn folder glyph on a soft accent disc -
                                // no emoji in the file browser.
                                let (r, _) = ui.allocate_exact_size(
                                    egui::vec2(64.0, 64.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter().circle_filled(
                                    r.center(),
                                    32.0,
                                    theme::accent(ui.visuals()).gamma_multiply(0.10),
                                );
                                widgets::nav_icon(
                                    ui.painter(),
                                    egui::Rect::from_center_size(
                                        r.center(),
                                        egui::vec2(24.0, 24.0),
                                    ),
                                    widgets::NavIcon::Folder,
                                    theme::accent(ui.visuals()),
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
                                        egui::RichText::new(
                                            "Drop files here or use Upload.",
                                        )
                                        .size(12.5)
                                        .weak(),
                                    );
                                }
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
                    let can_transfer = app.can_transfer();
                    resp.context_menu(|ui| {
                        if trash_view {
                            if ui
                                .add_enabled(
                                    can_transfer,
                                    egui::Button::new("Empty trash"),
                                )
                                .clicked()
                            {
                                ui.close();
                                menu_item = Some("emptytrash");
                            }
                            return;
                        }
                        if ui
                            .add_enabled(can_transfer, egui::Button::new("New folder..."))
                            .clicked()
                        {
                            ui.close();
                            menu_item = Some("newfolder");
                        }
                        if ui
                            .add_enabled(can_transfer, egui::Button::new("Upload files..."))
                            .clicked()
                        {
                            ui.close();
                            menu_item = Some("uploadfiles");
                        }
                        if ui
                            .add_enabled(can_transfer, egui::Button::new("Upload folder..."))
                            .clicked()
                        {
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

            if app.dialog.is_none() {
                if let Some(rel) = open_rel {
                    open_item(app, &rel);
                }
                apply_menu_item(app, menu_item);
            }

            // Delete key = move to trash (Qt event filter), but never while a
            // text field owns the keyboard or a dialog is already open.
            let del = ui.input(|i| i.key_pressed(egui::Key::Delete))
                && !ui.ctx().text_edit_focused()
                && app.dialog.is_none();
            if del
                && !trash_view
                && !app.cloud.busy
                && app.can_transfer()
                && !app.cloud.sel.is_empty()
            {
                *app.dialog = Some(super::Dialog::ConfirmTrash { n: app.cloud.sel.len() });
            }

            // F2 renames the one selected item, matching the desktop file
            // managers users already know.
            let f2 = ui.input(|i| i.key_pressed(egui::Key::F2))
                && !ui.ctx().text_edit_focused()
                && app.dialog.is_none()
                && !trash_view
                && !app.cloud.busy
                && app.can_transfer()
                && app.cloud.sel.len() == 1;
            if f2 {
                apply_menu_item(app, Some("rename"));
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
    app: &mut CloudView,
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
    // Qt selects the item under the pointer on a right-click. Without this,
    // a cell context menu was built from whatever was selected before, so
    // Rename/Trash could target a different file (or show the empty-space
    // menu when nothing was selected).
    let pointer_over_row = ui.rect_contains_pointer(rect);
    let secondary_click = resp.secondary_clicked()
        || (pointer_over_row && ui.input(|i| i.pointer.secondary_clicked()));
    if app.dialog.is_none() && secondary_click && !selected {
        app.cloud.sel = vec![e.rel.clone()];
        app.cloud.anchor = Some(e.rel.clone());
    }
    resp.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::ListItem, ui.is_enabled(), e.name.as_str())
    });
    ui.ctx()
        .accesskit_node_builder(resp.id, |n| n.set_selected(selected));
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
    if resp.has_focus() {
        let ring = if selected {
            visuals.selection.stroke.color
        } else {
            theme::accent(ui.visuals())
        };
        ui.painter().rect_stroke(
            rect,
            2.0,
            egui::Stroke::new(1.5, ring),
            egui::StrokeKind::Inside,
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
    let weak = if selected {
        visuals.selection.stroke.color
    } else {
        visuals.weak_text_color()
    };
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
    let raw_click = pointer_over_row && ui.input(|i| i.pointer.primary_clicked());
    let raw_double = pointer_over_row
        && ui.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary));

    // Selection: plain click replaces, ctrl toggles, shift ranges; Qt also
    // moved the current item when right-clicking outside the selection.
    // AccessKit activation is a fake primary click, which clicked_by ignores,
    // so ask for the action explicitly.
    let at_click = app.dialog.is_none()
        && ui.input(|i| {
            i.has_accesskit_action_request(resp.id, egui::accesskit::Action::Click)
        });
    if app.dialog.is_none()
        && (resp.clicked_by(egui::PointerButton::Primary) || raw_click || at_click)
    {
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
                // The table draws sorted(entries) filtered by the search
                // query. The old range used raw-entry indices against the
                // sorted list, so it selected the wrong rows under any sort
                // or search.
                let rows = sorted(&app.cloud.entries, app.cloud.sort_col, app.cloud.sort_desc);
                let query = app.cloud.query.trim().to_lowercase();
                let visible: Vec<&cli::RemoteEntry> = rows
                    .iter()
                    .filter(|r| query.is_empty() || r.name.to_lowercase().contains(&query))
                    .collect();
                let a = visible.iter().position(|r| r.rel == anchor);
                let b = visible.iter().position(|r| r.rel == e.rel);
                if let (Some(a), Some(b)) = (a, b) {
                    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                    *sel = visible[lo..=hi].iter().map(|r| r.rel.clone()).collect();
                } else {
                    *sel = vec![e.rel.clone()];
                    app.cloud.anchor = Some(e.rel.clone());
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

    if app.dialog.is_none() && (resp.double_clicked() || raw_double) {
        *open_rel = Some(e.rel.clone());
    }
    // Keyboard: Enter opens, Space toggles selection, matching the pointer
    // behaviour. Focus is requested on click so the next Tab/Enter lands here.
    if resp.clicked_by(egui::PointerButton::Primary) || raw_click || at_click {
        resp.request_focus();
    }
    if resp.gained_focus() {
        resp.scroll_to_me(Some(egui::Align::Center));
    }
    if app.dialog.is_none()
        && resp.has_focus()
        && ui.input(|i| i.key_pressed(egui::Key::Enter))
    {
        *open_rel = Some(e.rel.clone());
    }
    if app.dialog.is_none()
        && resp.has_focus()
        && ui.input(|i| i.key_pressed(egui::Key::Space))
    {
        if let Some(pos) = app.cloud.sel.iter().position(|r| r == &e.rel) {
            app.cloud.sel.remove(pos);
        } else {
            app.cloud.sel.push(e.rel.clone());
        }
        app.cloud.anchor = Some(e.rel.clone());
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
fn row_menu_body(app: &mut CloudView, ui: &mut egui::Ui, menu_item: &mut Option<&'static str>) {
    let can_transfer = app.can_transfer();
    if app.cloud.trash_view {
        if ui
            .add_enabled(can_transfer, egui::Button::new("Restore"))
            .clicked()
        {
            ui.close();
            *menu_item = Some("restore");
        }
        ui.separator();
        if ui
            .add_enabled(can_transfer, egui::Button::new("Delete permanently"))
            .clicked()
        {
            ui.close();
            *menu_item = Some("delete");
        }
        return;
    }
    let sel = &app.cloud.sel;
    if sel.is_empty() {
        if ui
            .add_enabled(can_transfer, egui::Button::new("New folder..."))
            .clicked()
        {
            ui.close();
            *menu_item = Some("newfolder");
        }
        if ui
            .add_enabled(can_transfer, egui::Button::new("Upload files..."))
            .clicked()
        {
            ui.close();
            *menu_item = Some("uploadfiles");
        }
        if ui
            .add_enabled(can_transfer, egui::Button::new("Upload folder..."))
            .clicked()
        {
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
            } else if !it.dir
                && !it.doc
                && ui
                    .add_enabled(can_transfer, egui::Button::new("Download"))
                    .clicked()
            {
                ui.close();
                *menu_item = Some("download");
            }
            if ui
                .add_enabled(can_transfer, egui::Button::new("Rename..."))
                .clicked()
            {
                ui.close();
                *menu_item = Some("rename");
            }
            if ui
                .add_enabled(can_transfer, egui::Button::new("Duplicate"))
                .clicked()
            {
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
        if ui
            .add_enabled(can_transfer, egui::Button::new("Move to..."))
            .clicked()
        {
            ui.close();
            *menu_item = Some("move");
        }
        if ui
            .add_enabled(can_transfer, egui::Button::new("Move to trash"))
            .clicked()
        {
            ui.close();
            *menu_item = Some("trash");
        }
    } else {
        if ui
            .add_enabled(
                can_transfer,
                egui::Button::new(format!("Download ({} items)", sel.len())),
            )
            .clicked()
        {
            ui.close();
            *menu_item = Some("download");
        }
        ui.separator();
        if ui
            .add_enabled(can_transfer, egui::Button::new("Move to..."))
            .clicked()
        {
            ui.close();
            *menu_item = Some("move");
        }
        if ui
            .add_enabled(can_transfer, egui::Button::new("Move to trash"))
            .clicked()
        {
            ui.close();
            *menu_item = Some("trash");
        }
    }
}

fn open_item(app: &mut CloudView, rel: &str) {
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

fn apply_menu_item(app: &mut CloudView, item: Option<&'static str>) {
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
            *app.input_focus = true;
            *app.dialog = Some(super::Dialog::Input {
                title: "New folder".to_string(),
                label: "Folder name:".to_string(),
                intent: super::InputIntent::CloudNewFolder,
            });
        }
        Some("rename") => {
            if let Some(rel) = app.cloud.sel.first().cloned() {
                let name = rel.rsplit('/').next().unwrap_or(&rel).to_string();
                *app.input_buf = name.clone();
                *app.input_focus = true;
                *app.dialog = Some(super::Dialog::Input {
                    title: "Rename".to_string(),
                    label: "New name:".to_string(),
                    intent: super::InputIntent::CloudRename(rel),
                });
            }
        }
        Some("move") => {
            app.cloud.picker = Some(Picker::default());
            *app.dialog = Some(super::Dialog::CloudMove);
            let rel = String::new();
            picker_expand(app, rel);
        }
        Some("trash") => {
            *app.dialog = Some(super::Dialog::ConfirmTrash { n: app.cloud.sel.len() });
        }
        Some("delete") => {
            *app.dialog = Some(super::Dialog::ConfirmDelete { n: app.cloud.sel.len() });
        }
        Some("emptytrash") => {
            *app.dialog = Some(super::Dialog::ConfirmEmptyTrash);
        }
        Some("duplicate") => action_duplicate(app),
        Some("selectall") => select_all_visible(app),
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
                *app.dialog = Some(super::Dialog::Info {
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

/// Select every row the table currently shows, not the files hidden by a
/// search. Shared by the context menu and Ctrl+A.
pub fn select_all_visible(app: &mut CloudView) {
    let query = app.cloud.query.trim().to_lowercase();
    app.cloud.sel = app
        .cloud
        .entries
        .iter()
        .filter(|e| query.is_empty() || e.name.to_lowercase().contains(&query))
        .map(|e| e.rel.clone())
        .collect();
}

fn bottom_row(app: &mut CloudView, ui: &mut egui::Ui) {
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
    let can_transfer = app.can_transfer();
    let dl_enabled =
        !sel.is_empty() && !docs_only && !trash_view && !busy && can_transfer;
    let query = app.cloud.query.trim().to_lowercase();
    let visible = app
        .cloud
        .entries
        .iter()
        .filter(|e| query.is_empty() || e.name.to_lowercase().contains(&query))
        .count();
    let items = if visible == 1 {
        "1 item".to_string()
    } else {
        format!("{visible} items")
    };

    if sel.is_empty() {
        ui.horizontal(|ui| {
            let short = widgets::elide(&status, 52);
            let resp = ui
                .add(egui::Label::new(egui::RichText::new(short).size(12.5).weak()).truncate());
            if resp.hovered() {
                let full = if status_tip.is_empty() { status } else { status_tip };
                resp.clone().on_hover_text(full);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(&items).size(12.0).weak());
            });
        });
        return;
    }

    let mut menu_item: Option<&'static str> = None;
    ui.horizontal_wrapped(|ui| {
        ui.label(
            egui::RichText::new(format!("{} selected", sel.len()))
                .strong()
                .size(12.5),
        );
        ui.separator();
        if !trash_view {
            // Keyboard route to the row context menu: Tab reaches this
            // button, Enter/Space opens it, and the menu is arrow-navigable.
            ui.menu_button("More", |ui| {
                if sel.len() == 1 {
                    if ui.button("Open").clicked() {
                        ui.close();
                        menu_item = Some("open");
                    }
                    if ui.button("Rename...").clicked() {
                        ui.close();
                        menu_item = Some("rename");
                    }
                    if ui.button("Duplicate").clicked() {
                        ui.close();
                        menu_item = Some("duplicate");
                    }
                    if ui.button("Details").clicked() {
                        ui.close();
                        menu_item = Some("details");
                    }
                    ui.separator();
                }
                if ui.button("Move to...").clicked() {
                    ui.close();
                    menu_item = Some("move");
                }
            })
            .response
            .on_hover_text("More actions (Open, Rename, Duplicate, Details, Move)");
        }
        if trash_view {
            if ui
                .add_enabled(!busy && can_transfer, egui::Button::new("Restore"))
                .clicked()
            {
                action_restore(app);
            }
            let delete = ui
                .add_enabled_ui(!busy && can_transfer, |ui| {
                    widgets::danger_ghost_button(ui, "Delete permanently")
                })
                .inner;
            if delete {
                *app.dialog = Some(super::Dialog::ConfirmDelete { n: sel.len() });
            }
        } else {
            if ui
                .add_enabled(dl_enabled, egui::Button::new("Download"))
                .clicked()
            {
                pick_download(app);
            }
            if ui
                .add_enabled(
                    !busy && can_transfer,
                    egui::Button::new("Move to trash"),
                )
                .on_hover_text("Move the selected items to the Proton trash")
                .clicked()
            {
                *app.dialog = Some(super::Dialog::ConfirmTrash { n: sel.len() });
            }
        }
        if ui
            .button("Clear")
            .on_hover_text("Clear the selection")
            .clicked()
        {
            app.cloud.sel.clear();
        }
        // The status/error line stays visible while a selection owns the
        // bottom row; otherwise a failed action would be invisible until the
        // user clears the selection.
        if !status.is_empty() {
            ui.separator();
            let short = widgets::elide(&status, 32);
            let resp = ui.add(
                egui::Label::new(egui::RichText::new(short).size(12.0).weak()).truncate(),
            );
            if resp.hovered() {
                let full = if status_tip.is_empty() { status } else { status_tip };
                resp.on_hover_text(full);
            }
        }
    });
    apply_menu_item(app, menu_item);
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
fn children(app: &CloudView, rel: &str) -> Option<Vec<cli::RemoteEntry>> {
    app.cloud
        .session
        .get(rel)
        .cloned()
        .or_else(|| app.cloud.disk.get(rel).map(|d| d.entries.clone()))
        .or_else(|| seed_entries(&app.ctx.paths.snapshot_file, rel))
}

/// Kick off a background listing for `rel` if we don't have it yet.
fn ensure_children(app: &mut CloudView, rel: &str) {
    if app.cloud.tree_loading.contains(rel) || children(app, rel).is_some() {
        return;
    }
    if let Some(at) = app.cloud.tree_failed.get(rel) {
        if epoch_now() - *at < 30.0 {
            return;
        }
        app.cloud.tree_failed.remove(rel);
    }
    app.cloud.tree_loading.insert(rel.to_string());
    let root = remote_root(&app.ctx.conf);
    let key = rel.to_string();
    let at = epoch_now();
    let tx = app.ctx.tx.clone();
    std::thread::spawn(move || {
        let result = cli::fetch_remote_entries(&root, &key);
        let _ = tx.send(Back::TreeEntries { key, at, result });
    });
}

fn toggle_tree(app: &mut CloudView, rel: &str) {
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
fn expand_path(app: &mut CloudView, rel: &str) {
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
fn go(app: &mut CloudView, rel: String) {
    *app.page = super::Page::Cloud;
    app.cloud.opened = true;
    load(app, rel, true, true, true);
}

fn go_trash(app: &mut CloudView) {
    *app.page = super::Page::Cloud;
    app.cloud.opened = true;
    trash_toggle(app, true);
}

/// The pane's nav list, in the product's left-rail order: Home, Activity,
/// "My files" with its lazy tree, Settings, Trash, About (the "+ New" pill
/// is drawn by the caller above it).
pub fn nav_ui(app: &mut CloudView, ui: &mut egui::Ui) {
    if !app.cloud.disk_loaded {
        app.cloud.disk_loaded = true;
        app.cloud.disk = load_disk_cache(&app.ctx.paths.cloud_cache_file);
    }
    if widgets::nav_row(ui, widgets::NavIcon::Home, "Home", *app.page == super::Page::Home) {
        *app.page = super::Page::Home;
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
        widgets::NavIcon::Photos,
        "Photos",
        *app.page == super::Page::Photos,
    ) {
        *app.page = super::Page::Photos;
    }
    ui.add_space(8.0);
    if widgets::nav_row(
        ui,
        widgets::NavIcon::Activity,
        "Activity",
        *app.page == super::Page::Activity,
    ) {
        *app.page = super::Page::Activity;
    }
    ui.add_space(8.0);
    if widgets::nav_row(
        ui,
        widgets::NavIcon::Settings,
        "Settings",
        *app.page == super::Page::Settings,
    ) {
        *app.page = super::Page::Settings;
    }
    ui.add_space(6.0);
    let trash_active = *app.page == super::Page::Cloud && app.cloud.trash_view;
    if widgets::nav_row(ui, widgets::NavIcon::Trash, "Trash", trash_active) && !app.cloud.busy {
        go_trash(app);
    }
    ui.add_space(6.0);
    if widgets::nav_row(
        ui,
        widgets::NavIcon::Recovery,
        "Recovery",
        *app.page == super::Page::Recovery,
    ) {
        *app.page = super::Page::Recovery;
    }
    ui.add_space(6.0);
    if widgets::nav_row(ui, widgets::NavIcon::About, "About", *app.page == super::Page::About) {
        *app.page = super::Page::About;
    }
}

/// "My files": click the row to go to the root, click the right-hand caret
/// to open the folder tree — DonnieDice/Proton's section row.
fn my_files_row(app: &mut CloudView, ui: &mut egui::Ui, root_open: bool) {
    let active = *app.page == super::Page::Cloud && !app.cloud.trash_view;
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
    resp.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::TreeItem, ui.is_enabled(), "My files")
    });
    ui.ctx().accesskit_node_builder(resp.id, |n| {
        n.set_level(1);
        n.set_expanded(root_open);
        n.set_selected(active);
    });
    if fill != egui::Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, egui::CornerRadius::same(9), fill);
    }
    if resp.gained_focus() {
        resp.scroll_to_me(Some(egui::Align::Center));
    }
    if resp.has_focus() {
        ui.painter().rect_stroke(
            rect,
            egui::CornerRadius::same(9),
            egui::Stroke::new(1.5, theme::accent(ui.visuals())),
            egui::StrokeKind::Inside,
        );
    }
    let color = if active {
        theme::accent_text(ui.visuals())
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
    let at_click = ui.input(|i| {
        i.has_accesskit_action_request(resp.id, egui::accesskit::Action::Click)
    });
    let expand = ui.input(|i| {
        i.has_accesskit_action_request(resp.id, egui::accesskit::Action::Expand)
    });
    let collapse = ui.input(|i| {
        i.has_accesskit_action_request(resp.id, egui::accesskit::Action::Collapse)
    });
    if resp.clicked_by(egui::PointerButton::Primary) || at_click {
        resp.request_focus();
        if let Some(pos) = resp.interact_pointer_pos() {
            if pos.x >= rect.right() - 26.0 {
                toggle_tree(app, "");
            } else {
                go(app, String::new());
            }
        } else {
            go(app, String::new());
        }
    }
    if resp.has_focus() {
        if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            go(app, String::new());
        }
        if ui.input(|i| i.key_pressed(egui::Key::Space)) {
            toggle_tree(app, "");
        }
        if (ui.input(|i| i.key_pressed(egui::Key::ArrowRight)) || expand) && !root_open {
            toggle_tree(app, "");
        }
        if (ui.input(|i| i.key_pressed(egui::Key::ArrowLeft)) || collapse) && root_open {
            toggle_tree(app, "");
        }
    }
}

/// One level of the folder tree: indented rows with a left caret; the open
/// folder wears the accent pill (Proton highlights where you are).
fn tree_rows(app: &mut CloudView, ui: &mut egui::Ui, parent: &str, depth: usize) {
    let Some(kids) = children(app, parent) else {
        let indent = 16.0 + depth as f32 * 14.0;
        if app.cloud.tree_loading.contains(parent) {
            ui.add_space(indent);
            ui.label(egui::RichText::new("Loading...").size(12.0).weak());
        } else if let Some(at) = app.cloud.tree_failed.get(parent)
            && epoch_now() - *at < 30.0
        {
            ui.add_space(indent);
            ui.label(egui::RichText::new("Couldn't load. Try again.").size(11.5).weak());
        }
        return;
    };
    let mut dirs: Vec<cli::RemoteEntry> = kids.into_iter().filter(|e| e.dir).collect();
    dirs.sort_by_key(|a| a.name.to_lowercase());
    let in_files = *app.page == super::Page::Cloud && !app.cloud.trash_view;
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
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::TreeItem, ui.is_enabled(), e.name.as_str())
        });
        ui.ctx().accesskit_node_builder(resp.id, |n| {
            n.set_level(depth + 2);
            n.set_expanded(open);
            n.set_selected(active);
        });
        if fill != egui::Color32::TRANSPARENT {
            ui.painter().rect_filled(rect, egui::CornerRadius::same(7), fill);
        }
        if resp.has_focus() {
            ui.painter().rect_stroke(
                rect,
                egui::CornerRadius::same(7),
                egui::Stroke::new(1.5, theme::accent(ui.visuals())),
                egui::StrokeKind::Inside,
            );
        }
        let color = if active {
            theme::accent_text(ui.visuals())
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
        // Width-aware elision: the sidebar can be dragged down to 176 px and
        // deep indentation leaves little room, so a fixed character count
        // either clipped the name or wasted space.
        let label_x = rect.left() + indent + 36.0;
        let label_w = (rect.right() - 8.0 - label_x).max(12.0);
        let label = elide_fit(ui, &e.name, egui::FontId::proportional(13.5), label_w);
        let truncated = label != e.name;
        ui.painter().text(
            rect.left_center() + egui::vec2(indent + 36.0, 0.0),
            egui::Align2::LEFT_CENTER,
            &label,
            egui::FontId::proportional(13.5),
            color,
        );
        if truncated {
            let _ = resp.clone().on_hover_text(&e.name);
        }
        let at_click = ui.input(|i| {
            i.has_accesskit_action_request(resp.id, egui::accesskit::Action::Click)
        });
        let expand = ui.input(|i| {
            i.has_accesskit_action_request(resp.id, egui::accesskit::Action::Expand)
        });
        let collapse = ui.input(|i| {
            i.has_accesskit_action_request(resp.id, egui::accesskit::Action::Collapse)
        });
        if resp.clicked_by(egui::PointerButton::Primary) || at_click {
            resp.request_focus();
            if let Some(pos) = resp.interact_pointer_pos() {
                if pos.x < rect.left() + indent + 16.0 {
                    toggle_tree(app, &r);
                } else {
                    go(app, r.clone());
                }
            } else {
                go(app, r.clone());
            }
        }
        if resp.gained_focus() {
            resp.scroll_to_me(Some(egui::Align::Center));
        }
        if resp.has_focus() {
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                go(app, r.clone());
            }
            if ui.input(|i| i.key_pressed(egui::Key::Space)) {
                toggle_tree(app, &r);
            }
            if (ui.input(|i| i.key_pressed(egui::Key::ArrowRight)) || expand) && !open {
                toggle_tree(app, &r);
            }
            if (ui.input(|i| i.key_pressed(egui::Key::ArrowLeft)) || collapse) && open {
                toggle_tree(app, &r);
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
