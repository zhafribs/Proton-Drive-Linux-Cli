//! Settings: sync options, notifications, background behavior, appearance,
//! updates, install helpers, advanced tuning - plus the conf/unit write flow
//! (`do_save`), all with the PyQt app's exact labels and validations.

use super::{widgets, App, Dialog};
use crate::{conf, desktop, service};
use eframe::egui;
use std::collections::{BTreeMap, HashMap, HashSet};

pub struct Draft {
    // --- conf-backed (Save writes these) ---
    pub local: String,
    pub remote: String,
    pub interval_idx: usize,
    pub inotify: bool,
    pub deletes: bool,
    pub workers: i32,
    pub retries: i32,
    pub exclude: String,
    pub selective: bool,
    pub env_text: String,

    // --- selective-sync tree ---
    /// Explicit checkbox state per rendered node; "" is the root row.
    pub nodes: HashMap<String, bool>,
    pub children: HashMap<String, Vec<(String, String)>>,
    pub loading: HashSet<String>,
    pub expanded: HashSet<String>,
    pub sel_hint: String,
    /// SUBFOLDERS from the conf at load time; implied states materialize from
    /// it until the user touches a checkbox.
    pub initial_selection: Vec<String>,
    pub sel_root_loaded: bool,

    // --- prefs-backed toggles (saved immediately) ---
    pub top: bool,
    pub close_tray: bool,
    pub start_tray: bool,
    pub autostart: bool,
    pub appstart: bool,
    pub theme_idx: usize,
    pub updates_auto: bool,
    pub note_errors: bool,
    pub note_conflicts: bool,
    pub note_done: bool,
    pub show_adv: bool,
}

impl Draft {
    pub fn load(
        values: &BTreeMap<String, String>,
        extras: &[String],
        prefs: &crate::prefs::Prefs,
        paths: &crate::paths::Paths,
    ) -> Draft {
        let get = |k: &str, d: &str| values.get(k).cloned().unwrap_or_else(|| d.to_string());
        let secs: u64 = get("PROTON_SYNC_INTERVAL", "300").parse().unwrap_or(300);
        let selection: Vec<String> = get("PROTON_SYNC_SUBFOLDERS", "")
            .split(',')
            .filter(|s| !s.is_empty())
            .map(|s| s.trim_matches('/').to_string())
            .collect();
        Draft {
            local: get("PROTON_SYNC_LOCAL", ""),
            remote: get("PROTON_SYNC_REMOTE", "/my-files"),
            interval_idx: conf::interval_index(secs),
            inotify: get("PROTON_SYNC_INOTIFY", "1") == "1",
            deletes: get("PROTON_SYNC_DELETES", "1") == "1",
            workers: get("PROTON_SYNC_WORKERS", "12").parse().unwrap_or(12).clamp(1, 16),
            retries: get("PROTON_SYNC_RETRIES", "3").parse().unwrap_or(3).clamp(1, 10),
            exclude: get("PROTON_SYNC_EXCLUDE", ""),
            selective: !selection.is_empty(),
            env_text: extras.join("\n"),
            nodes: HashMap::from([(String::new(), selection.is_empty())]),
            children: HashMap::new(),
            loading: HashSet::new(),
            expanded: HashSet::new(),
            sel_hint: String::new(),
            initial_selection: selection,
            sel_root_loaded: false,
            top: prefs.always_on_top(),
            close_tray: prefs.close_to_tray(),
            start_tray: prefs.start_in_tray(),
            autostart: false, // filled by the UnitEnabled worker
            appstart: desktop::autostart_installed(paths),
            theme_idx: match prefs.theme() {
                crate::prefs::Theme::System => 0,
                crate::prefs::Theme::Light => 1,
                crate::prefs::Theme::Dark => 2,
            },
            updates_auto: prefs.updates_auto(),
            note_errors: prefs.notify_errors(),
            note_conflicts: prefs.notify_conflicts(),
            note_done: prefs.notify_complete(),
            show_adv: false,
        }
    }

