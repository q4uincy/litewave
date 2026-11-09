//! iPod-style interface: a menu stack on an LCD, driven by a click wheel.

use super::widgets::{initial, tint_for};
use super::{fmt_time, Action, App, SpMsg};
use crate::art::{self, ArtKey};
use crate::config::Repeat;
use crate::library::Track;
use crate::spotify;
use crate::theme::mix;
use eframe::egui::{
    self, pos2, vec2, Align2, Color32, FontId, Key, Modifiers, Painter, Pos2, Rect, Sense, Shape,
    Stroke,
};
use std::{
    f32::consts::PI,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const ROWS: usize = 7;

#[derive(Clone, PartialEq)]
pub enum Node {
    Main,
    Music,
    Artists,
    Albums,
    Songs,
    LocalArtist(String),
    LocalAlbum(String, String),
    Settings,
    Spotify,
    SpPlaylists,
    SpPlaylist(String, String),
    SpLiked,
    SpAlbums,
    SpAlbum(String, String),
    SpArtists,
    SpArtist(String, String),
    NowPlaying,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Cmd {
    ShuffleAll,
    Themes,
    OpenFolder,
    ToggleShuffle,
    CycleRepeat,
    ClassicView,
    SpotifySetup,
    SpotifyDisconnect,
    CycleSkin,
    CycleWallpaper,
    AddWallpaper,
    CycleDim,
}

#[derive(Clone)]
pub enum Kind {
    Open(Node),
    Play(usize),
    Run(Cmd),
    Info,
}

#[derive(Clone)]
pub struct Item {
    pub label: String,
    pub kind: Kind,
    /// Cover image URL for Spotify rows.
    pub art: Option<String>,
}

#[derive(Clone, PartialEq)]
pub enum Load {
    Ready,
    Loading,
    Failed(String),
}

pub struct Page {
    pub node: Node,
    pub title: String,
    pub items: Vec<Item>,
    /// Track list backing `Kind::Play(i)` items.
    pub tracks: Vec<Track>,
    pub sel: usize,
    pub top: usize,
    pub load: Load,
}

#[derive(Default)]
pub struct IpodState {
    pub stack: Vec<Page>,
    wheel_acc: f32,
    scroll_acc: f32,
    last_angle: Option<f32>,
    vol_until: Option<Instant>,
}

fn item(label: &str, kind: Kind) -> Item {
    Item {
        label: label.to_string(),
        kind,
        art: None,
    }
}

fn play_page(tracks: Vec<Track>) -> (Vec<Item>, Vec<Track>) {
    let items = tracks
        .iter()
        .enumerate()
        .map(|(i, t)| Item {
            label: t.title.clone(),
            kind: Kind::Play(i),
            art: None,
        })
        .collect();
    (items, tracks)
}

fn entries_page(entries: Vec<spotify::Entry>, make: impl Fn(String, String) -> Node) -> Vec<Item> {
    entries
        .into_iter()
        .map(|e| {
            let art = e.art.clone();
            Item {
                label: e.name.clone(),
                kind: Kind::Open(make(e.id, e.name)),
                art,
            }
        })
        .collect()
}

/// Runs on a worker thread.
fn fetch(c: &spotify::Client, node: &Node) -> Result<(Vec<Item>, Vec<Track>), String> {
    match node {
        Node::SpPlaylists => Ok((entries_page(c.playlists()?, Node::SpPlaylist), vec![])),
        Node::SpPlaylist(id, _) => Ok(play_page(c.playlist_tracks(id)?)),
        Node::SpLiked => Ok(play_page(c.liked()?)),
        Node::SpAlbums => Ok((entries_page(c.saved_albums()?, Node::SpAlbum), vec![])),
        Node::SpAlbum(id, name) => Ok(play_page(c.album_tracks(id, name)?)),
        Node::SpArtists => Ok((entries_page(c.followed_artists()?, Node::SpArtist), vec![])),
        Node::SpArtist(id, _) => Ok((entries_page(c.artist_albums(id)?, Node::SpAlbum), vec![])),
        _ => Err("Not a Spotify page".to_string()),
    }
}

impl App {
    // ---- menu model --------------------------------------------------------

    fn local_artists(&self) -> Vec<String> {
        let mut v: Vec<String> = self.local.iter().map(|t| t.artist.clone()).collect();
        v.sort_by_key(|s| s.to_lowercase());
        v.dedup();
        v
    }

    fn local_albums(&self, artist: Option<&str>) -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> = self
            .local
            .iter()
            .filter(|t| artist.map_or(true, |a| t.artist == a))
            .map(|t| (t.artist.clone(), t.album.clone()))
            .collect();
        v.sort_by_key(|(a, al)| (a.to_lowercase(), al.to_lowercase()));
        v.dedup();
        v
    }

    pub(super) fn build_page(&self, node: &Node) -> Page {
        let mut p = Page {
            node: node.clone(),
            title: String::new(),
            items: Vec::new(),
            tracks: Vec::new(),
            sel: 0,
            top: 0,
            load: Load::Ready,
        };
        let album_label = |al: &str| {
            if al.is_empty() {
                "Unknown album".to_string()
            } else {
                al.to_string()
            }
        };
        match node {
            Node::Main => {
                p.title = "litewave".into();
                p.items = vec![
                    item("Music", Kind::Open(Node::Music)),
                    item("Spotify", Kind::Open(Node::Spotify)),
                    item("Now Playing", Kind::Open(Node::NowPlaying)),
                    item("Shuffle Songs", Kind::Run(Cmd::ShuffleAll)),
                    item("Settings", Kind::Open(Node::Settings)),
                ];
            }
            Node::Music => {
                p.title = "Music".into();
                p.items = vec![
                    item("Artists", Kind::Open(Node::Artists)),
                    item("Albums", Kind::Open(Node::Albums)),
                    item("Songs", Kind::Open(Node::Songs)),
                    item("Add music folder...", Kind::Run(Cmd::OpenFolder)),
                ];
            }
            Node::Artists => {
                p.title = "Artists".into();
                p.items = self
                    .local_artists()
                    .into_iter()
                    .map(|a| Item {
                        label: a.clone(),
                        kind: Kind::Open(Node::LocalArtist(a)),
                        art: None,
                    })
                    .collect();
            }
            Node::Albums => {
                p.title = "Albums".into();
                p.items = self
                    .local_albums(None)
                    .into_iter()
                    .map(|(a, al)| Item {
                        label: album_label(&al),
                        kind: Kind::Open(Node::LocalAlbum(a, al)),
                        art: None,
                    })
                    .collect();
            }
            Node::Songs => {
                p.title = "Songs".into();
                let (items, tracks) = play_page(self.local.clone());
                p.items = items;
                p.tracks = tracks;
            }
            Node::LocalArtist(a) => {
                p.title = a.clone();
                p.items = self
                    .local_albums(Some(a))
                    .into_iter()
                    .map(|(ar, al)| Item {
                        label: album_label(&al),
                        kind: Kind::Open(Node::LocalAlbum(ar, al)),
                        art: None,
                    })
                    .collect();
            }
            Node::LocalAlbum(a, al) => {
                p.title = album_label(al);
                let list: Vec<Track> = self
                    .local
                    .iter()
                    .filter(|t| t.artist == *a && t.album == *al)
                    .cloned()
                    .collect();
                let (items, tracks) = play_page(list);
                p.items = items;
                p.tracks = tracks;
            }
            Node::Settings => {
                p.title = "Settings".into();
                let on = |b: bool| if b { "On" } else { "Off" };
                let rep = match self.cfg.repeat {
                    Repeat::Off => "Off",
                    Repeat::All => "All",
                    Repeat::One => "One",
                };
                p.items = vec![
                    item(
                        &format!("Shuffle: {}", on(self.cfg.shuffle)),
                        Kind::Run(Cmd::ToggleShuffle),
                    ),
                    item(&format!("Repeat: {rep}"), Kind::Run(Cmd::CycleRepeat)),
                    item("Add music folder...", Kind::Run(Cmd::OpenFolder)),
                    item(
                        &format!("Skin: {}", skin_of(self.cfg.ipod_skin).name),
                        Kind::Run(Cmd::CycleSkin),
                    ),
                    item(
                        &format!("Wallpaper: {}", wallpaper_label(&self.cfg.ipod_wallpaper)),
                        Kind::Run(Cmd::CycleWallpaper),
                    ),
                    item("Add wallpaper...", Kind::Run(Cmd::AddWallpaper)),
                    item(
                        &format!("Wallpaper dim: {}%", self.cfg.ipod_wallpaper_dim as u32 * 100 / 255),
                        Kind::Run(Cmd::CycleDim),
                    ),
                    item("Themes...", Kind::Run(Cmd::Themes)),
                    item("Classic view", Kind::Run(Cmd::ClassicView)),
                    item("Spotify account", Kind::Run(Cmd::SpotifySetup)),
                ];
                if self.sp.is_some() {
                    p.items
                        .push(item("Disconnect Spotify", Kind::Run(Cmd::SpotifyDisconnect)));
                }
                p.items.push(item(
                    &format!("litewave {}", env!("CARGO_PKG_VERSION")),
                    Kind::Info,
                ));
            }
            Node::Spotify => {
                p.title = "Spotify".into();
                if !spotify::ENABLED {
                    p.items = vec![
                        item("Not in this build", Kind::Info),
                        item("Use --features spotify", Kind::Info),
                    ];
                } else if self.sp.is_none() {
                    p.items = vec![
                        item("Connect Spotify...", Kind::Run(Cmd::SpotifySetup)),
                        item(
                            if self.sp_status.is_empty() {
                                "Not connected"
                            } else {
                                self.sp_status.as_str()
                            },
                            Kind::Info,
                        ),
                    ];
                } else {
                    p.items = vec![
                        item("Playlists", Kind::Open(Node::SpPlaylists)),
                        item("Liked Songs", Kind::Open(Node::SpLiked)),
                        item("Albums", Kind::Open(Node::SpAlbums)),
                        item("Artists", Kind::Open(Node::SpArtists)),
                    ];
                }
            }
            Node::SpPlaylists => {
                p.title = "Playlists".into();
                p.load = Load::Loading;
            }
            Node::SpPlaylist(_, name) | Node::SpAlbum(_, name) | Node::SpArtist(_, name) => {
                p.title = name.clone();
                p.load = Load::Loading;
            }
            Node::SpLiked => {
                p.title = "Liked Songs".into();
                p.load = Load::Loading;
            }
            Node::SpAlbums => {
                p.title = "Albums".into();
                p.load = Load::Loading;
            }
            Node::SpArtists => {
                p.title = "Artists".into();
                p.load = Load::Loading;
            }
            Node::NowPlaying => {
                p.title = "Now Playing".into();
            }
        }
        if p.items.is_empty() && p.load == Load::Ready && p.node != Node::NowPlaying {
            p.items.push(item("(empty)", Kind::Info));
        }
        p
    }

    fn spawn_fetch(&self, node: Node) {
        let Some(client) = self.sp.clone() else { return };
        let tx = self.sp_tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let res = fetch(&client, &node);
            let _ = tx.send(SpMsg::Page(node, res));
            ctx.request_repaint();
        });
    }

    pub(super) fn ipod_push(&mut self, node: Node) {
        let page = self.build_page(&node);
        let loading = page.load == Load::Loading;
        self.ipod.stack.push(page);
        if loading {
            self.spawn_fetch(node);
        }
    }

    pub(super) fn sp_push(&mut self, node: Node) {
        let page = self.build_page(&node);
        let loading = page.load == Load::Loading;
        self.sp_nav.push(page);
        if loading {
            self.spawn_fetch(node);
        }
    }

    fn fill_page(stack: &mut [Page], node: &Node, res: &Result<(Vec<Item>, Vec<Track>), String>) {
        let found = stack
            .iter_mut()
            .rev()
            .find(|p| p.node == *node && p.load == Load::Loading);
        if let Some(p) = found {
            match res {
                Ok((items, tracks)) => {
                    p.items = items.clone();
                    p.tracks = tracks.clone();
                    p.load = Load::Ready;
                    if p.items.is_empty() {
                        p.items.push(item("(empty)", Kind::Info));
                    }
                }
                Err(e) => p.load = Load::Failed(e.clone()),
            }
        }
    }

    pub(super) fn ipod_receive(&mut self, node: Node, res: Result<(Vec<Item>, Vec<Track>), String>) {
        if let Ok((items, tracks)) = &res {
            if items.iter().any(|i| super::has_cjk(&i.label))
                || tracks.iter().any(|t| super::has_cjk(&t.title) || super::has_cjk(&t.artist))
            {
                self.cjk_needed = true;
                self.fonts_check = true;
            }
        }
        Self::fill_page(&mut self.ipod.stack, &node, &res);
        Self::fill_page(&mut self.sp_nav, &node, &res);
    }

    /// Rebuilds Settings/Spotify pages in place after a state change.
    pub(super) fn refresh_top(&mut self) {
        let Some(top) = self.ipod.stack.last() else { return };
        if !matches!(top.node, Node::Settings | Node::Spotify) {
            return;
        }
        let (node, sel, first) = (top.node.clone(), top.sel, top.top);
        let mut np = self.build_page(&node);
        np.sel = sel.min(np.items.len().saturating_sub(1));
        np.top = first.min(np.sel);
        if let Some(last) = self.ipod.stack.last_mut() {
            *last = np;
        }
    }

    // ---- navigation --------------------------------------------------------

    fn ipod_action(&mut self, a: Action) {
        let ctx = self.ctx.clone();
        self.apply(a, &ctx);
    }

    fn ipod_move(&mut self, d: i32) {
        let is_np = self
            .ipod
            .stack
            .last()
            .map_or(false, |p| p.node == Node::NowPlaying);
        if is_np {
            let v = (self.cfg.volume + 0.04 * d as f32).clamp(0.0, 1.0);
            self.ipod_action(Action::Volume(v));
            self.ipod.vol_until = Some(Instant::now() + Duration::from_millis(1500));
            return;
        }
        if let Some(p) = self.ipod.stack.last_mut() {
            if p.items.is_empty() {
                return;
            }
            let n = p.items.len() as i32;
            p.sel = (p.sel as i32 + d).clamp(0, n - 1) as usize;
            if p.sel < p.top {
                p.top = p.sel;
            } else if p.sel >= p.top + ROWS {
                p.top = p.sel + 1 - ROWS;
            }
        }
    }

    pub(super) fn ipod_back(&mut self) {
        if self.ipod.stack.len() > 1 {
            self.ipod.stack.pop();
        }
    }

    pub(super) fn ipod_select(&mut self) {
        let (is_np, chosen, tracks, is_songs) = match self.ipod.stack.last() {
            Some(p) => {
                let chosen = p.items.get(p.sel).cloned();
                let tracks = if matches!(chosen.as_ref().map(|i| &i.kind), Some(Kind::Play(_))) {
                    p.tracks.clone()
                } else {
                    Vec::new()
                };
                (p.node == Node::NowPlaying, chosen, tracks, p.node == Node::Songs)
            }
            None => return,
        };
        if is_np {
            self.ipod_action(Action::Toggle);
            return;
        }
        match chosen.map(|i| i.kind) {
            Some(Kind::Open(n)) => self.ipod_push(n),
            Some(Kind::Play(i)) => {
                self.set_list(tracks, i, is_songs);
                self.ipod_push(Node::NowPlaying);
            }
            Some(Kind::Run(c)) => self.ipod_run(c),
            _ => {}
        }
    }

    fn ipod_run(&mut self, c: Cmd) {
        match c {
            Cmd::ShuffleAll => {
                if self.local.is_empty() {
                    self.error = Some("No local music loaded. Use Classic view > Open folder.".into());
                    return;
                }
                let n = self.local.len();
                let k = self.rand(n);
                self.cfg.shuffle = true;
                self.cfg_dirty = true;
                self.set_list(self.local.clone(), k, true);
                self.ipod_push(Node::NowPlaying);
            }
            Cmd::ToggleShuffle => {
                self.ipod_action(Action::Shuffle);
                self.refresh_top();
            }
            Cmd::CycleRepeat => {
                self.ipod_action(Action::CycleRepeat);
                self.refresh_top();
            }
            Cmd::OpenFolder => self.ipod_action(Action::OpenFolder),
            Cmd::Themes => self.themes_open = true,
            Cmd::CycleWallpaper => {
                let mut opts: Vec<Option<String>> =
                    vec![None, Some("builtin:stardust".to_string())];
                opts.extend(
                    list_wallpapers()
                        .into_iter()
                        .map(|p| Some(p.to_string_lossy().into_owned())),
                );
                let i = opts
                    .iter()
                    .position(|o| *o == self.cfg.ipod_wallpaper)
                    .unwrap_or(0);
                self.cfg.ipod_wallpaper = opts[(i + 1) % opts.len()].clone();
                self.cfg_dirty = true;
                self.refresh_top();
            }
            Cmd::AddWallpaper => {
                let picked = rfd::FileDialog::new()
                    .add_filter("Image", &["png", "jpg", "jpeg", "webp"])
                    .pick_files();
                if let (Some(files), Some(dir)) = (picked, wallpapers_dir()) {
                    let _ = std::fs::create_dir_all(&dir);
                    let mut last = None;
                    for f in files {
                        if let Some(name) = f.file_name() {
                            let dest = dir.join(name);
                            if std::fs::copy(&f, &dest).is_ok() {
                                last = Some(dest);
                            }
                        }
                    }
                    if let Some(d) = last {
                        self.cfg.ipod_wallpaper = Some(d.to_string_lossy().into_owned());
                        self.cfg_dirty = true;
                        self.refresh_top();
                    }
                }
            }
            Cmd::CycleDim => {
                self.cfg.ipod_wallpaper_dim = match self.cfg.ipod_wallpaper_dim {
                    0..=85 => 110,
                    86..=130 => 150,
                    131..=170 => 190,
                    _ => 60,
                };
                self.cfg_dirty = true;
                self.refresh_top();
            }
            Cmd::CycleSkin => {
                self.cfg.ipod_skin = (self.cfg.ipod_skin + 1) % SKIN_COUNT;
                self.cfg_dirty = true;
                self.refresh_top();
            }
            Cmd::ClassicView => self.ipod_action(Action::SetIpod(false)),
            Cmd::SpotifySetup => self.sp_window = true,
            Cmd::SpotifyDisconnect => self.disconnect_spotify(),
        }
    }

    pub(super) fn ipod_keys(&mut self, ctx: &egui::Context) {
        let k = ctx.input_mut(|i| {
            [
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::Enter),
                i.consume_key(Modifiers::NONE, Key::Escape),
                i.consume_key(Modifiers::NONE, Key::Backspace),
                i.consume_key(Modifiers::NONE, Key::Space),
                i.consume_key(Modifiers::NONE, Key::ArrowLeft),
                i.consume_key(Modifiers::NONE, Key::ArrowRight),
            ]
        });
        if k[0] {
            self.ipod_move(-1);
        }
        if k[1] {
            self.ipod_move(1);
        }
        if k[2] {
            self.ipod_select();
        }
        if k[3] || k[4] {
            self.ipod_back();
        }
        if k[5] {
            self.ipod_action(Action::Toggle);
        }
        if k[6] {
            self.ipod_action(Action::Prev);
        }
        if k[7] {
            self.ipod_action(Action::Next);
        }
    }

    // ---- drawing -----------------------------------------------------------

    pub(super) fn ipod_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let sk = skin_of(self.cfg.ipod_skin);
        let accent = self.cfg.theme.accent();

        // Layout: a thin window-controls strip on top, the iPod centered below it.
        let full = ui.available_rect_before_wrap();
        let bar = Rect::from_min_size(full.min, vec2(full.width(), 34.0));
        let avail = Rect::from_min_max(pos2(full.min.x, full.min.y + 34.0), full.max);
        let s = (avail.width() / 320.0)
            .min(avail.height() / 540.0)
            .clamp(0.5, 2.2);
        let body = Rect::from_center_size(avail.center(), vec2(300.0, 500.0) * s);
        let at = |x: f32, y: f32| pos2(body.min.x + x * s, body.min.y + y * s);
        let painter = ui.painter().clone();

        let (ink, dim) = if sk.dark {
            (Color32::WHITE, Color32::from_rgb(150, 156, 168))
        } else {
            (Color32::from_rgb(30, 30, 40), Color32::from_rgb(96, 102, 114))
        };
        let screen_bg = if sk.dark {
            Color32::from_rgb(8, 9, 12)
        } else {
            Color32::from_rgb(244, 247, 250)
        };

        // Snapshot everything the drawing needs, so we can mutate self afterwards.
        let art_id = self.current_art(&ctx, art::BIG);
        let (title, is_np, rows, load) = match self.ipod.stack.last() {
            Some(pg) => {
                let end = (pg.top + ROWS).min(pg.items.len());
                let start = pg.top.min(end);
                let rows: Vec<(usize, String, bool, bool)> = pg.items[start..end]
                    .iter()
                    .enumerate()
                    .map(|(k, it)| {
                        (
                            start + k,
                            it.label.clone(),
                            matches!(it.kind, Kind::Open(_)),
                            start + k == pg.sel,
                        )
                    })
                    .collect();
                (pg.title.clone(), pg.node == Node::NowPlaying, rows, pg.load.clone())
            }
            None => (String::new(), false, Vec::new(), Load::Ready),
        };
        let np = self.now.as_ref().map(|t| {
            (
                t.title.clone(),
                t.artist.clone(),
                t.album.clone(),
                t.duration.as_secs_f32(),
            )
        });
        let pos = self.position_secs();
        let playing = self.player.as_ref().map_or(false, |p| p.is_playing());
        let buffering = self.player.as_ref().map_or(false, |p| p.is_buffering());
        let vol = self.cfg.volume;
        let show_vol = self.ipod.vol_until.map_or(false, |t| Instant::now() < t);
        let title = if self.scan_rx.is_some() {
            format!("Scanning {}", self.scan_count)
        } else {
            title
        };

        // ---- window controls (minimize / back to full window / close) -------
        let wallpaper = self.cfg.ipod_wallpaper.clone();
        let wp_dim = self.cfg.ipod_wallpaper_dim;
        let mut win_cmd: Option<usize> = None;
        let drag = ui.interact(bar, ui.id().with("ipod_drag"), Sense::drag());
        if drag.drag_started() {
            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        painter.text(
            pos2(bar.min.x + 12.0, bar.center().y),
            Align2::LEFT_CENTER,
            "litewave",
            FontId::proportional(12.0),
            Color32::from_white_alpha(110),
        );
        let buttons = [("Minimize", 76.0_f32), ("Full view", 76.0), ("Close", 56.0)];
        let mut x = bar.max.x - 8.0;
        for (i, (label, w)) in buttons.iter().enumerate().rev() {
            let r = Rect::from_min_size(pos2(x - w, bar.min.y + 5.0), vec2(*w, 24.0));
            x -= w + 8.0;
            let resp = ui.interact(r, ui.id().with(("ipod_win", i)), Sense::click());
            let fill = if resp.hovered() {
                Color32::from_white_alpha(56)
            } else {
                Color32::from_white_alpha(26)
            };
            painter.rect_filled(r, 12.0, fill);
            painter.text(
                r.center(),
                Align2::CENTER_CENTER,
                *label,
                FontId::proportional(12.0),
                Color32::from_gray(232),
            );
            if resp.clicked() {
                win_cmd = Some(i);
            }
        }

        // ---- body ------------------------------------------------------------
        let r_body = 30.0 * s;
        gradient_rounded(&painter, body, r_body, sk.body_top, sk.body_bottom);
        for (frac, alpha) in [(0.55_f32, 12_u8), (0.36, 12), (0.18, 14)] {
            let band = Rect::from_min_size(body.min, vec2(body.width(), body.height() * frac));
            painter
                .with_clip_rect(band)
                .rect_filled(body.shrink(3.0 * s), r_body - 3.0 * s, Color32::from_white_alpha(alpha));
        }
        painter.rect_stroke(body, r_body, Stroke::new(1.5, Color32::from_black_alpha(80)));
        painter.rect_stroke(
            body.shrink(2.0 * s),
            r_body - 2.0 * s,
            Stroke::new(1.0, Color32::from_white_alpha(80)),
        );

        // ---- screen ----------------------------------------------------------
        let bezel = Rect::from_min_size(at(18.0, 24.0), vec2(264.0, 196.0) * s);
        painter.rect_filled(bezel, 10.0 * s, sk.bezel);
        painter.rect_stroke(bezel, 10.0 * s, Stroke::new(1.0, Color32::from_white_alpha(40)));
        let screen = bezel.shrink(5.0 * s);
        painter.rect_filled(screen, 5.0 * s, screen_bg);
        let sp = painter.with_clip_rect(screen);

        // Backdrop (dark skins): the song's cover on Now Playing, the chosen wallpaper in menus.
        if sk.dark {
            let full_uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
            if is_np {
                if let Some(id) = art_id {
                    let ratio = (screen.height() / screen.width()).min(1.0);
                    let v0 = (1.0 - ratio) / 2.0;
                    sp.image(
                        id,
                        screen,
                        Rect::from_min_max(pos2(0.0, v0), pos2(1.0, 1.0 - v0)),
                        Color32::from_white_alpha(70),
                    );
                }
            } else if let Some(w) = wallpaper.as_deref() {
                if w == "builtin:stardust" {
                    paint_stardust(&sp, screen, s);
                } else if let Some(id) = self.art.get(&ctx, &ArtKey::Image(PathBuf::from(w)), 512) {
                    sp.image(id, screen, full_uv, Color32::WHITE);
                }
                // Dim so menu text stays as readable as on a plain screen.
                sp.rect_filled(screen, 0.0, Color32::from_black_alpha(wp_dim));
            }
        }

        // Header.
        let head = Rect::from_min_size(screen.min, vec2(screen.width(), 24.0 * s));
        if sk.dark {
            sp.text(
                pos2(head.min.x + 12.0 * s, head.center().y),
                Align2::LEFT_CENTER,
                title.to_uppercase(),
                FontId::proportional(10.5 * s),
                Color32::from_rgb(170, 176, 188),
            );
        } else {
            sp.rect_filled(head, 0.0, Color32::from_rgb(208, 220, 234));
            sp.text(
                head.center(),
                Align2::CENTER_CENTER,
                &title,
                FontId::proportional(13.0 * s),
                ink,
            );
        }
        if playing {
            tri(&sp, pos2(head.max.x - 46.0 * s, head.center().y), 8.0 * s, true, accent);
        }
        let bat = Rect::from_center_size(
            pos2(head.max.x - 20.0 * s, head.center().y),
            vec2(20.0 * s, 9.0 * s),
        );
        sp.rect_stroke(bat, 1.5 * s, Stroke::new(1.0, if sk.dark { dim } else { ink }));
        sp.rect_filled(bat.shrink(2.0 * s), 0.5 * s, Color32::from_rgb(60, 170, 90));

        let mut row_click: Option<usize> = None;
        let mut screen_click = false;

        // Content.
        let top = head.max.y;
        let left = screen.min.x + 12.0 * s;
        if is_np {
            let sr = ui.interact(screen, ui.id().with("ipod_screen"), Sense::click());
            if sr.clicked() {
                screen_click = true;
            }
            match &np {
                None => {
                    sp.text(
                        screen.center(),
                        Align2::CENTER_CENTER,
                        "Nothing playing",
                        FontId::proportional(14.0 * s),
                        dim,
                    );
                }
                Some((t, a, al, d)) => {
                    let cover = Rect::from_min_size(pos2(left, top + 8.0 * s), vec2(92.0, 92.0) * s);
                    match art_id {
                        Some(id) => {
                            sp.image(
                                id,
                                cover,
                                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                                Color32::WHITE,
                            );
                        }
                        None => {
                            sp.rect_filled(cover, 6.0 * s, tint_for(t));
                            sp.text(
                                cover.center(),
                                Align2::CENTER_CENTER,
                                initial(t),
                                FontId::proportional(34.0 * s),
                                Color32::from_white_alpha(200),
                            );
                        }
                    }
                    let tx = cover.max.x + 12.0 * s;
                    let wrap = (screen.max.x - tx - 10.0 * s).max(40.0);
                    let g = sp.layout(t.clone(), FontId::proportional(14.0 * s), ink, wrap);
                    let h = g.size().y;
                    sp.galley(pos2(tx, top + 10.0 * s), g, ink);
                    sp.text(
                        pos2(tx, top + 14.0 * s + h),
                        Align2::LEFT_TOP,
                        if buffering { "Buffering..." } else { a.as_str() },
                        FontId::proportional(12.0 * s),
                        dim,
                    );
                    sp.with_clip_rect(Rect::from_min_max(
                        pos2(tx, top),
                        pos2(screen.max.x - 6.0 * s, screen.max.y),
                    ))
                    .text(
                        pos2(tx, top + 30.0 * s + h),
                        Align2::LEFT_TOP,
                        al,
                        FontId::proportional(11.0 * s),
                        dim,
                    );

                    let by = screen.max.y - 34.0 * s;
                    let barr = Rect::from_min_max(
                        pos2(left, by),
                        pos2(screen.max.x - 12.0 * s, by + 6.0 * s),
                    );
                    sp.rect_filled(
                        barr,
                        3.0 * s,
                        if sk.dark { Color32::from_white_alpha(40) } else { Color32::from_rgb(200, 206, 214) },
                    );
                    if show_vol {
                        let fill = Rect::from_min_max(
                            barr.min,
                            pos2(barr.min.x + barr.width() * vol, barr.max.y),
                        );
                        sp.rect_filled(fill, 3.0 * s, accent);
                        sp.text(
                            pos2(left, by + 9.0 * s),
                            Align2::LEFT_TOP,
                            "Volume",
                            FontId::proportional(11.0 * s),
                            dim,
                        );
                    } else {
                        let frac = if *d > 0.0 { (pos / d).clamp(0.0, 1.0) } else { 0.0 };
                        let fill = Rect::from_min_max(
                            barr.min,
                            pos2(barr.min.x + barr.width() * frac, barr.max.y),
                        );
                        sp.rect_filled(fill, 3.0 * s, accent);
                        sp.text(
                            pos2(left, by + 9.0 * s),
                            Align2::LEFT_TOP,
                            fmt_time(pos),
                            FontId::proportional(11.0 * s),
                            dim,
                        );
                        sp.text(
                            pos2(barr.max.x, by + 9.0 * s),
                            Align2::RIGHT_TOP,
                            format!("-{}", fmt_time((d - pos).max(0.0))),
                            FontId::proportional(11.0 * s),
                            dim,
                        );
                    }
                }
            }
        } else {
            match &load {
                Load::Loading => {
                    sp.text(
                        screen.center(),
                        Align2::CENTER_CENTER,
                        "Loading...",
                        FontId::proportional(14.0 * s),
                        dim,
                    );
                }
                Load::Failed(e) => {
                    let red = Color32::from_rgb(255, 120, 120);
                    let g = sp.layout(
                        e.clone(),
                        FontId::proportional(12.0 * s),
                        red,
                        screen.width() - 24.0 * s,
                    );
                    sp.galley(pos2(left, top + 10.0 * s), g, red);
                }
                Load::Ready => {
                    for (k, (idx, label, chevron, selected)) in rows.iter().enumerate() {
                        let r = Rect::from_min_size(
                            pos2(screen.min.x, top + k as f32 * 23.0 * s),
                            vec2(screen.width(), 23.0 * s),
                        );
                        let col;
                        if sk.dark {
                            if *selected {
                                sp.circle_filled(pos2(r.min.x + 12.0 * s, r.center().y), 3.4 * s, accent);
                            }
                            col = if *selected { ink } else { dim };
                        } else {
                            if *selected {
                                sp.rect_filled(r, 0.0, Color32::from_rgb(52, 120, 214));
                            }
                            col = if *selected { Color32::WHITE } else { ink };
                        }
                        let tx = if sk.dark { r.min.x + 24.0 * s } else { r.min.x + 8.0 * s };
                        shadow_text(
                            &sp.with_clip_rect(Rect::from_min_max(
                                r.min,
                                pos2(r.max.x - 20.0 * s, r.max.y),
                            )),
                            pos2(tx, r.center().y),
                            Align2::LEFT_CENTER,
                            label,
                            FontId::proportional(if *selected && sk.dark { 13.5 * s } else { 13.0 * s }),
                            col,
                            sk.dark,
                        );
                        if *chevron {
                            sp.text(
                                pos2(r.max.x - 10.0 * s, r.center().y),
                                Align2::CENTER_CENTER,
                                ">",
                                FontId::proportional(13.0 * s),
                                col,
                            );
                        }
                        let rr = ui.interact(r, ui.id().with(("ipod_row", k)), Sense::click());
                        if rr.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if rr.clicked() {
                            row_click = Some(*idx);
                        }
                    }
                }
            }
        }

        // ---- click wheel -------------------------------------------------------
        let c = at(150.0, 362.0);
        let r_out = 112.0 * s;
        let r_in = 42.0 * s;
        painter.circle_filled(c, r_out, sk.wheel);
        painter.circle_stroke(c, r_out, Stroke::new(1.5, sk.ring));
        painter.circle_stroke(
            pos2(c.x, c.y - 1.0),
            r_out - 2.0,
            Stroke::new(1.0, Color32::from_white_alpha(if sk.dark { 18 } else { 90 })),
        );
        painter.circle_filled(c, r_in, sk.center);
        painter.circle_stroke(c, r_in, Stroke::new(1.0, sk.ring));
        painter.text(
            pos2(c.x, c.y - r_out * 0.72),
            Align2::CENTER_CENTER,
            sk.home,
            FontId::proportional(12.0 * s),
            sk.label,
        );
        let wr = r_out * 0.74;
        tri(&painter, pos2(c.x + wr - 5.0 * s, c.y), 11.0 * s, true, sk.label);
        tri(&painter, pos2(c.x + wr + 5.0 * s, c.y), 11.0 * s, true, sk.label);
        tri(&painter, pos2(c.x - wr + 5.0 * s, c.y), 11.0 * s, false, sk.label);
        tri(&painter, pos2(c.x - wr - 5.0 * s, c.y), 11.0 * s, false, sk.label);
        tri(&painter, pos2(c.x - 6.0 * s, c.y + wr), 11.0 * s, true, sk.label);
        for dx in [3.0_f32, 8.0] {
            painter.rect_filled(
                Rect::from_center_size(pos2(c.x + dx * s, c.y + wr), vec2(3.0 * s, 11.0 * s)),
                0.0,
                sk.label,
            );
        }
        if let Some(e) = &self.error {
            painter.text(
                pos2(full.center().x, full.max.y - 6.0),
                Align2::CENTER_BOTTOM,
                e,
                FontId::proportional(11.5),
                Color32::LIGHT_RED,
            );
        }

        // Wheel input: drag around the ring to scroll, click zones to navigate.
        let wheel_rect = Rect::from_center_size(c, vec2(2.0 * r_out, 2.0 * r_out));
        let resp = ui.interact(wheel_rect, ui.id().with("ipod_wheel"), Sense::click_and_drag());
        let mut moves: i32 = 0;
        if resp.dragged() {
            if let Some(ptr) = ui.input(|i| i.pointer.latest_pos()) {
                let d = ptr - c;
                if d.length() > r_in {
                    let ang = d.y.atan2(d.x);
                    if let Some(last) = self.ipod.last_angle {
                        let mut da = ang - last;
                        while da > PI {
                            da -= 2.0 * PI;
                        }
                        while da < -PI {
                            da += 2.0 * PI;
                        }
                        self.ipod.wheel_acc += da;
                    }
                    self.ipod.last_angle = Some(ang);
                }
            }
        } else {
            self.ipod.last_angle = None;
        }
        let step = PI / 9.0;
        while self.ipod.wheel_acc >= step {
            moves += 1;
            self.ipod.wheel_acc -= step;
        }
        while self.ipod.wheel_acc <= -step {
            moves -= 1;
            self.ipod.wheel_acc += step;
        }

        if ui.rect_contains_pointer(body) {
            let dy = ui.input(|i| i.smooth_scroll_delta.y);
            self.ipod.scroll_acc -= dy;
        }
        while self.ipod.scroll_acc >= 30.0 {
            moves += 1;
            self.ipod.scroll_acc -= 30.0;
        }
        while self.ipod.scroll_acc <= -30.0 {
            moves -= 1;
            self.ipod.scroll_acc += 30.0;
        }

        // 0 select, 1 menu/back, 2 next, 3 play/pause, 4 previous
        let mut click: Option<u8> = None;
        if resp.clicked() {
            if let Some(ptr) = ui.input(|i| i.pointer.latest_pos()) {
                let d = ptr - c;
                let dist = d.length();
                if dist <= r_in {
                    click = Some(0);
                } else if dist <= r_out {
                    let deg = d.y.atan2(d.x).to_degrees();
                    click = Some(if (-135.0..-45.0).contains(&deg) {
                        1
                    } else if (-45.0..45.0).contains(&deg) {
                        2
                    } else if (45.0..135.0).contains(&deg) {
                        3
                    } else {
                        4
                    });
                }
            }
        }

        if moves != 0 {
            self.ipod_move(moves);
        }
        match click {
            Some(0) => self.ipod_select(),
            Some(1) => self.ipod_back(),
            Some(2) => self.ipod_action(Action::Next),
            Some(3) => self.ipod_action(Action::Toggle),
            Some(4) => self.ipod_action(Action::Prev),
            _ => {}
        }
        if let Some(idx) = row_click {
            if let Some(p) = self.ipod.stack.last_mut() {
                p.sel = idx.min(p.items.len().saturating_sub(1));
            }
            self.ipod_select();
        }
        if screen_click {
            self.ipod_action(Action::Toggle);
        }
        match win_cmd {
            Some(0) => ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
            Some(1) => self.ipod_action(Action::SetIpod(false)),
            Some(_) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            None => {}
        }
    }
}

