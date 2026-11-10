use crate::theme::Theme;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
    pub repeat: Repeat,
    pub ipod_mode: bool,
    pub spotify_client_id: String,
    /// Stored in plain text in the user's config directory.
    pub spotify_refresh: Option<String>,
    pub theme: Theme,
    /// Wallpaper decode size: 0 low (1280x720), 1 medium (1920x1080), 2 high (2560x1440), 3 ultra (3840x2160).
    pub wallpaper_quality: u8,
    /// iPod body: 0 silver, 1 black, 2 classic white, 3 pink.
    pub ipod_skin: u8,
    /// iPod menu wallpaper: None, "builtin:stardust", or an image path.
    pub ipod_wallpaper: Option<String>,
    /// How much the iPod wallpaper is darkened (0-255).
    pub ipod_wallpaper_dim: u8,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            last_folder: None,
            volume: 0.7,
            shuffle: false,
            repeat: Repeat::Off,
            ipod_mode: false,
            spotify_client_id: String::new(),
            spotify_refresh: None,
            theme: Theme::default(),
            wallpaper_quality: 1,
            ipod_skin: 0,
            ipod_wallpaper: None,
            ipod_wallpaper_dim: 120,
        }
    }
}

fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("litewave").join("config.json"))
}

impl Config {
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
