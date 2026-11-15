//! Home screen (classic layout): filter chips, a grid of compact tiles built from
//! Liked Songs / playlists / albums, and shelves of cover cards. Everything reads the lists that
//! are already in `page_cache` (plus `App.recent`), covers are requested only for items that are
//! on screen, and every list is capped, so the extra memory is a handful of small textures.

use super::ipod::{Item, Kind, Node};
use super::widgets::{initial, paint_hub_tile, tint_for, HubIcon};
use super::{fmt_time, Action, App};
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
const CARD_H: f32 = 212.0;
const ROW_H: f32 = 56.0;
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
    /// A song of "Your top songs": (period, row).
    Top(usize, usize),
    /// A song played on its own (place in Spotify's recent list).
    Song(usize),
    Like(String),
    Unlike(String),
    RetryShelves,
}

impl Click {
    fn action(&self) -> Action {
        match self {
            Click::Open(n) => Action::HomeOpen(n.clone()),
            Click::Recent(k) => Action::PlayRecent(*k),
            Click::Top(k, i) => Action::PlayTop(*k, *i),
            Click::Song(i) => Action::PlayPlay(*i),
            Click::Like(id) => Action::SpSave(vec![format!("spotify:track:{id}")]),
            Click::Unlike(id) => Action::SpUnsave(vec![format!("spotify:track:{id}")]),
            Click::RetryShelves => Action::RetryShelves,
        }
    }
}

struct Tile {
    name: String,
    art: Option<String>,
    liked: bool,
    go: Click,
}

/// Same page, same key: used so a tile is not shown twice.
fn node_key(n: &Node) -> String {
    match n {
        Node::SpPlaylist(id, _) => format!("playlist:{id}"),
        Node::SpAlbum(id, _) => format!("album:{id}"),
        Node::SpArtist(id, _) => format!("artist:{id}"),
        Node::SpShow(id, _) => format!("show:{id}"),
        other => format!("{other:?}"),
    }
}

/// The library item (playlist / artist / podcast) that opens page `kind:id`.
fn find_item<'a>(items: &'a [Item], key: &str) -> Option<(&'a Item, &'a Node)> {
    open_items(items).find(|(_, n)| node_key(n) == key)
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

/// Text laid out in `width` with at most `rows` lines; what does not fit ends in an ellipsis.
/// Returns the height used.
fn clipped(p: &egui::Painter, pos: egui::Pos2, text: &str, font: FontId, color: Color32, width: f32, rows: usize) -> f32 {
    let mut job = egui::text::LayoutJob::single_section(text.to_string(), egui::TextFormat::simple(font, color));
    job.wrap = egui::text::TextWrapping {
        max_width: width.max(8.0),
        max_rows: rows,
        break_anywhere: false,
        overflow_character: Some('\u{2026}'),
    };
    let galley = p.layout_job(job);
    let h = galley.size().y;
    p.galley(pos, galley, color);
    h
}

/// A cover card in the Spotify desktop style: no box until hovered, a title line and up to two
/// grey lines under it.
fn card(ui: &mut egui::Ui, cache: &mut ArtCache, c: &Card, base: Color32, text: Color32, sub: Color32) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(CARD_W, CARD_H), Sense::click());
    if !ui.is_rect_visible(rect) {
        return false;
    }
    if resp.hovered() {
        ui.painter().rect_filled(rect, 10.0, mix(base, text, 0.08));
    }
    let cv = Rect::from_min_size(rect.min + vec2(PAD, PAD), vec2(COVER, COVER));
    let tex = c.art.as_ref().and_then(|k| cache.get(ui.ctx(), k, art::MID));
    let r = if c.circle { Rounding::same(COVER / 2.0) } else { Rounding::same(8.0) };
    cover(ui, cv, tex, &c.title, r);
    let tx = rect.min.x + PAD;
    let w = CARD_W - PAD * 2.0;
    let p = ui.painter();
    let th = clipped(p, pos2(tx, cv.max.y + 10.0), &c.title, FontId::proportional(14.5), text, w, 1);
    clipped(p, pos2(tx, cv.max.y + 12.0 + th), &c.sub, FontId::proportional(12.5), sub, w, 2);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// A heart outline (filled when `on`) centred at `hc`.
