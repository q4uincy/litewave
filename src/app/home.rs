//! Home screen (classic layout): filter chips, a grid of compact tiles built from
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

/// Spotify's own "made for you" playlists, recognised by name (the list does not keep owners).
fn is_made_for_you(name: &str) -> bool {
    let n = name.to_lowercase();
    n.starts_with("daily mix")
        || n.starts_with("discover weekly")
        || n.starts_with("release radar")
        || n.starts_with("on repeat")
        || n.starts_with("repeat rewind")
        || n.starts_with("time capsule")
        || n.starts_with("your top songs")
        || n.starts_with("your summer rewind")
        || n.starts_with("new music friday")
        || n.starts_with("this is ")
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

/// An album from Liked Songs with how many liked songs it has.
#[derive(Clone)]
struct LikedAlbum {
    id: String,
    name: String,
    artist: String,
    art: Option<String>,
}

/// Albums sorted by how many of the liked songs are on them (most first, ties by name).
/// Only the first `SHELF_MAX` are kept.
fn top_liked_albums(tracks: &[crate::library::Track]) -> Vec<LikedAlbum> {
    use std::collections::HashMap;
    let mut by_album: HashMap<&str, (usize, &crate::library::Track)> = HashMap::new();
    for t in tracks {
        let Some(id) = t.album_id.as_deref() else { continue };
        by_album.entry(id).and_modify(|e| e.0 += 1).or_insert((1, t));
    }
    let mut rows: Vec<(usize, &str, &crate::library::Track)> =
        by_album.into_iter().map(|(id, (n, t))| (n, id, t)).collect();
    rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.2.album.cmp(&b.2.album)));
    rows.into_iter()
        .take(SHELF_MAX)
        .map(|(_, id, t)| LikedAlbum {
            id: id.to_string(),
            name: t.album.clone(),
            artist: t.artist.clone(),
            art: t.art_url.clone(),
        })
        .collect()
}

impl App {
    fn cached_items(&self, key: &str) -> Vec<Item> {
        self.page_cache
            .get(key)
            .map(|(_, items, _)| items.iter().take(SHELF_MAX + 70).cloned().collect())
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
        for n in [Node::SpPlaylists, Node::SpAlbums, Node::SpArtists, Node::SpShows, Node::SpLiked] {
            self.want_list(n);
        }
        let base = ui.visuals().panel_fill;
        let playlists = self.cached_items("SpPlaylists");
        let albums = self.cached_items("SpAlbums");
        let artists = self.cached_items("SpArtists");
        let shows = self.cached_items("SpShows");

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
                    sub: match kind {
                        "Album" if !it.sub.is_empty() => it.sub.clone(),
                        "Playlist" if !it.sub.is_empty() => format!("Playlist \u{2022} {}", it.sub),
                        _ => kind.to_string(),
                    },
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
        // "Albums featuring songs you like": the albums that have the most songs in Liked Songs.
        // Counted once per change of the Liked Songs list, not on every frame.
        let liked_albums: Vec<Card> = {
            let id = egui::Id::new("liked_albums");
            let count = self.page_cache.get("SpLiked").map_or(0, |(_, _, t)| t.len());
            let memo: Option<(usize, Vec<LikedAlbum>)> = ui.data(|d| d.get_temp(id));
            let rows = match memo {
                Some((n, rows)) if n == count => rows,
                _ => {
                    let rows = self
                        .page_cache
                        .get("SpLiked")
                        .map(|(_, _, tracks)| top_liked_albums(tracks))
                        .unwrap_or_default();
                    ui.data_mut(|d| d.insert_temp(id, (count, rows.clone())));
                    rows
                }
            };
            rows.into_iter()
                .take(per_row)
                .map(|a| Card {
                    title: a.name.clone(),
                    sub: a.artist,
                    art: a.art.map(ArtKey::Url),
                    circle: false,
                    go: Click::Open(Node::SpAlbum(a.id, a.name)),
                })
                .collect()
        };
        let user = self.sp_user.clone();
        let mixes: Vec<Item> = playlists
            .iter()
            .filter(|it| {
                let n = it.label.to_lowercase();
                n.ends_with(" mix") && !n.starts_with("daily mix")
            })
            .cloned()
            .collect();
        let made: Vec<Item> = playlists.iter().filter(|it| is_made_for_you(&it.label)).cloned().collect();
        let shelves: Vec<(&str, Option<Node>, Vec<Card>)> = vec![
            ("Recents", None, recent),
            ("Made for you", Some(Node::SpPlaylists), from_items(&made, "Made for you", false)),
            ("Your playlists", Some(Node::SpPlaylists), from_items(&playlists, "Playlist", false)),
            ("Your albums", Some(Node::SpAlbums), from_items(&albums, "Album", false)),
            ("Albums featuring songs you like", None, liked_albums),
            ("Your top mixes", Some(Node::SpPlaylists), from_items(&mixes, "Made for you", false)),
            ("Your artists", Some(Node::SpArtists), from_items(&artists, "Artist", true)),
            ("Your podcasts", Some(Node::SpShows), from_items(&shows, "Podcast", false)),
        ];

        let mut clicked: Option<Click> = None;
        egui::ScrollArea::vertical()
            .id_source("home_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(TILE_GAP, TILE_GAP);
                ui.add_space(8.0);
                let fid = egui::Id::new("home_filter");
                let mut filter: u8 = ui.data(|d| d.get_temp::<u8>(fid)).unwrap_or(0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    for (i, label) in ["All", "Music", "Podcasts"].iter().enumerate() {
                        let on = filter == i as u8;
                        let chip = egui::Button::new(
                            egui::RichText::new(*label).size(13.5).strong().color(if on { base } else { text }),
                        )
                        .fill(if on { text } else { mix(base, text, 0.12) })
                        .rounding(18.0)
                        .min_size(vec2(0.0, 32.0));
                        if ui.add(chip).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            filter = i as u8;
                        }
                    }
                });
                ui.data_mut(|d| d.insert_temp(fid, filter));
                ui.add_space(6.0);

                let w = ((ui.available_width() - TILE_GAP * (cols as f32 - 1.0)) / cols as f32).max(80.0);
                for row in tiles.chunks(cols).filter(|_| filter != 2) {
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
                    let podcast = *title == "Your podcasts";
                    if cards.is_empty() || (filter == 1 && podcast) || (filter == 2 && !podcast) {
                        continue;
                    }
                    ui.add_space(14.0);
                    let named = *title == "Made for you" && !user.is_empty();
                    if named {
                        ui.label(egui::RichText::new("Made For").size(12.0).color(sub));
                    }
                    ui.horizontal(|ui| {
                        let head: &str = if named { user.as_str() } else { *title };
                        ui.label(egui::RichText::new(head).size(22.0).strong().color(text));
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
