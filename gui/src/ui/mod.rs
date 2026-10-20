//! The application: shell (sidebar + pages), background message plumbing,
//! dialogs, and notification triggers - ported from the PyQt GUI's window
//! logic with the same strings and behaviors.

pub mod about;
pub mod activity;
pub mod cloud;
pub mod home;
pub mod settings;
pub mod widgets;

use crate::{cli, conf, health, journal, notify, paths::Paths, prefs, release, service, theme, tray};
use eframe::egui;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Home,
    Cloud,
    Activity,
    Settings,
    About,
}

/// Messages from the poller/worker threads to the UI.
#[derive(Clone)]
pub enum Back {
    Poll { health: health::Health, active: bool, enabled: bool },
    Journal(Vec<String>),
    Identity(cli::Identity),
    Stats { files: u64, bytes: u64 },
    ShowWindow,
    SaveHint(String),
    /// Release check: (release, update available, installed CLI version).
    Update(Result<(release::Release, bool, String), String>),
    /// Download + swap finished; Ok is the new AppImage's path.
    UpdateInstalled(Result<String, String>),
    /// Selective-sync folder listing (parent rel "" = the root level).
    SelFolders { parent: String, folders: Vec<(String, String)> },
    /// Whether the sync unit is enabled at login (slow systemctl call).
    UnitEnabled(bool),
    /// Cloud browser: a folder listing landed (key + serial guard staleness).
    CloudEntries { key: String, serial: u64, result: Result<Vec<cli::RemoteEntry>, String> },
    /// A sidebar tree listing (never touches the visible folder).
    TreeEntries { key: String, result: Result<Vec<cli::RemoteEntry>, String> },
    /// Cloud browser: a CLI mutation finished (refresh follows).
    CloudMutated { ok: bool, err: String, done: String, notify: Option<(String, String)> },
    /// Cloud browser: a download finished.
    CloudDownload { ok: bool, err: String, name: String, n: usize },
    /// Cloud browser: a file dialog returned (None = cancelled).
    CloudPicked(Option<Picked>),
    /// Cloud browser: the move-to picker's folder listing landed.
    PickerFolders { parent: String, folders: Vec<(String, String)> },
    /// Tray menu/activation actions (sent from the ksni service thread).
    TrayToggle,
    TrayQuit,
    TraySyncNow,
    TrayOpenFolder,
    TrayOpenPath(String),
    TrayPause(Option<u64>),
    TrayResume,
}

/// A file-dialog answer from the Cloud page (portal/zenity/kdialog).
#[derive(Clone, Debug)]
pub enum Picked {
    UploadFiles(Vec<PathBuf>),
    UploadFolder(PathBuf),
    DownloadSave { rpath: String, dest: PathBuf },
    DownloadDir { rpaths: Vec<String>, dest: PathBuf, n: usize },
}

#[derive(Clone, Debug)]
pub enum Dialog {
    Info { title: String, body: String },
    ConfirmSave,
    ConfirmLogout,
    PauseMenu,
    DirBrowse { cwd: PathBuf },
    /// Text input (Cloud: rename / new folder); the buffer lives on App.
    Input { title: String, label: String, intent: InputIntent },
    ConfirmTrash { n: usize },
    /// The "Move to..." cloud folder picker.
    CloudMove,
}

#[derive(Clone, Debug)]
pub enum InputIntent {
    CloudRename(String), // old rel
    CloudNewFolder,
}

/// What a dialog close/confirm should do once the UI closure returns.
#[derive(Debug)]
enum DlgAct {
    Close,
    Save,
    Logout,
    Pause(Option<u64>),
    BrowseGo(PathBuf),
    BrowseSet(PathBuf),
    CloudRename { old: String, name: String },
    NewFolder(String),
    TrashConfirm,
    MoveSel(String), // picker row clicked
    MovePick(String), // picker confirmed: move selection here
}

#[derive(Default)]
pub struct UpdateUi {
    pub label: String,
    pub available: Option<release::Release>,
    pub checking: bool,
    pub downloading: bool,
    pub quit_at: Option<std::time::Instant>,
    /// automatic check schedule (startup + every 6h, updates_auto pref)
    pub next_check: Option<std::time::Instant>,
}

pub struct App {
    pub paths: Paths,
    pub prefs: prefs::Prefs,
    pub conf: BTreeMap<String, String>,
    pub conf_extras: Vec<String>,
    pub tx: mpsc::Sender<Back>,
    pub rx: mpsc::Receiver<Back>,
    pub _lock: std::fs::File,

    pub health: health::Health,
    pub active: bool,
    pub enabled: bool,
    pub page: Page,

    pub identity: Option<cli::Identity>,
    pub stats: Option<(u64, u64)>,

    pub draft: settings::Draft,
    pub changes: Vec<journal::ChangeRow>,
    pub log_lines: Vec<String>,
    pub act: activity::ActivityState,
    pub cloud: cloud::CloudState,
    /// Buffer of the open text-input dialog (rename / new folder).
    pub input_buf: String,
    pub input_focus: bool,

    pub cool: notify::Cooldowns,
    pub last_seen: Option<chrono::DateTime<chrono::FixedOffset>>,
    pub prev_hstatus: String,
    pub tray_warned: bool,

    pub dialog: Option<Dialog>,
    pub save_hint: String,
    pub sub_override: Option<(String, f64)>,
    pub toast: Option<(String, f64)>,
    pub upd: UpdateUi,

