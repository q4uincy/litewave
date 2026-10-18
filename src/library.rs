use eframe::egui;
use lofty::prelude::*;
use std::{
    path::{Path, PathBuf},
    sync::mpsc::{channel, Receiver},
    time::Duration,
};
use walkdir::WalkDir;

pub const EXTENSIONS: &[&str] = &["mp3", "flac", "ogg", "oga", "wav", "m4a", "aac"];

#[derive(Clone)]
pub struct Track {
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: Duration,
    /// Lowercased "title artist album", precomputed for fast filtering.
    pub search_key: String,
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
pub fn spawn_scan(root: PathBuf, ctx: egui::Context) -> Receiver<ScanMsg> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let mut tracks = Vec::new();
        for entry in WalkDir::new(&root)
            .follow_links(true)
            .into_iter()
            .filter_map(Result::ok)
        {
            if entry.file_type().is_file() && is_audio(entry.path()) {
                tracks.push(read_track(entry.path()));
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
}
