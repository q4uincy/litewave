//! The Settings page: one scrolling page of sections, each a rounded card of
//! "label + description ... control" rows (like Spotify's own settings).

use super::{Action, App, BUG_URL, REPO_URL};
use crate::config::Config;
use crate::{spotify, theme};
use eframe::egui::{self, Align, Color32, Layout, RichText};

/// A pill-shaped on/off switch.
fn toggle(ui: &mut egui::Ui, on: &mut bool, accent: Color32) -> egui::Response {
    let (rect, mut resp) = ui.allocate_exact_size(egui::vec2(40.0, 22.0), egui::Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool(resp.id, *on);
    let off = ui.visuals().widgets.hovered.bg_fill;
    let fill = off.lerp_to_gamma(accent, t);
    let x = egui::lerp((rect.left() + 11.0)..=(rect.right() - 11.0), t);
    let p = ui.painter();
    p.rect_filled(rect, 11.0, fill);
    p.circle_filled(egui::pos2(x, rect.center().y), 8.0, Color32::WHITE);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// One setting: title and a short description on the left, the control on the right.
fn row(ui: &mut egui::Ui, title: &str, desc: &str, add: impl FnOnce(&mut egui::Ui)) {
    let weak = ui.visuals().weak_text_color();
    ui.horizontal(|ui| {
        let w = ui.available_width();
        ui.vertical(|ui| {
            ui.set_max_width((w - 230.0).max(140.0));
            ui.label(RichText::new(title).size(14.5));
            if !desc.is_empty() {
                ui.label(RichText::new(desc).size(12.0).color(weak));
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), add);
    });
}

fn sep(ui: &mut egui::Ui) {
    ui.add_space(4.0);
    ui.separator();
    ui.add_space(4.0);
}

/// A heading followed by a card that holds the rows.
fn section(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(24.0);
    ui.label(RichText::new(title).size(20.0).strong());
    ui.add_space(8.0);
    let fill = ui.visuals().widgets.inactive.bg_fill;
    egui::Frame::none()
        .fill(fill)
        .rounding(12.0)
        .inner_margin(egui::Margin::symmetric(18.0, 10.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            body(ui);
        });
}

impl App {
    pub(crate) fn settings_page(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>) {
        let accent = self.cfg.theme.accent();
        let weak = ui.visuals().weak_text_color();
        let ctx = ui.ctx().clone();
        let connected = self.sp.is_some();
        let mut dirty = false;
        let mut quality_changed = false;
        let mut reset = false;

        ui.set_max_width(760.0);
        ui.add_space(6.0);
        ui.label(RichText::new("Settings").size(30.0).strong());

        // ---- account -------------------------------------------------------
        section(ui, "Account", |ui| {
            row(
                ui,
                "Spotify",
                if connected { "Connected" } else { "Not connected" },
                |ui| {
                    let label = if connected { "Manage" } else { "Connect" };
                    if ui.button(label).clicked() {
                        actions.push(Action::SpotifyWindow);
                    }
                },
            );
            sep(ui);
            row(
                ui,
                "Show litewave in Spotify Connect",
                "Your phone and other Spotify apps can see this player and control it. Applies the next time Spotify connects.",
                |ui| {
                    if toggle(ui, &mut self.cfg.spotify_connect, accent).changed() {
                        dirty = true;
                    }
                },
            );
            sep(ui);
            row(
                ui,
                "Allow explicit content",
                "When off, explicit songs are greyed out and skipped.",
                |ui| {
                    if toggle(ui, &mut self.cfg.allow_explicit, accent).changed() {
                        dirty = true;
                    }
                },
            );
        });

        // ---- playback ------------------------------------------------------
        section(ui, "Playback", |ui| {
            row(
                ui,
                "Streaming quality",
                "Spotify songs. Higher quality uses more data. Applies the next time Spotify connects.",
                |ui| {
                    let names = ["Low (96 kbps)", "Normal (160 kbps)", "High (320 kbps)"];
                    let cur = (self.cfg.spotify_quality as usize).min(2);
                    egui::ComboBox::from_id_source("set_quality")
                        .selected_text(names[cur])
                        .show_ui(ui, |ui| {
                            for (i, n) in names.iter().enumerate() {
                                if ui.selectable_label(cur == i, *n).clicked() {
                                    self.cfg.spotify_quality = i as u8;
                                    quality_changed = true;
                                }
                            }
                        });
                },
            );
            sep(ui);
            row(
                ui,
                "Normalize volume",
                "Keeps Spotify songs at a similar loudness.",
                |ui| {
                    if toggle(ui, &mut self.cfg.spotify_normalise, accent).changed() {
                        quality_changed = true;
                    }
                },
            );
            sep(ui);
            row(
                ui,
                "Gapless playback",
                "No silence between songs on Spotify.",
                |ui| {
                    if toggle(ui, &mut self.cfg.spotify_gapless, accent).changed() {
                        quality_changed = true;
                    }
                },
            );
            sep(ui);
            row(
                ui,
                "Show lyrics",
                "Synced lyrics for the playing song (local .lrc files or LRCLIB).",
                |ui| {
                    if toggle(ui, &mut self.cfg.show_lyrics, accent).changed() {
                        dirty = true;
                    }
                },
            );
            sep(ui);
            row(
                ui,
                "Sleep timer",
                "Pauses playback after a while.",
                |ui| {
                    let label = self.sleep_label();
                    let mut pick: Option<u32> = None;
                    egui::ComboBox::from_id_source("set_sleep")
                        .selected_text(label)
                        .show_ui(ui, |ui| {
                            for (m, n) in [
                                (0u32, "Off"),
                                (15, "15 minutes"),
                                (30, "30 minutes"),
                                (45, "45 minutes"),
                                (60, "1 hour"),
                                (90, "90 minutes"),
                            ] {
                                if ui.selectable_label(self.sleep_mins == m, n).clicked() {
                                    pick = Some(m);
                                }
                            }
                        });
                    if let Some(m) = pick {
                        self.set_sleep(m);
                    }
                },
            );
        });

        // ---- display -------------------------------------------------------
        section(ui, "Display", |ui| {
            row(
                ui,
                "Theme",
                &format!("Current: {}", self.cfg.theme.name),
                |ui| {
                    if ui.button("Browse themes").clicked() {
                        actions.push(Action::Themes);
                    }
                },
            );
            sep(ui);
            row(
                ui,
                "Add your own theme",
                "Drop a .json theme file (and its wallpaper) on the window, or copy it into the themes folder and press Reload on the Themes page.",
                |ui| {
                    if ui.button("Open folder").clicked() {
                        if let Some(dir) = theme::themes_dir() {
                            theme::open_folder(&dir);
                        }
                    }
                },
            );
            sep(ui);
            row(ui, "Interface size", "Scales the whole window.", |ui| {
                let mut pct = (self.cfg.ui_scale * 100.0).round();
                let r = ui.add(
                    egui::Slider::new(&mut pct, 80.0..=150.0)
                        .suffix("%")
                        .step_by(5.0),
                );
                self.cfg.ui_scale = pct / 100.0;
                // Apply once the slider is let go, so the layout does not jump while dragging.
                if r.drag_stopped() || (r.changed() && !r.dragged()) {
                    ctx.set_zoom_factor(self.cfg.ui_scale.clamp(0.8, 1.5));
                    dirty = true;
                }
            });
            sep(ui);
            row(
                ui,
                "iPod mode",
                "Switch to the small click-wheel player.",
                |ui| {
                    if ui.button("Switch").clicked() {
                        actions.push(Action::SetIpod(true));
                    }
                },
            );
        });

        // ---- library -------------------------------------------------------
        section(ui, "Your library", |ui| {
            let folder = self
                .cfg
                .last_folder
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "No folder chosen yet".to_string());
            row(ui, "Music folder", &folder, |ui| {
                if ui.button("Change").clicked() {
                    actions.push(Action::OpenFolder);
                }
            });
            sep(ui);
            row(ui, "Add songs", "Add single files to the library.", |ui| {
                if ui.button("Add").clicked() {
                    actions.push(Action::AddFiles);
                }
            });
            sep(ui);
            row(ui, "Rescan library", "Look for new, moved or removed files.", |ui| {
                if ui.button("Rescan").clicked() {
                    actions.push(Action::Rescan);
                }
            });
            sep(ui);
            row(
                ui,
                "Clear recent searches",
                "Forget the searches shown under the Spotify search box.",
                |ui| {
                    let empty = self.cfg.recent_searches.is_empty();
                    if ui.add_enabled(!empty, egui::Button::new("Clear")).clicked() {
                        self.cfg.recent_searches.clear();
                        dirty = true;
                    }
                },
            );
        });

        // ---- updates -------------------------------------------------------
        section(ui, "Updates", |ui| {
            let st = crate::update::status();
            row(
                ui,
                &format!("litewave {}", env!("CARGO_PKG_VERSION")),
                if st.is_empty() { "Check whether a newer version is out." } else { &st },
                |ui| {
                    if ui.button("Check for updates").clicked() {
                        crate::update::request_check();
                    }
                },
            );
            sep(ui);
            row(
                ui,
                "Check for updates on start",
                "Looks for a new version each time litewave opens (Windows).",
                |ui| {
                    if toggle(ui, &mut self.cfg.auto_update_check, accent).changed() {
                        dirty = true;
                    }
                },
            );
            sep(ui);
            ui.label(
                RichText::new(
                    "\"Update now\" installs the new version and restarts litewave. It only works with the installer version; with the portable zip, download the new zip instead.",
                )
                .size(12.0)
                .color(weak),
            );
            ui.hyperlink_to("All releases on GitHub", format!("{REPO_URL}/releases"));
        });

        // ---- about ---------------------------------------------------------
        section(ui, "About", |ui| {
            row(ui, "litewave", "A tiny, fast, native music player.", |ui| {
                ui.hyperlink_to("GitHub", REPO_URL);
            });
            sep(ui);
            row(ui, "Something broken?", "Tell us what happened.", |ui| {
                ui.hyperlink_to("Report a bug", BUG_URL);
            });
            sep(ui);
            row(
                ui,
                "Reset settings",
                "Back to defaults. Your Spotify sign-in, music folder and volume are kept.",
                |ui| {
                    let id = egui::Id::new("settings_reset_armed");
                    let armed = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
                    let label = if armed { "Click again to confirm" } else { "Reset" };
                    if ui.button(label).clicked() {
                        if armed {
                            reset = true;
                            ui.data_mut(|d| d.insert_temp(id, false));
                        } else {
                            ui.data_mut(|d| d.insert_temp(id, true));
                        }
                    }
                },
            );
            if !spotify::ENABLED {
                sep(ui);
                ui.label(
                    RichText::new("This build has no Spotify support.")
                        .size(12.0)
                        .color(weak),
                );
            }
        });
        ui.add_space(30.0);

        if reset {
            let old = std::mem::take(&mut self.cfg);
            let mut fresh = Config::default();
            fresh.last_folder = old.last_folder;
            fresh.volume = old.volume;
            fresh.spotify_client_id = old.spotify_client_id;
            fresh.spotify_refresh = old.spotify_refresh;
            fresh.spotify_browse_refresh = old.spotify_browse_refresh;
            self.cfg = fresh;
            ctx.set_zoom_factor(self.cfg.ui_scale);
            self.theme_changed(&ctx);
            quality_changed = true;
        }
        if quality_changed {
            spotify::set_playback_options(
                self.cfg.spotify_quality,
                self.cfg.spotify_normalise,
                self.cfg.spotify_gapless,
            );
            dirty = true;
        }
        if dirty {
            self.cfg_dirty = true;
        }
    }
}
