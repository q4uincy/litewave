mod classic;
mod ipod;
mod themes;
mod widgets;

use crate::config::{Config, Repeat};
use crate::art::ArtCache;
use crate::library::{self, ScanMsg, Track};
use crate::player::Playback;
use crate::spotify;
use crate::theme::{self, Theme};
use eframe::egui::{self, pos2, Color32, Key, Modifiers, Rect};
use ipod::{IpodState, Node};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::mpsc::{channel, Receiver, Sender},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const SEEK_STEP: f32 = 5.0;
const VOLUME_STEP: f32 = 0.05;

pub enum Action {
    Play(usize),
    Toggle,
    Next,
    Prev,
    Seek(f32),
    SeekRel(f32),
    Volume(f32),
    OpenFolder,
    Shuffle,
    CycleRepeat,
    SetIpod(bool),
    Library,
    Rescan,
    AddFiles,
    Themes,
    SpRoot(Node),
    SpPush(Node),
    SpBack,
    SpPlay(usize),
    SpotifyWindow,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum View {
    Library,
    Spotify,
}

pub enum SpMsg {
    Page(Node, Result<(Vec<ipod::Item>, Vec<Track>), String>),
    Connected(Result<(spotify::Client, spotify::Engine, String), String>),
}

pub struct App {
    ctx: egui::Context,
    player: Option<Playback>,
    cfg: Config,
    cfg_dirty: bool,
    error: Option<String>,
    /// Full local library.
    local: Vec<Track>,
    /// The active list / queue (local library, an album, a Spotify playlist...).
    tracks: Vec<Track>,
    list_is_local: bool,
    filtered: Vec<usize>,
    query: String,
    last_query: String,
    search_focus: bool,
    current: Option<usize>,
    history: Vec<usize>,
    scan_rx: Option<Receiver<ScanMsg>>,
    scan_count: usize,
    drag_pos: Option<f32>,
    window_title: String,
    rng: u64,
    ipod: IpodState,
    sp: Option<spotify::Client>,
    sp_tx: Sender<SpMsg>,
    sp_rx: Receiver<SpMsg>,
    sp_window: bool,
    sp_focus: bool,
    sp_status: String,
    was_focused: bool,
    themes_open: bool,
    theme_focus: bool,
    theme_name: String,
    theme_list: Vec<Theme>,
    bg_tex: Option<egui::TextureHandle>,
    bg_rx: Option<Receiver<Result<egui::ColorImage, String>>>,
    bg_path: Option<String>,
    cjk_loaded: bool,
    cjk_needed: bool,
    fonts_check: bool,
    art: ArtCache,
    view: View,
    sp_nav: Vec<ipod::Page>,
    sp_interactive: bool,
    window_ipod: bool,
    prev_size: Option<egui::Vec2>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let cfg = Config::load();
        let (player, error) = match Playback::new(cfg.volume) {
            Ok(p) => (Some(p), None),
            Err(e) => (None, Some(e)),
        };
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9_7F4A_7C15)
            | 1;
        let (sp_tx, sp_rx) = channel();
        let mut app = Self {
            ctx: cc.egui_ctx.clone(),
            player,
            cfg,
            cfg_dirty: false,
            error,
            local: Vec::new(),
            tracks: Vec::new(),
            list_is_local: true,
            filtered: Vec::new(),
            query: String::new(),
            last_query: String::new(),
            search_focus: false,
            current: None,
            history: Vec::new(),
            scan_rx: None,
            scan_count: 0,
            drag_pos: None,
            window_title: String::new(),
            rng: seed,
            ipod: IpodState::default(),
            sp: None,
            sp_tx,
            sp_rx,
            sp_window: false,
            sp_focus: false,
            sp_status: String::new(),
            was_focused: true,
            themes_open: false,
            theme_focus: false,
            theme_name: String::new(),
            theme_list: theme::all_themes(),
            bg_tex: None,
            bg_rx: None,
            bg_path: None,
            cjk_loaded: false,
            cjk_needed: false,
            fonts_check: false,
            art: ArtCache::new(),
            view: View::Library,
            sp_nav: Vec::new(),
            sp_interactive: false,
            window_ipod: false,
            prev_size: None,
        };
        app.apply_theme(&cc.egui_ctx);
        app.load_background(&cc.egui_ctx);
        let main = app.build_page(&Node::Main);
        app.ipod.stack.push(main);

