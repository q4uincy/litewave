use eframe::egui;
use lofty::prelude::*;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::mpsc::{channel, Receiver},
    time::Duration,
};
use walkdir::WalkDir;

pub const EXTENSIONS: &[&str] = &["mp3", "flac", "ogg", "oga", "wav", "m4a", "aac"];

#[derive(Clone)]
pub struct Track {
    /// Local file path (empty for Spotify tracks).
    pub path: PathBuf,
    /// Base-62 Spotify track id, if this is a Spotify track.
    pub spotify_id: Option<String>,
    /// Cover image URL (Spotify tracks).
    pub art_url: Option<String>,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: Duration,
    /// Lowercased "title artist album", precomputed for fast filtering.
    pub search_key: String,
}

impl Track {
    pub fn spotify(
        id: &str,
        title: &str,
        artist: &str,
        album: &str,
        duration_ms: u64,
        art_url: Option<String>,
    ) -> Self {
        Self {
            path: PathBuf::new(),
            spotify_id: Some(id.to_string()),
            art_url,
            title: title.to_string(),
            artist: artist.to_string(),
            album: album.to_string(),
            duration: Duration::from_millis(duration_ms),
            search_key: format!("{title} {artist} {album}").to_lowercase(),
        }
    }
}

impl Track {
    /// Where this track's cover comes from, if it can have one.
    pub fn art_key(&self) -> Option<crate::art::ArtKey> {
        match (&self.spotify_id, &self.art_url) {
            (Some(_), Some(u)) => Some(crate::art::ArtKey::Url(u.clone())),
            (Some(_), None) => None,
            _ => Some(crate::art::ArtKey::File(self.path.clone())),
        }
    }
}

pub enum ScanMsg {
    Progress(usize),
    Done(Vec<Track>),
}

pub fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map_or(false, |e| EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

pub fn read_track(path: &Path) -> Track {
    let mut title = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Unknown".to_string());
    let mut artist = String::new();
    let mut album = String::new();
    let mut duration = Duration::ZERO;

    if let Ok(tagged) = lofty::read_from_path(path) {
        duration = tagged.properties().duration();
        if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
            if let Some(t) = tag.title() {
                if !t.trim().is_empty() {
                    title = t.to_string();
                }
            }
            if let Some(a) = tag.artist() {
                artist = a.to_string();
            }
            if let Some(a) = tag.album() {
                album = a.to_string();
            }
        }
    }
    if artist.trim().is_empty() {
        artist = "Unknown artist".to_string();
    }

    let search_key = format!("{title} {artist} {album}").to_lowercase();
    Track {
        path: path.to_path_buf(),
        spotify_id: None,
        art_url: None,
        title,
        artist,
        album,
        duration,
        search_key,
    }
}

pub fn sort_tracks(tracks: &mut [Track]) {
    tracks.sort_by_cached_key(|t| {
        (
            t.artist.to_lowercase(),
            t.album.to_lowercase(),
            t.title.to_lowercase(),
        )
    });
}

/// Scans `root` on a background thread so the UI never blocks.
/// Tracks already in `known` (matched by path) are reused instead of re-reading their tags.
pub fn spawn_scan(
    root: PathBuf,
    ctx: egui::Context,
    known: HashMap<PathBuf, Track>,
) -> Receiver<ScanMsg> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let mut tracks = Vec::new();
        for entry in WalkDir::new(&root)
            .follow_links(true)
            .into_iter()
            .filter_map(Result::ok)
        {
            if entry.file_type().is_file() && is_audio(entry.path()) {
                let track = match known.get(entry.path()) {
                    Some(t) => t.clone(),
                    None => read_track(entry.path()),
                };
                tracks.push(track);
                if tracks.len() % 50 == 0 {
                    let _ = tx.send(ScanMsg::Progress(tracks.len()));
                    ctx.request_repaint();
                }
            }
        }
        sort_tracks(&mut tracks);
        let _ = tx.send(ScanMsg::Done(tracks));
        ctx.request_repaint();
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_audio_extensions() {
        assert!(is_audio(Path::new("a/b/song.MP3")));
        assert!(is_audio(Path::new("song.flac")));
        assert!(!is_audio(Path::new("notes.txt")));
        assert!(!is_audio(Path::new("noext")));
    }

    #[test]
    fn spotify_track_has_id_and_search_key() {
        let t = Track::spotify("abc", "Song", "Band", "Album", 1500, None);
        assert_eq!(t.spotify_id.as_deref(), Some("abc"));
        assert_eq!(t.duration.as_millis(), 1500);
        assert!(t.search_key.contains("band"));
    }
}
