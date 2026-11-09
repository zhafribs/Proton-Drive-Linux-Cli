//! systemd user-service control, the unit-file template, pause/resume via a
//! transient autoresume timer, and local folder stats.
//!
//! Byte-compatible with the PyQt app's unit (so `ensure_service_unit`
//! recognizes and reuses an existing install), with one robustness upgrade:
//! an AppImage path containing spaces is quoted in `ExecStart` too.

use crate::exec;
use crate::paths::JOURNAL_UNIT;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const T: Duration = Duration::from_secs(30);

/// One serialized service-control lane. The desired bit is written before
/// taking the lane, so a pause and a quick resume cannot interleave into the
/// opposite state: the last click wins and the worker re-checks it under the
/// lock.
static SERVICE_MTX: Mutex<()> = Mutex::new(());
static DESIRED_RUN: AtomicBool = AtomicBool::new(true);
static AUTORESUME_GEN: AtomicU64 = AtomicU64::new(0);
static SYSTEMD_AVAILABLE: OnceLock<bool> = OnceLock::new();
static SUPERVISOR_STARTED: AtomicBool = AtomicBool::new(false);

/// The fallback supervisor's child and the last PID it wrote. A GUI restart
/// can adopt a still-running engine through the PID file even though the
/// original `Child` handle died with the previous process.
struct Sup {
    child: Option<Child>,
    pid: Option<u32>,
}
static SUP: Mutex<Sup> = Mutex::new(Sup {
    child: None,
    pid: None,
});

/// True when there is no usable systemd user session and the GUI must own the
/// engine process. `PROTON_SYNC_SUPERVISOR=1` forces the fallback for testing;
/// `=0` forces systemd control when the unit exists.
pub fn uses_supervisor() -> bool {
    !*SYSTEMD_AVAILABLE.get_or_init(|| {
        if let Ok(v) = std::env::var("PROTON_SYNC_SUPERVISOR") {
            match v.as_str() {
                "1" | "true" => return false, // forced fallback: no systemd
                "0" | "false" => return true,  // forced systemd
                _ => {}
            }
        }
        let exe = which_systemctl();
        exec::run(
            &exe,
            &["--user", "show-environment"],
            Duration::from_secs(5),
        )
        .is_ok_and(|o| o.ok())
    })
}

pub fn can_start() -> bool {
    uses_supervisor() || unit_exists()
}

fn supervisor_pid_file() -> PathBuf {
    crate::paths::resolve().state_dir.join("supervisor.pid")
}

fn supervisor_log_file() -> PathBuf {
    crate::paths::resolve().state_dir.join("supervisor.log")
}

fn read_supervisor_pid() -> Option<u32> {
    std::fs::read_to_string(supervisor_pid_file())
        .ok()?
        .trim()
        .parse()
        .ok()
}

fn pid_is_engine(pid: u32) -> bool {
    let Ok(cmdline) = std::fs::read(format!("/proc/{pid}/cmdline")) else {
        return false;
    };
    let text = String::from_utf8_lossy(&cmdline);
    text.contains("proton-engine")
        || text.contains("__sync-engine")
        || text.contains("ProtonDrive-CLI")
}

fn write_supervisor_pid(pid: u32) {
    if let Some(parent) = supervisor_pid_file().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(supervisor_pid_file(), format!("{pid}\n"));
}

fn kill_pid(pid: u32) {
    let _ = Command::new("kill").arg("-TERM").arg(pid.to_string()).status();
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if !Path::new(&format!("/proc/{pid}")).exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = Command::new("kill").arg("-KILL").arg(pid.to_string()).status();
}