    /// A folder listing arrived: store it and materialize checkbox states.
    pub fn on_folders(&mut self, parent: &str, folders: Vec<(String, String)>) {
        let is_root = parent.is_empty();
        if is_root {
            self.sel_root_loaded = true;
        }
        for (_name, rel) in &folders {
            self.materialize(rel, parent);
        }
        self.children.insert(parent.to_string(), folders);
        self.loading.remove(parent);
    }

    /// State for a freshly listed node: parent's state, else implied by the
    /// conf selection (folder or any of its ancestors picked).
    fn materialize(&mut self, rel: &str, parent: &str) {
        if self.nodes.contains_key(rel) {
            return;
        }
        let from_parent = self.nodes.get(parent).copied().unwrap_or(false);
        let implied = self
            .initial_selection
            .iter()
            .any(|s| rel == s || (rel.starts_with(s.as_str()) && rel[s.len()..].starts_with('/')));
        self.nodes.insert(rel.to_string(), from_parent || implied);
    }

    /// Click: flip, cascade down through loaded children, reflect up.
    pub fn toggle_node(&mut self, rel: &str) {
        let new_val = !self.nodes.get(rel).copied().unwrap_or(false);
        self.set_recursive(rel, new_val);
        let segs: Vec<&str> = rel.split('/').collect();
        for depth in (0..segs.len()).rev() {
            self.recompute_node(&segs[..=depth].join("/"));
        }
        self.recompute_node("");
    }

    fn set_recursive(&mut self, rel: &str, val: bool) {
        self.nodes.insert(rel.to_string(), val);
        if let Some(kids) = self.children.get(rel).cloned() {
            for (_n, kid) in kids {
                self.set_recursive(&kid, val);
            }
        }
    }

    /// Parent becomes checked only when ALL loaded children are checked;
    /// partial state is display-only (Qt stored partial as non-checked too,
    /// so the save-time walk descends into it).
    fn recompute_node(&mut self, prefix: &str) {
        let all = match self.children.get(prefix) {
            Some(kids) if !kids.is_empty() => kids
                .iter()
                .all(|(_, k)| self.nodes.get(k).copied().unwrap_or(false)),
            _ => return,
        };
        self.nodes.insert(prefix.to_string(), all);
    }

    /// Checked-with-some-unchecked marker for the row glyph.
    fn partial(&self, prefix: &str) -> bool {
        match self.children.get(prefix) {
            Some(kids) if !kids.is_empty() => {
                let any_true = kids.iter().any(|(_, k)| self.nodes.get(k).copied().unwrap_or(false));
                let all_true =
                    kids.iter().all(|(_, k)| self.nodes.get(k).copied().unwrap_or(false));
                any_true && !all_true
            }
            _ => false,
        }
    }

    /// Qt's `_derive_selection`: root ticked = everything (empty list);
    /// otherwise walk, collecting checked nodes and descending into unchecked
    /// ones (unloaded branches contribute nothing, exactly like Qt).
    pub fn derive_selection(&self) -> Vec<String> {
        if self.nodes.get("").copied().unwrap_or(false) {
            return Vec::new();
        }
        let mut out = Vec::new();
        self.walk_derive("", &mut out);
        out
    }

    fn walk_derive(&self, parent: &str, out: &mut Vec<String>) {
        let Some(kids) = self.children.get(parent) else { return };
        for (_name, rel) in kids {
            if self.nodes.get(rel).copied().unwrap_or(false) {
                out.push(rel.clone());
            } else {
                self.walk_derive(rel, out);
            }
        }
    }

    /// "Refresh folder list": forget listings, keep the user's picks.
    pub fn refresh_roots(&mut self) {
        let root_checked = self.nodes.get("").copied().unwrap_or(false);
        let derived = self.derive_selection();
        self.nodes.retain(|k, _| k.is_empty());
        self.nodes.insert(String::new(), root_checked && derived.is_empty());
        self.children.retain(|k, _| !k.is_empty());
        self.loading.clear();
        self.expanded.clear();
        self.initial_selection = derived;
        self.loading.insert(String::new());
    }
}

