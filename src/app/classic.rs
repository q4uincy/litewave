//! The main (non-iPod) window: sidebar, track list and a Spotify-style player bar.

use super::widgets::{bar, icon_button, initial, nav_button, pill, tint_for, Icon};
use super::{fmt_time, Action, App};
use crate::config::Repeat;
use crate::theme::mix;
use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Rect, Sense};

const ROW_H: f32 = 52.0;

impl App {
    pub(super) fn classic_ui(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let accent = self.cfg.theme.accent();
        let text = self.cfg.theme.text();
        let sub = self.cfg.theme.subtext();
        let bg = self.cfg.theme.bg();
        let track_col = mix(self.cfg.theme.panel(), text, 0.25);

        let (title, artist, duration) = self
            .current
            .and_then(|i| self.tracks.get(i))
            .map(|t| (t.title.clone(), t.artist.clone(), t.duration))
            .unwrap_or_default();
        let pos = self.position_secs();
        let playing = self.player.as_ref().map_or(false, |p| p.is_playing());
        let loaded = self.player.as_ref().map_or(false, |p| p.is_loaded());
        let dur_s = duration.as_secs_f32();
        let shown = self.drag_pos.unwrap_or(pos);
        let screen_w = ctx.screen_rect().width();
        let show_side = screen_w >= 640.0;
        let wide = screen_w >= 820.0;
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
                    p.rect_filled(cover, 8.0, tint_for(&title));
                    p.text(
                        cover.center(),
                        Align2::CENTER_CENTER,
                        initial(&title),
                        FontId::proportional(22.0),
                        Color32::from_white_alpha(200),
                    );
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
                        &artist,
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
                            let (resp, f) =
                                bar(ui, 110.0, self.cfg.volume, accent, track_col, true);
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
                    if nav_button(ui, "Your Library", self.list_is_local, accent).clicked() {
                        actions.push(Action::Library);
                    }
                    if nav_button(ui, "iPod mode", false, accent).clicked() {
                        actions.push(Action::SetIpod(true));
                    }
                    if nav_button(ui, "Spotify", false, accent).clicked() {
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
                    });
                });
        }

        // ---- library -----------------------------------------------------
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(6.0);
            if !show_side {
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Open folder").clicked() {
                        actions.push(Action::OpenFolder);
                    }
                    if ui.button("Add songs").clicked() {
                        actions.push(Action::AddFiles);
                    }
                    if ui.button("Rescan").clicked() {
                        actions.push(Action::Rescan);
                    }
                    if ui.button("iPod").clicked() {
                        actions.push(Action::SetIpod(true));
                    }
                    if ui.button("Themes").clicked() {
                        actions.push(Action::Themes);
                    }
                });
                ui.add_space(4.0);
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
                ui.horizontal(|ui| {
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
                            let (rect, resp) = ui.allocate_exact_size(
                                vec2(ui.available_width(), ROW_H),
                                Sense::click(),
                            );
                            let selected = Some(i) == self.current;
                            if selected {
                                ui.painter().rect_filled(rect, 8.0, accent.gamma_multiply(0.16));
                            } else if resp.hovered() {
                                ui.painter().rect_filled(
                                    rect,
                                    8.0,
                                    ui.visuals().widgets.hovered.weak_bg_fill,
                                );
                            }
                            let cy = rect.center().y;
                            let p = ui.painter();

                            // Index, or a play marker on the current track.
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
                                    format!("{}", row + 1),
                                    FontId::proportional(13.0),
                                    if selected { accent } else { sub },
                                );
                            }

                            // Cover placeholder.
                            let cover = Rect::from_center_size(pos2(rect.min.x + 64.0, cy), vec2(38.0, 38.0));
                            p.rect_filled(cover, 6.0, tint_for(&t.title));
                            p.text(
                                cover.center(),
                                Align2::CENTER_CENTER,
                                initial(&t.title),
                                FontId::proportional(16.0),
                                Color32::from_white_alpha(190),
                            );

                            // Title + artist.
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

                            // Album.
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

                            // Duration.
                            p.text(
                                pos2(rect.max.x - 14.0, cy),
                                Align2::RIGHT_CENTER,
                                fmt_time(t.duration.as_secs_f32()),
                                FontId::proportional(13.0),
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
}