    pub hidden: bool,
    pub hide_on_start: bool,
    pub quitting: bool,
    pub style_dirty: bool,
    pub icon_tex: Option<egui::TextureHandle>,
    /// The system tray (None when the desktop has no tray host).
    pub tray: Option<ksni::blocking::Handle<tray::AppTray>>,
    /// Running on the Wayland backend (no working client-side hide).
    pub wayland_fallback: bool,
    /// Slot main() reads after the event loop ends: the freshly swapped-in
    /// AppImage to relaunch once the single-instance lock is free.
    pub relaunch_slot: std::sync::Arc<std::sync::Mutex<Option<std::path::PathBuf>>>,
}

/// "ProtonDrive" or "~/stuff" -> absolute path (the GUI only deals in
/// absolute paths, but be forgiving).
pub fn expand_user(s: &str) -> PathBuf {
    let s = s.trim();
    if s == "~" {
        return dirs::home_dir().unwrap_or_else(|| "/".into());
    }
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(s)
}

/// Decode a PNG into (width, height, RGBA8) - used for the window and menu
/// icon and the sidebar texture. RGB sources get an opaque alpha added.
pub fn decode_png(bytes: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let rgba = match info.color_type {
        png::ColorType::Rgba => {
            buf.truncate(w * h * 4);
            buf
        }
        png::ColorType::Rgb => {
            let mut out = Vec::with_capacity(w * h * 4);
            for px in buf[..w * h * 3].chunks_exact(3) {
                out.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
            out
        }
        _ => return None,
    };
    Some((w, h, rgba))
}

fn spawn_poller(tx: mpsc::Sender<Back>, paths: Paths) {
    std::thread::spawn(move || {
        let mut tick: u64 = 0;
        loop {
            let h = health::read(&paths.health_file);
            let active = service::is_active();
            let enabled = service::is_enabled();
            if tx.send(Back::Poll { health: h, active, enabled }).is_err() {
                break;
            }
            if tx.send(Back::Journal(service::journal_lines(600))).is_err() {
                break;
            }
            if tick % 7 == 0 {
                let c = conf::read(&paths.conf_file);
                let local = c.get("PROTON_SYNC_LOCAL").cloned().unwrap_or_default();
                let (files, bytes) = service::folder_stats(&expand_user(&local).to_string_lossy());
                if tx.send(Back::Stats { files, bytes }).is_err() {
                    break;
                }
            }
            let show = paths.state_dir.join("show-request");
            if show.exists() {
                let _ = std::fs::remove_file(&show);
                if tx.send(Back::ShowWindow).is_err() {
                    break;
                }
            }
            std::thread::sleep(Duration::from_secs(3));
            tick = tick.wrapping_add(1);
        }
    });
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        lock: std::fs::File,
        saved_wayland: Option<std::ffi::OsString>,
        relaunch_slot: std::sync::Arc<std::sync::Mutex<Option<std::path::PathBuf>>>,
    ) -> Result<Box<dyn eframe::App>, Box<dyn std::error::Error + Send + Sync>> {
        // Window already created on the X11/XWayland backend when we removed
        // WAYLAND_DISPLAY in main() - restore the session env so child
        // processes (the login terminal, xdg-open) see it again. Pure-Wayland
        // sessions (no XWayland) keep the Wayland backend: hiding then falls
        // back to compositor minimize, which not all compositors honor.
        let wayland_fallback =
            saved_wayland.is_none() && std::env::var_os("WAYLAND_DISPLAY").is_some();
        if let Some(wl) = saved_wayland {
            // SAFETY: the window exists and no thread spawned so far reads
            // WAYLAND_DISPLAY; later readers are child processes, which get
            // the restored session value.
            unsafe { std::env::set_var("WAYLAND_DISPLAY", wl) };
        }
        let paths = crate::paths::resolve();
        let _ = std::fs::create_dir_all(&paths.state_dir);
        // The window exists: if this is the first launch after an update
        // swap, confirm health - drop the rollback marker and its .old
        // backup. Running as an AppImage only, so dev runs never consume a
        // real update's marker.
        if paths.appimage.is_some() {
            release::update_confirm_health(&paths);
        }
        let prefs = prefs::Prefs::load(&paths.prefs_file);
        let upd_next_check = prefs.updates_auto().then(|| {
            std::time::Instant::now() + Duration::from_secs(5)
        });
        theme::apply(&cc.egui_ctx, &prefs.theme());
        theme::force(&cc.egui_ctx, &prefs.theme());
        let conf = conf::read(&paths.conf_file);
        let conf_extras = conf::read_extras(&paths.conf_file);
        let draft = settings::Draft::load(&conf, &conf_extras, &prefs, &paths);
        let (tx, rx) = mpsc::channel();

        spawn_poller(tx.clone(), paths.clone());

        // Identity (version + sign-in) as soon as the window exists.
        let tx_id = tx.clone();
        std::thread::spawn(move || {
            let ident = match cli::resolve_cli() {
                Some(c) => cli::fetch_identity(&c),
                None => cli::Identity {
                    version: "?".into(),
                    error: Some("not found".into()),
                    ..Default::default()
                },
            };
            let _ = tx_id.send(Back::Identity(ident));
        });

        // Keep the installed unit pointing at this copy (Qt parity).
        let appimage = paths.appimage.clone();
        std::thread::spawn(move || {
            let _ = service::ensure_service_unit(appimage.as_deref(), false);
        });

        // Login-enable state of the unit (slow systemctl call, off-thread).
        let tx_en = tx.clone();
        std::thread::spawn(move || {
            let _ = tx_en.send(Back::UnitEnabled(service::is_enabled()));
        });

        // Selective-sync roots load in the background at startup (Qt parity).
        let mut draft = draft;
        if draft.loading.insert(String::new()) {
            let remote = draft.remote.clone();
            let tx_sel = tx.clone();
            std::thread::spawn(move || {
                let folders = cli::fetch_remote_folders(&remote, "");
                let _ = tx_sel.send(Back::SelFolders {
                    parent: String::new(),
                    folders,
                });
            });
        }

        // Decode the app icon once: sidebar texture + tray base.
        let (icon_rgba, icon_w) = decode_png(include_bytes!("../icon-64.png"))
            .map(|(w, _h, px)| (px, w))
            .unwrap_or_else(|| (Vec::new(), 0));
        let icon_tex = if icon_rgba.is_empty() {
            None
        } else {
            Some(cc.egui_ctx.load_texture(
                "app-icon",
                egui::ColorImage::from_rgba_unmultiplied([icon_w, icon_w], &icon_rgba),
                egui::TextureOptions::LINEAR,
            ))
        };

        // System tray (Qt built it whenever a tray host existed).
        let tray = tray::spawn(tx.clone(), icon_rgba, icon_w);

        Ok(Box::new(App {
            hide_on_start: prefs.start_in_tray(),
            prefs,
            paths,
            conf,
            conf_extras,
            tx,
            rx,
            _lock: lock,
            health: health::Health::default(),
            active: false,
            enabled: false,
            page: Page::Home,

            identity: None,
            stats: None,
            draft,
            changes: Vec::new(),
            log_lines: Vec::new(),
            act: activity::ActivityState::default(),
            cloud: cloud::CloudState::default(),
            input_buf: String::new(),
            input_focus: false,
            cool: notify::Cooldowns::default(),
            last_seen: Some(chrono::Utc::now().into()),
            prev_hstatus: String::new(),
            tray_warned: false,
            dialog: None,
            save_hint: String::new(),
            sub_override: None,
            toast: None,
            upd: UpdateUi {
                next_check: upd_next_check,
                ..Default::default()
            },
            hidden: false,
            quitting: false,
            style_dirty: false,
            icon_tex,
            tray,
            wayland_fallback,
            relaunch_slot,
        }))
    }

    // ------------------------------------------------------------- status

    pub fn status_key(&self) -> &'static str {
        health::map_status(self.active, &self.health.status).key()
    }

    /// Qt put the STATUS_TEXT title on the side status label and the tray
    /// tooltip ("You're up to date", not a short chip).
    pub fn side_label(&self) -> &'static str {
        health::status_text(health::map_status(self.active, &self.health.status)).0
    }

    pub fn logged_in(&self) -> bool {
        self.identity.as_ref().is_some_and(|i| i.logged_in)
    }

    pub fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), self.now()));
    }

    fn now(&self) -> f64 {
        // ctx.time is captured in update; Approximation via Instant is fine
        // for a 4-second toast lifetime - keep a monotonic clock here.
        static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        START.get_or_init(Instant::now).elapsed().as_secs_f64()
    }

    // ------------------------------------------------------------- actions

    pub fn action_sync_now(&mut self) {
        if !self.active {
            self.dialog = Some(Dialog::Info {
                title: "Sync is paused".into(),
                body: "Syncing is paused. Press Resume first.".into(),
            });
            return;
        }
        let _ = std::fs::create_dir_all(&self.paths.state_dir);
        let _ = std::fs::write(&self.paths.wake_file, b"");
        self.sub_override = Some(("On it - checking for new changes...".into(), self.now() + 8.0));
    }

    pub fn action_open_folder(&self) {
        let root = expand_user(self.conf.get("PROTON_SYNC_LOCAL").map(String::as_str).unwrap_or(""));
        let _ = std::fs::create_dir_all(&root);
        std::thread::spawn(move || {
            let _ = std::process::Command::new("xdg-open")
                .arg(&root)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
        });
    }

    /// Qt `_open_sync_path`: open a change's parent folder (falling back to
    /// the sync root when the target is gone).
    pub fn action_open_sync_path(&self, rel: &str) {
        let root = expand_user(self.conf.get("PROTON_SYNC_LOCAL").map(String::as_str).unwrap_or(""));
        let rel = rel.trim_matches('/');
        let target = if rel.is_empty() { root.clone() } else { root.join(rel) };
        let dir = if target.is_dir() { target } else { root };
        std::thread::spawn(move || {
            let _ = std::process::Command::new("xdg-open")
                .arg(&dir)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
        });
    }

    /// Push status/tooltip/menu state to the tray (Qt refreshed the icon,
    /// tooltip and action enablement on every status refresh).
    fn tray_sync(&self) {
        let Some(handle) = &self.tray else { return };
        let status_key = self.status_key().to_string();
        let title = self.side_label().to_string();
        let active = self.active;
        let recent = tray::recent_labels(&self.changes);
        handle.update(move |t| t.apply(status_key, title, active, recent));
    }

    pub fn action_login(&mut self) {
        let Some(cli) = cli::resolve_cli() else {
            self.dialog = Some(Dialog::Info {
                title: "Not found".into(),
                body: "The Proton command-line tool is missing. Reinstall the app and try again."
                    .into(),
            });
            return;
        };
        match cli::find_terminal() {
            Some((exe, args)) => {
                let script = cli::login_script(&cli);
                let mut a: Vec<String> = args;
                a.extend(["sh".into(), "-c".into(), script]);
                std::thread::spawn(move || {
                    let _ = std::process::Command::new(exe)
                        .args(&a)
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn();
                });
                // Qt re-checked identity 10 s after opening the terminal.
                let tx = self.tx.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(10));
                    if let Some(c) = cli::resolve_cli() {
                        let _ = tx.send(Back::Identity(cli::fetch_identity(&c)));
                    }
                });
            }
            None => {
                self.dialog = Some(Dialog::Info {
                    title: "Sign in".into(),
                    body: "A terminal window could not be opened automatically.\n\nPlease open \
                           a terminal yourself and run:\n\n    proton-drive auth login"
                        .into(),
                });
            }
        }
    }

    pub fn action_logout_confirmed(&self) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            if let Some(c) = cli::resolve_cli() {
                cli::logout(&c);
                let _ = tx.send(Back::Identity(cli::fetch_identity(&c)));
            }
        });
    }

    pub fn action_pause(&self, secs: Option<u64>) {
        std::thread::spawn(move || {
            let _ = service::pause_for(secs);
        });
    }

    pub fn action_resume(&self) {
        let appimage = self.paths.appimage.clone();
        std::thread::spawn(move || {
            let _ = service::ensure_service_unit(appimage.as_deref(), false);
            let _ = service::resume();
        });
    }

    pub fn action_restart_service(&mut self) {
        let appimage = self.paths.appimage.clone();
        std::thread::spawn(move || {
            let _ = service::ensure_service_unit(appimage.as_deref(), false);
            let _ = service::restart();
        });
        self.toast("Syncing restarted.");
    }

    pub fn action_check_updates(&mut self) {
        if self.upd.checking {
            return;
        }
        self.upd.checking = true;
        self.upd.label = "Checking for updates...".into();
        let paths = self.paths.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            // The installed CLI version is read HERE, in the worker: the
            // identity fetch waits for a full cloud listing (up to two
            // minutes), so at the 5s startup check it is still empty and a
            // bogus "you're up to date" was reported (Qt noted the same).
            let cur = cli::resolve_cli()
                .and_then(|c| crate::ui::util_semver(&cli::cli_version(&c)))
                .unwrap_or_default();
            let r = match release::check(&paths) {
                Err(e) => Err(e),
                Ok((rel, _cached)) => {
                    let have = release::has_update(&cur, &rel);
                    Ok((rel, have, cur))
                }
            };
            let _ = tx.send(Back::Update(r));
        });
    }

    /// Qt's "Download and restart": download + checksum + staged start-up
    /// check + swap, then quit and let main() relaunch the new build.
    pub fn action_download_update(&mut self) {
        let Some(rel) = self.upd.available.clone() else { return };
        if self.upd.downloading {
            return;
        }
        self.upd.downloading = true;
        self.upd.label = "Downloading and verifying the update...".into();
        let tx = self.tx.clone();
        // x86_64 / aarch64, the release filenames' arch (uname -m).
        let arch = std::env::consts::ARCH;
        std::thread::spawn(move || {
            let r = release::download_update(&rel, arch)
                .and_then(|dl| release::install_update_and_restart(&dl, &rel.tag))
                .map(|p| p.to_string_lossy().into_owned());
            let _ = tx.send(Back::UpdateInstalled(r));
        });
    }

    pub fn open_url(url: &str) {
        let u = url.to_string();
        std::thread::spawn(move || {
            let _ = std::process::Command::new("xdg-open")
                .arg(&u)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
        });
    }

    // ------------------------------------------------------------- draining

    fn drain(&mut self, ctx: &egui::Context) {
        while let Ok(back) = self.rx.try_recv() {
            match back {
                Back::Poll { health, active, enabled } => {
                    self.on_poll(health, active, enabled);
                    self.tray_sync();
                }
                Back::Journal(lines) => {
                    self.on_journal(lines);
                    self.tray_sync();
                }
                Back::Identity(id) => {
                    self.identity = Some(id);
                }
                Back::Stats { files, bytes } => {
                    self.stats = Some((files, bytes));
                }
                Back::ShowWindow => {
                    self.hidden = false;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                Back::SaveHint(h) => self.save_hint = h,
                Back::SelFolders { parent, folders } => {
                    let selective = self.draft.selective;
                    self.draft.on_folders(&parent, folders);
                    if parent.is_empty() && selective {
                        self.draft.sel_hint = self
                            .draft
                            .children
                            .get("")
                            .map(|c| format!("{} folders", c.len()))
                            .unwrap_or_default();
                    }
                }
                Back::UnitEnabled(b) => self.draft.autostart = b,
                Back::CloudEntries { key, serial, result } => {
                    cloud::on_entries(self, key, serial, result)
                }
                Back::TreeEntries { key, result } => cloud::on_tree_entries(self, key, result),
                Back::CloudMutated { ok, err, done, notify } => {
                    cloud::on_mutated(self, ok, err, done, notify)
                }
                Back::CloudDownload { ok, err, name, n } => {
                    cloud::on_downloaded(self, ok, err, name, n)
                }
                Back::CloudPicked(p) => cloud::on_picked(self, p),
                Back::PickerFolders { parent, folders } => {
                    cloud::on_picker_folders(self, parent, folders)
                }
                Back::TrayToggle => {
                    let minimized =
                        ctx.input(|i| i.viewport().minimized.unwrap_or(false));
                    if self.hidden || minimized {
                        self.hidden = false;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    } else {
                        self.hide_to_tray(ctx);
                    }
                }
                Back::TrayQuit => self.quit(ctx),
                Back::TraySyncNow => self.action_sync_now(),
                Back::TrayOpenFolder => self.action_open_folder(),
                Back::TrayOpenPath(rel) => self.action_open_sync_path(&rel),
                Back::TrayPause(secs) => self.action_pause(secs),
                Back::TrayResume => self.action_resume(),
                Back::Update(r) => {
                    self.upd.checking = false;
                    match r {
                        Ok((rel, have, cur)) => {
                            if have {
                                self.upd.label = format!(
                                    "Update available: CLI {} + GUI {} (you have {} + {}).",
                                    rel.cli, rel.gui, cur, crate::GUI_VERSION
                                );
                                self.upd.available = Some(rel.clone());
                                if self.tray.is_some() && self.cool.cool_ok("update", 6 * 3600) {
                                    notify::send(
                                        "Update available",
                                        &format!(
                                            "Proton Drive {} is ready to install.",
                                            rel.tag
                                        ),
                                    );
                                }
                            } else {
                                self.upd.label = format!(
                                    "You're up to date (latest release: {}, {}).",
                                    rel.tag, rel.published
                                );
                                self.upd.available = None;
                            }
                        }
                        Err(e) => {
                            let _ = e;
                            self.upd.label =
                                "Could not check for updates right now.".to_string();
                            self.upd.available = None;
                        }
                    }
                }
                Back::UpdateInstalled(r) => {
                    self.upd.downloading = false;
                    match r {
                        Ok(target) => {
                            self.upd.label =
                                "Updated - the new version is starting...".to_string();
                            // main() starts it once the lock is free.
                            *self.relaunch_slot.lock().unwrap() =
                                Some(std::path::PathBuf::from(target));
                            self.upd.quit_at =
                                Some(std::time::Instant::now() + Duration::from_millis(1500));
                        }
                        Err(e) => {
                            self.upd.label = format!("Update failed: {e}");
                        }
                    }
                }
            }
        }
    }

    fn on_poll(&mut self, health: health::Health, active: bool, enabled: bool) {
        // "All done" transition notification (Qt parity).
        if self.prev_hstatus == "syncing"
            && health.status == "ok"
            && (health.uploaded > 0 || health.downloaded > 0)
            && self.prefs.notify_complete()
            && self.cool.cool_ok("done", 60)
        {
            notify::send(
                "All done",
                &format!(
                    "{} file(s) uploaded, {} downloaded.",
                    health.uploaded, health.downloaded
                ),
            );
        }
        self.prev_hstatus = health.status.clone();
        self.health = health;
        self.active = active;
        self.enabled = enabled;
    }

    fn on_journal(&mut self, lines: Vec<String>) {
        // Notifications: walk new lines chronologically (Qt parity).
        for ln in &lines {
            let Some(ts) = journal::ts(ln) else { continue };
            if self.last_seen.is_some_and(|last| ts <= last) {
                continue;
            }
            self.last_seen = Some(ts);
            let body = journal::body(ln);
            let name = journal::change_name(body);
            if body.contains("CONFLICT")
                && self.prefs.notify_conflicts()
                && self.cool.cool_ok("conflict", 30)
            {
                notify::send(
                    "Two versions were kept",
                    &format!("'{name}' changed in two places - both copies are safe."),
                );
            } else if (body.contains("ERROR") || body.contains("FAILED"))
                && self.prefs.notify_errors()
                && self.cool.cool_ok("error", 30)
            {
                let short: String = body.chars().take(150).collect();
                notify::send("Sync problem", &short);
            }
        }

        self.log_lines = if lines.len() > 600 {
            lines[lines.len() - 600..].to_vec()
        } else {
            lines.clone()
        };
        let mut rows = journal::parse_changes(&lines);
        if rows.len() > 300 {
            rows = rows.split_off(rows.len() - 300);
        }
        self.changes = rows;
    }

    // ------------------------------------------------------------- window

    fn remember_window_size(&mut self, ctx: &egui::Context) {
        let size = ctx.input(|i| i.viewport().inner_rect.map(|r| r.size()));
        if let Some(sz) = size {
            let (w, h) = (sz.x as u32, sz.y as u32);
            if w != self.prefs.window_w || h != self.prefs.window_h {
                self.prefs.set_window_size(w, h);
            }
        }
    }

    fn hide_window(&mut self, ctx: &egui::Context) {
        self.hidden = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        if !self.tray_warned {
            self.tray_warned = true;
            notify::send(
                "Proton Drive",
                "Still running - syncing continues in the background. Right-click the tray \
                 icon to quit.",
            );
        }
    }

    /// Hide the window the way Qt's QWindow::hide() did: unmap it (real on
    /// X11/XWayland). On the Wayland backend the unmap is a winit no-op, so
    /// fall back to asking the compositor to minimize instead.
    fn hide_to_tray(&mut self, ctx: &egui::Context) {
        self.remember_window_size(ctx);
        if self.wayland_fallback {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
        self.hide_window(ctx);
    }

    fn quit(&mut self, ctx: &egui::Context) {
        self.quitting = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    // ------------------------------------------------------------- shell

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        // Two-tone wordmark like Proton Drive's own logo: dark name + accent
        // product tag.
        ui.horizontal(|ui| {
            if let Some(tex) = &self.icon_tex {
                ui.add(egui::Image::new((tex.id(), egui::vec2(30.0, 30.0))));
            }
            ui.label(egui::RichText::new("Proton Drive").strong().size(16.5));
            ui.label(
                egui::RichText::new("CLI").strong()
                    .size(16.5)
                    .color(theme::accent(ui.visuals())),
            );
        });
        ui.add_space(14.0);

        // Proton's sidebar button: "+ New" opens the create/upload menu.
        cloud::new_button(self, ui);
        ui.add_space(10.0);

        // Home, Activity, My files (expandable folder tree), Settings,
        // Trash, About; long trees scroll while the footer stays anchored
        // at the bottom.
        egui::ScrollArea::vertical()
            .id_salt("nav-scroll")
            .auto_shrink([false, false])
            .max_height((ui.available_height() - 124.0).max(140.0))
            .show(ui, |ui| cloud::nav_ui(self, ui));

        ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
            ui.add_space(4.0);
            self.account_row(ui);
            ui.add_space(8.0);
            // Status card, like the Proton sidebar's footer block.
            let fill = theme::surface(ui.visuals());
            let stroke = ui.visuals().widgets.noninteractive.bg_stroke;
            egui::Frame::new()
                .fill(fill)
                .stroke(stroke)
                .corner_radius(egui::CornerRadius::same(10))
                .inner_margin(egui::Margin::symmetric(12, 9))
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let key = self.status_key();
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                        ui.painter().circle_filled(rect.center(), 5.0, theme::status_color(key));
                        ui.label(egui::RichText::new(self.side_label()).size(13.0));
                    });
                    ui.add_space(2.0);
                    ui.label(egui::RichText::new("Unofficial community app").size(11.5).weak());
                });
        });
    }

    /// The pane's account row (Proton Drive's sidebar footer): avatar +
    /// identity at the very bottom; the pages themselves live as rows in the
    /// nav list above.
    fn account_row(&mut self, ui: &mut egui::Ui) {
        let email = self.identity.as_ref().and_then(|i| i.email.clone());
        let label = match (&self.identity, &email) {
            (None, _) => "Checking your account...",
            (Some(_), Some(e)) => e.as_str(),
            (Some(_), None) => "You're not signed in yet",
        };
        let (rect, _resp) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 34.0), egui::Sense::hover());
        // Accent initial circle, like Home's account card.
        let c = egui::pos2(rect.left() + 14.0, rect.center().y);
        ui.painter().circle_filled(c, 11.0, theme::ACCENT.gamma_multiply(0.25));
        let ch = email
            .as_deref()
            .and_then(|e| e.chars().next())
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_else(|| "?".into());
        ui.painter().text(
            c,
            egui::Align2::CENTER_CENTER,
            ch,
            egui::FontId::proportional(12.0),
            theme::ACCENT,
        );
        ui.painter().text(
            rect.left_center() + egui::vec2(34.0, 0.0),
            egui::Align2::LEFT_CENTER,
            widgets::elide(label, 22),
            egui::FontId::proportional(13.0),
            ui.visuals().text_color(),
        );
    }

    fn central(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default()
            .frame(
                egui::Frame::central_panel(ui.style())
                    .inner_margin(egui::Margin::symmetric(30, 20)),
            )
            .show(ui, |ui| match self.page {
                Page::Home => home::draw(self, ui),
                Page::Cloud => cloud::draw(self, ui),
                Page::Activity => activity::draw(self, ui),
                Page::Settings => settings::draw(self, ui),
                Page::About => about::draw(self, ui),
            });
    }

    fn toast_ui(&mut self, ctx: &egui::Context) {
        let t = ctx.input(|i| i.time);
        if let Some((msg, at)) = &self.toast {
            if t - *at > 4.0 {
                self.toast = None;
                return;
            }
            let msg = msg.clone();
            egui::Area::new(egui::Id::unique("toast"))
                .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-18.0, -18.0))
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.label(msg);
                    });
                });
        }
    }

    fn dialog_ui(&mut self, ctx: &egui::Context) {
        let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let Some(dlg) = self.dialog.clone() else { return };
        let mut open = true;
        let mut act: Option<DlgAct> = None;

        match &dlg {
            Dialog::Info { title, body } => {
                egui::Window::new(title.clone())
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .open(&mut open)
                    .show(ctx, |ui| {
                        ui.label(body.clone());
                        ui.add_space(10.0);
                        if widgets::primary_button(ui, "OK") || escape {
                            act = Some(DlgAct::Close);
                        }
                    });
            }
            Dialog::ConfirmSave => {
                egui::Window::new("Save changes")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .open(&mut open)
                    .show(ctx, |ui| {
                        ui.label("Save your settings and restart syncing now?");
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            if ui.button("Cancel").clicked() || escape {
                                act = Some(DlgAct::Close);
                            }
                            if widgets::primary_button(ui, "Save") {
                                act = Some(DlgAct::Save);
                            }
                        });
                    });
            }
            Dialog::ConfirmLogout => {
                egui::Window::new("Sign out")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .open(&mut open)
                    .show(ctx, |ui| {
                        ui.label(
                            "Sign out of Proton on this computer?\n\nYour synced files stay on \
                             this computer.",
                        );
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            if ui.button("Cancel").clicked() || escape {
                                act = Some(DlgAct::Close);
                            }
                            if widgets::primary_button(ui, "Sign out") {
                                act = Some(DlgAct::Logout);
                            }
                        });
                    });
            }
            Dialog::PauseMenu => {
                egui::Window::new("Pause syncing")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .open(&mut open)
                    .show(ctx, |ui| {
                        if ui.button("Pause for 15 minutes").clicked() {
                            act = Some(DlgAct::Pause(Some(900)));
                        }
                        if ui.button("Pause for 1 hour").clicked() {
                            act = Some(DlgAct::Pause(Some(3600)));
                        }
                        if ui.button("Pause until I resume").clicked() {
                            act = Some(DlgAct::Pause(None));
                        }
                        if escape {
                            act = Some(DlgAct::Close);
                        }
                    });
            }
            Dialog::DirBrowse { cwd } => {
                let cwd = cwd.clone();
                egui::Window::new("Choose the folder to sync")
                    .collapsible(false)
                    .resizable(false)
                    .default_size([420.0, 480.0])
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .open(&mut open)
                    .show(ctx, |ui| {
                        ui.label(
                            egui::RichText::new(widgets::elide(&cwd.to_string_lossy(), 70))
                                .weak(),
                        );
                        ui.add_space(6.0);
                        egui::ScrollArea::vertical().max_height(340.0).show(ui, |ui| {
                            if let Some(parent) = cwd.parent() {
                                if ui.button("\u{2191}  Go up").clicked() {
                                    act = Some(DlgAct::BrowseGo(parent.to_path_buf()));
                                }
                            }
                            let mut dirs: Vec<String> = std::fs::read_dir(&cwd)
                                .map(|it| {
                                    it.flatten()
                                        .filter(|e| e.path().is_dir())
                                        .map(|e| e.file_name().to_string_lossy().into_owned())
                                        .filter(|n| !n.starts_with('.'))
                                        .collect()
                                })
                                .unwrap_or_default();
                            dirs.sort();
                            for name in dirs {
                                if ui.button(format!("\u{25b8}  {name}")).clicked() {
                                    act = Some(DlgAct::BrowseGo(cwd.join(name)));
                                }
                            }
                        });
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            if ui.button("Cancel").clicked() || escape {
                                act = Some(DlgAct::Close);
                            }
                            if widgets::primary_button(ui, "Choose this folder") {
                                act = Some(DlgAct::BrowseSet(cwd.clone()));
                            }
                        });
                    });
            }
            Dialog::Input { title, label, intent, .. } => {
                let title = title.clone();
                let label = label.clone();
                let intent = intent.clone();
                egui::Window::new(title)
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .open(&mut open)
                    .show(ctx, |ui| {
                        ui.label(label);
                        ui.add_space(6.0);
                        let id = egui::Id::unique("cloud-text-input");
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut self.input_buf)
                                .id(id)
                                .desired_width(300.0),
                        );
                        if self.input_focus {
                            ui.memory_mut(|m| m.request_focus(id));
                            self.input_focus = false;
                        }
                        let enter =
                            r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            if ui.button("Cancel").clicked() || escape {
                                act = Some(DlgAct::Close);
                            }
                            if widgets::primary_button(ui, "OK") || enter {
                                let name = self.input_buf.clone();
                                act = Some(match intent {
                                    InputIntent::CloudRename(old) => {
                                        DlgAct::CloudRename { old, name }
                                    }
                                    InputIntent::CloudNewFolder => DlgAct::NewFolder(name),
                                });
                            }
                        });
                    });
            }
            Dialog::ConfirmTrash { n } => {
                let word = if *n == 1 {
                    "this item".to_string()
                } else {
                    format!("these {n} items")
                };
                egui::Window::new("Move to trash?")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .open(&mut open)
                    .show(ctx, |ui| {
                        ui.label(format!(
                            "Move {word} to the Proton trash? You can restore it later from \
                             Trash."
                        ));
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            if ui.button("Cancel").clicked() || escape {
                                act = Some(DlgAct::Close);
                            }
                            if widgets::primary_button(ui, "Move to trash") {
                                act = Some(DlgAct::TrashConfirm);
                            }
                        });
                    });
            }
            Dialog::CloudMove => {
                let root = cloud::remote_root(&self.conf);
                egui::Window::new("Move to")
                    .collapsible(false)
                    .resizable(false)
                    .default_size([430.0, 480.0])
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .open(&mut open)
                    .show(ctx, |ui| {
                        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                            self.picker_row(
                                ui,
                                "",
                                &format!("My Files ({root})"),
                                0,
                                &mut act,
                            );
                        });
                        ui.add_space(10.0);
                        let can = self
                            .cloud
                            .picker
                            .as_ref()
                            .is_some_and(|p| p.sel.is_some());
                        ui.horizontal(|ui| {
                            if ui.button("Cancel").clicked() || escape {
                                act = Some(DlgAct::Close);
                            }
                            if can && widgets::primary_button(ui, "Move here") {
                                let sel = self
                                    .cloud
                                    .picker
                                    .as_ref()
                                    .and_then(|p| p.sel.clone())
                                    .unwrap_or_default();
                                act = Some(DlgAct::MovePick(sel));
                            }
                        });
                    });
            }
        }

        match act {
            Some(a) => self.apply_dialog(a),
            None if !open => self.dialog = None,
            None => {}
        }
    }

    /// One row of the "Move to..." folder tree (lazily expanded, like the
    /// PyQt picker dialog).
    fn picker_row(
        &mut self,
        ui: &mut egui::Ui,
        rel: &str,
        label: &str,
        depth: usize,
        act: &mut Option<DlgAct>,
    ) {
        let (expanded, selected) = {
            match self.cloud.picker.as_ref() {
                None => (false, false),
                Some(p) => (
                    p.expanded.contains(rel),
                    p.sel.as_deref() == Some(rel),
                ),
            }
        };
        ui.horizontal(|ui| {
            ui.add_space(depth as f32 * 16.0);
            if ui
                .button(if expanded { "\u{25be}" } else { "\u{25b8}" })
                .clicked()
            {
                cloud::picker_expand(self, rel.to_string());
            }
            if ui.selectable_label(selected, label).clicked() {
                *act = Some(DlgAct::MoveSel(rel.to_string()));
            }
        });
        if expanded {
            let children = self
                .cloud
                .picker
                .as_ref()
                .and_then(|p| p.children.get(rel))
                .cloned()
                .unwrap_or_default();
            for (name, child_rel) in children {
                self.picker_row(ui, &child_rel, &name, depth + 1, act);
            }
        }
    }

    /// Act on a confirmed dialog (after the closure releases &mut self).
    fn apply_dialog(&mut self, act: DlgAct) {
        match act {
            DlgAct::Close => {
                if matches!(self.dialog, Some(Dialog::CloudMove)) {
                    self.cloud.picker = None;
                }
                self.dialog = None;
            }
            DlgAct::Save => {
                self.dialog = None;
                settings::do_save(self);
            }
            DlgAct::Logout => {
                self.dialog = None;
                self.action_logout_confirmed();
            }
            DlgAct::Pause(secs) => {
                self.dialog = None;
                self.action_pause(secs);
            }
            DlgAct::BrowseGo(p) => self.dialog = Some(Dialog::DirBrowse { cwd: p }),
            DlgAct::BrowseSet(p) => {
                self.draft.local = p.to_string_lossy().into_owned();
                self.dialog = None;
            }
            DlgAct::CloudRename { old, name } => {
                self.dialog = None;
                cloud::action_rename(self, old, name);
            }
            DlgAct::NewFolder(name) => {
                self.dialog = None;
                cloud::action_new_folder(self, name);
            }
            DlgAct::TrashConfirm => {
                self.dialog = None;
                cloud::action_trash(self);
            }
            DlgAct::MoveSel(rel) => {
                if let Some(p) = self.cloud.picker.as_mut() {
                    p.sel = Some(rel);
                }
            }
            DlgAct::MovePick(rel) => {
                self.dialog = None;
                self.cloud.picker = None;
                cloud::action_move(self, rel);
            }
        }
    }
}

