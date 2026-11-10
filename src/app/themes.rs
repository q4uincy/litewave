//! The Themes window: presets, wallpaper, colors, and saving/sharing themes.

use super::App;
use crate::art::{self, ArtKey};
use crate::theme::{self, Theme};
use eframe::egui::{self, pos2, vec2, Align2, FontId, Rect, Sense, Stroke};

impl App {
    /// Floating Themes window (only used in iPod mode; the classic layout has a Themes page).
    pub(super) fn themes_window(&mut self, ctx: &egui::Context) {
        if !self.themes_open {
            if self.view != super::View::Themes {
                self.theme_focus = false;
            }
            return;
        }
        let mut open = true;
        egui::Window::new("Themes")
            .open(&mut open)
            .default_width(380.0)
            .collapsible(false)
            .show(ctx, |ui| {
                self.themes_ui(ctx, ui);
            });
        if !open {
            self.themes_open = false;
        }
    }

    /// The theme gallery (Themes page only): previews with a Download button.
    pub(super) fn theme_gallery_ui(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        ui.add_space(18.0);
        ui.separator();
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Theme gallery").size(18.0).strong());
            let label = if self.store_items.is_empty() { "Load gallery" } else { "Refresh" };
            if ui.add_enabled(!self.store_busy, egui::Button::new(label)).clicked() {
                self.store_load();
            }
        });
        if !self.store_note.is_empty() {
            ui.weak(self.store_note.clone());
        }
        ui.add_space(8.0);
        let mut install = None;
        let items = self.store_items.clone();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(12.0, 12.0);
            for item in items.iter() {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(220.0);
                    let size = vec2(220.0, 165.0);
                    let tex = if item.preview.starts_with("https://") {
                        self.art.get(ctx, &ArtKey::UrlWide(item.preview.clone()), art::BIG)
                    } else {
                        None
                    };
                    match tex {
                        Some(id) => {
                            ui.add(egui::Image::new(egui::load::SizedTexture::new(id, size)).rounding(8.0));
                        }
                        None => {
                            let (r, _) = ui.allocate_exact_size(size, Sense::hover());
                            ui.painter().rect_filled(r, 8.0, ui.visuals().faint_bg_color);
                        }
                    }
                    ui.label(egui::RichText::new(&item.name).strong());
                    if !item.author.is_empty() {
                        ui.weak(format!("by {}", item.author));
                    }
                    if !item.description.is_empty() {
                        ui.label(egui::RichText::new(&item.description).size(12.0));
                    }
                    if ui.add_enabled(!self.store_busy, egui::Button::new("Download")).clicked() {
                        install = Some(item.clone());
                    }
                });
            }
        });
        if let Some(i) = install {
            self.store_install(i);
        }
    }

    /// The Themes controls, shared by the floating window and the Themes page.
    pub(super) fn themes_ui(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        if self.theme_name.is_empty() {
            self.theme_name = self.cfg.theme.name.clone();
        }

        let mut changed = false;
        let mut pick: Option<Theme> = None;
        let mut choose_bg = false;
        let mut clear_bg = false;
        let mut save = false;
        let mut open_dir = false;
        let mut reload = false;
        let mut focus = false;
        let mut new_quality: Option<u8> = None;

        {
            {
                ui.label("Presets");
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    for t in self.theme_list.iter() {
                        let (rect, resp) = ui.allocate_exact_size(vec2(84.0, 54.0), Sense::click());
                        let p = ui.painter();
                        p.rect_filled(rect, 8.0, t.bg());
                        let inner = Rect::from_min_max(
                            pos2(rect.min.x + 6.0, rect.min.y + 6.0),
                            pos2(rect.max.x - 6.0, rect.min.y + 28.0),
                        );
                        p.rect_filled(inner, 5.0, t.panel());
                        p.circle_filled(pos2(inner.max.x - 10.0, inner.center().y), 5.0, t.accent());
                        p.text(
                            pos2(rect.center().x, rect.max.y - 12.0),
                            Align2::CENTER_CENTER,
                            &t.name,
                            FontId::proportional(11.0),
                            t.text(),
                        );
                        if t.name == self.cfg.theme.name {
                            p.rect_stroke(rect, 8.0, Stroke::new(2.0, t.accent()));
                        }
                        if resp.clicked() {
                            pick = Some(t.clone());
                        }
                        resp.on_hover_text(&t.name);
                    }
                });

                ui.separator();
                ui.label("Wallpaper");
                ui.horizontal(|ui| {
                    if ui.button("Choose image...").clicked() {
                        choose_bg = true;
                    }
                    if ui.button("Remove").clicked() {
                        clear_bg = true;
                    }
                });
                let bg_label = match &self.cfg.theme.background {
                    Some(p) => std::path::Path::new(p)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| p.clone()),
                    None => "None. You can also drop an image onto the window.".to_string(),
                };
                ui.weak(bg_label);
                ui.horizontal(|ui| {
                    ui.label("Quality");
                    for (i, label) in ["Low", "Medium", "High", "Ultra"].iter().enumerate() {
                        if ui
                            .selectable_label(self.cfg.wallpaper_quality as usize == i, *label)
                            .clicked()
                        {
                            new_quality = Some(i as u8);
                        }
                    }
                });
                ui.weak("RAM: Low ~4 MB, Medium ~8 MB, High ~15 MB, Ultra ~33 MB");
                if let Some(tex) = &self.bg_tex {
                    let [w, h] = tex.size();
                    let mut info = format!("Loaded at {w}x{h}.");
                    if (w as f32) < ctx.screen_rect().width() {
                        info.push_str(" Smaller than this window, so it will look a bit soft.");
                    }
                    ui.weak(info);
                }

                ui.separator();
                egui::Grid::new("theme_colors")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        for (label, which) in [("Accent", 0), ("Background", 1), ("Panels", 2), ("Text", 3)] {
                            ui.label(label);
                            let t = &mut self.cfg.theme;
                            let mut c = match which {
                                0 => t.accent(),
                                1 => t.bg(),
                                2 => t.panel(),
                                _ => t.text(),
                            };
                            if egui::color_picker::color_edit_button_srgba(
                                ui,
                                &mut c,
                                egui::color_picker::Alpha::Opaque,
                            )
                            .changed()
                            {
                                let h = theme::to_hex(c);
                                match which {
                                    0 => t.accent = h,
                                    1 => t.bg = h,
                                    2 => t.panel = h,
                                    _ => t.text = h,
                                }
                                t.subtext = theme::to_hex(theme::mix(t.text(), t.bg(), 0.5));
                                changed = true;
                            }
                            ui.end_row();
                        }
                    });

                ui.add_space(4.0);
                let mut opacity = self.cfg.theme.panel_alpha as f32 / 255.0 * 100.0;
                if ui
                    .add(egui::Slider::new(&mut opacity, 20.0..=100.0).text("Panel opacity %"))
                    .changed()
                {
                    self.cfg.theme.panel_alpha = (opacity / 100.0 * 255.0).round() as u8;
                    changed = true;
                }
                let mut dim = self.cfg.theme.dim as f32;
                if ui
                    .add(egui::Slider::new(&mut dim, 0.0..=230.0).text("Wallpaper dim"))
                    .changed()
                {
                    self.cfg.theme.dim = dim as u8;
                    changed = true;
                }
                let mut round = self.cfg.theme.rounding;
                if ui
                    .add(egui::Slider::new(&mut round, 0.0..=18.0).text("Roundness"))
                    .changed()
                {
                    self.cfg.theme.rounding = round;
                    changed = true;
                }

                ui.separator();
                ui.horizontal(|ui| {
                    let r = ui.add(egui::TextEdit::singleline(&mut self.theme_name).desired_width(140.0));
                    focus = r.has_focus();
                    if ui.button("Save theme").clicked() {
                        save = true;
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button("Open themes folder").clicked() {
                        open_dir = true;
                    }
                    if ui.button("Reload").clicked() {
                        reload = true;
                    }
                });
                ui.weak("Share a theme: send the .json (and its image) from the themes folder.");
            }
        }

        self.theme_focus = focus;

        if let Some(t) = pick {
            let keep = self.cfg.theme.background.clone();
            self.cfg.theme = t;
            if self.cfg.theme.background.is_none() {
                self.cfg.theme.background = keep;
            }
            self.theme_name = self.cfg.theme.name.clone();
            changed = true;
        }
        if choose_bg {
            if let Some(p) = rfd::FileDialog::new()
                .add_filter("Image", &["png", "jpg", "jpeg", "webp"])
                .pick_file()
            {
                self.cfg.theme.background = Some(p.to_string_lossy().into_owned());
                changed = true;
            }
        }
        if clear_bg {
            self.cfg.theme.background = None;
            changed = true;
        }
        if let Some(q) = new_quality {
            if q != self.cfg.wallpaper_quality {
                self.cfg.wallpaper_quality = q;
                self.bg_path = None;
                changed = true;
            }
        }
        if save {
            let mut t = self.cfg.theme.clone();
            t.name = self.theme_name.trim().to_string();
            if t.name.is_empty() {
                t.name = "My theme".to_string();
            }
            match theme::save_user_theme(&t) {
                Ok(saved) => {
                    self.cfg.theme = saved;
                    self.theme_list = theme::all_themes();
                    changed = true;
                }
                Err(e) => self.error = Some(format!("Could not save theme: {e}")),
            }
        }
        if reload {
            self.theme_list = theme::all_themes();
        }
        if open_dir {
            if let Some(dir) = theme::themes_dir() {
                theme::open_folder(&dir);
            }
        }
        if changed {
            self.theme_changed(ctx);
        }
    }
}
