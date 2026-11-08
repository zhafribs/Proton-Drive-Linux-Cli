//! Shared visual pieces: cards, toggles, buttons, the animated status orb,
//! avatars, and the activity/recent rows.

use crate::journal::{ChangeRow, Dir};
use crate::theme;
use eframe::egui;

/// Card container (title optional) - the PyQt `card()` helper, on the Proton
/// Drive web look: white surface + hairline border + soft shadow on the
/// lavender page.
pub fn card(ui: &mut egui::Ui, title: Option<&str>, add: impl FnOnce(&mut egui::Ui)) {
    let frame = theme::card_frame(ui);
    frame.show(ui, |ui| {
        // Proton's cards always span the content column.
        ui.set_min_width(ui.available_width());
        if let Some(t) = title {
            let heading = ui.label(egui::RichText::new(t).strong().size(15.0));
            heading.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::Heading, true, t)
            });
            ui.add_space(10.0);
        }
        add(ui);
    });
}

/// Uppercase muted section label ("NEVER SYNC FILES MATCHING").
pub fn section_label(ui: &mut egui::Ui, text: &str) {
    let resp = ui.label(egui::RichText::new(text.to_uppercase()).size(11.0).weak());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Heading, ui.is_enabled(), text));
    ui.add_space(4.0);
}
/// The 24 pt page title, exposed as a heading for screen readers.
pub fn page_title(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let resp = ui.label(egui::RichText::new(text).strong().size(24.0));
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Heading, ui.is_enabled(), text));
    resp
}
/// Keep keyboard focus visible inside a ScrollArea. egui 0.37 does not
/// scroll a focused widget into view by itself, so a Tab into an off-screen
/// control leaves the focus ring (and the control) invisible.
pub fn scroll_focused_into_view(ui: &egui::Ui) {
    if let Some(id) = ui.ctx().memory(|m| m.focused())
        && let Some(resp) = ui.ctx().read_response(id)
        && resp.gained_focus()
    {
        resp.scroll_to_me(Some(egui::Align::Center));
    }
}


/// One compact metric card (label, large value, context line). Shared by
/// Home and Photos so their summary rows look identical.
pub fn metric(ui: &mut egui::Ui, label: &str, value: &str, sub: &str) {
    card(ui, None, |ui| {
        ui.label(egui::RichText::new(label).size(11.5).weak());
        ui.add_space(2.0);
        // A long value ("415.7 MB") must stay on one line in the narrow
        // three-column row; the smaller size applies only in that case.
        let value_size = if ui.available_width() < 120.0 {
            18.5
        } else {
            23.0
        };
        ui.add(
            egui::Label::new(egui::RichText::new(value).strong().size(value_size))
                .truncate(),
        )
        .on_hover_text(value);
        ui.label(egui::RichText::new(sub).size(11.0).weak());
    });
}

/// A status pill: tinted track, colored dot, short label. Shared by Home and
/// Photos so a status reads the same everywhere.
pub fn status_pill(ui: &mut egui::Ui, key: &str, label: &str) {
    let color = theme::status_color(key);
    let text = theme::status_text_color(key, ui.visuals());
    egui::Frame::new()
        .fill(color.gamma_multiply(0.10))
        .corner_radius(theme::pill())
        .inner_margin(egui::Margin::symmetric(10, 4))
        .show(ui, |ui| {
            // An outer RTL header can flip a plain horizontal, so give the
            // pill's contents an exact LTR box. The label width comes from
            // the same font the label paints with.
            let text_w = ui
                .painter()
                .layout_no_wrap(
                    label.to_string(),
                    egui::FontId::proportional(11.5),
                    text,
                )
                .size()
                .x;
            let content_w = 7.0 + 5.0 + text_w + 2.0;
            ui.allocate_ui_with_layout(
                egui::vec2(content_w, 18.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 5.0;
                    let (rect, _) = ui.allocate_exact_size(
                        egui::vec2(7.0, 7.0),
                        egui::Sense::hover(),
                    );
                    ui.painter().circle_filled(rect.center(), 3.5, color);
                    let resp =
                        ui.label(egui::RichText::new(label).size(11.5).color(text));
                    resp.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::Status, true, label)
                    });
                    ui.ctx().accesskit_node_builder(resp.id, |n| {
                        n.set_live(egui::accesskit::Live::Polite)
                    });
                },
            );
        });
}