fn engine_command() -> Option<(PathBuf, Vec<&'static str>, Option<PathBuf>)> {
    let paths = crate::paths::resolve();
    // Prefer the engine binary next to this process. Inside an AppImage that
    // is the mounted `usr/bin/proton-engine`, so the supervisor owns the real
    // engine PID instead of an AppRun wrapper that may fork and exit.
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let engine = dir.join("proton-engine");
        if is_executable_file(&engine) {
            let drive_bin = unit_appimage(paths.appimage.as_deref());
            return Some((engine, vec!["--loop"], drive_bin));
        }
    }
    if let Some(appimage) = unit_appimage(paths.appimage.as_deref()) {
        return Some((appimage.clone(), vec!["__sync-engine"], Some(appimage)));
    }
    which("proton-engine").map(|p| (PathBuf::from(p), vec!["--loop"], None))
}

fn fallback_active_locked(sup: &mut Sup) -> bool {
    if let Some(child) = sup.child.as_mut() {
        match child.try_wait() {
            Ok(None) => return true,
            _ => sup.child = None,
        }
    }
    if let Some(pid) = sup.pid.or_else(read_supervisor_pid)
        && pid_is_engine(pid)
    {
        sup.pid = Some(pid);
        return true;
    }
    sup.pid = None;
    let _ = std::fs::remove_file(supervisor_pid_file());
    false
}

fn fallback_active() -> bool {
    let mut sup = SUP.lock().unwrap();
    fallback_active_locked(&mut sup)
}

fn fallback_start() -> bool {
    let mut sup = SUP.lock().unwrap();
    if fallback_active_locked(&mut sup) {
        return true;
    }
    let Some((exe, args, drive_bin)) = engine_command() else {
        eprintln!("supervisor: proton-engine was not found beside the GUI");
        return false;
    };
    let log = supervisor_log_file();
    if let Some(parent) = log.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&log)
        .ok();
    let mut cmd = Command::new(&exe);
    cmd.args(&args)
        .stdin(Stdio::null())
        .env("PROTON_GUI_VERSION", crate::GUI_VERSION)
        .env("MALLOC_ARENA_MAX", "2");
    if let Some(bin) = &drive_bin {
        cmd.env("PROTON_DRIVE_BIN", bin);
    }
    if let Some(base) = crate::paths::resolve().state_dir.parent() {
        cmd.env("XDG_STATE_HOME", base);
    }
    if let Some(base) = crate::paths::resolve().conf_file.parent() {
        cmd.env("XDG_CONFIG_HOME", base);
    }
    if let Some(file) = log_file
        && let Ok(err) = file.try_clone()
    {
        cmd.stdout(Stdio::from(file));
        cmd.stderr(Stdio::from(err));
    }
    match cmd.spawn() {
        Ok(child) => {
            let pid = child.id();
            let _ = writeln!(
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&log)
                    .unwrap_or_else(|_| std::fs::File::create("/dev/null").unwrap()),
                "supervisor: started proton-engine pid {pid}"
            );
            sup.child = Some(child);
            sup.pid = Some(pid);
            write_supervisor_pid(pid);
            true
        }
        Err(e) => {
            eprintln!("supervisor: could not start {}: {e}", exe.display());
            false
        }
    }
}

fn fallback_stop() -> bool {
    let mut sup = SUP.lock().unwrap();
    if let Some(mut child) = sup.child.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    if let Some(pid) = sup.pid.take().or_else(read_supervisor_pid)
        && pid_is_engine(pid)
    {
        kill_pid(pid);
    }
    let _ = std::fs::remove_file(supervisor_pid_file());
    true
}

/// Start the GUI-owned engine supervisor when systemd is unavailable. Safe to
/// call more than once; only one monitor thread is created.
pub fn spawn_supervisor() {
    if !uses_supervisor() || SUPERVISOR_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    if crate::paths::resolve().state_dir.join("paused").exists() {
        DESIRED_RUN.store(false, Ordering::SeqCst);
    } else {
        let _ = fallback_start();
    }
    std::thread::spawn(|| {
        loop {
            std::thread::sleep(Duration::from_secs(3));
            if !DESIRED_RUN.load(Ordering::SeqCst) {
                fallback_stop();
                continue;
            }
            if !fallback_active() && !fallback_start() {
                std::thread::sleep(Duration::from_secs(5));
            }
        }
    });
}

