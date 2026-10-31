//! Basic loop/scheduler for the Rust engine (experimental).
//! Starts with interval polling + inotify wake support similar to Python.

use crate::action::Action;
use crate::cli::Cli;
use crate::execute::execute;
use crate::filter::Filter;
use crate::plan::{plan_cycle, PlanConfig};
use crate::scan::{scan_local, scan_remote};
use crate::state_update::{refresh_state, RefreshOptions};
use anyhow::Result;
use proton_core::{conf, health::Health, state};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn state_dir() -> std::path::PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_STATE_HOME") {
        std::path::PathBuf::from(xdg).join("proton-sync")
    } else {
        dirs::home_dir().unwrap_or_else(|| std::path::Path::new("/").to_path_buf()).join(".local/state/proton-sync")
    }
}

fn cfg_values() -> std::collections::BTreeMap<String, String> {
    let home = dirs::home_dir().unwrap_or_else(|| std::path::Path::new("/").to_path_buf());
    let config_dir = dirs::config_dir().unwrap_or_else(|| home.join(".config"));
    conf::read(&config_dir.join("proton-sync.conf"))
}

pub fn run_loop() -> Result<()> {
    let _lock = acquire_process_lock()?;
    let values = cfg_values();
    let home = dirs::home_dir().unwrap_or_else(|| std::path::Path::new("/").to_path_buf());
    let local_root = values
        .get("PROTON_SYNC_LOCAL")
        .cloned()
        .unwrap_or_else(|| home.join("ProtonDrive").to_string_lossy().into_owned());
    let remote_root = values
        .get("PROTON_SYNC_REMOTE")
        .cloned()
        .unwrap_or_else(|| "/my-files".to_string());
    let subfolders = values.get("PROTON_SYNC_SUBFOLDERS").cloned().unwrap_or_default();
    let exclude = values.get("PROTON_SYNC_EXCLUDE").cloned().unwrap_or_default();
    let propagate_deletes = values
        .get("PROTON_SYNC_DELETES")
        .map(|v| v != "0")
        .unwrap_or(true);
    let interval: u64 = values
        .get("PROTON_SYNC_INTERVAL")
        .and_then(|v| v.parse().ok())
        .unwrap_or(300);

    let cli = Cli::new()?;
    let filter = Filter::new(&subfolders, &exclude);
    let wake = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let wake_event = Arc::new(std::sync::Condvar::new());
    let wake_mutex = Arc::new(Mutex::new(()));

    // inotify
    #[cfg(target_os = "linux")]
    {
        let inotify = values.get("PROTON_SYNC_INOTIFY").map(|v| v != "0").unwrap_or(true);
        if inotify {
            let root = local_root.clone();
            let w = wake.clone();
            let we = wake_event.clone();
            let wm = wake_mutex.clone();
            std::thread::spawn(move || {
                let _ = inotify_watch(&root, w, we, wm);
            });
        }
    }

    // wake file
    let wake_file = state_dir().join("wake");
    let w2 = wake.clone();
    let we2 = wake_event.clone();
    let wm2 = wake_mutex.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(1));
        if wake_file.exists() {
            let _ = fs::remove_file(&wake_file);
            w2.store(true, std::sync::atomic::Ordering::Relaxed);
            let _guard = wm2.lock().unwrap();
            we2.notify_all();
        }
    });

    // Primary: Proton's real event stream with the CLI's own keyring
    // session. If that is unavailable, fall back to the 3 s fingerprint
    // watcher; the interval remains the final fallback.
    let event_watch = crate::events::start(
        cli.clone(),
        remote_root.clone(),
        wake.clone(),
        wake_event.clone(),
        wake_mutex.clone(),
    );
    if event_watch.is_none() {
        crate::push::start(
            cli.clone(),
            remote_root.clone(),
            wake.clone(),
            wake_event.clone(),
            wake_mutex.clone(),
        );
    }

    let mut last_cycle = Instant::now();
    let mut first = true;
    loop {
        // the first cycle runs at once; later ones wait for the interval or a wake
        if !first {
            let deadline = (interval > 0).then(|| last_cycle + Duration::from_secs(interval));
            wait_for_cycle(&wake, &wake_event, &wake_mutex, deadline);
        }
        first = false;
        let scopes = match &event_watch {
            Some(watch) => watch.take(&wake),
            None => {
                wake.store(false, std::sync::atomic::Ordering::Relaxed);
                Default::default()
            }
        };
        let push_ctx = event_watch.as_ref().map(|watch| crate::events::PushContext {
            pending: &scopes,
            volume: &watch.volume,
            root_uid: &watch.root_uid,
        });
        let cycle_start = Instant::now();
        if let Err(err) = run_once_cycle(
            &cli,
            &local_root,
            &remote_root,
            &filter,
            propagate_deletes,
            push_ctx.as_ref(),
        ) {
            let message = format!("{err:#}");
            let status = if proton_core::health::is_login_error(&message) {
                "needs_login"
            } else {
                "error"
            };
            crate::health::write_health(&Health {
                status: status.to_string(),
                error: Some(message),
                cycle_seconds: Some(cycle_start.elapsed().as_secs_f64()),
                ..Default::default()
            });
        }
        crate::health::write_schedule(interval);
        last_cycle = Instant::now();
    }
}