// ---------------------------------------------------------------- page ----

pub fn draw(app: &mut App, ui: &mut egui::Ui) {
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.label(egui::RichText::new("Settings").strong().size(24.0));
            ui.add_space(12.0);

            folder_card(app, ui);
            ui.add_space(14.0);
            what_syncs_card(app, ui);
            ui.add_space(14.0);
            how_often_card(app, ui);
            ui.add_space(14.0);
            notifications_card(app, ui);
            ui.add_space(14.0);
            background_card(app, ui);
            ui.add_space(14.0);
            appearance_card(app, ui);
            ui.add_space(14.0);
            updates_card(app, ui);
            ui.add_space(14.0);
            install_card(app, ui);
            ui.add_space(14.0);

            // --- advanced (hidden) ---
            let label = if app.draft.show_adv {
                "Advanced and troubleshooting \u{25be}"
            } else {
                "Advanced and troubleshooting \u{25b8}"
            };
            if ui.add(egui::Button::new(label).frame(false)).clicked() {
                app.draft.show_adv = !app.draft.show_adv;
            }
            if app.draft.show_adv {
                ui.add_space(10.0);
                fine_tuning_card(app, ui);
                ui.add_space(14.0);
                troubleshooting_card(app, ui);
                ui.add_space(14.0);
                env_card(app, ui);
            }

            ui.add_space(18.0);

            // --- save bar ---
            ui.horizontal(|ui| {
                if widgets::primary_button(ui, "  Save changes") {
                    match validate(app) {
                        Some((title, body)) => {
                            app.dialog = Some(Dialog::Info { title, body });
                        }
                        None => app.dialog = Some(Dialog::ConfirmSave),
                    }
                }
                if ui.button("Discard").clicked() {
                    reload(app);
                }
                ui.label(egui::RichText::new(app.save_hint.clone()).weak());
            });
            ui.add_space(20.0);
        });
}

fn validate(app: &App) -> Option<(String, String)> {
    if !app.draft.local.starts_with('/') {
        return Some((
            "Folder missing".into(),
            "Please choose a folder to sync first.".into(),
        ));
    }
    if app.draft.selective
        && app.draft.derive_selection().is_empty()
        && app.draft.nodes.get("").copied() != Some(true)
    {
        return Some((
            "Nothing picked".into(),
            "You turned on selective sync but picked no folders.\nPick at least one folder in \
             the list, or switch selective sync off."
                .into(),
        ));
    }
    None
}

/// Re-read the conf into the draft (startup + the Discard button).
pub fn reload(app: &mut App) {
    let values = conf::read(&app.paths.conf_file);
    let extras = conf::read_extras(&app.paths.conf_file);
    app.conf = values.clone();
    app.conf_extras = extras.clone();
    app.draft = Draft::load(&values, &extras, &app.prefs, &app.paths);
    app.save_hint.clear();
    // Kick a fresh root listing (Qt re-ran _sel_load_root on every load).
    if app.draft.loading.insert(String::new()) {
        let remote = app.draft.remote.clone();
        let tx = app.tx.clone();
        std::thread::spawn(move || {
            let folders = cli_fetch(&remote, "");
            let _ = tx.send(super::Back::SelFolders {
                parent: String::new(),
                folders,
            });
        });
    }
}

fn cli_fetch(remote: &str, rel: &str) -> Vec<(String, String)> {
    crate::cli::fetch_remote_folders(remote, rel)
}