pub fn primary_button(ui: &mut egui::Ui, text: &str) -> bool {
    primary_button_response(ui, text).clicked()
}

/// The same primary button, returned as a response for focus management.
pub fn primary_button_response(ui: &mut egui::Ui, text: &str) -> egui::Response {
    primary_button_hint_response(ui, text, "")
}

pub fn primary_button_hint(ui: &mut egui::Ui, text: &str, hint: &str) -> bool {
    primary_button_hint_response(ui, text, hint).clicked()
}

pub fn primary_button_hint_response(
    ui: &mut egui::Ui,
    text: &str,
    hint: &str,
) -> egui::Response {
    let response = ui.add(
        egui::Button::new(egui::RichText::new(text).color(egui::Color32::WHITE))
            .fill(theme::ACCENT)
            .stroke(egui::Stroke::NONE)
            .corner_radius(theme::pill()),
    );
    if !hint.is_empty() {
        response.clone().on_hover_text(hint);
    }
    // Flat violet, not the old cyan cap: the overlay dropped white-on-cyan
    // contrast to ~3.8:1. The label is painted by egui before any overlay,
    // so a gradient would also tint the text itself.
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect,
            theme::pill(),
            egui::Stroke::new(1.5, egui::Color32::WHITE),
            egui::StrokeKind::Inside,
        );
    }
    response
}

/// Full-width primary CTA, for popups and empty states. Painted rather than
/// stretched from a `Button` so the label stays centered.
pub fn primary_button_full(ui: &mut egui::Ui, text: &str) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 34.0), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), text));
    let radius = theme::pill();
    let fill = if response.hovered() {
        theme::ACCENT.gamma_multiply(0.92)
    } else {
        theme::ACCENT
    };
    ui.painter().rect_filled(rect, radius, fill);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(13.0),
        egui::Color32::WHITE,
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            radius,
            egui::Stroke::new(1.5, egui::Color32::from_white_alpha(220)),
            egui::StrokeKind::Inside,
        );
    }
    response.clicked()
        || (response.has_focus()
            && ui.input(|i| {
                i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Space)
            }))
}

/// Round identity avatar: accent disc, cyan cap and the initial.
pub fn avatar(ui: &egui::Ui, center: egui::Pos2, radius: f32, initial: &str) {
    let painter = ui.painter();
    painter.circle_filled(center, radius, theme::ACCENT);
    painter.text(
        center,
        egui::Align2::CENTER_CENTER,
        initial,
        egui::FontId::proportional(radius * 0.93),
        egui::Color32::WHITE,
    );
}

pub fn secondary_button_hint(ui: &mut egui::Ui, text: &str, hint: &str) -> bool {
    let response = ui.add(egui::Button::new(text).corner_radius(theme::pill()));
    if !hint.is_empty() {
        response.clone().on_hover_text(hint);
    }
    response.clicked()
}

pub fn danger_ghost_button(ui: &mut egui::Ui, text: &str) -> bool {
    let red = theme::danger_text(ui.visuals());
    let response = ui.add(
        egui::Button::new(egui::RichText::new(text).color(red))
            // Keep the surface behind the red text on hover/focus too; the
            // accent active fill drops the text below AA.
            .fill(theme::glass_fill(ui.visuals()))
            .stroke(egui::Stroke::new(1.0, red.gamma_multiply(0.5)))
            .corner_radius(theme::pill()),
    );
    // The explicit fill/stroke also override egui's hover and focus styling,
    // so draw those cues ourselves.
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        ui.painter().rect_stroke(
            response.rect.expand(2.0),
            theme::pill(),
            egui::Stroke::new(1.0, red.gamma_multiply(0.8)),
            egui::StrokeKind::Inside,
        );
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect.expand(2.0),
            theme::pill(),
            egui::Stroke::new(1.5, theme::accent(ui.visuals())),
            egui::StrokeKind::Inside,
        );
    }
    response.clicked()
}