pub fn run_once_cycle(
    cli: &Cli,
    local_root: &str,
    remote_root: &str,
    filter: &Filter,
    propagate_deletes: bool,
    push: Option<&crate::events::PushContext<'_>>,
) -> Result<()> {
    let t0 = SystemTime::now();
    let started = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
    crate::health::write_health(&Health {
        status: "syncing".to_string(),
        started: Some(chrono::DateTime::<chrono::Utc>::from_timestamp(started, 0).unwrap_or_default().to_rfc3339()),
        ..Default::default()
    });

    let mut local = scan_local(Path::new(local_root), filter);
    let state_path = state_dir().join("state.json");
    let st = state::read(&state_path);

    // Nothing on disk and no state: a first sync. Read the cloud tree and
    // download it in one overlapped pass instead of walking it, planning,
    // then only then moving bytes.
    let first = st.is_empty() && local.is_empty();
    let mut failed: BTreeSet<String> = BTreeSet::new();
    let mut downloaded: BTreeSet<String> = BTreeSet::new();
    let remote = if first {
        let out = crate::firstsync::download_all(cli, remote_root, Path::new(local_root), filter)?;
        failed.extend(out.failed);
        downloaded.extend(out.downloaded);
        // the tree on disk changed: plan against what actually landed
        local = scan_local(Path::new(local_root), filter);
        crate::scan::RemoteScan { tree: out.tree, full: true }
    } else if let Some(push) = push {
        match scoped_remote(cli, remote_root, filter, push) {
            Ok(Some(scan)) => scan,
            Ok(None) => scan_remote(cli, remote_root, filter)?,
            Err(err) => {
                eprintln!("push: scoped refresh failed: {err:#}; full walk");
                scan_remote(cli, remote_root, filter)?
            }
        }
    } else {
        scan_remote(cli, remote_root, filter)?
    };
    // Publish the fresh walk immediately, before any transfers, so the GUI's
    // folder tree goes instant as soon as the scan finishes. A reused cached
    // walk keeps its own timestamp/skip bookkeeping; only full walks rewrite.
    if remote.full {
        publish_snapshot(cli, remote_root, filter, &remote.tree);
    }
    let remote = remote.tree;

    let config = PlanConfig { propagate_deletes, wipe_min: 5 };
    let result = plan_cycle(&remote, &local, &st, &config);

    if !result.actions.is_empty() {
        let (f, d) = execute(&result.actions, &remote, &local, remote_root, Path::new(local_root), filter)?;
        failed.extend(f);
        downloaded.extend(d);
    }

    let local2 = if result.actions.is_empty() && !first {
        local.clone()
    } else {
        scan_local(Path::new(local_root), filter)
    };
    // Re-list only the folders this cycle changed (uploads, trashes, moves)
    // instead of re-walking the whole cloud after every upload. This is the
    // Python 3.13.2 scoped post-upload refresh.
    let mut remote2 = remote.clone();
    let mut parents: BTreeSet<String> = BTreeSet::new();
    for action in &result.actions {
        let mut sides: Vec<&str> = Vec::new();
        match action {
            Action::Dl { .. } | Action::TrashLocal { .. } | Action::MvLocal { .. } => continue,
            Action::MvRemote { old, new } => {
                sides.push(old);
                sides.push(new);
            }
            Action::Up { rel, .. }
            | Action::TrashRemote { rel, .. }
            | Action::Conflict { rel, .. } => sides.push(rel),
        }
        for side in sides {
            let mut parent = side.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
            while !parent.is_empty() && !remote.contains_key(parent) {
                parent = parent.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
            }
            parents.insert(parent.to_string());
        }
    }
    if !parents.is_empty() {
        let scoped = crate::cli::with_cli_lock(|| -> Result<()> {
            for parent in &parents {
                crate::scan::apply_scope(cli, remote_root, filter, &mut remote2, parent)?;
            }
            publish_snapshot(cli, remote_root, filter, &remote2);
            Ok(())
        });
        if let Err(err) = scoped {
            eprintln!("post-cycle refresh failed: {err:#}; full walk");
            let out = scan_remote(cli, remote_root, filter)?;
            if out.full {
                publish_snapshot(cli, remote_root, filter, &out.tree);
            }
            remote2 = out.tree;
        }
    } else if !result.actions.is_empty() {
        // Only downloads/local changes: the remote tree is unchanged, but the
        // cached fingerprint still needs to be refreshed for the next peek.
        let _ = crate::cli::with_cli_lock(|| -> Result<()> {
            publish_snapshot(cli, remote_root, filter, &remote2);
            Ok(())
        });
    }

    let acted: BTreeSet<String> = result.actions.iter().map(|a| a.rel().to_string()).collect();
    let options = RefreshOptions {
        propagate_deletes,
        failed: &failed,
        acted: &acted,
        planned_local: Some(&local),
        downloaded: Some(&downloaded),
        preserve: Some(&BTreeSet::new()),
    };
    let saved = refresh_state(&remote2, &local2, &st, &options);
    let _ = state::write(&state_path, &saved);

    let secs = t0.elapsed().unwrap_or_default().as_secs_f64();
    crate::health::write_health(&Health {
        status: "ok".to_string(),
        cycle_seconds: Some(secs),
        actions: result.actions.len() as u64,
        uploaded: result.actions.iter().filter(|a| matches!(a, Action::Up { .. }) && !failed.contains(a.rel())).count() as u64,
        downloaded: downloaded.len() as u64,
        failed: failed.len() as u64,
        holds: result.holds as u64,
        remote_entries: Some(remote2.len() as u64),
        local_entries: Some(local2.len() as u64),
        state_records: Some(saved.len() as u64),
        ..Default::default()
    });
    Ok(())
}