fn start_locked() -> bool {
    if uses_supervisor() {
        return fallback_start();
    }
    systemctl(&["--user", "start", JOURNAL_UNIT])
}

fn stop_locked() -> bool {
    if uses_supervisor() {
        return fallback_stop();
    }
    systemctl(&["--user", "stop", JOURNAL_UNIT])
}

fn restart_locked() -> bool {
    if uses_supervisor() {
        fallback_stop();
        return fallback_start();
    }
    systemctl(&["--user", "restart", JOURNAL_UNIT])
}

fn systemctl(args: &[&str]) -> bool {
    let exe = which_systemctl();
    exec::run_ok(&exe, args, T)
}

fn which_systemctl() -> String {
    which("systemctl").unwrap_or_else(|| "systemctl".into())
}

fn which(name: &str) -> Option<String> {
    if let Ok(paths) = std::env::var("PATH") {
        for p in std::env::split_paths(&paths) {
            let c = p.join(name);
            if c.is_file() {
                return Some(c.to_string_lossy().into_owned());
            }
        }
    }
    None
}

pub fn is_active() -> bool {
    if uses_supervisor() {
        return fallback_active();
    }
    systemctl(&["--user", "is-active", "--quiet", JOURNAL_UNIT])
}

pub fn is_enabled() -> bool {
    if uses_supervisor() {
        // The fallback has no service to enable; the app's login entry is the
        // only thing that can start it.
        return true;
    }
    systemctl(&["--user", "is-enabled", "--quiet", JOURNAL_UNIT])
}

pub fn start() -> bool {
    DESIRED_RUN.store(true, Ordering::SeqCst);
    let _lane = SERVICE_MTX.lock().unwrap();
    if !DESIRED_RUN.load(Ordering::SeqCst) {
        return false;
    }
    start_locked()
}

pub fn restart() -> bool {
    DESIRED_RUN.store(true, Ordering::SeqCst);
    let _lane = SERVICE_MTX.lock().unwrap();
    if !DESIRED_RUN.load(Ordering::SeqCst) {
        return false;
    }
    restart_locked()
}

/// Restart the unit and confirm it stayed up. `systemctl restart` reports
/// success even when the unit dies immediately after exec - a stale
/// `ExecStart` does exactly that (status=203/EXEC) and used to leave "Sync
/// engine isn't running" on screen while the toast claimed success.
pub fn restart_and_confirm() -> bool {
    if !restart() {
        return false;
    }
    for _ in 0..10 {
        if is_active() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
    false
}

pub fn enable() -> bool {
    if uses_supervisor() {
        return true;
    }
    systemctl(&["--user", "enable", JOURNAL_UNIT])
}

pub fn disable() -> bool {
    if uses_supervisor() {
        return true;
    }
    systemctl(&["--user", "disable", JOURNAL_UNIT])
}

/// Stop syncing; schedule an automatic return after `secs`, or leave it off
/// until the user resumes (`None`).
pub fn pause_for(secs: Option<u64>) -> bool {
    DESIRED_RUN.store(false, Ordering::SeqCst);
    let _lane = SERVICE_MTX.lock().unwrap();
    if DESIRED_RUN.load(Ordering::SeqCst) {
        return false; // a resume was requested while this thread waited
    }
    if uses_supervisor() {
        let generation = AUTORESUME_GEN.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(secs) = secs {
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(secs));
                if AUTORESUME_GEN.load(Ordering::SeqCst) == generation
                    && !DESIRED_RUN.load(Ordering::SeqCst)
                {
                    DESIRED_RUN.store(true, Ordering::SeqCst);
                    let _ = fallback_start();
                }
            });
        }
        let _ = stop_locked();
        return true;
    }
    systemctl(&["--user", "stop", "proton-sync-autoresume.timer"]);
    systemctl(&["--user", "stop", "proton-sync-autoresume.service"]);
    let scheduled = match secs {
        Some(s) => {
            let exe = which("systemd-run").unwrap_or_else(|| "systemd-run".into());
            let on_active = format!("--on-active={s}");
            let unit_arg = format!("--unit={}-autoresume", JOURNAL_UNIT);
            exec::run_ok(
                &exe,
                &[
                    "--user",
                    &unit_arg,
                    &on_active,
                    "systemctl",
                    "--user",
                    "start",
                    JOURNAL_UNIT,
                ],
                T,
            )
        }
        None => true,
    };
    // Even if the autoresume timer could not be scheduled, stop now: the
    // caller has already recorded the pause, and leaving the engine running
    // would make the UI lie about the state.
    let _ = scheduled;
    stop_locked()
}

