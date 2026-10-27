//! Every filesystem location the app touches, resolved once at startup.
//!
//! Layout mirrors the PyQt app exactly (same conf, same state files, same
//! unit), so an upgrade from the Python GUI keeps the user's settings,
//! caches and installed service.

use std::path::PathBuf;
use std::sync::OnceLock;

pub const REPO_URL: &str = "https://github.com/zhafribs/Proton-Drive-Linux-Cli";
pub const RELEASE_API: &str =
    "https://api.github.com/repos/zhafribs/Proton-Drive-Linux-Cli/releases/latest";
pub const RELEASE_PAGE: &str =
    "https://github.com/zhafribs/Proton-Drive-Linux-Cli/releases/latest";
pub const JOURNAL_UNIT: &str = "proton-sync";

#[derive(Clone)]
pub struct Paths {
    pub conf_file: PathBuf,
    pub unit_file: PathBuf,
    pub state_dir: PathBuf,
    pub health_file: PathBuf,
    pub wake_file: PathBuf,
    pub lock_file: PathBuf,
    pub snapshot_file: PathBuf,
    pub cloud_cache_file: PathBuf,
    pub release_cache_file: PathBuf,
    pub update_mark_file: PathBuf,
    pub autostart_file: PathBuf,
    pub menu_desktop_file: PathBuf,
    pub menu_icon_file: PathBuf,
    pub prefs_file: PathBuf,
    /// AppImage self-path when running from one (updates + service unit).
    pub appimage: Option<PathBuf>,
}

/// The app-wide lock file that serialises every call into the official
/// CLI. The GUI and the background sync engine both drive that CLI, and
/// two of them refreshing the keyring session at the same time can lose
/// the session outright - see `clilock` for the details.
pub fn cli_lock_file() -> &'static PathBuf {
    static LOCK: OnceLock<PathBuf> = OnceLock::new();
    LOCK.get_or_init(|| resolve().state_dir.join("cli.lock"))
}

pub fn resolve() -> Paths {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let config = dirs::config_dir().unwrap_or_else(|| home.join(".config"));
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/state"))
        .join("proton-sync");

    // Qt required a real file and recorded its canonical path (resolve()).
    let appimage = std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .and_then(|p| std::fs::canonicalize(&p).ok().or(Some(p)));

    Paths {
        conf_file: config.join("proton-sync.conf"),
        unit_file: config.join("systemd/user/proton-sync.service"),
        state_dir: state.clone(),
        health_file: state.join("health.json"),
        wake_file: state.join("wake"),
        lock_file: state.join("gui.lock"),
        snapshot_file: state.join("remote-snapshot.json"),
        cloud_cache_file: state.join("cloud-cache.json"),
        release_cache_file: state.join("release-cache.json"),
        update_mark_file: state.join("update-pending.json"),
        autostart_file: config.join("autostart/proton-drive.desktop"),
        menu_desktop_file: home
            .join(".local/share/applications")
            .join("proton-drive.desktop"),
        menu_icon_file: home
            .join(".local/share/icons/hicolor/scalable/apps")
            .join("proton-drive-gui.svg"),
        prefs_file: config.join("ProtonDriveCLI/ProtonDriveGUI.conf"),
        appimage,
    }
}

/// The app icon SVG (release AppImage ships it next to the scripts, a repo
/// checkout has it in src/). Used for notifications and the app-menu entry.
pub fn find_icon_svg() -> Option<PathBuf> {
    let mut cands: Vec<PathBuf> = Vec::new();
    if let Ok(lib) = std::env::var("APPDIR") {
        cands.push(PathBuf::from(lib).join("usr/lib/proton-sync/protondrive-icon.svg"));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent() {
            cands.push(dir.join("../usr/lib/proton-sync/protondrive-icon.svg"));
            cands.push(dir.join("../src/protondrive-icon.svg"));
            cands.push(dir.join("protondrive-icon.svg"));
        }
    if let Some(home) = dirs::home_dir() {
        cands.push(home.join(".local/share/icons/hicolor/scalable/apps/proton-drive-gui.svg"));
    }
    cands.push(PathBuf::from(
        "/usr/share/icons/hicolor/scalable/apps/proton-drive-gui.svg",
    ));
    cands.into_iter().find(|p| p.is_file())
}
