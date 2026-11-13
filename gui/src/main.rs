//! Proton Drive for Linux - desktop GUI.
//!
//! The GUI and the background sync engine are Rust; every transfer still goes
//! through the official Proton Drive CLI (unchanged, MIT, bundled as-is).

mod cli;
mod clilock;
mod conf;
mod desktop;
mod exec;
mod health;
mod i18n;
mod journal;
mod notify;
mod paths;
mod portal;
mod prefs;
mod release;
mod service;
mod statewatch;
mod system_theme;
mod theme;
mod tray;
mod ui;
mod util;

use eframe::egui;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The GUI's own version (the release tag is `v<CLI>-gui<GUI>`).
pub const GUI_VERSION: &str = "3.33.0";

/// The systemd user unit path (shared with `service.rs`).
pub fn service_unit_path() -> PathBuf {
    paths::resolve().unit_file
}

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--diagnose") {
        if let Err(e) = diagnose() {
            eprintln!("diagnose failed: {e}");
        }
        return Ok(());
    }
    let smoke_run =
        args.iter().any(|a| a == "--smoke") || std::env::var_os("PROTON_SYNC_SMOKE").is_some();

    if smoke_run {
        // Upgrade-verification boot: prove the binary starts and can read its
        // own configuration. No window, no lock - the staged image runs this
        // on a machine that already has the live app open.
        if let Err(e) = smoke() {
            eprintln!("smoke failed: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    let p = paths::resolve();
    let _ = std::fs::create_dir_all(&p.state_dir);

    // Single instance: the QLockFile equivalent. A second launch pokes the
    // running window to the front instead of opening a duplicate.
    use fs2::FileExt;
    let lock = match std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&p.lock_file)
    {
        Ok(f) => f,
        Err(e) => {
            eprintln!("lock file: {e}");
            std::process::exit(1);
        }
    };
    if lock.try_lock_exclusive().is_err() {
        let show = p.state_dir.join("show-request");
        let _ = std::fs::write(&show, b"1");
        // The owner may instead be in the middle of quitting. Wait briefly
        // for the lock: if it is released, this launch must start normally
        // rather than exit and leave no instance running. A healthy owner
        // removes `show-request` within its poll interval, which ends the
        // wait immediately.
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut took = false;
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
            if lock.try_lock_exclusive().is_ok() {
                took = true;
                break;
            }
            if !show.exists() {
                break;
            }
        }
        if !took {
            eprintln!("Proton Drive is already running - bringing it to the front.");
            return Ok(());
        }
        let _ = std::fs::remove_file(&show);
        eprintln!("The previous instance exited; starting normally.");
    }

    // Update recovery: the marker decides whether this start is the freshly
    // swapped-in version (health is confirmed once its window is up) or a
    // version that never came up (its .old backup is restored and started).
    // The APPIMAGE env is authoritative even when its file is missing - that
    // is exactly the interrupted-swap case recovery exists for.
    if p.appimage.is_some() || std::env::var_os("APPIMAGE").is_some() {
        let (state, relaunch) = release::update_recovery_check(&p);
        if state == "rolled-back" {
            if let Some(target) = relaunch {
                spawn_detached(&target);
            }
            return Ok(());
        }
    }

    let prefs = prefs::Prefs::load(&p.prefs_file);

    // Wayland compositors cannot hide, focus, or un-minimize a window
    // client-side (winit no-ops all three on Wayland), which would break
    // close-to-tray; the Qt app could do it because QWindow::hide() unmaps
    // the surface. When XWayland is available, put the window on X11, where
    // hide/show/focus/minimize behave exactly like they did under Qt.
    // PROTON_SYNC_WAYLAND=1 opts back into native Wayland (e.g. for crisp
    // HiDPI) at the cost of window hiding.
    let saved_wayland = {
        let wl = std::env::var_os("WAYLAND_DISPLAY");
        let force_wayland = std::env::var_os("PROTON_SYNC_WAYLAND").is_some();
        if wl.is_some() && !force_wayland && std::env::var_os("DISPLAY").is_some() {
            // SAFETY: still single-threaded - nothing else can be reading the
            // environment during this call; ui::App::new restores the variable
            // once the window exists.
            unsafe { std::env::remove_var("WAYLAND_DISPLAY") };
            wl
        } else {
            None
        }
    };

    let icon = ui::decode_png(include_bytes!("icon-256.png")).map(|(w, h, rgba)| egui::IconData {
        rgba,
        width: w as u32,
        height: h as u32,
    });

    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title("Proton Drive")
        .with_inner_size([prefs.window_w as f32, prefs.window_h as f32])
        .with_min_inner_size([720.0, 400.0])
        // Start quietly in the tray without a window flash (honored on X11).
        .with_visible(!prefs.start_in_tray());
    if let Some(ic) = icon {
        viewport = viewport.with_icon(ic);
    }
    if prefs.always_on_top() {
        // Best effort: X11 honors this, Wayland compositors may not.
        viewport = viewport.with_always_on_top();
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    let relaunch_slot: std::sync::Arc<std::sync::Mutex<Option<PathBuf>>> =
        std::sync::Arc::default();
    let slot = relaunch_slot.clone();
    let res = eframe::run_native(
        "Proton Drive",
        options,
        Box::new(move |cc| ui::App::new(cc, lock, saved_wayland, slot)),
    );
    // A GUI-owned engine belongs to this window: closing the window (not
    // hiding to the tray) must stop it. The systemd service is independent
    // and is left running.
    service::shutdown();
    // After an in-app update the freshly swapped-in version starts here: the
    // App (and with it the single-instance lock) is dropped by now, so it
    // takes the lock instead of reporting "Already running".
    if let Some(target) = relaunch_slot.lock().unwrap().take() {
        spawn_detached(&target);
    }
    res
}

/// Start a process detached from this session (like Qt's
/// `Popen(..., start_new_session=True)`); it outlives us as an orphan.
fn spawn_detached(target: &Path) {
    let _ = std::process::Command::new(target)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// Read-only support report for a machine that will not sync. It never opens
/// a window or takes the GUI lock; every call is bounded.
/// Write to the diagnostic sink without panicking when the reader closes the
/// pipe (e.g. `--diagnose | head`). Rust's `println!` aborts on EPIPE.
macro_rules! diag {
    ($out:expr, $($arg:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!($out, $($arg)*);
    }};
}

fn diagnose() -> Result<(), String> {
    use std::net::{TcpStream, ToSocketAddrs};
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    let p = paths::resolve();
    let prefs = prefs::Prefs::load(&p.prefs_file);
    let c = conf::read(&p.conf_file);
    let remote = c
        .get("PROTON_SYNC_REMOTE")
        .filter(|s| !s.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| "/my-files".to_string());
    let local = c
        .get("PROTON_SYNC_LOCAL")
        .filter(|s| !s.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| "~/ProtonDrive".to_string());
    let h = health::read(&p.health_file);

    diag!(out, "Proton Drive for Linux {GUI_VERSION}");
    diag!(out, "Platform {} {}", std::env::consts::OS, std::env::consts::ARCH);
    diag!(out, "Config {}", p.conf_file.display());
    diag!(out, "State {}", p.state_dir.display());
    diag!(out, "Local {local} (exists: {})", Path::new(&local).is_dir());
    diag!(out, "Remote {remote}");
    diag!(out, 
        "Theme {} · interface {}%",
        prefs.theme().as_str(),
        prefs.ui_scale().percent()
    );
    diag!(out, 
        "Service active={} enabled={} supervisor={}",
        service::is_active(),
        service::is_enabled(),
        service::uses_supervisor()
    );

    match cli::resolve_cli() {
        Some(cli) => {
            diag!(out, "CLI {}", cli.display());
            diag!(out, "CLI version {}", cli::cli_version(&cli));
            diag!(out, "Session probe (fs info {remote}):");
            match cli::probe_session(&cli, &remote) {
                Ok(probe) => diag!(
                    out,
                    "  ok: {}",
                    probe.lines().next().unwrap_or("").trim()
                ),
                Err(e) => diag!(out, "  failed: {e}"),
            }
        }
        None => diag!(out, "CLI not found"),
    }

    diag!(out, "Network drive-api.proton.me:443 ...");
    match "drive-api.proton.me:443".to_socket_addrs().ok().and_then(|mut a| a.next()) {
        Some(addr) => match TcpStream::connect_timeout(&addr, Duration::from_secs(5)) {
            Ok(_) => diag!(out, "connected ({addr})"),
            Err(e) => diag!(out, "failed ({addr}): {e}"),
        },
        None => diag!(out, "DNS resolution failed"),
    }

    diag!(out, "\nhealth.json:");
    diag!(out, 
        "{}",
        serde_json::to_string_pretty(&h).unwrap_or_else(|_| "unreadable".to_string())
    );
    diag!(out, "\nRecent engine log:");
    for line in service::journal_lines(80) {
        diag!(out, "{line}");
    }
    let _ = std::io::Write::flush(&mut out);
    Ok(())
}

fn smoke() -> Result<(), String> {
    let p = paths::resolve();
    let _ = std::fs::create_dir_all(&p.state_dir);
    let prefs = prefs::Prefs::load(&p.prefs_file);
    let c = conf::read(&p.conf_file);
    let _extras = conf::read_extras(&p.conf_file);
    let local = c.get("PROTON_SYNC_LOCAL").cloned().unwrap_or_default();
    let health = health::read(&p.health_file);
    let cli_ok = cli::resolve_cli().map(|x| x.is_file()).unwrap_or(false);
    println!(
        "smoke ok: gui {GUI_VERSION}, conf keys {}, local {local}, cli {cli_ok}, \
         health status '{}', theme {} (system {})",
        c.len(),
        health.status,
        prefs.theme().as_str(),
        system_theme::detect().as_str()
    );
    if local.is_empty() || Path::new(&local).is_absolute() {
        Ok(())
    } else {
        Err("local sync path is not absolute".to_string())
    }
}
