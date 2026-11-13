//! Home screen (classic layout): a time-of-day greeting, a grid of compact tiles built from
//! Liked Songs / playlists / albums, and shelves of cover cards. Everything reads the lists that
//! are already in `page_cache` (plus `App.recent`), covers are requested only for items that are
//! on screen, and every list is capped, so the extra memory is a handful of small textures.

use super::ipod::{Item, Kind, Node};
use super::widgets::{initial, paint_hub_tile, tint_for, HubIcon};
use super::{Action, App};
use crate::art::{self, ArtCache, ArtKey};
use crate::theme::mix;
use eframe::egui::{
    self, load::SizedTexture, pos2, vec2, Align2, Color32, FontId, Rect, Rounding, Sense, TextureId,
};

const TILE_H: f32 = 56.0;
const TILE_GAP: f32 = 14.0;
/// Card: 10 px padding around a 128 px cover (the `art::MID` size, so it is drawn 1:1).
const COVER: f32 = 128.0;
const PAD: f32 = 10.0;
const CARD_W: f32 = COVER + PAD * 2.0;
const CARD_H: f32 = 196.0;
const CARD_GAP: f32 = 12.0;
/// No shelf shows more cards than this, however wide the window is.
const SHELF_MAX: usize = 10;

fn greeting() -> &'static str {
    use chrono::Timelike;
    match chrono::Local::now().hour() {
        5..=11 => "Good morning",
        12..=17 => "Good afternoon",
        _ => "Good evening",
    }
}

/// Where a tile or card leads.
#[derive(Clone)]
enum Click {
    Open(Node),
    Recent(usize),
}

impl Click {
    fn action(&self) -> Action {
        match self {
            Click::Open(n) => Action::HomeOpen(n.clone()),
            Click::Recent(k) => Action::PlayRecent(*k),
        }
    }
}

struct Tile {
    name: String,
    art: Option<String>,
    liked: bool,
    go: Click,
}

struct Card {
    title: String,
    sub: String,
    art: Option<ArtKey>,
    circle: bool,
    go: Click,
}

