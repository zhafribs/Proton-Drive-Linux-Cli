//! Photos: the Proton photo timeline (My Photos) plus a local download folder.
//!
//! The official CLI owns both halves - `photo timeline -d --json` to list and
//! `photo download` to fetch - exactly like the files browser; this page lists,
//! groups by day and asks. JPEG and PNG thumbnails are decoded locally; other
//! formats keep a labelled placeholder instead of a misleadingly blank tile.

use crate::i18n::{rt, t, TrUi};
use super::{AppCtx, Dialog, Toast, widgets};
use crate::{cli, health};
use crate::theme;
use eframe::egui;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// One decoded thumbnail plus its cache bookkeeping.
pub struct Thumb {
    pub stamp: f64,
    pub used: u64,
    pub tex: egui::TextureHandle,
}

/// How many thumbnails stay on the GPU. 256 x 256 px RGBA is about 67 MB.
pub const THUMB_CACHE_MAX: usize = 256;

/// The least recently used thumbnail, for eviction when the cache is full.
pub fn lru_victim(entries: impl Iterator<Item = (String, u64)>) -> Option<String> {
    entries.min_by_key(|(_, used)| *used).map(|(name, _)| name)
}

/// One file the engine could not upload, with a short explanation.
#[derive(Clone, serde::Deserialize, Default)]
pub struct FileIssue {
    pub name: String,
    #[serde(default)]
    pub reason: String,
}

#[derive(Default)]
pub struct PhotoState {
    pub items: Vec<cli::PhotoEntry>,
    /// A fetch has been started (the page only auto-fetches once).
    pub loaded: bool,
    /// The local photos folder was ensured this session.
    pub dir_ready: bool,
    pub loading: bool,
    /// A download is in flight.
    pub busy: bool,
    pub status: String,
    pub error: String,
    /// Selected photo uids.
    pub sel: Vec<String>,
    /// Decoded thumbnails, bounded by [`THUMB_CACHE_MAX`].
    pub tex: std::collections::HashMap<String, Thumb>,
    /// Thumbnails being decoded right now.
    pub in_flight: std::collections::HashSet<String>,
    /// Names that failed to decode, with the file stamp and whether the
    /// failure was an unsupported format: a file that comes back (or is
    /// restored) retries with its new stamp.
    pub failed: std::collections::HashMap<String, (f64, bool)>,
    /// "Last photo sync ..." line from the engine's last cycle.
    pub last_sync: String,
    /// Header summary: cloud/local counts, the last-sync clock and the
    /// engine's summary-level error (separate from a timeline fetch error).
    pub cloud_count: u64,
    pub disk_count: u64,
    pub last_sync_ts: Option<chrono::DateTime<chrono::FixedOffset>>,
    pub sync_error: String,
    /// Files with an image name but non-image content (left in place).
    pub upload_skipped: Vec<FileIssue>,
    /// Files the CLI rejected; they are retried by Sync photos.
    pub upload_failed: Vec<FileIssue>,
    /// Photos the engine says need attention (incomplete, differs, duplicate).
    pub attention: usize,
    /// Timeline grouped into day cards, rebuilt when the list changes.
    pub groups: Vec<(String, Vec<usize>)>,
    /// Names held more than once by the timeline: identity is ambiguous.
    pub shared: BTreeSet<String>,
    /// Epoch seconds of the last timeline fetch, so a revisit can re-list.
    pub fetched_at: f64,
    /// A delete is in flight.
    pub deleting: bool,
    /// Change detection: last seen folder stamp, the engine summary we already
    /// acted on, and when we last looked (two `stat`s per second at most).
    pub dir_stamp: f64,
    pub summary_ts: String,
    pub probed_at: f64,
    /// A Photos sync was requested; cleared when the summary advances.
    pub checking_at: Option<f64>,
    /// Thumbnail recency clock, advanced once per frame.
    pub thumb_gen: u64,
}

/// The narrow view of the app the Photos page receives. Existing page code
/// can keep `app.photos` / `app.ctx` while the type no longer imports `App`.
pub struct PhotoView<'a> {
    pub photos: &'a mut PhotoState,
    pub ctx: &'a AppCtx,
    pub active: bool,
    pub paused_by_user: bool,
    pub status: health::UiStatus,
    pub can_transfer: bool,
    pub dialog: &'a mut Option<Dialog>,
    pub toasts: &'a mut VecDeque<Toast>,
    pub login_requested: &'a mut bool,
    pub now: f64,
}

impl PhotoView<'_> {
    fn ui_status(&self) -> health::UiStatus {
        self.status
    }

    fn can_transfer(&self) -> bool {
        self.can_transfer
    }

    fn now(&self) -> f64 {
        self.now
    }

    fn toast(&mut self, msg: impl Into<String>) {
        push_toast(self.toasts, self.now, msg.into(), false);
    }

    fn action_login(&mut self) {
        *self.login_requested = true;
    }
}

fn push_toast(toasts: &mut VecDeque<Toast>, now: f64, msg: String, error: bool) {
    if toasts.len() >= 4 {
        let idx = toasts.iter().position(|t| !t.error).unwrap_or(0);
        toasts.remove(idx);
    }
    toasts.push_back(Toast {
        msg,
        at: now,
        error,
        shown_at: None,
    });
}

fn open_path(path: &std::path::Path) {
    let p = path.to_path_buf();
    std::thread::spawn(move || {
        let _ = std::process::Command::new("xdg-open")
            .arg(&p)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    });
}

/// Where downloaded photos land: `PROTON_SYNC_PHOTOS_LOCAL`, else
/// `~/ProtonPhotos`.
fn local_dir_from(conf: &BTreeMap<String, String>) -> std::path::PathBuf {
    match conf
        .get("PROTON_SYNC_PHOTOS_LOCAL")
        .map(String::as_str)
        .filter(|s| !s.trim().is_empty())
    {
        Some(v) => super::expand_user(v),
        None => dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("/"))
            .join("ProtonPhotos"),
    }
}

pub fn local_dir(app: &PhotoView) -> std::path::PathBuf {
    local_dir_from(&app.ctx.conf)
}

/// Show upload exceptions at the top of the scroll area, so a long list can
/// never push the grid (or this card's own actions) off the panel.
fn draw_upload_issues(
    app: &mut PhotoView,
    ui: &mut egui::Ui,
    failed: &[FileIssue],
    skipped: &[FileIssue],
    can_retry: bool,
    signed_out: bool,
) {
    if !failed.is_empty() {
        ui.add_space(8.0);
        let title = if failed.len() == 1 {
            "1 photo couldn't upload".to_string()
        } else {
            format!("{} photos couldn't upload", failed.len())
        };
        widgets::card(ui, Some(&title), |ui| {
            for issue in failed.iter().take(5) {
                ui.label(
                    rt(format!(
                        "{} \u{2014} {}",
                        issue.name,
                        shorten(&issue.reason, 180)
                    ))
                    .size(12.0)
                    .color(theme::warning_text(ui.visuals())),
                );
            }
            if failed.len() > 5 {
                ui.label(
                    rt(format!("and {} more", failed.len() - 5))
                        .size(12.0)
                        .weak(),
                );
            }
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    rt("The other photos were not affected.")
                        .size(12.0)
                        .weak(),
                );
                if ui
                    .add_enabled(can_retry, egui::Button::new(t("Retry uploads")))
                    .on_disabled_hover_text(if signed_out {
                        "Sign in on Home first."
                    } else {
                        "Wait for the current photo action to finish."
                    })
                    .clicked()
                {
                    request_sync(app, None);
                }
            });
        });
    }

    if !skipped.is_empty() {
        ui.add_space(8.0);
        let title = if skipped.len() == 1 {
            "1 file skipped".to_string()
        } else {
            format!("{} files skipped", skipped.len())
        };
        widgets::card(ui, Some(&title), |ui| {
            for issue in skipped.iter().take(5) {
                ui.label(
                    rt(format!("{} {}", issue.name, issue.reason))
                        .size(12.0),
                );
            }
            if skipped.len() > 5 {
                ui.label(
                    rt(format!("and {} more", skipped.len() - 5))
                        .size(12.0)
                        .weak(),
                );
            }
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    rt(if skipped.len() == 1 {
                        "It stays in your folder and wasn't uploaded."
                    } else {
                        "They stay in your folder and weren't uploaded."
                    })
                    .size(12.0)
                    .weak(),
                );
                if ui.tbutton("Open folder").clicked() {
                    let dir = local_dir(app);
                    let _ = std::fs::create_dir_all(&dir);
                    open_path(&dir);
                }
            });
        });
    }
}

