//! The application: shell (sidebar + pages), background message plumbing,
//! dialogs, and notification triggers - ported from the PyQt GUI's window
//! logic with the same strings and behaviors.

pub mod about;
pub mod activity;
pub mod cloud;
pub mod home;
pub mod settings;
pub mod widgets;

use crate::{
    cli, conf, health, journal, notify, paths::Paths, prefs, release, service,
    system_theme, theme, tray, util,
};
use eframe::egui;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Home,
    Cloud,
    Settings,
    About,
}

/// Messages from the poller/worker threads to the UI.
#[derive(Clone)]
pub enum Back {
    Poll { health: Box<health::Health>, active: bool, enabled: bool },
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
    /// Permanently delete the trash selection (cannot be undone).
    ConfirmDelete { n: usize },
    /// Empty the whole Proton trash (cannot be undone).
    ConfirmEmptyTrash,
    ConfirmClearCache,
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
    DeleteConfirm,
    EmptyTrashConfirm,
    ClearCache,
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
    /// Sidebar width in points; the pane default and the remembered size.
    pub nav_width: f32,
    /// Set by `logic()`, read by the poller: while the Home feed (the old
    /// Activity tab) is on screen the journal refreshes on the live cadence.
    pub journal_open: Arc<AtomicBool>,

    pub identity: Option<cli::Identity>,
    /// Re-probe sign-in after the engine reports a keyring problem: a locked
    /// keyring makes the CLI look "not signed in" until it is unlocked.
    pub identity_retry: Option<std::time::Instant>,
    /// Last successfully probed account email. Lets the account card show the
    /// address immediately, and distinguishes "session expired" (previous
    /// sign-in) from a fresh install.
    pub last_email: Option<String>,
    /// The user paused syncing (persisted across restarts via a marker file).
    pub paused_by_user: bool,
    pub paused_at: Option<std::time::Instant>,
    /// Set when a sign-in terminal was opened, so the orb can show progress.
    pub login_started: Option<std::time::Instant>,
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
    /// The desktop's colour scheme as last detected. Only used when the pref
    /// is `System`, which egui cannot resolve on Linux.
    pub system_theme: prefs::Theme,
    /// New colour schemes from `system_theme::watch` (OS flips).
    theme_rx: mpsc::Receiver<prefs::Theme>,
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
    if let Some(rest) = s.strip_prefix("~/")
        && let Some(home) = dirs::home_dir() {
            return home.join(rest);
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
            for px in buf[..w * h * 3].as_chunks::<3>().0 {
                out.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
            out
        }
        _ => return None,
    };
    Some((w, h, rgba))
}

/// How often the poller looks at the state files when nothing wakes it.
/// The expensive work (journalctl, systemctl, the folder walk) is gated
/// below; this heartbeat only covers a dead watcher and a page switch.
const HEARTBEAT: Duration = Duration::from_secs(3);
/// The unit state costs two `systemctl` spawns; 5 s is indistinguishable in
/// use and cuts the idle rate by 40%.
const UNIT_REFRESH: Duration = Duration::from_secs(5);
/// Telemetry can land several times a second during transfers. The window
/// only repaints every 900 ms, so there is no point forwarding faster.
const POLL_MIN_GAP: Duration = Duration::from_millis(900);
/// The journal feeds the Activity list and the conflict/error notifications.
/// Re-read it only when there is new telemetry or while Activity is open.
const JOURNAL_ACTIVE: Duration = Duration::from_secs(3);
const JOURNAL_IDLE: Duration = Duration::from_secs(30);
/// After each sync cycle the file/byte totals are accurate, so the blind
/// timer only has to catch edits made while sync is stopped.
const STATS_FALLBACK: Duration = Duration::from_secs(300);

fn journal_due(changed: bool, since: Duration, journal_open: bool) -> bool {
    (journal_open && since >= JOURNAL_ACTIVE)
        || (changed && since >= JOURNAL_ACTIVE)
        || since >= JOURNAL_IDLE
}

fn stats_due(have: bool, local_changed: bool, cycle_done: bool, since: Duration) -> bool {
    !have
        || local_changed
        || (cycle_done && since >= Duration::from_secs(5))
        || since >= STATS_FALLBACK
}