struct Skin {
    body_top: Color32,
    body_bottom: Color32,
    bezel: Color32,
    wheel: Color32,
    ring: Color32,
    label: Color32,
    center: Color32,
    dark: bool,
    home: &'static str,
    name: &'static str,
}

const SKIN_COUNT: u8 = 4;

fn skin_of(i: u8) -> Skin {
    match i % SKIN_COUNT {
        0 => Skin {
            name: "Silver",
            body_top: Color32::from_rgb(218, 222, 229),
            body_bottom: Color32::from_rgb(168, 174, 187),
            bezel: Color32::from_rgb(18, 19, 23),
            wheel: Color32::from_rgb(30, 31, 36),
            ring: Color32::from_rgb(78, 80, 90),
            label: Color32::from_rgb(214, 218, 226),
            center: Color32::from_rgb(48, 49, 56),
            dark: true,
            home: "HOME",
        },
        1 => Skin {
            name: "Black",
            body_top: Color32::from_rgb(58, 60, 67),
            body_bottom: Color32::from_rgb(20, 21, 25),
            bezel: Color32::from_rgb(6, 6, 8),
            wheel: Color32::from_rgb(42, 43, 49),
            ring: Color32::from_rgb(86, 88, 98),
            label: Color32::from_rgb(200, 204, 212),
            center: Color32::from_rgb(22, 23, 27),
            dark: true,
            home: "HOME",
        },
        2 => Skin {
            name: "Classic",
            body_top: Color32::from_rgb(240, 240, 244),
            body_bottom: Color32::from_rgb(220, 220, 227),
            bezel: Color32::from_rgb(28, 28, 32),
            wheel: Color32::from_rgb(250, 250, 251),
            ring: Color32::from_rgb(200, 200, 205),
            label: Color32::from_rgb(110, 116, 126),
            center: Color32::from_rgb(226, 226, 230),
            dark: false,
            home: "MENU",
        },
        _ => Skin {
            name: "Pink",
            body_top: Color32::from_rgb(252, 184, 212),
            body_bottom: Color32::from_rgb(232, 130, 176),
            bezel: Color32::from_rgb(40, 24, 32),
            wheel: Color32::from_rgb(255, 232, 242),
            ring: Color32::from_rgb(232, 170, 200),
            label: Color32::from_rgb(150, 80, 110),
            center: Color32::from_rgb(246, 196, 220),
            dark: true,
            home: "HOME",
        },
    }
}