        if let Some(dir) = app.cfg.last_folder.clone() {
            if dir.is_dir() {
                app.start_scan(dir, &cc.egui_ctx);
            }
        }
        if spotify::ENABLED && !app.cfg.spotify_client_id.trim().is_empty() {
            if let Some(rt) = app.cfg.spotify_refresh_token.clone() {
                app.start_spotify(Some(rt));
            }
        }
        app
    }

    // ---- library -----------------------------------------------------------

    fn start_scan(&mut self, dir: PathBuf, ctx: &egui::Context) {
        self.scan_rx = Some(library::spawn_scan(dir.clone(), ctx.clone(), self.known()));
        self.scan_count = 0;
        self.list_is_local = true;
        self.cfg.last_folder = Some(dir);
        self.cfg_dirty = true;
    }

    fn known(&self) -> HashMap<PathBuf, Track> {
        self.local
            .iter()
            .map(|t| (t.path.clone(), t.clone()))
            .collect()
    }

    /// Re-scans the saved folder in the background without touching the active queue.
    fn rescan(&mut self) {
        if self.scan_rx.is_some() {
            return;
        }
        let Some(dir) = self.cfg.last_folder.clone() else { return };
        if !dir.is_dir() {
            return;
        }
        self.scan_rx = Some(library::spawn_scan(dir, self.ctx.clone(), self.known()));
        self.scan_count = 0;
    }

    fn poll_scan(&mut self) {
        let Some(rx) = &self.scan_rx else { return };
        let mut done: Option<Vec<Track>> = None;
        while let Ok(msg) = rx.try_recv() {
            match msg {
                ScanMsg::Progress(n) => self.scan_count = n,
                ScanMsg::Done(v) => done = Some(v),
            }
        }
        if let Some(v) = done {
            self.scan_rx = None;
            self.local = v;
            self.fonts_check = true;
            if self.list_is_local {
                let current_path = self
                    .current
                    .and_then(|i| self.tracks.get(i))
                    .map(|t| t.path.clone());
                self.tracks = self.local.clone();
                self.history.clear();
                self.refilter();
                self.current = current_path
                    .and_then(|p| self.tracks.iter().position(|t| t.path == p));
            }
        }
    }

    fn refilter(&mut self) {
        let q = self.query.trim().to_lowercase();
        self.filtered = self
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| q.is_empty() || t.search_key.contains(&q))
            .map(|(i, _)| i)
            .collect();
        self.last_query = self.query.clone();
    }

    /// Replaces the active list/queue and starts playing `start`.
    fn set_list(&mut self, list: Vec<Track>, start: usize, is_local: bool) {
        self.tracks = list;
        self.list_is_local = is_local;
        self.history.clear();
        self.current = None;
        self.query.clear();
        self.refilter();
        self.fonts_check = true;
        self.play_index(start, true);
    }

    fn use_local(&mut self) {
        self.tracks = self.local.clone();
        self.list_is_local = true;
        self.history.clear();
        self.current = None;
        self.query.clear();
        self.refilter();
        self.fonts_check = true;
    }

    fn handle_drops(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if dropped.is_empty() {
            return;
        }
        let mut media = Vec::new();
        for p in dropped {
            let ext = p
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase())
                .unwrap_or_default();
            match ext.as_str() {
                "png" | "jpg" | "jpeg" | "webp" => {
                    self.cfg.theme.background = Some(p.to_string_lossy().into_owned());
                    self.theme_changed(ctx);
                }
                "json" => {
                    if let Some(t) = theme::import_theme(&p) {
                        self.cfg.theme = t;
                        self.theme_list = theme::all_themes();
                        self.theme_changed(ctx);
                    }
                }
                _ => media.push(p),
            }
        }
        if !media.is_empty() {
            self.add_paths(media, ctx);
        }
    }

    /// Adds dropped/picked files (or scans a dropped folder) and plays the first new track.
    fn add_paths(&mut self, paths: Vec<PathBuf>, ctx: &egui::Context) {
        let mut added = Vec::new();
        for p in paths {
            if p.is_dir() {
                self.start_scan(p, ctx);
                return;
            }
            if library::is_audio(&p) && !self.local.iter().any(|t| t.path == p) {
                added.push(library::read_track(&p));
            }
        }
        if !added.is_empty() {
            let first = self.local.len();
            self.local.extend(added);
            self.set_list(self.local.clone(), first, true);
        }
    }

    // ---- spotify -----------------------------------------------------------

    /// Logs in (interactive when `refresh` is None) and starts the playback engine.
    fn start_spotify(&mut self, refresh: Option<String>) {
        let id = self.cfg.spotify_client_id.trim().to_string();
        if id.is_empty() {
            self.sp_status = "Paste your Client ID first.".to_string();
            return;
        }
        self.sp_interactive = refresh.is_none();
        self.sp_status = "Connecting... finish the login in your browser.".to_string();
        let tx = self.sp_tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let res = (|| -> Result<(spotify::Client, spotify::Engine, String), String> {
                let client = match &refresh {
                    Some(r) => spotify::Client::from_refresh(&id, r)?,
                    None => spotify::Client::login(&id)?,
                };
                let engine = spotify::Engine::connect(&client.access_token()?)?;
                let rt = client.refresh_token();
                Ok((client, engine, rt))
            })();
            let _ = tx.send(SpMsg::Connected(res));
            ctx.request_repaint();
        });
    }

    fn poll_sp(&mut self) {
        while let Ok(msg) = self.sp_rx.try_recv() {
            match msg {
                SpMsg::Page(node, res) => self.ipod_receive(node, res),
                SpMsg::Connected(Ok((client, engine, refresh))) => {
                    self.sp = Some(client);
                    if let Some(p) = self.player.as_mut() {
                        p.set_spotify(engine);
                    }
                    self.cfg.spotify_refresh_token = Some(refresh);
                    self.cfg_dirty = true;
                    self.sp_status = "Connected.".to_string();
                    self.refresh_top();
                    if self.sp_interactive {
                        self.sp_interactive = false;
                        self.view = View::Spotify;
                        self.sp_nav.clear();
                        self.sp_push(Node::Spotify);
                    }
                }
                SpMsg::Connected(Err(e)) => {
                    self.sp_status = e.clone();
                    self.error = Some(e);
                }
            }
        }
    }

    fn disconnect_spotify(&mut self) {
        self.sp = None;
        if let Some(p) = self.player.as_mut() {
            p.drop_spotify();
        }
        self.cfg.spotify_refresh_token = None;
        self.cfg_dirty = true;
        self.sp_status = "Disconnected.".to_string();
        self.ipod.stack.truncate(1);
        self.sp_nav.clear();
        self.view = View::Library;
    }

    fn spotify_window(&mut self, ctx: &egui::Context) {
        if !self.sp_window {
            self.sp_focus = false;
            return;
        }
        let mut open = true;
        let mut connect = false;
        let mut changed = false;
        let mut focus = false;
        let mut open_log = false;
        let mut disconnect = false;
        egui::Window::new("Spotify")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                if !spotify::ENABLED {
                    ui.label("This build has no Spotify support.");
                    ui.label("Rebuild with: cargo run --release --features spotify");
                    return;
                }
                ui.label("Spotify only lets apps read your library through an app you register:");
                ui.label("1. Create an app at developer.spotify.com/dashboard");
                ui.label("2. Add this Redirect URI to it:");
                ui.code(spotify::REDIRECT_URI);
                ui.label("3. Paste the app's Client ID:");
                let r = ui.text_edit_singleline(&mut self.cfg.spotify_client_id);
                changed = r.changed();
                focus = r.has_focus();
                ui.horizontal(|ui| {
                    if ui.button("Connect").clicked() {
                        connect = true;
                    }
                    if self.sp.is_some() {
                        ui.label("Connected");
                    }
                });
                if !self.sp_status.is_empty() {
                    ui.weak(&self.sp_status);
                }
                ui.weak("Playback requires Spotify Premium.");
                let last = spotify::last_log();
                if !last.is_empty() {
                    ui.weak(format!("Last log: {last}"));
                }
                if let Some(p) = self.player.as_ref() {
                    if !p.sp_info().is_empty() {
                        ui.weak(format!("Last event: {}", p.sp_info()));
                    }
                }
                ui.horizontal(|ui| {
                    if ui.button("Open log folder").clicked() {
                        open_log = true;
                    }
                    if self.sp.is_some() && ui.button("Disconnect").clicked() {
                        disconnect = true;
                    }
                });
            });
        self.sp_focus = focus;
        if changed {
            self.cfg_dirty = true;
        }
        if connect {
            self.start_spotify(None);
        }
        if open_log {
            if let Some(dir) = spotify::log_path().as_deref().and_then(|p| p.parent()) {
                theme::open_folder(dir);
            }
        }
        if disconnect {
            self.disconnect_spotify();
        }
        if !open {
            self.sp_window = false;
        }
    }

    // ---- playback ----------------------------------------------------------

    // ---- theme, wallpaper, fonts --------------------------------------------

    fn apply_theme(&mut self, ctx: &egui::Context) {
        self.cfg.theme.apply(ctx, self.bg_tex.is_some());
    }

    fn theme_changed(&mut self, ctx: &egui::Context) {
        self.cfg_dirty = true;
        self.load_background(ctx);
        self.apply_theme(ctx);
    }

    /// Decodes the wallpaper on a worker thread. Larger images are downscaled (Lanczos) to the
    /// chosen quality: 1280x720 (~4 MB), 1920x1080 (~8 MB, default), 2560x1440 (~15 MB), 3840x2160 (~33 MB).
    fn load_background(&mut self, ctx: &egui::Context) {
        let want = self.cfg.theme.background.clone();
        if want == self.bg_path {
            return;
        }
        self.bg_path = want.clone();
        match want {
            None => {
                self.bg_tex = None;
                self.bg_rx = None;
            }
            Some(path) => {
                let (tx, rx) = channel();
                let ctx = ctx.clone();
                let (max_w, max_h) = match self.cfg.wallpaper_quality {
                    0 => (1280, 720),
                    2 => (2560, 1440),
                    3 => (3840, 2160),
                    _ => (1920, 1080),
                };
                std::thread::spawn(move || {
                    let res = (|| -> Result<egui::ColorImage, String> {
                        let img = image::open(&path).map_err(|e| e.to_string())?;
                        let img = if img.width() > max_w || img.height() > max_h {
                            img.resize(max_w, max_h, image::imageops::FilterType::Lanczos3)
                        } else {
                            img
                        };
                        let rgba = img.to_rgba8();
                        let size = [rgba.width() as usize, rgba.height() as usize];
                        Ok(egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()))
                    })();
                    let _ = tx.send(res);
                    ctx.request_repaint();
                });
                self.bg_rx = Some(rx);
            }
        }
    }

    fn poll_background(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.bg_rx else { return };
        match rx.try_recv() {
            Ok(Ok(img)) => {
                self.bg_tex = Some(ctx.load_texture("wallpaper", img, egui::TextureOptions::LINEAR));
                self.bg_rx = None;
                self.apply_theme(ctx);
            }
            Ok(Err(e)) => {
                self.error = Some(format!("Wallpaper: {e}"));
                self.bg_rx = None;
            }
            Err(_) => {}
        }
    }

    /// Paints the theme background (and wallpaper) behind every panel.
    fn paint_background(&self, ctx: &egui::Context) {
        let rect = ctx.screen_rect();
        let painter = ctx.layer_painter(egui::LayerId::background());
        painter.rect_filled(rect, 0.0, self.cfg.theme.bg());
        if let Some(tex) = &self.bg_tex {
            let [w, h] = tex.size();
            let (tw, th) = (w as f32, h as f32);
            let scale = (rect.width() / tw).max(rect.height() / th);
            let vis_w = rect.width() / scale;
            let vis_h = rect.height() / scale;
            let u0 = (tw - vis_w) / 2.0 / tw;
            let v0 = (th - vis_h) / 2.0 / th;
            let uv = Rect::from_min_max(pos2(u0, v0), pos2(1.0 - u0, 1.0 - v0));
            painter.image(tex.id(), rect, uv, Color32::WHITE);
            painter.rect_filled(rect, 0.0, Color32::from_black_alpha(self.cfg.theme.dim));
        }
    }

    /// Cover texture for the current track, if it has one and it has loaded.
    pub(crate) fn current_art(&mut self, ctx: &egui::Context, size: u32) -> Option<egui::TextureId> {
        let key = self
            .current
            .and_then(|i| self.tracks.get(i))
            .and_then(|t| t.art_key())?;
        self.art.get(ctx, &key, size)
    }

    fn rand(&mut self, n: usize) -> usize {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng % n.max(1) as u64) as usize
    }

    fn position_secs(&self) -> f32 {
        self.player
            .as_ref()
            .map_or(0.0, |p| p.position().as_secs_f32())
    }

    fn play_index(&mut self, i: usize, record: bool) {
        let Some(track) = self.tracks.get(i) else { return };
        let Some(player) = self.player.as_mut() else { return };
        match player.load(track, Duration::ZERO, true) {
            Ok(()) => {
                if record {
                    if let Some(c) = self.current {
                        if c != i {
                            self.history.push(c);
                            if self.history.len() > 200 {
                                self.history.remove(0);
                            }
                        }
                    }
                }
                self.current = Some(i);
                self.error = None;
            }
            Err(e) => self.error = Some(format!("{}: {e}", track.title)),
        }
    }

    fn next_index(&mut self, auto: bool) -> Option<usize> {
        if self.filtered.is_empty() {
            return None;
        }
        if auto && self.cfg.repeat == Repeat::One {
            return self.current;
        }
        if self.cfg.shuffle {
            let n = self.filtered.len();
            let k = self.rand(n);
            return Some(self.filtered[k]);
        }
        let pos = self
            .current
            .and_then(|c| self.filtered.iter().position(|&i| i == c));
        match pos {
            Some(p) if p + 1 < self.filtered.len() => Some(self.filtered[p + 1]),
            Some(_) if auto && self.cfg.repeat == Repeat::Off => None,
            _ => Some(self.filtered[0]),
        }
    }

    fn prev(&mut self) {
        if self.position_secs() > 3.0 {
            if let Some(i) = self.current {
                self.play_index(i, false);
            }
            return;
        }
        if let Some(i) = self.history.pop() {
            self.play_index(i, false);
            return;
        }
        if self.filtered.is_empty() {
            return;
        }
        let pos = self
            .current
            .and_then(|c| self.filtered.iter().position(|&i| i == c));
        let i = match pos {
            Some(p) if p > 0 => self.filtered[p - 1],
            _ => self.filtered[self.filtered.len() - 1],
        };
        self.play_index(i, false);
    }

    fn toggle(&mut self) {
        let loaded = self.player.as_ref().map_or(false, |p| p.is_loaded());
        if loaded {
            if let Some(p) = self.player.as_mut() {
                p.toggle();
            }
        } else if let Some(i) = self.current.or_else(|| self.filtered.first().copied()) {
            self.play_index(i, true);
        }
    }

    fn seek(&mut self, secs: f32) {
        let (Some(i), Some(p)) = (self.current, self.player.as_mut()) else {
            return;
        };
        if !p.is_loaded() {
            return;
        }
        let Some(track) = self.tracks.get(i) else { return };
        let dur = track.duration.as_secs_f32();
        let mut s = secs.max(0.0);
        if dur > 0.0 {
            s = s.min((dur - 0.5).max(0.0));
        }
        let playing = p.is_playing();
        if let Err(e) = p.load(track, Duration::from_secs_f32(s), playing) {
            self.error = Some(e);
        }
    }

    fn on_finished(&mut self) {
        match self.next_index(true) {
            Some(i) => self.play_index(i, true),
            None => {
                if let Some(p) = self.player.as_mut() {
                    p.stop();
                }
            }
        }
    }

    fn apply(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::Play(i) => self.play_index(i, true),
            Action::Toggle => self.toggle(),
            Action::Next => {
                if let Some(i) = self.next_index(false) {
                    self.play_index(i, true);
                }
            }
            Action::Prev => self.prev(),
            Action::Seek(s) => self.seek(s),
            Action::SeekRel(d) => {
                let pos = self.position_secs();
                self.seek(pos + d);
            }
            Action::Volume(v) => {
                let v = v.clamp(0.0, 1.0);
                self.cfg.volume = v;
                if let Some(p) = self.player.as_mut() {
                    p.set_volume(v);
                }
                self.cfg_dirty = true;
            }
            Action::OpenFolder => {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    self.start_scan(dir, ctx);
                }
            }
            Action::Shuffle => {
                self.cfg.shuffle = !self.cfg.shuffle;
                self.cfg_dirty = true;
            }
            Action::CycleRepeat => {
                self.cfg.repeat = match self.cfg.repeat {
                    Repeat::Off => Repeat::All,
                    Repeat::All => Repeat::One,
                    Repeat::One => Repeat::Off,
                };
                self.cfg_dirty = true;
            }
            Action::SetIpod(on) => {
                self.cfg.ipod_mode = on;
                self.cfg_dirty = true;
            }
            Action::Library => {
                self.view = View::Library;
                self.use_local();
            }
            Action::Themes => self.themes_open = true,
            Action::SpRoot(n) => {
                self.view = View::Spotify;
                self.sp_nav.clear();
                self.sp_push(n);
            }
            Action::SpPush(n) => self.sp_push(n),
            Action::SpBack => {
                if self.sp_nav.len() > 1 {
                    self.sp_nav.pop();
                }
            }
            Action::SpPlay(i) => {
                let list = self.sp_nav.last().map(|p| p.tracks.clone()).unwrap_or_default();
                if !list.is_empty() {
                    self.set_list(list, i, false);
                }
            }
            Action::Rescan => self.rescan(),
            Action::AddFiles => {
                if let Some(paths) = rfd::FileDialog::new()
                    .add_filter("Audio", library::EXTENSIONS)
                    .pick_files()
                {
                    self.add_paths(paths, ctx);
                }
            }
            Action::SpotifyWindow => self.sp_window = true,
        }
    }

    // ---- classic UI --------------------------------------------------------

    fn classic_keys(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let keys = ctx.input_mut(|i| {
            [
                i.consume_key(Modifiers::NONE, Key::Space),
                i.consume_key(Modifiers::NONE, Key::ArrowRight),
                i.consume_key(Modifiers::NONE, Key::ArrowLeft),
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::N),
                i.consume_key(Modifiers::NONE, Key::P),
                i.consume_key(Modifiers::NONE, Key::S),
                i.consume_key(Modifiers::NONE, Key::R),
            ]
        });
        if keys[0] {
            actions.push(Action::Toggle);
        }
        if keys[1] {
            actions.push(Action::SeekRel(SEEK_STEP));
        }
        if keys[2] {
            actions.push(Action::SeekRel(-SEEK_STEP));
        }
        if keys[3] {
            actions.push(Action::Volume(self.cfg.volume + VOLUME_STEP));
        }
        if keys[4] {
            actions.push(Action::Volume(self.cfg.volume - VOLUME_STEP));
        }
        if keys[5] {
            actions.push(Action::Next);
        }
        if keys[6] {
            actions.push(Action::Prev);
        }
        if keys[7] {
            actions.push(Action::Shuffle);
        }
        if keys[8] {
            actions.push(Action::CycleRepeat);
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_scan();
        self.poll_sp();
        self.art.poll(ctx);
        if let Some(p) = self.player.as_mut() {
            for m in p.poll() {
                self.error = Some(m);
            }
        }
        self.poll_background(ctx);
        if self.fonts_check && !self.cjk_loaded {
            self.fonts_check = false;
            let need = self.cjk_needed
                || self
                    .local
                    .iter()
                    .chain(self.tracks.iter())
                    .any(|t| has_cjk(&t.title) || has_cjk(&t.artist) || has_cjk(&t.album));
            if need {
                install_fallback_fonts(ctx);
                self.cjk_loaded = true;
            }
        }
        let focused = ctx.input(|i| i.focused);
        if focused && !self.was_focused {
            self.rescan();
        }
        self.was_focused = focused;
        self.handle_drops(ctx);
        if self.player.as_ref().map_or(false, |p| p.finished()) {
            self.on_finished();
        }

        // The iPod is a compact, borderless window; leaving iPod mode restores the full window.
        if self.cfg.ipod_mode != self.window_ipod {
            self.window_ipod = self.cfg.ipod_mode;
            if self.window_ipod {
                self.prev_size = Some(ctx.screen_rect().size());
                ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(egui::vec2(300.0, 480.0)));
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(360.0, 680.0)));
                ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(false));
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(egui::vec2(420.0, 480.0)));
                let size = self.prev_size.unwrap_or(egui::vec2(1000.0, 640.0));
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            }
        }

        self.paint_background(ctx);

        let mut actions: Vec<Action> = Vec::new();

        if self.cfg.ipod_mode {
            self.search_focus = false;
            if !self.sp_focus && !self.theme_focus {
                self.ipod_keys(ctx);
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::none())
                .show(ctx, |ui| self.ipod_ui(ui));
        } else {
            if !self.search_focus && !self.sp_focus && !self.theme_focus {
                self.classic_keys(ctx, &mut actions);
            }
            self.classic_ui(ctx, &mut actions);
        }

        self.spotify_window(ctx);
        self.themes_window(ctx);

        for a in actions {
            self.apply(a, ctx);
        }
        if self.query != self.last_query {
            self.refilter();
        }

        // Window title follows the current track.
        let want = match self.current.and_then(|i| self.tracks.get(i)) {
            Some(t) => format!("{} - {}", t.title, t.artist),
            None => "litewave".to_string(),
        };
        if want != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(want.clone()));
            self.window_title = want;
        }

        // Debounced config save (not while a slider is being dragged).
        if self.cfg_dirty && !ctx.input(|i| i.pointer.any_down()) {
            self.cfg.save();
            self.cfg_dirty = false;
        }

        if self.player.as_ref().map_or(false, |p| p.is_playing()) {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }
}