/// Busy indicator. With motion off it becomes a static ring, so the row still
/// says "working" without a continuous repaint.
pub fn busy_spinner(ui: &mut egui::Ui, animate: bool) {
    if animate {
        ui.spinner();
    } else {
        let (rect, resp) =
            ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
        resp.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::Role::ProgressIndicator,
                ui.is_enabled(),
                "Working",
            )
        });
        ui.painter().circle_stroke(
            rect.center(),
            7.0,
            egui::Stroke::new(2.0, ui.visuals().weak_text_color()),
        );
    }
}

/// Toggle row: checkbox + optional hover explanation.
pub fn toggle(ui: &mut egui::Ui, label: &str, value: &mut bool, hint: Option<&str>) -> bool {
    let response = ui.checkbox(value, label);
    if let Some(h) = hint {
        response.clone().on_hover_text(h);
    }
    response.changed()
}

/// Status disc: a soft tone of the state colour, one ring, and a glyph.
pub fn status_orb(ui: &mut egui::Ui, status: &str, animate: bool) {
    let size = 72.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let c = rect.center();
    let color = theme::status_color(status);
    let ink = theme::status_text_color(status, ui.visuals());
    let painter = ui.painter();
    let r = size / 2.0 - 5.0;

    // Flat status disc: a soft tone of the state colour with a single ring.
    // No glow or glass; the glyph carries the meaning.
    painter.circle_filled(c, r, color.gamma_multiply(if ui.visuals().dark_mode {
        0.18
    } else {
        0.10
    }));
    painter.circle_stroke(
        c,
        r,
        egui::Stroke::new(1.0, color.gamma_multiply(0.35)),
    );

    let t = if animate {
        ui.ctx().input(|i| i.time) as f32
    } else {
        0.0
    };
    if status == "syncing" {
        if animate {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(40));
        }
        // Restrained sweep: a single arc on the ring, no pulsing dots.
        let a0 = (t * 140.0f32)
            .to_radians()
            .rem_euclid(std::f32::consts::TAU);
        let steps = 24;
        let pts: Vec<egui::Pos2> = (0..=steps)
            .map(|i| {
                let a = a0 + (i as f32 / steps as f32) * std::f32::consts::FRAC_PI_2;
                c + egui::vec2(a.cos(), a.sin()) * r
            })
            .collect();
        painter.add(egui::Shape::line(pts, egui::Stroke::new(3.5, ink)));
        return;
    }

    if status == "signing_in" {
        if animate {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(60));
        }
        // A small key: the sign-in terminal is open.
        let pulse = if animate {
            0.55 + 0.45 * ((t * 3.0).sin() + 1.0) / 2.0
        } else {
            1.0
        };
        let w = egui::Stroke::new(2.6, ink.gamma_multiply(pulse));
        painter.circle_stroke(c + egui::vec2(-4.0, 0.0), 4.0, w);
        painter.add(egui::Shape::line(
            vec![c + egui::vec2(0.0, 0.0), c + egui::vec2(8.0, 0.0)],
            w,
        ));
        painter.add(egui::Shape::line(
            vec![c + egui::vec2(5.0, 0.0), c + egui::vec2(5.0, 3.0)],
            w,
        ));
        return;
    }

    // Static glyph per state, painted in the contrasting status ink.
    let white = ink;
    match status {
        "synced" => {
            let w = egui::Stroke::new(3.8, white);
            painter.add(egui::Shape::line(
                vec![c + egui::vec2(-9.0, 0.5), c + egui::vec2(-3.0, 6.5)],
                w,
            ));
            painter.add(egui::Shape::line(
                vec![c + egui::vec2(-3.0, 6.5), c + egui::vec2(9.5, -6.5)],
                w,
            ));
        }
        "paused" | "deletes_held" => {
            for dx in [-5.5f32, 5.5] {
                let bar =
                    egui::Rect::from_center_size(c + egui::vec2(dx, 0.0), egui::vec2(4.5, 17.0));
                painter.rect_filled(bar, egui::CornerRadius::same(2), white);
            }
        }
        "error" => {
            let bar =
                egui::Rect::from_center_size(c + egui::vec2(0.0, -3.0), egui::vec2(4.5, 14.0));
            painter.rect_filled(bar, egui::CornerRadius::same(2), white);
            painter.circle_filled(c + egui::vec2(0.0, 8.5), 2.6, white);
        }
        "signed_out" => {
            // Sign-in arrow entering a door.
            let w = egui::Stroke::new(3.2, white);
            painter.add(egui::Shape::line(
                vec![
                    c + egui::vec2(4.0, -10.0),
                    c + egui::vec2(10.0, -10.0),
                    c + egui::vec2(10.0, 10.0),
                    c + egui::vec2(4.0, 10.0),
                ],
                w,
            ));
            painter.add(egui::Shape::line(
                vec![c + egui::vec2(-11.0, 0.0), c + egui::vec2(6.0, 0.0)],
                w,
            ));
            painter.add(egui::Shape::line(
                vec![
                    c + egui::vec2(1.0, -5.0),
                    c + egui::vec2(6.0, 0.0),
                    c + egui::vec2(1.0, 5.0),
                ],
                w,
            ));
        }
        "needs_login" => {
            // Padlock: the session is locked out, not a fresh install.
            let w = egui::Stroke::new(3.0, white);
            // Shackle: a semicircle whose ends meet the body.
            let arc_c = c + egui::vec2(0.0, -1.0);
            let mut pts = Vec::new();
            for i in 0..=16 {
                let a = std::f32::consts::PI + (i as f32 / 16.0) * std::f32::consts::PI;
                pts.push(arc_c + egui::vec2(a.cos(), a.sin()) * 6.0);
            }
            painter.add(egui::Shape::line(pts, w));
            let body =
                egui::Rect::from_center_size(c + egui::vec2(0.0, 5.0), egui::vec2(18.0, 13.0));
            painter.rect_filled(body, egui::CornerRadius::same(3), white);
            painter.circle_filled(c + egui::vec2(0.0, 4.0), 2.4, theme::glass_fill(ui.visuals()));
        }
        "engine_stopped" => {
            // Power symbol: the engine is not running.
            let w = egui::Stroke::new(3.4, white);
            let start = -std::f32::consts::FRAC_PI_2 + 0.55;
            let sweep = std::f32::consts::TAU - 1.1;
            let pts: Vec<egui::Pos2> = (0..=32)
                .map(|i| {
                    let a = start + (i as f32 / 32.0) * sweep;
                    c + egui::vec2(a.cos(), a.sin()) * 9.5
                })
                .collect();
            painter.add(egui::Shape::line(pts, w));
            painter.add(egui::Shape::line(
                vec![c + egui::vec2(0.0, -11.0), c + egui::vec2(0.0, 0.5)],
                w,
            ));
        }
        "starting" => {
            // Clock: waiting for the engine's first report.
            let w = egui::Stroke::new(3.0, white);
            painter.circle_stroke(c, 9.5, w);
            painter.add(egui::Shape::line(
                vec![c, c + egui::vec2(0.0, -6.0)],
                egui::Stroke::new(2.6, white),
            ));
            painter.add(egui::Shape::line(
                vec![c, c + egui::vec2(4.5, 2.5)],
                egui::Stroke::new(2.6, white),
            ));
        }
        _ => {
            // Unknown: a quiet ring.
            painter.circle_stroke(c, 8.0, egui::Stroke::new(2.6, white));
        }
    }
}