/// First semver-looking triple inside a `--version` banner.
pub fn util_semver(s: &str) -> Option<String> {
    let re = regex::Regex::new(r"(\d+\.\d+\.\d+)").ok()?;
    re.find(s).map(|m| m.as_str().to_string())
}

impl eframe::App for App {
    /// Called every frame while visible, and also while hidden (via
    /// `run_logic`) - so the sync poller keeps flowing to the tray even when
    /// the window is closed to the background.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain(ctx);

        // Cloud: ensure the initial listing even if the page was reached
        // without a nav click (e.g. it becomes the startup page one day).
        if self.page == Page::Cloud && !self.cloud.opened {
            self.cloud.opened = true;
            cloud::on_shown(self);
        }

        // Updates: automatic check 5s after start, then every 6h (Qt's
        // updates_timer); the ETag cache keeps this free of rate limits.
        if let Some(t) = self.upd.next_check {
            if std::time::Instant::now() >= t {
                self.upd.next_check =
                    Some(std::time::Instant::now() + Duration::from_secs(6 * 3600));
                if self.prefs.updates_auto() {
                    self.action_check_updates();
                }
            }
        }
        // After an update swap: quit once the status label has been seen.
        if let Some(t) = self.upd.quit_at {
            if std::time::Instant::now() >= t {
                self.upd.quit_at = None;
                self.quit(ctx);
            }
        }

        if self.hide_on_start {
            self.hide_on_start = false;
            self.hide_window(ctx);
        }

        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if self.prefs.close_to_tray() {
                self.hide_to_tray(ctx);
            } else {
                self.quit(ctx);
            }
        }

        if self.style_dirty {
            theme::apply(ctx, &self.prefs.theme());
            theme::force(ctx, &self.prefs.theme());
            self.style_dirty = false;
        }

        ctx.request_repaint_after(Duration::from_millis(900));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // White sidebar against the lavender content page - the Proton Drive
        // web layout. The pane's right edge is draggable (176-400 px); 216.0
        // is the width this pane has always had, so a fresh install looks
        // exactly like before, and egui remembers a dragged width across
        // restarts.
        egui::Panel::left("nav")
            .resizable(true)
            .default_size(216.0)
            .min_size(176.0)
            .max_size(400.0)
            .frame(
                egui::Frame::side_top_panel(ui.style())
                    .inner_margin(egui::Margin::symmetric(16, 18))
                    .fill(theme::surface(ui.visuals())),
            )
            .show(ui, |ui| self.sidebar(ui));

        self.central(ui);

        let ctx = ui.ctx().clone();
        self.toast_ui(&ctx);
        self.dialog_ui(&ctx);
    }
}