/// Undo a pause: cancel the pending autoresume and start syncing now.
pub fn resume() -> bool {
    DESIRED_RUN.store(true, Ordering::SeqCst);
    let _lane = SERVICE_MTX.lock().unwrap();
    if !DESIRED_RUN.load(Ordering::SeqCst) {
        return false; // a pause was requested while this thread waited
    }
    if uses_supervisor() {
        AUTORESUME_GEN.fetch_add(1, Ordering::SeqCst);
        return start_locked();
    }
    systemctl(&["--user", "stop", "proton-sync-autoresume.timer"]);
    start_locked()
}

fn quote_for_unit(v: &str) -> String {
    if v.contains(' ') {
        format!("\"{v}\"")
    } else {
        v.to_string()
    }
}

/// The AppImage the unit should run. The recorded path can be gone - an update
/// may install the new build under a new file name - and then every start dies
/// with `203/EXEC` while the unit still names the old file, which is what left
/// "Sync engine isn't running" stuck on screen in 3.16.91 and earlier. Fall back
/// to the build we are actually running, else the newest Drive AppImage beside
/// the old one.
fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = path.metadata() else { return false };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn unit_appimage(recorded: Option<&Path>) -> Option<std::path::PathBuf> {
    let recorded = recorded?;
    if is_executable_file(recorded) {
        return Some(recorded.to_path_buf());
    }
    let dir = recorded.parent()?;
    let want = format!("GUI-{}-", crate::GUI_VERSION);
    let mut best: Option<(bool, std::time::SystemTime, std::path::PathBuf)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !(name.starts_with("ProtonDrive-CLI-") && name.ends_with(".AppImage")) {
            continue;
        }
        if !is_executable_file(&entry.path()) {
            continue;
        }
        let mtime = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        let ours = name.contains(&want);
        if best.as_ref().is_none_or(|(b, t, _)| (ours, mtime) > (*b, *t)) {
            best = Some((ours, mtime, entry.path()));
        }
    }
    best.map(|(_, _, p)| p)
}

