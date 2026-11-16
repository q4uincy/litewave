mod classic;
mod home;
mod ipod;
mod settings;
mod themes;
mod widgets;

use crate::config::{Config, Repeat, ShuffleMode};
use crate::art::ArtCache;
use crate::library::{self, ScanMsg, Track};
use crate::lyrics::{self, Lyrics, LyricsResult, Status as LyricsStatus};
use crate::player::Playback;
use crate::spotify;
use crate::store::{self, StoreItem};
use crate::ytdlp;
use crate::theme::{self, Theme};
use eframe::egui::{self, pos2, Color32, Key, Modifiers, Rect};
use ipod::{IpodState, Load, Node};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{channel, Receiver, Sender},
        Arc,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// Project links.
pub(crate) const REPO_URL: &str = "https://github.com/q4uincy/litewave";
pub(crate) const BUG_URL: &str = "https://github.com/q4uincy/litewave/issues/new";
pub(crate) const GUIDE_URL: &str = "https://q4uincy.github.io/litewave/setup-guide.html";

const SEEK_STEP: f32 = 5.0;
const VOLUME_STEP: f32 = 0.05;
/// How many songs the Recent tab remembers.
const RECENT_MAX: usize = 30;

pub enum Action {
    Play(usize),
    Toggle,
    Next,
    Prev,
    Seek(f32),
    SeekRel(f32),
    Volume(f32),
    OpenFolder,
    /// Cycles Off -> Shuffle -> Smart shuffle (the S key).
    Shuffle,
    SetShuffle(ShuffleMode),
    /// Start (or pause/resume) the list shown in the library view.
    PlayAll,
    /// Start the Spotify track list that is open.
    SpPlayAll,
    CycleRepeat,
    SetIpod(bool),
    Library,
    OpenYoutube,
    Rescan,
    AddFiles,
    Themes,
    Settings,
    SpRoot(Node),
    SpPush(Node),
    SpBack,
    SpPlay(usize),
    SpRefresh,
    /// A Spotify link opens that page; anything else is searched.
    SpLink(String),
    /// Search-as-you-type: replaces the open search page instead of adding to the history.
    SpLive(String),
    /// Forget the recent searches.
    ClearRecent,
    SpAddToPlaylist(String, Vec<String>),
    SpRemoveFromPlaylist(String, Vec<String>),
    SpSave(Vec<String>),
    SpUnsave(Vec<String>),
    SpDialog(Dialog),
    SpDialogOk,
    SpotifyWindow,
    ToggleLyrics,
    /// Opens or closes the lyrics panel in the main window.
    ToggleLyricsPanel,
    /// Opens or closes the full-screen lyrics view.
    ToggleLyricsFull,
    ToggleExplicit,
    /// Open the artist / album of a song row: (Spotify id, name, is a Spotify song).
    OpenArtist(Option<String>, String, bool),
    OpenAlbum(Option<String>, String, bool),
    /// Jump to the playlist / album / page the current song is playing from.
    GoToSource,
    /// Queue: play a song next / add it to the end / remove one / clear / show the page.
    QueueNext(Track),
    QueueAdd(Track),
    QueueRemove(usize),
    QueueClear,
    /// Opens or closes the Queue / Recent panel.
    OpenQueue,
    /// Plays a song from the Recent tab right away (the list it came from is kept for later).
    PlayRecent(usize),
    /// Plays a queued song now (it leaves the queue).
    QueuePlay(usize),
    /// Plays a song of Home's tiles (a song played on its own): its place in the recent list.
    PlayPlay(usize),
    /// Plays a song of Home's "Your top songs": (period, row).
    PlayTop(usize, usize),
    /// Asks for Spotify's own Home shelves again right now.
    RetryShelves,
    /// Shows the Home screen.
    Home,
    /// Opens a playlist / album / artist / Liked Songs from the Home screen (Back returns to Home).
    HomeOpen(Node),
    /// The "..." button of a song row: the song and where to open its menu.
    RowMenu(Track, egui::Pos2),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum View {
    Home,
    Library,
    Spotify,
    YouTube,
    Themes,
    Settings,
    Account,
}

/// Result of a Spotify sign-in: browsing client, playback engine, playback refresh token,
/// and the browsing refresh token (only when the user's own Client ID is used).
pub type SpLogin = (spotify::Client, Result<spotify::Engine, String>, String, Option<String>);

pub enum SpMsg {
    Page(Node, Result<(Vec<ipod::Item>, Vec<Track>), String>),
    Connected(Result<SpLogin, String>),
    /// The account's display name.
    UserName(String),
    /// Spotify ids of all Liked Songs.
    Liked(Vec<String>),
    /// An edit finished: a message to show and the page to reload (if any).
    Done(Result<String, String>, Option<Node>),
    /// The playlists the user is allowed to edit.
    Editable(Result<Vec<spotify::Entry>, String>),
    /// Details of a song the phone started that is not in litewave's list.
    Remote(String, Result<Track, String>),
    /// Spotify's own Home shelves. `Err` holds the seconds Spotify asked us to wait (0 = none).
    Shelves(Result<Vec<spotify::Shelf>, u64>),
    /// Ids of the playlists / albums / artists played from lately, newest first.
    RecentCtx(Result<(Vec<String>, Vec<spotify::Entry>, Vec<spotify::Entry>, Vec<spotify::RecentPlay>), String>),
    /// Your top songs for one period (0 = last 4 weeks, 1 = last 6 months, 2 = past year).
    TopSongs(usize, Result<Vec<Track>, String>),
}

/// Results from the theme gallery threads.
pub enum StoreMsg {
    Index(Result<Vec<StoreItem>, String>),
    Installed(Result<String, String>),
}

/// Results from the yt-dlp worker threads.
pub enum YtMsg {
    Results(Result<Vec<Track>, String>),
    /// A download finished: the video id and the cached file.
    Audio(String, Result<PathBuf, String>),
}

/// Small popup asking for a name or a confirmation.
#[derive(Clone, PartialEq)]
pub enum Dialog {
    NewPlaylist,
    Rename(String, String),
    Delete(String, String),
}

pub struct App {
    ctx: egui::Context,
    player: Option<Playback>,
    cfg: Config,
    cfg_dirty: bool,
    /// A settings save is running on a worker thread (Windows Credential Manager can be slow).
    cfg_saving: Arc<AtomicBool>,
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
    /// The song that is playing, kept even when browsing replaces the queue.
    now: Option<Track>,
    history: Vec<usize>,
    /// Songs already heard in the current shuffle round (indexes into `tracks`).
    shuffle_seen: HashSet<usize>,
    /// The Spotify page (and its name) the playing list was started from, for "go back to it".
    playing_from: Option<(Node, String)>,
    /// Where the shuffle menu is anchored while it is open.
    shuffle_menu: Option<Rect>,
    /// Playlists the user can edit (for "Add to playlist").
    pub(crate) sp_editable: Vec<spotify::Entry>,
    /// A short "done" message shown above the player bar for a few seconds.
    notice: Option<(String, Instant)>,
    dialog: Option<Dialog>,
    dialog_text: String,
    yt_tx: Sender<YtMsg>,
    yt_rx: Receiver<YtMsg>,
    /// The YouTube video id that is downloading and should start playing when it is ready.
    yt_wait: Option<String>,
    scan_rx: Option<Receiver<ScanMsg>>,
    scan_count: usize,
    drag_pos: Option<f32>,
    window_title: String,
    rng: u64,
    ipod: IpodState,
    sp: Option<spotify::Client>,
    /// Reads playlists the Web API refuses, through the playback connection.
    sp_session: Option<spotify::SessionHandle>,
    /// The last song the phone started (litewave id).
    remote_id: Option<String>,
    sp_tx: Sender<SpMsg>,
    sp_rx: Receiver<SpMsg>,
    sp_window: bool,
    /// iPod mode search box: (text, already focused once).
    pub(crate) ipod_search: Option<(String, bool)>,
    /// iPod "Shuffle Songs" is waiting for Liked Songs to load.
    pub(crate) shuffle_pending: bool,
    /// Sleep timer: when to pause, and the length chosen (minutes, 0 = off).
    pub(crate) sleep_at: Option<Instant>,
    pub(crate) sleep_mins: u32,
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
    logo_tex: Option<egui::TextureHandle>,
    cjk_loaded: bool,
    cjk_needed: bool,
    fonts_check: bool,
    art: ArtCache,
    view: View,
    sp_nav: Vec<ipod::Page>,
    /// Spotify ids of the songs in Liked Songs (drives the heart and the menus).
    pub(crate) liked_ids: HashSet<String>,
    sp_interactive: bool,
    pub(crate) sp_link: String,
    /// Text in the YouTube section's search box.
    pub(crate) yt_query: String,
    /// The last YouTube search results, kept so the YouTube tab can show them again.
    yt_list: Vec<Track>,
    media: Option<crate::mediakeys::MediaKeys>,
    tray: Option<crate::tray::Tray>,
    /// The window is hidden in the tray.
    tray_hidden: bool,
    pub(crate) store_items: Vec<StoreItem>,
    pub(crate) store_busy: bool,
    pub(crate) store_note: String,
    store_tx: Sender<StoreMsg>,
    store_rx: Receiver<StoreMsg>,
    /// Songs queued with "Play next" / "Add to queue".
    pub(crate) queue: Vec<Track>,
    /// The last few songs that were played, newest first (memory only, capped).
    pub(crate) recent: VecDeque<Track>,
    /// The Spotify account's display name (Home shows "Made For <name>").
    pub(crate) sp_user: String,
    /// Spotify's own Home shelves and when they were read (private feed; None until read).
    pub(crate) sp_shelves: Option<(Instant, Vec<spotify::Shelf>)>,
    /// Earliest time to ask for the shelves again (set after every request, longer after a failure).
    shelves_next: Option<Instant>,
    /// A shelves request is running.
    shelves_busy: bool,
    /// Spotify's recently played contexts (newest first) and when they were read.
    pub(crate) sp_recent: Option<(Instant, Vec<String>)>,
    recent_busy: bool,
    /// Albums of Spotify's recently played songs, and your top artists (Home shelves).
    pub(crate) sp_recent_albums: Vec<spotify::Entry>,
    pub(crate) sp_top_artists: Vec<spotify::Entry>,
    /// Spotify's recently played songs with where they were played from (newest first): Home's tiles.
    pub(crate) sp_recent_plays: Vec<spotify::RecentPlay>,
    /// Your top songs per period (4 weeks, 6 months, year), when each was read, and whether a request runs.
    pub(crate) sp_top_songs: [Option<(Instant, Vec<Track>)>; 3],
    top_songs_busy: [bool; 3],
    top_songs_tried: [Option<Instant>; 3],
    /// Which tab of the queue panel is shown: false = Queue, true = Recent.
    pub(crate) queue_recent_tab: bool,
    /// When the sidebar last asked for the playlist list (so a failed load is not retried every frame).
    pub(crate) list_fetch: HashMap<String, Instant>,
    /// Set when a page was opened from Home: that page's node, so Back can return to Home.
    pub(crate) home_back: Option<Node>,
    /// The list position to continue from after queued songs.
    queue_resume: Option<usize>,
    pub(crate) row_menu: Option<(Track, egui::Pos2, Instant)>,
    /// When the search box text last changed, and the last query sent while typing.
    pub(crate) sp_typed: Instant,
    pub(crate) sp_live: String,
    /// Recently loaded Spotify pages, so going back does not hit the rate-limited API again.
    pub(crate) page_cache: HashMap<String, (Instant, Vec<ipod::Item>, Vec<Track>)>,
    window_ipod: bool,
    prev_size: Option<egui::Vec2>,
    /// Lyrics of the song that is playing (None until found).
    lyrics: Option<Lyrics>,
    lyrics_status: LyricsStatus,
    lyrics_rx: Option<Receiver<LyricsResult>>,
    /// Identifies the song the lyrics belong to; empty when lyrics are off or nothing plays.
    lyrics_key: String,
    /// The lyric line the main window last scrolled to.
    lyric_shown: Option<usize>,
    /// Full-screen lyrics: open, the eased scroll position and its target (points), the line it
    /// follows, whether it follows the song (the mouse wheel pauses that until the next line),
    /// "jump instead of glide" for the next frame, and the song its state belongs to.
    pub(crate) lyrics_full: bool,
    pub(crate) lyr_cur: f32,
    pub(crate) lyr_tgt: f32,
    pub(crate) lyr_line: Option<usize>,
    pub(crate) lyr_follow: bool,
    pub(crate) lyr_snap: bool,
    pub(crate) lyr_key: String,
    /// Last local song + position from the previous run (restored once the library is scanned).
    resume: Option<(std::path::PathBuf, f32)>,
    resume_saved: Instant,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let cfg = Config::load();
        cc.egui_ctx.set_zoom_factor(cfg.ui_scale.clamp(0.8, 1.5));
        spotify::set_playback_options(cfg.spotify_quality, cfg.spotify_normalise, cfg.spotify_gapless);
        crate::eq::set_all(&cfg.eq_gains);
        crate::eq::set_enabled(cfg.eq_enabled);
        let (player, error) = match Playback::new(cfg.volume) {
            Ok(p) => (Some(p), None),
            Err(e) => (
                None,
                Some(format!(
                    "No sound device found, so nothing can play. Connect speakers or headphones (and make sure Windows sound is not disabled), then restart litewave. ({e})"
                )),
            ),
        };
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9_7F4A_7C15)
            | 1;
        let (sp_tx, sp_rx) = channel();
        let (yt_tx, yt_rx) = channel();
        let (store_tx, store_rx) = channel();
        let mut app = Self {
            ctx: cc.egui_ctx.clone(),
            player,
            cfg,
            cfg_dirty: false,
            cfg_saving: Arc::new(AtomicBool::new(false)),
            error,
            local: Vec::new(),
            tracks: Vec::new(),
            list_is_local: true,
            filtered: Vec::new(),
            query: String::new(),
            last_query: String::new(),
            search_focus: false,
            current: None,
            now: None,
            history: Vec::new(),
            shuffle_seen: HashSet::new(),
            playing_from: None,
            shuffle_menu: None,
            sp_editable: Vec::new(),
            notice: None,
            dialog: None,
            dialog_text: String::new(),
            yt_tx,
            yt_rx,
            yt_wait: None,
            scan_rx: None,
            scan_count: 0,
            drag_pos: None,
            window_title: String::new(),
            rng: seed,
            ipod: IpodState::default(),
            sp: None,
            sp_session: None,
            remote_id: None,
            sp_tx,
            sp_rx,
            sp_window: false,
            ipod_search: None,
            shuffle_pending: false,
            sleep_at: None,
            sleep_mins: 0,
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
            logo_tex: load_logo(&cc.egui_ctx),
            cjk_loaded: false,
            cjk_needed: false,
            fonts_check: false,
            art: ArtCache::new(),
            view: View::Library,
            sp_nav: Vec::new(),
            liked_ids: HashSet::new(),
            sp_interactive: false,
            sp_link: String::new(),
            yt_query: String::new(),
            yt_list: Vec::new(),
            media: crate::mediakeys::MediaKeys::new(cc),
            tray: crate::tray::Tray::new(cc),
            tray_hidden: false,
            store_items: Vec::new(),
            store_busy: false,
            store_note: String::new(),
            store_tx,
            store_rx,
            queue: Vec::new(),
            recent: VecDeque::new(),
            sp_user: String::new(),
            sp_shelves: None,
            shelves_next: None,
            shelves_busy: false,
            sp_recent: None,
            recent_busy: false,
            sp_recent_albums: Vec::new(),
            sp_top_artists: Vec::new(),
            sp_recent_plays: Vec::new(),
            sp_top_songs: [None, None, None],
            top_songs_busy: [false; 3],
            top_songs_tried: [None; 3],
            queue_recent_tab: false,
            list_fetch: HashMap::new(),
            home_back: None,
            queue_resume: None,
            row_menu: None,
            sp_typed: Instant::now(),
            sp_live: String::new(),
            page_cache: HashMap::new(),
            window_ipod: false,
            prev_size: None,
            lyrics: None,
            lyrics_status: LyricsStatus::Idle,
            lyrics_rx: None,
            lyrics_key: String::new(),
            lyric_shown: None,
            lyrics_full: false,
            lyr_cur: 0.0,
            lyr_tgt: 0.0,
            lyr_line: None,
            lyr_follow: true,
            lyr_snap: true,
            lyr_key: String::new(),
            resume: crate::resume::load(),
            resume_saved: Instant::now(),
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
        if spotify::ENABLED {
            if let Some(rt) = app.cfg.spotify_refresh.clone() {
                app.start_spotify(Some(rt));
            }
        }
        app
    }