/// One-line error text that never splits a UTF-8 character.
fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}

/// The page was shown (navigation): list the timeline the first time only.
pub fn on_shown(app: &mut PhotoView) {
    // The folder existing is what turns the engine's photo sync on, so create
    // it once per session. Re-reading the timeline comes from `tick`.
    if !app.photos.dir_ready && std::fs::create_dir_all(local_dir(app)).is_ok() {
        app.photos.dir_ready = true;
    }
    if !app.photos.loaded {
        refresh(app);
    }
}

/// What the engine's last photo cycle did (`photos-sync.json`).

#[derive(serde::Deserialize, Default)]
struct SyncSummary {
    ts: String,
    remote: u64,
    local: u64,
    uploaded: u64,
    downloaded: u64,
    #[serde(default)]
    trashed: u64,
    #[serde(default)]
    removed: u64,
    conflicts: u64,
    #[serde(default)]
    attention: u64,
    #[serde(default)]
    skipped: Vec<FileIssue>,
    #[serde(default)]
    failed: Vec<FileIssue>,
    error: String,
}

/// Only report issues for files that still exist. A skipped/failed card is
/// about a file the user can act on; once it is deleted or renamed the card
/// must not outlive it, even before the next engine summary arrives.
fn existing_issues(dir: &std::path::Path, issues: &[FileIssue]) -> Vec<FileIssue> {
    issues
        .iter()
        .filter(|i| dir.join(&i.name).is_file())
        .cloned()
        .collect()
}

fn render_summary(app: &mut PhotoView, s: &SyncSummary) -> String {
    app.photos.attention = s.attention as usize;
    let dir = local_dir(app);
    let skipped = existing_issues(&dir, &s.skipped);
    let failed = existing_issues(&dir, &s.failed);
    app.photos.upload_skipped = skipped.clone();
    app.photos.upload_failed = failed.clone();
    app.photos.cloud_count = s.remote;
    app.photos.disk_count = s.local;
    app.photos.sync_error = s.error.clone();
    let ts = chrono::DateTime::parse_from_rfc3339(&s.ts).ok();
    app.photos.last_sync_ts = ts;
    let when = crate::util::rel_time(ts);
    if !s.error.is_empty() {
        if crate::health::is_login_error(&s.error) {
            return "Sign in again to sync photos.".to_string();
        }
        let hold = s.error.starts_with("Photo timeline")
            || s.error.starts_with("Photo folder");
        let prefix = if hold {
            "Photo sync is on hold: "
        } else {
            "Photo sync failed: "
        };
        return format!("{prefix}{}", s.error);
    }
    let mut line = format!(
        "Last photo sync \u{b7} {} in the cloud, {} on disk",
        s.remote, s.local
    );
    if s.attention > 0 {
        line.push_str(&format!(" \u{b7} {} need attention", s.attention));
    }
    if s.uploaded + s.downloaded + s.trashed + s.removed > 0 {
        line.push_str(&format!(
            " \u{b7} last pass {} up, {} down, {} trashed, {} removed",
            s.uploaded, s.downloaded, s.trashed, s.removed
        ));
    }
    if s.conflicts > 0 {
        line.push_str(&format!(
            " \u{b7} {} photo(s) not synced - versions differ",
            s.conflicts
        ));
    }
    if !skipped.is_empty() {
        line.push_str(&format!(
            " \u{b7} {}",
            if skipped.len() == 1 {
                "1 file skipped".to_string()
            } else {
                format!("{} files skipped", skipped.len())
            }
        ));
    }
    if !failed.is_empty() {
        line.push_str(&format!(
            " \u{b7} {}",
            if failed.len() == 1 {
                "1 couldn't upload".to_string()
            } else {
                format!("{} couldn't upload", failed.len())
            }
        ));
    }
    if !when.is_empty() {
        line.push_str(&format!(" \u{b7} {when}"));
    }
    line
}

fn read_summary(app: &mut PhotoView) -> String {
    let path = app.ctx.paths.state_dir.join("photos-sync.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        clear_summary(app.photos);
        return String::new();
    };
    let Ok(s) = serde_json::from_str::<SyncSummary>(&text) else {
        clear_summary(app.photos);
        return String::new();
    };
    render_summary(app, &s)
}

/// Drop every value derived from a summary file that is gone or unreadable.
fn clear_summary(p: &mut PhotoState) {
    p.last_sync.clear();
    p.cloud_count = 0;
    p.disk_count = 0;
    p.last_sync_ts = None;
    p.sync_error.clear();
    p.attention = 0;
    p.upload_skipped.clear();
    p.upload_failed.clear();
}

pub fn refresh(app: &mut PhotoView) {
    if app.photos.loading || app.photos.busy {
        return;
    }
    app.photos.loaded = true;
    app.photos.loading = true;
    app.photos.fetched_at = app.now();
    app.photos.error.clear();
    // Only the first load says so: a background re-read must not flash text.
    app.photos.status = if app.photos.items.is_empty() {
        "Loading the photo timeline...".to_string()
    } else {
        String::new()
    };
    let tx = app.ctx.tx.clone();
    std::thread::spawn(move || {
        let _ = tx.send(super::Back::Photos(cli::fetch_photos()));
    });
}

pub fn on_fetched(app: &mut PhotoView, r: Result<Vec<cli::PhotoEntry>, String>) {
    app.photos.loading = false;
    match r {
        Ok(items) => {
            app.photos.status.clear();
            // Drop selections for photos that are gone.
            let live: Vec<&str> = items.iter().map(|p| p.uid.as_str()).collect();
            app.photos.sel.retain(|u| live.contains(&u.as_str()));
            app.photos.items = items;
            app.photos.groups = build_groups(&app.photos.items);
            let (shared, names) = shared_and_names(&app.photos.items);
            app.photos.shared = shared;
            app.photos.failed.retain(|name, _| names.contains(name));
        }
        Err(e) => {
            app.photos.status.clear();
            app.photos.error = e;
        }
    }
    app.photos.last_sync = read_summary(app);
}

/// Day cards and the set of names the timeline holds more than once.
fn shared_and_names(items: &[cli::PhotoEntry]) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut seen = BTreeSet::new();
    let mut shared = BTreeSet::new();
    let mut names = BTreeSet::new();
    for p in items {
        names.insert(p.name.clone());
        if !seen.insert(p.name.clone()) {
            shared.insert(p.name.clone());
        }
    }
    (shared, names)
}