/// Keep the sync service pointing at THIS copy of the app - byte-identical
/// to the PyQt app's unit (which this app reuses rather than rewrites).
/// Writes + daemon-reload only; callers restart as needed.
///
/// Returns "ok" | "written" | "failed" | "" (not running from an AppImage,
/// or a smoke run, which must leave the live unit alone).
pub fn ensure_service_unit(appimage: Option<&Path>, smoke: bool) -> String {
    if smoke {
        return String::new();
    }
    if uses_supervisor() {
        return "supervisor".to_string();
    }
    let exe = match unit_appimage(appimage) {
        Some(p) => p.to_string_lossy().into_owned(),
        None => return String::new(),
    };
    // Qt ran Path.resolve() - the symlink ~/.local/bin/proton-drive stays
    // valid either way, but the canonical path is what the unit records.
    let exe =
        std::fs::canonicalize(&exe).map(|p| p.to_string_lossy().into_owned()).unwrap_or(exe);
    let paths = crate::paths::resolve();
    let conf = paths.conf_file.to_string_lossy().into_owned();
    let state_base = paths
        .state_dir
        .parent()
        .unwrap_or(Path::new("/"))
        .to_string_lossy()
        .into_owned();
    let config_base = paths
        .conf_file
        .parent()
        .unwrap_or(Path::new("/"))
        .to_string_lossy()
        .into_owned();
    let unit = format!(
        "[Unit]\n\
         Description=Proton Drive folder sync (official CLI, two-way)\n\
         \n\
         [Service]\n\
         Type=simple\n\
         Environment={}\n\
         Environment=PROTON_GUI_VERSION={}\n\
         Environment=MALLOC_ARENA_MAX=2\n\
         Environment={}\n\
         Environment={}\n\
         EnvironmentFile={}\n\
         ExecStart={} __sync-engine\n\
         Restart=on-failure\n\
         RestartSec=15\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        quote_for_unit(&format!("PROTON_DRIVE_BIN={exe}")),
        crate::GUI_VERSION,
        quote_for_unit(&format!("XDG_STATE_HOME={state_base}")),
        quote_for_unit(&format!("XDG_CONFIG_HOME={config_base}")),
        quote_for_unit(&format!("-{conf}")),
        quote_for_unit(&exe)
    );
    let current = std::fs::read_to_string(crate::service_unit_path()).unwrap_or_default();
    if current == unit {
        return "ok".to_string();
    }
    let unit_path = crate::service_unit_path();
    if std::fs::create_dir_all(unit_path.parent().unwrap_or(Path::new("."))).is_err() {
        return "failed".to_string();
    }
    // Write beside the unit and rename, so a crash or ENOSPC never leaves a
    // truncated service file that the next restart will try to execute.
    let tmp = unit_path.with_extension("service.tmp");
    if std::fs::write(&tmp, &unit).is_err() || std::fs::rename(&tmp, &unit_path).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return "failed".to_string();
    }
    if !systemctl(&["--user", "daemon-reload"]) {
        return "failed".to_string();
    }
    "written".to_string()
}

/// Is there a unit file at all (repo installs may predate this app)?
pub fn unit_exists() -> bool {
    crate::service_unit_path().is_file()
}

/// Does the installed unit look like a current one (has EnvironmentFile)?
pub fn unit_has_environment_file() -> bool {
    std::fs::read_to_string(crate::service_unit_path())
        .map(|t| t.contains("EnvironmentFile"))
        .unwrap_or(false)
}

/// Ask the running engine to check now. The file is a signal only: the engine
/// consumes it within a second, runs a full cloud walk and removes it. Without
/// that reader the button was a no-op (fixed in 3.16.91).
pub fn request_full_check(paths: &crate::paths::Paths) -> std::io::Result<()> {
    std::fs::create_dir_all(&paths.state_dir)?;
    std::fs::write(paths.state_dir.join("audit-request"), b"")
}

/// Drop only the cached remote snapshot. State, local files and settings stay.
pub fn clear_snapshot_cache(paths: &crate::paths::Paths) -> std::io::Result<()> {
    match std::fs::remove_file(&paths.snapshot_file) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// `journalctl --user -u proton-sync` -n <n> (same shape the PyQt app used).
pub fn journal_lines(n: usize) -> Vec<String> {
    if uses_supervisor() {
        let text = std::fs::read_to_string(supervisor_log_file()).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        let start = lines.len().saturating_sub(n);
        return lines[start..].iter().map(|s| s.to_string()).collect();
    }
    let exe = match which("journalctl") {
        Some(e) => e,
        None => return Vec::new(),
    };
    let n_arg = n.to_string();
    match exec::run(
        &exe,
        &["--user", "-u", JOURNAL_UNIT, "--no-pager", "-n", &n_arg, "-o", "short-iso-precise"],
        Duration::from_secs(10),
    ) {
        Ok(o) if o.ok() => o.stdout.lines().map(str::to_string).collect(),
        _ => Vec::new(),
    }
}

/// Files + bytes in the local sync folder (walked off-thread by the caller).
pub fn folder_stats(local: &str) -> (u64, u64) {
    let root = Path::new(local);
    if !root.is_dir() {
        return (0, 0);
    }
    let mut files = 0u64;
    let mut bytes = 0u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for e in entries.flatten() {
            let path = e.path();
            // The engine scanner deliberately ignores symlinks; the GUI's
            // counter must use the same rule so it never walks outside the
            // sync root (or stalls on a symlink loop).
            let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
            let kind = meta.file_type();
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                stack.push(path);
            } else if kind.is_file() {
                files += 1;
                bytes += meta.len();
            }
        }
    }
    (files, bytes)
}

