//! Spotify support. With the `spotify` cargo feature this talks to the official Web API
//! (library browsing, PKCE login with the user's own Client ID) and uses librespot for
//! built-in playback (signed in separately with librespot's own client ID). Without the feature, `stub` provides the same API and every call
//! returns a friendly "not in this build" error.

#[cfg(feature = "spotify")]
mod real;
#[cfg(feature = "spotify")]
pub use real::{
    last_log, log_path, set_playback_options, status_note, Client, Engine, SessionHandle, REDIRECT_URI, SHARED_CLIENT_ID,
};

#[cfg(not(feature = "spotify"))]
mod stub;
#[cfg(not(feature = "spotify"))]
pub use stub::{
    last_log, log_path, set_playback_options, status_note, Client, Engine, SessionHandle, REDIRECT_URI, SHARED_CLIENT_ID,
};

/// What a catalogue search found (up to 10 of each kind).
#[derive(Clone, Default)]
pub struct SearchResults {
    pub tracks: Vec<crate::library::Track>,
    pub artists: Vec<Entry>,
    pub albums: Vec<Entry>,
    pub playlists: Vec<Entry>,
    pub shows: Vec<Entry>,
}

pub const ENABLED: bool = cfg!(feature = "spotify");

/// A playlist, album or artist row.
#[derive(Clone)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub sub: String,
    pub art: Option<String>,
}

/// Events from the playback engine for the track it was last asked to play.
#[derive(Clone, Debug)]
pub enum SpEvent {
    /// Audio started (or resumed) at this position in milliseconds.
    Playing(u32),
    Paused,
    Ended,
    /// Spotify refused or failed to load the track.
    Unavailable,
    /// The phone (or another Spotify app) started this track on litewave: its id in litewave's
    /// format (`<id>` or `episode:<id>`) and the position in milliseconds.
    Remote(String, u32),
    /// Anything else, for diagnostics.
    Info(String),
}