fn spawn_poller(tx: mpsc::Sender<Back>, paths: Paths, journal_open: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let (wake_tx, wake_rx) = mpsc::channel();
        // inotify makes the window react to a write immediately; the short
        // heartbeat below still runs, so a failed watch only costs latency.
        let mut live = crate::statewatch::spawn(&paths.state_dir, wake_tx);

        let mut active = service::is_active();
        let mut enabled = service::is_enabled();
        let mut last_unit = Instant::now();
        let mut last_poll: Option<Instant> = None;
        let mut dirty = true;
        let mut last_journal: Option<Instant> = None;
        let mut last_stats: Option<Instant> = None;
        let mut have_stats = false;
        let mut local = String::new();
        let mut fingerprint = String::new();

        loop {
            // The first pass publishes the state immediately; after that,
            // sleep until a write lands (or the heartbeat expires).
            if last_poll.is_some() {
                if live {
                    match wake_rx.recv_timeout(HEARTBEAT) {
                        Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            // The watcher stopped (I/O error, or the directory
                            // went away): keep polling on the heartbeat.
                            live = false;
                        }
                    }
                    // A rename + create burst becomes one refresh.
                    while live && wake_rx.try_recv().is_ok() {}
                } else {
                    std::thread::sleep(HEARTBEAT);
                }
            }

            let h = health::read(&paths.health_file);
            // `ts` is second-granular, so include the counters that move
            // between writes; the pair is enough to spot new telemetry.
            let now_fingerprint = format!(
                "{}|{}|{}|{}|{}|{}|{}",
                h.ts.as_deref().unwrap_or(""),
                h.status,
                h.uploaded,
                h.downloaded,
                h.actions,
                h.transfer_done.unwrap_or(0),
                h.audit_phase.as_deref().unwrap_or(""),
            );
            let changed = now_fingerprint != fingerprint;
            fingerprint = now_fingerprint;
            dirty |= changed;

            if last_unit.elapsed() >= UNIT_REFRESH {
                active = service::is_active();
                enabled = service::is_enabled();
                last_unit = Instant::now();
            }
            // Batch telemetry bursts to the repaint cadence; the heartbeat
            // still forwards an unchanged snapshot in case a status was lost.
            let repaint_due = last_poll.is_none_or(|t| t.elapsed() >= POLL_MIN_GAP);
            let heartbeat_due = last_poll.is_none_or(|t| t.elapsed() >= HEARTBEAT);
            if (dirty && repaint_due) || heartbeat_due {
                if tx.send(Back::Poll { health: Box::new(h.clone()), active, enabled }).is_err() {
                    break;
                }
                last_poll = Some(Instant::now());
                dirty = false;
            }

            // A second launch pokes this file. The watcher wakes us on the
            // create, so the window comes forward at once; the stat below is
            // the fallback for a missed event.
            let show = paths.state_dir.join("show-request");
            if show.exists() {
                let _ = std::fs::remove_file(&show);
                if tx.send(Back::ShowWindow).is_err() {
                    break;
                }
            }

            let since = last_journal.map_or(Duration::MAX, |t| t.elapsed());
            let watching = journal_open.load(Ordering::Relaxed);
            if journal_due(changed, since, watching) {
                last_journal = Some(Instant::now());
                if tx.send(Back::Journal(service::journal_lines(600))).is_err() {
                    break;
                }
            }

            let c = conf::read(&paths.conf_file);
            let configured = c.get("PROTON_SYNC_LOCAL").cloned().unwrap_or_default();
            // A cycle ends with an "ok" write; while it runs the status is
            // "syncing", so this does not walk the tree mid-transfer.
            let cycle_done = changed && h.status == "ok";
            let since = last_stats.map_or(Duration::MAX, |t| t.elapsed());
            if stats_due(have_stats, configured != local, cycle_done, since) {
                let (files, bytes) =
                    service::folder_stats(&expand_user(&configured).to_string_lossy());
                if tx.send(Back::Stats { files, bytes }).is_err() {
                    break;
                }
                last_stats = Some(Instant::now());
                local = configured;
                have_stats = true;
            }
        }
    });
}

/// Sidebar stat chip: glass mini-card with an accent number and a muted
/// label ("1,037 / files", "1.1 GB / on disk").
fn stat_chip(ui: &mut egui::Ui, value: &str, label: &str, w: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 50.0), egui::Sense::hover());
    ui.painter().rect(
        rect,
        egui::CornerRadius::same(12),
        theme::glass_fill(ui.visuals()),
        theme::glass_stroke(ui.visuals()),
        egui::StrokeKind::Inside,
    );
    // Painted rather than laid out: a fixed height keeps the footer's
    // measured height honest, and width-aware elision stops a long count
    // overflowing the card.
    let pad = 8.0;
    let room = (w - 2.0 * pad).max(10.0);
    ui.painter().text(
        rect.left_top() + egui::vec2(pad, pad),
        egui::Align2::LEFT_TOP,
        widgets::elide(value, (room / 7.2) as usize),
        egui::FontId::proportional(if w < 84.0 { 13.0 } else { 15.0 }),
        theme::accent(ui.visuals()),
    );
    ui.painter().text(
        rect.left_bottom() + egui::vec2(pad, -pad),
        egui::Align2::LEFT_BOTTOM,
        widgets::elide(label, (room / 5.2) as usize),
        egui::FontId::proportional(if w < 84.0 { 10.0 } else { 10.5 }),
        ui.visuals().weak_text_color(),
    );
}

