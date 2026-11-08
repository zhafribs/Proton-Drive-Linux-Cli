//! The application: shell (sidebar + pages), background message plumbing,
//! dialogs, and notification triggers - ported from the PyQt GUI's window
//! logic with the same strings and behaviors.

pub mod about;
pub mod activity;
pub mod cloud;
pub mod home;
pub mod photos;
pub mod settings;
pub mod widgets;

use crate::{
    cli, conf, health, journal, notify, paths::Paths, prefs, release, service, system_theme, theme,
    tray, util,
};
use eframe::egui;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Home,
    Cloud,
    Photos,
    Activity,
    Settings,
    About,
}

/// Messages from the poller/worker threads to the UI.
#[derive(Clone)]
pub enum Back {
    Poll {
        health: Box<health::Health>,
        active: bool,
        enabled: bool,
    },
    Journal(Vec<String>),
    Identity(cli::Identity),
    Stats {
        files: u64,
        bytes: u64,
    },
    ShowWindow,
    SaveHint(String),
    /// Release check: (release, update available, installed CLI version).
    Update(Result<(release::Release, bool, String), String>),
    /// Download + swap finished; Ok is the new AppImage's path.
    UpdateInstalled(Result<String, String>),
    /// Selective-sync folder listing (parent rel "" = the root level).
    SelFolders {
        parent: String,
        folders: Vec<(String, String)>,
    },
    /// Whether the sync unit is enabled at login (slow systemctl call).
    UnitEnabled(bool),
    /// "Restart syncing now" finished; `false` = systemctl refused.
    ServiceRestarted(bool),
    /// A background action finished with a user-visible message.
    Toast { msg: String, error: bool },
    /// Sign-out succeeded: the old account's caches are gone.
    AccountCleared,
    /// Photo timeline for the Photos page.
    Photos(Result<Vec<cli::PhotoEntry>, String>),
    /// Photo download finished; Ok is how many were asked for.
    PhotosDownloaded(Result<usize, String>),
    /// Photos moved to the trash; Ok is how many.
    PhotosDeleted(Result<usize, String>),
    /// A thumbnail was decoded: (name, file stamp, w, h, RGBA).
    PhotoThumb {
        name: String,
        stamp: f64,
        result: Result<(usize, usize, Vec<u8>), String>,
    },
    /// Cloud browser: a folder listing landed (key + serial guard staleness).
    CloudEntries {
        key: String,
        serial: u64,
        result: Result<Vec<cli::RemoteEntry>, String>,
    },
    /// A sidebar tree listing (never touches the visible folder).
    TreeEntries {
        key: String,
        at: f64,
        result: Result<Vec<cli::RemoteEntry>, String>,
    },
    /// Cloud browser: a CLI mutation finished (refresh follows).
    CloudMutated {
        ok: bool,
        err: String,
        done: String,
        notify: Option<(String, String)>,
    },
    /// Cloud browser: a download finished.
    CloudDownload {
        ok: bool,
        err: String,
        name: String,
        n: usize,
    },
    /// Cloud browser: a file dialog returned (None = cancelled).
    CloudPicked(Option<Picked>),
    /// Cloud browser: no native dialog could be started.
    CloudPickFailed(String),
    /// Cloud browser: the move-to picker's folder listing landed.
    PickerFolders {
        parent: String,
        folders: Vec<(String, String)>,
    },
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
    UploadFiles {
        paths: Vec<PathBuf>,
        /// Remote folder that was open when the picker opened.
        target: String,
    },
    UploadFolder {
        dir: PathBuf,
        /// Remote folder that was open when the picker opened.
        target: String,
    },
    DownloadSave {
        rpath: String,
        dest: PathBuf,
    },
    DownloadDir {
        rpaths: Vec<String>,
        dest: PathBuf,
        n: usize,
    },
}

#[derive(Clone, Debug)]
pub enum Dialog {
    Info {
        title: String,
        body: String,
    },
    ConfirmSave,
    ConfirmLogout,
    PauseMenu,
    DirBrowse {
        cwd: PathBuf,
    },
    /// Text input (Cloud: rename / new folder); the buffer lives on App.
    Input {
        title: String,
        label: String,
        intent: InputIntent,
    },
    /// The F1 shortcut reference.
    Shortcuts,
    ConfirmTrash {
        n: usize,
    },
    /// Approve one exact, reviewed mass deletion for the next sync cycle.
    ConfirmHeldDeletes {
        local: u64,
        cloud: u64,
        local_digest: String,
        cloud_digest: String,
        sample: Vec<String>,
    },
    /// Permanently delete the trash selection (cannot be undone).
    ConfirmDelete {
        n: usize,
    },
    /// Empty the whole Proton trash (cannot be undone).
    ConfirmEmptyTrash,
    ConfirmClearCache,
    /// Move the selected photos to the Proton photos trash (also removes the
    /// local copies in the photo folder).
    ConfirmPhotoDelete {
        uids: Vec<String>,
    },
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
    CloudRename {
        old: String,
        name: String,
    },
    NewFolder(String),
    TrashConfirm,
    HeldDeleteConfirm {
        local: u64,
        cloud: u64,
        local_digest: String,
        cloud_digest: String,
    },
    DeleteConfirm,
    PhotoDeleteConfirm(Vec<String>),
    EmptyTrashConfirm,
    ClearCache,
    MoveSel(String),  // picker row clicked
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
    /// True while the user is dragging the pane divider.
    nav_drag: bool,
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
    pub photos: photos::PhotoState,
    /// Buffer of the open text-input dialog (rename / new folder).
    pub input_buf: String,
    pub input_focus: bool,
    /// Set by Ctrl+F; the Files search field takes focus on the next frame.
    pub search_focus: bool,

    pub cool: notify::Cooldowns,
    pub last_seen: Option<chrono::DateTime<chrono::FixedOffset>>,
    pub prev_hstatus: String,
    pub tray_warned: bool,

    pub dialog: Option<Dialog>,
    /// First frame of a dialog: focus the safe action once.
    dialog_seen: Option<std::mem::Discriminant<Dialog>>,
    dialog_focus: bool,
    pub save_hint: String,
    pub sub_override: Option<(String, f64)>,
    pub toasts: std::collections::VecDeque<Toast>,
    /// Prefs as of the last Save/startup; Discard restores this snapshot.
    pub prefs_baseline: Option<prefs::Prefs>,
    /// Selected Settings tab (General, Sync, Notifications, Appearance,
    /// Advanced, App). Kept on App so it survives a Save/reload.
    pub settings_tab: usize,
    pub upd: UpdateUi,

    pub hidden: bool,
    pub hide_on_start: bool,
    pub quitting: bool,
    pub style_dirty: bool,
    /// The desktop's colour scheme as last detected. Only used when the pref
    /// is `System`, which egui cannot resolve on Linux.
    pub system_theme: prefs::Theme,
    /// Live Appearance previews. They change the current frame but are only
    /// written to prefs by Save; closing the app discards them.
    pub preview_theme: Option<prefs::Theme>,
    pub preview_scale: Option<prefs::UiScale>,
    /// The zoom whose minimum window size has been applied. Changing it grows
    /// the window to the smallest layout that still fits every page.
    scale_applied: Option<f32>,
    /// New colour schemes from `system_theme::watch` (OS flips).
    theme_rx: mpsc::Receiver<prefs::Theme>,
    pub icon_tex: Option<egui::TextureHandle>,
    /// The system tray (None when the desktop has no tray host).
    pub tray: Option<tray::Tray>,
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
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    PathBuf::from(s)
}

