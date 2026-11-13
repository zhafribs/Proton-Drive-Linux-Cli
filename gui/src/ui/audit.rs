//! Headless UI confidence checks: render every page through egui_kittest so
//! AccessKit supplies the same names a screen reader sees, keyboard events
//! travel through egui's real focus system, and the wgpu renderer produces the
//! screenshot matrix committed under `docs/audit/m8/`.

use super::{App, AppCtx, Dialog, InputIntent, Page, UpdateUi};
use crate::health::Health;
use crate::journal::{ChangeKind, ChangeRow};
use super::settings;
use crate::{cli, paths::Paths, prefs};
use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;

fn temp_dir(tag: &str) -> PathBuf {
    // Every harness gets its own prefs/state. Tests for the same page run in
    // parallel, and a shared directory let one test's Save/onboarding rewrite
    // another test's fixture.
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "pd-ui-audit-{}-{tag}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("state")).unwrap();
    dir
}

fn sample_changes() -> Vec<ChangeRow> {
    let now = chrono::Utc::now();
    [
        (ChangeKind::Uploaded, "report.pdf", "Documents/report.pdf"),
        (ChangeKind::Downloaded, "photo.jpg", "Photos/photo.jpg"),
        (ChangeKind::FolderCreated, "Invoices", "Documents/Invoices"),
        (ChangeKind::Removed, "old.zip", "Downloads/old.zip"),
        (ChangeKind::Conflict, "notes.md", "Documents/notes.md"),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (kind, name, rel))| ChangeRow {
        kind,
        name: name.into(),
        rel: rel.into(),
        ts: Some((now - chrono::Duration::minutes(i as i64 * 7)).into()),
    })
    .collect()
}