impl App {
    /// eframe's `AppCreator` returns a boxed app, not `Self` - hence the
    /// `new` name without a `Self` return.
    #[allow(clippy::new_ret_no_self)]
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
        // Login/menu entries whose AppImage moved or vanished would silently
        // never launch (the xdg-autostart generator skips missing Exec
        // programs); point them at this copy again.
        crate::desktop::refresh_entries(&paths);
        // The window exists: if this is the first launch after an update
        // swap, confirm health - drop the rollback marker and its .old
        // backup. Running as an AppImage only, so dev runs never consume a
        // real update's marker.
        if paths.appimage.is_some() {
            release::update_confirm_health(&paths);
        }
        let prefs = prefs::Prefs::load(&paths.prefs_file);
        // Read the desktop's scheme before the first frame so the first paint
        // already matches it (Linux never reports it to egui itself).
        let detected_theme = system_theme::detect();
        let upd_next_check = prefs.updates_auto().then(|| {
            std::time::Instant::now() + Duration::from_secs(5)
        });
        theme::apply(&cc.egui_ctx, &prefs.theme());
        theme::force(
            &cc.egui_ctx,
            &system_theme::resolve(&prefs.theme(), &detected_theme),
        );
        // egui draws text at fixed point sizes with its own font - the
        // interface scale pref is the only way to shrink/grow it.
        cc.egui_ctx.set_zoom_factor(prefs.ui_scale().factor());
        let conf = conf::read(&paths.conf_file);
        let conf_extras = conf::read_extras(&paths.conf_file);
        let draft = settings::Draft::load(&conf, &conf_extras, &prefs, &paths);
        let (tx, rx) = mpsc::channel();
        let journal = Arc::new(AtomicBool::new(false));