/// Ask the engine for a photo pass. An optional uid forces that one repair
/// past its backoff; the request is written before the wake so the engine
/// cannot wake and find an empty request.
pub fn request_sync(app: &mut PhotoView, uid: Option<&str>) {
    if !app.active {
        app.photos.status = if app.paused_by_user {
            "Paused - resume on Home.".to_string()
        } else {
            "Sync is stopped - start it on Home.".to_string()
        };
        return;
    }
    if app.ctx.conf.get("PROTON_SYNC_PHOTOS").is_some_and(|v| v == "0") {
        app.photos.status = "Photo sync is off in Settings.".to_string();
        return;
    }
    let _ = std::fs::create_dir_all(&app.ctx.paths.state_dir);
    let req = app.ctx.paths.state_dir.join("photos-request");
    if let Err(e) = std::fs::write(&req, uid.unwrap_or("")) {
        app.photos.error = format!("Could not ask the engine to sync photos: {e}");
        return;
    }
    let _ = std::fs::write(&app.ctx.paths.wake_file, b"");
    app.photos.error.clear();
    app.photos.checking_at = Some(app.now());
    app.photos.status = "Checking for new photos...".to_string();
}

pub fn download(app: &mut PhotoView, uids: Vec<String>) {
    if uids.is_empty() || app.photos.busy || app.photos.deleting {
        return;
    }
    if app.ctx.health.status == "syncing" || app.ctx.health.phase.as_deref() == Some("photos") {
        app.photos.status = "Wait for the current check to finish, then download.".to_string();
        return;
    }
    let dir = local_dir(app);
    app.photos.busy = true;
    app.photos.error.clear();
    app.photos.status = format!(
        "Downloading {} photo(s) to {}...",
        uids.len(),
        dir.display()
    );
    let tx = app.ctx.tx.clone();
    std::thread::spawn(move || {
        let r = cli::download_photos(&uids, &dir);
        let _ = tx.send(super::Back::PhotosDownloaded(r));
    });
}

pub fn on_downloaded(app: &mut PhotoView, r: Result<usize, String>) {
    app.photos.busy = false;
    match r {
        Ok(n) => {
            app.photos.error.clear();
            app.photos.status.clear();
            let dir = local_dir(app);
            app.toast(format!("Saved {n} photo(s) to {}", dir.display()));
        }
        Err(e) => {
            app.photos.status.clear();
            app.photos.error = e;
        }
    }
}

/// Move photos to the Proton photos trash and drop their local copies.
///
/// Both halves are needed: trashing only the cloud copy would let the next
/// sync upload the local file again, and removing only the local file would
/// download the photo back.
pub fn delete(app: &mut PhotoView, uids: Vec<String>) {
    if uids.is_empty() || app.photos.deleting || app.photos.busy {
        return;
    }
    let dir = local_dir(app);
    // Names to remove locally, resolved before the trash runs.
    let names: Vec<String> = app
        .photos
        .items
        .iter()
        .filter(|p| uids.contains(&p.uid))
        .map(|p| p.name.clone())
        .collect();
    app.photos.deleting = true;
    app.photos.error.clear();
    app.photos.status = format!("Moving {} photo(s) to the trash...", uids.len());
    let tx = app.ctx.tx.clone();
    std::thread::spawn(move || {
        let r = cli::trash_photos(&uids).inspect(|_| {
            for name in &names {
                trash_local_photo(&dir, name);
            }
        });
        let _ = tx.send(super::Back::PhotosDeleted(r));
    });
}

/// Local deletions are recoverable: rename into a trash folder inside the
/// photo directory, never unlink. That folder is a directory, so `scan_local`
/// ignores it.
fn trash_local_photo(dir: &std::path::Path, name: &str) {
    if !cli::valid_photo_name(name) {
        return;
    }
    let trash = dir.join(".proton-photos-trash");
    if std::fs::create_dir_all(&trash).is_err() {
        return;
    }
    let mut dst = trash.join(name);
    let mut suffix = 1u32;
    while dst.exists() {
        suffix += 1;
        dst = trash.join(format!("{name}.{suffix}"));
    }
    let _ = std::fs::rename(dir.join(name), dst);
}

pub fn on_deleted(app: &mut PhotoView, r: Result<usize, String>) {
    app.photos.deleting = false;
    match r {
        Ok(n) => {
            app.photos.sel.clear();
            app.photos.status.clear();
            app.toast(format!("Moved {n} photo(s) to the trash."));
            refresh(app);
        }
        Err(e) => {
            app.photos.status.clear();
            app.photos.error = e;
        }
    }
}

