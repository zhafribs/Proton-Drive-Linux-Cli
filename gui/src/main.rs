//! Proton Drive for Linux - desktop GUI.
//!
//! One static binary replaces the bundled Python/Qt stack for the desktop
//! app; the background sync engine stays the proven Python script, and every
//! transfer still goes through the official Proton Drive CLI (unchanged,
//! MIT, bundled as-is).

mod cli;
mod conf;
mod desktop;
mod exec;
mod health;
mod journal;
mod notify;
mod paths;
mod prefs;
mod release;
mod service;
mod theme;
mod ui;
mod util;

use eframe::egui;
use std::path::{Path, PathBuf};

/// The GUI's own version (the release tag is `v<CLI>-gui<GUI>`).
pub const GUI_VERSION: &str = "3.14.0";

/// The systemd user unit path (shared with `service.rs`).
pub fn service_unit_path() -> PathBuf {
    paths::resolve().unit_file
}

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let smoke_run = args.iter().any(|a| a == "--smoke")
        || std::env::var_os("PROTON_SYNC_SMOKE").is_some();

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
        let _ = std::fs::write(p.state_dir.join("show-request"), b"1");
        eprintln!("Proton Drive is already running - bringing it to the front.");
        return Ok(());
    }

    let prefs = prefs::Prefs::load(&p.prefs_file);
    let icon = ui::decode_png(include_bytes!("icon-256.png")).map(|(w, h, rgba)| {
        egui::IconData { rgba, width: w as u32, height: h as u32 }
    });

    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title("Proton Drive")
        .with_inner_size([prefs.window_w as f32, prefs.window_h as f32])
        .with_min_inner_size([920.0, 540.0]);
    if let Some(ic) = icon {
        viewport = viewport.with_icon(ic);
    }
    if prefs.always_on_top() {
        // Best effort: X11 honors this, Wayland compositors may not.
        viewport = viewport.with_always_on_top();
    }

    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native(
        "Proton Drive",
        options,
        Box::new(move |cc| ui::App::new(cc, lock)),
    )
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
         health status '{}', theme {}",
        c.len(),
        health.status,
        prefs.theme().as_str()
    );
    if Path::new(&local).is_absolute() {
        Ok(())
    } else {
        Err("local sync path missing".to_string())
    }
}