/// Write conf + restart flow (worker-side; the hint lands via SaveHint).
pub fn do_save(app: &mut App) {
    if let Some((title, body)) = validate(app) {
        app.dialog = Some(Dialog::Info { title, body });
        return;
    }
    let d = &app.draft;
    let selection = if d.selective { d.derive_selection() } else { Vec::new() };
    let mut values: BTreeMap<String, String> = BTreeMap::new();
    values.insert("PROTON_SYNC_LOCAL".into(), d.local.clone());
    values.insert("PROTON_SYNC_REMOTE".into(), d.remote.clone());
    values.insert(
        "PROTON_SYNC_INTERVAL".into(),
        conf::INTERVAL_CHOICES[d.interval_idx.min(conf::INTERVAL_CHOICES.len() - 1)]
            .1
            .to_string(),
    );
    values.insert("PROTON_SYNC_DELETES".into(), flag(d.deletes));
    values.insert("PROTON_SYNC_INOTIFY".into(), flag(d.inotify));
    values.insert("PROTON_SYNC_WORKERS".into(), d.workers.to_string());
    values.insert("PROTON_SYNC_RETRIES".into(), d.retries.to_string());
    values.insert("PROTON_SYNC_SUBFOLDERS".into(), selection.join(","));
    values.insert("PROTON_SYNC_EXCLUDE".into(), d.exclude.clone());
    let extras: Vec<String> = d
        .env_text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();

    conf::write(&app.paths.conf_file, &values, &extras);
    app.conf = conf::read(&app.paths.conf_file);
    app.conf_extras = conf::read_extras(&app.paths.conf_file);
    app.draft.initial_selection = selection;
    app.save_hint = "Saving...".into();

    let appimage = app.paths.appimage.clone();
    let tx = app.tx.clone();
    std::thread::spawn(move || {
        let r = service::ensure_service_unit(appimage.as_deref(), false);
        let hint = match r.as_str() {
            "failed" => "Saved, but the sync service could not be updated.".to_string(),
            "" => {
                if service::unit_exists() && !service::unit_has_environment_file() {
                    "Saved. Your install is from an older version - run \
                     scripts/install-sync.sh once to update it."
                        .to_string()
                } else if service::restart() {
                    "Saved. Syncing restarted.".to_string()
                } else {
                    "Saved, but syncing could not be restarted.".to_string()
                }
            }
            written_or_ok => {
                if !service::restart() {
                    "Saved, but syncing could not be restarted.".to_string()
                } else if written_or_ok == "written" {
                    "Saved. Sync service now points at this app copy.".to_string()
                } else {
                    "Saved. Syncing restarted.".to_string()
                }
            }
        };
        let _ = tx.send(super::Back::SaveHint(hint));
    });
}

fn flag(b: bool) -> String {
    if b { "1".into() } else { "0".into() }
}

// --------------------------------------------------------------- cards ----

fn folder_card(app: &mut App, ui: &mut egui::Ui) {
    widgets::card(ui, Some("Where your files live"), |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("\u{1f4c2}").size(26.0));
            ui.add_space(12.0);
            ui.vertical(|ui| {
                ui.label(app.draft.local.clone());
                ui.label(egui::RichText::new("Only this folder is synced.").size(12.0).weak());
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Choose folder...").clicked() {
                    let start = super::expand_user(&app.draft.local);
                    let start = if start.is_dir() { start } else { dirs::home_dir().unwrap_or_default() };
                    app.dialog = Some(Dialog::DirBrowse { cwd: start });
                }
            });
        });
    });
}

fn what_syncs_card(app: &mut App, ui: &mut egui::Ui) {
    widgets::card(ui, Some("What gets synced"), |ui| {
        let mut selective = app.draft.selective;
        if ui
            .checkbox(
                &mut selective,
                "Only sync the folders I pick (instead of everything)",
            )
            .changed()
        {
            app.draft.selective = selective;
            if !selective {
                app.draft.sel_hint = "Everything in your cloud is synced.".into();
            } else {
                app.draft.sel_hint = if app.draft.sel_root_loaded {
                    format!("{} folders", app.draft.children.get("").map(Vec::len).unwrap_or(0))
                } else {
                    "Loading folders...".into()
                };
                if !app.draft.children.contains_key("") {
                    kick_root_load(app);
                }
            }
        }

        if app.draft.selective {
            ui.add_space(6.0);
            sel_tree(app, ui, "");
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("Refresh folder list").clicked() {
                    refresh_roots(app);
                }
                ui.label(egui::RichText::new(app.draft.sel_hint.clone()).size(12.0).weak());
            });
        }

        ui.add_space(10.0);
        widgets::section_label(ui, "NEVER SYNC FILES MATCHING");
        ui.add(
            egui::TextEdit::singleline(&mut app.draft.exclude)
                .hint_text("For example:  *.tmp, .cache, *.iso")
                .desired_width(f32::INFINITY),
        );
    });
}