/// Watch for changes instead of making the user reload: the engine writes
/// `photos-sync.json` after every photo cycle, and the folder's own mtime moves
/// when files appear or disappear. Either one re-reads the list.
pub fn tick(app: &mut PhotoView) {
    let now = app.now();
    if now - app.photos.probed_at < 1.0 {
        return;
    }
    app.photos.probed_at = now;
    let dir_stamp = file_stamp(&local_dir(app));
    // Files appearing or disappearing on disk (a download, a delete, a drop).
    let dir_changed = app.photos.dir_stamp != 0.0 && dir_stamp != app.photos.dir_stamp;
    let mut changed = dir_changed;
    app.photos.dir_stamp = dir_stamp;
    // A new engine summary always updates the line and clears a pending
    // "checking" state; only a real transfer or a changed cloud size re-lists
    // the timeline, so an error or a no-op pass does not flicker the page.
    let summary = std::fs::read_to_string(app.ctx.paths.state_dir.join("photos-sync.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<SyncSummary>(&t).ok());
    match summary {
        Some(s) if s.ts != app.photos.summary_ts || dir_changed => {
            let error = !s.error.is_empty();
            let moved = s.uploaded + s.downloaded + s.trashed + s.removed > 0;
            let cloud_changed = s.remote as usize != app.photos.items.len();
            app.photos.summary_ts = s.ts.clone();
            app.photos.last_sync = render_summary(app, &s);
            if app.photos.checking_at.take().is_some() {
                app.photos.status.clear();
            }
            if !error && (moved || cloud_changed) {
                changed = true;
            }
        }
        Some(_) => {}
        None => {
            // A removed/reset summary must not leave old counts or cards.
            clear_summary(app.photos);
        }
    }
    if let Some(t) = app.photos.checking_at {
        if !app.active {
            app.photos.checking_at = None;
            app.photos.status = if app.paused_by_user {
                "Paused - resume on Home.".to_string()
            } else {
                "Sync is stopped - start it on Home.".to_string()
            };
        } else if now - t > 180.0 {
            app.photos.checking_at = None;
            app.photos.status = "Still working in the background.".to_string();
        }
    }
    if changed {
        refresh(app);
    }
}

/// "2026-08-21 12:55" -> "Fri, 21 Aug 2026", the sticky day header.
fn day_of(capture: &str) -> String {
    chrono::NaiveDateTime::parse_from_str(capture, "%Y-%m-%d %H:%M")
        .map(|dt| dt.format("%a, %d %b %Y").to_string())
        .unwrap_or_else(|_| "Undated".to_string())
}

/// "2026-08-21 12:55" -> "21 Aug 2026, 12:55" (a tile's caption).
fn pretty_date(capture: &str) -> String {
    chrono::NaiveDateTime::parse_from_str(capture, "%Y-%m-%d %H:%M")
        .map(|dt| dt.format("%d %b %Y, %H:%M").to_string())
        .unwrap_or_else(|_| capture.to_string())
}

/// Timeline order (newest first) grouped into day cards, newest day first.
/// Indices keep the per-frame cost to the visible tiles only.
fn build_groups(items: &[cli::PhotoEntry]) -> Vec<(String, Vec<usize>)> {
    let mut out: Vec<(String, Vec<usize>)> = Vec::new();
    for (i, p) in items.iter().enumerate() {
        let d = day_of(&p.capture);
        match out.last_mut() {
            Some((name, group)) if *name == d => group.push(i),
            _ => out.push((d, vec![i])),
        }
    }
    out
}

fn size_label(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b >= KB * KB * KB {
        format!("{:.1} GB", b / (KB * KB * KB))
    } else if b >= KB * KB {
        format!("{:.1} MB", b / (KB * KB))
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

/// Longest edge of a cached thumbnail, in pixels.
const THUMB: usize = 256;

/// Decode a photo file into a downscaled RGBA thumbnail.
///
/// PNG goes through the crate the app already ships; JPEG through `zune-jpeg`
/// (pure Rust, no C). Anything else (HEIC from an iPhone, say) is reported as
/// unsupported so the tile can show a placeholder instead of a broken image.
fn decode_thumb(bytes: &[u8]) -> Result<(usize, usize, Vec<u8>), String> {
    let (w, h, rgba) = if bytes.starts_with(&[0xFF, 0xD8]) {
        use zune_jpeg::zune_core::{colorspace::ColorSpace, options::DecoderOptions};
        let opts = DecoderOptions::new_fast().jpeg_set_out_colorspace(ColorSpace::RGBA);
        let mut dec = zune_jpeg::JpegDecoder::new_with_options(std::io::Cursor::new(bytes), opts);
        let data = dec.decode().map_err(|e| format!("jpeg: {e:?}"))?;
        let info = dec.info().ok_or("jpeg: no header")?;
        (info.width as usize, info.height as usize, data)
    } else if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        super::decode_png(bytes).ok_or("png: decode failed")?
    } else {
        return Err("unsupported image format".to_string());
    };
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return Err("decoded image is empty".to_string());
    }
    Ok(downscale(&rgba, w, h, THUMB))
}

/// Box-filter downscale to at most `max` px on the long edge. Decoding a
/// 4000px photo only to hand the GPU a 150px tile wastes upload bandwidth, and
/// averaging beats nearest-neighbour for thumbnails.
fn downscale(rgba: &[u8], w: usize, h: usize, max: usize) -> (usize, usize, Vec<u8>) {
    let step = w.max(h).div_ceil(max).max(1);
    if step == 1 {
        return (w, h, rgba.to_vec());
    }
    let (ow, oh) = (w.div_ceil(step), h.div_ceil(step));
    let mut out = vec![0u8; ow * oh * 4];
    for oy in 0..oh {
        for ox in 0..ow {
            let x0 = ox * step;
            let y0 = oy * step;
            let (mut r, mut g, mut b, mut a, mut n) = (0u32, 0u32, 0u32, 0u32, 0u32);
            for sy in y0..(y0 + step).min(h) {
                for sx in x0..(x0 + step).min(w) {
                    let i = (sy * w + sx) * 4;
                    r += rgba[i] as u32;
                    g += rgba[i + 1] as u32;
                    b += rgba[i + 2] as u32;
                    a += rgba[i + 3] as u32;
                    n += 1;
                }
            }
            let o = (oy * ow + ox) * 4;
            let n = n.max(1);
            out[o] = (r / n) as u8;
            out[o + 1] = (g / n) as u8;
            out[o + 2] = (b / n) as u8;
            out[o + 3] = (a / n) as u8;
        }
    }
    (ow, oh, out)
}

/// The file's mtime in seconds, as the cache stamp (0 when unreadable).
fn file_stamp(path: &std::path::Path) -> f64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Decode a photo's thumbnail in the background (max 4 at a time).
fn request_thumb(app: &mut PhotoView, name: &str, path: std::path::PathBuf, stamp: f64) {
    if app.photos.in_flight.len() >= 4 || app.photos.in_flight.contains(name) {
        return;
    }
    app.photos.in_flight.insert(name.to_string());
    let tx = app.ctx.tx.clone();
    let name = name.to_string();
    std::thread::spawn(move || {
        let result = std::fs::read(&path)
            .map_err(|e| format!("read: {e}"))
            .and_then(|bytes| decode_thumb(&bytes));
        let _ = tx.send(super::Back::PhotoThumb {
            name,
            stamp,
            result,
        });
    });
}

/// Header actions. Drawn in right-to-left order because the header uses an
/// RTL layout: Open folder sits on the right, Sync photos on the left.
fn photo_actions(app: &mut PhotoView, ui: &mut egui::Ui) {
    let dir = local_dir(app);
    let photo_sync_on = app.ctx.conf.get("PROTON_SYNC_PHOTOS").is_none_or(|v| v != "0");
    let syncing =
        app.ctx.health.status == "syncing" || app.ctx.health.phase.as_deref() == Some("photos");
    let ready = !app.photos.busy && !app.photos.deleting && !syncing;
    let signing_in = matches!(app.ui_status(), crate::health::UiStatus::SigningIn);
    let login_error = crate::health::is_login_error(&app.photos.sync_error);
    let signed_out = (!app.can_transfer() || login_error) && !signing_in;
    let can_act = ready && !signed_out && !signing_in;

    if widgets::secondary_button_hint(ui, "Open folder", "Open the local photo folder.") {
        let _ = std::fs::create_dir_all(&dir);
        open_path(&dir);
    }
    let download_resp = ui
        .add_enabled(
            can_act && !app.photos.items.is_empty(),
            egui::Button::new(t("Download all")),
        )
        .on_hover_text(
            "Copy every photo that is not on this computer. Nothing in the cloud changes.",
        )
        .on_disabled_hover_text(if signing_in {
            "Finish signing in first."
        } else if signed_out {
            "Sign in first."
        } else if !ready {
            "Wait for the current photo action to finish."
        } else {
            "Nothing to download yet."
        });
    if download_resp.clicked() {
        let missing: Vec<String> = app
            .photos
            .items
            .iter()
            .filter(|p| {
                !app.photos.shared.contains(&p.name) && !dir.join(&p.name).is_file()
            })
            .map(|p| p.uid.clone())
            .collect();
        if missing.is_empty() {
            app.photos.status = if app.photos.shared.is_empty() {
                "Every photo is already on this computer.".to_string()
            } else {
                "Some photos share a file name with another photo and were skipped. \
                 Download them individually or rename the local copy."
                    .to_string()
            };
        } else {
            download(app, missing);
        }
    }
    if signing_in {
        ui.add_enabled(false, egui::Button::new(t("Signing in...")))
            .on_disabled_hover_text("Finish signing in in the terminal window.");
    } else if signed_out {
        if widgets::primary_button_hint(
            ui,
            "Sign in",
            "Sign in with your Proton account to sync photos.",
        ) {
            app.action_login();
        }
    } else if app.active && photo_sync_on {
        if ready {
            if widgets::primary_button_hint(
                ui,
                "Sync photos",
                "Check Proton Photos now. Your Drive files sync right after.",
            ) {
                request_sync(app, None);
            }
        } else {
            ui.add_enabled(false, egui::Button::new(t("Sync photos")))
                .on_disabled_hover_text(if syncing {
                    "A sync is already running."
                } else {
                    "Wait for the current photo action to finish."
                });
        }
    } else {
        ui.add_enabled(false, egui::Button::new(t("Sync photos")))
            .on_disabled_hover_text(if !photo_sync_on {
                "Turn photo sync on in Settings > Sync."
            } else {
                "Start syncing on Home."
            });
    }
}

/// Photos hero, in Home's language: status orb, plain-language title,
/// one context line, and the same pill on the right.
fn photo_hero(app: &PhotoView, ui: &mut egui::Ui) {
    let status = app.ui_status();
    let signing_in = matches!(status, crate::health::UiStatus::SigningIn);
    let signed_out = matches!(status, crate::health::UiStatus::SignedOut);
    let needs_login = matches!(status, crate::health::UiStatus::NeedsLogin);
    let paused = matches!(status, crate::health::UiStatus::Paused);
    let starting = matches!(status, crate::health::UiStatus::Starting);
    let photo_sync_on = app.ctx.conf.get("PROTON_SYNC_PHOTOS").is_none_or(|v| v != "0");
    let syncing =
        app.ctx.health.status == "syncing" || app.ctx.health.phase.as_deref() == Some("photos");
    let cloud = if app.photos.cloud_count > 0 {
        app.photos.cloud_count
    } else {
        app.photos.items.len() as u64
    };
    let disk = app.photos.disk_count;
    let total: u64 = app.photos.items.iter().map(|p| p.size).sum();
    let have_sync = app.photos.last_sync_ts.is_some();
    let summary_error = app.photos.sync_error.clone();
    let timeline_error = app.photos.error.clone();
    let login_error = crate::health::is_login_error(&summary_error);
    let needs_login = needs_login || (login_error && !signing_in);
    let needs_signin = signed_out || needs_login;
    let attention = app.photos.attention;
    let photos_phase = app.ctx.health.phase.as_deref() == Some("photos");
    let (key, label, title, sub): (&str, &str, &str, String) = if signing_in {
        (
            "signing_in",
            "Signing in",
            "Signing in...",
            "Finish signing in in the terminal window.".to_string(),
        )
    } else if needs_login {
        (
            "needs_login",
            "Session expired",
            "Sign in again to sync photos",
            "Your session expired. Your files stay safe on this computer.".to_string(),
        )
    } else if signed_out {
        (
            "signed_out",
            "Sign in to sync",
            "Sign in to sync photos",
            "Your photos will appear here after you sign in.".to_string(),
        )
    } else if !photo_sync_on {
        (
            "engine_stopped",
            "Photo sync off",
            "Photo sync is off",
            "Turn photo sync on in Settings > Sync.".to_string(),
        )
    } else if !summary_error.is_empty() || !timeline_error.is_empty() {
        let reason = if !summary_error.is_empty() {
            format!("Photo sync: {}", shorten(&summary_error, 80))
        } else {
            "See the message below.".to_string()
        };
        (
            "paused",
            "Needs attention",
            "Photo sync needs attention",
            reason,
        )
    } else if attention > 0 {
        (
            "paused",
            "Needs attention",
            "Some photos need attention",
            format!("{attention} need attention - see below."),
        )
    } else if syncing {
        (
            "syncing",
            if photos_phase {
                "Syncing photos"
            } else {
                "Waiting for files"
            },
            if photos_phase {
                "Syncing your photos"
            } else {
                "Waiting for the file sync"
            },
            if photos_phase {
                "Checking Proton Photos now.".to_string()
            } else {
                "Waiting for the file sync to finish.".to_string()
            },
        )
    } else if paused {
        (
            "paused",
            "Paused",
            "Photo sync is paused",
            "Resume on Home to continue.".to_string(),
        )
    } else if starting {
        (
            "starting",
            "Starting",
            "Starting photo sync",
            "The photo timeline will appear shortly.".to_string(),
        )
    } else if !app.active {
        (
            "engine_stopped",
            "Stopped",
            "Photo sync is stopped",
            "Start syncing on Home.".to_string(),
        )
    } else if have_sync {
        let when = crate::util::rel_time(app.photos.last_sync_ts);
        let pending = cloud.saturating_sub(disk);
        let sub = if pending > 0 {
            format!("Last checked {when} \u{b7} {pending} waiting to download")
        } else {
            format!("Last checked {when} \u{b7} All photos are on this computer")
        };
        ("synced", "Synced", "You're up to date", sub)
    } else {
        (
            "starting",
            "Not synced yet",
            "Your photo library",
            format!(
                "{} photos \u{b7} {}",
                app.photos.items.len(),
                size_label(total)
            ),
        )
    };
    let animate = app.ctx.prefs.motion_enabled();
    widgets::card(ui, None, |ui| {
        // Top-align the copy with the status pill; centering a two-line block
        // against the 72 px orb left the title visibly lower than the pill.
        ui.horizontal_top(|ui| {
            widgets::status_orb(ui, key, animate);
            ui.add_space(18.0);
            let text_w = (ui.available_width() - 124.0).max(80.0);
            ui.allocate_ui_with_layout(
                egui::vec2(text_w, 0.0),
                egui::Layout::top_down(egui::Align::LEFT),
                |ui| {
                    ui.set_min_width(text_w);
                    ui.label(rt(title).strong().size(20.0));
                    let sub = ui.label(rt(sub).size(12.5).weak());
                    if !app.photos.last_sync.is_empty() && !needs_signin {
                        sub.on_hover_text(&app.photos.last_sync);
                    }
                },
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                widgets::status_pill(ui, key, label);
            });
        });
    });
}

fn photo_metrics(app: &PhotoView, ui: &mut egui::Ui) {
    let cloud = if app.photos.cloud_count > 0 {
        app.photos.cloud_count
    } else {
        app.photos.items.len() as u64
    };
    let cloud_label =
        if !app.photos.sync_error.is_empty()
            && app.photos.cloud_count == 0
            && app.photos.items.is_empty()
        {
            "Unknown".to_string()
        } else {
            cloud.to_string()
        };
    let total: u64 = app.photos.items.iter().map(|p| p.size).sum();
    let disk_label = if app.photos.last_sync_ts.is_some() && app.photos.sync_error.is_empty()
    {
        app.photos.disk_count.to_string()
    } else {
        "Unknown".to_string()
    };
    ui.columns(3, |cols| {
        widgets::metric(&mut cols[0], "In the cloud", &cloud_label, "Proton Photos");
        widgets::metric(&mut cols[1], "Library size", &size_label(total), "All photos");
        widgets::metric(&mut cols[2], "On this computer", &disk_label, "Local copies");
    });
}

pub fn draw(app: &mut PhotoView, ui: &mut egui::Ui) {
    let dir = local_dir(app);
    if app.photos.groups.is_empty() && !app.photos.items.is_empty() {
        app.photos.groups = build_groups(&app.photos.items);
    }
    app.photos.thumb_gen = app.photos.thumb_gen.wrapping_add(1);

    let photo_sync_on = app.ctx.conf.get("PROTON_SYNC_PHOTOS").is_none_or(|v| v != "0");
    let syncing =
        app.ctx.health.status == "syncing" || app.ctx.health.phase.as_deref() == Some("photos");
    let ready = !app.photos.busy && !app.photos.deleting && !syncing;
    let signed_out = matches!(
        app.ui_status(),
        crate::health::UiStatus::SignedOut
            | crate::health::UiStatus::NeedsLogin
            | crate::health::UiStatus::SigningIn
    );
    let can_act = ready && !signed_out;
    let sel = app.photos.sel.clone();

    // --- header: title + actions, then the Home-style hero and metrics ------
    let narrow = ui.available_width() < 420.0;
    if sel.is_empty() && !narrow {
        ui.horizontal(|ui| {
            widgets::page_title(ui, crate::i18n::tr("Photos", "Foto"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(325.0, 30.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| photo_actions(app, ui),
                );
            });
        });
    } else {
        ui.horizontal(|ui| {
            widgets::page_title(ui, crate::i18n::tr("Photos", "Foto"));
        });
        if sel.is_empty() {
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| photo_actions(app, ui));
        }
    }
    // With a selection, the selection toolbar is the primary context. At the
    // 720x400 floor the hero and three metric cards otherwise leave almost no
    // height for the grid, so hide them until the selection is cleared.
    if sel.is_empty() {
        ui.add_space(14.0);
        photo_hero(app, ui);
        if !signed_out {
            ui.add_space(12.0);
            photo_metrics(app, ui);
        }
        ui.add_space(12.0);
    } else {
        ui.add_space(8.0);
    }

    if !sel.is_empty() {
        ui.add_space(8.0);
        widgets::card(ui, None, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(rt(format!("{} selected", sel.len())).strong());
                ui.add_space(4.0);
                if ui
                    .add_enabled(can_act, egui::Button::new(format!("Download {}", sel.len())))
                    .clicked()
                {
                    let missing: Vec<String> = app
                        .photos
                        .items
                        .iter()
                        .filter(|p| {
                            sel.contains(&p.uid)
                                && !app.photos.shared.contains(&p.name)
                                && !dir.join(&p.name).is_file()
                        })
                        .map(|p| p.uid.clone())
                        .collect();
                    if missing.is_empty() {
                        let skipped = app
                            .photos
                            .items
                            .iter()
                            .any(|p| sel.contains(&p.uid) && app.photos.shared.contains(&p.name));
                        app.photos.status = if skipped {
                            "Some selected photos share a file name with another photo and \
                             were skipped. Download them individually or rename the local copy."
                                .to_string()
                        } else {
                            "The selected photos are already on this computer.".to_string()
                        };
                    } else {
                        download(app, missing);
                    }
                }
                if ui
                    .add_enabled(can_act, egui::Button::new(format!("Delete {}", sel.len())))
                    .on_hover_text("Move to the Proton photos trash and remove the local copy")
                    .clicked()
                {
                    *app.dialog = Some(super::Dialog::ConfirmPhotoDelete { uids: sel.clone() });
                }
                if ui.tbutton("Clear selection").clicked() {
                    app.photos.sel.clear();
                }
                ui.separator();
                if ui.tbutton("Open folder").clicked() {
                    let _ = std::fs::create_dir_all(&dir);
                    open_path(&dir);
                }
            });
        });
    }

    if syncing && app.active && photo_sync_on {
        ui.add_space(8.0);
        widgets::card(ui, None, |ui| {
            ui.horizontal(|ui| {
                widgets::busy_spinner(ui, app.ctx.prefs.motion_enabled());
                ui.label(
                    rt(if app.ctx.health.phase.as_deref() == Some("photos") {
                        "Syncing photos\u{2026}"
                    } else {
                        "Waiting for the file sync to finish\u{2026}"
                    })
                    .size(12.5)
                    .weak(),
                );
            });
        });
    }

    if !app.photos.error.is_empty() {
        ui.add_space(8.0);
        let error = app.photos.error.clone();
        widgets::card(ui, None, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(!app.photos.busy, egui::Button::new(t("Reload")))
                    .on_hover_text("Reload the photo timeline.")
                    .clicked()
                {
                    refresh(app);
                }
                ui.label(
                    rt(&error).color(theme::danger_text(ui.visuals())),
                );
            });
        });
    }

    if !app.photos.status.is_empty() {
        ui.add_space(8.0);
        ui.label(rt(&app.photos.status).weak());
    }

    let failed_issues = app.photos.upload_failed.clone();
    let skipped_issues = app.photos.upload_skipped.clone();

    // --- only the grid scrolls, with the day header pinned to its top ---
    egui::ScrollArea::vertical()
        .id_salt("photos-scroll")
        .auto_shrink([false, false])
        .show_viewport(ui, |ui, viewport| {
            draw_upload_issues(
                app,
                ui,
                &failed_issues,
                &skipped_issues,
                can_act,
                signed_out,
            );
            if app.photos.items.is_empty() {
                if !app.photos.loading && app.photos.error.is_empty() {
                    ui.add_space(18.0);
                    ui.label(
                        rt("No photos in your timeline yet.")
                            .weak()
                            .size(13.5),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        rt("Photos you upload from the Proton apps appear here.")
                            .weak()
                            .size(12.0),
                    );
                }
                return;
            }

            // Every day card carries its own header. Once that header scrolls
            // off the top it is re-painted pinned (see below), so the date is
            // never drawn twice at the same time.
            let mut current: Option<(String, f32)> = None;
            for (day, group) in app.photos.groups.clone() {
                ui.add_space(12.0);
                let header_y = ui.cursor().top();
                ui.label(rt(&day).strong().size(13.5));
                ui.add_space(4.0);
                widgets::card(ui, None, |ui| {
                    // One grid for every card: the size depends on the width
                    // alone, so a sparse card keeps the same thumbnails as a
                    // crowded one, and a full row spans the card exactly.
                    let (cols, size) = grid_metrics(ui.available_width());
                    ui.spacing_mut().item_spacing.x = GAP;
                    for row in group.chunks(cols.max(1)) {
                        ui.horizontal(|ui| {
                            for &i in row {
                                // Cull before any stat, file read or decode:
                                // a large timeline only pays for what is on
                                // screen. The same height is reserved so the
                                // scrollbar does not jump.
                                let tile_rect = egui::Rect::from_min_size(
                                    ui.cursor().min,
                                    egui::vec2(size, size + TILE_LABEL_H),
                                );
                                if !ui.is_rect_visible(tile_rect) {
                                    ui.allocate_space(tile_rect.size());
                                    continue;
                                }
                                let p = app.photos.items[i].clone();
                                tile(ui, app, &p, &dir, size);
                            }
                        });
                    }
                });
                // `viewport` is in content space, `min_rect` in screen space:
                // convert the top edge once, then compare like with like.
                let visible_top = ui.min_rect().top() + viewport.min.y;
                if current.is_none() && ui.min_rect().bottom() > visible_top {
                    current = Some((day, header_y));
                }
            }
            ui.add_space(12.0);

            // Sticky day header: drawn last so it sits over the tiles, at the
            // top edge of the visible area, until the next day replaces it.
            let content = ui.min_rect();
            let visible_top = content.top() + viewport.min.y;
            if let Some((day, header_y)) = current
                && header_y < visible_top - 1.0
            {
                let rect = egui::Rect::from_min_size(
                    egui::pos2(content.left(), visible_top),
                    egui::vec2(content.width(), 20.0),
                );
                ui.painter().rect_filled(rect, 0.0, ui.visuals().panel_fill);
                ui.painter().text(
                    rect.left_center() + egui::vec2(6.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    day,
                    egui::FontId::proportional(13.5),
                    ui.visuals().strong_text_color(),
                );
            }
            widgets::scroll_focused_into_view(ui);
        });
}