/// A fully local `App` with no pollers, tray, service or network. It uses the
/// same `draw` path as the real window, so the audit sees the real shell.
fn fixture(page: Page) -> App {
    let dir = temp_dir(&format!("{page:?}"));
    let state = dir.join("state");
    let paths = Paths {
        conf_file: dir.join("proton-sync.conf"),
        unit_file: dir.join("proton-sync.service"),
        state_dir: state.clone(),
        health_file: state.join("health.json"),
        wake_file: state.join("wake"),
        lock_file: dir.join("gui.lock"),
        snapshot_file: state.join("remote-snapshot.json"),
        cloud_cache_file: state.join("cloud-cache.json"),
        release_cache_file: state.join("release-cache.json"),
        update_mark_file: state.join("update-in-progress.json"),
        autostart_file: dir.join("autostart.desktop"),
        menu_desktop_file: dir.join("menu.desktop"),
        menu_icon_file: dir.join("menu.png"),
        prefs_file: dir.join("prefs.conf"),
        appimage: None,
    };
    let prefs = prefs::Prefs::load(&paths.prefs_file);
    let conf = BTreeMap::new();
    let conf_extras = Vec::new();
    let draft = settings::Draft::load(&conf, &conf_extras, &prefs, &paths);
    let (tx, rx) = mpsc::channel();

    let mut health = Health {
        status: "ok".into(),
        remote_entries: Some(42),
        local_entries: Some(42),
        uploaded: 3,
        downloaded: 2,
        actions: 9,
        conflicts: 0,
        ..Default::default()
    };
    health.ts = Some(chrono::Utc::now().to_rfc3339());

    let cloud = super::cloud::CloudState {
        opened: true,
        fetched: std::collections::HashMap::from([(String::new(), 0.0)]),
        entries: vec![
        cli::RemoteEntry {
            name: "Documents".into(),
            rel: "Documents".into(),
            uid: "d1".into(),
            dir: true,
            size: 0,
            mtime: String::new(),
            doc: false,
        },
        cli::RemoteEntry {
            name: "Photos".into(),
            rel: "Photos".into(),
            uid: "d2".into(),
            dir: true,
            size: 0,
            mtime: String::new(),
            doc: false,
        },
        cli::RemoteEntry {
            name: "report.pdf".into(),
            rel: "report.pdf".into(),
            uid: "f1".into(),
            dir: false,
            size: 2_400_000,
            mtime: "2026-11-10 12:30".into(),
            doc: false,
        },
        cli::RemoteEntry {
            name: "notes.md".into(),
            rel: "notes.md".into(),
            uid: "f2".into(),
            dir: false,
            size: 8_192,
            mtime: "2026-11-11 09:05".into(),
            doc: false,
        },
        ],
        ..Default::default()
    };

    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&paths.lock_file)
        .unwrap();

    App {
        ctx: AppCtx {
            paths,
            prefs: prefs.clone(),
            conf,
            conf_extras,
            tx,
            rx,
            health,
            identity: Some(cli::Identity {
                version: "0.9.0".into(),
                email: Some("you@example.com".into()),
                logged_in: true,
                error: None,
            }),
        },
        _lock: lock,
        active: true,
        enabled: true,
        page,
        last_page: page,
        nav_width: prefs.nav_width,
        nav_drag: false,
        journal_open: Arc::new(AtomicBool::new(false)),
        identity_retry: None,
        last_email: Some("you@example.com".into()),
        paused_by_user: false,
        paused_at: None,
        login_started: None,
        stats: Some((128, 2_500_000_000)),
        draft,
        changes: sample_changes(),
        log_lines: vec![
            "2026-11-12 09:00:01 sync: uploaded report.pdf".into(),
            "2026-11-12 09:00:03 sync: downloaded photo.jpg".into(),
        ],
        act: Default::default(),
        cloud,
        cloud_login_requested: false,
        photos: Default::default(),
        photo_login_requested: false,
        recovery: Default::default(),
        input_buf: String::new(),
        input_focus: false,
        search_focus: false,
        cool: Default::default(),
        last_seen: Some(chrono::Utc::now().into()),
        prev_hstatus: String::new(),
        tray_warned: false,
        dialog: None,
        dialog_seen: None,
        dialog_focus: false,
        save_hint: String::new(),
        save_checking: false,
        sub_override: None,
        toasts: Default::default(),
        prefs_baseline: Some(prefs),
        settings_tab: 0,
        upd: UpdateUi::default(),
        onboarding: None,
        hidden: false,
        hide_on_start: false,
        quitting: false,
        style_dirty: false,
        system_theme: prefs::Theme::Dark,
        preview_theme: None,
        preview_scale: None,
        scale_applied: None,
        theme_rx: mpsc::channel().1,
        icon_tex: None,
        tray: None,
        wayland_fallback: false,
        relaunch_slot: Default::default(),
    }
}

/// `scale` is the interface scale (50/100/200%). The window size is in
/// logical points, so a 720x400 physical capture at 50% is a 1440x800 point
/// layout and at 200% it is a 720x400 point layout on a 2x surface.
fn build_harness(page: Page, size: egui::Vec2, scale: f32) -> Harness<'static, App> {
    Harness::builder()
        .with_size(size)
        .with_pixels_per_point(scale)
        .with_context_setup(|ctx| {
            ctx.add_font_provider(std::sync::Arc::new(
                egui_system_font_provider::SystemFontProvider::new(),
            ));
        })
        .build_ui_state(|ui, app: &mut App| app.draw(ui), fixture(page))
}

fn harness(page: Page, size: egui::Vec2, scale: f32) -> Harness<'static, App> {
    let mut harness = build_harness(page, size, scale);
    harness.run();
    harness
}

#[test]
fn kittest_renders_a_frame() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(320.0, 120.0))
        .wgpu()
        .build_ui(|ui| {
            ui.heading("Proton Drive");
            let _ = ui.button("Save");
        });
    harness.run();
    let image = harness.render().expect("wgpu render");
    assert_eq!(image.width(), 320);
    assert_eq!(image.height(), 120);
}

#[test]
fn every_page_renders_at_the_minimum_window() {
    for page in [
        Page::Home,
        Page::Cloud,
        Page::Photos,
        Page::Activity,
        Page::Settings,
        Page::About,
        Page::Recovery,
    ] {
        // kittest panics on an input widget with no accessible name.
        let _ = harness(page, egui::vec2(720.0, 400.0), 1.0);
    }
}


