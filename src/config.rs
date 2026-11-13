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
    /// The Queue / Recent panel on the right of the main window (closed by default).
    pub queue_panel: bool,
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
    /// Check GitHub for a newer litewave when the app starts.
    pub auto_update_check: bool,
    /// Interface zoom (0.8 - 1.5).
    pub ui_scale: f32,
    /// Spotify streaming quality: 0 low (96 kbps), 1 normal (160), 2 high (320). Applies on the next connect.
    pub spotify_quality: u8,
    /// Spotify volume normalisation (keeps songs at a similar loudness).
    pub spotify_normalise: bool,
    /// Spotify gapless playback.
    pub spotify_gapless: bool,
    /// Windows: the window's close button hides litewave to the tray icon instead of quitting.
    pub close_to_tray: bool,
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
            queue_panel: false,
            allow_explicit: true,
            ytdlp_path: String::new(),
            ytdlp_browser: String::new(),
            spotify_connect: true,
            recent_searches: Vec::new(),
            auto_update_check: true,
            ui_scale: 1.0,
            spotify_quality: 1,
            spotify_normalise: true,
            spotify_gapless: true,
            close_to_tray: false,
        }
    }
}

/// Spotify sign-in tokens live in Windows Credential Manager instead of config.json.
/// If the vault is unavailable, they stay in config.json as before.
#[cfg(windows)]
mod vault {
    pub fn get(name: &str) -> Option<String> {
        keyring::Entry::new("litewave", name).ok()?.get_password().ok()
    }
    pub fn set(name: &str, value: &str) -> bool {
        keyring::Entry::new("litewave", name)
            .and_then(|e| e.set_password(value))
            .is_ok()
    }
    pub fn delete(name: &str) {
        if let Ok(e) = keyring::Entry::new("litewave", name) {
            let _ = e.delete_credential();
        }
    }
}

#[cfg(not(windows))]
mod vault {
    pub fn get(_name: &str) -> Option<String> {
        None
    }
    pub fn set(_name: &str, _value: &str) -> bool {
        false
    }
    pub fn delete(_name: &str) {}
}

/// Moves a token into the vault. Returns what should still be written to config.json.
fn stash(name: &str, token: &Option<String>) -> Option<String> {
    match token {
        Some(t) => {
            if vault::set(name, t) {
                None
            } else {
                Some(t.clone())
            }
        }
        None => {
            vault::delete(name);
            None
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
        let mut cfg: Config = config_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        // A token still in config.json (older versions) is moved to the vault on the next save.
        if cfg.spotify_refresh.is_none() {
            cfg.spotify_refresh = vault::get("spotify_refresh");
        }
        if cfg.spotify_browse_refresh.is_none() {
            cfg.spotify_browse_refresh = vault::get("spotify_browse_refresh");
        }
        cfg
    }

    pub fn save(&self) {
        let Some(path) = config_path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut copy = self.clone();
        copy.spotify_refresh = stash("spotify_refresh", &self.spotify_refresh);
        copy.spotify_browse_refresh = stash("spotify_browse_refresh", &self.spotify_browse_refresh);
        if let Ok(json) = serde_json::to_string_pretty(&copy) {
            let _ = std::fs::write(path, json);
        }
    }
}
