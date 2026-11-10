use super::{Entry, SearchResults, SpEvent};
use crate::library::Track;
use std::{path::PathBuf, time::Duration};

const OFF: &str = "This build has no Spotify support. Rebuild with: cargo run --release --features spotify";

pub const SHARED_CLIENT_ID: &str = "";
pub const REDIRECT_URI: &str = "";

pub fn last_log() -> String {
    String::new()
}

pub fn log_path() -> Option<PathBuf> {
    None
}

pub fn status_note() -> String {
    String::new()
}

#[derive(Clone)]
pub struct Client;

impl Client {
    pub fn login(_client_id: &str) -> Result<Client, String> {
        Err(OFF.to_string())
    }
    pub fn from_refresh(_refresh: &str, _client_id: &str) -> Result<Client, String> {
        Err(OFF.to_string())
    }
    pub fn refresh_token(&self) -> String {
        String::new()
    }
    pub fn access_token(&self) -> Result<String, String> {
        Err(OFF.to_string())
    }
    pub fn playlists(&self) -> Result<Vec<Entry>, String> {
        Err(OFF.to_string())
    }
    pub fn playlist_tracks(&self, _id: &str) -> Result<Vec<Track>, String> {
        Err(OFF.to_string())
    }
    pub fn liked(&self) -> Result<Vec<Track>, String> {
        Err(OFF.to_string())
    }
    pub fn saved_albums(&self) -> Result<Vec<Entry>, String> {
        Err(OFF.to_string())
    }
    pub fn album_tracks(&self, _id: &str, _name: &str) -> Result<Vec<Track>, String> {
        Err(OFF.to_string())
    }
    pub fn followed_artists(&self) -> Result<Vec<Entry>, String> {
        Err(OFF.to_string())
    }
    pub fn artist_albums(&self, _id: &str) -> Result<Vec<Entry>, String> {
        Err(OFF.to_string())
    }
    pub fn search(&self, _q: &str) -> Result<SearchResults, String> {
        Err(OFF.to_string())
    }
    pub fn saved_shows(&self) -> Result<Vec<Entry>, String> {
        Err(OFF.to_string())
    }
    pub fn show_episodes(&self, _id: &str, _name: &str) -> Result<Vec<Track>, String> {
        Err(OFF.to_string())
    }
    pub fn editable_playlists(&self) -> Result<Vec<Entry>, String> {
        Err(OFF.to_string())
    }
    pub fn create_playlist(&self, _name: &str) -> Result<Entry, String> {
        Err(OFF.to_string())
    }
    pub fn rename_playlist(&self, _id: &str, _name: &str) -> Result<(), String> {
        Err(OFF.to_string())
    }
    pub fn add_to_playlist(&self, _id: &str, _uris: &[String]) -> Result<(), String> {
        Err(OFF.to_string())
    }
    pub fn remove_from_playlist(&self, _id: &str, _uris: &[String]) -> Result<(), String> {
        Err(OFF.to_string())
    }
    pub fn library_add(&self, _uris: &[String]) -> Result<(), String> {
        Err(OFF.to_string())
    }
    pub fn library_remove(&self, _uris: &[String]) -> Result<(), String> {
        Err(OFF.to_string())
    }
}

pub struct Engine;

impl Engine {
    pub fn connect(
        _access_token: &str,
        _ctx: eframe::egui::Context,
        _want_connect: bool,
        _volume: f32,
    ) -> Result<Engine, String> {
        Err(OFF.to_string())
    }
    pub fn has_connect(&self) -> bool {
        false
    }
    pub fn set_volume(&self, _v: f32) {}
    pub fn load_direct(&self, _id: &str, _start: Duration, _autoplay: bool) -> Result<(), String> {
        Ok(())
    }

    pub fn set_volume_local(&self, _v: f32) {}

    pub fn set_volume_remote(&self, _v: f32) {}

    pub fn load(&self, _id: &str, _start: Duration, _autoplay: bool) -> Result<(), String> {
        Err(OFF.to_string())
    }
    pub fn handle(&self) -> SessionHandle {
        SessionHandle
    }
    pub fn pause(&self) {}
    pub fn resume(&self) {}
    pub fn stop(&self) {}
    pub fn poll(&self) -> Vec<SpEvent> {
        Vec::new()
    }
}

/// Stand-in for the playback-connection reader (not in this build).
#[derive(Clone)]
pub struct SessionHandle;

impl SessionHandle {
    pub fn track(&self, _id: &str) -> Result<crate::library::Track, String> {
        Err(OFF.to_string())
    }
    pub fn playlist(&self, _id: &str) -> Result<Vec<crate::library::Track>, String> {
        Err(OFF.to_string())
    }
}
