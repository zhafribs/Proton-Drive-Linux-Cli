//! Shared visual pieces: cards, toggles, buttons, the animated status orb,
//! avatars, and the activity/recent rows.

use crate::journal::{ChangeRow, Dir};
use crate::theme;
use eframe::egui;

/// Card container (title optional) - the PyQt `card()` helper, on the Proton
/// Drive web look: white surface + hairline border on the lavender page.
pub fn card(ui: &mut egui::Ui, title: Option<&str>, add: impl FnOnce(&mut egui::Ui)) {
    let fill = theme::surface(ui.visuals());
    let frame = egui::Frame::group(ui.style())
        .fill(fill)
        .corner_radius(egui::CornerRadius::same(12))
        .inner_margin(egui::Margin::symmetric(18, 16));
    frame.show(ui, |ui| {
        // Proton's cards always span the content column.
        ui.set_min_width(ui.available_width());
        if let Some(t) = title {
            ui.label(egui::RichText::new(t).strong().size(15.0));
            ui.add_space(8.0);
        }
        add(ui);
    });
}

/// Uppercase muted section label ("NEVER SYNC FILES MATCHING").
pub fn section_label(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text.to_uppercase()).size(11.0).weak());
    ui.add_space(4.0);
}

pub fn primary_button(ui: &mut egui::Ui, text: &str) -> bool {
    ui.add(
        egui::Button::new(egui::RichText::new(text).color(egui::Color32::WHITE))
            .fill(theme::ACCENT)
            .stroke(egui::Stroke::NONE)
            .corner_radius(theme::pill()),
    )
    .clicked()
}

pub fn secondary_button(ui: &mut egui::Ui, text: &str) -> bool {
    ui.add(egui::Button::new(text).corner_radius(theme::pill())).clicked()
}

pub fn danger_ghost_button(ui: &mut egui::Ui, text: &str) -> bool {
    ui.add(
        egui::Button::new(egui::RichText::new(text).color(theme::ERROR))
            .stroke(egui::Stroke::new(1.0, theme::ERROR.gamma_multiply(0.5)))
            .corner_radius(theme::pill()),
    )
    .clicked()
}

/// Toggle row: checkbox + optional hint line under the label.
pub fn toggle(ui: &mut egui::Ui, label: &str, value: &mut bool, hint: Option<&str>) -> bool {
    let mut changed = false;
    ui.vertical(|ui| {
        changed = ui.checkbox(value, label).changed();
        if let Some(h) = hint {
            ui.label(egui::RichText::new(h).size(12.0).weak()).on_hover_text(h);
        }
    });
    changed
}

/// Status orb: a modern status ring - a soft ambient halo, a Proton-navy
/// glass core, the status color as the ring (a comet sweep + pulsing dots
/// while syncing) and a state glyph in the middle.
pub fn status_orb(ui: &mut egui::Ui, status: &str) {
    let size = 104.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let c = rect.center();
    let color = theme::status_color(status);
    let painter = ui.painter();
    let r = size / 2.0 - 8.0;

    // Ambient halo: two ultra-soft discs so the ring glows instead of
    // sitting flat on the page.
    painter.circle_filled(c, r + 9.0, color.gamma_multiply(0.05));
    painter.circle_filled(c, r + 4.5, color.gamma_multiply(0.09));

    // Glass core: Proton navy with a lighter cap for a domed sheen.
    painter.circle_filled(c, r - 7.0, theme::NAVY);
    painter.circle_filled(
        c + egui::vec2(0.0, -r * 0.14),
        (r - 7.0) * 0.80,
        theme::NAVY.gamma_multiply(1.5),
    );

    // The ring itself: a calm track in the status color.
    painter.circle_stroke(
        c,
        r,
        egui::Stroke::new(6.0, color.gamma_multiply(0.22)),
    );

    if status == "syncing" {
        // Smooth motion: ~25 fps while the orb is on screen and animating.
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(40));
        let t = ui.ctx().input(|i| i.time) as f32;
        // Comet sweep riding the ring.
        let a0 = (t * 140.0f32).to_radians().rem_euclid(std::f32::consts::TAU);
        let a1 = a0 + std::f32::consts::FRAC_PI_2;
        let steps = 24;
        let pts: Vec<egui::Pos2> = (0..=steps)
            .map(|i| {
                let a = a0 + (i as f32 / steps as f32) * std::f32::consts::FRAC_PI_2;
                c + egui::vec2(a.cos(), a.sin()) * r
            })
            .collect();
        painter.add(egui::Shape::line(pts, egui::Stroke::new(6.0, color)));
        painter.circle_filled(
            c + egui::vec2(a1.cos(), a1.sin()) * r,
            3.2,
            color.gamma_multiply(1.6),
        );
        // Three pulsing dots in the core.
        for k in 0..3 {
            let ph = (t * 3.0 + k as f32 * std::f32::consts::TAU / 3.0)
                .rem_euclid(std::f32::consts::TAU);
            let a = 0.35 + 0.65 * ((ph.sin() + 1.0) / 2.0);
            let dx = (k as f32 - 1.0) * 11.0;
            painter.circle_filled(
                c + egui::vec2(dx, 0.0),
                3.4,
                egui::Color32::WHITE.gamma_multiply(a),
            );
        }
        return;
    }

    // Static glyph per state, painted white on the glass core.
    let white = egui::Color32::WHITE;
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
        "paused" => {
            for dx in [-5.5f32, 5.5] {
                let bar = egui::Rect::from_center_size(
                    c + egui::vec2(dx, 0.0),
                    egui::vec2(4.5, 17.0),
                );
                painter.rect_filled(bar, egui::CornerRadius::same(2), white);
            }
        }
        "error" => {
            let bar =
                egui::Rect::from_center_size(c + egui::vec2(0.0, -3.0), egui::vec2(4.5, 14.0));
            painter.rect_filled(bar, egui::CornerRadius::same(2), white);
            painter.circle_filled(c + egui::vec2(0.0, 8.5), 2.6, white);
        }
        _ => {
            // Unknown: a quiet white ring.
            painter.circle_stroke(c, 8.0, egui::Stroke::new(2.6, white));
        }
    }
}