    // ---- library -----------------------------------------------------------

    /// Plays files (or folders of music) opened with litewave or dropped onto the window.
    pub fn open_paths(&mut self, paths: Vec<PathBuf>) {
        let mut files: Vec<PathBuf> = Vec::new();
        for p in paths {
            if p.is_dir() {
                for e in walkdir::WalkDir::new(&p).into_iter().flatten() {
                    if e.file_type().is_file() && crate::library::is_audio(e.path()) {
                        files.push(e.path().to_path_buf());
                    }
                }
            } else if crate::library::is_audio(&p) {
                files.push(p);
            }
        }
        if files.is_empty() {
            return;
        }
        files.sort();
        let list: Vec<Track> = files.iter().map(|f| crate::library::read_track(f)).collect();
        self.set_list(list, 0, false);
    }

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
                self.shuffle_seen.clear();
                self.refilter();
                self.current = current_path
                    .and_then(|p| self.tracks.iter().position(|t| t.path == p));
            }
            // Continue where the last run stopped (paused, at the saved position).
            if self.list_is_local && self.current.is_none() && self.now.is_none() {
                if let Some((path, pos)) = self.resume.take() {
                    if let Some(i) = self.tracks.iter().position(|t| t.path == path) {
                        let track = self.tracks[i].clone();
                        if let Some(p) = self.player.as_mut() {
                            if p.load(&track, Duration::from_secs_f32(pos), false).is_ok() {
                                self.now = Some(track);
                                self.current = Some(i);
                            }
                        }
                    }
                }
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
        self.playing_from = None;
        self.history.clear();
        self.shuffle_seen.clear();
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
        self.shuffle_seen.clear();
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
        self.sp_interactive = refresh.is_none();
        self.sp_status = if refresh.is_some() {
            "Reconnecting to Spotify...".to_string()
        } else {
            if self.cfg.spotify_client_id.trim().is_empty() {
                "Connecting... finish the login in your browser.".to_string()
            } else {
                "Connecting... approve both Spotify pages in your browser (one for playback, one for your own Client ID).".to_string()
            }
        };
        let own_id = self.cfg.spotify_client_id.trim().to_string();
        if !own_id.is_empty() && own_id.len() != 32 {
            let e = "That does not look like a Client ID (it is 32 characters). Copy it from your app on developer.spotify.com/dashboard.".to_string();
            self.sp_status = e.clone();
            self.error = Some(e);
            self.sp_interactive = false;
            return;
        }
        let browse_refresh = self.cfg.spotify_browse_refresh.clone();
        let want_connect = true; // litewave always shows up as a device in Spotify
        let start_volume = self.cfg.volume;
        let tx = self.sp_tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let res = (|| -> Result<SpLogin, String> {
                spotify::note("sign-in: waiting for the browser (or refreshing a saved sign-in)");
                // Playback signs in with librespot's own client ID: Spotify accepts it for streaming.
                // With the user's own Client ID there are two sign-ins; they run in one browser tab,
                // one right after the other.
                let (play, browse_now) = match &refresh {
                    Some(r) => (spotify::Client::from_refresh(r, spotify::SHARED_CLIENT_ID)?, None),
                    None if own_id.is_empty() => (spotify::Client::login(spotify::SHARED_CLIENT_ID)?, None),
                    None => spotify::Client::login_pair(spotify::SHARED_CLIENT_ID, Some(own_id.as_str()))?,
                };
                spotify::note("sign-in: done, starting playback");
                // Playback needs Premium, so a failure here is reported but does not undo the sign-in.
                let engine = play
                    .access_token()
                    .and_then(|t| spotify::Engine::connect(&t, ctx.clone(), want_connect, start_volume));
                spotify::note(if engine.is_ok() { "playback: started" } else { "playback: could not start" });
                let play_rt = play.refresh_token();
                if own_id.is_empty() {
                    // No own app: browsing shares the playback sign-in (and Spotify rate-limits it).
                    return Ok((play, engine, play_rt, None));
                }
                // Browsing uses the user's own Spotify app, which has its own request quota.
                let browse = match (browse_now, &browse_refresh) {
                    (Some(b), _) => b,
                    (None, Some(r)) => spotify::Client::from_refresh(r, &own_id)?,
                    (None, None) => {
                        return Err("Sign in again to use your own Client ID for browsing.".to_string())
                    }
                };
                let browse_rt = browse.refresh_token();
                Ok((browse, engine, play_rt, Some(browse_rt)))
            })();
            let _ = tx.send(SpMsg::Connected(res));
            ctx.request_repaint();
        });
    }

    fn poll_sp(&mut self) {
        while let Ok(msg) = self.sp_rx.try_recv() {
            match msg {
                SpMsg::Page(node, res) => {
                    if node == Node::SpLiked {
                        if let Ok((_, tracks)) = &res {
                            self.liked_ids = tracks.iter().filter_map(|t| t.spotify_id.clone()).collect();
                        }
                    }
                    self.ipod_receive(node, res)
                }
                SpMsg::Liked(ids) => self.liked_ids = ids.into_iter().collect(),
                SpMsg::UserName(n) => self.sp_user = n,
                SpMsg::Connected(Ok((client, engine, refresh, browse_refresh))) => {
                    spotify::note("window: sign-in result received");
                    self.sp = Some(client);
                    if let Some(c) = self.sp.clone() {
                        let tx = self.sp_tx.clone();
                        let ctx = self.ctx.clone();
                        std::thread::spawn(move || {
                            if let Ok(n) = c.user_name() {
                                let _ = tx.send(SpMsg::UserName(n));
                                ctx.request_repaint();
                            }
                        });
                    }
                    if let Some(c) = self.sp.clone() {
                        let tx = self.sp_tx.clone();
                        let ctx = self.ctx.clone();
                        std::thread::spawn(move || {
                            // Let the page the user opened first load first; this read then reuses its copy.
                            std::thread::sleep(Duration::from_secs(3));
                            if let Ok(v) = c.liked() {
                                let _ = tx.send(SpMsg::Liked(v.into_iter().filter_map(|t| t.spotify_id).collect()));
                                ctx.request_repaint();
                            }
                        });
                    }
                    match engine {
                        Ok(e) => {
                            let device = e.has_connect();
                            self.sp_session = Some(e.handle());
                            if let Some(p) = self.player.as_mut() {
                                p.set_spotify(e);
                            }
                            self.sp_status = if device {
                                "Connected. Browsing and playback are ready. litewave also shows up as a device in your Spotify apps."
                                    .to_string()
                            } else if self.cfg.spotify_connect {
                                "Connected. Browsing and playback are ready. (The phone device could not start; see the log.)"
                                    .to_string()
                            } else {
                                "Connected. Browsing and playback are ready.".to_string()
                            };
                        }
                        Err(e) => {
                            self.sp_status =
                                format!("Connected for browsing. Playback is unavailable: {e}");
                            self.error = Some(self.sp_status.clone());
                        }
                    }
                    self.cfg.spotify_refresh = Some(refresh);
                    self.cfg.spotify_browse_refresh = browse_refresh;
                    self.cfg.spotify_welcome_done = true;
                    self.cfg_dirty = true;
                    self.shelves_next = None;
                    self.refresh_top();
                    self.load_editable();
                    spotify::note("window: sign-in result applied");
                    if self.sp_interactive {
                        self.sp_interactive = false;
                        self.sp_nav.clear();
                        self.view = View::Home;
                    } else if self.view == View::Library {
                        // Connected (also after an automatic reconnect at startup): Home is the default.
                        self.view = View::Home;
                    }
                }
                SpMsg::Editable(Ok(list)) => self.sp_editable = list,
                SpMsg::Remote(id, Ok(t)) => {
                    // Only if the phone is still on that song.
                    if self.now.is_none() || self.remote_id.as_deref() == Some(id.as_str()) {
                        self.now = Some(t);
                        self.current = None;
                    }
                }
                SpMsg::Remote(_, Err(_)) => {}
                SpMsg::Shelves(res) => {
                    self.shelves_busy = false;
                    match res {
                        Ok(v) => {
                            self.sp_shelves = Some((Instant::now(), v));
                            self.shelves_next = None;
                        }
                        // Quietly: no message, Home just keeps what it has (or the library shelves).
                        Err(wait) => {
                            // A failed Home feed is not worth retrying every 2 minutes: each try also runs
                            // five searches. Spotify's own wait (if longer) still wins.
                            let secs = wait.max(900);
                            self.shelves_next = Some(Instant::now() + Duration::from_secs(secs));
                        }
                    }
                }
                SpMsg::RecentCtx(res) => {
                    self.recent_busy = false;
                    // On failure (e.g. signed in before the permission existed) keep the old order,
                    // but remember the try so it is not repeated every frame.
                    match res {
                        Ok((ids, albums, top, plays)) => {
                            if !plays.is_empty() {
                                self.sp_recent_plays = plays;
                            }
                            self.sp_recent = Some((Instant::now(), ids));
                            if !albums.is_empty() {
                                self.sp_recent_albums = albums;
                            }
                            if !top.is_empty() {
                                self.sp_top_artists = top;
                            }
                            save_recent(
                                self.sp_recent.as_ref().map(|r| r.1.as_slice()).unwrap_or(&[]),
                                &self.sp_recent_albums,
                                &self.sp_top_artists,
                                &self.sp_recent_plays,
                            );
                        }
                        // Spotify said no (rate limit ...): keep the order and tiles we already have.
                        Err(_) => {
                            let old = self.sp_recent.take().map(|r| r.1).unwrap_or_default();
                            self.sp_recent = Some((Instant::now(), old));
                        }
                    }
                }
                SpMsg::TopSongs(k, res) => {
                    if k < 3 {
                        self.top_songs_busy[k] = false;
                        // On failure keep what is shown; `top_songs_tried` stops a retry every frame.
                        if let Ok(v) = res {
                            self.sp_top_songs[k] = Some((Instant::now(), v));
                        }
                    }
                }
                SpMsg::Editable(Err(_)) => {}
                SpMsg::Done(res, reload) => {
                    match res {
                        Ok(msg) => self.notice = Some((msg, Instant::now())),
                        Err(e) => self.error = Some(e),
                    }
                    if let Some(node) = reload {
                        self.reload_page(&node);
                    }
                }
                SpMsg::Connected(Err(e)) => {
                    self.sp_status = e.clone();
                    self.error = Some(e);
                    // A saved sign-in that no longer works is dropped so the next Connect starts fresh.
                    self.cfg.spotify_refresh = None;
                    self.cfg.spotify_browse_refresh = None;
                    self.cfg_dirty = true;
                }
            }
        }
    }

    /// Fetches the playlists the user may edit, in the background.
    fn load_editable(&self) {
        let Some(client) = self.sp.clone() else { return };
        let tx = self.sp_tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(SpMsg::Editable(client.editable_playlists()));
            ctx.request_repaint();
        });
    }

    /// Runs one Spotify edit on a worker thread and reports back with a message.
    fn sp_task<F>(&self, reload: Option<Node>, work: F)
    where
        F: FnOnce(&spotify::Client) -> Result<String, String> + Send + 'static,
    {
        let Some(client) = self.sp.clone() else {
            return;
        };
        let tx = self.sp_tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(SpMsg::Done(work(&client), reload));
            ctx.request_repaint();
        });
    }

    /// Drops the cached copy of a page and loads it again if it is open.
    fn reload_page(&mut self, node: &Node) {
        if matches!(node, Node::SpLiked) {
            spotify::forget_liked();
        }
        self.page_cache.remove(&format!("{node:?}"));
        if let Some(i) = self.sp_nav.iter().rposition(|p| p.node == *node) {
            let page = self.sp_nav.remove(i);
            // Keep the pages above it in place; only the reloaded one is rebuilt.
            let (mut fresh, fetch) = self.page_for(&page.node);
            fresh.sel = page.sel;
            self.sp_nav.insert(i, fresh);
            if fetch {
                self.spawn_fetch(page.node);
            }
        }
        if matches!(node, Node::SpPlaylists) {
            self.load_editable();
        }
    }

    /// Asks for a library list (playlists, albums, artists) unless a fresh copy is cached or a request
    /// was sent a moment ago (so a failed load is not retried every frame).
    pub(crate) fn want_list(&mut self, node: Node) {
        let key = format!("{node:?}");
        let fresh = self
            .page_cache
            .get(&key)
            .map_or(false, |(at, _, _)| at.elapsed() < Duration::from_secs(600));
        let recent = self
            .list_fetch
            .get(&key)
            .map_or(false, |t| t.elapsed() < Duration::from_secs(30));
        if fresh || recent {
            return;
        }
        self.list_fetch.insert(key, Instant::now());
        self.spawn_fetch(node);
    }

    /// Reads Spotify's recently played list (at most every 5 minutes). Safe to call every frame.
    pub(crate) fn want_recent(&mut self) {
        let Some(client) = self.sp.clone() else { return };
        // After a restart (or while Spotify is rate-limiting) show the last copy that was saved.
        {
            use std::sync::atomic::{AtomicBool, Ordering};
            static LOADED: AtomicBool = AtomicBool::new(false);
            if !LOADED.swap(true, Ordering::Relaxed) && self.sp_recent.is_none() && self.sp_recent_plays.is_empty() {
                if let Some((ids, albums, top, plays)) = load_recent() {
                    // Dated in the past, so a fresh copy is still asked for right away.
                    let old = Instant::now().checked_sub(Duration::from_secs(400)).unwrap_or_else(Instant::now);
                    self.sp_recent = Some((old, ids));
                    self.sp_recent_albums = albums;
                    self.sp_top_artists = top;
                    self.sp_recent_plays = plays;
                }
            }
        }
        let fresh = self
            .sp_recent
            .as_ref()
            .map_or(false, |(at, _)| at.elapsed() < Duration::from_secs(300));
        if fresh || self.recent_busy {
            return;
        }
        self.recent_busy = true;
        let tx = self.sp_tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let top = client.top_artists().unwrap_or_default();
            let res = client.recent_contexts().map(|(ids, albums, plays)| (ids, albums, top, plays));
            let _ = tx.send(SpMsg::RecentCtx(res));
            ctx.request_repaint();
        });
    }

    /// Reads your top songs for period `k` (0 = last 4 weeks, 1 = 6 months, 2 = past year) unless a copy
    /// under 15 minutes old exists, a request is running, or the last try was under a minute ago.
    pub(crate) fn want_top_songs(&mut self, k: usize) {
        let Some(client) = self.sp.clone() else { return };
        if k >= 3 || self.top_songs_busy[k] {
            return;
        }
        let fresh = self.sp_top_songs[k]
            .as_ref()
            .map_or(false, |(at, _)| at.elapsed() < Duration::from_secs(900));
        let tried = self.top_songs_tried[k].map_or(false, |t| t.elapsed() < Duration::from_secs(60));
        if fresh || tried {
            return;
        }
        self.top_songs_busy[k] = true;
        self.top_songs_tried[k] = Some(Instant::now());
        let range = ["short_term", "medium_term", "long_term"][k];
        let tx = self.sp_tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(SpMsg::TopSongs(k, client.top_tracks(range)));
            ctx.request_repaint();
        });
    }

    /// Place of a playlist / album / artist in Spotify's recently played list (0 = newest);
    /// `usize::MAX` when it is not there.
    pub(crate) fn recent_rank(&self, node: &Node) -> usize {
        let id = match node {
            Node::SpPlaylist(id, _) | Node::SpAlbum(id, _) | Node::SpArtist(id, _) => id,
            _ => return usize::MAX,
        };
        self.sp_recent
            .as_ref()
            .and_then(|(_, v)| v.iter().position(|x| x == id))
            .unwrap_or(usize::MAX)
    }

    /// Asks for Spotify's own Home shelves on a worker thread, unless they are off in Settings, there is
    /// no playback connection, a copy under 10 minutes old exists, a request is running, or the last one
    /// failed less than 2 minutes ago (longer if Spotify asked us to wait). Safe to call every frame.
    pub(crate) fn want_shelves(&mut self) {
        if !self.cfg.home_shelves || self.shelves_busy {
            return;
        }
        let (Some(session), Some(client)) = (self.sp_session.clone(), self.sp.clone()) else {
            // Signed in for browsing but no playback connection: say so once in the log.
            if self.sp.is_some() {
                use std::sync::atomic::{AtomicBool, Ordering};
                static NOTED: AtomicBool = AtomicBool::new(false);
                if !NOTED.swap(true, Ordering::Relaxed) {
                    spotify::note("home shelves: waiting for the playback connection (it needs Premium and a finished sign-in)");
                }
            }
            return;
        };
        let fresh = self
            .sp_shelves
            .as_ref()
            .map_or(false, |(at, _)| at.elapsed() < Duration::from_secs(600));
        let waiting = self.shelves_next.map_or(false, |t| Instant::now() < t);
        if fresh || waiting {
            return;
        }
        self.shelves_busy = true;
        self.shelves_next = Some(Instant::now() + Duration::from_secs(120));
        let tx = self.sp_tx.clone();
        let ctx = self.ctx.clone();
        let own_app = !self.cfg.spotify_client_id.trim().is_empty();
        let play_refresh = self.cfg.spotify_refresh.clone();
        std::thread::spawn(move || {
            let res = match client.access_token() {
                Ok(token) => {
                    let first = session.home_shelves(&token);
                    // A personal app in Development Mode is often refused for Spotify's Home (403). Spotifast
                    // takes Home from the playback sign-in's app instead, so try that token once.
                    if let (true, true, Some(r)) = (own_app, matches!(first, Err(0)), play_refresh.as_ref()) {
                        match spotify::Client::from_refresh(r, spotify::SHARED_CLIENT_ID).and_then(|c| c.access_token()) {
                            Ok(t2) => {
                                spotify::note("home shelves: trying again with the playback sign-in's token");
                                session.home_shelves(&t2)
                            }
                            Err(_) => first,
                        }
                    } else {
                        first
                    }
                }
                Err(_) => {
                    spotify::note("home shelves: no Web API token yet");
                    Err(0)
                }
            };
            let (mut shelves, wait) = match res {
                Ok(v) => (v, 0),
                Err(w) => (Vec::new(), w),
            };
            // Like Spotifast: when Spotify's private Home gives no Made for you, look for Spotify's own
            // playlists (Daily Mix, Discover Weekly ...) by search instead.
            if !shelves.iter().any(|s| s.title == "Made for you") && spotify::rate_limit_wait().is_none() {
                // Keep why the private Home feed failed: the search below may add its own line, and the
                // Home page shows the newest one.
                let home_note = spotify::last_shelves_note().replace("home shelves: ", "");
                match client.made_for_you_search() {
                    Ok(cards) => {
                        spotify::note(&format!("home shelves: Made for you found by search ({} playlists)", cards.len()));
                        shelves.insert(
                            0,
                            spotify::Shelf { title: "Made for you".to_string(), subtitle: String::new(), cards },
                        );
                    }
                    Err(e) => {
                        let first = if home_note.is_empty() { "no answer".to_string() } else { home_note };
                        spotify::note(&format!("home shelves: Home feed: {first}; search: {e}"));
                    }
                }
            }
            let res = if shelves.is_empty() { Err(wait) } else { Ok(shelves) };
            let _ = tx.send(SpMsg::Shelves(res));
            ctx.request_repaint();
        });
    }

    /// True when Back has somewhere to go: an earlier Spotify page, or Home (for a page opened from it).
    pub(crate) fn sp_can_back(&self) -> bool {
        self.view == View::Spotify
            && (self.sp_nav.len() > 1
                || (self.sp_nav.len() == 1 && self.home_back.as_ref() == self.sp_nav.first().map(|p| &p.node)))
    }

    fn playlist_name(&self, pid: &str) -> String {
        self.sp_editable
            .iter()
            .find(|e| e.id == pid)
            .map(|e| e.name.clone())
            .unwrap_or_else(|| "the playlist".to_string())
    }

    /// The open page of playlist `pid`, if there is one.
    fn playlist_node(&self, pid: &str) -> Option<Node> {
        self.sp_nav
            .iter()
            .find(|p| matches!(&p.node, Node::SpPlaylist(id, _) if id == pid))
            .map(|p| p.node.clone())
    }

    fn forget_playlist(&mut self, pid: &str) {
        let key = format!("SpPlaylist(\"{pid}\"");
        self.page_cache.retain(|k, _| !k.contains(&key));
    }

    /// Library lists are cached for a while; saving or removing something makes them stale.
    fn forget_library_pages(&mut self) {
        self.list_fetch.clear();
        for n in [Node::SpLiked, Node::SpPlaylists, Node::SpAlbums, Node::SpArtists, Node::SpShows] {
            self.page_cache.remove(&format!("{n:?}"));
        }
    }

    /// OK pressed in the name / confirm popup.
    fn dialog_ok(&mut self) {
        let Some(d) = self.dialog.take() else { return };
        let name = self.dialog_text.trim().to_string();
        match d {
            Dialog::NewPlaylist => {
                if name.is_empty() {
                    self.dialog = Some(Dialog::NewPlaylist);
                    return;
                }
                self.forget_library_pages();
                self.sp_task(Some(Node::SpPlaylists), move |c| {
                    let p = c.create_playlist(&name)?;
                    Ok(format!("Created playlist {}", p.name))
                });
            }
            Dialog::Rename(id, old) => {
                if name.is_empty() || name == old {
                    return;
                }
                self.forget_library_pages();
                self.forget_playlist(&id);
                for p in self.sp_nav.iter_mut() {
                    if matches!(&p.node, Node::SpPlaylist(pid, _) if *pid == id) {
                        p.title = name.clone();
                    }
                }
                self.sp_task(Some(Node::SpPlaylists), move |c| {
                    c.rename_playlist(&id, &name)?;
                    Ok(format!("Renamed to {name}"))
                });
            }
            Dialog::Delete(id, name) => {
                self.forget_library_pages();
                self.forget_playlist(&id);
                // Leave the page of the playlist that is being deleted.
                if matches!(self.sp_nav.last().map(|p| &p.node), Some(Node::SpPlaylist(pid, _)) if *pid == id)
                    && self.sp_nav.len() > 1
                {
                    self.sp_nav.pop();
                }
                self.sp_task(Some(Node::SpPlaylists), move |c| {
                    c.library_remove(&[format!("spotify:playlist:{id}")])?;
                    Ok(format!("Deleted {name}"))
                });
            }
        }
    }

    fn disconnect_spotify(&mut self) {
        spotify::clear_liked();
        self.sp_recent_plays.clear();
        self.sp_recent_albums.clear();
        self.sp_top_artists.clear();
        if let Some(p) = recent_cache_path() {
            let _ = std::fs::remove_file(p);
        }
        self.sp = None;
        self.sp_session = None;
        self.sp_shelves = None;
        self.sp_recent = None;
        self.sp_top_songs = [None, None, None];
        self.top_songs_tried = [None; 3];
        self.shelves_next = None;
        self.sp_editable.clear();
        if let Some(p) = self.player.as_mut() {
            p.drop_spotify();
        }
        self.cfg.spotify_refresh = None;
        self.cfg.spotify_browse_refresh = None;
        self.cfg_dirty = true;
        self.sp_status = "Disconnected.".to_string();
        self.ipod.stack.truncate(1);
        self.sp_nav.clear();
        self.page_cache.clear();
        self.list_fetch.clear();
        self.home_back = None;
        self.view = View::Library;
    }

    /// The Spotify account page (classic layout): status, one sign-in button, details under "Advanced".
    pub(crate) fn spotify_page(&mut self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("Spotify").size(24.0).strong());
        ui.add_space(10.0);
        if !spotify::ENABLED {
            ui.label("This build has no Spotify support.");
            ui.label("Rebuild with: cargo run --release --features spotify");
            return;
        }
        let accent = self.cfg.theme.accent();
        let connected = self.sp.is_some();
        let weak = ui.visuals().weak_text_color();
        let mut connect = false;
        let mut disconnect = false;
        let mut id_changed = false;
        let mut explicit_changed = false;
        let mut open_log = false;

        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width().min(520.0));
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                let dot = if connected { accent } else { egui::Color32::GRAY };
                ui.painter().circle_filled(r.center(), 5.0, dot);
                ui.label(
                    egui::RichText::new(if connected { "Connected" } else { "Not connected" })
                        .size(17.0)
                        .strong(),
                );
            });
            if connected {
                ui.label(egui::RichText::new("Device: litewave").color(weak));
            }
            if !self.sp_status.is_empty() {
                ui.label(egui::RichText::new(self.sp_status.clone()).size(12.0).color(weak));
            }
            ui.add_space(6.0);
            if connected {
                if ui.button("Sign out").clicked() {
                    disconnect = true;
                }
            } else if ui
                .add(
                    egui::Button::new(egui::RichText::new("Sign in with Spotify").color(egui::Color32::BLACK))
                        .fill(accent),
                )
                .clicked()
            {
                connect = true;
            }
        });

        if !connected {
            ui.add_space(14.0);
            ui.label(egui::RichText::new("Set up in 3 steps").strong());
            ui.add_space(4.0);
            let id_ok = self.cfg.spotify_client_id.trim().len() >= 32;
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(if id_ok { accent } else { weak }, "1.");
                ui.label("Create an app on");
                ui.hyperlink_to("developer.spotify.com/dashboard", "https://developer.spotify.com/dashboard");
                ui.label("and add this Redirect URI:");
            });
            ui.monospace(spotify::REDIRECT_URI);
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.colored_label(if id_ok { accent } else { weak }, "2.");
                ui.label("Paste its Client ID:");
            });
            if ui
                .add(
                    egui::TextEdit::singleline(&mut self.cfg.spotify_client_id)
                        .hint_text("32-character Client ID")
                        .desired_width(320.0),
                )
                .changed()
            {
                id_changed = true;
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.colored_label(weak, "3.");
                ui.label("Press Sign in with Spotify above and approve in the browser.");
            });
        } else {
            ui.add_space(12.0);
            if ui
                .checkbox(&mut self.cfg.allow_explicit, "Allow explicit content")
                .changed()
            {
                explicit_changed = true;
            }
        }

        ui.add_space(12.0);
        egui::CollapsingHeader::new("Help").show(ui, |ui| {
            ui.label("Spotify needs a Premium account, to create the app and to play.");
            ui.label("litewave shows up as a device named \"litewave\" in your Spotify apps.");
            ui.hyperlink_to("Step-by-step guide with pictures", GUIDE_URL);
            ui.hyperlink_to("Open the Spotify developer dashboard", "https://developer.spotify.com/dashboard");
            ui.hyperlink_to("Report a bug", BUG_URL);
        });
        egui::CollapsingHeader::new("Advanced").show(ui, |ui| {
            if connected {
                ui.label("Client ID");
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut self.cfg.spotify_client_id)
                            .hint_text("32-character Client ID")
                            .desired_width(320.0),
                    )
                    .changed()
                {
                    id_changed = true;
                }
                ui.label("Redirect URI");
                ui.monospace(spotify::REDIRECT_URI);
            }
            ui.weak("Unofficial: litewave signs in the way other open-source Spotify players do, so Spotify can change or block it.");
            let last = spotify::last_log();
            if !last.is_empty() {
                ui.weak(format!("Last log: {last}"));
            }
            if let Some(p) = self.player.as_ref() {
                if !p.sp_info().is_empty() {
                    ui.weak(format!("Last event: {}", p.sp_info()));
                }
            }
            if ui.button("Open log folder").clicked() {
                open_log = true;
            }
        });

        if explicit_changed {
            self.cfg_dirty = true;
        }
        if id_changed {
            // The saved browsing sign-in belongs to the old Client ID.
            self.cfg.spotify_browse_refresh = None;
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
    }

    /// Shown once, on the first launch with Spotify support and no sign-in: the three setup steps in one
    /// place, like Spotifast's sign-in screen. Skip keeps litewave a plain local player; the Spotify page
    /// in the sidebar stays available for later.
    fn first_run_window(&mut self, ctx: &egui::Context) {
        if !spotify::ENABLED
            || self.cfg.spotify_welcome_done
            || self.cfg.spotify_refresh.is_some()
            || self.sp.is_some()
            || self.sp_window
        {
            return;
        }
        // Keep typing in the Client ID box from triggering play / pause shortcuts.
        self.sp_focus = true;
        let accent = self.cfg.theme.accent();
        let weak = ctx.style().visuals.weak_text_color();
        let id_len = self.cfg.spotify_client_id.trim().len();
        let id_ok = id_len == 32;
        let busy = self.sp_interactive;
        let mut connect = false;
        let mut skip = false;
        let mut id_changed = false;
        egui::Window::new("Connect Spotify")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_width(460.0);
                ui.label(egui::RichText::new("Welcome to litewave").size(22.0).strong());
                ui.label(
                    egui::RichText::new("Spotify is optional. Without it litewave plays your own music files.")
                        .color(weak),
                );
                ui.add_space(10.0);

                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(if id_ok { accent } else { weak }, "1.");
                    ui.label("Create a free Spotify app (the account needs Premium):");
                });
                ui.hyperlink_to("Open developer.spotify.com/dashboard", "https://developer.spotify.com/dashboard");
                ui.label("Add this exact Redirect URI to the app and tick Web API:");
                ui.monospace(spotify::REDIRECT_URI);
                ui.hyperlink_to("Step-by-step guide with pictures", GUIDE_URL);
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    ui.colored_label(if id_ok { accent } else { weak }, "2.");
                    ui.label("Paste the app's Client ID (Ctrl+V):");
                });
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut self.cfg.spotify_client_id)
                            .hint_text("32-character Client ID")
                            .desired_width(420.0),
                    )
                    .changed()
                {
                    id_changed = true;
                }
                if id_len == 0 {
                    ui.weak("It is on the app's page in the dashboard.");
                } else if id_ok {
                    ui.colored_label(accent, "Looks right.");
                } else {
                    ui.weak(format!("{id_len} of 32 characters."));
                }
                ui.add_space(8.0);

                ui.horizontal(|ui| {
                    ui.colored_label(weak, "3.");
                    ui.label("Sign in, then approve in the browser (twice: browsing, then playback).");
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let label = if busy { "Waiting for the browser..." } else { "Sign in with Spotify" };
                    if ui
                        .add_enabled(
                            id_ok && !busy,
                            egui::Button::new(egui::RichText::new(label).color(egui::Color32::BLACK)).fill(accent),
                        )
                        .clicked()
                    {
                        connect = true;
                    }
                    if ui.button("Skip for now").clicked() {
                        skip = true;
                    }
                });
                if !self.sp_status.is_empty() {
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new(self.sp_status.clone()).size(12.0).color(weak));
                }
            });
        if id_changed {
            // The saved browsing sign-in belongs to the old Client ID.
            self.cfg.spotify_browse_refresh = None;
            self.cfg_dirty = true;
        }
        if connect {
            self.start_spotify(None);
        }
        if skip {
            self.cfg.spotify_welcome_done = true;
            self.cfg_dirty = true;
        }
    }

    /// Settings page (classic layout): updates, appearance, Spotify, playback notes, about.
    fn spotify_window(&mut self, ctx: &egui::Context) {
        self.sp_focus = false;
        if !self.sp_window {
            return;
        }
        let mut open = true;
        let mut connect = false;
        let mut open_log = false;
        let mut disconnect = false;
        let mut id_changed = false;
        let mut explicit_changed = false;
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
                ui.label("Sign in with your Spotify account. A browser tab opens to approve litewave.");
                ui.add_space(6.0);
                ui.label("Your Spotify app Client ID (stops \"too many requests\"):");
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut self.cfg.spotify_client_id)
                            .hint_text("32-character Client ID")
                            .desired_width(320.0),
                    )
                    .changed()
                {
                    id_changed = true;
                }
                ui.weak("1. Create an app on the Spotify developer dashboard (needs Premium).");
                ui.hyperlink_to("Open developer.spotify.com/dashboard", "https://developer.spotify.com/dashboard");
                ui.weak("2. Add this exact Redirect URI to the app, then paste its Client ID above:");
                ui.monospace(spotify::REDIRECT_URI);
                ui.weak("3. Click Connect Spotify. With your own Client ID you approve twice (browsing, then playback).");
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let label = if self.sp.is_some() { "Sign in again" } else { "Connect Spotify" };
                    if ui.button(label).clicked() {
                        connect = true;
                    }
                    if self.sp.is_some() && ui.button("Disconnect").clicked() {
                        disconnect = true;
                    }
                });
                if !self.sp_status.is_empty() {
                    ui.add_space(4.0);
                    ui.label(&self.sp_status);
                }
                ui.add_space(6.0);
                if ui
                    .checkbox(&mut self.cfg.allow_explicit, "Allow explicit content")
                    .changed()
                {
                    explicit_changed = true;
                }
                ui.weak("When off, explicit songs are greyed out and skipped (like Spotify's setting).");
                ui.add_space(6.0);
                ui.weak("litewave always shows up as a device named \"litewave\" in your Spotify apps. Your phone can pause, seek and change the volume, and you can send music to this PC from its device list.");
                ui.weak("Spotify needs a Premium account, to create the app and to play.");
                ui.weak("Unofficial: litewave signs in the way other open-source Spotify players do, so Spotify can change or block it.");
                let last = spotify::last_log();
                if !last.is_empty() {
                    ui.add_space(4.0);
                    ui.weak(format!("Last log: {last}"));
                }
                if let Some(p) = self.player.as_ref() {
                    if !p.sp_info().is_empty() {
                        ui.weak(format!("Last event: {}", p.sp_info()));
                    }
                }
                if ui.button("Open log folder").clicked() {
                    open_log = true;
                }
            });
        if explicit_changed {
            self.cfg_dirty = true;
        }
        if id_changed {
            // The saved browsing sign-in belongs to the old Client ID.
            self.cfg.spotify_browse_refresh = None;
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

    // ---- lyrics ------------------------------------------------------------

    /// Collects a finished lyrics lookup and starts a new one when the song changes.
    fn sync_lyrics(&mut self) {
        let got = self.lyrics_rx.as_ref().and_then(|rx| rx.try_recv().ok());
        if let Some(res) = got {
            self.lyrics_rx = None;
            // A result for a song we already skipped is dropped.
            if res.key == self.lyrics_key {
                match res.result {
                    Ok(Some(l)) => {
                        self.lyrics = Some(l);
                        self.lyrics_status = LyricsStatus::Ready;
                        self.lyric_shown = None;
                    }
                    Ok(None) => self.lyrics_status = LyricsStatus::Missing,
                    Err(_) => self.lyrics_status = LyricsStatus::Failed,
                }
            }
        }

        let key = match (&self.now, true) {
            (Some(t), true) => format!(
                "{}|{}|{}",
                t.path.display(),
                t.spotify_id.as_deref().unwrap_or(""),
                t.title
            ),
            _ => String::new(),
        };
        if key != self.lyrics_key {
            self.lyrics_key = key.clone();
            self.lyrics = None;
            self.lyrics_rx = None;
            self.lyric_shown = None;
            self.lyrics_status = LyricsStatus::Idle;
            if !key.is_empty() {
                if let Some(t) = self.now.clone() {
                    self.lyrics_rx = Some(lyrics::spawn_load(t, key, self.ctx.clone()));
                    self.lyrics_status = LyricsStatus::Loading;
                }
            }
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
        // Key = path + blur, so changing the blur reloads the same image.
        let blur = self.cfg.theme.blur.min(12);
        let want = self.cfg.theme.background.as_ref().map(|p| format!("{p}\u{1}{blur}"));
        if want == self.bg_path {
            return;
        }
        self.bg_path = want.clone();
        match want {
            None => {
                self.bg_tex = None;
                self.bg_rx = None;
            }
            Some(key) => {
                let path = key.split('\u{1}').next().unwrap_or_default().to_string();
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
                        // A blurred picture has no detail to lose, so blur a smaller copy (fast, low RAM).
                        let img = if blur > 0 {
                            let small = if img.width() > 960 || img.height() > 540 {
                                img.resize(960, 540, image::imageops::FilterType::Triangle)
                            } else {
                                img
                            };
                            small.blur(blur as f32)
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
            let tint = self.cfg.theme.tint();
            painter.rect_filled(
                rect,
                0.0,
                Color32::from_rgba_unmultiplied(tint.r(), tint.g(), tint.b(), self.cfg.theme.dim),
            );
        }
        // Soft accent glow rising from the bottom edge (works with or without a wallpaper).
        let glow = self.cfg.theme.glow;
        if glow > 0 {
            let a = self.cfg.theme.accent2();
            let clear = Color32::from_rgba_unmultiplied(a.r(), a.g(), a.b(), 0);
            let strong = Color32::from_rgba_unmultiplied(a.r(), a.g(), a.b(), glow);
            let top = rect.max.y - rect.height() * 0.65;
            let mut mesh = egui::Mesh::default();
            mesh.colored_vertex(pos2(rect.min.x, top), clear);
            mesh.colored_vertex(pos2(rect.max.x, top), clear);
            mesh.colored_vertex(pos2(rect.max.x, rect.max.y), strong);
            mesh.colored_vertex(pos2(rect.min.x, rect.max.y), strong);
            mesh.add_triangle(0, 1, 2);
            mesh.add_triangle(0, 2, 3);
            painter.add(egui::Shape::mesh(mesh));
        }
    }

    /// Cover texture for the current track, if it has one and it has loaded.
    pub(crate) fn current_art(&mut self, ctx: &egui::Context, size: u32) -> Option<egui::TextureId> {
        let key = self.now.as_ref().and_then(|t| t.art_key())?;
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
        if track.explicit && !self.cfg.allow_explicit {
            self.error = Some(format!(
                "\"{}\" is explicit. Turn on \"Explicit content\" in the sidebar to play it.",
                track.title
            ));
            return;
        }
        // YouTube songs are downloaded first; playback starts from `poll_yt` when the file is ready.
        if let Some(id) = track.yt_id.clone() {
            if track.path.as_os_str().is_empty() || !track.path.exists() {
                self.start_yt_download(i, id);
                return;
            }
        }
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
                let played = track.clone();
                self.now = Some(played.clone());
                self.note_recent(played);
                self.current = Some(i);
                self.shuffle_seen.insert(i);
                self.error = None;
            }
            Err(e) => self.error = Some(format!("{}: {e}", track.title)),
        }
    }

    /// "Off", "15 min", ... or the time left.
    pub(crate) fn sleep_label(&self) -> String {
        match self.sleep_at {
            Some(t) if self.sleep_mins > 0 => {
                let left = t.saturating_duration_since(Instant::now()).as_secs() / 60 + 1;
                format!("{left} min left")
            }
            _ => "Off".to_string(),
        }
    }

    pub(crate) fn set_sleep(&mut self, mins: u32) {
        self.sleep_mins = mins;
        self.sleep_at = if mins == 0 {
            None
        } else {
            Some(Instant::now() + Duration::from_secs(mins as u64 * 60))
        };
    }

    /// Pauses playback when the sleep timer runs out.
    fn check_sleep(&mut self, ctx: &egui::Context) {
        let Some(at) = self.sleep_at else { return };
        if Instant::now() >= at {
            self.sleep_at = None;
            self.sleep_mins = 0;
            if let Some(p) = self.player.as_mut() {
                if p.is_playing() {
                    p.toggle();
                }
            }
            self.notice = Some(("Sleep timer: paused.".to_string(), Instant::now()));
            self.refresh_top();
        } else {
            ctx.request_repaint_after(Duration::from_secs(20));
        }
    }

    /// iPod mode: a small box to type a Spotify search into.
    fn ipod_search_ui(&mut self, ctx: &egui::Context) {
        let Some((mut text, mut focused)) = self.ipod_search.clone() else { return };
        let mut go = false;
        let mut cancel = false;
        let mut has_focus = false;
        egui::Window::new("Search Spotify")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                let r = ui.add(
                    egui::TextEdit::singleline(&mut text)
                        .hint_text("Song, artist, album...")
                        .desired_width(220.0),
                );
                if !focused {
                    r.request_focus();
                    focused = true;
                }
                has_focus = r.has_focus();
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    go = true;
                }
                ui.horizontal(|ui| {
                    if ui.button("Search").clicked() {
                        go = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if has_focus {
            self.sp_focus = true;
        }
        if go && !text.trim().is_empty() {
            let q = text.trim().to_string();
            self.ipod_search = None;
            self.remember_search(&q);
            self.ipod_push(Node::SpSearch(q));
        } else if cancel || (go && text.trim().is_empty()) {
            self.ipod_search = None;
        } else {
            self.ipod_search = Some((text, focused));
        }
    }

    /// Puts a search at the top of the recent list.
    fn remember_search(&mut self, q: &str) {
        let q = q.trim();
        if q.is_empty() {
            return;
        }
        self.cfg.recent_searches.retain(|r| !r.eq_ignore_ascii_case(q));
        self.cfg.recent_searches.insert(0, q.to_string());
        self.cfg.recent_searches.truncate(10);
        self.cfg_dirty = true;
    }

    /// The phone started a song on litewave. Show it, and follow it in the list if it is there.
    fn on_remote(&mut self, id: String) {
        self.remote_id = Some(id.clone());
        self.error = None;
        if let Some(i) = self
            .tracks
            .iter()
            .position(|t| t.spotify_id.as_deref() == Some(id.as_str()))
        {
            self.now = Some(self.tracks[i].clone());
            self.current = Some(i);
            self.shuffle_seen.insert(i);
            return;
        }
        let Some(h) = self.sp_session.clone() else { return };
        let tx = self.sp_tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let r = h.track(&id);
            let _ = tx.send(SpMsg::Remote(id, r));
            ctx.request_repaint();
        });
    }

    /// Click on an artist or album in a song row: open its Spotify page, or filter the local library.
    fn open_meta(&mut self, album: bool, id: Option<String>, name: String, spotify: bool) {
        if !spotify {
            self.view = View::Library;
            if !self.list_is_local {
                self.use_local();
            }
            self.query = name;
            return;
        }
        if self.sp.is_none() {
            self.error = Some("Connect Spotify first.".to_string());
            return;
        }
        let node = match (id, album) {
            (Some(id), true) => Node::SpAlbum(id, name),
            (Some(id), false) => Node::SpArtist(id, name),
            (None, _) => Node::SpSearch(name),
        };
        self.view = View::Spotify;
        self.sp_nav.clear();
        self.sp_push(node);
    }

    // ---- YouTube (yt-dlp) ----------------------------------------------------

    fn ytdlp_bin(&self) -> Option<PathBuf> {
        ytdlp::find(&self.cfg.ytdlp_path)
    }

    /// Searches YouTube on a thread; the results replace the song list.
    fn yt_search(&mut self, query: String) {
        let Some(bin) = self.ytdlp_bin() else {
            self.error = Some(
                "yt-dlp was not found. Install it (https://github.com/yt-dlp/yt-dlp) and make sure it is on your PATH, or set its path in config.json (ytdlp_path)."
                    .to_string(),
            );
            return;
        };
        self.notice = Some((format!("Searching YouTube for \"{query}\"..."), Instant::now()));
        let tx = self.yt_tx.clone();
        let ctx = self.ctx.clone();
        let browser = self.cfg.ytdlp_browser.clone();
        std::thread::spawn(move || {
            let _ = tx.send(YtMsg::Results(ytdlp::search(&bin, &query, 15, &browser)));
            ctx.request_repaint();
        });
    }

    /// Downloads song `i` on a thread. When it finishes, `poll_yt` starts it (if still wanted).
    fn start_yt_download(&mut self, i: usize, id: String) {
        if self.yt_wait.as_deref() == Some(id.as_str()) {
            return;
        }
        let Some(bin) = self.ytdlp_bin() else {
            self.error = Some("yt-dlp was not found, so this song cannot be played.".to_string());
            return;
        };
        if let Some(t) = self.tracks.get(i) {
            self.now = Some(t.clone());
        }
        self.yt_wait = Some(id.clone());
        self.error = None;
        self.notice = Some(("Downloading from YouTube...".to_string(), Instant::now()));
        let tx = self.yt_tx.clone();
        let ctx = self.ctx.clone();
        let browser = self.cfg.ytdlp_browser.clone();
        std::thread::spawn(move || {
            let r = ytdlp::fetch_audio(&bin, &id, &browser);
            let _ = tx.send(YtMsg::Audio(id, r));
            ctx.request_repaint();
        });
    }

    /// Loads the theme list on a thread.
    pub(crate) fn store_load(&mut self) {
        if self.store_busy {
            return;
        }
        self.store_busy = true;
        self.store_note = "Loading...".to_string();
        let tx = self.store_tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(StoreMsg::Index(store::fetch_index()));
            ctx.request_repaint();
        });
    }

    /// Downloads one theme on a thread.
    pub(crate) fn store_install(&mut self, item: StoreItem) {
        if self.store_busy {
            return;
        }
        self.store_busy = true;
        self.store_note = format!("Downloading {}...", item.name);
        let tx = self.store_tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(StoreMsg::Installed(store::install(&item)));
            ctx.request_repaint();
        });
    }

    fn poll_store(&mut self) {
        while let Ok(msg) = self.store_rx.try_recv() {
            self.store_busy = false;
            match msg {
                StoreMsg::Index(Ok(list)) => {
                    self.store_note = if list.is_empty() { "No themes in the gallery yet.".to_string() } else { String::new() };
                    self.store_items = list;
                }
                StoreMsg::Index(Err(e)) => self.store_note = e,
                StoreMsg::Installed(Ok(name)) => {
                    self.theme_list = theme::all_themes();
                    self.store_note = format!("Installed \"{name}\". Pick it under Presets above.");
                }
                StoreMsg::Installed(Err(e)) => self.store_note = e,
            }
        }
    }

    fn poll_yt(&mut self) {
        while let Ok(msg) = self.yt_rx.try_recv() {
            match msg {
                YtMsg::Results(Ok(list)) => {
                    if list.is_empty() {
                        self.error = Some("No YouTube results.".to_string());
                    } else {
                        self.yt_list = list.clone();
                        self.tracks = list;
                        self.list_is_local = false;
                        self.history.clear();
                        self.shuffle_seen.clear();
                        self.current = None;
                        self.query.clear();
                        self.refilter();
                        self.view = View::YouTube;
                        self.notice = None;
                        self.error = None;
                    }
                }
                YtMsg::Results(Err(e)) => self.error = Some(e),
                YtMsg::Audio(id, result) => {
                    if self.yt_wait.as_deref() != Some(id.as_str()) {
                        continue; // the user already picked something else
                    }
                    self.yt_wait = None;
                    match result {
                        Ok(path) => {
                            // Remember the file on every row for this video, then play it.
                            let mut at = None;
                            for (k, t) in self.tracks.iter_mut().enumerate() {
                                if t.yt_id.as_deref() == Some(id.as_str()) {
                                    t.path = path.clone();
                                    at.get_or_insert(k);
                                }
                            }
                            match at {
                                Some(k) => self.play_index(k, true),
                                None => self.error = Some("The song left the list.".to_string()),
                            }
                        }
                        Err(e) => self.error = Some(format!("YouTube: {e}")),
                    }
                }
            }
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
            return self.next_shuffled();
        }
        let base = match self.current {
            Some(c) if self.filtered.contains(&c) => Some(c),
            _ => self.queue_resume.filter(|r| self.filtered.contains(r)),
        };
        let pos = base.and_then(|c| self.filtered.iter().position(|&i| i == c));
        // Next playable song after the current one (explicit songs are skipped when they are off).
        let after = pos.map_or(0, |p| p + 1);
        if let Some(i) = self
            .filtered
            .iter()
            .skip(after)
            .copied()
            .find(|&i| !self.is_blocked(i))
        {
            return Some(i);
        }
        if auto && self.cfg.repeat == Repeat::Off && pos.is_some() {
            return None;
        }
        self.filtered.iter().copied().find(|&i| !self.is_blocked(i))
    }

    /// True for explicit songs while "Explicit content" is off.
    fn is_blocked(&self, i: usize) -> bool {
        !self.cfg.allow_explicit && self.tracks.get(i).map_or(false, |t| t.explicit)
    }

    /// Shuffle without repeats: every song plays once per round. Smart shuffle also avoids
    /// playing the same artist twice in a row (when the list has other artists to choose from).
    fn next_shuffled(&mut self) -> Option<usize> {
        let cur = self.current;
        let mut pool: Vec<usize> = self
            .filtered
            .iter()
            .copied()
            .filter(|i| Some(*i) != cur && !self.shuffle_seen.contains(i) && !self.is_blocked(*i))
            .collect();
        if pool.is_empty() {
            // Everything was heard: start a new round.
            self.shuffle_seen.clear();
            if let Some(c) = cur {
                self.shuffle_seen.insert(c);
            }
            pool = self
                .filtered
                .iter()
                .copied()
                .filter(|i| Some(*i) != cur && !self.is_blocked(*i))
                .collect();
        }
        if pool.is_empty() {
            return cur.or_else(|| self.filtered.first().copied());
        }
        if self.cfg.smart_shuffle {
            let artist = cur
                .and_then(|c| self.tracks.get(c))
                .map(|t| t.artist.to_lowercase());
            if let Some(artist) = artist {
                let varied: Vec<usize> = pool
                    .iter()
                    .copied()
                    .filter(|&i| {
                        self.tracks
                            .get(i)
                            .map_or(true, |t| t.artist.to_lowercase() != artist)
                    })
                    .collect();
                if !varied.is_empty() {
                    pool = varied;
                }
            }
        }
        let k = self.rand(pool.len());
        Some(pool[k])
    }

    /// First song for a "Play" button: random when shuffle is on, otherwise the top of the list.
    fn start_index(&mut self) -> Option<usize> {
        if self.filtered.is_empty() {
            return None;
        }
        if self.cfg.shuffle {
            let k = self.rand(self.filtered.len());
            Some(self.filtered[k])
        } else {
            self.filtered.first().copied()
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
        } else if self.now.is_none()
            && self.view == View::Spotify
            && self.sp_nav.last().map_or(false, |p| !p.tracks.is_empty())
        {
            // Nothing is playing yet and a Spotify list is open: play that list.
            let ctx = self.ctx.clone();
            self.apply(Action::SpPlayAll, &ctx);
        } else if let Some(i) = self.current.or_else(|| self.filtered.first().copied()) {
            self.play_index(i, true);
        }
    }

    fn seek(&mut self, secs: f32) {
        if self.yt_wait.is_some() {
            return; // still downloading the next YouTube song
        }
        let Some(track) = self.now.clone() else { return };
        let Some(p) = self.player.as_mut() else { return };
        if !p.is_loaded() {
            return;
        }
        let dur = track.duration.as_secs_f32();
        let mut s = secs.max(0.0);
        if dur > 0.0 {
            s = s.min((dur - 0.5).max(0.0));
        }
        let playing = p.is_playing();
        if let Err(e) = p.load(&track, Duration::from_secs_f32(s), playing) {
            self.error = Some(e);
        }
    }

    /// Plays the first queued song. Returns false when the queue is empty.
    fn play_from_queue(&mut self) -> bool {
        if self.queue.is_empty() {
            return false;
        }
        let t = self.queue.remove(0);
        self.play_extra(t);
        true
    }

    /// Plays a song that is not in the active list (a queued or recent one). The list position is
    /// remembered so the list carries on from there afterwards.
    fn play_extra(&mut self, t: Track) {
        if let Some(c) = self.current {
            if self.filtered.contains(&c) {
                self.queue_resume = Some(c);
            }
        }
        self.tracks.push(t);
        let i = self.tracks.len() - 1;
        self.play_index(i, true);
    }

    /// Puts a song at the front of the Recent list (no duplicates, at most `RECENT_MAX`).
    fn note_recent(&mut self, t: Track) {
        let same = |a: &Track, b: &Track| {
            if a.spotify_id.is_some() || b.spotify_id.is_some() {
                a.spotify_id == b.spotify_id
            } else if a.yt_id.is_some() || b.yt_id.is_some() {
                a.yt_id == b.yt_id
            } else {
                a.path == b.path
            }
        };
        self.recent.retain(|r| !same(r, &t));
        self.recent.push_front(t);
        self.recent.truncate(RECENT_MAX);
    }

    fn on_finished(&mut self) {
        if self.cfg.repeat != Repeat::One && self.play_from_queue() {
            return;
        }
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
                if self.play_from_queue() {
                } else if let Some(i) = self.next_index(false) {
                    self.play_index(i, true);
                }
            }
            Action::QueueNext(t) => {
                self.queue.insert(0, t);
                self.notice = Some(("Playing next".to_string(), Instant::now()));
            }
            Action::QueueAdd(t) => {
                self.queue.push(t);
                self.notice = Some(("Added to queue".to_string(), Instant::now()));
            }
            Action::QueueRemove(k) => {
                if k < self.queue.len() {
                    self.queue.remove(k);
                }
            }
            Action::QueueClear => self.queue.clear(),
            Action::OpenQueue => {
                self.cfg.queue_panel = !self.cfg.queue_panel;
                if self.cfg.queue_panel {
                    // One right-hand panel at a time.
                    self.cfg.lyrics_panel = false;
                }
                self.cfg_dirty = true;
            }
            Action::QueuePlay(k) => {
                if k < self.queue.len() {
                    let t = self.queue.remove(k);
                    self.play_extra(t);
                }
            }
            Action::PlayRecent(k) => {
                if let Some(t) = self.recent.get(k).cloned() {
                    self.play_extra(t);
                }
            }
            Action::RowMenu(t, pos) => self.row_menu = Some((t, pos, Instant::now())),
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
                let next = match self.cfg.shuffle_mode() {
                    ShuffleMode::Off => ShuffleMode::On,
                    ShuffleMode::On => ShuffleMode::Smart,
                    ShuffleMode::Smart => ShuffleMode::Off,
                };
                self.cfg.set_shuffle_mode(next);
                self.shuffle_seen.clear();
                if let Some(c) = self.current {
                    self.shuffle_seen.insert(c);
                }
                self.cfg_dirty = true;
            }
            Action::SetShuffle(mode) => {
                self.cfg.set_shuffle_mode(mode);
                self.shuffle_seen.clear();
                if let Some(c) = self.current {
                    self.shuffle_seen.insert(c);
                }
                self.cfg_dirty = true;
            }
            Action::PlayAll => {
                if let Some(i) = self.start_index() {
                    self.play_index(i, true);
                }
            }
            Action::SpPlayAll => {
                let list = self.sp_nav.last().map(|p| p.tracks.clone()).unwrap_or_default();
                if !list.is_empty() {
                    let start = if self.cfg.shuffle { self.rand(list.len()) } else { 0 };
                    let from = self.sp_nav.last().map(|p| (p.node.clone(), p.title.clone()));
                    self.set_list(list, start, false);
                    self.playing_from = from;
                }
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
            Action::OpenYoutube => {
                self.view = View::YouTube;
                // Do not show Liked songs / playlists here: bring back the last YouTube results.
                if !self.tracks.iter().any(|t| t.yt_id.is_some()) && !self.yt_list.is_empty() {
                    self.tracks = self.yt_list.clone();
                    self.list_is_local = false;
                    self.history.clear();
                    self.shuffle_seen.clear();
                    self.current = None;
                    self.query.clear();
                    self.refilter();
                }
            }
            Action::Themes => {
                if self.cfg.ipod_mode {
                    self.themes_open = true;
                } else {
                    self.view = View::Themes;
                }
            }
            Action::Settings => self.view = View::Settings,
            Action::SpRoot(n) => {
                if matches!(&n, Node::SpSearch(q) if q.is_empty()) {
                    // The empty search page suggests things from the user's library.
                    for k in [Node::SpPlaylists, Node::SpLiked] {
                        let fresh = self
                            .page_cache
                            .get(&format!("{k:?}"))
                            .map_or(false, |(at, _, _)| at.elapsed() < Duration::from_secs(600));
                        if !fresh {
                            self.spawn_fetch(k);
                        }
                    }
                    self.sp_link.clear();
                    self.sp_live.clear();
                }
                self.view = View::Spotify;
                self.sp_nav.clear();
                self.sp_push(n);
            }
            Action::SpPush(n) => self.sp_push(n),
            Action::SpRefresh => {
                if let Some(p) = self.sp_nav.pop() {
                    self.page_cache.remove(&format!("{:?}", p.node));
                    self.sp_push(p.node);
                }
            }
            Action::SpLink(text)
                if text.trim_start().to_lowercase().starts_with("yt:")
                    || text.trim_start().to_lowercase().starts_with("yt ") =>
            {
                self.notice = Some(("YouTube is coming soon.".to_string(), Instant::now()));
            }
            Action::SpLink(text) => match parse_spotify_link(&text) {
                Some(node) => {
                    self.view = View::Spotify;
                    self.sp_nav.clear();
                    self.sp_push(node);
                }
                None => {
                    // Not a link: search the Spotify catalogue.
                    self.view = View::Spotify;
                    self.sp_nav.clear();
                    self.sp_live = text.clone();
                    self.remember_search(&text);
                    self.sp_push(Node::SpSearch(text));
                }
            },
            Action::SpLive(q) => {
                self.view = View::Spotify;
                self.sp_live = q.clone();
                let on_search = matches!(self.sp_nav.last().map(|p| &p.node), Some(Node::SpSearch(_)));
                let prev = if on_search { self.sp_nav.pop() } else { None };
                self.sp_push(Node::SpSearch(q));
                // Keep the old results on screen while the new ones load (no flicker).
                if let (Some(old), Some(new)) = (prev, self.sp_nav.last_mut()) {
                    if new.load == Load::Loading && new.items.is_empty() && old.load == Load::Ready {
                        new.items = old.items;
                        new.tracks = old.tracks;
                    }
                }
            }
            Action::ClearRecent => {
                self.cfg.recent_searches.clear();
                self.cfg_dirty = true;
            }
            Action::SpAddToPlaylist(pid, uris) => {
                let name = self.playlist_name(&pid);
                let reload = self.playlist_node(&pid);
                self.forget_playlist(&pid);
                let n = uris.len();
                self.sp_task(reload, move |c| {
                    c.add_to_playlist(&pid, &uris)?;
                    Ok(format!("Added {n} song{} to {name}", if n == 1 { "" } else { "s" }))
                });
            }
            Action::SpRemoveFromPlaylist(pid, uris) => {
                let name = self.playlist_name(&pid);
                let reload = self.playlist_node(&pid);
                self.forget_playlist(&pid);
                self.sp_task(reload, move |c| {
                    c.remove_from_playlist(&pid, &uris)?;
                    Ok(format!("Removed from {name}"))
                });
            }
            Action::SpSave(uris) => {
                for u in &uris {
                    if let Some(id) = u.strip_prefix("spotify:track:") {
                        self.liked_ids.insert(id.to_string());
                    }
                }
                self.forget_library_pages();
                self.sp_task(None, move |c| {
                    c.library_add(&uris)?;
                    Ok("Saved to your library".to_string())
                });
            }
            Action::SpUnsave(uris) => {
                for u in &uris {
                    if let Some(id) = u.strip_prefix("spotify:track:") {
                        self.liked_ids.remove(id);
                    }
                }
                self.forget_library_pages();
                let reload = self.sp_nav.last().map(|p| p.node.clone()).filter(|n| {
                    matches!(
                        n,
                        Node::SpLiked | Node::SpPlaylists | Node::SpAlbums | Node::SpArtists | Node::SpShows
                    )
                });
                self.sp_task(reload, move |c| {
                    c.library_remove(&uris)?;
                    Ok("Removed from your library".to_string())
                });
            }
            Action::SpDialog(d) => {
                self.dialog_text = match &d {
                    Dialog::Rename(_, name) => name.clone(),
                    _ => String::new(),
                };
                self.dialog = Some(d);
            }
            Action::SpDialogOk => self.dialog_ok(),
            Action::SpBack => {
                if self.sp_nav.len() > 1 {
                    self.sp_nav.pop();
                } else if self.sp_can_back() {
                    self.view = View::Home;
                }
            }
            Action::PlayTop(k, i) => {
                let t = self.sp_top_songs.get(k).and_then(|o| o.as_ref()).and_then(|(_, v)| v.get(i)).cloned();
                if let Some(t) = t {
                    self.play_extra(t);
                }
            }
            Action::PlayPlay(i) => {
                if let Some(t) = self.sp_recent_plays.get(i).map(|p| p.track.clone()) {
                    self.play_extra(t);
                }
            }
            Action::RetryShelves => {
                self.shelves_next = None;
                self.sp_shelves = None;
            }
            Action::Home => self.view = View::Home,
            Action::HomeOpen(n) => {
                self.home_back = Some(n.clone());
                self.view = View::Spotify;
                self.sp_nav.clear();
                self.sp_push(n);
            }
            Action::SpPlay(i) => {
                let list = self.sp_nav.last().map(|p| p.tracks.clone()).unwrap_or_default();
                if !list.is_empty() {
                    let from = self.sp_nav.last().map(|p| (p.node.clone(), p.title.clone()));
                    self.set_list(list, i, false);
                    self.playing_from = from;
                }
            }
            Action::OpenArtist(id, name, sp) => self.open_meta(false, id, name, sp),
            Action::OpenAlbum(id, name, sp) => self.open_meta(true, id, name, sp),
            Action::GoToSource => match self.playing_from.clone() {
                Some((node, _)) => {
                    self.view = View::Spotify;
                    self.sp_nav.clear();
                    self.sp_push(node);
                }
                None => self.view = View::Library,
            },
            Action::Rescan => self.rescan(),
            Action::AddFiles => {
                if let Some(paths) = rfd::FileDialog::new()
                    .add_filter("Audio", library::EXTENSIONS)
                    .pick_files()
                {
                    self.add_paths(paths, ctx);
                }
            }
            Action::SpotifyWindow => {
                if self.cfg.ipod_mode {
                    self.sp_window = true;
                } else {
                    self.view = View::Account;
                }
            }
            Action::ToggleExplicit => {
                self.cfg.allow_explicit = !self.cfg.allow_explicit;
                self.cfg_dirty = true;
            }
            Action::ToggleLyricsPanel => {
                self.cfg.lyrics_panel = !self.cfg.lyrics_panel;
                if self.cfg.lyrics_panel {
                    self.cfg.show_lyrics = true;
                    self.cfg.queue_panel = false;
                }
                self.cfg_dirty = true;
            }
            Action::ToggleLyricsFull => {
                self.lyrics_full = !self.lyrics_full;
                if self.lyrics_full {
                    self.cfg.show_lyrics = true;
                    self.lyr_snap = true;
                    self.lyr_follow = true;
                    self.cfg_dirty = true;
                }
            }
            Action::ToggleLyrics => {
                self.cfg.show_lyrics = !self.cfg.show_lyrics;
                self.cfg_dirty = true;
            }
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
        self.poll_yt();
        self.poll_store();
        self.sync_lyrics();
        self.art.poll(ctx);
        let mut remote = None;
        if let Some(p) = self.player.as_mut() {
            for m in p.poll() {
                self.error = Some(m);
            }
            remote = p.take_remote();
            // The song did start after all (slowly): drop the stale "did not start" message.
            if (p.is_playing() || p.position() > Duration::from_millis(1500))
                && self.error.as_deref().map_or(false, |e| e.starts_with("Spotify did not start"))
            {
                self.error = None;
            }
            if p.volume_pending() {
                ctx.request_repaint_after(Duration::from_millis(120));
            }
        }
        if let Some(id) = remote {
            self.on_remote(id);
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
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(false));
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
        self.first_run_window(ctx);
        self.ipod_search_ui(ctx);
        self.themes_window(ctx);
        self.check_sleep(ctx);

        // Media keys and the system media overlay.
        let playing_now = self.player.as_ref().map_or(false, |p| p.is_playing());
        if let Some(m) = self.media.as_mut() {
            use crate::mediakeys::Cmd;
            for c in m.poll() {
                match c {
                    Cmd::Toggle => actions.push(Action::Toggle),
                    Cmd::Play if !playing_now => actions.push(Action::Toggle),
                    Cmd::Pause if playing_now => actions.push(Action::Toggle),
                    Cmd::Next => actions.push(Action::Next),
                    Cmd::Prev => actions.push(Action::Prev),
                    _ => {}
                }
            }
            let now = self
                .now
                .as_ref()
                .map(|t| (t.title.as_str(), t.artist.as_str(), t.album.as_str(), t.duration));
            m.update(now, playing_now);
        }

        // Tray icon: its menu, left click, hover text, and "close button hides to tray".
        if let Some(t) = self.tray.as_mut() {
            use crate::tray::Cmd;
            for c in t.poll() {
                match c {
                    Cmd::PlayPause => actions.push(Action::Toggle),
                    Cmd::Next => actions.push(Action::Next),
                    Cmd::Prev => actions.push(Action::Prev),
                    Cmd::Show | Cmd::ShowHide => {
                        let hide = c == Cmd::ShowHide && !self.tray_hidden;
                        self.tray_hidden = hide;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(!hide));
                        if !hide {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                        }
                    }
                    Cmd::Quit => {
                        crate::tray::QUITTING.store(true, std::sync::atomic::Ordering::SeqCst);
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
            }
            let tip = match self.now.as_ref() {
                Some(n) if !n.artist.is_empty() => format!("{} - {}", n.artist, n.title),
                Some(n) => n.title.clone(),
                None => "litewave".to_string(),
            };
            t.set_tooltip(&tip);
            let quitting = crate::tray::QUITTING.load(std::sync::atomic::Ordering::SeqCst);
            if self.cfg.close_to_tray
                && !quitting
                && ctx.input(|i| i.viewport().close_requested())
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                self.tray_hidden = true;
            }
        }

        for a in actions {
            self.apply(a, ctx);
        }
        if self.query != self.last_query {
            self.refilter();
        }

        // The window title is always the app name (the song is shown inside the app).
        let want = "litewave".to_string();
        if want != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(want.clone()));
            self.window_title = want;
        }

        // Remember the local song and position every few seconds.
        if self.resume_saved.elapsed() >= Duration::from_secs(5) {
            self.resume_saved = Instant::now();
            if let Some(t) = &self.now {
                if t.spotify_id.is_none() && t.yt_id.is_none() && !t.path.as_os_str().is_empty() {
                    crate::resume::save(&t.path, self.position_secs());
                }
            }
        }

        // Debounced config save (not while a slider is being dragged).
        // The save runs on a worker thread: it writes the Spotify tokens to Windows Credential Manager,
        // and nothing like that may ever stall the window.
        if self.cfg_dirty && !ctx.input(|i| i.pointer.any_down()) {
            if self.cfg_saving.load(Ordering::Acquire) {
                ctx.request_repaint_after(Duration::from_millis(100)); // try again shortly
            } else {
                self.cfg_saving.store(true, Ordering::Release);
                let cfg = self.cfg.clone();
                let flag = self.cfg_saving.clone();
                std::thread::spawn(move || {
                    cfg.save();
                    flag.store(false, Ordering::Release);
                });
                self.cfg_dirty = false;
            }
        }

        if self.player.as_ref().map_or(false, |p| p.is_playing() || p.is_buffering()) {
            // Synced lyrics change line mid-second, so tick faster while they are on screen.
            let synced = self.cfg.show_lyrics && self.lyrics.as_ref().map_or(false, |l| l.synced);
            let playing_now = self.player.as_ref().map_or(false, |p| p.is_playing());
            // Fast enough for the equalizer bars to look smooth; slower while only buffering.
            ctx.request_repaint_after(Duration::from_millis(if playing_now {
                if synced { 50 } else { 60 }
            } else {
                250
            }));
        }
    }
}

