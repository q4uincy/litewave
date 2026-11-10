//! The main (non-iPod) window: sidebar, library / Spotify lists and a Spotify-style player bar.

use super::ipod::{Item, Kind, Load, Node};
use super::widgets::{
    bar, hub_icon, icon_button, initial, mode_button, nav_button, paint_hub_tile, search_bar,
    tint_for, Icon,
    ModeIcon,
};
use super::{buffering_text, fmt_time, loading_hint, Action, App, Dialog, View, BUG_URL};
use crate::art::{self, ArtCache, ArtKey};
use crate::config::{Repeat, ShuffleMode};
use crate::library::Track;
use crate::lyrics::{self, Status as LyricsStatus};
use crate::spotify;
use crate::theme::mix;
use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Rect, Sense, TextureId};

const ROW_H: f32 = 52.0;

/// Cover image, or a colored placeholder with the title's first letter.
fn paint_cover(p: &egui::Painter, rect: Rect, tex: Option<TextureId>, title: &str) {
    match tex {
        Some(id) => {
            p.image(
                id,
                rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => {
            p.rect_filled(rect, 6.0, tint_for(title));
            p.text(
                rect.center(),
                Align2::CENTER_CENTER,
                initial(title),
                FontId::proportional(rect.height() * 0.42),
                Color32::from_white_alpha(200),
            );
        }
    }
}

/// One song row: index/play marker, cover, title + artist, album, duration.
#[allow(clippy::too_many_arguments)]
fn track_row(
    ui: &mut egui::Ui,
    cache: &mut ArtCache,
    row_no: usize,
    t: &Track,
    selected: bool,
    playing: bool,
    wide: bool,
    accent: Color32,
    text: Color32,
    sub: Color32,
    blocked: bool,
    actions: &mut Vec<Action>,
) -> egui::Response {
    // Explicit songs are greyed out while "Explicit content" is off.
    let (accent, text, sub) = if blocked {
        (accent.gamma_multiply(0.5), text.gamma_multiply(0.4), sub.gamma_multiply(0.5))
    } else {
        (accent, text, sub)
    };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
    let tex = t.art_key().and_then(|k| cache.get(ui.ctx(), &k, art::THUMB));
    if selected {
        ui.painter().rect_filled(rect, 8.0, accent.gamma_multiply(0.16));
    } else if resp.hovered() {
        ui.painter()
            .rect_filled(rect, 8.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    let cy = rect.center().y;

    // Left marker: number, equalizer while playing, and a play / pause button on hover.
    let mark = pos2(rect.min.x + 22.0, cy);
    let hovered = resp.hovered();
    let clock = ui.input(|i| i.time) as f32;
    let p = ui.painter();
    if selected && playing && !hovered {
        for k in 0..3 {
            let h = 5.0 + 7.0 * (0.5 + 0.5 * (clock * 5.0 + k as f32 * 1.9).sin());
            let x = mark.x - 7.0 + k as f32 * 5.5;
            p.rect_filled(
                Rect::from_min_max(pos2(x, mark.y + 6.0 - h), pos2(x + 3.5, mark.y + 6.0)),
                1.0,
                accent,
            );
        }
    } else if hovered || selected {
        let col = if selected { accent } else { text };
        if selected && playing {
            for dx in [-3.5_f32, 3.5] {
                p.rect_filled(
                    Rect::from_center_size(pos2(mark.x + dx, mark.y), vec2(3.5, 12.0)),
                    1.0,
                    col,
                );
            }
        } else {
            p.add(egui::Shape::convex_polygon(
                vec![
                    pos2(mark.x - 4.0, mark.y - 6.5),
                    pos2(mark.x - 4.0, mark.y + 6.5),
                    pos2(mark.x + 7.0, mark.y),
                ],
                col,
                egui::Stroke::NONE,
            ));
        }
    } else {
        p.text(
            mark,
            Align2::CENTER_CENTER,
            format!("{}", row_no + 1),
            FontId::proportional(13.0),
            sub,
        );
    }

    let cover = Rect::from_center_size(pos2(rect.min.x + 64.0, cy), vec2(38.0, 38.0));
    paint_cover(p, cover, tex, &t.title);

    let tx = rect.min.x + 94.0;
    let dur_w = 64.0;
    let album_w = if wide { rect.width() * 0.26 } else { 0.0 };
    let title_right = rect.max.x - dur_w - album_w - 12.0;
    let pc = p.with_clip_rect(Rect::from_min_max(
        pos2(tx, rect.min.y),
        pos2(title_right.max(tx + 20.0), rect.max.y),
    ));
    pc.text(
        pos2(tx, cy - 9.0),
        Align2::LEFT_CENTER,
        &t.title,
        FontId::proportional(14.5),
        if selected { accent } else { text },
    );
    let mut ax = tx;
    if t.explicit {
        let badge = Rect::from_center_size(pos2(tx + 6.5, cy + 11.0), vec2(13.0, 13.0));
        pc.rect_filled(badge, 2.0, sub.gamma_multiply(0.8));
        pc.text(
            badge.center(),
            Align2::CENTER_CENTER,
            "E",
            FontId::proportional(9.5),
            Color32::BLACK,
        );
        ax += 18.0;
    }
    let artist_rect = pc.text(
        pos2(ax, cy + 11.0),
        Align2::LEFT_CENTER,
        &t.artist,
        FontId::proportional(12.0),
        sub,
    );
    let mut album_rect: Option<Rect> = None;
    if wide {
        let ax = rect.max.x - dur_w - album_w;
        let clip = Rect::from_min_max(
            pos2(ax, rect.min.y),
            pos2(rect.max.x - dur_w - 8.0, rect.max.y),
        );
        let r = p.with_clip_rect(clip).text(
            pos2(ax, cy),
            Align2::LEFT_CENTER,
            &t.album,
            FontId::proportional(13.0),
            sub,
        );
        album_rect = Some(r.intersect(clip));
    }
    p.text(
        pos2(rect.max.x - 14.0, cy),
        Align2::RIGHT_CENTER,
        fmt_time(t.duration.as_secs_f32()),
        FontId::proportional(13.0),
        sub,
    );

    // "..." button (shown on hover): play next, add to queue, save.
    if !blocked {
        let more = Rect::from_center_size(pos2(rect.max.x - 54.0, cy), vec2(24.0, 24.0));
        if ui.rect_contains_pointer(rect) {
            ui.painter().text(
                more.center(),
                Align2::CENTER_CENTER,
                "...",
                FontId::proportional(16.0),
                text,
            );
        }
        if ui
            .interact(more, ui.id().with(("more", row_no)), Sense::click())
            .clicked()
        {
            actions.push(Action::RowMenu(t.clone(), more.left_bottom()));
        }
    }

    // The artist and the album are links: Spotify songs open their page, local songs filter the library.
    if !blocked && !t.is_episode() && t.yt_id.is_none() {
        let from_sp = t.spotify_id.is_some();
        let title_clip = Rect::from_min_max(
            pos2(tx, rect.min.y),
            pos2(title_right.max(tx + 20.0), rect.max.y),
        );
        let artist_hit = artist_rect.intersect(title_clip);
        if !t.artist.is_empty()
            && t.artist != "Unknown artist"
            && link_hit(ui, artist_hit, ui.id().with(("artist", row_no)), text)
        {
            actions.push(Action::OpenArtist(t.artist_id.clone(), t.artist.clone(), from_sp));
        }
        if let Some(r) = album_rect {
            if !t.album.is_empty() && link_hit(ui, r, ui.id().with(("album", row_no)), text) {
                actions.push(Action::OpenAlbum(t.album_id.clone(), t.album.clone(), from_sp));
            }
        }
    }
    resp
}

/// A clickable piece of text inside a row: pointer cursor and an underline while hovered.
fn link_hit(ui: &egui::Ui, rect: Rect, id: egui::Id, color: Color32) -> bool {
    if !rect.is_positive() {
        return false;
    }
    let r = ui
        .interact(rect, id, Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if r.hovered() {
        ui.painter().line_segment(
            [pos2(rect.min.x, rect.max.y), pos2(rect.max.x, rect.max.y)],
            egui::Stroke::new(1.0, color),
        );
    }
    r.clicked()
}

/// Column titles above a song list.
fn list_header(ui: &mut egui::Ui, wide: bool, sub: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
    let p = ui.painter();
    let cy = rect.center().y;
    let font = FontId::proportional(12.0);
    p.text(pos2(rect.min.x + 22.0, cy), Align2::CENTER_CENTER, "#", font.clone(), sub);
    p.text(pos2(rect.min.x + 94.0, cy), Align2::LEFT_CENTER, "Title", font.clone(), sub);
    if wide {
        let ax = rect.max.x - 64.0 - rect.width() * 0.26;
        p.text(pos2(ax, cy), Align2::LEFT_CENTER, "Album", font.clone(), sub);
    }
    p.text(pos2(rect.max.x - 14.0, cy), Align2::RIGHT_CENTER, "Time", font, sub);
    p.line_segment(
        [pos2(rect.min.x, rect.max.y), pos2(rect.max.x, rect.max.y)],
        egui::Stroke::new(1.0, sub.gamma_multiply(0.25)),
    );
}

/// Spotify's list header controls: the big round play / pause button and the shuffle menu button.
/// Call inside a `ui.horizontal`.
#[allow(clippy::too_many_arguments)]
fn list_controls(
    ui: &mut egui::Ui,
    actions: &mut Vec<Action>,
    menu: &mut Option<Rect>,
    playing_here: bool,
    on_play: Action,
    mode: ShuffleMode,
    bg: Color32,
    accent: Color32,
) {
    let icon = if playing_here { Icon::Pause } else { Icon::Play };
    if icon_button(ui, icon, 50.0, bg, Some(accent)).clicked() {
        actions.push(on_play);
    }
    ui.add_space(4.0);
    let r = mode_button(ui, ModeIcon::Shuffle, mode != ShuffleMode::Off, accent, mode == ShuffleMode::Smart, false)
        .on_hover_text("Shuffle options");
    if r.clicked() {
        *menu = if menu.is_some() { None } else { Some(r.rect) };
    }
}

impl App {
    /// The popup behind the shuffle buttons: Off, Shuffle, Smart shuffle.
    fn shuffle_menu_ui(
        &mut self,
        ctx: &egui::Context,
        actions: &mut Vec<Action>,
        accent: Color32,
        text: Color32,
        sub: Color32,
    ) {
        let Some(anchor) = self.shuffle_menu else { return };
        let above = anchor.center().y > ctx.screen_rect().center().y;
        let (pivot, pos) = if above {
            (Align2::LEFT_BOTTOM, anchor.left_top() - vec2(0.0, 8.0))
        } else {
            (Align2::LEFT_TOP, anchor.left_bottom() + vec2(0.0, 8.0))
        };
        let current = self.cfg.shuffle_mode();
        let mut close = false;
        let out = egui::Area::new(egui::Id::new("shuffle_menu"))
            .order(egui::Order::Foreground)
            .pivot(pivot)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(250.0);
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for (mode, label, hint) in [
                        (ShuffleMode::Off, "Off", "Play in order"),
                        (ShuffleMode::On, "Shuffle", "Random order, no repeats"),
                        (
                            ShuffleMode::Smart,
                            "Smart shuffle",
                            "Random, but never the same artist twice in a row",
                        ),
                    ] {
                        let (rect, resp) =
                            ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::click());
                        if resp.hovered() {
                            ui.painter()
                                .rect_filled(rect, 6.0, ui.visuals().widgets.hovered.weak_bg_fill);
                        }
                        let on = mode == current;
                        let p = ui.painter();
                        if on {
                            p.circle_filled(pos2(rect.min.x + 14.0, rect.center().y), 4.0, accent);
                        }
                        let clip = p.with_clip_rect(rect);
                        clip.text(
                            pos2(rect.min.x + 30.0, rect.center().y - 8.0),
                            Align2::LEFT_CENTER,
                            label,
                            FontId::proportional(14.0),
                            if on { accent } else { text },
                        );
                        clip.text(
                            pos2(rect.min.x + 30.0, rect.center().y + 10.0),
                            Align2::LEFT_CENTER,
                            hint,
                            FontId::proportional(11.0),
                            sub,
                        );
                        if resp.clicked() {
                            actions.push(Action::SetShuffle(mode));
                            close = true;
                        }
                    }
                });
            });
        let menu_rect = out.response.rect;
        let pressed_outside = ctx.input(|i| {
            i.pointer.any_pressed()
                && i.pointer
                    .interact_pos()
                    .map_or(false, |p| !menu_rect.contains(p) && !anchor.contains(p))
        });
        if close || pressed_outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.shuffle_menu = None;
        }
    }

    pub(super) fn classic_ui(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let accent = self.cfg.theme.accent();
        let text = self.cfg.theme.text();
        let sub = self.cfg.theme.subtext();
        let bg = self.cfg.theme.bg();
        let track_col = mix(self.cfg.theme.panel(), text, 0.25);

        let (title, artist, duration) = self
            .now
            .as_ref()
            .map(|t| (t.title.clone(), t.artist.clone(), t.duration))
            .unwrap_or_default();
        let bar_art = self.current_art(ctx, art::MID);
        let pos = self.position_secs();
        let playing = self.player.as_ref().map_or(false, |p| p.is_playing());
        let buf_secs = self.player.as_ref().and_then(|p| p.buffering_secs());
        let loaded = self.player.as_ref().map_or(false, |p| p.is_loaded());
        let dur_s = duration.as_secs_f32();
        let shown = self.drag_pos.unwrap_or(pos);
        let screen_w = ctx.screen_rect().width();
        let show_side = screen_w >= 640.0;
        let wide = screen_w >= 820.0;
        let lyrics_side = self.cfg.lyrics_panel && screen_w >= 760.0;
        let current_sp_id = self.now.as_ref().and_then(|t| t.spotify_id.clone());
        let mode = self.cfg.shuffle_mode();
        let allow_explicit = self.cfg.allow_explicit;
        let mut dismiss_error = false;
        let from_name: Option<String> = self.playing_from.as_ref().map(|(_, n)| n.clone());

        // ---- player bar --------------------------------------------------
        egui::TopBottomPanel::bottom("player")
            .exact_height(92.0)
            .show(ctx, |ui| {
                let full = ui.available_rect_before_wrap();
                let w = full.width();
                let narrow = w < 600.0;
                let side_w = if narrow { 0.0 } else { (w * 0.26).clamp(140.0, 300.0) };
                let gap = if narrow { 0.0 } else { 12.0 };
                let center = Rect::from_min_max(
                    pos2(full.min.x + side_w + gap, full.min.y),
                    pos2(full.max.x - side_w - gap, full.max.y),
                );

                // Now playing (left).
                if !narrow && !title.is_empty() {
                    let cy = full.center().y;
                    let p = ui.painter();
                    let cover = Rect::from_center_size(pos2(full.min.x + 28.0, cy), vec2(56.0, 56.0));
                    paint_cover(p, cover, bar_art, &title);
                    let tx = cover.max.x + 12.0;
                    let clip = Rect::from_min_max(pos2(tx, full.min.y), pos2(full.min.x + side_w, full.max.y));
                    let pc = p.with_clip_rect(clip);
                    pc.text(
                        pos2(tx, cy - 10.0),
                        Align2::LEFT_CENTER,
                        &title,
                        FontId::proportional(14.5),
                        text,
                    );
                    pc.text(
                        pos2(tx, cy + 10.0),
                        Align2::LEFT_CENTER,
                        match buf_secs {
                            Some(secs) => buffering_text(secs),
                            None => artist.as_str(),
                        },
                        FontId::proportional(12.0),
                        sub,
                    );
                    // Click the cover or title to go back to where the song is playing from.
                    let hit = Rect::from_min_max(cover.min, pos2(full.min.x + side_w, cover.max.y));
                    let r = ui
                        .interact(hit, ui.id().with("now_playing"), Sense::click())
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text(match &from_name {
                            Some(n) => format!("Go to {n}"),
                            None => "Go to the song list".to_string(),
                        });
                    if r.clicked() {
                        actions.push(Action::GoToSource);
                    }
                }

                // Transport (center).
                let ctrl = Rect::from_center_size(
                    pos2(center.center().x, full.min.y + 30.0),
                    vec2(320.0, 42.0),
                );
                ui.allocate_ui_at_rect(ctrl, |ui| {
                    ui.horizontal_centered(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        let shuffle_resp = mode_button(
                            ui,
                            ModeIcon::Shuffle,
                            mode != ShuffleMode::Off,
                            accent,
                            mode == ShuffleMode::Smart,
                            false,
                        )
                        .on_hover_text(match mode {
                            ShuffleMode::Off => "Shuffle: off",
                            ShuffleMode::On => "Shuffle: on",
                            ShuffleMode::Smart => "Smart shuffle: on",
                        });
                        if shuffle_resp.clicked() {
                            self.shuffle_menu = if self.shuffle_menu.is_some() {
                                None
                            } else {
                                Some(shuffle_resp.rect)
                            };
                        }
                        if icon_button(ui, Icon::Prev, 34.0, text, None).clicked() {
                            actions.push(Action::Prev);
                        }
                        let icon = if playing || buf_secs.is_some() { Icon::Pause } else { Icon::Play };
                        if icon_button(ui, icon, 42.0, bg, Some(text)).clicked() {
                            actions.push(Action::Toggle);
                        }
                        if icon_button(ui, Icon::Next, 34.0, text, None).clicked() {
                            actions.push(Action::Next);
                        }
                        let rep = match self.cfg.repeat {
                            Repeat::Off => "Repeat: off",
                            Repeat::All => "Repeat: all",
                            Repeat::One => "Repeat: one",
                        };
                        if mode_button(
                            ui,
                            ModeIcon::Repeat,
                            self.cfg.repeat != Repeat::Off,
                            accent,
                            false,
                            self.cfg.repeat == Repeat::One,
                        )
                        .on_hover_text(rep)
                        .clicked()
                        {
                            actions.push(Action::CycleRepeat);
                        }
                    });
                });

                // Seek bar (center, under the buttons).
                let seek = Rect::from_min_size(
                    pos2(center.min.x, full.min.y + 58.0),
                    vec2(center.width(), 22.0),
                );
                ui.allocate_ui_at_rect(seek, |ui| {
                    ui.horizontal_centered(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        ui.label(egui::RichText::new(fmt_time(shown)).size(11.5).color(sub));
                        let bw = (ui.available_width() - 48.0).max(40.0);
                        let enabled = loaded && dur_s > 0.0;
                        let (resp, f) = bar(ui, bw, shown / dur_s.max(0.1), accent, track_col, enabled);
                        if resp.dragged() {
                            self.drag_pos = Some(f * dur_s);
                        }
                        if resp.drag_stopped() {
                            if let Some(p) = self.drag_pos.take() {
                                actions.push(Action::Seek(p));
                            }
                        } else if resp.clicked() {
                            self.drag_pos = None;
                            actions.push(Action::Seek(f * dur_s));
                        }
                        ui.label(egui::RichText::new(fmt_time(dur_s)).size(11.5).color(sub));
                    });
                });

                // Volume (right).
                if !narrow {
                    let vr = Rect::from_min_size(
                        pos2(full.max.x - side_w, full.center().y - 17.0),
                        vec2(side_w, 34.0),
                    );
                    let vol_w = if side_w < 220.0 { 70.0 } else { 110.0 };
                    ui.allocate_ui_at_rect(vr, |ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let (resp, f) = bar(ui, vol_w, self.cfg.volume, accent, track_col, true);
                            if resp.dragged() || resp.clicked() {
                                actions.push(Action::Volume(f));
                            }
                            ui.add_sized(
                                [34.0, 16.0],
                                egui::Label::new(
                                    egui::RichText::new(format!("{}%", (self.cfg.volume * 100.0).round() as i32))
                                        .size(11.5)
                                        .color(sub),
                                ),
                            );
                            let lyr = mode_button(
                                ui,
                                ModeIcon::Lyrics,
                                self.cfg.lyrics_panel,
                                accent,
                                false,
                                false,
                            )
                            .on_hover_text("Lyrics");
                            if lyr.clicked() {
                                actions.push(Action::ToggleLyricsPanel);
                            }
                        });
                    });
                }
            });

        // ---- sidebar -----------------------------------------------------
        if show_side {
            egui::SidePanel::left("side")
                .exact_width(196.0)
                .resizable(false)
                .show(ctx, |ui| {
                    // Tall window: footer pinned to the bottom. Short window: everything scrolls.
                    let tall = ui.available_height() >= 720.0;
                    let mut content = |ui: &mut egui::Ui| {
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if let Some(tex) = &self.logo_tex {
                            ui.add(
                                egui::Image::new(egui::load::SizedTexture::new(tex.id(), egui::vec2(28.0, 28.0)))
                                    .rounding(7.0),
                            );
                        }
                        ui.label(egui::RichText::new("litewave").size(22.0).strong().color(accent));
                    });
                    ui.add_space(14.0);
                    if nav_button(ui, "Your Library", self.view == View::Library, accent).clicked() {
                        actions.push(Action::Library);
                    }
                    if nav_button(ui, "iPod mode", false, accent).clicked() {
                        actions.push(Action::SetIpod(true));
                    }
                    if nav_button(ui, "Lyrics", self.cfg.lyrics_panel, accent).clicked() {
                        actions.push(Action::ToggleLyricsPanel);
                    }

                    ui.add_space(14.0);
                    ui.label(egui::RichText::new("SPOTIFY").size(11.0).color(sub));
                    ui.add_space(4.0);
                    if self.sp.is_some() {
                        let root = self.sp_nav.first().map(|p| p.node.clone());
                        let in_sp = self.view == View::Spotify;
                        for (label, node) in [
                            ("Search", Node::SpSearch(String::new())),
                            ("Liked Songs", Node::SpLiked),
                            ("Playlists", Node::SpPlaylists),
                            ("Albums", Node::SpAlbums),
                            ("Artists", Node::SpArtists),
                            ("Podcasts", Node::SpShows),
                        ] {
                            let top_is_search =
                                matches!(self.sp_nav.last().map(|p| &p.node), Some(Node::SpSearch(_)));
                            let active = in_sp
                                && match (&root, &node) {
                                    (_, Node::SpSearch(_)) => top_is_search,
                                    (Some(r), n) => r == n && !top_is_search,
                                    _ => false,
                                };
                            if nav_button(ui, label, active, accent).clicked() {
                                actions.push(Action::SpRoot(node));
                            }
                        }
                        if nav_button(ui, "+ New playlist", false, accent).clicked() {
                            actions.push(Action::SpDialog(Dialog::NewPlaylist));
                        }
                    } else if nav_button(ui, "Connect Spotify", false, accent).clicked() {
                        actions.push(Action::SpotifyWindow);
                    }

                    ui.add_space(14.0);
                    ui.label(egui::RichText::new("MUSIC").size(11.0).color(sub));
                    ui.add_space(4.0);
                    let qlabel = if self.queue.is_empty() {
                        "Queue".to_string()
                    } else {
                        format!("Queue ({})", self.queue.len())
                    };
                    if nav_button(ui, &qlabel, self.view == View::Queue, accent).clicked() {
                        actions.push(Action::OpenQueue);
                    }
                    if nav_button(ui, "Open folder", false, accent).clicked() {
                        actions.push(Action::OpenFolder);
                    }
                    if nav_button(ui, "Add songs", false, accent).clicked() {
                        actions.push(Action::AddFiles);
                    }
                    if nav_button(ui, "Rescan", false, accent).clicked() {
                        actions.push(Action::Rescan);
                    }

                    ui.add_space(14.0);
                    ui.label(egui::RichText::new("YOUTUBE").size(11.0).color(sub));
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("  Coming soon").size(13.0).color(sub));
                    let explicit_label = if self.cfg.allow_explicit {
                        "Explicit content: on"
                    } else {
                        "Explicit content: off"
                    };
                    if tall {
                        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                            ui.add_space(10.0);
                            if nav_button(ui, explicit_label, !self.cfg.allow_explicit, accent).clicked() {
                                actions.push(Action::ToggleExplicit);
                            }
                            if nav_button(ui, "Report a bug", false, accent).clicked() {
                                ui.ctx().open_url(egui::OpenUrl::new_tab(BUG_URL));
                            }
                            if nav_button(ui, "Themes", self.view == View::Themes, accent).clicked() {
                                actions.push(Action::Themes);
                            }
                            if nav_button(ui, "Spotify account", self.view == View::Account, accent).clicked() {
                                actions.push(Action::SpotifyWindow);
                            }
                        });
                    } else {
                        ui.add_space(14.0);
                        if nav_button(ui, "Spotify account", self.view == View::Account, accent).clicked() {
                            actions.push(Action::SpotifyWindow);
                        }
                        if nav_button(ui, "Themes", self.view == View::Themes, accent).clicked() {
                            actions.push(Action::Themes);
                        }
                        if nav_button(ui, "Report a bug", false, accent).clicked() {
                            ui.ctx().open_url(egui::OpenUrl::new_tab(BUG_URL));
                        }
                        if nav_button(ui, explicit_label, !self.cfg.allow_explicit, accent).clicked() {
                            actions.push(Action::ToggleExplicit);
                        }
                        ui.add_space(10.0);
                    }
                    };
                    if tall {
                        content(ui);
                    } else {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| content(ui));
                    }
                });
        }

        // ---- lyrics (right) ----------------------------------------------
        if lyrics_side {
            egui::SidePanel::right("lyrics")
                .exact_width((screen_w * 0.3).clamp(280.0, 400.0))
                .resizable(false)
                .show(ctx, |ui| self.lyrics_panel(ui, actions, pos, text, sub));
        }

        // ---- main area ---------------------------------------------------
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(6.0);
            if !show_side {
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Library").clicked() {
                        actions.push(Action::Library);
                    }
                    if ui.button("Open folder").clicked() {
                        actions.push(Action::OpenFolder);
                    }
                    if ui.button("Add songs").clicked() {
                        actions.push(Action::AddFiles);
                    }
                    if ui.button("Lyrics").clicked() {
                        actions.push(Action::ToggleLyricsPanel);
                    }
                    if ui.button("iPod").clicked() {
                        actions.push(Action::SetIpod(true));
                    }
                    if ui.button("Themes").clicked() {
                        actions.push(Action::Themes);
                    }
                    if ui.button("Spotify").clicked() {
                        if self.sp.is_some() {
                            actions.push(Action::SpRoot(Node::SpLiked));
                        } else {
                            actions.push(Action::SpotifyWindow);
                        }
                    }
                });
                ui.add_space(4.0);
            }

            // Narrow window: the lyrics take over the main area instead of a side panel.
            if self.cfg.lyrics_panel && !lyrics_side {
                self.search_focus = false;
                self.lyrics_panel(ui, actions, pos, text, sub);
                return;
            }

            if self.view == View::Account {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    self.spotify_page(ui);
                });
                return;
            }

            if self.view == View::Queue {
                ui.label(egui::RichText::new("Queue").size(24.0).strong());
                ui.add_space(8.0);
                if let Some(t) = &self.now {
                    ui.label(egui::RichText::new("Now playing").size(12.0).color(sub));
                    ui.label(egui::RichText::new(format!("{} - {}", t.title, t.artist)).color(text));
                    ui.add_space(10.0);
                }
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("Next in queue ({})", self.queue.len()))
                            .size(12.0)
                            .color(sub),
                    );
                    if !self.queue.is_empty() && ui.small_button("Clear").clicked() {
                        actions.push(Action::QueueClear);
                    }
                });
                if self.queue.is_empty() {
                    ui.add_space(6.0);
                    ui.label(
                        egui::RichText::new("Nothing queued. Hover a song, press \"...\" and pick Play next or Add to queue.")
                            .color(sub),
                    );
                }
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    for (k, t) in self.queue.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(format!("{}", k + 1)).color(sub));
                            ui.label(egui::RichText::new(&t.title).color(text));
                            ui.label(egui::RichText::new(&t.artist).color(sub));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button("Remove").clicked() {
                                    actions.push(Action::QueueRemove(k));
                                }
                            });
                        });
                    }
                });
                return;
            }

            if self.view == View::Themes {
                ui.label(egui::RichText::new("Themes").size(24.0).strong());
                ui.add_space(8.0);
                let ctx2 = ui.ctx().clone();
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    self.themes_ui(&ctx2, ui);
                    self.theme_gallery_ui(&ctx2, ui);
                });
                return;
            }

            if self.view == View::Spotify && self.sp.is_some() {
                self.spotify_view(ui, actions, accent, text, sub, bg, mode, playing, wide, &current_sp_id);
                return;
            }

            ui.horizontal(|ui| {
                let heading = if self.view == View::YouTube {
                    "YouTube"
                } else if self.list_is_local {
                    "Your Library"
                } else {
                    "Playing from"
                };
                ui.label(egui::RichText::new(heading).size(24.0).strong());
                let n = self.filtered.len();
                let count = if self.scan_rx.is_some() {
                    format!("Scanning... {}", self.scan_count)
                } else {
                    format!("{} track{}", n, if n == 1 { "" } else { "s" })
                };
                ui.label(egui::RichText::new(count).size(13.0).color(sub));
            });
            ui.add_space(6.0);
            let here = self.current.is_some() && loaded;
            let on_play = if here { Action::Toggle } else { Action::PlayAll };
            ui.horizontal(|ui| {
                list_controls(
                    ui,
                    actions,
                    &mut self.shuffle_menu,
                    here && playing,
                    on_play,
                    mode,
                    bg,
                    accent,
                );
                ui.add_space(8.0);
                if self.view == View::YouTube {
                    let r = search_bar(
                        ui,
                        &mut self.yt_query,
                        "Search YouTube (Enter)",
                        accent,
                        text,
                        sub,
                        mix(bg, text, 0.08),
                    );
                    self.search_focus = r.has_focus();
                    if r.lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter))
                        && !self.yt_query.trim().is_empty()
                    {
                        actions.push(Action::SpLink(format!("yt: {}", self.yt_query.trim())));
                    }
                } else {
                    let r = search_bar(
                        ui,
                        &mut self.query,
                        "Search your library",
                        accent,
                        text,
                        sub,
                        mix(bg, text, 0.08),
                    );
                    self.search_focus = r.has_focus();
                }
            });
            if self.view == View::YouTube {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Sign-in browser:").size(12.0).color(sub));
                    let cur = if self.cfg.ytdlp_browser.is_empty() {
                        "none".to_string()
                    } else {
                        self.cfg.ytdlp_browser.clone()
                    };
                    egui::ComboBox::from_id_source("yt_browser")
                        .selected_text(cur)
                        .show_ui(ui, |ui| {
                            for b in ["", "firefox", "edge", "chrome", "brave"] {
                                let label = if b.is_empty() { "none" } else { b };
                                if ui
                                    .selectable_value(&mut self.cfg.ytdlp_browser, b.to_string(), label)
                                    .changed()
                                {
                                    self.cfg_dirty = true;
                                }
                            }
                        });
                    ui.label(
                        egui::RichText::new("Needed if YouTube says \"not a bot\". Sign in to youtube.com in that browser, then close it. Firefox is most reliable. Use a spare Google account, not your main one.")
                            .size(11.5)
                            .color(sub),
                    );
                });
            }
            if let Some(err) = &self.error {
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(Color32::LIGHT_RED, err);
                    if ui.small_button("dismiss").clicked() {
                        dismiss_error = true;
                    }
                });
            }
            ui.add_space(8.0);

            if self.view == View::YouTube && !self.tracks.iter().any(|t| t.yt_id.is_some()) {
                ui.add_space(16.0);
                ui.label(egui::RichText::new("Type a song or artist above and press Enter.").color(sub));
                ui.label(
                    egui::RichText::new("Needs yt-dlp installed:  winget install yt-dlp.yt-dlp")
                        .size(12.0)
                        .color(sub),
                );
                return;
            }
            if self.tracks.is_empty() && self.scan_rx.is_none() {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        egui::RichText::new("Drop a folder or songs here, or use Open folder")
                            .color(sub),
                    );
                });
                return;
            }

            let n = self.filtered.len();
            list_header(ui, wide, sub);
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show_rows(ui, ROW_H, n, |ui, range| {
                        for row in range {
                            let i = self.filtered[row];
                            let t = &self.tracks[i];
                            let selected = Some(i) == self.current;
                            let resp = track_row(
                                ui,
                                &mut self.art,
                                row,
                                t,
                                selected,
                                selected && playing,
                                wide,
                                accent,
                                text,
                                sub,
                                t.explicit && !allow_explicit,
                                actions,
                            );
                            resp.context_menu(|ui| queue_buttons(ui, actions, t));
                            if resp.clicked() {
                                // Clicking the song that is current pauses / resumes it.
                                actions.push(if selected { Action::Toggle } else { Action::Play(i) });
                            }
                        }
                    });
            });
        });

        self.shuffle_menu_ui(ctx, actions, accent, text, sub);
        self.row_menu_ui(ctx, actions);
        self.dialog_ui(ctx, actions);
        self.notice_ui(ctx);

        if dismiss_error {
            self.error = None;
        }
    }

    /// Lyrics panel: synced lines follow the song (click a line to jump there), plain lyrics scroll.
    fn lyrics_panel(
        &mut self,
        ui: &mut egui::Ui,
        actions: &mut Vec<Action>,
        pos: f32,
        text: Color32,
        sub: Color32,
    ) {
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Lyrics").size(18.0).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Close").clicked() {
                    actions.push(Action::ToggleLyricsPanel);
                }
            });
        });
        if let Some(t) = &self.now {
            ui.label(
                egui::RichText::new(format!("{} - {}", t.title, t.artist))
                    .size(12.0)
                    .color(sub),
            );
        }
        ui.add_space(8.0);

        let Some(l) = self.lyrics.as_ref() else {
            ui.add_space(24.0);
            let msg = match self.lyrics_status {
                LyricsStatus::Loading => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(egui::RichText::new("Finding lyrics...").color(sub));
                    });
                    return;
                }
                LyricsStatus::Missing => {
                    "No lyrics found for this song.\n\nPut a .lrc file with the same name next to the song to add your own."
                }
                LyricsStatus::Failed => "Could not reach the lyrics service. Are you online?",
                _ => {
                    if self.now.is_some() {
                        ""
                    } else {
                        "Play a song to see its lyrics."
                    }
                }
            };
            ui.label(egui::RichText::new(msg).color(sub));
            return;
        };

        let synced = l.synced;
        let cur = l.current(pos + lyrics::LEAD);
        // Only re-center when the line changes, so the user can still scroll by hand.
        let recenter = cur.is_some() && cur != self.lyric_shown;
        self.lyric_shown = cur;
        if !synced {
            ui.label(
                egui::RichText::new("Not synced: these lyrics have no timing, so they will not follow along.")
                    .size(11.0)
                    .color(sub),
            );
            ui.add_space(6.0);
        }
        let pad = if synced { 28.0 } else { 0.0 };
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(pad);
                for (i, (t, line)) in l.lines.iter().enumerate() {
                    let active = Some(i) == cur;
                    let shown = if line.trim().is_empty() { "• • •" } else { line.as_str() };
                    let (size, col) = if !synced {
                        (15.0, text)
                    } else if active {
                        (22.0, text)
                    } else if cur.map_or(false, |c| i < c) {
                        (18.0, sub.gamma_multiply(0.55))
                    } else {
                        (18.0, sub)
                    };
                    let rt = egui::RichText::new(shown).size(size).color(col);
                    let rt = if active { rt.strong() } else { rt };
                    let r = ui.add(egui::Label::new(rt).sense(Sense::click()).selectable(false));
                    if synced {
                        let r = r.on_hover_cursor(egui::CursorIcon::PointingHand);
                        if r.clicked() {
                            actions.push(Action::Seek(*t));
                        }
                        if active && recenter {
                            r.scroll_to_me(Some(egui::Align::Center));
                        }
                        ui.add_space(12.0);
                    } else {
                        ui.add_space(4.0);
                    }
                }
                ui.add_space(pad);
            });
    }

    /// Spotify browsing in the main window: search, playlists, albums, artists, podcasts and track lists.
    #[allow(clippy::too_many_arguments)]
    fn spotify_view(
        &mut self,
        ui: &mut egui::Ui,
        actions: &mut Vec<Action>,
        accent: Color32,
        text: Color32,
        sub: Color32,
        bg: Color32,
        mode: ShuffleMode,
        playing: bool,
        wide: bool,
        current_sp_id: &Option<String>,
    ) {
        let Some(page) = self.sp_nav.last() else {
            ui.label(egui::RichText::new("Pick something from the sidebar").color(sub));
            return;
        };
        // The playlist on this page, when it is one the user may edit.
        let edit_pl: Option<(String, String)> = match &page.node {
            Node::SpPlaylist(id, name) if self.sp_editable.iter().any(|e| e.id == *id) => {
                Some((id.clone(), name.clone()))
            }
            _ => None,
        };
        let allow_explicit = self.cfg.allow_explicit;
        let page_uri = node_uri(&page.node);
        let on_liked = page.node == Node::SpLiked;
        let is_search = matches!(page.node, Node::SpSearch(_));

        ui.horizontal(|ui| {
            if self.sp_nav.len() > 1 && ui.button("Back").clicked() {
                actions.push(Action::SpBack);
            }
            ui.label(egui::RichText::new(&page.title).size(24.0).strong());
            if ui.small_button("Refresh").clicked() {
                actions.push(Action::SpRefresh);
            }
            if let Some((id, name)) = &edit_pl {
                if ui.small_button("Rename").clicked() {
                    actions.push(Action::SpDialog(Dialog::Rename(id.clone(), name.clone())));
                }
                if ui.small_button("Delete").clicked() {
                    actions.push(Action::SpDialog(Dialog::Delete(id.clone(), name.clone())));
                }
            } else if let Some(uri) = &page_uri {
                let label = if matches!(page.node, Node::SpArtist(..)) { "Follow" } else { "Save" };
                if ui.small_button(label).clicked() {
                    actions.push(Action::SpSave(vec![uri.clone()]));
                }
            }
        });
        ui.add_space(4.0);
        let r = search_bar(
            ui,
            &mut self.sp_link,
            "Search Spotify or paste a link (Enter)",
            accent,
            text,
            sub,
            mix(bg, text, 0.08),
        );
        self.search_focus = r.has_focus();
        if r.changed() {
            self.sp_typed = std::time::Instant::now();
        }
        if r.lost_focus()
            && ui.input(|i| i.key_pressed(egui::Key::Enter))
            && !self.sp_link.trim().is_empty()
        {
            actions.push(Action::SpLink(self.sp_link.trim().to_string()));
        } else {
            // Search while typing: wait for a short pause, then show results (like Spotify).
            let q = self.sp_link.trim().to_string();
            let lower = q.to_lowercase();
            let special = lower.starts_with("yt:")
                || lower.starts_with("yt ")
                || super::parse_spotify_link(&q).is_some();
            if q.is_empty() {
                if is_search && !self.sp_live.is_empty() {
                    actions.push(Action::SpLive(String::new()));
                }
            } else if q.chars().count() >= 1 && !special && q != self.sp_live {
                if self.sp_typed.elapsed() >= std::time::Duration::from_millis(350) {
                    actions.push(Action::SpLive(q));
                } else {
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(120));
                }
            }
        }
        if let Some(err) = &self.error {
            ui.colored_label(Color32::LIGHT_RED, err);
        }
        ui.add_space(8.0);

        match &page.load {
            Load::Loading if !(is_search && !page.items.is_empty()) => {
                ui.add_space(6.0);
                let secs = page.since.elapsed().as_secs_f32();
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(16.0).color(accent));
                    ui.label(egui::RichText::new("Loading...").color(sub));
                    ui.label(egui::RichText::new(loading_hint(secs)).size(12.0).color(sub));
                });
                let note = spotify::status_note();
                if !note.is_empty() {
                    ui.label(egui::RichText::new(note).size(12.0).color(sub));
                }
                ui.add_space(10.0);
                // Grey placeholder rows, so the page does not jump when the songs arrive.
                let faint = mix(bg, text, 0.07);
                for k in 0..9 {
                    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::hover());
                    let cy = rect.center().y;
                    let p = ui.painter();
                    p.rect_filled(Rect::from_center_size(pos2(rect.min.x + 30.0, cy), vec2(38.0, 38.0)), 6.0, faint);
                    let w = 150.0 + ((k * 53) % 140) as f32;
                    p.rect_filled(
                        Rect::from_min_size(pos2(rect.min.x + 60.0, cy - 11.0), vec2(w, 9.0)),
                        4.0,
                        faint,
                    );
                    p.rect_filled(
                        Rect::from_min_size(pos2(rect.min.x + 60.0, cy + 4.0), vec2(w * 0.6, 7.0)),
                        4.0,
                        faint,
                    );
                }
                // Keep the waiting text moving even when nothing else repaints.
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
            }
            Load::Failed(e) => {
                ui.add_space(12.0);
                ui.colored_label(Color32::LIGHT_RED, e);
            }
            Load::Ready | Load::Loading if is_search => {
                let edit_id = edit_pl.as_ref().map(|(id, _)| id.as_str());
                ui.push_id("search_results", |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            let empty_query = matches!(&page.node, Node::SpSearch(q) if q.is_empty());
                            if empty_query {
                                if !self.cfg.recent_searches.is_empty() {
                                    ui.add_space(8.0);
                                    ui.horizontal(|ui| {
                                        ui.label(egui::RichText::new("Recent searches").size(20.0).strong());
                                        if ui.small_button("Clear").clicked() {
                                            actions.push(Action::ClearRecent);
                                        }
                                    });
                                    ui.add_space(6.0);
                                    ui.horizontal_wrapped(|ui| {
                                        ui.spacing_mut().item_spacing = egui::vec2(8.0, 8.0);
                                        for r in &self.cfg.recent_searches {
                                            if ui
                                                .add(egui::Button::new(egui::RichText::new(r.as_str()).size(13.0)).rounding(14.0).min_size(vec2(0.0, 28.0)))
                                                .clicked()
                                            {
                                                self.sp_link = r.clone();
                                                actions.push(Action::SpLink(r.clone()));
                                            }
                                        }
                                    });
                                }
                                // Your playlists as cover cards.
                                if let Some((_, items, _)) = self.page_cache.get("SpPlaylists") {
                                    ui.add_space(18.0);
                                    ui.label(egui::RichText::new("Your playlists").size(20.0).strong());
                                    ui.add_space(8.0);
                                    let card_w = 132.0;
                                    ui.horizontal_wrapped(|ui| {
                                        ui.spacing_mut().item_spacing = vec2(16.0, 16.0);
                                        for it in items.iter().take(30) {
                                            let Kind::Open(node) = &it.kind else { continue };
                                            let (rect, resp) = ui
                                                .allocate_exact_size(vec2(card_w, card_w + 34.0), Sense::click());
                                            let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                                            if resp.hovered() {
                                                ui.painter().rect_filled(
                                                    rect.expand(6.0),
                                                    10.0,
                                                    ui.visuals().widgets.hovered.weak_bg_fill,
                                                );
                                            }
                                            let tex = it
                                                .art
                                                .clone()
                                                .and_then(|u| self.art.get(ui.ctx(), &ArtKey::Url(u), art::MID));
                                            let cover = Rect::from_min_size(rect.min, vec2(card_w, card_w));
                                            let p = ui.painter();
                                            paint_cover(p, cover, tex, &it.label);
                                            p.with_clip_rect(Rect::from_min_max(
                                                pos2(rect.min.x, cover.max.y + 4.0),
                                                rect.max,
                                            ))
                                            .text(
                                                pos2(rect.min.x, cover.max.y + 10.0),
                                                Align2::LEFT_CENTER,
                                                &it.label,
                                                FontId::proportional(13.5),
                                                text,
                                            );
                                            if resp.clicked() {
                                                actions.push(Action::SpPush(node.clone()));
                                            }
                                        }
                                    });
                                }
                                ui.add_space(14.0);
                            }
                            for it in &page.items {
                                match &it.kind {
                                    Kind::Info => match it.label.strip_prefix("# ") {
                                        Some(head) => {
                                            ui.add_space(14.0);
                                            ui.label(egui::RichText::new(head).size(18.0).strong());
                                            ui.add_space(4.0);
                                        }
                                        None => {
                                            ui.label(egui::RichText::new(&it.label).color(sub));
                                        }
                                    },
                                    Kind::Play(i) => {
                                        let Some(t) = page.tracks.get(*i) else { continue };
                                        let selected = current_sp_id.is_some()
                                            && current_sp_id.as_deref() == t.spotify_id.as_deref();
                                        let clicked = sp_track_row(
                                            ui,
                                            &mut self.art,
                                            *i,
                                            t,
                                            selected,
                                            selected && playing,
                                            wide,
                                            (accent, text, sub),
                                            &self.sp_editable,
                                            edit_id,
                                            on_liked,
                                            t.explicit && !allow_explicit,
                                            actions,
                                        );
                                        if clicked {
                                            actions.push(if selected { Action::Toggle } else { Action::SpPlay(*i) });
                                        }
                                    }
                                    Kind::Open(node) => {
                                        let resp = entry_row(ui, &mut self.art, it, text, sub);
                                        if let Some(uri) = node_uri(node) {
                                            resp.context_menu(|ui| {
                                                entry_menu(ui, actions, node, &uri, &self.sp_editable)
                                            });
                                        }
                                        if resp.clicked() {
                                            actions.push(Action::SpPush(node.clone()));
                                        }
                                    }
                                    Kind::Run(_) => {}
                                }
                            }
                        });
                });
            }
            Load::Ready => {
                let is_tracks = !page.tracks.is_empty();
                let n = if is_tracks { page.tracks.len() } else { page.items.len() };
                let edit_id = edit_pl.as_ref().map(|(id, _)| id.as_str());
                if is_tracks {
                    let here = current_sp_id.is_some()
                        && page.tracks.iter().any(|t| t.spotify_id == *current_sp_id);
                    let on_play = if here { Action::Toggle } else { Action::SpPlayAll };
                    ui.horizontal(|ui| {
                        list_controls(
                            ui,
                            actions,
                            &mut self.shuffle_menu,
                            here && playing,
                            on_play,
                            mode,
                            bg,
                            accent,
                        );
                        ui.add_space(8.0);
                        ui.label(
                            egui::RichText::new("Right-click a song for more options")
                                .size(11.5)
                                .color(sub),
                        );
                    });
                    ui.add_space(4.0);
                    list_header(ui, wide, sub);
                }
                ui.push_id(self.sp_nav.len(), |ui| {
                    ui.scope(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show_rows(ui, ROW_H, n, |ui, range| {
                                for i in range {
                                    if is_tracks {
                                        let t = &page.tracks[i];
                                        let selected = current_sp_id.is_some()
                                            && current_sp_id.as_deref() == t.spotify_id.as_deref();
                                        let clicked = sp_track_row(
                                            ui,
                                            &mut self.art,
                                            i,
                                            t,
                                            selected,
                                            selected && playing,
                                            wide,
                                            (accent, text, sub),
                                            &self.sp_editable,
                                            edit_id,
                                            on_liked,
                                            t.explicit && !allow_explicit,
                                            actions,
                                        );
                                        if clicked {
                                            actions.push(if selected { Action::Toggle } else { Action::SpPlay(i) });
                                        }
                                    } else {
                                        let it = &page.items[i];
                                        let resp = entry_row(ui, &mut self.art, it, text, sub);
                                        if let Kind::Open(node) = &it.kind {
                                            if let Some(uri) = node_uri(node) {
                                                resp.context_menu(|ui| {
                                                    entry_menu(ui, actions, node, &uri, &self.sp_editable)
                                                });
                                            }
                                            if resp.clicked() {
                                                actions.push(Action::SpPush(node.clone()));
                                            }
                                        }
                                    }
                                }
                            });
                    });
                });
            }
            // Never reached (handled above); keeps the match exhaustive.
            Load::Loading => {}
        }
    }

    /// The name / confirm popup for creating, renaming and deleting playlists.
    fn dialog_ui(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let Some(d) = self.dialog.clone() else { return };
        let (title, ok_label) = match &d {
            Dialog::NewPlaylist => ("New playlist", "Create"),
            Dialog::Rename(..) => ("Rename playlist", "Rename"),
            Dialog::Delete(..) => ("Delete playlist", "Delete"),
        };
        let mut ok = false;
        let mut cancel = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let accent = self.cfg.theme.accent();
        let danger = matches!(d, Dialog::Delete(..));
        egui::Window::new(title)
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .frame(
                egui::Frame::popup(&ctx.style())
                    .inner_margin(egui::Margin::same(18.0))
                    .rounding(14.0),
            )
            .show(ctx, |ui| {
                ui.set_width(320.0);
                ui.label(egui::RichText::new(title).size(18.0).strong());
                ui.add_space(10.0);
                match &d {
                    Dialog::Delete(_, name) => {
                        ui.label(format!("Delete \"{name}\"? This removes it from your library."));
                    }
                    _ => {
                        let r = ui.add(
                            egui::TextEdit::singleline(&mut self.dialog_text)
                                .hint_text("Playlist name")
                                .margin(egui::vec2(10.0, 8.0))
                                .desired_width(f32::INFINITY),
                        );
                        if ui.ctx().memory(|m| m.focused().is_none()) {
                            r.request_focus();
                        }
                        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            ok = true;
                        }
                    }
                }
                ui.add_space(14.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (fill, fg) = if danger {
                        (Color32::from_rgb(200, 60, 60), Color32::WHITE)
                    } else {
                        (accent, Color32::BLACK)
                    };
                    let b = egui::Button::new(egui::RichText::new(ok_label).color(fg).strong())
                        .fill(fill)
                        .rounding(16.0)
                        .min_size(vec2(84.0, 30.0));
                    if ui.add(b).clicked() {
                        ok = true;
                    }
                    let c = egui::Button::new("Cancel")
                        .frame(false)
                        .min_size(vec2(70.0, 30.0));
                    if ui.add(c).clicked() {
                        cancel = true;
                    }
                });
            });
        // Keep the keyboard shortcuts (space = pause...) out of the way while the popup is open.
        self.search_focus = true;
        if ok {
            actions.push(Action::SpDialogOk);
        } else if cancel {
            self.dialog = None;
        }
    }

    /// The popup of a song row's "..." button.
    fn row_menu_ui(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let Some((t, pos, opened)) = self.row_menu.clone() else { return };
        let mut close = false;
        let out = egui::Area::new(egui::Id::new("row_menu"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(170.0);
                    if ui.button("Play next").clicked() {
                        actions.push(Action::QueueNext(t.clone()));
                        close = true;
                    }
                    if ui.button("Add to queue").clicked() {
                        actions.push(Action::QueueAdd(t.clone()));
                        close = true;
                    }
                    if let Some(uri) = t.spotify_uri() {
                        if ui.button("Save to Liked Songs").clicked() {
                            actions.push(Action::SpSave(vec![uri]));
                            close = true;
                        }
                    }
                });
            });
        let at = ctx.input(|i| i.pointer.interact_pos()).unwrap_or_default();
        let outside = ctx.input(|i| i.pointer.any_click())
            && opened.elapsed() > std::time::Duration::from_millis(200)
            && !out.response.rect.contains(at);
        let esc = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        if close || outside || esc {
            self.row_menu = None;
        }
    }

    /// A short "done" message above the player bar.
    fn notice_ui(&mut self, ctx: &egui::Context) {
        let (msg, expired) = match &self.notice {
            Some((m, at)) => (m.clone(), at.elapsed() > std::time::Duration::from_secs(4)),
            None => return,
        };
        if expired {
            self.notice = None;
            return;
        }
        egui::Area::new(egui::Id::new("notice"))
            .order(egui::Order::Foreground)
            .anchor(Align2::CENTER_BOTTOM, vec2(0.0, -104.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.label(msg);
                });
            });
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
    }
}