fn how_often_card(app: &mut App, ui: &mut egui::Ui) {
    widgets::card(ui, Some("How often"), |ui| {
        let mut inotify = app.draft.inotify;
        if widgets::toggle(ui, "Upload my changes right away", &mut inotify, None) {
            app.draft.inotify = inotify;
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("Look for changes in the cloud");
            egui::ComboBox::from_id_salt("interval")
                .selected_text(conf::INTERVAL_CHOICES[app.draft.interval_idx].0)
                .show_ui(ui, |ui| {
                    for (i, (label, _)) in conf::INTERVAL_CHOICES.iter().enumerate() {
                        ui.selectable_value(&mut app.draft.interval_idx, i, *label);
                    }
                });
        });
        ui.add_space(4.0);
        let mut deletes = app.draft.deletes;
        if widgets::toggle(
            ui,
            "Also remove files I delete",
            &mut deletes,
            Some("Deleted files go to the trash."),
        ) {
            app.draft.deletes = deletes;
        }
    });
}

fn notifications_card(app: &mut App, ui: &mut egui::Ui) {
    widgets::card(ui, Some("Notifications"), |ui| {
        let mut v = app.draft.note_errors;
        if ui.checkbox(&mut v, "Tell me if something fails to sync").changed() {
            app.draft.note_errors = v;
            app.prefs.set_bool("notify_errors", v);
        }
        let mut v = app.draft.note_conflicts;
        if ui
            .checkbox(
                &mut v,
                "Tell me if a file was edited in two places at once",
            )
            .changed()
        {
            app.draft.note_conflicts = v;
            app.prefs.set_bool("notify_conflicts", v);
        }
        let mut v = app.draft.note_done;
        if ui.checkbox(&mut v, "Tell me when syncing finished").changed() {
            app.draft.note_done = v;
            app.prefs.set_bool("notify_complete", v);
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button("Send a test notification").clicked() {
                crate::notify::send(
                    "Proton Drive",
                    "This is what notifications look like.",
                );
            }
        });
    });
}

fn background_card(app: &mut App, ui: &mut egui::Ui) {
    widgets::card(ui, Some("Running in the background"), |ui| {
        let mut v = app.draft.top;
        if widgets::toggle(ui, "Keep this window above other windows", &mut v, None) {
            app.draft.top = v;
            app.prefs.set_bool("always_on_top", v);
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::WindowLevel(if v {
                    egui::WindowLevel::AlwaysOnTop
                } else {
                    egui::WindowLevel::Normal
                }));
        }
        let mut v = app.draft.close_tray;
        if widgets::toggle(ui, "Keep running when I close the window", &mut v, None) {
            app.draft.close_tray = v;
            app.prefs.set_bool("close_to_tray", v);
        }
        let mut v = app.draft.start_tray;
        if widgets::toggle(ui, "Start quietly in the tray", &mut v, None) {
            app.draft.start_tray = v;
            app.prefs.set_bool("start_in_tray", v);
        }

        // Start syncing at login: enable/disable the systemd unit (worker).
        let mut v = app.draft.autostart;
        if ui.checkbox(&mut v, "Start syncing when I log in").changed() {
            app.draft.autostart = v;
            app.save_hint = "Applying...".into();
            let paths_appimage = app.paths.appimage.clone();
            let tx = app.tx.clone();
            std::thread::spawn(move || {
                if v {
                    let _ = service::ensure_service_unit(paths_appimage.as_deref(), false);
                }
                let ok = if v { service::enable() } else { service::disable() };
                let _ = tx.send(super::Back::SaveHint(
                    if ok { "Will start when you log in." } else { "Could not change start-up." }
                        .to_string(),
                ));
                let _ = tx.send(super::Back::UnitEnabled(ok && v));
            });
        }

        // Open the app at login: the autostart desktop entry (sync file, fast).
        let mut v = app.draft.appstart;
        if ui
            .checkbox(&mut v, "Open Proton Drive when I log in")
            .changed()
        {
            app.draft.appstart = v;
            desktop::set_autostart(&app.paths, v);
        }
    });
}

