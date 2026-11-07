//! iPod-style interface: a menu stack on an LCD, driven by a click wheel.

use super::{fmt_time, Action, App, SpMsg};
use crate::config::Repeat;
use crate::library::Track;
use crate::spotify;
use eframe::egui::{
    self, pos2, vec2, Align2, Color32, FontId, Key, Modifiers, Painter, Pos2, Rect, Sense, Shape,
    Stroke,
};
use std::{
    f32::consts::PI,
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
    OpenFolder,
    ToggleShuffle,
    CycleRepeat,
    ClassicView,
    SpotifySetup,
    SpotifyDisconnect,
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
    }
}

fn play_page(tracks: Vec<Track>) -> (Vec<Item>, Vec<Track>) {
    let items = tracks
        .iter()
        .enumerate()
        .map(|(i, t)| Item {
            label: t.title.clone(),
            kind: Kind::Play(i),
        })
        .collect();
    (items, tracks)
}

fn entries_page(entries: Vec<spotify::Entry>, make: impl Fn(String, String) -> Node) -> Vec<Item> {
    entries
        .into_iter()
        .map(|e| Item {
            label: e.name.clone(),
            kind: Kind::Open(make(e.id, e.name)),
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

    pub(super) fn ipod_receive(&mut self, node: Node, res: Result<(Vec<Item>, Vec<Track>), String>) {
        let found = self
            .ipod
            .stack
            .iter_mut()
            .rev()
            .find(|p| p.node == node && p.load == Load::Loading);
        if let Some(p) = found {
            match res {
                Ok((items, tracks)) => {
                    p.items = items;
                    p.tracks = tracks;
                    p.load = Load::Ready;
                    if p.items.is_empty() {
                        p.items.push(item("(empty)", Kind::Info));
                    }
                }
                Err(e) => p.load = Load::Failed(e),
            }
        }
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
        let avail = ui.available_rect_before_wrap();
        let s = (avail.width() / 320.0)
            .min(avail.height() / 540.0)
            .clamp(0.5, 2.2);
        let body = Rect::from_center_size(avail.center(), vec2(300.0, 500.0) * s);
        let at = |x: f32, y: f32| pos2(body.min.x + x * s, body.min.y + y * s);
        let painter = ui.painter().clone();
        let ink = Color32::from_rgb(30, 30, 40);
        let gray = Color32::from_rgb(110, 116, 126);

        // Snapshot everything the drawing needs, so we can mutate self afterwards.
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
        let np = self
            .current
            .and_then(|i| self.tracks.get(i))
            .map(|t| (t.title.clone(), t.artist.clone(), t.album.clone(), t.duration.as_secs_f32()));
        let pos = self.position_secs();
        let playing = self.player.as_ref().map_or(false, |p| p.is_playing());
        let vol = self.cfg.volume;
        let show_vol = self.ipod.vol_until.map_or(false, |t| Instant::now() < t);
        let title = if self.scan_rx.is_some() {
            format!("Scanning {}", self.scan_count)
        } else {
            title
        };

        // Body and bezel.
        painter.rect_filled(body, 30.0 * s, Color32::from_rgb(232, 232, 236));
        painter.rect_stroke(body, 30.0 * s, Stroke::new(1.5, Color32::from_rgb(170, 170, 176)));
        let bezel = Rect::from_min_size(at(18.0, 22.0), vec2(264.0, 196.0) * s);
        painter.rect_filled(bezel, 8.0 * s, Color32::from_rgb(28, 28, 30));
        let screen = bezel.shrink(4.0 * s);
        painter.rect_filled(screen, 4.0 * s, Color32::from_rgb(244, 247, 250));
        let sp = painter.with_clip_rect(screen);

        // Header.
        let head = Rect::from_min_size(screen.min, vec2(screen.width(), 22.0 * s));
        sp.rect_filled(head, 0.0, Color32::from_rgb(208, 220, 234));
        sp.text(
            head.center(),
            Align2::CENTER_CENTER,
            &title,
            FontId::proportional(13.0 * s),
            ink,
        );
        if playing {
            tri(&sp, pos2(head.min.x + 12.0 * s, head.center().y), 9.0 * s, true, ink);
        }
        let bat = Rect::from_center_size(
            pos2(head.max.x - 20.0 * s, head.center().y),
            vec2(20.0 * s, 9.0 * s),
        );
        sp.rect_stroke(bat, 1.5 * s, Stroke::new(1.0, ink));
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
                        gray,
                    );
                }
                Some((t, a, al, d)) => {
                    let g = sp.layout(
                        t.clone(),
                        FontId::proportional(15.0 * s),
                        ink,
                        screen.width() - 24.0 * s,
                    );
                    let h = g.size().y;
                    sp.galley(pos2(left, top + 10.0 * s), g, ink);
                    sp.text(
                        pos2(left, top + 14.0 * s + h),
                        Align2::LEFT_TOP,
                        a,
                        FontId::proportional(12.0 * s),
                        gray,
                    );
                    sp.text(
                        pos2(left, top + 30.0 * s + h),
                        Align2::LEFT_TOP,
                        al,
                        FontId::proportional(11.0 * s),
                        gray,
                    );

                    let by = screen.max.y - 34.0 * s;
                    let bar = Rect::from_min_max(
                        pos2(left, by),
                        pos2(screen.max.x - 12.0 * s, by + 6.0 * s),
                    );
                    sp.rect_filled(bar, 3.0 * s, Color32::from_rgb(200, 206, 214));
                    let accent = Color32::from_rgb(52, 120, 214);
                    if show_vol {
                        let fill = Rect::from_min_max(
                            bar.min,
                            pos2(bar.min.x + bar.width() * vol, bar.max.y),
                        );
                        sp.rect_filled(fill, 3.0 * s, accent);
                        sp.text(
                            pos2(left, by + 9.0 * s),
                            Align2::LEFT_TOP,
                            "Volume",
                            FontId::proportional(11.0 * s),
                            gray,
                        );
                    } else {
                        let frac = if *d > 0.0 { (pos / d).clamp(0.0, 1.0) } else { 0.0 };
                        let fill = Rect::from_min_max(
                            bar.min,
                            pos2(bar.min.x + bar.width() * frac, bar.max.y),
                        );
                        sp.rect_filled(fill, 3.0 * s, accent);
                        sp.text(
                            pos2(left, by + 9.0 * s),
                            Align2::LEFT_TOP,
                            fmt_time(pos),
                            FontId::proportional(11.0 * s),
                            gray,
                        );
                        sp.text(
                            pos2(bar.max.x, by + 9.0 * s),
                            Align2::RIGHT_TOP,
                            format!("-{}", fmt_time((d - pos).max(0.0))),
                            FontId::proportional(11.0 * s),
                            gray,
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
                        gray,
                    );
                }
                Load::Failed(e) => {
                    let g = sp.layout(
                        e.clone(),
                        FontId::proportional(12.0 * s),
                        Color32::from_rgb(180, 40, 40),
                        screen.width() - 24.0 * s,
                    );
                    sp.galley(pos2(left, top + 10.0 * s), g, Color32::from_rgb(180, 40, 40));
                }
                Load::Ready => {
                    for (k, (idx, label, chevron, selected)) in rows.iter().enumerate() {
                        let r = Rect::from_min_size(
                            pos2(screen.min.x, top + k as f32 * 23.0 * s),
                            vec2(screen.width(), 23.0 * s),
                        );
                        if *selected {
                            sp.rect_filled(r, 0.0, Color32::from_rgb(52, 120, 214));
                        }
                        let col = if *selected { Color32::WHITE } else { ink };
                        sp.with_clip_rect(Rect::from_min_max(
                            r.min,
                            pos2(r.max.x - 20.0 * s, r.max.y),
                        ))
                        .text(
                            pos2(r.min.x + 8.0 * s, r.center().y),
                            Align2::LEFT_CENTER,
                            label,
                            FontId::proportional(13.0 * s),
                            col,
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

        // Click wheel.
        let c = at(150.0, 362.0);
        let r_out = 112.0 * s;
        let r_in = 42.0 * s;
        painter.circle_filled(c, r_out, Color32::from_rgb(250, 250, 251));
        painter.circle_stroke(c, r_out, Stroke::new(1.0, Color32::from_rgb(200, 200, 205)));
        painter.circle_filled(c, r_in, Color32::from_rgb(226, 226, 230));
        painter.circle_stroke(c, r_in, Stroke::new(1.0, Color32::from_rgb(190, 190, 196)));
        painter.text(
            pos2(c.x, c.y - r_out * 0.72),
            Align2::CENTER_CENTER,
            "MENU",
            FontId::proportional(12.0 * s),
            gray,
        );
        let wr = r_out * 0.74;
        tri(&painter, pos2(c.x + wr - 5.0 * s, c.y), 11.0 * s, true, gray);
        tri(&painter, pos2(c.x + wr + 5.0 * s, c.y), 11.0 * s, true, gray);
        tri(&painter, pos2(c.x - wr + 5.0 * s, c.y), 11.0 * s, false, gray);
        tri(&painter, pos2(c.x - wr - 5.0 * s, c.y), 11.0 * s, false, gray);
        tri(&painter, pos2(c.x - 6.0 * s, c.y + wr), 11.0 * s, true, gray);
        for dx in [3.0_f32, 8.0] {
            painter.rect_filled(
                Rect::from_center_size(pos2(c.x + dx * s, c.y + wr), vec2(3.0 * s, 11.0 * s)),
                0.0,
                gray,
            );
        }
        if let Some(e) = &self.error {
            painter.text(
                pos2(body.center().x, body.max.y + 8.0 * s),
                Align2::CENTER_TOP,
                e,
                FontId::proportional(12.0 * s),
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
