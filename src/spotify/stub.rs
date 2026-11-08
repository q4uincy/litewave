use super::{Entry, SpEvent};
use crate::library::Track;
use std::{path::PathBuf, time::Duration};

const OFF: &str = "This build has no Spotify support. Rebuild with: cargo run --release --features spotify";

pub fn last_log() -> String {
    String::new()
}

pub fn log_path() -> Option<PathBuf> {
    None
}

#[derive(Clone)]
pub struct Client;

impl Client {
    pub fn login(_client_id: &str) -> Result<Client, String> {
        Err(OFF.to_string())
    }
    pub fn from_refresh(_client_id: &str, _refresh: &str) -> Result<Client, String> {
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
}

pub struct Engine;

impl Engine {
    pub fn connect(_access_token: &str) -> Result<Engine, String> {
        Err(OFF.to_string())
    }
    pub fn load(&self, _id: &str, _start: Duration, _autoplay: bool) -> Result<(), String> {
        Err(OFF.to_string())
    }
    pub fn pause(&self) {}
    pub fn resume(&self) {}
    pub fn stop(&self) {}
    pub fn poll(&self) -> Vec<SpEvent> {
        Vec::new()
    }
}