fn dir_color(dir: Dir, visuals: &egui::Visuals) -> egui::Color32 {
    match dir {
        Dir::Up => theme::success_text(visuals),
        Dir::Down => theme::info_text(visuals),
        Dir::Other => theme::muted(visuals),
        Dir::Warn => theme::warning_text(visuals),
    }
}

/// Compact Home activity row: icon + file name + time. The action/path line
/// belongs to the full Activity page, not the Home preview.
pub fn move_row_compact(ui: &mut egui::Ui, row: &ChangeRow, when: &str) {
    let color = dir_color(row.dir(), ui.visuals());
    let height = 32.0;
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );
    let a11y = format!("{}: {}, {}", row.label(), row.name, when);
    resp.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::ListItem, ui.is_enabled(), a11y.as_str())
    });
    if resp.hovered() {
        ui.painter().rect_filled(
            rect,
            egui::CornerRadius::same(8),
            ui.visuals().widgets.hovered.weak_bg_fill,
        );
    }

    // Fixed icon box, painted at the row's exact vertical centre. This is the
    // alignment the text glyph lacked when the row had two text lines.
    let icon = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 10.0, rect.center().y),
        egui::vec2(18.0, 18.0),
    );
    move_icon(ui.painter(), icon, row.dir(), color);

    let time_w = 64.0;
    let name_rect = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 28.0, rect.top()),
        egui::pos2(rect.right() - time_w - 8.0, rect.bottom()),
    );
    let mut name_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(name_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    let name = name_ui.add(
        egui::Label::new(egui::RichText::new(row.name.clone()).strong().size(13.5)).truncate(),
    );
    name.on_hover_text(row.rel.clone());

    ui.painter().text(
        egui::pos2(rect.right() - 8.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        when,
        egui::FontId::proportional(12.0),
        ui.visuals().weak_text_color(),
    );
    resp.on_hover_text(row.rel.clone());
}

/// One activity row: colored glyph, name, parent path, action label, time.
pub fn move_row(ui: &mut egui::Ui, row: &ChangeRow, parent: &str, when: &str) {
    let color = dir_color(row.dir(), ui.visuals());
    let height = 40.0;
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );
    let a11y = format!("{}: {}, {}, {}", row.label(), row.name, parent, when);
    resp.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::ListItem, ui.is_enabled(), a11y.as_str())
    });
    if resp.hovered() {
        ui.painter().rect_filled(
            rect,
            egui::CornerRadius::same(8),
            ui.visuals().widgets.hovered.weak_bg_fill,
        );
    }

    // The icon sits on the title line, not halfway between title and path.
    let icon = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 10.0, rect.top() + 10.0),
        egui::vec2(18.0, 18.0),
    );
    move_icon(ui.painter(), icon, row.dir(), color);

    let time_w = 64.0;
    let text_rect = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 28.0, rect.top()),
        egui::pos2(rect.right() - time_w - 8.0, rect.bottom()),
    );
    let mut text_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(text_rect)
            .layout(egui::Layout::top_down(egui::Align::LEFT)),
    );
    text_ui.set_max_width(text_rect.width());
    let name = text_ui.add(
        egui::Label::new(egui::RichText::new(row.name.clone()).strong().size(14.0)).truncate(),
    );
    name.on_hover_text(row.rel.clone());
    let mut meta = row.label().to_string();
    if !parent.is_empty() {
        meta.push_str(" \u{b7} ");
        meta.push_str(parent);
    }
    text_ui.add(egui::Label::new(egui::RichText::new(meta).size(12.0).weak()).truncate());

    ui.painter().text(
        egui::pos2(rect.right() - 8.0, rect.top() + 10.0),
        egui::Align2::RIGHT_CENTER,
        when,
        egui::FontId::proportional(12.0),
        ui.visuals().weak_text_color(),
    );
    resp.on_hover_text(row.rel.clone());
    ui.add_space(2.0);
}

