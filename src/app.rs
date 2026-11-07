mod ipod;

use crate::config::{Config, Repeat};
use crate::library::{self, ScanMsg, Track};
use crate::player::Playback;
use crate::spotify;
use eframe::egui::{self, Align2, Color32, Key, Modifiers, Rect, Sense, TextStyle};
use ipod::{IpodState, Node};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::mpsc::{channel, Receiver, Sender},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const ROW_H: f32 = 26.0;
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
    SpotifyWindow,
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
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        apply_theme(&cc.egui_ctx);
        install_fallback_fonts(&cc.egui_ctx);
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
        };
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
        self.play_index(start, true);
    }

    fn use_local(&mut self) {
        self.tracks = self.local.clone();
        self.list_is_local = true;
        self.history.clear();
        self.current = None;
        self.query.clear();
        self.refilter();
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
        self.add_paths(dropped, ctx);
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
            });
        self.sp_focus = focus;
        if changed {
            self.cfg_dirty = true;
        }
        if connect {
            self.start_spotify(None);
        }
        if !open {
            self.sp_window = false;
        }
    }

    // ---- playback ----------------------------------------------------------

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
            Action::Library => self.use_local(),
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

    fn classic_ui(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let (title, artist, duration) = self
            .current
            .and_then(|i| self.tracks.get(i))
            .map(|t| (t.title.clone(), t.artist.clone(), t.duration))
            .unwrap_or_default();
        let pos = self.position_secs();
        let playing = self.player.as_ref().map_or(false, |p| p.is_playing());
        let loaded = self.player.as_ref().map_or(false, |p| p.is_loaded());
        let dur_s = duration.as_secs_f32();
        let mut dismiss_error = false;

        egui::TopBottomPanel::bottom("player")
            .min_height(104.0)
            .show(ctx, |ui| {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if title.is_empty() {
                        ui.weak("Nothing playing");
                    } else {
                        ui.label(egui::RichText::new(&title).strong().size(15.0));
                        ui.weak(&artist);
                    }
                });
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    if ui.button("Prev").clicked() {
                        actions.push(Action::Prev);
                    }
                    let label = if playing { "Pause" } else { "Play" };
                    if ui
                        .add_sized([64.0, 24.0], egui::Button::new(label))
                        .clicked()
                    {
                        actions.push(Action::Toggle);
                    }
                    if ui.button("Next").clicked() {
                        actions.push(Action::Next);
                    }
                    ui.separator();

                    let shown = self.drag_pos.unwrap_or(pos);
                    ui.monospace(fmt_time(shown));
                    ui.spacing_mut().slider_width = (ui.available_width() - 70.0).max(60.0);
                    let mut v = shown.min(dur_s.max(0.1));
                    let resp = ui.add_enabled(
                        loaded && dur_s > 0.0,
                        egui::Slider::new(&mut v, 0.0..=dur_s.max(0.1))
                            .show_value(false)
                            .trailing_fill(true),
                    );
                    if resp.changed() {
                        self.drag_pos = Some(v);
                    }
                    if resp.drag_stopped() || resp.clicked() {
                        if let Some(p) = self.drag_pos.take() {
                            actions.push(Action::Seek(p));
                        }
                    }
                    ui.monospace(fmt_time(dur_s));
                });
                ui.horizontal(|ui| {
                    if ui.selectable_label(self.cfg.shuffle, "Shuffle").clicked() {
                        actions.push(Action::Shuffle);
                    }
                    let rep_label = match self.cfg.repeat {
                        Repeat::Off => "Repeat",
                        Repeat::All => "Repeat all",
                        Repeat::One => "Repeat one",
                    };
                    if ui
                        .selectable_label(self.cfg.repeat != Repeat::Off, rep_label)
                        .clicked()
                    {
                        actions.push(Action::CycleRepeat);
                    }
                    ui.separator();
                    ui.weak("Vol");
                    ui.spacing_mut().slider_width = 110.0;
                    let mut vol = self.cfg.volume;
                    if ui
                        .add(egui::Slider::new(&mut vol, 0.0..=1.0).show_value(false))
                        .changed()
                    {
                        actions.push(Action::Volume(vol));
                    }
                });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui
                    .button("Open folder")
                    .on_hover_text("Pick the folder that holds your music. The picker only lists folders, not files.")
                    .clicked()
                {
                    actions.push(Action::OpenFolder);
                }
                if ui
                    .button("Add songs")
                    .on_hover_text("Pick individual audio files")
                    .clicked()
                {
                    actions.push(Action::AddFiles);
                }
                if ui.button("Library").clicked() {
                    actions.push(Action::Library);
                }
                if ui.button("Rescan").clicked() {
                    actions.push(Action::Rescan);
                }
                if ui.button("iPod mode").clicked() {
                    actions.push(Action::SetIpod(true));
                }
                if ui.button("Spotify").clicked() {
                    actions.push(Action::SpotifyWindow);
                }
            });
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .hint_text("Search title, artist, album")
                    .desired_width(f32::INFINITY),
            );
            self.search_focus = r.has_focus();
            ui.horizontal(|ui| {
                if self.scan_rx.is_some() {
                    ui.weak(format!("Scanning... {} tracks", self.scan_count));
                } else {
                    let n = self.filtered.len();
                    ui.weak(format!("{} track{}", n, if n == 1 { "" } else { "s" }));
                }
                if let Some(err) = &self.error {
                    ui.colored_label(Color32::LIGHT_RED, err);
                    if ui.small_button("dismiss").clicked() {
                        dismiss_error = true;
                    }
                }
            });
            ui.add_space(4.0);

            if self.tracks.is_empty() && self.scan_rx.is_none() {
                ui.centered_and_justified(|ui| {
                    ui.weak("Drop a folder or audio files here, or click Open folder");
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
                                egui::vec2(ui.available_width(), ROW_H),
                                Sense::click(),
                            );
                            let selected = Some(i) == self.current;
                            if selected {
                                ui.painter()
                                    .rect_filled(rect, 4.0, ui.visuals().selection.bg_fill);
                            } else if resp.hovered() {
                                ui.painter().rect_filled(
                                    rect,
                                    4.0,
                                    ui.visuals().widgets.hovered.weak_bg_fill,
                                );
                            }
                            let font = TextStyle::Body.resolve(ui.style());
                            let main = if selected {
                                ui.visuals().strong_text_color()
                            } else {
                                ui.visuals().text_color()
                            };
                            let weak = if selected {
                                Color32::from_rgb(222, 238, 234)
                            } else {
                                ui.visuals().weak_text_color()
                            };
                            let cy = rect.center().y;
                            let col2 = rect.min.x + rect.width() * 0.55;
                            let right = rect.max.x - 8.0;
                            let painter = ui.painter();

                            painter
                                .with_clip_rect(Rect::from_min_max(
                                    rect.min,
                                    egui::pos2(col2 - 8.0, rect.max.y),
                                ))
                                .text(
                                    egui::pos2(rect.min.x + 8.0, cy),
                                    Align2::LEFT_CENTER,
                                    &t.title,
                                    font.clone(),
                                    main,
                                );
                            painter
                                .with_clip_rect(Rect::from_min_max(
                                    egui::pos2(col2, rect.min.y),
                                    egui::pos2(right - 56.0, rect.max.y),
                                ))
                                .text(
                                    egui::pos2(col2, cy),
                                    Align2::LEFT_CENTER,
                                    &t.artist,
                                    font.clone(),
                                    weak,
                                );
                            painter.text(
                                egui::pos2(right, cy),
                                Align2::RIGHT_CENTER,
                                fmt_time(t.duration.as_secs_f32()),
                                font,
                                weak,
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

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_scan();
        self.poll_sp();
        let focused = ctx.input(|i| i.focused);
        if focused && !self.was_focused {
            self.rescan();
        }
        self.was_focused = focused;
        self.handle_drops(ctx);
        if self.player.as_ref().map_or(false, |p| p.finished()) {
            self.on_finished();
        }

        let mut actions: Vec<Action> = Vec::new();

        if self.cfg.ipod_mode {
            self.search_focus = false;
            if !self.sp_focus {
                self.ipod_keys(ctx);
            }
            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(Color32::from_rgb(22, 22, 26)))
                .show(ctx, |ui| self.ipod_ui(ui));
        } else {
            if !self.search_focus && !self.sp_focus {
                self.classic_keys(ctx, &mut actions);
            }
            self.classic_ui(ctx, &mut actions);
        }

        self.spotify_window(ctx);

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

/// egui's built-in font has no CJK glyphs, so song titles in Japanese/Chinese/Korean show as boxes.
/// Load whichever system fonts exist and use them as fallbacks.
fn install_fallback_fonts(ctx: &egui::Context) {
    let candidates = [
        r"C:\Windows\Fonts\YuGothM.ttc",
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\malgun.ttf",
        r"C:\Windows\Fonts\seguisym.ttf",
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/AppleSDGothicNeo.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
    ];
    let mut fonts = egui::FontDefinitions::default();
    let mut added = false;
    for (i, path) in candidates.iter().enumerate() {
        if let Ok(bytes) = std::fs::read(path) {
            let name = format!("fallback{i}");
            fonts
                .font_data
                .insert(name.clone(), egui::FontData::from_owned(bytes));
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts.families.entry(family).or_default().push(name.clone());
            }
            added = true;
        }
    }
    if added {
        ctx.set_fonts(fonts);
    }
}

fn apply_theme(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    let accent = Color32::from_rgb(0, 168, 150);
    v.selection.bg_fill = accent.gamma_multiply(0.55);
    v.selection.stroke.color = accent;
    v.hyperlink_color = accent;
    ctx.set_visuals(v);
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
