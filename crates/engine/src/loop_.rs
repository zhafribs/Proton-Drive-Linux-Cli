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
        let root = local_root.clone();
        let w = wake.clone();
        let we = wake_event.clone();
        let wm = wake_mutex.clone();
        std::thread::spawn(move || {
            let _ = inotify_watch(&root, w, we, wm);
        });
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

    // push listener (best-effort)
    let _push = crate::push::PushListener::start(wake_event.clone(), wake_mutex.clone());

    let mut last_cycle = Instant::now();
    let mut first = true;
    loop {
        // the first cycle runs at once; later ones wait for the interval or a wake
        if !first {
            if interval == 0 {
                let guard = wake_mutex.lock().unwrap();
                drop(wake_event.wait_timeout(guard, Duration::from_secs(3600)).unwrap());
            } else {
                let elapsed = last_cycle.elapsed().as_secs();
                if elapsed < interval {
                    std::thread::sleep(Duration::from_secs(interval - elapsed));
                }
            }
        }
        first = false;
        wake.store(false, std::sync::atomic::Ordering::Relaxed);
        let _ = run_once_cycle(&cli, &local_root, &remote_root, &filter, propagate_deletes);
        last_cycle = Instant::now();
    }
}

pub fn run_once_cycle(cli: &Cli, local_root: &str, remote_root: &str, filter: &Filter, propagate_deletes: bool) -> Result<()> {
    let t0 = SystemTime::now();
    let started = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
    let _ = crate::health::write_health(&Health {
        status: "syncing".to_string(),
        started: Some(chrono::DateTime::<chrono::Utc>::from_timestamp(started, 0).unwrap_or_default().to_rfc3339()),
        ..Default::default()
    });

    let local = scan_local(Path::new(local_root), filter);
    let remote = scan_remote(cli, remote_root, filter)?;
    let state_path = state_dir().join("state.json");
    let st = state::read(&state_path);

    let config = PlanConfig { propagate_deletes, wipe_min: 5 };
    let result = plan_cycle(&remote, &local, &st, &config);

    let mut failed: BTreeSet<String> = BTreeSet::new();
    let mut downloaded: BTreeSet<String> = BTreeSet::new();
    if !result.actions.is_empty() {
        let (f, d) = execute(&result.actions, &remote, &local, remote_root, Path::new(local_root), filter)?;
        failed = f;
        downloaded = d;
    }

    // rescan only when something was mutated; an idle cycle reuses the first walk
    let (local2, remote2) = if result.actions.is_empty() {
        (local.clone(), remote.clone())
    } else {
        (scan_local(Path::new(local_root), filter), scan_remote(cli, remote_root, filter)?)
    };

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
    let _ = crate::health::write_health(&Health {
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

#[cfg(target_os = "linux")]
fn inotify_watch(
    root: &str,
    wake: Arc<std::sync::atomic::AtomicBool>,
    wake_event: Arc<std::sync::Condvar>,
    wake_mutex: Arc<Mutex<()>>,
) -> Result<()> {
    use anyhow::bail;
    use std::ffi::CString;
    let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC) };
    if fd < 0 {
        bail!("inotify_init1 failed");
    }
    fn add_watch(fd: i32, path: &str) {
        let c = CString::new(path).unwrap_or_default();
        unsafe {
            libc::inotify_add_watch(fd, c.as_ptr(), libc::IN_MODIFY | libc::IN_CREATE | libc::IN_DELETE | libc::IN_MOVED_FROM | libc::IN_MOVED_TO | libc::IN_CLOSE_WRITE);
        }
    }
    add_watch(fd, root);
    let mut buf = vec![0u8; 4096];
    loop {
        let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        if n <= 0 { break; }
        wake.store(true, std::sync::atomic::Ordering::Relaxed);
        let _guard = wake_mutex.lock().unwrap();
        wake_event.notify_all();
    }
    Ok(())
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
