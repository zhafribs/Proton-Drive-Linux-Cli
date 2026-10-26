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
    scheduled && stop()
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
    let exe = match appimage {
        Some(p) if p.is_file() => p.to_string_lossy().into_owned(),
        _ => return String::new(),
    };
    // Qt ran Path.resolve() - the symlink ~/.local/bin/proton-drive stays
    // valid either way, but the canonical path is what the unit records.
    let exe =
        std::fs::canonicalize(&exe).map(|p| p.to_string_lossy().into_owned()).unwrap_or(exe);
    let conf = crate::paths::resolve().conf_file.to_string_lossy().into_owned();
    let unit = format!(
        "[Unit]\n\
         Description=Proton Drive folder sync (official CLI, two-way)\n\
         \n\
         [Service]\n\
         Type=simple\n\
         Environment={}\n\
         Environment={}\n\
         Environment=MALLOC_ARENA_MAX=2\n\
         EnvironmentFile={}\n\
         ExecStart={} __sync-engine\n\
         Restart=on-failure\n\
         RestartSec=15\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        quote_for_unit(&format!("PROTON_DRIVE_BIN={exe}")),
        quote_for_unit("PROTON_SYNC_ENGINE=rust"),
        quote_for_unit(&format!("-{conf}")),
        quote_for_unit(&exe)
    );
    let current = std::fs::read_to_string(crate::service_unit_path()).unwrap_or_default();
    if current == unit {
        return "ok".to_string();
    }
    if std::fs::create_dir_all(crate::service_unit_path().parent().unwrap_or(Path::new(".")))
        .is_err()
        || std::fs::write(crate::service_unit_path(), &unit).is_err()
    {
        return "failed".to_string();
    }
    systemctl(&["--user", "daemon-reload"]);
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

/// Ask the running engine to start a fresh background audit. The file is a
/// signal only; the engine removes it once it has started the audit.
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
            let Ok(meta) = e.metadata() else { continue };
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file() {
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
