//! The launcher paints its own chrome: a dark olive HUD with a faint grid,
//! drifting scanlines and amber corner brackets. Everything here is drawn with
//! egui's painter, so Linux and Windows look identical and no art assets ship.

use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Mesh, Painter, Pos2, Rect, Shape, Stroke,
    TextStyle,
};

pub const BG: Color32 = Color32::from_rgb(9, 11, 9);
pub const BG_DEEP: Color32 = Color32::from_rgb(6, 8, 6);
pub const PANEL: Color32 = Color32::from_rgb(14, 17, 13);
pub const PANEL_2: Color32 = Color32::from_rgb(19, 23, 17);
pub const LINE: Color32 = Color32::from_rgb(42, 50, 36);
pub const GRID: Color32 = Color32::from_rgba_premultiplied(74, 88, 58, 14);
pub const GRID_MAJOR: Color32 = Color32::from_rgba_premultiplied(92, 110, 72, 28);
pub const TEXT: Color32 = Color32::from_rgb(198, 206, 184);
pub const DIM: Color32 = Color32::from_rgb(110, 122, 94);
pub const FAINT: Color32 = Color32::from_rgb(68, 78, 60);
pub const AMBER: Color32 = Color32::from_rgb(224, 167, 60);
pub const AMBER_DIM: Color32 = Color32::from_rgb(126, 92, 32);
pub const GREEN: Color32 = Color32::from_rgb(143, 191, 106);
pub const RED: Color32 = Color32::from_rgb(198, 92, 64);

/// Monospace everywhere, square corners, no shadows: a field terminal, not a desktop app.
pub fn install(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 7.0);
        s.spacing.button_padding = egui::vec2(12.0, 7.0);
        s.spacing.interact_size = egui::vec2(44.0, 24.0);

        s.text_styles
            .insert(TextStyle::Small, FontId::new(10.0, FontFamily::Monospace));
        s.text_styles
            .insert(TextStyle::Body, FontId::new(12.5, FontFamily::Monospace));
        s.text_styles
            .insert(TextStyle::Button, FontId::new(11.5, FontFamily::Monospace));
        s.text_styles
            .insert(TextStyle::Heading, FontId::new(18.0, FontFamily::Monospace));
        s.text_styles.insert(
            TextStyle::Monospace,
            FontId::new(11.5, FontFamily::Monospace),
        );

        let v = &mut s.visuals;
        v.panel_fill = BG;
        v.window_fill = PANEL;
        v.extreme_bg_color = BG_DEEP;
        v.faint_bg_color = PANEL_2;
        v.override_text_color = Some(TEXT);
        v.window_stroke = Stroke::new(1.0, LINE);
        v.hyperlink_color = AMBER;
        v.selection.bg_fill = AMBER_DIM;
        v.selection.stroke = Stroke::new(1.0, AMBER);

        let w = &mut v.widgets;
        for widget in [
            &mut w.noninteractive,
            &mut w.inactive,
            &mut w.hovered,
            &mut w.active,
            &mut w.open,
        ] {
            widget.corner_radius = CornerRadius::ZERO;
        }
        w.noninteractive.bg_fill = PANEL;
        w.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
        w.noninteractive.fg_stroke = Stroke::new(1.0, DIM);

        w.inactive.bg_fill = PANEL_2;
        w.inactive.weak_bg_fill = PANEL_2;
        w.inactive.bg_stroke = Stroke::new(1.0, LINE);
        w.inactive.fg_stroke = Stroke::new(1.0, TEXT);

        w.hovered.bg_fill = Color32::from_rgb(34, 40, 29);
        w.hovered.weak_bg_fill = Color32::from_rgb(34, 40, 29);
        w.hovered.bg_stroke = Stroke::new(1.0, AMBER_DIM);
        w.hovered.fg_stroke = Stroke::new(1.0, AMBER);

        w.active.bg_fill = Color32::from_rgb(46, 42, 24);
        w.active.weak_bg_fill = Color32::from_rgb(46, 42, 24);
        w.active.bg_stroke = Stroke::new(1.0, AMBER);
        w.active.fg_stroke = Stroke::new(1.0, AMBER);

        w.open.bg_fill = PANEL_2;
        w.open.bg_stroke = Stroke::new(1.0, LINE);
        w.open.fg_stroke = Stroke::new(1.0, TEXT);
    });
}

/// Olive-to-black vertical gradient, a survey grid and slow scanlines.
pub fn backdrop(painter: &Painter, rect: Rect, time: f64) {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.left_top(), Color32::from_rgb(13, 16, 12));
    mesh.colored_vertex(rect.right_top(), Color32::from_rgb(13, 16, 12));
    mesh.colored_vertex(rect.left_bottom(), BG_DEEP);
    mesh.colored_vertex(rect.right_bottom(), BG_DEEP);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 2, 3);
    painter.add(Shape::mesh(mesh));

    let step = 30.0;
    let mut x = rect.left();
    let mut i = 0;
    while x <= rect.right() {
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            Stroke::new(1.0, if i % 4 == 0 { GRID_MAJOR } else { GRID }),
        );
        x += step;
        i += 1;
    }
    let mut y = rect.top();
    let mut j = 0;
    while y <= rect.bottom() {
        painter.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            Stroke::new(1.0, if j % 4 == 0 { GRID_MAJOR } else { GRID }),
        );
        y += step;
        j += 1;
    }

    // Scanlines drift downward a few pixels a second: enough to read as a live feed.
    let offset = (time * 9.0) as f32 % 4.0;
    let mut sy = rect.top() + offset;
    while sy < rect.bottom() {
        painter.hline(
            rect.x_range(),
            sy,
            Stroke::new(1.0, Color32::from_black_alpha(16)),
        );
        sy += 4.0;
    }
}

/// Four L-shaped corner marks.
pub fn brackets(painter: &Painter, rect: Rect, len: f32, stroke: Stroke) {
    let r = rect.shrink(0.5);
    let corners = [
        (r.left_top(), 1.0, 1.0),
        (r.right_top(), -1.0, 1.0),
        (r.left_bottom(), 1.0, -1.0),
        (r.right_bottom(), -1.0, -1.0),
    ];
    for (corner, dx, dy) in corners {
        painter.line_segment([corner, egui::pos2(corner.x + dx * len, corner.y)], stroke);
        painter.line_segment([corner, egui::pos2(corner.x, corner.y + dy * len)], stroke);
    }
}

pub fn dot(painter: &Painter, center: Pos2, radius: f32, color: Color32) {
    painter.circle_filled(center, radius, color);
}

/// Keeps the tail of a long path: `…/pz-10-years-later-client`.
pub fn short(s: &str, keep: usize) -> String {
    let count = s.chars().count();
    if count <= keep {
        return s.to_string();
    }
    let tail: String = s.chars().skip(count - keep).collect();
    format!("…{tail}")
}

pub fn bytes(n: u64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.2} ГБ", n as f64 / 1e9)
    } else if n >= 1_000_000 {
        format!("{:.1} МБ", n as f64 / 1e6)
    } else if n >= 1_000 {
        format!("{:.0} КБ", n as f64 / 1e3)
    } else {
        format!("{n} Б")
    }
}