/// Decode a PNG into (width, height, RGBA8) - used for the window and menu
/// icon, the sidebar texture and photo thumbnails.
///
/// EXPAND turns palette entries into RGB/RGBA, tRNS into alpha and sub-8-bit
/// samples into 8-bit; STRIP_16 drops 16-bit channels to 8-bit. Without this,
/// perfectly valid indexed wallpapers (small on disk, large in pixels) failed
/// with "Can't preview".
pub fn decode_png(bytes: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    if info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    let samples = info.color_type.samples();
    let data = buf.get(..w.checked_mul(h)?.checked_mul(samples)?)?;
    let rgba = match info.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => {
            let mut out = Vec::with_capacity(w * h * 4);
            for px in data.as_chunks::<3>().0 {
                out.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
            out
        }
        png::ColorType::Grayscale => {
            let mut out = Vec::with_capacity(w * h * 4);
            for &g in data {
                out.extend_from_slice(&[g, g, g, 255]);
            }
            out
        }
        png::ColorType::GrayscaleAlpha => {
            let mut out = Vec::with_capacity(w * h * 4);
            for px in data.as_chunks::<2>().0 {
                out.extend_from_slice(&[px[0], px[0], px[0], px[1]]);
            }
            out
        }
        // EXPAND should have converted palette entries; keep the guard.
        png::ColorType::Indexed => return None,
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
                "{}|{}|{}|{}|{}|{}|{}|{}|{}",
                h.ts.as_deref().unwrap_or(""),
                h.status,
                h.uploaded,
                h.downloaded,
                h.actions,
                h.transfer_done.unwrap_or(0),
                h.audit_phase.as_deref().unwrap_or(""),
                h.deletes_held,
                h.deletes_wipe_guard,
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
                if tx
                    .send(Back::Poll {
                        health: Box::new(h.clone()),
                        active,
                        enabled,
                    })
                    .is_err()
                {
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

/// Two-column shortcut reference: monospace keys, plain-language action.
fn shortcut_grid(ui: &mut egui::Ui, id: &str, rows: &[(&str, &str)]) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([20.0, 6.0])
        .show(ui, |ui| {
            for (keys, action) in rows {
                ui.label(
                    egui::RichText::new(*keys)
                        .monospace()
                        .strong()
                        .size(12.5),
                );
                ui.label(egui::RichText::new(*action).size(12.5).weak());
                ui.end_row();
            }
        });
}

/// Focus the safe/default action once when a dialog opens.
fn focus_first(pending: &mut bool, response: &egui::Response) {
    if *pending {
        response.request_focus();
        *pending = false;
    }
}

/// Account-popup action row: a full-width, focusable menu item with a
/// logout glyph. Danger rows hover with a red tint.
fn account_action(ui: &mut egui::Ui, label: &str, danger: bool) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 34.0), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
    if response.hovered() {
        let fill = if danger {
            theme::ERROR.gamma_multiply(0.14)
        } else {
            ui.visuals().widgets.hovered.weak_bg_fill
        };
        ui.painter()
            .rect_filled(rect, egui::CornerRadius::same(9), fill);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            egui::CornerRadius::same(9),
            egui::Stroke::new(1.5, theme::accent(ui.visuals())),
            egui::StrokeKind::Inside,
        );
    }
    let color = if danger {
        theme::danger_text(ui.visuals())
    } else {
        ui.visuals().strong_text_color()
    };
    // Door + arrow: the familiar sign-out glyph, drawn to the nav icon weight.
    let c = egui::pos2(rect.left() + 18.0, rect.center().y);
    let stroke = egui::Stroke::new(1.5, color);
    ui.painter().add(egui::Shape::line(
        vec![
            c + egui::vec2(6.0, -6.0),
            c + egui::vec2(1.0, -6.0),
            c + egui::vec2(1.0, 6.0),
            c + egui::vec2(6.0, 6.0),
        ],
        stroke,
    ));
    ui.painter().line_segment(
        [c + egui::vec2(1.0, 0.0), c + egui::vec2(12.0, 0.0)],
        stroke,
    );
    ui.painter().add(egui::Shape::line(
        vec![
            c + egui::vec2(8.0, -4.0),
            c + egui::vec2(12.0, 0.0),
            c + egui::vec2(8.0, 4.0),
        ],
        stroke,
    ));
    ui.painter().text(
        rect.left_center() + egui::vec2(36.0, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(13.0),
        color,
    );
    response.clicked() || (response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
}

/// One queued message. Errors stay until dismissed; info expires after four
/// seconds. A queue means a second message never silently replaces the first.
#[derive(Clone)]
pub struct Toast {
    pub msg: String,
    pub at: f64,
    pub error: bool,
    /// Set when this toast first reaches the front; the four-second info
    /// lifetime starts here, not at enqueue, so queued messages are readable.
    pub shown_at: Option<f64>,
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
        let upd_next_check = prefs
            .updates_auto()
            .then(|| std::time::Instant::now() + Duration::from_secs(5));
        theme::apply(&cc.egui_ctx, &prefs.theme(), prefs.motion_enabled());
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
            prefs_baseline: Some(prefs.clone()),
            settings_tab: 0,
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
            nav_drag: false,
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
            photos: photos::PhotoState::default(),
            input_buf: String::new(),
            input_focus: false,
            search_focus: false,
            cool: notify::Cooldowns::default(),
            last_seen: Some(chrono::Utc::now().into()),
            prev_hstatus: String::new(),
            tray_warned: false,
            dialog: None,
            dialog_seen: None,
            dialog_focus: false,
            save_hint: String::new(),
            sub_override: None,
            toasts: std::collections::VecDeque::new(),
            upd: UpdateUi {
                next_check: upd_next_check,
                ..Default::default()
            },
            hidden: false,
            quitting: false,
            style_dirty: false,
            system_theme: detected_theme,
            preview_theme: None,
            preview_scale: None,
            scale_applied: None,
            theme_rx,
            icon_tex,
            tray,
            wayland_fallback,
            relaunch_slot,
        }))
    }

    // ------------------------------------------------------------- status

    /// The theme to paint with: the live Appearance preview, else `System`
    /// using the desktop's last answer.
    fn resolved_theme(&self) -> prefs::Theme {
        let pref = self
            .preview_theme
            .clone()
            .unwrap_or_else(|| self.prefs.theme());
        system_theme::resolve(&pref, &self.system_theme)
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
        // A busy lock, locked keyring or network error is not a sign-out.
        // Leave the identity unknown so the orb shows Starting/Synced instead
        // of the welcome screen; `needs_login` from the engine still wins.
        let transient_identity_error = self
            .identity
            .as_ref()
            .and_then(|i| i.error.as_deref())
            .is_some_and(|e| !health::is_login_error(e));
        let identity = if transient_identity_error {
            None
        } else {
            self.identity.as_ref().map(|i| i.logged_in)
        };
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

    /// Account-scoped actions need a session. An unresolved probe is treated
    /// as allowed: it settles within a frame or two, and blocking the first
    /// click after launch would be worse than a possible failure toast.
    pub fn can_transfer(&self) -> bool {
        !matches!(
            self.ui_status(),
            health::UiStatus::SignedOut
                | health::UiStatus::NeedsLogin
                | health::UiStatus::SigningIn
        )
    }

    pub fn toast(&mut self, msg: impl Into<String>) {
        self.push_toast(msg.into(), false);
    }

    pub fn toast_error(&mut self, msg: impl Into<String>) {
        self.push_toast(msg.into(), true);
    }

    fn push_toast(&mut self, msg: String, error: bool) {
        if self.toasts.len() >= 4 {
            // Drop an old info before an unacknowledged error.
            let idx = self
                .toasts
                .iter()
                .position(|t| !t.error)
                .unwrap_or(0);
            self.toasts.remove(idx);
        }
        self.toasts.push_back(Toast {
            msg,
            at: self.now(),
            error,
            shown_at: None,
        });
    }

    fn now(&self) -> f64 {
        // Monotonic clock for the four-second toast lifetime.
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
        self.sub_override = Some((
            "On it - checking for new changes...".into(),
            self.now() + 8.0,
        ));
    }

    pub fn action_open_folder(&self) {
        let root = expand_user(
            self.conf
                .get("PROTON_SYNC_LOCAL")
                .map(String::as_str)
                .unwrap_or(""),
        );
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
        let root = expand_user(
            self.conf
                .get("PROTON_SYNC_LOCAL")
                .map(String::as_str)
                .unwrap_or(""),
        );
        let rel = rel.trim_matches('/');
        let target = if rel.is_empty() {
            root.clone()
        } else {
            root.join(rel)
        };
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
    /// tooltip and action enablement on every status refresh). The icon needs
    /// no timer any more: the dot's colour is the state, so one `update` per
    /// status change repaints it.
    fn tray_sync(&self) {
        let Some(tray) = &self.tray else { return };
        let status_key = self.status_key().to_string();
        let title = self.side_label().to_string();
        let active = self.active;
        let recent = tray::recent_labels(&self.changes);
        tray.update(move |t| t.apply(status_key, title, active, recent));
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
                        let _ = tx.send(Back::Identity(cli::fetch_identity_waiting(&c)));
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
        let state_dir = self.paths.state_dir.clone();
        std::thread::spawn(move || {
            if let Some(c) = cli::resolve_cli() {
                match cli::logout(&c) {
                    Ok(()) => {
                        for name in [
                            "state.json",
                            "remote-snapshot.json",
                            "deletes-approval.json",
                            "sync-roots",
                            "quota.json",
                            "photos-state.json",
                            "photos-sync.json",
                            "timeline.json",
                            "last-email",
                        ] {
                            let _ = std::fs::remove_file(state_dir.join(name));
                        }
                        let _ = tx.send(Back::AccountCleared);
                    }
                    Err(e) => {
                        let _ = tx.send(Back::Toast {
                            msg: format!("Could not sign out: {e}"),
                            error: true,
                        });
                    }
                }
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
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = service::ensure_service_unit(appimage.as_deref(), false);
            // Report the real outcome: `systemctl` can fail (no unit, no
            // systemd session) and the old code toasted success regardless.
            let _ = tx.send(Back::ServiceRestarted(service::restart_and_confirm()));
        });
        self.toast("Restarting syncing...");
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
        let Some(rel) = self.upd.available.clone() else {
            return;
        };
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
                Back::Poll {
                    health,
                    active,
                    enabled,
                } => {
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
                    let signed_in_now =
                        id.logged_in && self.identity.as_ref().is_some_and(|i| !i.logged_in);
                    if id.logged_in {
                        self.login_started = None;
                        if let Some(email) = id.email.clone() {
                            let _ = std::fs::write(self.paths.state_dir.join("last-email"), &email);
                            self.last_email = Some(email);
                        }
                        if was_signing_in || signed_in_now {
                            // A successful sign-in must start a cycle now, even
                            // when interval=0 left the engine waiting. Restart
                            // an active service so it picks up the fresh CLI
                            // session; never start one the user paused.
                            let _ = std::fs::write(&self.paths.wake_file, b"");
                            if !self.paused_by_user {
                                std::thread::spawn(|| {
                                    if service::unit_exists() {
                                        let _ = service::start();
                                    }
                                });
                            }
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
                        self.identity_retry = Some(Instant::now() + Duration::from_secs(10));
                    }
                }
                Back::Stats { files, bytes } => {
                    self.stats = Some((files, bytes));
                }
                Back::ShowWindow => self.show_from_tray(ctx),
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
                Back::UnitEnabled(b) => {
                    if !self.draft.autostart_touched {
                        self.draft.autostart = b;
                    }
                }
                Back::ServiceRestarted(ok) => {
                    if ok {
                        self.toast("Syncing restarted.");
                    } else {
                        self.toast_error(
                            "Could not restart syncing - run: systemctl --user restart proton-sync",
                        );
                    }
                }
                Back::Toast { msg, error } => {
                    if error {
                        self.toast_error(msg);
                    } else {
                        self.toast(msg);
                    }
                }
                Back::AccountCleared => {
                    cloud::reset(self);
                    self.draft.initial_selection.clear();
                    self.draft.children.clear();
                    self.draft.expanded.clear();
                    self.draft.loading.clear();
                    self.draft.nodes.clear();
                    self.draft.nodes.insert(String::new(), true);
                    self.toast("Signed out. The next account starts with a fresh cache.");
                }
                Back::Photos(r) => photos::on_fetched(self, r),
                Back::PhotosDownloaded(r) => photos::on_downloaded(self, r),
                Back::PhotosDeleted(r) => photos::on_deleted(self, r),
                Back::PhotoThumb {
                    name,
                    stamp,
                    result,
                } => {
                    self.photos.in_flight.remove(&name);
                    match result {
                        Ok((w, h, rgba)) => {
                            let tex = ctx.load_texture(
                                format!("photo-thumb-{name}"),
                                egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba),
                                egui::TextureOptions::LINEAR,
                            );
                            // Bound the cache: a big timeline must not keep
                            // every thumbnail on the GPU. Evict the least
                            // recently used instead of clearing them all, so
                            // scrolling does not re-decode the whole page.
                            if self.photos.tex.len() >= photos::THUMB_CACHE_MAX
                                && let Some(victim) = photos::lru_victim(
                                    self.photos.tex.iter().map(|(k, v)| (k.clone(), v.used)),
                                )
                            {
                                self.photos.tex.remove(&victim);
                            }
                            self.photos.tex.insert(
                                name,
                                photos::Thumb {
                                    stamp,
                                    used: self.photos.thumb_gen,
                                    tex,
                                },
                            );
                        }
                        Err(e) => {
                            let unsupported = e.starts_with("unsupported image format");
                            self.photos.failed.insert(name, (stamp, unsupported));
                        }
                    }
                }
                Back::CloudEntries {
                    key,
                    serial,
                    result,
                } => cloud::on_entries(self, key, serial, result),
                Back::TreeEntries { key, at, result } => {
                    cloud::on_tree_entries(self, key, at, result)
                }
                Back::CloudMutated {
                    ok,
                    err,
                    done,
                    notify,
                } => cloud::on_mutated(self, ok, err, done, notify),
                Back::CloudDownload { ok, err, name, n } => {
                    cloud::on_downloaded(self, ok, err, name, n)
                }
                Back::CloudPicked(p) => cloud::on_picked(self, p),
                Back::CloudPickFailed(msg) => self.toast_error(msg),
                Back::PickerFolders { parent, folders } => {
                    cloud::on_picker_folders(self, parent, folders)
                }
                Back::TrayToggle => {
                    let minimized = ctx.input(|i| i.viewport().minimized.unwrap_or(false));
                    if self.hidden || minimized {
                        self.show_from_tray(ctx);
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
                                    rel.cli,
                                    rel.gui,
                                    cur,
                                    crate::GUI_VERSION
                                );
                                self.upd.available = Some(rel.clone());
                                if self.tray.is_some() && self.cool.cool_ok("update", 6 * 3600) {
                                    notify::send(
                                        "Update available",
                                        &format!("Proton Drive {} is ready to install.", rel.tag),
                                    );
                                }
                            } else {
                                self.upd.label = if rel.published.is_empty() {
                                    format!("You're up to date (latest release: {}).", rel.tag)
                                } else {
                                    format!(
                                        "You're up to date (latest release: {}, {}).",
                                        rel.tag, rel.published
                                    )
                                };
                                self.upd.available = None;
                            }
                        }
                        Err(e) => {
                            self.upd.label = format!(
                                "Could not check for updates right now. {e}"
                            );
                            self.upd.available = None;
                        }
                    }
                }
                Back::UpdateInstalled(r) => {
                    self.upd.downloading = false;
                    match r {
                        Ok(target) => {
                            self.upd.label = "Updated - the new version is starting...".to_string();
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
        let identity_error = self.identity.as_ref().and_then(|i| i.error.as_deref());
        // A timeout/lock error is not a sign-out: retry it even when the
        // engine is not running, or a transient probe failure would leave a
        // signed-in user on the welcome screen for the whole session.
        let transient_error =
            identity_error.is_some_and(|e| !health::is_login_error(e));
        let probe_retryable = self.identity.as_ref().is_some_and(|i| {
            i.error
                .as_deref()
                .is_none_or(|e| !health::is_login_error(e))
        });
        if (keyring_issue || (active && engine_signed_in) || transient_error)
            && probe_retryable
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
            let verb = body.split_whitespace().next().unwrap_or("");
            if verb == "CONFLICT"
                && self.prefs.notify_conflicts()
                && self.cool.cool_ok("conflict", 30)
            {
                notify::send(
                    "Two versions were kept",
                    &format!("'{name}' changed in two places - both copies are safe."),
                );
            } else if matches!(verb, "ERROR" | "FAILED" | "REFUSE")
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
            // Store physical pixels at the desktop scale: egui points times
            // the interface zoom. Restoring that number as logical points
            // keeps the same physical window across zoom changes.
            let zoom = ctx.zoom_factor();
            let (w, h) = ((sz.x * zoom).round() as u32, (sz.y * zoom).round() as u32);
            if w != self.prefs.window_w || h != self.prefs.window_h {
                self.prefs.set_window_size(w, h);
            }
        }
    }

    fn hide_window(&mut self, ctx: &egui::Context, warn: bool) {
        self.hidden = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        if warn && !self.tray_warned {
            self.tray_warned = true;
            let message = if self.tray.is_some() {
                "Still running - syncing continues in the background. Right-click the tray \
                 icon to quit."
            } else {
                "Still running - syncing continues in the background. Start the app again \
                 to reopen this window."
            };
            notify::send("Proton Drive", message);
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
        self.hide_window(ctx, true);
    }

    /// Bring the window back from the tray (also used by notification clicks).
    /// On Hyprland this also floats it, like a launcher.
    fn show_from_tray(&mut self, ctx: &egui::Context) {
        self.hidden = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        crate::desktop::float_on_show();
    }

    fn quit(&mut self, ctx: &egui::Context) {
        self.remember_window_size(ctx);
        self.quitting = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    pub(crate) fn show_shortcuts(&mut self) {
        self.dialog = Some(Dialog::Shortcuts);
    }

    /// Global shortcuts. Typing in a field keeps every key except quit and
    /// help, so Ctrl+F does not steal focus while the search box is active.
    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Q)) {
            self.quit(ctx);
            return;
        }
        if self.dialog.is_some() {
            return;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::F1)) {
            self.show_shortcuts();
            return;
        }
        if ctx.text_edit_focused() {
            return;
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::A))
            && self.page == Page::Cloud
            && !self.cloud.trash_view
        {
            cloud::select_all_visible(self);
            return;
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::F)) {
            if self.cloud.trash_view {
                cloud::trash_toggle(self, false);
            }
            self.page = Page::Cloud;
            self.search_focus = true;
        }
    }

    // ------------------------------------------------------------- shell

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        // Wordmark: app icon + product name, with the platform line beneath.
        // The compact mark is used when the pane is too narrow for the text.
        ui.horizontal(|ui| {
            let compact = ui.available_width() < 162.0;
            let logo = if compact { 26.0 } else { 30.0 };
            if let Some(tex) = &self.icon_tex {
                ui.add(egui::Image::new((tex.id(), egui::vec2(logo, logo))));
            }
            if compact {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new("Proton Drive").strong().size(14.0),
                    )
                    .truncate(),
                );
            } else {
                ui.vertical(|ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new("Proton Drive").strong().size(14.5),
                        )
                        .truncate(),
                    );
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new("Linux · Community edition")
                                .size(10.5)
                                .weak(),
                        )
                        .truncate(),
                    );
                });
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
                // Status card, like the Proton sidebar's footer block.
                let status_card = egui::Frame::new()
                    .fill(theme::glass_fill(ui.visuals()))
                    .stroke(theme::glass_stroke(ui.visuals()))
                    .corner_radius(egui::CornerRadius::same(10))
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
                                egui::Label::new(
                                    egui::RichText::new(health::short_label(self.status_key()))
                                        .size(13.0),
                                )
                                .truncate(),
                            );
                        });
                        ui.add_space(2.0);
                        let when = self
                            .health
                            .ts_dt()
                            .map(|dt| {
                                format!("Updated {}", util::rel_time(Some(dt)))
                            })
                            .unwrap_or_else(|| "Community edition".to_string());
                        ui.add(
                            egui::Label::new(egui::RichText::new(when).size(11.5).weak())
                                .truncate(),
                        );
                    });
                let status_resp = ui
                    .interact(
                        status_card.response.rect,
                        ui.scope_id().with("sidebar-status"),
                        egui::Sense::click(),
                    )
                    .on_hover_text("View sync activity");
                status_resp.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::Role::Button,
                        ui.is_enabled(),
                        "Sync status",
                    )
                });
                if status_resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if status_resp.has_focus() {
                    ui.painter().rect_stroke(
                        status_card.response.rect,
                        egui::CornerRadius::same(10),
                        egui::Stroke::new(1.5, theme::accent(ui.visuals())),
                        egui::StrokeKind::Inside,
                    );
                }
                let activate = status_resp.clicked()
                    || (status_resp.has_focus()
                        && ui.input(|i| {
                            i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Space)
                        }));
                if activate {
                    self.page = Page::Activity;
                }
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
            "Signed out"
        } else {
            "Checking\u{2026}"
        };
        let signed_in = !matches!(
            self.ui_status(),
            health::UiStatus::SignedOut
                | health::UiStatus::NeedsLogin
                | health::UiStatus::SigningIn
        ) && (self.logged_in() || self.last_email.is_some());
        let hint = if signed_in {
            "Account \u{b7} click to sign out"
        } else {
            "Account \u{b7} click to sign in"
        };
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 46.0), egui::Sense::click());
        let account_name = label.to_string();
        let hover = match email.as_deref() {
            Some(e) => format!("{e}\n{hint}"),
            None => hint.to_string(),
        };
        let response = response.on_hover_text(hover);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::Role::Button,
                ui.is_enabled(),
                format!("Account menu - {account_name}"),
            )
        });
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
        let ch = email
            .as_deref()
            .and_then(|e| e.chars().next())
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_else(|| "?".into());
        let c = egui::pos2(rect.left() + 22.0, rect.center().y);
        widgets::avatar(ui, c, 15.0, &ch);
        // Presence dot: the live status colour with a ring.
        let dot = c + egui::vec2(10.0, 10.0);
        ui.painter()
            .circle_filled(dot, 4.5, theme::status_color(self.status_key()));
        ui.painter()
            .circle_stroke(dot, 4.5, egui::Stroke::new(1.5, ui.visuals().panel_fill));
        ui.painter().text(
            rect.left_center() + egui::vec2(48.0, 0.0),
            egui::Align2::LEFT_CENTER,
            // Width-aware elision: a fixed character count overflowed the
            // pill (and its chevron) on a narrow pane.
            widgets::elide(
                label,
                ((rect.width() - 80.0) / 6.6).clamp(4.0, 24.0) as usize,
            ),
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

        egui::Popup::menu(&response)
            .gap(8.0)
            .frame(
                egui::Frame::popup(ui.style())
                    .fill(ui.visuals().window_fill)
                    .stroke(theme::glass_stroke(ui.visuals()))
                    .corner_radius(egui::CornerRadius::same(14))
                    .inner_margin(egui::Margin::same(12)),
            )
            .show(|ui| {
                // Pin the width: the full-width CTA would otherwise ask for
                // the whole screen and stretch the popup across the page.
                ui.set_width(220.0);
                ui.horizontal(|ui| {
                    let (av, _) =
                        ui.allocate_exact_size(egui::vec2(34.0, 34.0), egui::Sense::hover());
                    widgets::avatar(ui, av.center(), 16.0, &ch);
                    ui.add_space(4.0);
                    ui.vertical(|ui| {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(email.as_deref().unwrap_or(label))
                                    .strong()
                                    .size(13.0),
                            )
                            .truncate(),
                        );
                        ui.add_space(1.0);
                        ui.horizontal(|ui| {
                            let (d, _) = ui
                                .allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                            ui.painter().circle_filled(
                                d.center(),
                                4.0,
                                theme::status_color(self.status_key()),
                            );
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(self.side_label()).size(11.5).weak(),
                                )
                                .truncate(),
                            );
                        });
                    });
                });
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(6.0);
                if signed_in {
                    if account_action(ui, "Sign out", true) {
                        self.dialog = Some(Dialog::ConfirmLogout);
                        ui.close();
                    }
                } else if widgets::primary_button_full(ui, "Sign in") {
                    self.action_login();
                    ui.close();
                }
            });
    }

    fn central(&mut self, ui: &mut egui::Ui) {
        let page = if ui.visuals().dark_mode {
            theme::DARK_PAGE
        } else {
            theme::PAGE
        };
        egui::CentralPanel::default()
            .frame(
                egui::Frame::central_panel(ui.style())
                    .fill(page)
                    .inner_margin(egui::Margin::symmetric(30, 20)),
            )
            .show(ui, |ui| match self.page {
                Page::Home => home::draw(self, ui),
                Page::Cloud => cloud::draw(self, ui),
                Page::Photos => photos::draw(self, ui),
                Page::Activity => activity::draw(self, ui, "Activity"),
                Page::Settings => settings::draw(self, ui),
                Page::About => about::draw(self, ui),
            });
    }

    fn toast_ui(&mut self, ctx: &egui::Context) {
        // Same clock as `toast()`; ctx.time starts at app launch, so mixing
        // the two made most toasts expire on their first frame.
        let t = self.now();
        if let Some(front) = self.toasts.front_mut()
            && front.shown_at.is_none()
        {
            front.shown_at = Some(t);
        }
        if let Some(front) = self.toasts.front()
            && !front.error
            && t - front.shown_at.unwrap_or(front.at) > 4.0
        {
            self.toasts.pop_front();
            ctx.request_repaint();
            return;
        }
        let Some(front) = self.toasts.front() else {
            return;
        };
        let (msg, since, error) = (
            front.msg.clone(),
            front.shown_at.unwrap_or(front.at),
            front.error,
        );
        let age = (t - since) as f32;
        let animate = self.prefs.motion_enabled();
        // Errors stay until dismissed; info slides in and fades out. With
        // motion off the toast appears and leaves without the slide/fade.
        let (fade, slide) = if error || !animate {
            (1.0, 0.0)
        } else if age < 0.15 {
            (age / 0.15, (1.0 - age / 0.15) * 14.0)
        } else if age > 3.7 {
            ((4.0 - age) / 0.3, 0.0)
        } else {
            (1.0, 0.0)
        };
        if !error && animate {
            ctx.request_repaint();
        }
        let mut dismiss = false;
        let area = egui::Area::new(egui::Id::unique("toast"))
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-18.0, -18.0 + slide))
            .show(ctx, |ui| {
                ui.set_opacity(fade.clamp(0.0, 1.0));
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let resp = ui.label(&msg);
                        if error {
                            resp.widget_info(|| {
                                egui::WidgetInfo::labeled(egui::Role::Alert, true, &msg)
                            });
                            ui.ctx().accesskit_node_builder(resp.id, |n| {
                                n.set_live(egui::accesskit::Live::Assertive);
                            });
                            if ui.small_button("Dismiss").clicked() {
                                dismiss = true;
                            }
                        } else {
                            resp.widget_info(|| {
                                egui::WidgetInfo::labeled(egui::Role::Status, true, &msg)
                            });
                            ui.ctx().accesskit_node_builder(resp.id, |n| {
                                n.set_live(egui::accesskit::Live::Polite);
                            });
                        }
                    });
                });
            });
        // Hover pauses an info toast; Escape dismisses an error when no dialog
        // is using the key.
        let hovered = ctx
            .pointer_hover_pos()
            .is_some_and(|p| area.response.rect.contains(p));
        if hovered
            && !error
            && let Some(front) = self.toasts.front_mut()
        {
            front.shown_at = Some(t);
        }
        if error && self.dialog.is_none() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            dismiss = true;
        }
        if dismiss {
            self.toasts.pop_front();
        }
    }

    fn dialog_ui(&mut self, ctx: &egui::Context) {
        let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let Some(dlg) = self.dialog.clone() else {
            self.dialog_seen = None;
            self.dialog_focus = false;
            return;
        };
        let disc = std::mem::discriminant(&dlg);
        if self.dialog_seen != Some(disc) {
            self.dialog_seen = Some(disc);
            self.dialog_focus = true;
        }
        let mut focus_pending = self.dialog_focus;
        // Modal input blocker: pointer clicks and drags stay out of the page
        // while a dialog is up. The dialog windows are Foreground, above this.
        egui::Area::new(egui::Id::unique("dialog-blocker"))
            .order(egui::Order::Middle)
            .fixed_pos(egui::Pos2::ZERO)
            .show(ctx, |ui| {
                let screen = ctx.viewport_rect();
                // Block pointer input but stay out of the Tab order: CLICK |
                // DRAG without FOCUSABLE.
                let resp = ui.allocate_rect(screen, egui::Sense::CLICK | egui::Sense::DRAG);
                resp.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::Role::GenericContainer,
                        false,
                        "Dialog open",
                    )
                });
                ui.painter()
                    .rect_filled(screen, 0, egui::Color32::from_black_alpha(50));
            });
        let mut open = true;
        let mut act: Option<DlgAct> = None;

        match &dlg {
            Dialog::Info { title, body } => {
                egui::Window::new(title.clone())
                    .collapsible(false)
                    .order(egui::Order::Foreground)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .open(&mut open)
                    .show(ctx, |ui| {
                        ui.label(body.clone());
                        ui.add_space(10.0);
                        let ok = widgets::primary_button_response(ui, "OK");
                        focus_first(&mut focus_pending, &ok);
                        if ok.clicked() || escape {
                            act = Some(DlgAct::Close);
                        }
                    });
            }
            Dialog::Shortcuts => {
                egui::Window::new("Keyboard shortcuts")
                    .collapsible(false)
                    .order(egui::Order::Foreground)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .open(&mut open)
                    .show(ctx, |ui| {
                        ui.label(
                            egui::RichText::new("Anywhere")
                                .strong()
                                .size(12.5),
                        );
                        ui.add_space(4.0);
                        shortcut_grid(
                            ui,
                            "shortcut-anywhere",
                            &[
                                ("Ctrl+Q", "Quit"),
                                ("F1", "Open this list"),
                                ("Esc", "Close a dialog"),
                            ],
                        );
                        ui.add_space(12.0);
                        ui.label(
                            egui::RichText::new("In My Files")
                                .strong()
                                .size(12.5),
                        );
                        ui.add_space(4.0);
                        shortcut_grid(
                            ui,
                            "shortcut-files",
                            &[
                                ("Ctrl+F", "Search files"),
                                ("Ctrl+A", "Select all visible files"),
                                ("Delete", "Move the selection to trash"),
                                ("F2", "Rename the selected item"),
                                ("Enter", "Open the focused folder or file"),
                                ("Space", "Select or deselect the focused file"),
                            ],
                        );
                        ui.add_space(12.0);
                        let ok = widgets::primary_button_response(ui, "OK");
                        focus_first(&mut focus_pending, &ok);
                        if ok.clicked() || escape {
                            act = Some(DlgAct::Close);
                        }
                    });
            }
            Dialog::ConfirmSave => {
                egui::Window::new("Save changes")
                    .collapsible(false)
                    .order(egui::Order::Foreground)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .open(&mut open)
                    .show(ctx, |ui| {
                        ui.label("Save your settings now?");
                        if settings::root_change_pending(self) {
                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new(
                                    "The sync folder or remote root changed. Saving archives \
                                     the current sync history and clears cached cloud rows \
                                     before the next sync.",
                                )
                                .size(12.5)
                                .weak(),
                            );
                        }
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            let cancel = ui.button("Cancel");
                            focus_first(&mut focus_pending, &cancel);
                            if cancel.clicked() || escape {
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
                    .order(egui::Order::Foreground)
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
                            let cancel = ui.button("Cancel");
                            focus_first(&mut focus_pending, &cancel);
                            if cancel.clicked() || escape {
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
                    .order(egui::Order::Foreground)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .open(&mut open)
                    .show(ctx, |ui| {
                        let first = ui.button("Pause for 15 minutes");
                        focus_first(&mut focus_pending, &first);
                        if first.clicked() {
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
                    .order(egui::Order::Foreground)
                    .resizable(false)
                    .default_size([420.0, 480.0])
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .open(&mut open)
                    .show(ctx, |ui| {
                        ui.label(
                            egui::RichText::new(widgets::elide(&cwd.to_string_lossy(), 70)).weak(),
                        );
                        ui.add_space(6.0);
                        egui::ScrollArea::vertical()
                            .max_height(340.0)
                            .show(ui, |ui| {
                                if let Some(parent) = cwd.parent()
                                    && ui.button("\u{2191}  Go up").clicked()
                                {
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
                            let cancel = ui.button("Cancel");
                            focus_first(&mut focus_pending, &cancel);
                            if cancel.clicked() || escape {
                                act = Some(DlgAct::Close);
                            }
                            if widgets::primary_button(ui, "Choose this folder") {
                                act = Some(DlgAct::BrowseSet(cwd.clone()));
                            }
                        });
                    });
            }
            Dialog::Input {
                title,
                label,
                intent,
                ..
            } => {
                let title = title.clone();
                let label = label.clone();
                let intent = intent.clone();
                egui::Window::new(title)
                    .collapsible(false)
                    .order(egui::Order::Foreground)
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
                            focus_pending = false;
                        }
                        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            let cancel = ui.button("Cancel");
                            focus_first(&mut focus_pending, &cancel);
                            if cancel.clicked() || escape {
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
                    .order(egui::Order::Foreground)
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
                            let cancel = ui.button("Cancel");
                            focus_first(&mut focus_pending, &cancel);
                            if cancel.clicked() || escape {
                                act = Some(DlgAct::Close);
                            }
                            if widgets::primary_button(ui, "Move to trash") {
                                act = Some(DlgAct::TrashConfirm);
                            }
                        });
                    });
            }
            Dialog::ConfirmHeldDeletes {
                local,
                cloud,
                local_digest,
                cloud_digest,
                sample,
            } => {
                let total = local + cloud;
                egui::Window::new(format!("Delete {total} files?"))
                    .collapsible(false)
                    .order(egui::Order::Foreground)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .open(&mut open)
                    .show(ctx, |ui| {
                        if *local > 0 {
                            ui.label(format!(
                                "{local} files are missing from the cloud and will be moved \
                                 to this computer's trash. You can restore them until it is emptied."
                            ));
                        }
                        if *cloud > 0 {
                            ui.label(format!(
                                "{cloud} files are missing on this computer and will be moved \
                                 to the Proton trash. You can restore them in any Proton Drive app."
                            ));
                        }
                        if !sample.is_empty() {
                            ui.add_space(4.0);
                            ui.label(format!("Includes: {}.", sample.join(", ")));
                        }
                        ui.add_space(8.0);
                        ui.label("This approval applies once to this exact set of files.");
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            let cancel = ui.button("Cancel");
                            focus_first(&mut focus_pending, &cancel);
                            if cancel.clicked() || escape {
                                act = Some(DlgAct::Close);
                            }
                            if widgets::danger_ghost_button(ui, &format!("Delete {total} files")) {
                                act = Some(DlgAct::HeldDeleteConfirm {
                                    local: *local,
                                    cloud: *cloud,
                                    local_digest: local_digest.clone(),
                                    cloud_digest: cloud_digest.clone(),
                                });
                            }
                        });
                    });
            }
            Dialog::ConfirmPhotoDelete { uids } => {
                let n = uids.len();
                let word = if n == 1 {
                    "this photo".to_string()
                } else {
                    format!("these {n} photos")
                };
                egui::Window::new("Move to the photos trash?")
                    .collapsible(false)
                    .order(egui::Order::Foreground)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .open(&mut open)
                    .show(ctx, |ui| {
                        ui.label(format!(
                            "Move {word} to the Proton photos trash? They leave the \
                             timeline, and their copies in the photo folder are removed \
                             too. You can restore them from Photos in any Proton app."
                        ));
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            let cancel = ui.button("Cancel");
                            focus_first(&mut focus_pending, &cancel);
                            if cancel.clicked() || escape {
                                act = Some(DlgAct::Close);
                            }
                            if widgets::primary_button(ui, "Move to trash") {
                                act = Some(DlgAct::PhotoDeleteConfirm(uids.clone()));
                            }
                        });
                    });
            }
            Dialog::ConfirmDelete { n } => {
                let word = if *n == 1 {
                    "this item".to_string()
                } else {
                    format!("these {n} items")
                };
                egui::Window::new("Delete permanently?")
                    .collapsible(false)
                    .order(egui::Order::Foreground)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .open(&mut open)
                    .show(ctx, |ui| {
                        ui.label(format!("Permanently delete {word}? This cannot be undone."));
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            let cancel = ui.button("Cancel");
                            focus_first(&mut focus_pending, &cancel);
                            if cancel.clicked() || escape {
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
                    .order(egui::Order::Foreground)
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
                            let cancel = ui.button("Cancel");
                            focus_first(&mut focus_pending, &cancel);
                            if cancel.clicked() || escape {
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
                    .order(egui::Order::Foreground)
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
                            let cancel = ui.button("Cancel");
                            focus_first(&mut focus_pending, &cancel);
                            if cancel.clicked() || escape {
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
                    .order(egui::Order::Foreground)
                    .resizable(false)
                    .default_size([430.0, 480.0])
                    .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                    .open(&mut open)
                    .show(ctx, |ui| {
                        egui::ScrollArea::vertical()
                            .max_height(360.0)
                            .show(ui, |ui| {
                                self.picker_row(ui, "", &format!("My Files ({root})"), 0, &mut act);
                            });
                        ui.add_space(10.0);
                        let can = self.cloud.picker.as_ref().is_some_and(|p| p.sel.is_some());
                        ui.horizontal(|ui| {
                            let cancel = ui.button("Cancel");
                            focus_first(&mut focus_pending, &cancel);
                            if cancel.clicked() || escape {
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
            Some(a) => self.apply_dialog(a, ctx),
            None if !open => self.dialog = None,
            None => {}
        }
        // One shot: a dialog gets focus on its first frame, not every frame.
        self.dialog_focus = false;
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
                Some(p) => (p.expanded.contains(rel), p.sel.as_deref() == Some(rel)),
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
    fn apply_dialog(&mut self, act: DlgAct, ctx: &egui::Context) {
        match act {
            DlgAct::Close => {
                if matches!(self.dialog, Some(Dialog::CloudMove)) {
                    self.cloud.picker = None;
                }
                self.dialog = None;
            }
            DlgAct::Save => {
                self.dialog = None;
                settings::do_save(self, ctx);
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
            DlgAct::HeldDeleteConfirm {
                local,
                cloud,
                local_digest,
                cloud_digest,
            } => {
                self.dialog = None;
                let approval = proton_core::delete_approval::DeleteApproval {
                    local,
                    cloud,
                    local_digest,
                    cloud_digest,
                    created_at: proton_core::delete_approval::now_secs(),
                    local_root: self
                        .conf
                        .get("PROTON_SYNC_LOCAL")
                        .cloned()
                        .unwrap_or_default(),
                    remote_root: self
                        .conf
                        .get("PROTON_SYNC_REMOTE")
                        .cloned()
                        .unwrap_or_default(),
                };
                let path = self.paths.state_dir.join("deletes-approval.json");
                match proton_core::delete_approval::write(&path, &approval) {
                    Ok(()) => {
                        let _ = std::fs::write(&self.paths.wake_file, b"");
                        self.toast(format!(
                            "Approved {} deletion(s) once. Syncing now.",
                            local + cloud
                        ));
                    }
                    Err(e) => self.toast_error(format!("Could not approve the deletion: {e}")),
                }
            }
            DlgAct::PhotoDeleteConfirm(uids) => {
                self.dialog = None;
                photos::delete(self, uids);
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
                    Err(e) => self.toast_error(format!("Could not clear the snapshot cache: {e}")),
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
            .store(matches!(self.page, Page::Home | Page::Activity) && !self.hidden, Ordering::Relaxed);
        self.drain(ctx);
        self.drain_theme(ctx);

        // Re-probe sign-in after a keyring-lock report (see on_poll).
        if let Some(t) = self.identity_retry
            && Instant::now() >= t
        {
            self.identity_retry = None;
            let tx = self.tx.clone();
            let waiting = self
                .login_started
                .is_some_and(|t| t.elapsed() < Duration::from_secs(300))
                && !self.logged_in();
            std::thread::spawn(move || {
                if let Some(c) = cli::resolve_cli() {
                    let identity = if waiting {
                        cli::fetch_identity_waiting(&c)
                    } else {
                        cli::fetch_identity(&c)
                    };
                    let _ = tx.send(Back::Identity(identity));
                }
            });
        }

        // Cloud: ensure the initial listing even if the page was reached
        // without a nav click (e.g. it becomes the startup page one day).
        if self.page == Page::Cloud && !self.cloud.opened {
            self.cloud.opened = true;
            cloud::on_shown(self);
        }

        // Photos: same idea - the timeline is fetched once, on first show,
        // then re-read whenever the folder or the engine's summary changes.
        if self.page == Page::Photos {
            photos::on_shown(self);
            photos::tick(self);
        }

        // Updates: automatic check 5s after start, then every 6h (Qt's
        // updates_timer); the ETag cache keeps this free of rate limits.
        if let Some(t) = self.upd.next_check
            && std::time::Instant::now() >= t
        {
            self.upd.next_check = Some(std::time::Instant::now() + Duration::from_secs(6 * 3600));
            if self.prefs.updates_auto() {
                self.action_check_updates();
            }
        }
        // After an update swap: quit once the status label has been seen.
        if let Some(t) = self.upd.quit_at
            && std::time::Instant::now() >= t
        {
            self.upd.quit_at = None;
            self.quit(ctx);
        }

        if self.hide_on_start {
            self.hide_on_start = false;
            if hides_to_tray(true, self.tray.is_some()) {
                // Starting quietly is not a user close; save the tray hint
                // for the first real close.
                self.hide_window(ctx, false);
            } else {
                // main.rs starts the viewport hidden when start-in-tray is
                // set. With no tray there is no way back, so show it now.
                self.show_from_tray(ctx);
            }
        }

        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if hides_to_tray(self.prefs.close_to_tray(), self.tray.is_some()) {
                self.hide_to_tray(ctx);
            } else {
                self.quit(ctx);
            }
        }

        if self.style_dirty {
            // Re-read now: the user just picked a theme, and "Match my
            // system" must reflect the desktop this second, not the value
            // the watcher last saw.
            let pref = self
                .preview_theme
                .clone()
                .unwrap_or_else(|| self.prefs.theme());
            if pref == prefs::Theme::System {
                self.system_theme = system_theme::detect();
            }
            theme::apply(ctx, &pref, self.prefs.motion_enabled());
            theme::force(ctx, &self.resolved_theme());
            self.style_dirty = false;
        }

        // Keep the interface scale in sync with the live preview or the pref.
        let want_zoom = self
            .preview_scale
            .unwrap_or_else(|| self.prefs.ui_scale())
            .factor();
        if (ctx.zoom_factor() - want_zoom).abs() > 0.001 {
            ctx.set_zoom_factor(want_zoom);
        }
        // A larger interface needs a larger physical window, or the fixed
        // sidebar and three-column cards collide. The minimum is in egui
        // points, so egui-winit multiplies it by the current pixels-per-point
        // (desktop scale * zoom). Cap it to the monitor so a 200% setting on a
        // small screen opens full-screen instead of off-screen.
        if self.scale_applied != Some(want_zoom) {
            self.scale_applied = Some(want_zoom);
            let monitor = ctx.input(|i| i.viewport().monitor_size);
            let min = match monitor {
                Some(m) if m.x > 1.0 && m.y > 1.0 => {
                    egui::vec2(720.0_f32.min(m.x), 400.0_f32.min(m.y))
                }
                _ => egui::vec2(720.0, 400.0),
            };
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(min));
            if let Some(size) = ctx.input(|i| i.viewport().inner_rect.map(|r| r.size())) {
                let target = egui::vec2(size.x.max(min.x), size.y.max(min.y));
                if target.x > size.x + 0.5 || target.y > size.y + 0.5 {
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(target));
                }
            }
        }

        // Awake once a second for relative times and the status pollers. The
        // poller does not own the egui context, so this is also what delivers
        // a second-launch "show window" poke while the app sits in the tray.
        ctx.request_repaint_after(Duration::from_millis(900));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.handle_shortcuts(ui.ctx());
        // A dialog owns the keyboard: disabling the page behind it keeps Tab
        // inside the dialog, and the modal blocker already owns the pointer.
        if self.dialog.is_some() {
            ui.disable();
        }
        // Drag-and-drop upload: files dropped on My Files upload into the
        // folder that is currently open.
        if self.dialog.is_none()
            && self.page == Page::Cloud
            && !self.cloud.trash_view
            && !self.cloud.busy
            && self.can_transfer()
        {
            let dropped: Vec<std::path::PathBuf> = ui.ctx().input(|i| {
                i.raw
                    .dropped_files
                    .iter()
                    .map(|f| f.path().to_path_buf())
                    .collect()
            });
            if !dropped.is_empty() {
                cloud::on_picked(
                    self,
                    Some(Picked::UploadFiles {
                        paths: dropped,
                        target: self.cloud.rel.clone(),
                    }),
                );
            }
        }
        // Solid page backdrop: hierarchy comes from surfaces and borders,
        // not from a decorative animated background.
        // White sidebar against the neutral content page.
        // The pane's right edge is draggable (176-400 px); 216.0
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
            [
                egui::pos2(prect.right(), prect.top() + 8.0),
                egui::pos2(prect.right(), prect.bottom() - 8.0),
            ],
            egui::Stroke::new(1.5, sep),
        );
        if hovering {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }

        // Remember the width only after a real divider drag. Persisting any
        // difference also saved the width whenever a small window clamped the
        // pane, permanently losing the user's chosen size.
        let pressed = ui.input(|i| i.pointer.primary_pressed());
        if hovering && pressed {
            self.nav_drag = true;
        }
        let released = ui.input(|i| i.pointer.primary_released() || !i.pointer.any_down());
        if self.nav_drag && released {
            let w = prect.width();
            if (w - self.nav_width).abs() > 0.5 {
                self.nav_width = w;
                self.prefs.set_nav_width(w);
            }
            self.nav_drag = false;
        }

        self.central(ui);

        let ctx = ui.ctx().clone();
        if self.dialog.is_none() {
            self.toast_ui(&ctx);
        }
        self.dialog_ui(&ctx);
    }
}

/// A close/start-in-tray request only hides when a tray actually exists to
/// restore the window from. On a tray-less desktop, closing must quit.
fn hides_to_tray(enabled: bool, tray_available: bool) -> bool {
    enabled && tray_available
}

#[cfg(test)]
mod tray_policy_tests {
    use super::hides_to_tray;

    #[test]
    fn hiding_requires_a_real_tray() {
        assert!(hides_to_tray(true, true));
        assert!(!hides_to_tray(true, false));
        assert!(!hides_to_tray(false, true));
        assert!(!hides_to_tray(false, false));
    }
}

#[cfg(test)]
mod glyph_tests {
    use eframe::egui;
    use egui::FontId;
    use egui::epaint::text::{FontDefinitions, FontProvider, Fonts, TextOptions};
    use std::sync::Arc;

    /// Every character the UI paints as an icon rather than as text.
    ///
    /// Most of them do not exist in the fonts this app bundles (Ubuntu-Light,
    /// Hack, egui-icons), so they only render through the system font fallback
    /// that eframe's `system_fonts` feature installs. 3.16.19 dropped that
    /// feature while switching to glow, and these turned into blank boxes: the
    /// pane carets, the sort arrow, the tri-state boxes, the upload/download
    /// arrows, the folder emoji, and the info icon.
    const ICON_GLYPHS: &[char] = &[
        '\u{25b8}',  // ▸ collapsed caret
        '\u{25be}',  // ▾ expanded caret
        '\u{25bc}',  // ▼ sort descending
        '\u{25b2}',  // ▲ sort ascending
        '\u{2611}',  // ☑ folder fully selected
        '\u{2610}',  // ☐ folder not selected
        '\u{25d0}',  // ◐ folder partly selected
        '\u{2191}',  // ↑ upload
        '\u{2193}',  // ↓ download
        '\u{1f4c1}', // 📁 local folder card
        '\u{1f4c2}', // 📂 selective-sync folder card
        '\u{2139}',  // ℹ info icon
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
mod png_decode_tests {
    use super::decode_png;

    fn encode(
        w: u32,
        h: u32,
        color: png::ColorType,
        depth: png::BitDepth,
        palette: Option<Vec<u8>>,
        trns: Option<Vec<u8>>,
        data: &[u8],
    ) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(color);
            enc.set_depth(depth);
            if let Some(p) = palette {
                enc.set_palette(p);
            }
            if let Some(t) = trns {
                enc.set_trns(t);
            }
            let mut writer = enc.write_header().unwrap();
            writer.write_image_data(data).unwrap();
        }
        out
    }

    #[test]
    fn indexed_pngs_decode_with_palette_and_alpha() {
        // 8-bit indexed: two pixels, red then green.
        let png = encode(
            2,
            1,
            png::ColorType::Indexed,
            png::BitDepth::Eight,
            Some(vec![255, 0, 0, 0, 255, 0]),
            None,
            &[0, 1],
        );
        let (w, h, rgba) = decode_png(&png).expect("indexed 8-bit");
        assert_eq!((w, h), (2, 1));
        assert_eq!(&rgba[..8], &[255, 0, 0, 255, 0, 255, 0, 255]);

        // 4-bit indexed with transparency: four pixels packed two per byte.
        let png = encode(
            4,
            1,
            png::ColorType::Indexed,
            png::BitDepth::Four,
            Some(vec![0, 0, 0, 255, 255, 255]),
            Some(vec![128]),
            &[0x01, 0x01],
        );
        let (w, h, rgba) = decode_png(&png).expect("indexed 4-bit");
        assert_eq!((w, h), (4, 1));
        assert_eq!(&rgba[..8], &[0, 0, 0, 128, 255, 255, 255, 255]);
    }

    #[test]
    fn grayscale_and_16_bit_pngs_decode() {
        let png = encode(
            2,
            1,
            png::ColorType::Grayscale,
            png::BitDepth::Eight,
            None,
            None,
            &[10, 200],
        );
        let (_, _, rgba) = decode_png(&png).expect("gray");
        assert_eq!(&rgba[..8], &[10, 10, 10, 255, 200, 200, 200, 255]);

        let png = encode(
            2,
            1,
            png::ColorType::GrayscaleAlpha,
            png::BitDepth::Eight,
            None,
            None,
            &[10, 128, 200, 255],
        );
        let (_, _, rgba) = decode_png(&png).expect("gray alpha");
        assert_eq!(&rgba[..8], &[10, 10, 10, 128, 200, 200, 200, 255]);

        // 16-bit RGB is stripped to its high byte.
        let png = encode(
            1,
            1,
            png::ColorType::Rgb,
            png::BitDepth::Sixteen,
            None,
            None,
            &[0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc],
        );
        let (_, _, rgba) = decode_png(&png).expect("16-bit");
        assert_eq!(&rgba[..4], &[0x12, 0x56, 0x9a, 255]);
    }

    #[test]
    fn garbage_is_not_a_png() {
        assert!(decode_png(b"not a png").is_none());
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