/// `spotify:<kind>:<id>` for things that can be saved, followed or removed from the library.
fn node_uri(n: &Node) -> Option<String> {
    match n {
        Node::SpPlaylist(id, _) => Some(format!("spotify:playlist:{id}")),
        Node::SpAlbum(id, _) => Some(format!("spotify:album:{id}")),
        Node::SpArtist(id, _) => Some(format!("spotify:artist:{id}")),
        Node::SpShow(id, _) => Some(format!("spotify:show:{id}")),
        _ => None,
    }
}

/// Artist / album / playlist / podcast row: cover, name and a ">" marker.
fn entry_row(ui: &mut egui::Ui, cache: &mut ArtCache, it: &Item, text: Color32, sub: Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
    let tex = it
        .art
        .clone()
        .and_then(|u| cache.get(ui.ctx(), &ArtKey::Url(u), art::THUMB));
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, 8.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    let cy = rect.center().y;
    let p = ui.painter();
    let cover = Rect::from_center_size(pos2(rect.min.x + 30.0, cy), vec2(38.0, 38.0));
    match (tex, hub_icon(&it.label)) {
        (None, Some(kind)) => paint_hub_tile(p, cover, kind),
        _ => paint_cover(p, cover, tex, &it.label),
    }
    p.with_clip_rect(Rect::from_min_max(
        pos2(rect.min.x + 60.0, rect.min.y),
        pos2(rect.max.x - 30.0, rect.max.y),
    ))
    .text(
        pos2(rect.min.x + 60.0, cy),
        Align2::LEFT_CENTER,
        &it.label,
        FontId::proportional(14.5),
        text,
    );
    if matches!(it.kind, Kind::Open(_)) {
        p.text(
            pos2(rect.max.x - 16.0, cy),
            Align2::CENTER_CENTER,
            ">",
            FontId::proportional(14.0),
            sub,
        );
    }
    resp
}