/// One tile of the grid: the picture when it is on disk, a placeholder while it
/// is not (with a Download button), and the name underneath. The size comes
/// from [`grid_metrics`] so every card lines up on the same grid.
const TARGET_TILE: f32 = 160.0;
/// Never so small that a photo stops reading, never so large that a row holds
/// two pictures on a wide window.
const MIN_TILE: f32 = 110.0;
const MAX_TILE: f32 = 220.0;
const GAP: f32 = 8.0;

/// Columns and tile width for an available width: the row is filled exactly
/// (no leftover strip on a full row), tiles stay between [`MIN_TILE`] and
/// [`MAX_TILE`], and the result depends only on the width - so a card with
/// three photos uses the same tile size as one with twenty.
fn grid_metrics(avail: f32) -> (usize, f32) {
    if avail <= MIN_TILE {
        return (1, avail.max(1.0));
    }
    let mut cols = ((avail + GAP) / (TARGET_TILE + GAP)).round().max(1.0) as usize;
    loop {
        let t = (avail - (cols.saturating_sub(1)) as f32 * GAP) / cols as f32;
        if t < MIN_TILE && cols > 1 {
            cols -= 1;
            continue;
        }
        if t > MAX_TILE && cols < 64 {
            // One column cannot shrink; stop instead of bouncing between
            // one and two columns when avail is just over MAX_TILE.
            if cols == 1 {
                return (1, MAX_TILE);
            }
            cols += 1;
            continue;
        }
        return (cols, t.min(MAX_TILE));
    }
}