/// Draw the direction mark as geometry so it has no font baseline offset.
fn move_icon(painter: &egui::Painter, r: egui::Rect, dir: Dir, color: egui::Color32) {
    let stroke = egui::Stroke::new(1.8, color);
    let c = r.center();
    match dir {
        Dir::Up => {
            painter.add(egui::Shape::line(
                vec![c + egui::vec2(0.0, 5.0), c + egui::vec2(0.0, -5.0)],
                stroke,
            ));
            painter.add(egui::Shape::line(
                vec![
                    c + egui::vec2(-4.5, 0.5),
                    c + egui::vec2(0.0, -5.0),
                    c + egui::vec2(4.5, 0.5),
                ],
                stroke,
            ));
        }
        Dir::Down => {
            painter.add(egui::Shape::line(
                vec![c + egui::vec2(0.0, -5.0), c + egui::vec2(0.0, 5.0)],
                stroke,
            ));
            painter.add(egui::Shape::line(
                vec![
                    c + egui::vec2(-4.5, -0.5),
                    c + egui::vec2(0.0, 5.0),
                    c + egui::vec2(4.5, -0.5),
                ],
                stroke,
            ));
        }
        Dir::Other => {
            painter.add(egui::Shape::line(
                vec![c + egui::vec2(-5.0, -5.0), c + egui::vec2(5.0, 5.0)],
                stroke,
            ));
            painter.add(egui::Shape::line(
                vec![c + egui::vec2(-5.0, 5.0), c + egui::vec2(5.0, -5.0)],
                stroke,
            ));
        }
        Dir::Warn => {
            painter.add(egui::Shape::line(
                vec![c + egui::vec2(0.0, -5.0), c + egui::vec2(0.0, 2.0)],
                stroke,
            ));
            painter.circle_filled(c + egui::vec2(0.0, 5.0), 1.4, color);
        }
    }
}

