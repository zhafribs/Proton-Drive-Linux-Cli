//! Activity: the friendly "Files moved" list and the raw technical log,
//! exactly the two segments the PyQt app offered.

use super::{widgets, App};
use crate::journal::Dir;
use eframe::egui;

#[derive(Default)]
pub struct ActivityState {
    pub seg: usize,  // 0 = files moved, 1 = technical log
    pub kind: usize, // 0 everything, 1 uploaded, 2 downloaded, 3 other
    pub query: String,
    pub log_query: String,
}

const KINDS: [&str; 4] = ["Everything", "Uploaded", "Downloaded", "Other"];

pub fn draw(app: &mut App, ui: &mut egui::Ui) {
    // Header row: title + segment switch (Qt's rounded segmented control).
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Activity").strong().size(24.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            egui::Frame::new()
                .fill(ui.visuals().widgets.noninteractive.bg_fill.gamma_multiply(0.5))
                .corner_radius(egui::CornerRadius::same(12))
                .inner_margin(egui::Margin::same(5))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let sel0 = app.act.seg == 0;
                        if ui.selectable_label(sel0, "Files moved").clicked() {
                            app.act.seg = 0;
                        }
                        let sel1 = app.act.seg == 1;
                        if ui.selectable_label(sel1, "Technical log").clicked() {
                            app.act.seg = 1;
                        }
                    });
                });
        });
    });
    ui.add_space(12.0);

    if app.act.seg == 0 {
        files_tab(app, ui);
    } else {
        log_tab(app, ui);
    }
}

fn files_tab(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut app.act.query)
                .hint_text("Search file names...")
                .desired_width(f32::INFINITY),
        );
        egui::ComboBox::from_id_salt("act-kind")
            .selected_text(KINDS[app.act.kind])
            .width(140.0)
            .show_ui(ui, |ui| {
                for (i, k) in KINDS.iter().enumerate() {
                    ui.selectable_value(&mut app.act.kind, i, *k);
                }
            });
    });
    ui.add_space(8.0);

    let query = app.act.query.trim().to_lowercase();
    let kind = app.act.kind;

    let mut shown = 0usize;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for row in app.changes.iter().rev() {
                let pass_kind = match kind {
                    1 => row.dir() == Dir::Up,
                    2 => row.dir() == Dir::Down,
                    3 => matches!(row.dir(), Dir::Other | Dir::Warn),
                    _ => true,
                };
                if !pass_kind {
                    continue;
                }
                if !query.is_empty() && !row.rel.to_lowercase().contains(&query) {
                    continue;
                }
                let full = row.rel.trim_end_matches('/');
                let name = full.rsplit('/').next().unwrap_or(full);
                let parent = {
                    let p = &full[..full.len() - name.len()];
                    p.trim_end_matches('/')
                };
                let when = crate::util::rel_time(row.ts);
                widgets::move_row(ui, row, if parent.is_empty() { "/" } else { parent }, &when);
                shown += 1;
                if shown >= 120 {
                    break;
                }
            }
        });

    ui.add_space(6.0);
    if shown == 0 {
        ui.label(
            egui::RichText::new(
                "Nothing has moved yet - change a file and it will appear here.",
            )
            .weak(),
        );
    } else {
        ui.label(egui::RichText::new(format!("{shown} items")).size(12.0).weak());
    }
}

fn log_tab(app: &mut App, ui: &mut egui::Ui) {
    let query = app.act.log_query.trim().to_lowercase();
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut app.act.log_query)
                .hint_text("Filter log lines...")
                .desired_width(f32::INFINITY),
        );
        if ui.button("Copy log").clicked() {
            let text: Vec<&str> = app
                .log_lines
                .iter()
                .map(String::as_str)
                .filter(|l| query.is_empty() || l.to_lowercase().contains(&query))
                .collect();
            ui.ctx().copy_text(text.join("\n"));
            app.toast("Log copied to clipboard.");
        }
    });
    ui.add_space(8.0);

    let lines: Vec<&str> = app
        .log_lines
        .iter()
        .map(String::as_str)
        .filter(|l| query.is_empty() || l.to_lowercase().contains(&query))
        .collect();
    let start = lines.len().saturating_sub(600);
    egui::ScrollArea::vertical()
        .stick_to_bottom(true)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for l in &lines[start..] {
                ui.label(egui::RichText::new(*l).monospace().size(12.0));
            }
        });
}

