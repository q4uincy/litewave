use crate::theme::Theme;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Spotify-style shuffle: off, plain shuffle, or smart shuffle (spreads artists out).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShuffleMode {
    Off,
    On,
    Smart,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum Repeat {
    #[default]
    Off,
    All,
    One,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub last_folder: Option<PathBuf>,
    pub volume: f32,
    pub shuffle: bool,
    /// With `shuffle` on: also keep the same artist from playing twice in a row.
    pub smart_shuffle: bool,
    pub repeat: Repeat,
    pub ipod_mode: bool,
    /// The user's own Spotify app Client ID (used for browsing). Empty = shared sign-in.
    pub spotify_client_id: String,
    /// Playback sign-in refresh token (librespot's client ID). Stored in plain text in the user's config directory.
    pub spotify_refresh: Option<String>,
    /// Refresh token for browsing, issued to `spotify_client_id` (the user's own Spotify app).
    pub spotify_browse_refresh: Option<String>,
    pub theme: Theme,
    /// Wallpaper decode size: 0 low (1280x720), 1 medium (1920x1080), 2 high (2560x1440), 3 ultra (3840x2160).
    pub wallpaper_quality: u8,
    /// iPod body: 0 silver, 1 black, 2 classic white, 3 pink.
    pub ipod_skin: u8,
    /// iPod menu wallpaper: None, "builtin:stardust", or an image path.
    pub ipod_wallpaper: Option<String>,
    /// How much the iPod wallpaper is darkened (0-255).
    pub ipod_wallpaper_dim: u8,
    /// Show synced lyrics (local .lrc files or LRCLIB) while a song plays.
    pub show_lyrics: bool,
    /// The lyrics panel in the main window (opened with the lyrics button; closed by default).
    pub lyrics_panel: bool,
    /// Like Spotify's "Allow explicit content": when off, explicit songs are greyed out and skipped.
    pub allow_explicit: bool,
    /// Path to the yt-dlp program for the optional YouTube source. Empty = look on the PATH.
    pub ytdlp_path: String,
    /// Browser whose YouTube login yt-dlp may use ("chrome", "edge", "firefox", "brave"), or empty.
    pub ytdlp_browser: String,
    /// Show litewave as a device in Spotify (Spotify Connect): your phone can see what plays here
    /// and control it. Takes effect the next time Spotify connects.
    pub spotify_connect: bool,
    /// Last searches typed in the Spotify search box (newest first, at most 10).
    pub recent_searches: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            last_folder: None,
            volume: 0.7,
            shuffle: false,
            smart_shuffle: false,
            repeat: Repeat::Off,
            ipod_mode: false,
            spotify_client_id: String::new(),
            spotify_refresh: None,
            spotify_browse_refresh: None,
            theme: Theme::default(),
            wallpaper_quality: 1,
            ipod_skin: 0,
            ipod_wallpaper: None,
            ipod_wallpaper_dim: 120,
            show_lyrics: true,
            lyrics_panel: false,
            allow_explicit: true,
            ytdlp_path: String::new(),
            ytdlp_browser: String::new(),
            spotify_connect: true,
            recent_searches: Vec::new(),
        }
    }
}

fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("litewave").join("config.json"))
}

impl Config {
    pub fn shuffle_mode(&self) -> ShuffleMode {
        match (self.shuffle, self.smart_shuffle) {
            (false, _) => ShuffleMode::Off,
            (true, false) => ShuffleMode::On,
            (true, true) => ShuffleMode::Smart,
        }
    }

    pub fn set_shuffle_mode(&mut self, mode: ShuffleMode) {
        self.shuffle = mode != ShuffleMode::Off;
        self.smart_shuffle = mode == ShuffleMode::Smart;
    }

    pub fn load() -> Self {
        config_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(path) = config_path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }
}
