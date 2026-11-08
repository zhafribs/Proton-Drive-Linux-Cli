//! systemd user-service control, the unit-file template, pause/resume via a
//! transient autoresume timer, and local folder stats.
//!
//! Byte-compatible with the PyQt app's unit (so `ensure_service_unit`
//! recognizes and reuses an existing install), with one robustness upgrade:
//! an AppImage path containing spaces is quoted in `ExecStart` too.

use crate::exec;
use crate::paths::JOURNAL_UNIT;
use std::path::Path;
use std::time::Duration;

const T: Duration = Duration::from_secs(30);

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
    systemctl(&["--user", "is-active", "--quiet", JOURNAL_UNIT])
}

pub fn is_enabled() -> bool {
    systemctl(&["--user", "is-enabled", "--quiet", JOURNAL_UNIT])
}

pub fn start() -> bool {
    systemctl(&["--user", "start", JOURNAL_UNIT])
}

pub fn stop() -> bool {
    systemctl(&["--user", "stop", JOURNAL_UNIT])
}

pub fn restart() -> bool {
    systemctl(&["--user", "restart", JOURNAL_UNIT])
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
    systemctl(&["--user", "enable", JOURNAL_UNIT])
}

pub fn disable() -> bool {
    systemctl(&["--user", "disable", JOURNAL_UNIT])
}

/// Stop syncing; schedule an automatic return after `secs`, or leave it off
/// until the user resumes (`None`).
pub fn pause_for(secs: Option<u64>) -> bool {
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
    stop()
}

/// Undo a pause: cancel the pending autoresume and start syncing now.
pub fn resume() -> bool {
    systemctl(&["--user", "stop", "proton-sync-autoresume.timer"]);
    start()
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
    let exe = match which("journalctl") {
        Some(e) => e,
        None => return Vec::new(),
    };
    let n_arg = n.to_string();
    match exec::run(
        &exe,
        &["--user", "-u", JOURNAL_UNIT, "--no-pager", "-n", &n_arg, "-o", "short-iso"],
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