/// Cover image (rounded as asked) or a colored placeholder with the first letter.
fn cover(ui: &mut egui::Ui, rect: Rect, tex: Option<TextureId>, title: &str, rounding: Rounding) {
    match tex {
        Some(id) => {
            ui.put(
                rect,
                egui::Image::new(SizedTexture::new(id, rect.size()))
                    .fit_to_exact_size(rect.size())
                    .rounding(rounding),
            );
        }
        None => {
            let p = ui.painter();
            p.rect_filled(rect, rounding, tint_for(title));
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

fn tile(ui: &mut egui::Ui, cache: &mut ArtCache, w: f32, t: &Tile, base: Color32, text: Color32) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(w, TILE_H), Sense::click());
    if !ui.is_rect_visible(rect) {
        return false;
    }
    let fill = if resp.hovered() { mix(base, text, 0.18) } else { mix(base, text, 0.10) };
    ui.painter().rect_filled(rect, 6.0, fill);
    let c = Rect::from_min_size(rect.min, vec2(TILE_H, TILE_H));
    if t.liked {
        paint_hub_tile(ui.painter(), c, HubIcon::Liked);
    } else {
        let tex = t.art.clone().and_then(|u| cache.get(ui.ctx(), &ArtKey::Url(u), art::THUMB));
        let r = Rounding { nw: 6.0, sw: 6.0, ne: 0.0, se: 0.0 };
        cover(ui, c, tex, &t.name, r);
    }
    ui.painter()
        .with_clip_rect(Rect::from_min_max(pos2(c.max.x + 12.0, rect.min.y), pos2(rect.max.x - 8.0, rect.max.y)))
        .text(
            pos2(c.max.x + 12.0, rect.center().y),
            Align2::LEFT_CENTER,
            &t.name,
            FontId::proportional(15.0),
            text,
        );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

fn card(ui: &mut egui::Ui, cache: &mut ArtCache, c: &Card, base: Color32, text: Color32, sub: Color32) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(CARD_W, CARD_H), Sense::click());
    if !ui.is_rect_visible(rect) {
        return false;
    }
    let fill = if resp.hovered() { mix(base, text, 0.10) } else { mix(base, text, 0.04) };
    ui.painter().rect_filled(rect, 8.0, fill);
    let cv = Rect::from_min_size(rect.min + vec2(PAD, PAD), vec2(COVER, COVER));
    let tex = c.art.as_ref().and_then(|k| cache.get(ui.ctx(), k, art::MID));
    let r = if c.circle { Rounding::same(COVER / 2.0) } else { Rounding::same(6.0) };
    cover(ui, cv, tex, &c.title, r);
    let tx = rect.min.x + PAD;
    let p = ui
        .painter()
        .with_clip_rect(Rect::from_min_max(pos2(tx, rect.min.y), pos2(rect.max.x - PAD, rect.max.y)));
    p.text(
        pos2(tx, cv.max.y + 12.0),
        Align2::LEFT_TOP,
        &c.title,
        FontId::proportional(14.5),
        text,
    );
    p.text(
        pos2(tx, cv.max.y + 32.0),
        Align2::LEFT_TOP,
        &c.sub,
        FontId::proportional(12.5),
        sub,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// Playlist / album / artist items from a cached list.
fn open_items(items: &[Item]) -> impl Iterator<Item = (&Item, &Node)> {
    items.iter().filter_map(|it| match &it.kind {
        Kind::Open(n) => Some((it, n)),
        _ => None,
    })
}

impl App {
    fn cached_items(&self, key: &str) -> Vec<Item> {
        self.page_cache
            .get(key)
            .map(|(_, items, _)| items.iter().take(SHELF_MAX + 12).cloned().collect())
            .unwrap_or_default()
    }

    /// The Home screen. Draws into the central panel.
    pub(super) fn home_view(
        &mut self,
        ui: &mut egui::Ui,
        actions: &mut Vec<Action>,
        _accent: Color32,
        text: Color32,
        sub: Color32,
    ) {
        for n in [Node::SpPlaylists, Node::SpAlbums, Node::SpArtists] {
            self.want_list(n);
        }
        let base = ui.visuals().panel_fill;
        let playlists = self.cached_items("SpPlaylists");
        let albums = self.cached_items("SpAlbums");
        let artists = self.cached_items("SpArtists");

        // Greeting tiles: Liked Songs first, then playlists and albums taking turns.
        let avail = ui.available_width();
        let (cols, rows) = if avail < 560.0 {
            (2usize, 4usize)
        } else if avail < 900.0 {
            (3, 3)
        } else {
            (4, 2)
        };
        let mut tiles = vec![Tile {
            name: "Liked Songs".to_string(),
            art: None,
            liked: true,
            go: Click::Open(Node::SpLiked),
        }];
        let mut pl = open_items(&playlists);
        let mut al = open_items(&albums);
        while tiles.len() < cols * rows {
            let next = if tiles.len() % 2 == 1 { pl.next().or_else(|| al.next()) } else { al.next().or_else(|| pl.next()) };
            let Some((it, n)) = next else { break };
            tiles.push(Tile { name: it.label.clone(), art: it.art.clone(), liked: false, go: Click::Open(n.clone()) });
        }

        // Shelves.
        let per_row = (((avail + CARD_GAP) / (CARD_W + CARD_GAP)).floor() as usize).clamp(1, SHELF_MAX);
        let from_items = |items: &[Item], kind: &str, circle: bool| -> Vec<Card> {
            open_items(items)
                .take(per_row)
                .map(|(it, n)| Card {
                    title: it.label.clone(),
                    sub: kind.to_string(),
                    art: it.art.clone().map(ArtKey::Url),
                    circle,
                    go: Click::Open(n.clone()),
                })
                .collect()
        };
        let recent: Vec<Card> = self
            .recent
            .iter()
            .enumerate()
            .take(per_row)
            .map(|(k, t)| Card {
                title: t.title.clone(),
                sub: t.artist.clone(),
                art: t.art_key(),
                circle: false,
                go: Click::Recent(k),
            })
            .collect();
        let shelves: Vec<(&str, Option<Node>, Vec<Card>)> = vec![
            ("Recently played", None, recent),
            ("Your playlists", Some(Node::SpPlaylists), from_items(&playlists, "Playlist", false)),
            ("Your albums", Some(Node::SpAlbums), from_items(&albums, "Album", false)),
            ("Your artists", Some(Node::SpArtists), from_items(&artists, "Artist", true)),
        ];

        let mut clicked: Option<Click> = None;
        egui::ScrollArea::vertical()
            .id_source("home_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(TILE_GAP, TILE_GAP);
                ui.add_space(4.0);
                ui.label(egui::RichText::new(greeting()).size(30.0).strong().color(text));
                ui.add_space(6.0);

                let w = ((ui.available_width() - TILE_GAP * (cols as f32 - 1.0)) / cols as f32).max(80.0);
                for row in tiles.chunks(cols) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = TILE_GAP;
                        for t in row {
                            if tile(ui, &mut self.art, w, t, base, text) {
                                clicked = Some(t.go.clone());
                            }
                        }
                    });
                }

                for (title, all, cards) in &shelves {
                    if cards.is_empty() {
                        continue;
                    }
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(*title).size(22.0).strong().color(text));
                        if let Some(n) = all {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                let r = ui
                                    .add(
                                        egui::Label::new(egui::RichText::new("Show all").size(13.0).strong().color(sub))
                                            .sense(Sense::click()),
                                    )
                                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                                if r.clicked() {
                                    clicked = Some(Click::Open(n.clone()));
                                }
                            });
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = CARD_GAP;
                        for c in cards {
                            if card(ui, &mut self.art, c, base, text, sub) {
                                clicked = Some(c.go.clone());
                            }
                        }
                    });
                }
                ui.add_space(16.0);
            });

        if let Some(c) = clicked {
            actions.push(c.action());
        }
    }
}