/// Understands open.spotify.com links and spotify: URIs for playlists, albums and artists.
fn parse_spotify_link(s: &str) -> Option<Node> {
    let parts: Vec<&str> = s.trim().split(|c| c == '/' || c == ':' || c == '?').collect();
    for w in parts.windows(2) {
        let id = w[1];
        if id.len() == 22 && id.chars().all(|c| c.is_ascii_alphanumeric()) {
            return match w[0] {
                "playlist" => Some(Node::SpPlaylist(id.to_string(), "Playlist".to_string())),
                "album" => Some(Node::SpAlbum(id.to_string(), "Album".to_string())),
                "artist" => Some(Node::SpArtist(id.to_string(), "Artist".to_string())),
                _ => None,
            };
        }
    }
    None
}

/// Short status shown instead of the artist while a Spotify track is buffering.
/// It gets friendlier the longer the wait, so a slow start never looks like a freeze.
pub(crate) fn buffering_text(secs: f32) -> &'static str {
    if secs < 3.0 {
        "Buffering..."
    } else if secs < 15.0 {
        "Loading, please wait"
    } else {
        "Still loading, hang on"
    }
}

/// Hint under "Loading..." on Spotify pages.
pub(crate) fn loading_hint(secs: f32) -> &'static str {
    if secs < 4.0 {
        "This can take a few seconds"
    } else if secs < 12.0 {
        "Spotify can be slow, hang on"
    } else {
        "Taking a while. Go back and retry?"
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
    use super::{buffering_text, fmt_time, loading_hint};

    #[test]
    fn waiting_messages_change_over_time() {
        assert_eq!(buffering_text(0.5), "Buffering...");
        assert_ne!(buffering_text(5.0), buffering_text(0.5));
        assert_ne!(buffering_text(30.0), buffering_text(5.0));
        assert_ne!(loading_hint(1.0), loading_hint(20.0));
    }

    #[test]
    fn formats_time() {
        assert_eq!(fmt_time(0.0), "0:00");
        assert_eq!(fmt_time(65.4), "1:05");
        assert_eq!(fmt_time(3725.0), "1:02:05");
        assert_eq!(fmt_time(-3.0), "0:00");
    }
}