/// A Spotify song / episode row with its right-click menu. Returns true when the row was clicked.
#[allow(clippy::too_many_arguments)]
fn sp_track_row(
    ui: &mut egui::Ui,
    cache: &mut ArtCache,
    row_no: usize,
    t: &Track,
    selected: bool,
    playing: bool,
    wide: bool,
    colors: (Color32, Color32, Color32),
    editable: &[spotify::Entry],
    edit_pid: Option<&str>,
    on_liked: bool,
    blocked: bool,
    actions: &mut Vec<Action>,
) -> bool {
    let (accent, text, sub) = colors;
    let resp = track_row(
        ui, cache, row_no, t, selected, playing, wide, accent, text, sub, blocked, actions,
    );
    resp.context_menu(|ui| track_menu(ui, actions, t, editable, edit_pid, on_liked));
    resp.clicked()
}

/// "Play next" and "Add to queue" entries for a song menu.
fn queue_buttons(ui: &mut egui::Ui, actions: &mut Vec<Action>, t: &Track) {
    if ui.button("Play next").clicked() {
        actions.push(Action::QueueNext(t.clone()));
        ui.close_menu();
    }
    if ui.button("Add to queue").clicked() {
        actions.push(Action::QueueAdd(t.clone()));
        ui.close_menu();
    }
}