/// Publish a complete remote walk for the GUI's instant folder tree and first
/// paint. Only an unfiltered walk is written: a selective-sync snapshot would
/// omit folders the user can still browse in the cloud, and the GUI treats the
/// snapshot as the whole folder list.
/// Refresh only the folders named by the event stream, reusing the rest of
/// the cached snapshot. Returns None when the snapshot cannot be trusted;
/// the caller then falls back to the normal fingerprint/full walk.
fn scoped_remote(
    cli: &Cli,
    remote_root: &str,
    filter: &Filter,
    push: &crate::events::PushContext<'_>,
) -> Result<Option<crate::scan::RemoteScan>> {
    let path = state_dir().join("remote-snapshot.json");
    let Some(snapshot) = proton_core::snapshot::read(&path) else {
        return Ok(None);
    };
    if snapshot.sig != filter.signature(remote_root) || snapshot.tree.is_empty() {
        return Ok(None);
    }
    let mut tree = snapshot.view();
    let (rels, unknown) = crate::events::resolve_scopes(
        push.pending,
        &tree,
        push.volume,
        push.root_uid,
    );
    if unknown > 0 || rels.is_empty() {
        return Ok(None);
    }
    crate::cli::with_cli_lock(|| -> Result<()> {
        for rel in &rels {
            crate::scan::apply_scope(cli, remote_root, filter, &mut tree, rel)?;
        }
        publish_snapshot(cli, remote_root, filter, &tree);
        Ok(())
    })?;
    Ok(Some(crate::scan::RemoteScan { tree, full: false }))
}