/// Human byte size, matching the PyQt app's "%.1f GB / %.0f MB / %.0f KB".
pub fn human_size(bytes: u64) -> String {
    const GB: u64 = 1 << 30;
    const MB: u64 = 1 << 20;
    const KB: u64 = 1 << 10;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.0} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.0} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_template_shape() {
        // Quoting applies to paths with spaces (this repo lives in one).
        assert_eq!(quote_for_unit("plain"), "plain");
        assert_eq!(quote_for_unit("a b"), "\"a b\"");
    }

    /// An update can leave the unit naming a file that no longer exists
    /// (203/EXEC forever); the next write must follow the build we run.
    #[test]
    fn unit_appimage_follows_a_renamed_update() {
        let dir = std::env::temp_dir().join(format!(
            "pdservice-image-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let gone = dir.join("ProtonDrive-CLI-0.9.0-GUI-3.13.0-x86_64.AppImage");
        let current = dir.join(format!(
            "ProtonDrive-CLI-0.9.0-GUI-{}-x86_64.AppImage",
            crate::GUI_VERSION
        ));
        std::fs::write(&current, b"appimage").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&current, std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }
        // Unrelated or non-executable AppImages are never picked up.
        std::fs::write(dir.join("ZapFast-0.21.0-x86_64.AppImage"), b"x").unwrap();
        let stale = dir.join("ProtonDrive-CLI-0.9.0-GUI-3.12.0-x86_64.AppImage");
        std::fs::write(&stale, b"not executable").unwrap();

        // An existing path is used as-is; a vanished one resolves to the
        // build we are running.
        assert_eq!(unit_appimage(Some(&current)), Some(current.clone()));
        assert_eq!(unit_appimage(Some(&gone)), Some(current.clone()));
        assert_eq!(unit_appimage(None), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn request_full_check_and_clear_cache_touch_only_expected_files() {
        let mut paths = crate::paths::resolve();
        let dir = std::env::temp_dir().join(format!(
            "pdservice-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        paths.state_dir = dir.clone();
        paths.snapshot_file = dir.join("remote-snapshot.json");
        std::fs::write(&paths.snapshot_file, b"cache").unwrap();

        request_full_check(&paths).unwrap();
        assert!(dir.join("audit-request").is_file());
        clear_snapshot_cache(&paths).unwrap();
        assert!(!paths.snapshot_file.exists());
        assert!(dir.join("audit-request").is_file());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn folder_stats_skips_symlinks_and_never_leaves_the_root() {
        let dir = std::env::temp_dir().join(format!(
            "pdservice-stats-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let outside = dir.join("outside");
        let root = dir.join("root");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(root.join("a.bin"), b"abc").unwrap();
        std::fs::write(root.join("sub/b.bin"), b"defg").unwrap();
        std::fs::write(outside.join("big.bin"), vec![0u8; 4096]).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
        assert_eq!(folder_stats(root.to_str().unwrap()), (2, 7));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn human_size_units() {
        const KB: u64 = 1024;
        const MB: u64 = 1024 * 1024;
        const GB: u64 = 1024 * 1024 * 1024;
        assert_eq!(human_size(2 * GB + 5 * (GB / 10)), "2.5 GB");
        assert_eq!(human_size(400 * MB), "400 MB");
        assert_eq!(human_size(3 * KB), "3 KB");
        assert_eq!(human_size(10), "10 B");
    }
}
