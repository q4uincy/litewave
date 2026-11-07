//! Small hand-drawn widgets. Everything is painted with shapes (no icon fonts, no images),
//! which keeps memory use and font requirements low.

use eframe::egui::{
    self, pos2, vec2, Align2, Color32, FontId, Painter, Pos2, Rect, Response, Sense, Shape,
    Stroke, Ui,
};

#[derive(Clone, Copy, PartialEq)]
pub enum Icon {
    Play,
    Pause,
    Prev,
    Next,
}

fn tri(p: &Painter, c: Pos2, h: f32, right: bool, col: Color32) {
    let d = if right { 1.0 } else { -1.0 };
    let pts = vec![
        pos2(c.x - d * h * 0.4, c.y - h * 0.5),
        pos2(c.x - d * h * 0.4, c.y + h * 0.5),
        pos2(c.x + d * h * 0.6, c.y),
    ];
    p.add(Shape::convex_polygon(pts, col, Stroke::NONE));
}

/// Round transport button. With `bg` it is a filled circle (the main play button).
pub fn icon_button(ui: &mut Ui, icon: Icon, size: f32, fg: Color32, bg: Option<Color32>) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let hover = resp.hovered();
    let p = ui.painter();
    let c = rect.center();
    let fg = match bg {
        Some(_) => fg,
        None => {
            if hover {
                fg
            } else {
                fg.gamma_multiply(0.75)
            }
        }
    };
    if let Some(b) = bg {
        let r = if hover { size * 0.5 } else { size * 0.47 };
        p.circle_filled(c, r, b);
    }
    let h = size * 0.40;
    match icon {
        Icon::Play => tri(p, c + vec2(size * 0.04, 0.0), h, true, fg),
        Icon::Pause => {
            for dx in [-0.11_f32, 0.11] {
                p.rect_filled(
                    Rect::from_center_size(c + vec2(dx * size, 0.0), vec2(size * 0.12, h)),
                    1.5,
                    fg,
                );
            }
        }
        Icon::Next => {
            tri(p, c + vec2(-size * 0.07, 0.0), h * 0.85, true, fg);
            p.rect_filled(
                Rect::from_center_size(c + vec2(size * 0.16, 0.0), vec2(size * 0.06, h * 0.85)),
                1.0,
                fg,
            );
        }
        Icon::Prev => {
            tri(p, c + vec2(size * 0.07, 0.0), h * 0.85, false, fg);
            p.rect_filled(
                Rect::from_center_size(c + vec2(-size * 0.16, 0.0), vec2(size * 0.06, h * 0.85)),
                1.0,
                fg,
            );
        }
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Rounded toggle with a text label (shuffle / repeat).
pub fn pill(ui: &mut Ui, text: &str, on: bool, accent: Color32, width: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 26.0), Sense::click());
    let base = ui.visuals().text_color();
    let col = if on {
        accent
    } else if resp.hovered() {
        base
    } else {
        base.gamma_multiply(0.65)
    };
    if on {
        ui.painter().rect_filled(rect, 13.0, accent.gamma_multiply(0.18));
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 13.0, base.gamma_multiply(0.08));
    }
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(12.5),
        col,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Flat sidebar entry.
pub fn nav_button(ui: &mut Ui, label: &str, active: bool, accent: Color32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
    if active {
        ui.painter().rect_filled(rect, 8.0, accent.gamma_multiply(0.20));
    } else if resp.hovered() {
        ui.painter()
            .rect_filled(rect, 8.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    let col = if active { accent } else { ui.visuals().text_color() };
    ui.painter().text(
        pos2(rect.min.x + 14.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(14.0),
        col,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Thin progress/volume bar. Returns the response and the fraction under the pointer while
/// it is clicked or dragged (otherwise `frac` unchanged).
pub fn bar(
    ui: &mut Ui,
    width: f32,
    frac: f32,
    fill: Color32,
    track: Color32,
    enabled: bool,
) -> (Response, f32) {
    let sense = if enabled { Sense::click_and_drag() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 18.0), sense);
    let mut f = frac.clamp(0.0, 1.0);
    if enabled && (resp.dragged() || resp.clicked() || resp.drag_stopped()) {
        if let Some(pos) = ui.input(|i| i.pointer.latest_pos()) {
            f = ((pos.x - rect.left()) / rect.width().max(1.0)).clamp(0.0, 1.0);
        }
    }
    let active = enabled && (resp.hovered() || resp.dragged());
    let h = if active { 6.0 } else { 4.0 };
    let line = Rect::from_center_size(rect.center(), vec2(rect.width(), h));
    let p = ui.painter();
    p.rect_filled(line, h / 2.0, track);
    let filled = Rect::from_min_max(line.min, pos2(line.min.x + line.width() * f, line.max.y));
    p.rect_filled(filled, h / 2.0, if active { fill } else { fill.gamma_multiply(0.9) });
    if active {
        p.circle_filled(pos2(filled.max.x, rect.center().y), 6.0, Color32::WHITE);
    }
    (resp, f)
}

fn hsv(h: f32, s: f32, v: f32) -> Color32 {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);
    let (r, g, b) = match (i as i32).rem_euclid(6) {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

/// Stable placeholder color for a track's "cover" square.
pub fn tint_for(s: &str) -> Color32 {
    let mut h: u32 = 2166136261;
    for b in s.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    hsv((h % 360) as f32 / 360.0, 0.42, 0.50)
}

pub fn initial(s: &str) -> String {
    s.chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().collect::<String>())
        .unwrap_or_else(|| "#".to_string())
}