fn appearance_card(app: &mut App, ui: &mut egui::Ui) {
    widgets::card(ui, Some("Appearance"), |ui| {
        ui.horizontal(|ui| {
            ui.label("Theme");
            const THEMES: [&str; 3] = ["Match my system", "Light", "Dark"];
            egui::ComboBox::from_id_salt("theme")
                .selected_text(THEMES[app.draft.theme_idx.min(2)])
                .show_ui(ui, |ui| {
                    for (i, t) in THEMES.iter().enumerate() {
                        if ui.selectable_value(&mut app.draft.theme_idx, i, *t).changed() {
                            let theme = match i {
                                1 => crate::prefs::Theme::Light,
                                2 => crate::prefs::Theme::Dark,
                                _ => crate::prefs::Theme::System,
                            };
                            app.prefs.set("theme", theme.as_str());
                            app.style_dirty = true;
                        }
                    }
                });
        });
    });
}

fn updates_card(app: &mut App, ui: &mut egui::Ui) {
    widgets::card(ui, Some("Updates"), |ui| {
        let mut v = app.draft.updates_auto;
        if widgets::toggle(ui, "Check for new versions automatically", &mut v, None) {
            app.draft.updates_auto = v;
            app.prefs.set_bool("updates_auto", v);
        }
        ui.add_space(4.0);
        if !app.upd.label.is_empty() {
            ui.label(egui::RichText::new(app.upd.label.clone()).size(12.0).weak());
        }
        ui.add_space(6.0);
        let has_update = app.upd.available.is_some() && app.paths.appimage.is_some();
        ui.horizontal(|ui| {
            if ui.button("Check now").clicked() {
                app.action_check_updates();
            }
            if has_update
                && ui
                    .add_enabled(
                        !app.upd.downloading,
                        egui::Button::new("Download and restart"),
                    )
                    .clicked()
            {
                app.action_download_update();
            }
            if ui.button("Open download page").clicked() {
                App::open_url(crate::paths::RELEASE_PAGE);
            }
        });
    });
}

fn install_card(app: &mut App, ui: &mut egui::Ui) {
    widgets::card(ui, Some("Installing on Linux"), |ui| {
        let installed = desktop::menu_installed(&app.paths);
        ui.label(
            egui::RichText::new(if installed {
                "Installed in your app menu."
            } else {
                "Not in your app menu yet."
            })
            .size(12.0)
            .weak(),
        );
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button("Add to my app menu").clicked() {
                desktop::set_menu(&app.paths, true);
                app.toast("Added to your app menu.");
            }
            if ui.button("Remove from app menu").clicked() {
                desktop::set_menu(&app.paths, false);
                app.toast("Removed from your app menu.");
            }
        });
    });
}

fn fine_tuning_card(app: &mut App, ui: &mut egui::Ui) {
    widgets::card(ui, Some("Fine-tuning"), |ui| {
        ui.horizontal(|ui| {
            ui.label("Parallel connections");
            ui.add(egui::DragValue::new(&mut app.draft.workers).range(1..=16));
            ui.add_space(24.0);
            ui.label("Retries per folder");
            ui.add(egui::DragValue::new(&mut app.draft.retries).range(1..=10));
        });
    });
}

