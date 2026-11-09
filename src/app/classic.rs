//! The main (non-iPod) window: sidebar, library / Spotify lists and a Spotify-style player bar.

use super::ipod::{Kind, Load, Node};
use super::widgets::{bar, icon_button, initial, nav_button, pill, tint_for, Icon};
use super::{fmt_time, Action, App, View};
use crate::art::{self, ArtCache, ArtKey};
use crate::config::Repeat;
use crate::library::Track;
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
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
    let tex = t.art_key().and_then(|k| cache.get(ui.ctx(), &k, art::THUMB));
    if selected {
        ui.painter().rect_filled(rect, 8.0, accent.gamma_multiply(0.16));
    } else if resp.hovered() {
        ui.painter()
            .rect_filled(rect, 8.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    let cy = rect.center().y;
    let p = ui.painter();

    if selected && playing {
        let c = pos2(rect.min.x + 22.0, cy);
        p.add(egui::Shape::convex_polygon(
            vec![
                pos2(c.x - 4.0, c.y - 6.0),
                pos2(c.x - 4.0, c.y + 6.0),
                pos2(c.x + 6.0, c.y),
            ],
            accent,
            egui::Stroke::NONE,
        ));
    } else {
        p.text(
            pos2(rect.min.x + 22.0, cy),
            Align2::CENTER_CENTER,
            format!("{}", row_no + 1),
            FontId::proportional(13.0),
            if selected { accent } else { sub },
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
    pc.text(
        pos2(tx, cy + 11.0),
        Align2::LEFT_CENTER,
        &t.artist,
        FontId::proportional(12.0),
        sub,
    );
    if wide {
        let ax = rect.max.x - dur_w - album_w;
        p.with_clip_rect(Rect::from_min_max(
            pos2(ax, rect.min.y),
            pos2(rect.max.x - dur_w - 8.0, rect.max.y),
        ))
        .text(
            pos2(ax, cy),
            Align2::LEFT_CENTER,
            &t.album,
            FontId::proportional(13.0),
            sub,
        );
    }
    p.text(
        pos2(rect.max.x - 14.0, cy),
        Align2::RIGHT_CENTER,
        fmt_time(t.duration.as_secs_f32()),
        FontId::proportional(13.0),
        sub,
    );
    resp
}

impl App {
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
        let buffering = self.player.as_ref().map_or(false, |p| p.is_buffering());
        let loaded = self.player.as_ref().map_or(false, |p| p.is_loaded());
        let dur_s = duration.as_secs_f32();
        let shown = self.drag_pos.unwrap_or(pos);
        let screen_w = ctx.screen_rect().width();
        let show_side = screen_w >= 640.0;
        let wide = screen_w >= 820.0;
        let current_sp_id = self.now.as_ref().and_then(|t| t.spotify_id.clone());
        let mut dismiss_error = false;

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
                        if buffering { "Buffering..." } else { artist.as_str() },
                        FontId::proportional(12.0),
                        sub,
                    );
                }

                // Transport (center).
                let ctrl = Rect::from_center_size(
                    pos2(center.center().x, full.min.y + 30.0),
                    vec2(300.0, 42.0),
                );
                ui.allocate_ui_at_rect(ctrl, |ui| {
                    ui.horizontal_centered(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        if pill(ui, "Shuffle", self.cfg.shuffle, accent, 72.0).clicked() {
                            actions.push(Action::Shuffle);
                        }
                        if icon_button(ui, Icon::Prev, 34.0, text, None).clicked() {
                            actions.push(Action::Prev);
                        }
                        let icon = if playing { Icon::Pause } else { Icon::Play };
                        if icon_button(ui, icon, 42.0, bg, Some(text)).clicked() {
                            actions.push(Action::Toggle);
                        }
                        if icon_button(ui, Icon::Next, 34.0, text, None).clicked() {
                            actions.push(Action::Next);
                        }
                        let rep = match self.cfg.repeat {
                            Repeat::Off => "Repeat",
                            Repeat::All => "Repeat all",
                            Repeat::One => "Repeat one",
                        };
                        if pill(ui, rep, self.cfg.repeat != Repeat::Off, accent, 72.0).clicked() {
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
                        pos2(full.max.x - side_w, full.center().y - 11.0),
                        vec2(side_w, 22.0),
                    );
                    ui.allocate_ui_at_rect(vr, |ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let (resp, f) = bar(ui, 110.0, self.cfg.volume, accent, track_col, true);
                            if resp.dragged() || resp.clicked() {
                                actions.push(Action::Volume(f));
                            }
                            ui.label(egui::RichText::new("Vol").size(11.5).color(sub));
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
                    ui.add_space(12.0);
                    ui.label(egui::RichText::new("litewave").size(22.0).strong().color(accent));
                    ui.add_space(14.0);
                    if nav_button(ui, "Your Library", self.view == View::Library, accent).clicked() {
                        actions.push(Action::Library);
                    }
                    if nav_button(ui, "iPod mode", false, accent).clicked() {
                        actions.push(Action::SetIpod(true));
                    }

                    ui.add_space(14.0);
                    ui.label(egui::RichText::new("SPOTIFY").size(11.0).color(sub));
                    ui.add_space(4.0);
                    if self.sp.is_some() {
                        let root = self.sp_nav.first().map(|p| p.node.clone());
                        let in_sp = self.view == View::Spotify;
                        for (label, node) in [
                            ("Liked Songs", Node::SpLiked),
                            ("Playlists", Node::SpPlaylists),
                            ("Albums", Node::SpAlbums),
                            ("Artists", Node::SpArtists),
                        ] {
                            let active = in_sp && root.as_ref() == Some(&node);
                            if nav_button(ui, label, active, accent).clicked() {
                                actions.push(Action::SpRoot(node));
                            }
                        }
                    } else if nav_button(ui, "Connect Spotify", false, accent).clicked() {
                        actions.push(Action::SpotifyWindow);
                    }

                    ui.add_space(14.0);
                    ui.label(egui::RichText::new("MUSIC").size(11.0).color(sub));
                    ui.add_space(4.0);
                    if nav_button(ui, "Open folder", false, accent).clicked() {
                        actions.push(Action::OpenFolder);
                    }
                    if nav_button(ui, "Add songs", false, accent).clicked() {
                        actions.push(Action::AddFiles);
                    }
                    if nav_button(ui, "Rescan", false, accent).clicked() {
                        actions.push(Action::Rescan);
                    }
                    ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                        ui.add_space(10.0);
                        if nav_button(ui, "Themes", false, accent).clicked() {
                            actions.push(Action::Themes);
                        }
                        if nav_button(ui, "Spotify account", false, accent).clicked() {
                            actions.push(Action::SpotifyWindow);
                        }
                    });
                });
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

            if self.view == View::Spotify && self.sp.is_some() {
                self.spotify_view(ui, actions, accent, text, sub, playing, wide, &current_sp_id);
                return;
            }

            ui.horizontal(|ui| {
                let heading = if self.list_is_local { "Your Library" } else { "Playing from" };
                ui.label(egui::RichText::new(heading).size(24.0).strong());
                let n = self.filtered.len();
                let count = if self.scan_rx.is_some() {
                    format!("Scanning... {}", self.scan_count)
                } else {
                    format!("{} track{}", n, if n == 1 { "" } else { "s" })
                };
                ui.label(egui::RichText::new(count).size(13.0).color(sub));
            });
            ui.add_space(8.0);
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .hint_text("Search songs, artists, albums")
                    .font(FontId::proportional(15.0))
                    .desired_width(f32::INFINITY),
            );
            self.search_focus = r.has_focus();
            if let Some(err) = &self.error {
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(Color32::LIGHT_RED, err);
                    if ui.small_button("dismiss").clicked() {
                        dismiss_error = true;
                    }
                });
            }
            ui.add_space(8.0);

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
                            );
                            if resp.clicked() {
                                actions.push(Action::Play(i));
                            }
                        }
                    });
            });
        });

        if dismiss_error {
            self.error = None;
        }
    }

    /// Spotify browsing in the main window: playlists, albums, artists and track lists.
    #[allow(clippy::too_many_arguments)]
    fn spotify_view(
        &mut self,
        ui: &mut egui::Ui,
        actions: &mut Vec<Action>,
        accent: Color32,
        text: Color32,
        sub: Color32,
        playing: bool,
        wide: bool,
        current_sp_id: &Option<String>,
    ) {
        let Some(page) = self.sp_nav.last() else {
            ui.label(egui::RichText::new("Pick something from the sidebar").color(sub));
            return;
        };

        ui.horizontal(|ui| {
            if self.sp_nav.len() > 1 && ui.button("Back").clicked() {
                actions.push(Action::SpBack);
            }
            ui.label(egui::RichText::new(&page.title).size(24.0).strong());
        });
        ui.add_space(4.0);
        let r = ui.add(
            egui::TextEdit::singleline(&mut self.sp_link)
                .hint_text("Paste a Spotify playlist, album or artist link and press Enter")
                .desired_width(f32::INFINITY),
        );
        self.search_focus = r.has_focus();
        if r.lost_focus()
            && ui.input(|i| i.key_pressed(egui::Key::Enter))
            && !self.sp_link.trim().is_empty()
        {
            actions.push(Action::SpLink(self.sp_link.trim().to_string()));
            self.sp_link.clear();
        }
        if let Some(err) = &self.error {
            ui.colored_label(Color32::LIGHT_RED, err);
        }
        ui.add_space(8.0);

        match &page.load {
            Load::Loading => {
                ui.add_space(16.0);
                ui.label(egui::RichText::new("Loading...").color(sub));
            }
            Load::Failed(e) => {
                ui.add_space(12.0);
                ui.colored_label(Color32::LIGHT_RED, e);
            }
            Load::Ready => {
                let is_tracks = !page.tracks.is_empty();
                let n = if is_tracks { page.tracks.len() } else { page.items.len() };
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
                                        let resp = track_row(
                                            ui,
                                            &mut self.art,
                                            i,
                                            t,
                                            selected,
                                            selected && playing,
                                            wide,
                                            accent,
                                            text,
                                            sub,
                                        );
                                        if resp.clicked() {
                                            actions.push(Action::SpPlay(i));
                                        }
                                    } else {
                                        let it = &page.items[i];
                                        let (rect, resp) = ui.allocate_exact_size(
                                            vec2(ui.available_width(), ROW_H),
                                            Sense::click(),
                                        );
                                        let tex = it
                                            .art
                                            .clone()
                                            .and_then(|u| self.art.get(ui.ctx(), &ArtKey::Url(u), art::THUMB));
                                        if resp.hovered() {
                                            ui.painter().rect_filled(
                                                rect,
                                                8.0,
                                                ui.visuals().widgets.hovered.weak_bg_fill,
                                            );
                                        }
                                        let cy = rect.center().y;
                                        let p = ui.painter();
                                        let cover = Rect::from_center_size(
                                            pos2(rect.min.x + 30.0, cy),
                                            vec2(38.0, 38.0),
                                        );
                                        paint_cover(p, cover, tex, &it.label);
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
                                        if resp.clicked() {
                                            if let Kind::Open(node) = &it.kind {
                                                actions.push(Action::SpPush(node.clone()));
                                            }
                                        }
                                    }
                                }
                            });
                    });
                });
            }
        }
    }
}