/// Account avatar: initial of the e-mail (or "?" when signed out).
pub fn avatar(ui: &mut egui::Ui, email: Option<&str>) {
    let s = 44.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(s, s), egui::Sense::hover());
    let painter = ui.painter();
    painter.circle_filled(rect.center(), s / 2.0, theme::ACCENT.gamma_multiply(0.25));
    let ch = email
        .and_then(|e| e.chars().next())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "?".into());
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        ch,
        egui::FontId::proportional(20.0),
        theme::ACCENT,
    );
}

fn dir_color(dir: Dir) -> egui::Color32 {
    match dir {
        Dir::Up => theme::SUCCESS,
        Dir::Down => theme::INFO,
        Dir::Other => theme::MUTED,
        Dir::Warn => theme::WARNING,
    }
}

/// One activity/recent row: colored glyph, name, parent path, label, time.
pub fn move_row(ui: &mut egui::Ui, row: &ChangeRow, parent: &str, when: &str) {
    let color = dir_color(row.dir());
    ui.horizontal(|ui| {
        let g = ui.label(egui::RichText::new(row.glyph()).strong().size(15.0).color(color));
        g.on_hover_text(row.rel.clone());
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(row.name.clone()).strong().size(14.0));
                ui.label(egui::RichText::new(row.label()).size(12.0).weak());
            });
            if !parent.is_empty() {
                ui.label(egui::RichText::new(parent).size(12.0).weak());
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new(when).size(12.0).weak());
        });
    });
    ui.add_space(2.0);
}

/// Line/solid glyphs painted for the sidebar nav (Proton-style icons).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NavIcon {
    Home,
    Activity,
    Folder,
    Settings,
    Trash,
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
        NavIcon::Activity => {
            // Pulse line - the activity feed's heartbeat.
            painter.add(egui::Shape::line(
                vec![
                    p(0.06, 0.56),
                    p(0.30, 0.56),
                    p(0.42, 0.24),
                    p(0.56, 0.76),
                    p(0.66, 0.50),
                    p(0.94, 0.50),
                ],
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
        NavIcon::Settings => {
            // Gear: teeth fused to the spindle ring, so it reads as a gear
            // and not a sun.
            let ctr = p(0.5, 0.5);
            for i in 0..8 {
                let a = (i as f32) * std::f32::consts::TAU / 8.0;
                let (s, cs) = a.sin_cos();
                let dir = egui::vec2(cs, s);
                painter.add(egui::Shape::line(
                    vec![ctr + dir * (0.24 * r.width()), ctr + dir * (0.42 * r.width())],
                    stroke,
                ));
            }
            painter.circle_stroke(ctr, 0.26 * r.width(), stroke);
            painter.circle_stroke(ctr, 0.09 * r.width(), stroke);
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
            painter.add(egui::Shape::line(vec![p(0.44, 0.48), p(0.46, 0.76)], stroke));
            painter.add(egui::Shape::line(vec![p(0.56, 0.48), p(0.54, 0.76)], stroke));
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
/// Proton Drive sidebar; otherwise plain text with a tinted hover.
pub fn nav_row(ui: &mut egui::Ui, icon: NavIcon, label: &str, selected: bool) -> bool {
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 36.0), egui::Sense::click());
    let fill = if selected {
        theme::accent_soft(ui.visuals())
    } else if resp.hovered() {
        ui.visuals().widgets.hovered.bg_fill
    } else {
        egui::Color32::TRANSPARENT
    };
    ui.painter().rect(
        rect,
        egui::CornerRadius::same(9),
        fill,
        egui::Stroke::NONE,
        egui::StrokeKind::Inside,
    );
    let color = if selected {
        theme::accent(ui.visuals())
    } else {
        ui.visuals().weak_text_color()
    };
    let icon_rect = egui::Rect::from_center_size(
        egui::pos2(rect.left() + 22.0, rect.center().y),
        egui::vec2(17.0, 17.0),
    );
    nav_icon(&ui.painter(), icon_rect, icon, color);
    ui.painter().text(
        rect.left_center() + egui::vec2(42.0, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(14.5),
        color,
    );
    resp.clicked()
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