/// Right-click menu of a song: add to a playlist, save to Liked Songs, remove from this playlist.
fn track_menu(
    ui: &mut egui::Ui,
    actions: &mut Vec<Action>,
    t: &Track,
    editable: &[spotify::Entry],
    edit_pid: Option<&str>,
    on_liked: bool,
) {
    queue_buttons(ui, actions, t);
    let Some(uri) = t.spotify_uri() else {
        return;
    };
    ui.separator();
    ui.menu_button("Add to playlist", |ui| {
        if editable.is_empty() {
            ui.label("No playlists you can edit yet");
        }
        egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
            for e in editable {
                if ui.button(e.name.as_str()).clicked() {
                    actions.push(Action::SpAddToPlaylist(e.id.clone(), vec![uri.clone()]));
                    ui.close_menu();
                }
            }
        });
    });
    if on_liked {
        if ui.button("Remove from Liked Songs").clicked() {
            actions.push(Action::SpUnsave(vec![uri.clone()]));
            ui.close_menu();
        }
    } else {
        let label = if t.is_episode() { "Save episode" } else { "Save to Liked Songs" };
        if ui.button(label).clicked() {
            actions.push(Action::SpSave(vec![uri.clone()]));
            ui.close_menu();
        }
    }
    if let Some(pid) = edit_pid {
        if ui.button("Remove from this playlist").clicked() {
            actions.push(Action::SpRemoveFromPlaylist(pid.to_string(), vec![uri.clone()]));
            ui.close_menu();
        }
    }
}

/// Right-click menu of an album / artist / playlist / podcast row.
fn entry_menu(
    ui: &mut egui::Ui,
    actions: &mut Vec<Action>,
    node: &Node,
    uri: &str,
    editable: &[spotify::Entry],
) {
    let artist = matches!(node, Node::SpArtist(..));
    if ui.button(if artist { "Follow" } else { "Save to your library" }).clicked() {
        actions.push(Action::SpSave(vec![uri.to_string()]));
        ui.close_menu();
    }
    if ui.button(if artist { "Unfollow" } else { "Remove from your library" }).clicked() {
        actions.push(Action::SpUnsave(vec![uri.to_string()]));
        ui.close_menu();
    }
    if let Node::SpPlaylist(id, name) = node {
        if editable.iter().any(|e| e.id == *id) {
            if ui.button("Rename").clicked() {
                actions.push(Action::SpDialog(Dialog::Rename(id.clone(), name.clone())));
                ui.close_menu();
            }
            if ui.button("Delete playlist").clicked() {
                actions.push(Action::SpDialog(Dialog::Delete(id.clone(), name.clone())));
                ui.close_menu();
            }
        }
    }
}
