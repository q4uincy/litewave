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
    pub spotify_refresh_token: Option<String>,
    pub theme: Theme,
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
            spotify_refresh_token: None,
            theme: Theme::default(),
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