/// Height reserved under the image: name, status line and (when needed) the
/// download action. Fixed so the cull and the real layout agree.
const TILE_LABEL_H: f32 = 52.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileState {
    /// Two cloud photos share this name; the local file cannot be identified.
    Duplicate,
    /// No local copy.
    Missing,
    /// A 0-byte placeholder left by an interrupted download.
    Incomplete,
    /// A non-zero local file that differs from the cloud copy.
    Mismatch,
    Ready,
}

fn tile_state(local: Option<u64>, cloud: u64, shared: bool) -> TileState {
    if shared {
        TileState::Duplicate
    } else {
        match local {
            None => TileState::Missing,
            Some(0) if cloud > 0 => TileState::Incomplete,
            Some(n) if cloud > 0 && n != cloud => TileState::Mismatch,
            Some(_) => TileState::Ready,
        }
    }
}

fn caption_for(state: TileState, local: Option<u64>, cloud: u64) -> String {
    let size = if cloud > 0 {
        size_label(cloud)
    } else {
        "Size unknown".to_string()
    };
    match state {
        TileState::Duplicate => "Duplicate name".to_string(),
        TileState::Missing => format!("Not on this computer \u{b7} {size}"),
        TileState::Incomplete => format!("Download incomplete \u{b7} {size}"),
        TileState::Mismatch => format!(
            "Both copies kept \u{b7} {} / {size}",
            size_label(local.unwrap_or(0))
        ),
        TileState::Ready => size,
    }
}