fn troubleshooting_card(app: &mut App, ui: &mut egui::Ui) {
    widgets::card(ui, Some("If something goes wrong"), |ui| {
        ui.horizontal(|ui| {
            if ui.button("Restart syncing now").clicked() {
                app.action_restart_service();
            }
            if ui.button("Open settings file").clicked() {
                let p = app.paths.conf_file.clone();
                App::open_url(&p.to_string_lossy());
            }
            if ui.button("Copy the log command").clicked() {
                ui.ctx()
                    .copy_text("journalctl --user -u proton-sync -f".to_string());
                app.toast("copied to clipboard");
            }
        });
    });
}

fn env_card(app: &mut App, ui: &mut egui::Ui) {
    widgets::card(ui, Some("Extra environment variables"), |ui| {
        ui.add(
            egui::TextEdit::multiline(&mut app.draft.env_text)
                .hint_text("One KEY=VALUE per line, e.g.\nHTTPS_PROXY=http://proxy:3128")
                .desired_rows(3)
                .desired_width(f32::INFINITY),
        );
    });
}

// ----------------------------------------------------- selective tree ----

fn kick_root_load(app: &mut App) {
    if app.draft.loading.insert(String::new()) {
        let remote = app.draft.remote.clone();
        let tx = app.tx.clone();
        std::thread::spawn(move || {
            let folders = crate::cli::fetch_remote_folders(&remote, "");
            let _ = tx.send(super::Back::SelFolders {
                parent: String::new(),
                folders,
            });
        });
    }
}

fn refresh_roots(app: &mut App) {
    app.draft.refresh_roots();
    kick_root_load(app);
}

/// One level of the picker, Qt's tree in egui rows: expand arrow, tri-state
/// glyph, label; states cascade exactly like the QTreeWidget did.
fn sel_tree(app: &mut App, ui: &mut egui::Ui, parent: &str) {
    let kids = app.draft.children.get(parent).cloned().unwrap_or_default();
    if kids.is_empty() {
        if app.draft.loading.contains(parent) {
            ui.label(egui::RichText::new("Loading...").weak());
        } else if parent.is_empty() {
            ui.label(egui::RichText::new("...").weak());
        }
        return;
    }
    for (_name, rel) in &kids {
        sel_row(app, ui, rel, &_name);
        if app.draft.expanded.contains(rel) {
            let rel2 = rel.clone();
            ui.indent(("sel", rel2.clone()), |ui| {
                if !app.draft.children.contains_key(&rel2)
                    && !app.draft.loading.contains(&rel2)
                {
                    // lazy load this level
                    app.draft.loading.insert(rel2.clone());
                    let remote = app.draft.remote.clone();
                    let r = rel2.clone();
                    let tx = app.tx.clone();
                    std::thread::spawn(move || {
                        let folders = crate::cli::fetch_remote_folders(&remote, &r);
                        let _ = tx.send(super::Back::SelFolders { parent: r, folders });
                    });
                }
                sel_tree(app, ui, &rel2);
            });
        }
    }
}

fn sel_row(app: &mut App, ui: &mut egui::Ui, rel: &str, label: &str) {
    ui.horizontal(|ui| {
        let loaded_empty = app
            .draft
            .children
            .get(rel)
            .is_some_and(|k| k.is_empty());
        if !loaded_empty {
            let expanded = app.draft.expanded.contains(rel);
            let arrow = if expanded { "\u{25be}" } else { "\u{25b8}" };
            if ui.add(egui::Label::new(arrow).sense(egui::Sense::click())).clicked() {
                if expanded {
                    app.draft.expanded.remove(rel);
                } else {
                    app.draft.expanded.insert(rel.to_string());
                }
            }
        } else {
            ui.add_space(18.0);
        }

        let state = app.draft.nodes.get(rel).copied().unwrap_or(false);
        let partial = app.draft.partial(rel);
        let glyph = if partial {
            "\u{25d0}"
        } else if state {
            "\u{2611}"
        } else {
            "\u{2610}"
        };
        let resp = ui.add(egui::Label::new(glyph).sense(egui::Sense::click()));
        if resp.clicked() {
            app.draft.toggle_node(rel);
        }
        ui.label(label);
    });
}