#[test]
fn keyboard_only_reaches_settings_and_activates_it() {
    let mut harness = harness(Page::Home, egui::vec2(1024.0, 720.0), 1.0);
    let mut focused = std::collections::HashSet::new();
    let mut reached = false;
    for _ in 0..80 {
        harness.key_press(egui::Key::Tab);
        harness.run();
        if let Some(id) = harness.ctx.memory(|m| m.focused()) {
            focused.insert(id);
        }
        if harness
            .get_all_by_label("Settings")
            .any(|node| node.accesskit_node().is_focused())
        {
            reached = true;
            break;
        }
    }
    assert!(reached, "Tab never reached the Settings nav row");
    assert!(
        focused.len() >= 5,
        "Tab visited only {} controls before Settings",
        focused.len()
    );

    harness.key_press(egui::Key::Enter);
    harness.run();
    assert_eq!(harness.state().page, Page::Settings);
}

fn save_png(image: &image::RgbaImage, path: &std::path::Path) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let file = std::fs::File::create(path).unwrap();
    let mut enc = png::Encoder::new(file, image.width(), image.height());
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .unwrap()
        .write_image_data(image.as_raw())
        .unwrap();
}

fn capture(page: Page, physical: egui::Vec2, scale: f32) {
    let logical = physical / scale;
    let mut harness = build_harness(page, logical, scale);
    // Some pages keep a spinner repainting; step a fixed number of frames
    // instead of waiting for a settle that will never come.
    for _ in 0..4 {
        harness.step();
    }
    harness.check_accessibility();
    let image = harness.render().expect("wgpu screenshot");
    assert_eq!((image.width(), image.height()), (physical.x as u32, physical.y as u32));
    let colors: std::collections::HashSet<[u8; 4]> =
        image.pixels().map(|p| p.0).collect();
    assert!(colors.len() > 8, "{page:?} at {scale}x rendered a blank surface");
    if let Ok(dir) = std::env::var("PROTON_UI_SCREENSHOTS") {
        let name = format!(
            "{}-{}-{}x{}.png",
            format!("{page:?}").to_lowercase(),
            (scale * 100.0).round() as u32,
            physical.x as u32,
            physical.y as u32
        );
        save_png(&image, &std::path::Path::new(&dir).join(name));
    }
}

#[test]
fn screenshot_matrix_covers_pages_and_scales() {
    for page in [
        Page::Home,
        Page::Cloud,
        Page::Photos,
        Page::Activity,
        Page::Settings,
        Page::About,
        Page::Recovery,
    ] {
        capture(page, egui::vec2(720.0, 400.0), 1.0);
    }
    for page in [Page::Home, Page::Cloud, Page::Settings, Page::About] {
        capture(page, egui::vec2(720.0, 400.0), 0.5);
        capture(page, egui::vec2(1440.0, 800.0), 2.0);
    }
}

#[test]
fn status_variants_render_without_missing_names() {
    for (status, paused, signed_out) in [
        ("ok", false, false),
        ("syncing", false, false),
        ("needs_login", false, false),
        ("error", false, false),
        ("ok", true, false),
        ("deletes_held", false, false),
        ("ok", false, true),
    ] {
        let mut harness = harness(Page::Home, egui::vec2(720.0, 400.0), 1.0);
        let app = harness.state_mut();
        app.ctx.health.status = status.into();
        app.paused_by_user = paused;
        if signed_out {
            app.ctx.identity = None;
            app.last_email = None;
        }
        harness.run();
    }
}