fn wallpapers_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("litewave").join("wallpapers"))
}

fn list_wallpapers() -> Vec<PathBuf> {
    let Some(dir) = wallpapers_dir() else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut v: Vec<PathBuf> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension().and_then(|e| e.to_str()).map_or(false, |e| {
                matches!(e.to_lowercase().as_str(), "png" | "jpg" | "jpeg" | "webp")
            })
        })
        .collect();
    v.sort();
    v
}

fn wallpaper_label(w: &Option<String>) -> String {
    match w.as_deref() {
        None => "Off".to_string(),
        Some("builtin:stardust") => "Stardust".to_string(),
        Some(p) => Path::new(p)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Custom".to_string()),
    }
}

/// Text with a soft dark shadow so it stays readable over a wallpaper.
fn shadow_text(
    p: &Painter,
    pos: Pos2,
    anchor: Align2,
    text: &str,
    font: FontId,
    col: Color32,
    shadow: bool,
) {
    if shadow {
        p.text(pos + vec2(1.0, 1.0), anchor, text, font.clone(), Color32::from_black_alpha(200));
    }
    p.text(pos, anchor, text, font, col);
}

/// Built-in dark wallpaper with scattered four-point sparkles.
fn paint_stardust(p: &Painter, screen: Rect, s: f32) {
    p.rect_filled(screen, 0.0, Color32::from_rgb(10, 8, 16));
    let mut seed: u32 = 0x9E37_79B9;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed % 10_000) as f32 / 10_000.0
    };
    for _ in 0..34 {
        let c = pos2(
            screen.min.x + rnd() * screen.width(),
            screen.min.y + rnd() * screen.height(),
        );
        let r = (2.0 + rnd() * 7.0) * s;
        let col = Color32::from_white_alpha((70.0 + rnd() * 150.0) as u8);
        let t = r * 0.16;
        p.add(Shape::convex_polygon(
            vec![pos2(c.x - r, c.y), pos2(c.x, c.y - t), pos2(c.x + r, c.y), pos2(c.x, c.y + t)],
            col,
            Stroke::NONE,
        ));
        p.add(Shape::convex_polygon(
            vec![pos2(c.x, c.y - r), pos2(c.x + t, c.y), pos2(c.x, c.y + r), pos2(c.x - t, c.y)],
            col,
            Stroke::NONE,
        ));
    }
}

/// Vertical gradient inside a rounded rectangle, drawn as clipped bands.
fn gradient_rounded(p: &Painter, rect: Rect, rounding: f32, top: Color32, bottom: Color32) {
    let n = 28;
    let h = rect.height() / n as f32;
    for i in 0..n {
        let t = i as f32 / (n - 1) as f32;
        let band = Rect::from_min_max(
            pos2(rect.min.x - 1.0, rect.min.y + h * i as f32 - 0.5),
            pos2(rect.max.x + 1.0, rect.min.y + h * (i + 1) as f32 + 0.5),
        );
        p.with_clip_rect(band)
            .rect_filled(rect, rounding, mix(top, bottom, t));
    }
}

/// Filled triangle pointing right (or left), centred on `c`.
fn tri(p: &Painter, c: Pos2, h: f32, right: bool, col: Color32) {
    let d = if right { 1.0 } else { -1.0 };
    let pts = vec![
        pos2(c.x - d * h * 0.4, c.y - h * 0.5),
        pos2(c.x - d * h * 0.4, c.y + h * 0.5),
        pos2(c.x + d * h * 0.6, c.y),
    ];
    p.add(Shape::convex_polygon(pts, col, Stroke::NONE));
}