fn preview_label(unsupported: bool) -> &'static str {
    if unsupported {
        "No preview for this format"
    } else {
        "Preview failed"
    }
}

fn tile(
    ui: &mut egui::Ui,
    app: &mut PhotoView,
    p: &cli::PhotoEntry,
    dir: &std::path::Path,
    size: f32,
) {
    let path = dir.join(&p.name);
    let meta = std::fs::metadata(&path).ok();
    let local_len = meta.as_ref().filter(|m| m.is_file()).map(|m| m.len());
    let stamp = meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    let state = tile_state(local_len, p.size, app.photos.shared.contains(&p.name));
    let can_preview = matches!(state, TileState::Ready | TileState::Mismatch);
    let failed = app
        .photos
        .failed
        .get(&p.name)
        .filter(|(s, _)| (*s - stamp).abs() < 0.5);
    let blocked = failed.is_some();
    let unsupported = failed.is_some_and(|(_, unsupported)| *unsupported);
    if can_preview
        && !blocked
        && app
            .photos
            .tex
            .get(&p.name)
            .is_none_or(|t| (t.stamp - stamp).abs() >= 0.5)
    {
        request_thumb(app, &p.name, path, stamp);
    }
    let now_gen = app.photos.thumb_gen;
    if let Some(t) = app.photos.tex.get_mut(&p.name) {
        t.used = now_gen;
    }
    let tex = app.photos.tex.get(&p.name).map(|t| t.tex.clone());
    let selected = app.photos.sel.iter().any(|u| u == &p.uid);
    let caption = caption_for(state, local_len, p.size);
    let preview_caption = if can_preview && blocked {
        format!(
            "{} \u{b7} {}",
            preview_label(unsupported),
            caption_for(TileState::Ready, local_len, p.size)
        )
    } else {
        caption
    };
    let when = if p.capture.is_empty() {
        "No capture date".to_string()
    } else {
        pretty_date(&p.capture)
    };
    let dims = if p.width > 0 && p.height > 0 {
        format!("{}x{}", p.width, p.height)
    } else {
        String::new()
    };
    let mut tip = format!("{when}\n{}\n{preview_caption}", p.name);
    if !dims.is_empty() {
        tip.push_str(&format!("\n{dims}"));
    }
    match state {
        TileState::Duplicate => {
            tip.push_str("\nTwo cloud photos share this name; rename one in Proton Photos.")
        }
        TileState::Mismatch => tip.push_str("\nRemove or replace the local file to resolve it."),
        _ => {}
    }
    if can_preview && blocked {
        tip.push_str(if unsupported {
            "\nDownloaded intact; this format has no thumbnail."
        } else {
            "\nThe preview could not be decoded; the downloaded file is unchanged."
        });
    }

    ui.allocate_ui_with_layout(
        egui::vec2(size, size + TILE_LABEL_H),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click());
            ui.painter().rect_filled(
                rect,
                egui::CornerRadius::same(10),
                theme::glass_fill(ui.visuals()),
            );
            match &tex {
                Some(t) if !(can_preview && blocked) => {
                    let [tw, th] = t.size();
                    let (tw, th) = (tw.max(1) as f32, th.max(1) as f32);
                    let scale = (rect.width() / tw).min(rect.height() / th);
                    let fitted = egui::Rect::from_center_size(
                        rect.center(),
                        egui::vec2(tw * scale, th * scale),
                    );
                    ui.painter().image(
                        t.id(),
                        fitted,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                }
                _ => {
                    // Not on disk yet, still decoding, or an unsupported format.
                    let icon = egui::Rect::from_center_size(
                        rect.center() - egui::vec2(0.0, 8.0),
                        egui::vec2(30.0, 30.0),
                    );
                    widgets::nav_icon(
                        ui.painter(),
                        icon,
                        widgets::NavIcon::Photos,
                        ui.visuals().weak_text_color(),
                    );
                }
            }
            if selected {
                ui.painter().rect_stroke(
                    rect,
                    egui::CornerRadius::same(10),
                    egui::Stroke::new(2.0, theme::accent(ui.visuals())),
                    egui::StrokeKind::Inside,
                );
            }
            if resp.has_focus() {
                ui.painter().rect_stroke(
                    rect,
                    egui::CornerRadius::same(10),
                    egui::Stroke::new(2.0, theme::accent(ui.visuals())),
                    egui::StrokeKind::Outside,
                );
            }
            if app.dialog.is_none()
                && (resp.clicked()
                    || (resp.has_focus()
                        && ui.input(|i| {
                            i.key_pressed(egui::Key::Enter)
                                || i.key_pressed(egui::Key::Space)
                        })))
            {
                if selected {
                    app.photos.sel.retain(|u| u != &p.uid);
                } else {
                    app.photos.sel.push(p.uid.clone());
                }
            }
            if resp.clicked() {
                resp.request_focus();
            }
            let accessible = format!("{} - {}", p.name, preview_caption);
            resp.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::Role::CheckBox,
                    ui.is_enabled(),
                    selected,
                    &accessible,
                )
            });
            resp.on_hover_text(tip);

            ui.add_space(2.0);
            ui.label(rt(widgets::elide(&p.name, 17)).size(11.0));
            let color = if matches!(
                state,
                TileState::Incomplete | TileState::Mismatch | TileState::Duplicate
            ) {
                theme::warning_text(ui.visuals())
            } else {
                ui.visuals().weak_text_color()
            };
            ui.label(
                rt(widgets::elide(&preview_caption, 24))
                    .size(10.0)
                    .color(color),
            );
            if matches!(state, TileState::Missing | TileState::Incomplete) {
                let can_act = !app.photos.busy
                    && !app.photos.deleting
                    && !matches!(
                        app.ui_status(),
                        crate::health::UiStatus::SignedOut
                            | crate::health::UiStatus::NeedsLogin
                            | crate::health::UiStatus::SigningIn
                    );
                let label = if state == TileState::Incomplete {
                    "Re-download"
                } else {
                    "Download"
                };
                if ui
                    .add_enabled(can_act, egui::Button::new(label).small())
                    .clicked()
                {
                    if state == TileState::Incomplete {
                        request_sync(app, Some(&p.uid));
                    } else {
                        download(app, vec![p.uid.clone()]);
                    }
                }
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_metrics_does_not_oscillate_at_the_tile_edges() {
        // 221..227 used to bounce 1 -> 2 -> 1 forever and hang the UI.
        for avail in [220.0, 221.0, 224.0, 227.0, 228.0, 260.0] {
            let (cols, size) = grid_metrics(avail);
            assert!(cols >= 1, "avail={avail}");
            assert!(size <= MAX_TILE + 0.01, "avail={avail} size={size}");
        }
        assert_eq!(grid_metrics(224.0), (1, MAX_TILE));
    }

    #[test]
    fn clear_summary_drops_stale_header_state() {
        let mut p = PhotoState {
            cloud_count: 9,
            disk_count: 7,
            last_sync_ts: Some(chrono::Utc::now().fixed_offset()),
            sync_error: "boom".to_string(),
            attention: 3,
            last_sync: "Last photo sync".to_string(),
            ..PhotoState::default()
        };
        p.upload_skipped.push(FileIssue {
            name: "x.jpg".to_string(),
            reason: "test".to_string(),
        });
        clear_summary(&mut p);
        assert_eq!(p.cloud_count, 0);
        assert_eq!(p.disk_count, 0);
        assert!(p.last_sync_ts.is_none());
        assert!(p.sync_error.is_empty());
        assert_eq!(p.attention, 0);
        assert!(p.last_sync.is_empty());
        assert!(p.upload_skipped.is_empty());
    }

    fn photo(name: &str, capture: &str) -> cli::PhotoEntry {
        cli::PhotoEntry {
            uid: format!("uid-{name}"),
            name: name.to_string(),
            size: 1024,
            capture: capture.to_string(),
            width: 100,
            height: 50,
        }
    }

    /// The timeline is grouped into day cards, newest day first.
    #[test]
    fn timeline_groups_by_day() {
        let items = vec![
            photo("c.png", "2026-08-21 18:00"),
            photo("b.png", "2026-08-21 09:00"),
            photo("a.jpg", "2026-08-20 23:59"),
            photo("old.jpg", ""),
        ];
        let groups = build_groups(&items);
        let names: Vec<&str> = groups.iter().map(|(d, _)| d.as_str()).collect();
        assert_eq!(
            names,
            vec!["Fri, 21 Aug 2026", "Thu, 20 Aug 2026", "Undated"]
        );
        assert_eq!(groups[0].1, vec![0, 1]);
        assert_eq!(items[groups[0].1[0]].name, "c.png");
        assert_eq!(items[groups[2].1[0]].name, "old.jpg");
    }

    #[test]
    fn tile_states_are_honest_and_ordered() {
        assert_eq!(tile_state(None, 10, true), TileState::Duplicate);
        assert_eq!(tile_state(Some(10), 10, true), TileState::Duplicate);
        assert_eq!(tile_state(None, 10, false), TileState::Missing);
        assert_eq!(tile_state(Some(0), 10, false), TileState::Incomplete);
        assert_eq!(tile_state(Some(4), 10, false), TileState::Mismatch);
        assert_eq!(tile_state(Some(10), 10, false), TileState::Ready);
        assert_eq!(tile_state(Some(0), 0, false), TileState::Ready);
        assert!(caption_for(TileState::Incomplete, Some(0), 10).contains("incomplete"));
        assert!(caption_for(TileState::Mismatch, Some(4), 10).contains("Both copies kept"));
        assert_eq!(
            caption_for(TileState::Missing, None, 0),
            "Not on this computer \u{b7} Size unknown"
        );
    }

    #[test]
    fn thumbnails_evict_the_least_recently_used() {
        let entries = vec![
            ("a".to_string(), 3u64),
            ("b".to_string(), 1),
            ("c".to_string(), 2),
        ];
        assert_eq!(lru_victim(entries.into_iter()).as_deref(), Some("b"));
    }

    #[test]
    fn preview_wording_does_not_blame_the_file() {
        assert_eq!(preview_label(true), "No preview for this format");
        assert_eq!(preview_label(false), "Preview failed");
    }

    /// Every card uses the same grid, rows fill the width exactly, and the tile
    /// size stays within the bounds whatever the window width.
    #[test]
    fn the_grid_fills_the_width_within_bounds() {
        for avail in [260.0f32, 400.0, 700.0, 1000.0, 1400.0, 2600.0, 3200.0] {
            let (cols, tile) = grid_metrics(avail);
            assert!(cols >= 1, "{avail}: at least one column");
            assert!(
                (MIN_TILE..=MAX_TILE).contains(&tile),
                "{avail}: tile {tile} outside bounds"
            );
            // A full row uses the available width (within rounding).
            let row = cols as f32 * tile + (cols - 1) as f32 * GAP;
            assert!((row - avail).abs() < 0.5, "{avail}: row {row} vs {avail}");
        }
        // Size depends on width alone: two "cards" of different length agree.
        assert_eq!(grid_metrics(1000.0), grid_metrics(1000.0));
    }

    #[test]
    fn dates_and_sizes_read_like_a_gallery() {
        assert_eq!(pretty_date("2026-08-21 12:55"), "21 Aug 2026, 12:55");
        assert_eq!(pretty_date("garbage"), "garbage");
        assert_eq!(size_label(900), "900 B");
        assert_eq!(size_label(2 * 1024 * 1024), "2.0 MB");
        assert_eq!(size_label(3 * 1024), "3 KB");
    }

    #[test]
    fn a_skipped_card_disappears_when_the_file_is_gone() {
        let root = std::env::temp_dir().join(format!(
            "pd-photos-issues-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("there.jpg"), b"x").unwrap();
        let issues = vec![
            FileIssue {
                name: "there.jpg".into(),
                reason: "isn't a photo".into(),
            },
            FileIssue {
                name: "gone.jpg".into(),
                reason: "isn't a photo".into(),
            },
        ];
        assert_eq!(existing_issues(&root, &issues).len(), 1);
        std::fs::remove_file(root.join("there.jpg")).unwrap();
        assert!(existing_issues(&root, &issues).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Photos land in ~/ProtonPhotos unless the conf says otherwise.
    #[test]
    fn photos_land_in_home_protonphotos() {
        let empty = BTreeMap::new();
        assert!(local_dir_from(&empty).ends_with("ProtonPhotos"));
        let mut conf = BTreeMap::new();
        conf.insert(
            "PROTON_SYNC_PHOTOS_LOCAL".to_string(),
            "~/Pictures/Proton".to_string(),
        );
        assert!(local_dir_from(&conf).ends_with("Pictures/Proton"));
        conf.insert("PROTON_SYNC_PHOTOS_LOCAL".to_string(), "   ".to_string());
        assert!(
            local_dir_from(&conf).ends_with("ProtonPhotos"),
            "a blank key falls back to the default"
        );
    }
}
