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

#[derive(Clone, Copy, PartialEq)]
pub enum ModeIcon {
    Shuffle,
    Repeat,
    Lyrics,
}

/// Drawn shuffle / repeat button. `on` tints it with the accent and adds the small dot
/// underneath; `smart` adds a "+" (smart shuffle) and `one` a "1" (repeat one).
pub fn mode_button(
    ui: &mut Ui,
    icon: ModeIcon,
    on: bool,
    accent: Color32,
    smart: bool,
    one: bool,
) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::click());
    let base = ui.visuals().text_color();
    let col = if on {
        accent
    } else if resp.hovered() {
        base
    } else {
        base.gamma_multiply(0.65)
    };
    let p = ui.painter();
    let c = rect.center() - vec2(0.0, 1.0);
    let st = Stroke::new(1.8, col);
    let head = |p: &Painter, x: f32, y: f32, right: bool| {
        let d = if right { 1.0 } else { -1.0 };
        p.add(Shape::convex_polygon(
            vec![
                pos2(x - d * 1.0, y - 3.5),
                pos2(x - d * 1.0, y + 3.5),
                pos2(x + d * 4.5, y),
            ],
            col,
            Stroke::NONE,
        ));
    };
    match icon {
        ModeIcon::Shuffle => {
            p.line_segment([pos2(c.x - 8.0, c.y - 5.0), pos2(c.x + 6.0, c.y + 5.0)], st);
            p.line_segment([pos2(c.x - 8.0, c.y + 5.0), pos2(c.x + 6.0, c.y - 5.0)], st);
            head(p, c.x + 6.0, c.y - 5.0, true);
            head(p, c.x + 6.0, c.y + 5.0, true);
            if smart {
                p.text(
                    pos2(c.x + 10.0, c.y - 10.0),
                    Align2::CENTER_CENTER,
                    "+",
                    FontId::proportional(12.0),
                    col,
                );
            }
        }
        ModeIcon::Lyrics => {
            // Microphone (Spotify's lyrics icon): capsule, cradle, stem and base.
            p.rect_stroke(Rect::from_center_size(pos2(c.x, c.y - 3.5), vec2(6.0, 10.0)), 3.0, st);
            let cradle = [
                pos2(c.x - 6.5, c.y - 1.5),
                pos2(c.x - 5.5, c.y + 3.0),
                pos2(c.x, c.y + 5.5),
                pos2(c.x + 5.5, c.y + 3.0),
                pos2(c.x + 6.5, c.y - 1.5),
            ];
            for w in cradle.windows(2) {
                p.line_segment([w[0], w[1]], st);
            }
            p.line_segment([pos2(c.x, c.y + 5.5), pos2(c.x, c.y + 8.5)], st);
            p.line_segment([pos2(c.x - 3.5, c.y + 8.5), pos2(c.x + 3.5, c.y + 8.5)], st);
        }
        ModeIcon::Repeat => {
            p.rect_stroke(Rect::from_center_size(c, vec2(18.0, 12.0)), 4.0, st);
            head(p, c.x + 2.0, c.y - 6.0, true);
            head(p, c.x - 2.0, c.y + 6.0, false);
            if one {
                p.text(c, Align2::CENTER_CENTER, "1", FontId::proportional(9.0), col);
            }
        }
    }
    if on {
        p.circle_filled(pos2(rect.center().x, rect.max.y - 3.0), 1.8, col);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Flat sidebar entry.
pub fn nav_button(ui: &mut Ui, label: &str, active: bool, _accent: Color32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, 8.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    let col = if active {
        ui.visuals().strong_text_color()
    } else {
        ui.visuals().text_color().gamma_multiply(0.72)
    };
    ui.painter().text(
        pos2(rect.min.x + 14.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(15.0),
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
    p.rect_filled(filled, h / 2.0, if active { fill } else { Color32::from_gray(235) });
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

#[derive(Clone, Copy, PartialEq)]
pub enum HubIcon {
    Playlists,
    Liked,
    Albums,
    Artists,
}

/// The four Spotify hub rows get drawn icons instead of a letter.
pub fn hub_icon(label: &str) -> Option<HubIcon> {
    match label {
        "Playlists" => Some(HubIcon::Playlists),
        "Liked Songs" => Some(HubIcon::Liked),
        "Albums" => Some(HubIcon::Albums),
        "Artists" => Some(HubIcon::Artists),
        _ => None,
    }
}

pub fn paint_hub_tile(p: &Painter, rect: Rect, kind: HubIcon) {
    let bg = match kind {
        HubIcon::Liked => Color32::from_rgb(98, 76, 214),
        HubIcon::Playlists => Color32::from_rgb(36, 124, 156),
        HubIcon::Albums => Color32::from_rgb(58, 146, 84),
        HubIcon::Artists => Color32::from_rgb(190, 96, 120),
    };
    p.rect_filled(rect, 6.0, bg);
    let c = rect.center();
    let s = rect.height();
    let fg = Color32::WHITE;
    match kind {
        HubIcon::Liked => {
            let r = s * 0.15;
            p.circle_filled(pos2(c.x - s * 0.13, c.y - s * 0.08), r, fg);
            p.circle_filled(pos2(c.x + s * 0.13, c.y - s * 0.08), r, fg);
            p.add(Shape::convex_polygon(
                vec![
                    pos2(c.x - s * 0.27, c.y - s * 0.02),
                    pos2(c.x + s * 0.27, c.y - s * 0.02),
                    pos2(c.x, c.y + s * 0.27),
                ],
                fg,
                Stroke::NONE,
            ));
        }
        HubIcon::Playlists => {
            for dy in [-0.16_f32, 0.0, 0.16] {
                p.rect_filled(
                    Rect::from_center_size(pos2(c.x, c.y + dy * s), vec2(s * 0.5, s * 0.07)),
                    s * 0.035,
                    fg,
                );
            }
        }
        HubIcon::Albums => {
            p.circle_stroke(c, s * 0.26, Stroke::new(s * 0.06, fg));
            p.circle_filled(c, s * 0.06, fg);
        }
        HubIcon::Artists => {
            p.circle_filled(pos2(c.x, c.y - s * 0.1), s * 0.11, fg);
            p.with_clip_rect(rect)
                .circle_filled(pos2(c.x, c.y + s * 0.34), s * 0.24, fg);
        }
    }
}

/// Spotify-style search field: a rounded pill with a drawn magnifier, a clear (x) button and an
/// accent ring while it has focus. Escape clears it. Returns the text field's response, so callers
/// can use `has_focus()` / `lost_focus()` exactly as with a plain `TextEdit`.
pub fn search_bar(
    ui: &mut Ui,
    text: &mut String,
    hint: &str,
    accent: Color32,
    fg: Color32,
    sub: Color32,
    fill: Color32,
) -> Response {
    let h = 38.0;
    let id = ui.id().with(("search_bar", hint));
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
    let focused = ui.memory(|m| m.has_focus(id));
    let hovered = ui.rect_contains_pointer(rect);

    // Background pill and ring.
    let ring = if focused {
        Stroke::new(2.0, accent)
    } else if hovered {
        Stroke::new(1.0, sub.gamma_multiply(0.6))
    } else {
        Stroke::new(1.0, sub.gamma_multiply(0.25))
    };
    ui.painter()
        .rect(rect, egui::Rounding::same(h / 2.0), fill, ring);

    // Magnifier.
    let c = pos2(rect.min.x + 20.0, rect.center().y - 1.0);
    let icon = if focused { fg } else { sub };
    ui.painter().circle_stroke(c, 5.5, Stroke::new(1.8, icon));
    ui.painter().line_segment(
        [pos2(c.x + 4.0, c.y + 4.0), pos2(c.x + 9.0, c.y + 9.0)],
        Stroke::new(1.8, icon),
    );

    // The text field sits inside the pill, without its own frame.
    let inner = Rect::from_min_max(
        pos2(rect.min.x + 38.0, rect.min.y + 4.0),
        pos2(rect.max.x - 36.0, rect.max.y - 4.0),
    );
    let resp = ui.put(
        inner,
        egui::TextEdit::singleline(text)
            .id(id)
            .hint_text(hint)
            .frame(false)
            .text_color(fg)
            .font(FontId::proportional(15.0))
            .desired_width(inner.width()),
    );

    // Clear button.
    if !text.is_empty() {
        let xr = Rect::from_center_size(pos2(rect.max.x - 20.0, rect.center().y), vec2(24.0, 24.0));
        let xresp = ui.interact(xr, id.with("clear"), Sense::click());
        let col = if xresp.hovered() { fg } else { sub };
        let m = 4.5;
        let xc = xr.center();
        ui.painter().line_segment(
            [pos2(xc.x - m, xc.y - m), pos2(xc.x + m, xc.y + m)],
            Stroke::new(1.6, col),
        );
        ui.painter().line_segment(
            [pos2(xc.x - m, xc.y + m), pos2(xc.x + m, xc.y - m)],
            Stroke::new(1.6, col),
        );
        if xresp.clicked() {
            text.clear();
            ui.memory_mut(|m| m.request_focus(id));
        }
    }
    if focused && ui.input(|i| i.key_pressed(egui::Key::Escape)) && !text.is_empty() {
        text.clear();
    }
    resp
}