/// Line/solid glyphs painted for the sidebar nav (Proton-style icons).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NavIcon {
    Home,
    Folder,
    Photos,
    Activity,
    Settings,
    Trash,
    Recovery,
    About,
}

/// Paint one nav glyph into `r` - hand-drawn from lines and rounded rects so
/// it matches the stroke weight of Proton's line icons without shipping a
/// font or an asset.
pub fn nav_icon(painter: &egui::Painter, r: egui::Rect, kind: NavIcon, c: egui::Color32) {
    let stroke = egui::Stroke::new(1.6, c);
    let p = |x: f32, y: f32| egui::pos2(r.left() + x * r.width(), r.top() + y * r.height());
    match kind {
        NavIcon::Home => {
            // Line house: roof, walls, door.
            painter.add(egui::Shape::line(
                vec![p(0.08, 0.46), p(0.50, 0.10), p(0.92, 0.46)],
                stroke,
            ));
            painter.add(egui::Shape::line(
                vec![p(0.20, 0.42), p(0.20, 0.88), p(0.80, 0.88), p(0.80, 0.42)],
                stroke,
            ));
            painter.add(egui::Shape::line(
                vec![p(0.41, 0.88), p(0.41, 0.64), p(0.59, 0.64), p(0.59, 0.88)],
                stroke,
            ));
        }
        NavIcon::Folder => {
            // Filled silhouette: tab + body (Proton's solid folder glyph).
            painter.rect_filled(
                egui::Rect::from_min_max(p(0.10, 0.22), p(0.48, 0.40)),
                egui::CornerRadius::same(3),
                c,
            );
            painter.rect_filled(
                egui::Rect::from_min_max(p(0.10, 0.34), p(0.90, 0.84)),
                egui::CornerRadius::same(4),
                c,
            );
        }
        NavIcon::Photos => {
            // Photo frame: rounded canvas, a sun and a horizon line.
            painter.rect_stroke(
                egui::Rect::from_min_max(p(0.09, 0.19), p(0.91, 0.81)),
                egui::CornerRadius::same(3),
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.circle_stroke(p(0.34, 0.38), 0.08 * r.width(), stroke);
            painter.add(egui::Shape::line(
                vec![
                    p(0.16, 0.72),
                    p(0.42, 0.47),
                    p(0.60, 0.66),
                    p(0.73, 0.55),
                    p(0.85, 0.68),
                ],
                stroke,
            ));
        }
        NavIcon::Settings => {
            // Gear: teeth fused to the spindle ring, so it reads as a gear
            // and not a sun.
            let ctr = p(0.5, 0.5);
            for i in 0..8 {
                let a = (i as f32) * std::f32::consts::TAU / 8.0;
                let (s, cs) = a.sin_cos();
                let dir = egui::vec2(cs, s);
                painter.add(egui::Shape::line(
                    vec![
                        ctr + dir * (0.24 * r.width()),
                        ctr + dir * (0.42 * r.width()),
                    ],
                    stroke,
                ));
            }
            painter.circle_stroke(ctr, 0.26 * r.width(), stroke);
            painter.circle_stroke(ctr, 0.09 * r.width(), stroke);
        }
        NavIcon::Activity => {
            // Heartbeat / timeline pulse.
            painter.add(egui::Shape::line(
                vec![
                    p(0.08, 0.52),
                    p(0.30, 0.52),
                    p(0.40, 0.28),
                    p(0.56, 0.76),
                    p(0.66, 0.52),
                    p(0.92, 0.52),
                ],
                stroke,
            ));
        }
        NavIcon::Trash => {
            // Lid + handle, then the tapered bin with two ribs.
            painter.add(egui::Shape::line(
                vec![p(0.10, 0.30), p(0.90, 0.30)],
                stroke,
            ));
            painter.add(egui::Shape::line(
                vec![p(0.38, 0.14), p(0.62, 0.14)],
                stroke,
            ));
            painter.add(egui::Shape::line(
                vec![p(0.22, 0.38), p(0.27, 0.86), p(0.73, 0.86), p(0.78, 0.38)],
                stroke,
            ));
            painter.add(egui::Shape::line(
                vec![p(0.44, 0.48), p(0.46, 0.76)],
                stroke,
            ));
            painter.add(egui::Shape::line(
                vec![p(0.56, 0.48), p(0.54, 0.76)],
                stroke,
            ));
        }
        NavIcon::Recovery => {
            // Circular restore arrow.
            painter.circle_stroke(p(0.5, 0.5), 0.36 * r.width(), stroke);
            painter.add(egui::Shape::line(
                vec![p(0.70, 0.16), p(0.90, 0.22), p(0.82, 0.42)],
                stroke,
            ));
        }
        NavIcon::About => {
            // Info bubble: ring with an "i".
            painter.circle_stroke(p(0.5, 0.5), 0.42 * r.width(), stroke);
            painter.circle_filled(p(0.5, 0.32), 0.05 * r.width(), c);
            painter.add(egui::Shape::line(vec![p(0.5, 0.46), p(0.5, 0.72)], stroke));
        }
    }
}

/// Full-width nav row painted directly (egui 0.37 dropped the widget form of
/// `SelectableLabel`): soft-purple active pill + purple icon/text, like the
/// Proton Drive sidebar; otherwise plain text with a tinted hover. The active
/// row also draws a small accent bar on its left edge - the "you are here"
/// cue that holds even in monochrome.
pub fn nav_row(ui: &mut egui::Ui, icon: NavIcon, label: &str, selected: bool) -> bool {
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 38.0), egui::Sense::click());
    let fill = if selected {
        theme::accent_soft(ui.visuals())
    } else if resp.hovered() {
        ui.visuals().widgets.hovered.bg_fill
    } else {
        egui::Color32::TRANSPARENT
    };
    ui.painter().rect(
        rect,
        egui::CornerRadius::same(12),
        fill,
        egui::Stroke::NONE,
        egui::StrokeKind::Inside,
    );
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    resp.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::ListItem, ui.is_enabled(), label)
    });
    ui.ctx()
        .accesskit_node_builder(resp.id, |n| n.set_selected(selected));
    if resp.gained_focus() {
        resp.scroll_to_me(Some(egui::Align::Center));
    }
    if resp.has_focus() {
        ui.painter().rect_stroke(
            rect,
            egui::CornerRadius::same(12),
            egui::Stroke::new(1.5, theme::accent(ui.visuals())),
            egui::StrokeKind::Inside,
        );
    }
    if selected {
        let bar = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 1.0, rect.top() + 9.0),
            egui::vec2(3.0, rect.height() - 18.0),
        );
        ui.painter().rect_filled(
            bar,
            egui::CornerRadius::same(2),
            theme::accent(ui.visuals()),
        );
        ui.painter().rect_stroke(
            rect,
            egui::CornerRadius::same(12),
            egui::Stroke::new(1.0, theme::accent(ui.visuals()).gamma_multiply(0.55)),
            egui::StrokeKind::Inside,
        );
    }
    let color = if selected {
        theme::accent_text(ui.visuals())
    } else {
        ui.visuals().weak_text_color()
    };
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 22.0, rect.center().y),
        egui::vec2(17.0, 17.0),
    );
    nav_icon(ui.painter(), icon_rect, icon, color);
    ui.painter().text(
        rect.left_center() + egui::vec2(42.0, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(14.5),
        color,
    );
    resp.clicked()
        || (resp.has_focus()
            && ui.input(|i| {
                i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Space)
            }))
}