        spawn_poller(tx.clone(), paths.clone(), journal.clone());
        let theme_rx = system_theme::watch(cc.egui_ctx.clone());

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
        let health_file = paths.health_file.clone();
        std::thread::spawn(move || {
            let unit = service::ensure_service_unit(appimage.as_deref(), false);
            let active = service::is_active();
            // After an in-app update the AppImage at the same path is
            // replaced, so the unit never changes and the running service
            // keeps exec'ing the old engine. The engine stamps its build into
            // health.json; restart when it does not match this GUI.
            let stale = crate::health::read(&health_file)
                .build
                .as_deref()
                .is_some_and(|b| b != crate::GUI_VERSION);
            if active && (unit == "written" || stale) {
                let _ = service::restart();
            }
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

        let nav_width = prefs.nav_width;
        let paused_by_user = paths.state_dir.join("paused").exists();
        let last_email = std::fs::read_to_string(paths.state_dir.join("last-email"))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
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
            nav_width,
            journal_open: journal.clone(),

            identity: None,
            identity_retry: None,
            last_email,
            paused_by_user,
            paused_at: paused_by_user.then(Instant::now),
            login_started: None,
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
            system_theme: detected_theme,
            theme_rx,
            icon_tex,
            tray,
            wayland_fallback,
            relaunch_slot,
        }))
    }

    // ------------------------------------------------------------- status

    /// The theme to paint with: `System` uses the desktop's last answer.
    fn resolved_theme(&self) -> prefs::Theme {
        system_theme::resolve(&self.prefs.theme(), &self.system_theme)
    }

    /// The rich status the Home orb and the sidebar share: engine liveness,
    /// the engine's report, the account probe, and transient login/pause state.
    pub fn ui_status(&self) -> health::UiStatus {
        let signing_in = self
            .login_started
            .is_some_and(|t| t.elapsed() < Duration::from_secs(300))
            && !self.logged_in();
        let session_error = self
            .identity
            .as_ref()
            .and_then(|i| i.error.as_deref())
            .is_some_and(health::is_login_error);
        let identity = self.identity.as_ref().map(|i| i.logged_in);
        health::derive_status(
            self.active,
            &self.health.status,
            identity,
            signing_in,
            self.paused_by_user,
            self.last_email.is_some(),
            session_error,
        )
    }

    pub fn status_key(&self) -> &'static str {
        self.ui_status().key()
    }

    /// Qt put the STATUS_TEXT title on the side status label and the tray
    /// tooltip ("You're up to date", not a short chip).
    pub fn side_label(&self) -> &'static str {
        health::status_text(self.ui_status()).0
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
                self.login_started = Some(Instant::now());
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

    pub fn action_logout_confirmed(&mut self) {
        // Explicit sign-out: the next "not signed in" state is the welcome
        // screen, not "session expired".
        self.last_email = None;
        self.login_started = None;
        let _ = std::fs::remove_file(self.paths.state_dir.join("last-email"));
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            if let Some(c) = cli::resolve_cli() {
                cli::logout(&c);
                let _ = tx.send(Back::Identity(cli::fetch_identity(&c)));
            }
        });
    }

    pub fn action_pause(&mut self, secs: Option<u64>) {
        self.paused_by_user = true;
        self.paused_at = Some(Instant::now());
        let _ = std::fs::create_dir_all(&self.paths.state_dir);
        let _ = std::fs::write(self.paths.state_dir.join("paused"), b"1");
        std::thread::spawn(move || {
            let _ = service::pause_for(secs);
        });
    }

    pub fn action_resume(&mut self) {
        self.paused_by_user = false;
        self.paused_at = None;
        let _ = std::fs::remove_file(self.paths.state_dir.join("paused"));
        let appimage = self.paths.appimage.clone();
        std::thread::spawn(move || {
            let _ = service::ensure_service_unit(appimage.as_deref(), false);
            let _ = service::resume();
        });
    }

    pub fn action_restart_service(&mut self) {
        self.paused_by_user = false;
        self.paused_at = None;
        let _ = std::fs::remove_file(self.paths.state_dir.join("paused"));
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
                    self.on_poll(*health, active, enabled);
                    self.tray_sync();
                }
                Back::Journal(lines) => {
                    self.on_journal(lines);
                    self.tray_sync();
                }
                Back::Identity(id) => {
                    // Waking the engine is only needed when this is a fresh
                    // sign-in, not the ordinary startup identity probe (the
                    // service already runs its first cycle on its own).
                    let was_signing_in = self.login_started.is_some();
                    let signed_in_now = id.logged_in
                        && self
                            .identity
                            .as_ref()
                            .is_some_and(|i| !i.logged_in);
                    if id.logged_in {
                        self.login_started = None;
                        if let Some(email) = id.email.clone() {
                            let _ = std::fs::write(
                                self.paths.state_dir.join("last-email"),
                                &email,
                            );
                            self.last_email = Some(email);
                        }
                        if was_signing_in || signed_in_now {
                            // A successful sign-in must start a cycle now, even
                            // when interval=0 left the engine waiting.
                            let _ = std::fs::write(&self.paths.wake_file, b"");
                        }
                    }
                    self.identity = Some(id);
                    // Keep probing while the sign-in terminal is open - after
                    // each result, not on a blind timer, so probes never stack
                    // up behind the engine's CLI lock.
                    if !self.logged_in()
                        && self
                            .login_started
                            .is_some_and(|t| t.elapsed() < Duration::from_secs(300))
                        && self.identity_retry.is_none()
                    {
                        self.identity_retry =
                            Some(Instant::now() + Duration::from_secs(10));
                    }
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

    /// Take the desktop's latest colour scheme. Only the "Match my system"
    /// preference reacts; an explicit Light/Dark keeps winning.
    fn drain_theme(&mut self, ctx: &egui::Context) {
        let mut latest = None;
        while let Ok(theme) = self.theme_rx.try_recv() {
            latest = Some(theme);
        }
        let Some(theme) = latest else { return };
        if theme == self.system_theme {
            return;
        }
        self.system_theme = theme.clone();
        if matches!(self.prefs.theme(), prefs::Theme::System) {
            theme::force(ctx, &theme);
            ctx.request_repaint();
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
        // A locked keyring makes `fs list` fail with "You need to login first",
        // so the startup identity probe wrongly lands on "not signed in" even
        // though the session is safe in the keyring. Once the engine reports
        // the keyring problem (and asks the desktop to unlock it), re-probe so
        // the window does not stay on the welcome screen.
        let keyring_issue = health
            .error
            .as_deref()
            .is_some_and(|e| e.to_ascii_lowercase().contains("keyring"));
        // Also retry once the engine reports healthy again (it has just
        // unlocked the keyring and is syncing): the first identity probe was
        // answered while the keyring was still locked.
        let engine_signed_in = matches!(health.status.as_str(), "ok" | "syncing");
        let identity_ready = self.identity.as_ref().is_some_and(|i| i.error.is_none());
        if (keyring_issue || (active && engine_signed_in))
            && identity_ready
            && !self.logged_in()
            && self.identity_retry.is_none()
        {
            self.identity_retry = Some(Instant::now() + Duration::from_secs(15));
        }
        self.health = health;
        self.active = active;
        self.enabled = enabled;
        // A timed pause auto-resumes (or the user starts it elsewhere): clear
        // the paused marker so a later crash reads as "engine stopped", not
        // "paused". The 8 s grace avoids clearing while the stop is in flight.
        if active
            && self.paused_by_user
            && self
                .paused_at
                .is_some_and(|t| t.elapsed() >= Duration::from_secs(8))
        {
            self.paused_by_user = false;
            self.paused_at = None;
            let _ = std::fs::remove_file(self.paths.state_dir.join("paused"));
        }
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
            // Full wordmark needs ~160px of content; below that the pane
            // would refuse to shrink, so switch to a compact mark that fits.
            let compact = ui.available_width() < 162.0;
            let logo = if compact { 24.0 } else { 30.0 };
            let size = if compact { 14.0 } else { 16.5 };
            if let Some(tex) = &self.icon_tex {
                ui.add(egui::Image::new((tex.id(), egui::vec2(logo, logo))));
            }
            // Truncating labels let the pane shrink instead of the wordmark
            // forcing it back to its natural width.
            ui.add(
                egui::Label::new(egui::RichText::new("Proton Drive").strong().size(size))
                    .truncate(),
            );
            if !compact {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new("CLI")
                            .strong()
                            .size(size)
                            .color(theme::accent(ui.visuals())),
                    )
                    .truncate(),
                );
            }
        });
        ui.add_space(14.0);

        // Footer pinned to the bottom; the nav list then takes what is
        // left. Shown as a bottom panel (not a bottom-up layout) so its
        // height is measured instead of guessed at, and the account row
        // always sits above the window edge.
        egui::Panel::bottom("nav-footer")
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                ui.add_space(6.0);
                if let Some((files, bytes)) = self.stats {
                    let w = ((ui.available_width() - 8.0) / 2.0).max(40.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        stat_chip(ui, &util::thousands(files), "files", w);
                        ui.add_space(8.0);
                        stat_chip(ui, &service::human_size(bytes), "on disk", w);
                    });
                    ui.add_space(12.0);
                }
                // Status card, like the Proton sidebar's footer block.
                egui::Frame::new()
                    .fill(theme::glass_fill(ui.visuals()))
                    .stroke(theme::glass_stroke(ui.visuals()))
                    .corner_radius(egui::CornerRadius::same(12))
                    .inner_margin(egui::Margin::symmetric(12, 9))
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.horizontal(|ui| {
                            let key = self.status_key();
                            let (rect, _) = ui
                                .allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                            ui.painter().circle_filled(
                                rect.center(),
                                5.0,
                                theme::status_color(key),
                            );
                            ui.add(
                                egui::Label::new(egui::RichText::new(self.side_label()).size(13.0))
                                    .truncate(),
                            );
                        });
                        ui.add_space(2.0);
                        // Wrap rather than truncate: the footer measures its
                        // own height, so a second line costs nothing.
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new("Independent community edition")
                                    .size(11.5)
                                    .weak(),
                            )
                            .wrap(),
                        );
                    });
                ui.add_space(8.0);
                self.account_row(ui);
                ui.add_space(4.0);
            });

        // Home, My files (expandable folder tree), Settings, Trash, About;
        // long trees scroll in the space above the footer.
        egui::ScrollArea::vertical()
            .id_salt("nav-scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| cloud::nav_ui(self, ui));
    }

    /// The pane's account row (Proton Drive's sidebar footer): avatar +
    /// identity at the very bottom; the pages themselves live as rows in the
    /// nav list above.
    fn account_row(&mut self, ui: &mut egui::Ui) {
        let email = self
            .identity
            .as_ref()
            .and_then(|i| i.email.clone())
            .or_else(|| self.last_email.clone());
        let label = if let Some(email) = email.as_deref() {
            email
        } else if self.identity.is_some() {
            "You're not signed in yet"
        } else {
            "Checking your account..."
        };
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 46.0), egui::Sense::click());
        let response = response.on_hover_text("Account \u{b7} click to sign out or sign in");
        // Glass pill: accent-glow edge on hover/focus, presence dot on the
        // avatar, chevron hinting the menu.
        ui.painter().rect_filled(
            rect,
            egui::CornerRadius::same(12),
            theme::glass_fill(ui.visuals()),
        );
        let pill_stroke = if response.hovered() || response.has_focus() {
            egui::Stroke::new(1.0, theme::accent(ui.visuals()).gamma_multiply(0.8))
        } else {
            theme::glass_stroke(ui.visuals())
        };
        ui.painter().rect_stroke(
            rect,
            egui::CornerRadius::same(12),
            pill_stroke,
            egui::StrokeKind::Inside,
        );
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        // Avatar: gradient violet disc with a cyan cap and the initial.
        let c = egui::pos2(rect.left() + 22.0, rect.center().y);
        ui.painter().circle_filled(c, 15.0, theme::ACCENT);
        ui.painter().circle_filled(
            c + egui::vec2(0.0, -6.0),
            9.5,
            theme::CYAN.gamma_multiply(0.45),
        );
        let ch = email
            .as_deref()
            .and_then(|e| e.chars().next())
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_else(|| "?".into());
        ui.painter().text(
            c,
            egui::Align2::CENTER_CENTER,
            ch,
            egui::FontId::proportional(14.0),
            egui::Color32::WHITE,
        );
        // Presence dot: the live status colour with a ring.
        let dot = c + egui::vec2(10.0, 10.0);
        ui.painter()
            .circle_filled(dot, 4.5, theme::status_color(self.status_key()));
        ui.painter().circle_stroke(
            dot,
            4.5,
            egui::Stroke::new(1.5, ui.visuals().panel_fill),
        );
        ui.painter().text(
            rect.left_center() + egui::vec2(48.0, 0.0),
            egui::Align2::LEFT_CENTER,
            // Width-aware elision: a fixed character count overflowed the
            // pill (and its chevron) on a narrow pane.
            widgets::elide(label, ((rect.width() - 80.0) / 6.6).clamp(4.0, 24.0) as usize),
            egui::FontId::proportional(13.5),
            ui.visuals().strong_text_color(),
        );
        ui.painter().text(
            rect.right_center() + egui::vec2(-12.0, 0.0),
            egui::Align2::RIGHT_CENTER,
            "\u{25be}",
            egui::FontId::proportional(12.0),
            ui.visuals().weak_text_color(),
        );

        let signed_in = self.logged_in()
            || (self.last_email.is_some()
                && !matches!(
                    self.ui_status(),
                    health::UiStatus::SignedOut
                        | health::UiStatus::NeedsLogin
                        | health::UiStatus::SigningIn
                ));
        egui::Popup::menu(&response).show(|ui| {
            ui.set_min_width(140.0);
            if signed_in {
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("Sign out").color(theme::ERROR),
                        )
                        .frame(false),
                    )
                    .clicked()
                {
                    self.dialog = Some(Dialog::ConfirmLogout);
                    ui.close();
                }
            } else if ui.button("Sign in").clicked() {
                self.action_login();
                ui.close();
            }
        });
    }

    fn central(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default()
            .frame(
                egui::Frame::central_panel(ui.style())
                    .fill(egui::Color32::TRANSPARENT)
                    .inner_margin(egui::Margin::symmetric(30, 20)),
            )
            .show(ui, |ui| match self.page {
                Page::Home => home::draw(self, ui),
                Page::Cloud => cloud::draw(self, ui),
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
            let age = (t - *at) as f32;
            // Slide up and fade in over 150 ms, fade out over the last 300.
            let (fade, slide) = if age < 0.15 {
                (age / 0.15, (1.0 - age / 0.15) * 14.0)
            } else if age > 3.7 {
                ((4.0 - age) / 0.3, 0.0)
            } else {
                (1.0, 0.0)
            };
            ctx.request_repaint();
            let msg = msg.clone();
            egui::Area::new(egui::Id::unique("toast"))
                .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-18.0, -18.0 + slide))
                .show(ctx, |ui| {
                    ui.set_opacity(fade.clamp(0.0, 1.0));
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
                            if let Some(parent) = cwd.parent()
                                && ui.button("\u{2191}  Go up").clicked() {
                                    act = Some(DlgAct::BrowseGo(parent.to_path_buf()));
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
            Dialog::ConfirmDelete { n } => {
                let word = if *n == 1 { "this item".to_string() } else { format!("these {n} items") };
                egui::Window::new("Delete permanently?")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .open(&mut open)
                    .show(ctx, |ui| {
                        ui.label(format!(
                            "Permanently delete {word}? This cannot be undone."
                        ));
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            if ui.button("Cancel").clicked() || escape {
                                act = Some(DlgAct::Close);
                            }
                            if widgets::danger_ghost_button(ui, "Delete permanently") {
                                act = Some(DlgAct::DeleteConfirm);
                            }
                        });
                    });
            }
            Dialog::ConfirmEmptyTrash => {
                egui::Window::new("Empty trash?")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .open(&mut open)
                    .show(ctx, |ui| {
                        ui.label(
                            "Permanently delete everything in the Proton trash? This cannot be \
                             undone.",
                        );
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            if ui.button("Cancel").clicked() || escape {
                                act = Some(DlgAct::Close);
                            }
                            if widgets::danger_ghost_button(ui, "Empty trash") {
                                act = Some(DlgAct::EmptyTrashConfirm);
                            }
                        });
                    });
            }
            Dialog::ConfirmClearCache => {
                egui::Window::new("Clear snapshot cache")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .open(&mut open)
                    .show(ctx, |ui| {
                        ui.label(
                            "Remove the cached remote metadata? Local files, sync state and \
                             settings stay untouched. The next start will take a full scan.",
                        );
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            if ui.button("Cancel").clicked() || escape {
                                act = Some(DlgAct::Close);
                            }
                            if widgets::primary_button(ui, "Clear cache") {
                                act = Some(DlgAct::ClearCache);
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
            DlgAct::DeleteConfirm => {
                self.dialog = None;
                cloud::action_delete_permanent(self);
            }
            DlgAct::EmptyTrashConfirm => {
                self.dialog = None;
                cloud::action_empty_trash(self);
            }
            DlgAct::ClearCache => {
                self.dialog = None;
                match service::clear_snapshot_cache(&self.paths) {
                    Ok(()) => self.toast("Snapshot cache cleared. The next start will scan fully."),
                    Err(e) => self.toast(format!("Could not clear the snapshot cache: {e}")),
                }
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
        self.journal_open
            .store(self.page == Page::Home && !self.hidden, Ordering::Relaxed);
        self.drain(ctx);
        self.drain_theme(ctx);

        // Re-probe sign-in after a keyring-lock report (see on_poll).
        if let Some(t) = self.identity_retry
            && Instant::now() >= t {
                self.identity_retry = None;
                let tx = self.tx.clone();
                std::thread::spawn(move || {
                    if let Some(c) = cli::resolve_cli() {
                        let _ = tx.send(Back::Identity(cli::fetch_identity(&c)));
                    }
                });
            }

        // Cloud: ensure the initial listing even if the page was reached
        // without a nav click (e.g. it becomes the startup page one day).
        if self.page == Page::Cloud && !self.cloud.opened {
            self.cloud.opened = true;
            cloud::on_shown(self);
        }

        // Updates: automatic check 5s after start, then every 6h (Qt's
        // updates_timer); the ETag cache keeps this free of rate limits.
        if let Some(t) = self.upd.next_check
            && std::time::Instant::now() >= t {
                self.upd.next_check =
                    Some(std::time::Instant::now() + Duration::from_secs(6 * 3600));
                if self.prefs.updates_auto() {
                    self.action_check_updates();
                }
            }
        // After an update swap: quit once the status label has been seen.
        if let Some(t) = self.upd.quit_at
            && std::time::Instant::now() >= t {
                self.upd.quit_at = None;
                self.quit(ctx);
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
            // Re-read now: the user just picked a theme, and "Match my
            // system" must reflect the desktop this second, not the value
            // the watcher last saw.
            if matches!(self.prefs.theme(), prefs::Theme::System) {
                self.system_theme = system_theme::detect();
            }
            theme::apply(ctx, &self.prefs.theme());
            theme::force(ctx, &self.resolved_theme());
            self.style_dirty = false;
        }

        // Keep the interface scale in sync with the pref (Settings applies
        // it immediately too; this catches an edited config file).
        let want_zoom = self.prefs.ui_scale().factor();
        if (ctx.zoom_factor() - want_zoom).abs() > 0.001 {
            ctx.set_zoom_factor(want_zoom);
        }

        ctx.request_repaint_after(Duration::from_millis(900));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Drag-and-drop upload: files dropped on My Files upload into the
        // folder that is currently open.
        if self.page == Page::Cloud && !self.cloud.trash_view && !self.cloud.busy {
            let dropped: Vec<std::path::PathBuf> = ui.ctx().input(|i| {
                i.raw
                    .dropped_files
                    .iter()
                    .map(|f| f.path().to_path_buf())
                    .collect()
            });
            if !dropped.is_empty() {
                cloud::on_picked(self, Some(Picked::UploadFiles(dropped)));
            }
        }
        // Aurora backdrop: painted under both panels so the glass picks up
        // the glow. One request_repaint per frame keeps the drift smooth;
        // `PROTON_UI_MOTION=0` or a hidden window freezes it.
        theme::paint_aurora(ui, ui.max_rect());
        if theme::motion_enabled() && !self.hidden {
            ui.ctx().request_repaint();
        }
        // White sidebar against the lavender content page - the Proton Drive
        // web layout. The pane's right edge is draggable (176-400 px); 216.0
        // is the width this pane has always had, so a fresh install looks
        // exactly like before, and egui remembers a dragged width across
        // restarts.
        // Keep the central pane above its layout floor: the file list needs
        // the card margins plus the three columns (min Name 132 + Size 84 +
        // Modified 124 + two gaps) and the page margins. 480 leaves room for
        // the row padding and a few px of rounding slack; below this the
        // list would have to overlap.
        let nav_max = (ui.available_width() - 480.0).clamp(176.0, 400.0);
        let panel = egui::Panel::left("nav")
            .resizable(true)
            .default_size(self.nav_width)
            .min_size(150.0)
            .max_size(nav_max)
            .frame(
                egui::Frame::side_top_panel(ui.style())
                    .inner_margin(egui::Margin::symmetric(16, 18))
                    .fill(theme::glass_fill(ui.visuals()))
                    .stroke(theme::glass_stroke(ui.visuals())),
            )
            .show(ui, |ui| self.sidebar(ui));

        // Visible grab line on the pane edge: brighter on hover so the
        // divider is discoverable on glass.
        let prect = panel.response.rect;
        let grab = egui::Rect::from_center_size(
            egui::pos2(prect.right(), prect.center().y),
            egui::vec2(8.0, 48.0),
        );
        let hovering = ui.rect_contains_pointer(grab);
        let sep = if hovering {
            theme::accent(ui.visuals())
        } else {
            theme::glass_stroke(ui.visuals()).color
        };
        ui.painter().line_segment(
            [egui::pos2(prect.right(), prect.top() + 8.0), egui::pos2(prect.right(), prect.bottom() - 8.0)],
            egui::Stroke::new(1.5, sep),
        );
        if hovering {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }

        // Remember the width for the next start (persist only after the drag
        // ends, and never feed the live width back into `default_size` - that
        // feedback made the pane fight the pointer).
        let w = prect.width();
        if (w - self.nav_width).abs() > 0.5 && !ui.input(|i| i.pointer.any_down()) {
            self.nav_width = w;
            self.prefs.set_nav_width(w);
        }

        self.central(ui);

        let ctx = ui.ctx().clone();
        self.toast_ui(&ctx);
        self.dialog_ui(&ctx);
    }
}

#[cfg(test)]
mod glyph_tests {
    use eframe::egui;
    use egui::epaint::text::{FontDefinitions, FontProvider, Fonts, TextOptions};
    use egui::FontId;
    use std::sync::Arc;

    /// Every character the UI paints as an icon rather than as text.
    ///
    /// None of them exist in the fonts this app bundles (Ubuntu-Light, Hack,
    /// egui-icons), so they only render through the system font fallback that
    /// eframe's `system_fonts` feature installs. 3.16.19 dropped that feature
    /// while switching to glow, and these turned into blank boxes: the pane
    /// carets, the sort arrow, the tri-state boxes, the upload/download arrows
    /// and the folder emoji.
    const ICON_GLYPHS: &[char] = &[
        '\u{25b8}', // ▸ collapsed caret
        '\u{25be}', // ▾ expanded caret
        '\u{25bc}', // ▼ sort descending
        '\u{25b2}', // ▲ sort ascending
        '\u{2611}', // ☑ folder fully selected
        '\u{2610}', // ☐ folder not selected
        '\u{25d0}', // ◐ folder partly selected
        '\u{2191}', // ↑ upload
        '\u{2193}', // ↓ download
        '\u{1f4c1}', // 📁 local folder card
        '\u{1f4c2}', // 📂 selective-sync folder card
    ];

    /// A character's font files, not just its glyph, must exist - a tofu box
    /// would still be "drawn", so asking the rasterizer is not enough.
    #[test]
    fn icon_glyphs_resolve_through_system_fallback() {
        let mut fonts = Fonts::new(TextOptions::default(), FontDefinitions::default());
        let provider: Arc<dyn FontProvider> =
            Arc::new(egui_system_font_provider::SystemFontProvider::new());
        fonts = fonts.with_font_providers(vec![provider]);

        let missing: Vec<String> = ICON_GLYPHS
            .iter()
            .filter(|c| !fonts.has_glyph(&FontId::proportional(14.0), **c))
            .map(|c| format!("U+{:04X} {}", *c as u32, c))
            .collect();

        assert!(
            missing.is_empty(),
            "no font on this machine resolves these UI glyphs: {missing:?}\n\
             The bundled fonts do not contain them - check that eframe is built \
             with the `system_fonts` feature in gui/Cargo.toml."
        );
    }
}

#[cfg(test)]
mod poll_cadence {
    use super::*;

    #[test]
    fn journal_is_event_driven_but_never_starves() {
        let s = Duration::from_secs;
        // Fresh telemetry: refresh at the live cadence.
        assert!(journal_due(true, s(3), false));
        assert!(!journal_due(true, s(1), false));
        // No telemetry: the slow safety net only.
        assert!(!journal_due(false, s(3), false));
        assert!(!journal_due(false, s(29), false));
        assert!(journal_due(false, s(30), false));
        // The Activity page is a live log, so it ticks even while quiet.
        assert!(journal_due(false, s(3), true));
        assert!(!journal_due(false, s(1), true));
    }

    fn scratch_paths() -> (PathBuf, Paths) {
        let root = std::env::temp_dir().join(format!(
            "proton-gui-poller-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let state = root.join("state");
        std::fs::create_dir_all(&state).unwrap();
        let paths = Paths {
            conf_file: root.join("proton-sync.conf"),
            unit_file: root.join("proton-sync.service"),
            state_dir: state.clone(),
            health_file: state.join("health.json"),
            wake_file: state.join("wake"),
            lock_file: state.join("gui.lock"),
            snapshot_file: state.join("remote-snapshot.json"),
            cloud_cache_file: state.join("cloud-cache.json"),
            release_cache_file: state.join("release-cache.json"),
            update_mark_file: state.join("update-pending.json"),
            autostart_file: root.join("autostart.desktop"),
            menu_desktop_file: root.join("menu.desktop"),
            menu_icon_file: root.join("icon.svg"),
            prefs_file: root.join("gui.conf"),
            appimage: None,
        };
        (root, paths)
    }

    /// The real thread, not just the cadence helpers: a health write and a
    /// second launch must both reach the UI channel.
    #[test]
    fn poller_reacts_to_state_writes() {
        let (root, paths) = scratch_paths();
        let (tx, rx) = mpsc::channel();
        spawn_poller(tx, paths.clone(), Arc::new(AtomicBool::new(false)));

        std::fs::write(
            &paths.health_file,
            r#"{"status":"syncing","ts":"2026-10-26T12:00:00+00:00"}"#,
        )
        .unwrap();
        std::fs::write(paths.state_dir.join("show-request"), b"1").unwrap();

        let (mut saw_poll, mut saw_show) = (false, false);
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline && !(saw_poll && saw_show) {
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(Back::Poll { health, .. }) if health.status == "syncing" => saw_poll = true,
                Ok(Back::ShowWindow) => saw_show = true,
                Ok(_) | Err(_) => {}
            }
        }
        assert!(saw_poll, "a health.json write should reach the UI");
        assert!(saw_show, "show-request should reach the UI");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn stats_refresh_after_cycles_not_on_a_timer() {
        let s = Duration::from_secs;
        // Nothing known yet, a new folder, or a finished cycle: refresh.
        assert!(stats_due(false, false, false, s(0)));
        assert!(stats_due(true, true, false, s(1)));
        assert!(stats_due(true, false, true, s(6)));
        // Inside a cycle, or just after one: keep the cached totals.
        assert!(!stats_due(true, false, true, s(2)));
        assert!(!stats_due(true, false, false, s(29)));
        // The fallback catches edits made while sync is stopped.
        assert!(stats_due(true, false, false, s(300)));
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    /// Regression: a chip row wider than the pane made the sidebar grow a
    /// few pixels every frame until it hit its maximum width.
    #[test]
    fn sidebar_chips_do_not_ratchet_the_pane() {
        let ctx = egui::Context::default();
        let mut widths = Vec::new();
        for _ in 0..4 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    egui::vec2(945.0, 900.0),
                )),
                ..Default::default()
            };
            ctx.run_ui(input, |root| {
                egui::CentralPanel::default().show(root, |ui| {
                    let panel = egui::Panel::left("nav")
                        .resizable(false)
                        .default_size(200.0)
                        .max_size(400.0)
                        .frame(
                            egui::Frame::side_top_panel(ui.style())
                                .inner_margin(egui::Margin::symmetric(16, 18)),
                        )
                        .show(ui, |ui| {
                            let w = ((ui.available_width() - 8.0) / 2.0).max(40.0);
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 0.0;
                                stat_chip(ui, "1,037", "files", w);
                                ui.add_space(8.0);
                                stat_chip(ui, "1.1 GB", "on disk", w);
                            });
                        });
                    widths.push(panel.response.rect.width());
                });
            })
            .drop_without_applying_deltas();
        }
        let first = widths[0];
        let last = *widths.last().unwrap();
        assert!(
            (last - first).abs() < 0.5,
            "pane ratcheted from {first} to {last}: {widths:?}"
        );
    }
}