fn publish_snapshot(
    cli: &Cli,
    remote_root: &str,
    filter: &Filter,
    tree: &proton_core::snapshot::RemoteTree,
) {
    if !filter.is_unfiltered() {
        return;
    }
    let fp = cli
        .run_json(&["fs", "size", remote_root], false)
        .ok()
        .map(|v| {
            vec![
                v.get("size").and_then(|s| s.as_i64()).unwrap_or(-1),
                v.get("numberOfDescendants")
                    .and_then(|n| n.as_i64())
                    .unwrap_or(-1),
            ]
        })
        .unwrap_or_default();
    let snapshot = proton_core::snapshot::Snapshot {
        sig: filter.signature(remote_root),
        fp,
        ts: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0),
        skips: 0,
        tree: tree.clone(),
    };
    let _ = proton_core::snapshot::write(&state_dir().join("remote-snapshot.json"), &snapshot);
}

#[cfg(target_os = "linux")]
fn inotify_watch(
    root: &str,
    wake: Arc<std::sync::atomic::AtomicBool>,
    wake_event: Arc<std::sync::Condvar>,
    wake_mutex: Arc<Mutex<()>>,
) -> Result<()> {
    use anyhow::bail;
    use std::collections::HashMap;
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};

    // Watching only the root missed every change in a subfolder, so a file
    // added in e.g. "personal/" waited for the next interval or Sync now.
    fn add_watch(fd: i32, path: &Path, dirs: &mut HashMap<i32, PathBuf>) {
        let Ok(path_c) = CString::new(path.as_os_str().as_bytes()) else {
            return;
        };
        let mask = libc::IN_MODIFY
            | libc::IN_CREATE
            | libc::IN_DELETE
            | libc::IN_MOVED_FROM
            | libc::IN_MOVED_TO
            | libc::IN_CLOSE_WRITE;
        let wd = unsafe { libc::inotify_add_watch(fd, path_c.as_ptr(), mask) };
        if wd >= 0 {
            dirs.insert(wd, path.to_path_buf());
        }
    }

    fn add_tree(fd: i32, path: &Path, dirs: &mut HashMap<i32, PathBuf>) {
        add_watch(fd, path, dirs);
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                add_tree(fd, &entry.path(), dirs);
            }
        }
    }

    let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC) };
    if fd < 0 {
        bail!("inotify_init1 failed");
    }
    let mut dirs: HashMap<i32, PathBuf> = HashMap::new();
    // The sync folder may not exist yet on a fresh install. Create it here
    // (the first sync would anyway) so the watch is not silently lost until
    // the service restarts.
    let _ = std::fs::create_dir_all(root);
    add_tree(fd, Path::new(root), &mut dirs);

    let mut buf = vec![0u8; 8192];
    let event_size = std::mem::size_of::<libc::inotify_event>();
    loop {
        let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if n <= 0 {
            break;
        }
        let mut offset = 0usize;
        // Files are written in pieces: waking on IN_CREATE/IN_MODIFY starts a
        // cycle mid-copy and uploads a partial file. Only settled changes
        // (close-write, move, delete, new folder) start a cycle.
        let mut settled = false;
        while offset + event_size <= n as usize {
            let event = unsafe { &*(buf.as_ptr().add(offset) as *const libc::inotify_event) };
            // A new folder gets its own watch so later changes inside it
            // are seen too. The event name is relative to the watched dir.
            if event.mask & (libc::IN_CREATE | libc::IN_MOVED_TO) != 0 && event.len > 0 {
                let raw = unsafe {
                    std::slice::from_raw_parts(
                        buf.as_ptr().add(offset + event_size),
                        event.len as usize,
                    )
                };
                if let Ok(name) = std::ffi::CStr::from_bytes_until_nul(raw)
                    && let Some(dir) = dirs.get(&event.wd)
                {
                    let path = dir.join(name.to_string_lossy().as_ref());
                    if path.is_dir() {
                        add_tree(fd, &path, &mut dirs);
                        settled = true;
                    }
                }
            }
            if event.mask
                & (libc::IN_CLOSE_WRITE
                    | libc::IN_MOVED_FROM
                    | libc::IN_MOVED_TO
                    | libc::IN_DELETE)
                != 0
            {
                settled = true;
            }
            offset += event_size + event.len as usize;
        }
        if settled {
            wake.store(true, std::sync::atomic::Ordering::Relaxed);
            let _guard = wake_mutex.lock().unwrap();
            wake_event.notify_all();
        }
    }
    Ok(())
}

