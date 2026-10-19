//! Desktop integration: login autostart entry and the app-menu entry, with
//! the exact file contents the PyQt app wrote (so upgrades stay idempotent).

use crate::exec;
use crate::paths::Paths;
use std::path::Path;

/// How this app is started from outside (login autostart, app menu).
/// AppImage first, then the manual-install shim, then the running binary.
pub fn launch_command(paths: &Paths) -> String {
    if let Some(p) = &paths.appimage {
        return p.to_string_lossy().into_owned();
    }
    if let Some(home) = dirs::home_dir() {
        let shim = home.join(".local/bin/proton-drive-gui");
        if shim.is_file() {
            return shim.to_string_lossy().into_owned();
        }
    }
    std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "proton-drive".into())
}

fn desktop_exec(cmd: &str) -> String {
    if cmd.contains(' ') {
        format!("\"{cmd}\"")
    } else {
        cmd.to_string()
    }
}

pub fn autostart_installed(paths: &Paths) -> bool {
    paths.autostart_file.is_file()
}

pub fn set_autostart(paths: &Paths, on: bool) {
    if on {
        let _ = std::fs::create_dir_all(
            paths.autostart_file.parent().unwrap_or(Path::new(".")),
        );
        let content = format!(
            "[Desktop Entry]\nType=Application\nName=Proton Drive\n\
             Comment=Keep your Proton Drive folder in sync\n\
             Exec={}\nX-GNOME-Autostart-enabled=true\n",
            desktop_exec(&launch_command(paths))
        );
        let _ = std::fs::write(&paths.autostart_file, content);
    } else {
        let _ = std::fs::remove_file(&paths.autostart_file);
    }
}

pub fn menu_installed(paths: &Paths) -> bool {
    paths.menu_desktop_file.is_file()
}

pub fn set_menu(paths: &Paths, on: bool) {
    if on {
        let _ = std::fs::create_dir_all(
            paths.menu_desktop_file.parent().unwrap_or(Path::new(".")),
        );
        let content = format!(
            "[Desktop Entry]\nType=Application\nName=Proton Drive\n\
             Comment=Keep your Proton Drive folder in sync\n\
             Exec={}\nIcon={}\nCategories=Utility;Network;\n\
             Terminal=false\nStartupNotify=true\n",
            desktop_exec(&launch_command(paths)),
            paths.menu_icon_file.display()
        );
        let _ = std::fs::write(&paths.menu_desktop_file, content);
        if let Some(icon) = crate::paths::find_icon_svg() {
            let _ = std::fs::create_dir_all(
                paths.menu_icon_file.parent().unwrap_or(Path::new(".")),
            );
            let _ = std::fs::copy(icon, &paths.menu_icon_file);
        }
    } else {
        let _ = std::fs::remove_file(&paths.menu_desktop_file);
        let _ = std::fs::remove_file(&paths.menu_icon_file);
    }
    if let Some(exe) = which("update-desktop-database") {
        if let Some(dir) = paths.menu_desktop_file.parent() {
            let _ = exec::run_ok(
                &exe,
                &[&dir.to_string_lossy()],
                std::time::Duration::from_secs(15),
            );
        }
    }
}

fn which(name: &str) -> Option<String> {
    std::env::var("PATH").ok().and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|p| p.join(name))
            .find(|p| p.is_file())
            .map(|p| p.to_string_lossy().into_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_exec_quotes_only_spaced_paths() {
        assert_eq!(desktop_exec("/opt/pd.AppImage"), "/opt/pd.AppImage");
        assert_eq!(
            desktop_exec("/home/me/Appimage Projects/x.AppImage"),
            "\"/home/me/Appimage Projects/x.AppImage\""
        );
    }

    #[test]
    fn autostart_and_menu_roundtrip() {
        let tmp = std::env::temp_dir().join(format!("pddesktop-{}", std::process::id()));
        let mut paths = crate::paths::resolve();
        paths.autostart_file = tmp.join("autostart/proton-drive.desktop");
        paths.menu_desktop_file = tmp.join("applications/proton-drive.desktop");
        paths.menu_icon_file = tmp.join("icons/proton-drive-gui.svg");

        assert!(!autostart_installed(&paths));
        set_autostart(&paths, true);
        let text = std::fs::read_to_string(&paths.autostart_file).unwrap();
        assert!(text.starts_with("[Desktop Entry]\nType=Application\nName=Proton Drive\n"));
        assert!(text.contains("X-GNOME-Autostart-enabled=true"));
        assert!(autostart_installed(&paths));
        set_autostart(&paths, false);
        assert!(!autostart_installed(&paths));

        set_menu(&paths, true);
        let text = std::fs::read_to_string(&paths.menu_desktop_file).unwrap();
        assert!(text.contains("Categories=Utility;Network;"));
        assert!(text.contains("Terminal=false"));
        assert!(menu_installed(&paths));
        set_menu(&paths, false);
        assert!(!menu_installed(&paths));

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