fn heart(p: &egui::Painter, hc: egui::Pos2, on: bool, col: Color32) {
    let pts: Vec<egui::Pos2> = (0..32)
        .map(|i| {
            let t = i as f32 / 32.0 * std::f32::consts::TAU;
            let x = 16.0 * t.sin().powi(3);
            let y = 13.0 * t.cos() - 5.0 * (2.0 * t).cos() - 2.0 * (3.0 * t).cos() - (4.0 * t).cos();
            pos2(hc.x + x * 0.5, hc.y - y * 0.5 - 1.0)
        })
        .collect();
    if on {
        let mid = pos2(hc.x, hc.y - 1.0);
        for i in 0..pts.len() {
            let tri = vec![mid, pts[i], pts[(i + 1) % pts.len()]];
            p.add(egui::Shape::convex_polygon(tri, col, egui::Stroke::NONE));
        }
    }
    for i in 0..pts.len() {
        p.line_segment([pts[i], pts[(i + 1) % pts.len()]], egui::Stroke::new(1.5_f32, col));
    }
}

/// One row of "Your top songs": number, cover, title and artist, album, heart, length.
/// Returns (row clicked, heart clicked).
#[allow(clippy::too_many_arguments)]
fn song_row(
    ui: &mut egui::Ui,
    cache: &mut ArtCache,
    n: usize,
    t: &crate::library::Track,
    liked: bool,
    base: Color32,
    text: Color32,
    sub: Color32,
    accent: Color32,
) -> (bool, bool) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
    if !ui.is_rect_visible(rect) {
        return (false, false);
    }
    if resp.hovered() {
        ui.painter().rect_filled(rect, 8.0, mix(base, text, 0.08));
    }
    let cy = rect.center().y;
    let wide = rect.width() > 640.0;
    let p = ui.painter();
    p.text(pos2(rect.min.x + 26.0, cy), Align2::CENTER_CENTER, n.to_string(), FontId::proportional(14.0), sub);
    let cv = Rect::from_center_size(pos2(rect.min.x + 44.0 + 20.0, cy), vec2(40.0, 40.0));
    let tex = t.art_key().and_then(|k| cache.get(ui.ctx(), &k, art::THUMB));
    cover(ui, cv, tex, &t.title, Rounding::same(4.0));
    let tx = cv.max.x + 12.0;
    let right = rect.max.x - 14.0;
    // Right to left: length, heart, then (wide windows) the album column.
    let dur_x = right;
    let heart_c = pos2(right - 62.0, cy);
    let name_w = if wide { rect.width() * 0.42 } else { (heart_c.x - 22.0 - tx).max(60.0) };
    let p = ui.painter();
    let th = clipped(p, pos2(tx, cy - 19.0), &t.title, FontId::proportional(15.0), text, name_w, 1);
    clipped(p, pos2(tx, cy - 17.0 + th), &t.artist, FontId::proportional(12.5), sub, name_w, 1);
    if wide {
        let ax = rect.min.x + rect.width() * 0.58;
        let aw = (heart_c.x - 30.0 - ax).max(40.0);
        clipped(p, pos2(ax, cy - 8.0), &t.album, FontId::proportional(13.5), sub, aw, 1);
    }
    p.text(
        pos2(dur_x, cy),
        Align2::RIGHT_CENTER,
        fmt_time(t.duration.as_secs_f32()),
        FontId::proportional(13.5),
        sub,
    );
    let mut heart_clicked = false;
    if t.spotify_id.is_some() {
        let hr = ui
            .interact(Rect::from_center_size(heart_c, vec2(28.0, 28.0)), resp.id.with("heart"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if liked || resp.hovered() || hr.hovered() {
            let col = if liked { accent } else if hr.hovered() { text } else { sub };
            heart(ui.painter(), heart_c, liked, col);
        }
        heart_clicked = hr.clicked();
    }
    let row_clicked = resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !heart_clicked;
    (row_clicked, heart_clicked)
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
        accent: Color32,
        text: Color32,
        sub: Color32,
    ) {
        for n in [Node::SpPlaylists, Node::SpAlbums, Node::SpArtists, Node::SpShows, Node::SpLiked] {
            self.want_list(n);
        }
        // Spotify's own shelves (read quietly in the background; nothing is shown until they arrive).
        self.want_shelves();
        self.want_recent();
        let range_id = egui::Id::new("top_range");
        let range: usize = ui.data(|d| d.get_temp::<usize>(range_id)).unwrap_or(0).min(2);
        self.want_top_songs(range);
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
        // Quick-access tiles, as Spotify's Home (and Spotifast) show them: what you played last, newest
        // first (the playlist, album, artist or podcast a song was played from, Liked Songs, or the song
        // itself when it was played on its own). Liked Songs and your first playlists fill the rest.
        let max_tiles = cols * rows;
        let liked_tile = || Tile { name: "Liked Songs".to_string(), art: None, liked: true, go: Click::Open(Node::SpLiked) };
        let mut tiles: Vec<Tile> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (i, play) in self.sp_recent_plays.iter().enumerate() {
            if tiles.len() >= max_tiles {
                break;
            }
            let t = &play.track;
            let ctx = play.context.as_deref().unwrap_or("");
            let id = ctx.rsplit(':').next().unwrap_or("");
            let tile: Option<(String, Tile)> = if ctx.ends_with(":collection") {
                Some(("liked".to_string(), liked_tile()))
            } else if ctx.starts_with("spotify:playlist:") {
                let key = format!("playlist:{id}");
                find_item(&playlists, &key)
                    .map(|(it, n)| (key, Tile { name: it.label.clone(), art: it.art.clone(), liked: false, go: Click::Open(n.clone()) }))
            } else if ctx.starts_with("spotify:artist:") {
                let key = format!("artist:{id}");
                find_item(&artists, &key)
                    .map(|(it, n)| (key, Tile { name: it.label.clone(), art: it.art.clone(), liked: false, go: Click::Open(n.clone()) }))
            } else if ctx.starts_with("spotify:show:") {
                let key = format!("show:{id}");
                find_item(&shows, &key)
                    .map(|(it, n)| (key, Tile { name: it.label.clone(), art: it.art.clone(), liked: false, go: Click::Open(n.clone()) }))
            } else if ctx.starts_with("spotify:album:") && t.album_id.as_deref() == Some(id) {
                Some((
                    format!("album:{id}"),
                    Tile {
                        name: t.album.clone(),
                        art: t.art_url.clone(),
                        liked: false,
                        go: Click::Open(Node::SpAlbum(id.to_string(), t.album.clone())),
                    },
                ))
            } else {
                // A song played on its own, or from somewhere Home cannot name.
                t.album_id.as_ref().map(|aid| {
                    (
                        format!("album:{aid}"),
                        Tile { name: t.title.clone(), art: t.art_url.clone(), liked: false, go: Click::Song(i) },
                    )
                })
            };
            if let Some((key, tile)) = tile {
                if seen.insert(key) {
                    tiles.push(tile);
                }
            }
        }
        let mut filler: Vec<(String, Tile)> = vec![("liked".to_string(), liked_tile())];
        for (it, n) in open_items(&playlists) {
            filler.push((
                node_key(n),
                Tile { name: it.label.clone(), art: it.art.clone(), liked: false, go: Click::Open(n.clone()) },
            ));
        }
        for (key, tile) in filler {
            if tiles.len() >= max_tiles {
                break;
            }
            if seen.insert(key) {
                tiles.push(tile);
            }
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
        let has_recent_albums = !recent_albums.is_empty();
        let has_top_artists = !top_artists.is_empty();
        // Order like Spotify: Made for you, top mixes, then what you played, then your library. A shelf
        // that says the same as an earlier one is left out (Recents when Recently played exists).
        let mut library: Vec<(&str, Option<Node>, Vec<Card>)> = vec![
            ("Made for you", Some(Node::SpPlaylists), from_items(&made, "Made for you", false)),
            ("Your top mixes", Some(Node::SpPlaylists), from_items(&mixes, "Made for you", false)),
            ("Recently played", None, recent_albums),
            ("Albums featuring songs you like", None, liked_albums),
            ("Your top artists", None, top_artists),
            ("Recents", None, recent),
            ("Your playlists", Some(Node::SpPlaylists), from_items(&own, "Playlist", false)),
            ("Your albums", Some(Node::SpAlbums), from_items(&albums, "Album", false)),
            ("Your artists", Some(Node::SpArtists), from_items(&artists, "Artist", true)),
            ("Your podcasts", Some(Node::SpShows), from_items(&shows, "Podcast", false)),
        ];
        library.retain(|(t, _, _)| !((*t == "Recents" && has_recent_albums) || (*t == "Your artists" && has_top_artists)));
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

        // "Your top songs" sits right after Spotify's Made for you / Your top mixes.
        let top_pos = shelves
            .iter()
            .rposition(|s| s.title == "Made for you" || s.title == "Your top mixes")
            .map_or(0, |i| i + 1);
        let signed_in = self.sp.is_some();
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
                if self.cfg.home_shelves && signed_in && self.sp_shelves.is_none() && !self.shelves_busy {
                    let why = if self.sp_session.is_none() {
                        "Made for you needs the playback sign-in (Premium). Sign in again under Spotify account.".to_string()
                    } else if self.shelves_next.is_some() {
                        // The newest "home shelves" line of the log says what went wrong.
                        let log = crate::spotify::last_log();
                        let line = log
                            .rsplit("  |  ")
                            .find(|l| l.contains("home shelves"))
                            .unwrap_or("Spotify did not answer")
                            .replace("home shelves: ", "");
                        format!("Made for you and Your top mixes could not load: {line}.")
                    } else {
                        String::new()
                    };
                    if !why.is_empty() {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            ui.label(egui::RichText::new(why).size(12.0).color(sub));
                            if ui
                                .add(egui::Label::new(egui::RichText::new("Retry").size(12.0).strong().color(text)).sense(Sense::click()))
                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                .clicked()
                            {
                                clicked = Some(Click::RetryShelves);
                            }
                        });
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

                for idx in 0..=shelves.len() {
                    if idx == top_pos && signed_in && filter != 2 {
                        ui.add_space(14.0);
                        ui.label(egui::RichText::new("Your top songs").size(22.0).strong().color(text));
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            for (i, label) in ["Last 4 weeks", "Last 6 months", "Past year"].iter().enumerate() {
                                let on = range == i;
                                let chip = egui::Button::new(
                                    egui::RichText::new(*label).size(12.5).strong().color(if on { base } else { text }),
                                )
                                .fill(if on { text } else { mix(base, text, 0.12) })
                                .rounding(16.0)
                                .min_size(vec2(0.0, 28.0));
                                if ui.add(chip).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                    ui.data_mut(|d| d.insert_temp(range_id, i));
                                }
                            }
                        });
                        let more_id = egui::Id::new("top_more");
                        let more: bool = ui.data(|d| d.get_temp::<bool>(more_id)).unwrap_or(false);
                        let list: &[crate::library::Track] =
                            self.sp_top_songs[range].as_ref().map(|(_, v)| v.as_slice()).unwrap_or(&[]);
                        if list.is_empty() {
                            let msg = if self.sp_top_songs[range].is_some() {
                                "Nothing here yet. Listen for a while and your top songs will show up."
                            } else {
                                "Loading your top songs. If this stays empty, sign out of Spotify and sign in again to allow top listening."
                            };
                            ui.label(egui::RichText::new(msg).size(12.5).color(sub));
                        }
                        let n = if more { list.len() } else { list.len().min(5) };
                        ui.scope(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            for (i, t) in list.iter().take(n).enumerate() {
                                let liked = t.spotify_id.as_ref().map_or(false, |id| self.liked_ids.contains(id));
                                let (row, heart_hit) = song_row(ui, &mut self.art, i + 1, t, liked, base, text, sub, accent);
                                if heart_hit {
                                    if let Some(id) = &t.spotify_id {
                                        clicked = Some(if liked { Click::Unlike(id.clone()) } else { Click::Like(id.clone()) });
                                    }
                                } else if row {
                                    clicked = Some(Click::Top(range, i));
                                }
                            }
                        });
                        if list.len() > 5 {
                            let label = if more { "Show less" } else { "Show more top songs" };
                            let r = ui
                                .add(egui::Label::new(egui::RichText::new(label).size(13.0).strong().color(sub)).sense(Sense::click()))
                                .on_hover_cursor(egui::CursorIcon::PointingHand);
                            if r.clicked() {
                                ui.data_mut(|d| d.insert_temp(more_id, !more));
                            }
                        }
                    }
                    let Some(shelf) = shelves.get(idx) else { continue };
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