/// Block until a wake arrives or the interval deadline passes.
///
/// Checking the flag before waiting closes the race where a change lands
/// while a cycle is running: the condvar notification is lost, but the flag
/// is not, so the next cycle still starts immediately instead of sleeping
/// until the following interval.
fn wait_for_cycle(
    wake: &std::sync::atomic::AtomicBool,
    wake_event: &std::sync::Condvar,
    wake_mutex: &Mutex<()>,
    deadline: Option<Instant>,
) {
    let mut guard = wake_mutex.lock().unwrap();
    loop {
        if wake.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let wait = match deadline {
            Some(deadline) => {
                let now = Instant::now();
                if now >= deadline {
                    return;
                }
                deadline - now
            }
            None => Duration::from_secs(3600),
        };
        guard = wake_event.wait_timeout(guard, wait).unwrap().0;
    }
}

use fs2::FileExt;
fn acquire_process_lock() -> anyhow::Result<std::fs::File> {
    let sd = state_dir();
    std::fs::create_dir_all(&sd)?;
    let lock_path = sd.join("sync.lock");
    let fh = std::fs::OpenOptions::new().create(true).write(true).truncate(false).open(&lock_path)?;
    fh.try_lock_exclusive().map_err(|_| anyhow::anyhow!("another proton-sync run is already active"))?;
    Ok(fh)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn wait_until(mut done: impl FnMut() -> bool, timeout: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if done() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        done()
    }

    /// Regression: a file dropped in a subfolder must wake the loop, not
    /// wait for the next interval or a manual "Sync now".
    #[test]
    fn subfolder_change_wakes_the_loop() {
        let root = std::env::temp_dir().join(format!(
            "pd-inotify-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let sub = root.join("personal");
        std::fs::create_dir_all(&sub).unwrap();

        let wake = Arc::new(AtomicBool::new(false));
        let event = Arc::new(std::sync::Condvar::new());
        let mutex = Arc::new(Mutex::new(()));
        {
            let root = root.to_string_lossy().into_owned();
            let (w, e, m) = (wake.clone(), event.clone(), mutex.clone());
            std::thread::spawn(move || {
                let _ = inotify_watch(&root, w, e, m);
            });
        }
        // Give inotify_add_watch a moment to register.
        std::thread::sleep(Duration::from_millis(150));
        std::fs::write(sub.join("new.txt"), b"x").unwrap();

        let woke = wait_until(|| wake.load(Ordering::Relaxed), Duration::from_secs(2));
        std::fs::remove_dir_all(&root).ok();
        assert!(woke, "subfolder change did not wake the engine");
    }

    /// A file copied in pieces must not start a cycle until the writer closes
    /// it, otherwise the CLI uploads a partial (or empty) snapshot.
    #[test]
    fn partial_writes_do_not_wake_until_close() {
        use std::io::Write;
        let root = std::env::temp_dir().join(format!(
            "pd-inotify-partial-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let wake = Arc::new(AtomicBool::new(false));
        let event = Arc::new(std::sync::Condvar::new());
        let mutex = Arc::new(Mutex::new(()));
        {
            let root = root.to_string_lossy().into_owned();
            let (w, e, m) = (wake.clone(), event.clone(), mutex.clone());
            std::thread::spawn(move || {
                let _ = inotify_watch(&root, w, e, m);
            });
        }
        std::thread::sleep(Duration::from_millis(150));

        let mut file = std::fs::File::create(root.join("copy.bin")).unwrap();
        file.write_all(b"first half").unwrap();
        file.flush().unwrap();
        std::thread::sleep(Duration::from_millis(400));
        let woke_early = wake.load(Ordering::Relaxed);
        drop(file);
        let woke_after_close = wait_until(|| wake.load(Ordering::Relaxed), Duration::from_secs(2));
        std::fs::remove_dir_all(&root).ok();
        assert!(!woke_early, "a partial write started a sync cycle");
        assert!(woke_after_close, "closing the file did not wake the engine");
    }

    /// Fresh installs have no sync folder yet; the watcher must create it
    /// instead of silently watching nothing until the service restarts.
    #[test]
    fn missing_root_is_created_and_watched() {
        let root = std::env::temp_dir().join(format!(
            "pd-inotify-missing-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let wake = Arc::new(AtomicBool::new(false));
        let event = Arc::new(std::sync::Condvar::new());
        let mutex = Arc::new(Mutex::new(()));
        {
            let root = root.to_string_lossy().into_owned();
            let (w, e, m) = (wake.clone(), event.clone(), mutex.clone());
            std::thread::spawn(move || {
                let _ = inotify_watch(&root, w, e, m);
            });
        }
        std::thread::sleep(Duration::from_millis(200));
        let created = root.is_dir();
        if created {
            std::fs::write(root.join("new.txt"), b"x").unwrap();
        }
        let woke = wait_until(|| wake.load(Ordering::Relaxed), Duration::from_secs(2));
        std::fs::remove_dir_all(&root).ok();
        assert!(created, "the watcher did not create the sync root");
        assert!(woke, "a file in the newly created root did not wake the engine");
    }

    /// A change that lands while a cycle is running must not be lost: the
    /// flag is checked before sleeping, so the next cycle starts at once.
    #[test]
    fn wake_before_wait_is_not_lost() {
        let wake = AtomicBool::new(true);
        let event = std::sync::Condvar::new();
        let mutex = Mutex::new(());
        let started = Instant::now();
        wait_for_cycle(
            &wake,
            &event,
            &mutex,
            Some(Instant::now() + Duration::from_secs(5)),
        );
        assert!(started.elapsed() < Duration::from_millis(200));
    }

    /// With an interval set, a quiet wait still ends at the deadline rather
    /// than sleeping forever or until a wake.
    #[test]
    fn interval_deadline_ends_the_wait() {
        let wake = AtomicBool::new(false);
        let event = std::sync::Condvar::new();
        let mutex = Mutex::new(());
        let started = Instant::now();
        wait_for_cycle(
            &wake,
            &event,
            &mutex,
            Some(Instant::now() + Duration::from_millis(80)),
        );
        let elapsed = started.elapsed();
        assert!(
            elapsed >= Duration::from_millis(50) && elapsed < Duration::from_secs(2),
            "unexpected wait: {elapsed:?}"
        );
    }
}