pub(crate) fn has_cjk(s: &str) -> bool {
    s.chars().any(|c| {
        matches!(
            c as u32,
            0x2E80..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF
        )
    })
}

/// egui's built-in font has no CJK glyphs, so Japanese/Chinese/Korean titles show as boxes.
/// These system fonts are only loaded when a loaded song actually needs them (about 10-25 MB),
/// so a Latin-only library pays nothing.
fn install_fallback_fonts(ctx: &egui::Context) {
    let groups: [&[&str]; 2] = [
        &[
            r"C:\Windows\Fonts\YuGothM.ttc",
            r"C:\Windows\Fonts\msyh.ttc",
            "/System/Library/Fonts/PingFang.ttc",
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        ],
        &[
            r"C:\Windows\Fonts\malgun.ttf",
            "/System/Library/Fonts/AppleSDGothicNeo.ttc",
        ],
    ];
    let mut fonts = egui::FontDefinitions::default();
    let mut added = false;
    for (g, group) in groups.iter().enumerate() {
        for path in group.iter() {
            if let Ok(bytes) = std::fs::read(path) {
                let name = format!("cjk{g}");
                fonts
                    .font_data
                    .insert(name.clone(), egui::FontData::from_owned(bytes));
                for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                    fonts.families.entry(family).or_default().push(name.clone());
                }
                added = true;
                break;
            }
        }
    }
    if added {
        ctx.set_fonts(fonts);
    }
}

pub(crate) fn fmt_time(secs: f32) -> String {
    let s = secs.max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::fmt_time;

    #[test]
    fn formats_time() {
        assert_eq!(fmt_time(0.0), "0:00");
        assert_eq!(fmt_time(65.4), "1:05");
        assert_eq!(fmt_time(3725.0), "1:02:05");
        assert_eq!(fmt_time(-3.0), "0:00");
    }
}
