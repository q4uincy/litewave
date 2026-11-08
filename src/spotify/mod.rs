//! Spotify support. With the `spotify` cargo feature this talks to the official Web API
//! (library browsing, PKCE login with the user's own Client ID) and uses librespot for
//! built-in playback. Without the feature, `stub` provides the same API and every call
//! returns a friendly "not in this build" error.

#[cfg(feature = "spotify")]
mod real;
#[cfg(feature = "spotify")]
pub use real::{last_log, log_path, Client, Engine};

#[cfg(not(feature = "spotify"))]
mod stub;
#[cfg(not(feature = "spotify"))]
pub use stub::{last_log, log_path, Client, Engine};

pub const ENABLED: bool = cfg!(feature = "spotify");
pub const REDIRECT_URI: &str = "http://127.0.0.1:8898/callback";

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
    /// Anything else, for diagnostics.
    Info(String),
}
