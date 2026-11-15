//! Home screen (classic layout): filter chips, a grid of compact tiles built from
//! Liked Songs / playlists / albums, and shelves of cover cards. Everything reads the lists that
//! are already in `page_cache` (plus `App.recent`), covers are requested only for items that are
//! on screen, and every list is capped, so the extra memory is a handful of small textures.

use super::ipod::{Item, Kind, Node};
use super::widgets::{initial, paint_hub_tile, tint_for, HubIcon};
use super::{Action, App};
use crate::art::{self, ArtCache, ArtKey};
use crate::spotify::{ShelfCard, ShelfKind};
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

fn greeting() -> &'static str {
    use chrono::Timelike;
    match chrono::Local::now().hour() {
        5..=11 => "Good morning",
        12..=17 => "Good afternoon",
        _ => "Good evening",
    }
}

/// Spotify's own playlists (Daily Mix, Blends, Discover Weekly ...): owned by "Spotify", or
/// recognised by name when the owner is not known.
fn is_spotify_made(it: &Item) -> bool {
    it.sub == "Spotify" || is_made_for_you(&it.label)
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

/// One shelf on Home: Spotify's own (`real`, with its subtitle) or one built from the library.
struct HomeShelf {
    title: String,
    subtitle: String,
    /// "Show all" target, where there is one.
    all: Option<Node>,
    cards: Vec<Card>,
    real: bool,
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
            // Paint only: `ui.put` would move the layout cursor back to the end of the cover and
            // make the next tile or card overlap this one.
            egui::Image::new(SizedTexture::new(id, rect.size()))
                .fit_to_exact_size(rect.size())
                .rounding(rounding)
                .paint_at(ui, rect);
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

/// A card of one of Spotify's own shelves: the same look as the library cards, opening the same pages.
fn shelf_card(c: &ShelfCard) -> Card {
    let e = &c.entry;
    let (node, circle) = match c.kind {
        ShelfKind::Playlist => (Node::SpPlaylist(e.id.clone(), e.name.clone()), false),
        ShelfKind::Album => (Node::SpAlbum(e.id.clone(), e.name.clone()), false),
        ShelfKind::Artist => (Node::SpArtist(e.id.clone(), e.name.clone()), true),
        ShelfKind::Show => (Node::SpShow(e.id.clone(), e.name.clone()), false),
    };
    Card {
        title: e.name.clone(),
        sub: e.sub.clone(),
        art: e.art.clone().map(ArtKey::Url),
        circle,
        go: Click::Open(node),
    }
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
        // Spotify's own shelves (read quietly in the background; nothing is shown until they arrive).
        self.want_shelves();
        self.want_recent();
        let base = ui.visuals().panel_fill;
        let mut playlists = self.cached_items("SpPlaylists");
        let mut albums = self.cached_items("SpAlbums");
        // Spotify shows what you played last first; the sort is stable, so the rest keeps its order.
        let rank = |it: &Item| match &it.kind {
            Kind::Open(n) => self.recent_rank(n),
            _ => usize::MAX,
        };
        playlists.sort_by_key(|it| rank(it));
        albums.sort_by_key(|it| rank(it));
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
        // Spotify's own shelves come first, in Spotify's order and with Spotify's titles. The library
        // shelves follow, except the ones Spotify's cover: its Made for you / Your top mixes replace the
        // guessed ones, and its "Albums featuring songs you like" replaces the counted one.
        let real: Vec<(String, String, Vec<Card>)> = if self.cfg.home_shelves {
            self.sp_shelves
                .as_ref()
                .map(|(_, v)| {
                    v.iter()
                        .filter(|s| !s.cards.is_empty())
                        .map(|s| {
                            (
                                s.title.clone(),
                                s.subtitle.clone(),
                                s.cards.iter().take(per_row).map(shelf_card).collect::<Vec<Card>>(),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        // Titles Spotify's shelves already use: the library shelf of the same name is left out.
        let real_titles: Vec<String> = real.iter().map(|(t, _, _)| t.to_lowercase()).collect();
        let user = self.sp_user.clone();
        let mixes: Vec<Item> = playlists
            .iter()
            .filter(|it| {
                let n = it.label.to_lowercase();
                n.ends_with(" mix") && !n.starts_with("daily mix")
            })
            .cloned()
            .collect();
        let made: Vec<Item> = playlists.iter().filter(|it| is_spotify_made(it)).cloned().collect();
        let own: Vec<Item> = playlists.iter().filter(|it| !is_spotify_made(it)).cloned().collect();
        let entry_cards = |list: &[crate::spotify::Entry], album: bool| -> Vec<Card> {
            list.iter()
                .take(per_row)
                .map(|e| Card {
                    title: e.name.clone(),
                    sub: e.sub.clone(),
                    art: e.art.clone().map(ArtKey::Url),
                    circle: !album,
                    go: Click::Open(if album {
                        Node::SpAlbum(e.id.clone(), e.name.clone())
                    } else {
                        Node::SpArtist(e.id.clone(), e.name.clone())
                    }),
                })
                .collect()
        };
        let recent_albums = entry_cards(&self.sp_recent_albums, true);
        let top_artists = entry_cards(&self.sp_top_artists, false);
        let library: Vec<(&str, Option<Node>, Vec<Card>)> = vec![
            ("Recents", None, recent),
            ("Made for you", Some(Node::SpPlaylists), from_items(&made, "Made for you", false)),
            ("Your playlists", Some(Node::SpPlaylists), from_items(&own, "Playlist", false)),
            ("Your albums", Some(Node::SpAlbums), from_items(&albums, "Album", false)),
            ("Albums featuring songs you like", None, liked_albums),
            ("Your top mixes", Some(Node::SpPlaylists), from_items(&mixes, "Made for you", false)),
            ("Recently played", None, recent_albums),
            ("Your top artists", None, top_artists),
            ("Your artists", Some(Node::SpArtists), from_items(&artists, "Artist", true)),
            ("Your podcasts", Some(Node::SpShows), from_items(&shows, "Podcast", false)),
        ];
        let mut shelves: Vec<HomeShelf> = Vec::new();
        // Where litewave has a page for the same shelf ("Your playlists" ...), "Show all" opens it.
        let all_for = |t: &str| {
            library
                .iter()
                .find(|(lt, _, _)| lt.eq_ignore_ascii_case(t))
                .and_then(|(_, a, _)| a.clone())
        };
        for (title, subtitle, cards) in real {
            let all = all_for(&title);
            shelves.push(HomeShelf { title, subtitle, all, cards, real: true });
        }
        shelves.extend(
            library
                .into_iter()
                .filter(|(t, _, _)| !real_titles.contains(&t.to_lowercase()))
                .map(|(t, all, cards)| HomeShelf { title: t.to_string(), subtitle: String::new(), all, cards, real: false }),
        );

        let mut clicked: Option<Click> = None;
        egui::ScrollArea::vertical()
            .id_source("home_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(TILE_GAP, TILE_GAP);
                ui.add_space(6.0);
                ui.label(egui::RichText::new(greeting()).size(26.0).strong().color(text));
                ui.add_space(2.0);
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
                if self.cfg.home_shelves && self.sp.is_some() && self.sp_shelves.is_none() && !self.shelves_busy {
                    let why = if self.sp_session.is_none() {
                        "Made for you needs the playback sign-in (Premium). Sign in again under Spotify account."
                    } else if self.shelves_next.is_some() {
                        "Made for you could not load. The reason is in Settings > Help > open log (lines starting with \"home shelves\")."
                    } else {
                        ""
                    };
                    if !why.is_empty() {
                        ui.label(egui::RichText::new(why).size(12.0).color(sub));
                    }
                }
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

                for shelf in &shelves {
                    let (title, all, cards) = (&shelf.title, &shelf.all, &shelf.cards);
                    let podcast = title.as_str() == "Your podcasts" && !shelf.real;
                    if cards.is_empty() || (filter == 1 && podcast) || (filter == 2 && !podcast) {
                        continue;
                    }
                    ui.add_space(14.0);
                    let named = title.as_str() == "Made for you" && !shelf.real && !user.is_empty();
                    if named {
                        ui.label(egui::RichText::new("Made For").size(12.0).color(sub));
                    } else if !shelf.subtitle.is_empty() {
                        // Spotify's own small line above the title ("Brand new music from artists you love.").
                        ui.label(egui::RichText::new(shelf.subtitle.as_str()).size(12.0).color(sub));
                    }
                    ui.horizontal(|ui| {
                        let head: &str = if named { user.as_str() } else { title.as_str() };
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