/// Toolbar glyphs for the files view (refresh ring + the two arrows),
/// painted in the same line style as the sidebar nav icons.
pub enum TopIcon {
    Refresh,
    ArrowLeft,
    ArrowRight,
}

pub fn top_icon(painter: &egui::Painter, r: egui::Rect, kind: TopIcon, c: egui::Color32) {
    let stroke = egui::Stroke::new(1.8, c);
    let p = |x: f32, y: f32| egui::pos2(r.left() + x * r.width(), r.top() + y * r.height());
    match kind {
        TopIcon::ArrowLeft => {
            painter.add(egui::Shape::line(vec![p(0.30, 0.5), p(0.70, 0.5)], stroke));
            painter.add(egui::Shape::line(
                vec![p(0.46, 0.33), p(0.29, 0.5), p(0.46, 0.67)],
                stroke,
            ));
        }
        TopIcon::ArrowRight => {
            painter.add(egui::Shape::line(vec![p(0.30, 0.5), p(0.70, 0.5)], stroke));
            painter.add(egui::Shape::line(
                vec![p(0.54, 0.33), p(0.71, 0.5), p(0.54, 0.67)],
                stroke,
            ));
        }
        TopIcon::Refresh => {
            // ~300° ring with a tangential arrowhead where the ring ends,
            // so it reads as "fetch again" like every sync client's icon.
            let ctr = p(0.5, 0.5);
            let rad = 0.21 * r.width().min(r.height());
            let a0 = (-55.0f32).to_radians();
            let span = 300.0f32.to_radians();
            let steps = 26;
            let pts: Vec<egui::Pos2> = (0..=steps)
                .map(|i| {
                    let a = a0 + span * (i as f32 / steps as f32);
                    ctr + egui::vec2(a.cos(), a.sin()) * rad
                })
                .collect();
            painter.add(egui::Shape::line(pts, stroke));
            let a1 = a0 + span;
            let tip = ctr + egui::vec2(a1.cos(), a1.sin()) * rad;
            let tang = egui::vec2(-a1.sin(), a1.cos()); // along the ring's travel
            let out = egui::vec2(a1.cos(), a1.sin());
            painter.add(egui::Shape::convex_polygon(
                vec![
                    tip + tang * 4.5,
                    tip - tang * 2.0 + out * 4.0,
                    tip - tang * 2.0 - out * 4.0,
                ],
                c,
                egui::Stroke::NONE,
            ));
        }
    }
}