#[test]
fn dialogs_render_without_missing_names() {
    let dialogs = vec![
        Dialog::Info {
            title: "About this update".into(),
            body: "SHA-256 verified before install.".into(),
        },
        Dialog::ConfirmSave,
        Dialog::ConfirmLogout,
        Dialog::PauseMenu,
        Dialog::DirBrowse {
            cwd: std::path::PathBuf::from("/home/you/ProtonDrive"),
        },
        Dialog::Input {
            title: "New folder".into(),
            label: "Folder name".into(),
            intent: InputIntent::CloudNewFolder,
        },
        Dialog::Shortcuts,
        Dialog::ConfirmTrash { n: 3 },
        Dialog::ConfirmDelete { n: 2 },
        Dialog::ConfirmEmptyTrash,
        Dialog::ConfirmClearCache,
        Dialog::ConfirmPhotoDelete {
            uids: vec!["a".into(), "b".into()],
        },
        Dialog::CloudMove,
        Dialog::ConfirmHeldDeletes {
            local: 2,
            cloud: 3,
            local_digest: "local".into(),
            cloud_digest: "cloud".into(),
            sample: vec!["Documents/report.pdf".into()],
        },
    ];
    for dialog in dialogs {
        let mut harness = harness(Page::Cloud, egui::vec2(720.0, 400.0), 1.0);
        harness.state_mut().dialog = Some(dialog);
        harness.run();
    }
}

#[test]
fn onboarding_steps_render_without_missing_names() {
    for step in 0..3 {
        let mut harness = harness(Page::Home, egui::vec2(720.0, 400.0), 1.0);
        harness.state_mut().onboarding = Some(step);
        harness.run();
        assert_eq!(harness.state().onboarding, Some(step));
    }
}

#[test]
fn update_progress_renders_without_missing_names() {
    let mut harness = harness(Page::Home, egui::vec2(720.0, 400.0), 1.0);
    harness.state_mut().upd.downloading = true;
    harness.state_mut().upd.label = "Downloading and verifying the update...".into();
    // The spinner requests repaints forever; step instead of run().
    for _ in 0..4 {
        harness.step();
    }
    harness.check_accessibility();
}

#[test]
fn every_page_renders_in_malay() {
    crate::i18n::set_thread(Some(crate::i18n::Language::Malay));
    for page in [
        Page::Home,
        Page::Cloud,
        Page::Photos,
        Page::Activity,
        Page::Settings,
        Page::About,
        Page::Recovery,
    ] {
        let _ = harness(page, egui::vec2(720.0, 400.0), 1.0);
    }
    for step in 0..3 {
        let mut harness = harness(Page::Home, egui::vec2(720.0, 400.0), 1.0);
        harness.state_mut().onboarding = Some(step);
        harness.run();
    }
    crate::i18n::set_thread(None);
}

#[test]
fn malay_screenshot_matrix() {
    crate::i18n::set_thread(Some(crate::i18n::Language::Malay));
    for page in [
        Page::Home,
        Page::Cloud,
        Page::Photos,
        Page::Activity,
        Page::Settings,
        Page::About,
        Page::Recovery,
    ] {
        capture(page, egui::vec2(1024.0, 720.0), 1.0);
    }
    crate::i18n::set_thread(None);
}

#[test]
fn dialogs_render_in_malay() {
    crate::i18n::set_thread(Some(crate::i18n::Language::Malay));
    let dialogs = vec![
        Dialog::Info {
            title: "About this update".into(),
            body: "SHA-256 verified before install.".into(),
        },
        Dialog::ConfirmSave,
        Dialog::ConfirmLogout,
        Dialog::PauseMenu,
        Dialog::DirBrowse {
            cwd: std::path::PathBuf::from("/home/you/ProtonDrive"),
        },
        Dialog::Input {
            title: "New folder".into(),
            label: "Folder name".into(),
            intent: InputIntent::CloudNewFolder,
        },
        Dialog::Shortcuts,
        Dialog::ConfirmTrash { n: 3 },
        Dialog::ConfirmDelete { n: 2 },
        Dialog::ConfirmEmptyTrash,
        Dialog::ConfirmClearCache,
        Dialog::ConfirmPhotoDelete {
            uids: vec!["a".into(), "b".into()],
        },
        Dialog::CloudMove,
        Dialog::ConfirmHeldDeletes {
            local: 2,
            cloud: 3,
            local_digest: "local".into(),
            cloud_digest: "cloud".into(),
            sample: vec!["Documents/report.pdf".into()],
        },
    ];
    for dialog in dialogs {
        let mut harness = harness(Page::Cloud, egui::vec2(720.0, 400.0), 1.0);
        harness.state_mut().dialog = Some(dialog);
        harness.run();
    }
    crate::i18n::set_thread(None);
}
