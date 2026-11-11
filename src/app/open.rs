//! Opening files and folders from "Open with litewave", the command line and drag-and-drop.
use super::App;
use crate::library::{self, Track};
use std::path::PathBuf;

impl App {
    /// Plays files (or folders of music) opened with litewave or dropped onto the window.
    pub fn open_paths(&mut self, paths: Vec<PathBuf>) {
        let mut files: Vec<PathBuf> = Vec::new();
        for p in paths {
            if p.is_dir() {
                for e in walkdir::WalkDir::new(&p).into_iter().flatten() {
                    if e.file_type().is_file() && library::is_audio(e.path()) {
                        files.push(e.path().to_path_buf());
                    }
                }
            } else if library::is_audio(&p) {
                files.push(p);
            }
        }
        if files.is_empty() {
            return;
        }
        files.sort();
        let list: Vec<Track> = files.iter().map(|f| library::read_track(f)).collect();
        self.set_list(list, 0, false);
    }
}