/// The litewave logo as a texture, for the sidebar header.
fn load_logo(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let img = image::load_from_memory(include_bytes!("../assets/icon.png")).ok()?;
    let img = img.resize(96, 96, image::imageops::FilterType::Lanczos3).to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
    Some(ctx.load_texture("litewave-logo", color, egui::TextureOptions::LINEAR))
}


// ---- saved recent plays -------------------------------------------------------------------------
// Home's quick tiles and the "Recently played" order of your playlists come from Spotify's recently played
// list. A copy is kept on disk so they are still there after a restart and while Spotify is rate-limiting.

fn recent_cache_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|d| d.join("litewave").join("recent.json"))
}

fn entries_json(v: &[spotify::Entry]) -> serde_json::Value {
    serde_json::Value::Array(
        v.iter()
            .map(|e| serde_json::json!({"id": e.id, "name": e.name, "sub": e.sub, "art": e.art}))
            .collect(),
    )
}

fn entries_from(v: &serde_json::Value) -> Vec<spotify::Entry> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| {
                    Some(spotify::Entry {
                        id: x["id"].as_str()?.to_string(),
                        name: x["name"].as_str()?.to_string(),
                        sub: x["sub"].as_str().unwrap_or("").to_string(),
                        art: x["art"].as_str().map(|s| s.to_string()),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn save_recent(
    ids: &[String],
    albums: &[spotify::Entry],
    top: &[spotify::Entry],
    plays: &[spotify::RecentPlay],
) {
    let Some(path) = recent_cache_path() else { return };
    let plays_json: Vec<serde_json::Value> = plays
        .iter()
        .filter_map(|p| {
            let t = &p.track;
            let id = t.spotify_id.as_ref()?;
            Some(serde_json::json!({
                "ctx": p.context,
                "id": id,
                "title": t.title,
                "artist": t.artist,
                "album": t.album,
                "album_id": t.album_id,
                "artist_id": t.artist_id,
                "art": t.art_url,
                "ms": t.duration.as_millis() as u64,
                "explicit": t.explicit,
            }))
        })
        .collect();
    let v = serde_json::json!({
        "ids": ids,
        "albums": entries_json(albums),
        "top": entries_json(top),
        "plays": plays_json,
    });
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, v.to_string());
}

#[allow(clippy::type_complexity)]
fn load_recent() -> Option<(Vec<String>, Vec<spotify::Entry>, Vec<spotify::Entry>, Vec<spotify::RecentPlay>)> {
    let text = std::fs::read_to_string(recent_cache_path()?).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let ids: Vec<String> = v["ids"]
        .as_array()?
        .iter()
        .filter_map(|x| x.as_str().map(|s| s.to_string()))
        .collect();
    let plays: Vec<spotify::RecentPlay> = v["plays"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| {
                    let mut t = crate::library::Track::spotify(
                        x["id"].as_str()?,
                        x["title"].as_str().unwrap_or(""),
                        x["artist"].as_str().unwrap_or(""),
                        x["album"].as_str().unwrap_or(""),
                        x["ms"].as_u64().unwrap_or(0),
                        x["art"].as_str().map(|s| s.to_string()),
                    );
                    t.album_id = x["album_id"].as_str().map(|s| s.to_string());
                    t.artist_id = x["artist_id"].as_str().map(|s| s.to_string());
                    t.explicit = x["explicit"].as_bool().unwrap_or(false);
                    Some(spotify::RecentPlay { context: x["ctx"].as_str().map(|s| s.to_string()), track: t })
                })
                .collect()
        })
        .unwrap_or_default();
    Some((ids, entries_from(&v["albums"]), entries_from(&v["top"]), plays))
}