/// Elide long paths for narrow rows.
pub fn elide(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}\u{2026}")
    }
}

/// Keep both ends of a path: the leaf matters more than the middle.
pub fn elide_middle(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(1);
    let head = keep / 2;
    let tail = keep - head;
    let start: String = s.chars().take(head).collect();
    let end: String = s.chars().skip(n - tail).collect();
    format!("{start}\u{2026}{end}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::journal::{ChangeKind, ChangeRow};

    #[test]
    fn middle_elision_keeps_the_leaf() {
        let path = "/home/someone/a/very/long/ProtonDrive";
        let out = elide_middle(path, 24);
        assert_eq!(out.chars().count(), 24);
        assert!(out.starts_with("/home"));
        assert!(out.ends_with("ProtonDrive"));
        assert!(out.contains('\u{2026}'));
        assert_eq!(elide_middle("short", 10), "short");
    }

    /// Golden-layout gate: at a narrow card width the activity row must not
    /// overflow its container (the timestamp column absorbs the slack).
    #[test]
    fn move_row_stays_inside_a_narrow_card() {
        let ctx = egui::Context::default();
        let mut width = 0.0;
        for _ in 0..2 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::pos2(0.0, 0.0),
                    egui::vec2(320.0, 400.0),
                )),
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.set_max_width(300.0);
                    let row = ChangeRow {
                        kind: ChangeKind::Downloaded,
                        name: "A very long file name that would overflow.txt".into(),
                        rel: "Docs/A very long file name that would overflow.txt".into(),
                        ts: None,
                    };
                    move_row(ui, &row, "Docs", "11 h ago");
                    width = ui.min_rect().width();
                });
            });
            out.textures_delta.clear();
        }
        // The hover track extends 4px beyond each side; anything within the
        // card plus that padding counts as fitting.
        assert!(width <= 308.0, "row layout width {width}");
    }
}
