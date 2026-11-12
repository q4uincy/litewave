use crate::library::Track;
use crate::spotify::{self, SpEvent};
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};
use std::sync::atomic::{AtomicBool, Ordering};

/// Set while a file is being opened, so an expected decoder panic does not show the crash window.
pub static EXPECT_PANIC: AtomicBool = AtomicBool::new(false);
use std::{
    fs::File,
    io::BufReader,
    path::Path,
    time::{Duration, Instant},
};

/// Local-file player (rodio). Position is tracked with our own clock.
pub struct Player {
    _stream: OutputStream,
    handle: OutputStreamHandle,
    sink: Option<Sink>,
    volume: f32,
    base: Duration,
    started: Option<Instant>,
}

impl Player {
    pub fn new(volume: f32) -> Result<Self, String> {
        let (stream, handle) =
            OutputStream::try_default().map_err(|e| format!("No audio output device: {e}"))?;
        Ok(Self {
            _stream: stream,
            handle,
            sink: None,
            volume: volume.clamp(0.0, 1.0),
            base: Duration::ZERO,
            started: None,
        })
    }

    /// Loads `path`, starting at `start`. Also used for seeking.
    pub fn load(&mut self, path: &Path, start: Duration, autoplay: bool) -> Result<(), String> {
        if let Some(old) = self.sink.take() {
            old.stop();
        }
        self.base = start;
        self.started = None;

        let file = File::open(path).map_err(|e| format!("Cannot open file: {e}"))?;
        // rodio 0.19 can panic ("Seek errors should not occur during initialization") on some
        // files, such as certain downloaded m4a files. Catch that so a bad file is an error
        // message instead of a crash. The panic hook in main.rs stays quiet while this flag is set.
        EXPECT_PANIC.store(true, Ordering::Relaxed);
        let made = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Decoder::new(BufReader::with_capacity(256 * 1024, file))
        }));
        EXPECT_PANIC.store(false, Ordering::Relaxed);
        let decoder = match made {
            Ok(r) => r.map_err(|e| format!("Cannot decode file: {e}"))?,
            Err(_) => return Err("Cannot decode this file: its format could not be read.".to_string()),
        };
        let sink = Sink::try_new(&self.handle).map_err(|e| format!("Audio error: {e}"))?;
        sink.set_volume(self.volume);
        if !autoplay {
            sink.pause();
        }
        if start.is_zero() {
            sink.append(decoder);
        } else {
            // Jump straight to the position. skip_duration would decode everything before it,
            // which makes seeking (and resuming) late in a song slow.
            sink.append(decoder);
            if sink.try_seek(start).is_err() {
                // The format cannot seek: reopen and skip the slow way.
                sink.stop();
                let file = File::open(path).map_err(|e| format!("Cannot open file: {e}"))?;
                let again = Decoder::new(BufReader::with_capacity(256 * 1024, file))
                    .map_err(|e| format!("Cannot decode file: {e}"))?;
                let sink2 = Sink::try_new(&self.handle).map_err(|e| format!("Audio error: {e}"))?;
                sink2.set_volume(self.volume);
                if !autoplay {
                    sink2.pause();
                }
                sink2.append(again.skip_duration(start));
                self.sink = Some(sink2);
                if autoplay {
                    self.started = Some(Instant::now());
                }
                return Ok(());
            }
        }
        self.sink = Some(sink);
        if autoplay {
            self.started = Some(Instant::now());
        }
        Ok(())
    }

    pub fn toggle(&mut self) {
        let Some(sink) = &self.sink else { return };
        if sink.is_paused() {
            sink.play();
            self.started = Some(Instant::now());
        } else {
            sink.pause();
            self.base = self.position();
            self.started = None;
        }
    }

    pub fn stop(&mut self) {
        if let Some(s) = self.sink.take() {
            s.stop();
        }
        self.base = Duration::ZERO;
        self.started = None;
    }

    pub fn set_volume(&mut self, v: f32) {
        self.volume = v.clamp(0.0, 1.0);
        if let Some(s) = &self.sink {
            s.set_volume(self.volume);
        }
    }

    pub fn position(&self) -> Duration {
        self.base + self.started.map(|s| s.elapsed()).unwrap_or_default()
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    pub fn is_loaded(&self) -> bool {
        self.sink.is_some()
    }

    pub fn is_playing(&self) -> bool {
        self.started.is_some()
    }

    /// True when a playing track has run out of samples.
    pub fn finished(&self) -> bool {
        self.started.is_some() && self.sink.as_ref().map_or(false, |s| s.empty())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Active {
    None,
    Local,
    Spotify,
}

/// Routes playback to the local player or the Spotify engine, depending on the track.
pub struct Playback {
    local: Player,
    sp: Option<spotify::Engine>,
    active: Active,
    sp_base: Duration,
    sp_started: Option<Instant>,
    sp_loaded: bool,
    sp_loading: bool,
    sp_loading_since: Option<Instant>,
    sp_ended: bool,
    sp_track: Option<String>,
    sp_retries: u8,
    vol_pending: Option<f32>,
    vol_sent: Instant,
    sp_info: String,
    /// True while the phone (Spotify Connect) is the one choosing what plays.
    sp_remote: bool,
    /// A song the phone just started, waiting for the app to pick it up.
    remote_pending: Option<String>,
}

impl Playback {
    pub fn new(volume: f32) -> Result<Self, String> {
        Ok(Self {
            local: Player::new(volume)?,
            sp: None,
            active: Active::None,
            sp_base: Duration::ZERO,
            sp_started: None,
            sp_loaded: false,
            sp_loading: false,
            sp_loading_since: None,
            sp_ended: false,
            sp_track: None,
            sp_retries: 0,
            vol_pending: None,
            vol_sent: Instant::now(),
            sp_info: String::new(),
            sp_remote: false,
            remote_pending: None,
        })
    }

    pub fn set_spotify(&mut self, engine: spotify::Engine) {
        self.sp = Some(engine);
    }

    pub fn drop_spotify(&mut self) {
        if self.active == Active::Spotify {
            self.stop();
        }
        self.sp = None;
    }

    /// True while a Spotify track has been requested but audio has not started yet.
    pub fn is_buffering(&self) -> bool {
        self.active == Active::Spotify && self.sp_loading
    }

    /// Seconds a Spotify track has been buffering, if it is buffering right now.
    pub fn buffering_secs(&self) -> Option<f32> {
        if !self.is_buffering() {
            return None;
        }
        Some(self.sp_loading_since.map_or(0.0, |t| t.elapsed().as_secs_f32()))
    }

    /// The most recent diagnostic event from the Spotify engine.
    pub fn sp_info(&self) -> &str {
        &self.sp_info
    }

    /// Processes Spotify engine events. Returns error messages to show to the user.
    pub fn poll(&mut self) -> Vec<String> {
        let mut msgs = Vec::new();
        let events = match &self.sp {
            Some(sp) => sp.poll(),
            None => return msgs,
        };
        // Send the volume to Spotify Connect at most 4 times a second.
        if let Some(v) = self.vol_pending {
            if self.vol_sent.elapsed() >= Duration::from_millis(250) {
                if let Some(sp) = &self.sp {
                    sp.set_volume_remote(v);
                }
                self.vol_pending = None;
                self.vol_sent = Instant::now();
            }
        }
        // Watchdog: a track that does not start. First try again the normal way (what clicking the
        // song a second time does), then without Spotify Connect, then give up with a message.
        if self.active == Active::Spotify && self.sp_loading && !self.sp_remote {
            let waited = self.sp_loading_since.map_or(0.0, |t| t.elapsed().as_secs_f32());
            if let (Some(sp), Some(id)) = (&self.sp, self.sp_track.clone()) {
                if waited > 4.0 && self.sp_retries == 0 {
                    self.sp_retries = 1;
                    let _ = sp.load(&id, self.sp_base, true);
                    self.sp_loading_since = Some(Instant::now());
                } else if waited > 5.0 && self.sp_retries == 1 {
                    self.sp_retries = 2;
                    let _ = sp.load_direct(&id, self.sp_base, true);
                    self.sp_loading_since = Some(Instant::now());
                } else if waited > 10.0 && self.sp_retries >= 2 {
                    self.sp_loading = false;
                    self.sp_loaded = false;
                    self.sp_started = None;
                    msgs.push("Spotify did not start this song. Try it again, or sign in again in the Spotify window.".to_string());
                }
            }
        }
        for ev in events {
            match ev {
                SpEvent::Playing(pos) => {
                    if self.active == Active::Spotify {
                        self.sp_base = Duration::from_millis(pos as u64);
                        self.sp_started = Some(Instant::now());
                        self.sp_loading = false;
                    }
                }
                SpEvent::Paused => {
                    if self.active == Active::Spotify && self.sp_started.is_some() {
                        self.sp_base = self.position();
                        self.sp_started = None;
                    }
                }
                SpEvent::Ended => {
                    if self.active == Active::Spotify {
                        if self.sp_remote {
                            // The phone's queue decides what comes next, not litewave's list.
                            // If nothing follows, show it as stopped.
                            self.sp_base = self.position();
                            self.sp_started = None;
                        } else {
                            self.sp_ended = true;
                        }
                    }
                }
                SpEvent::Remote(id, pos) => {
                    self.local.stop();
                    self.active = Active::Spotify;
                    self.sp_remote = true;
                    self.sp_loaded = true;
                    self.sp_loading = false;
                    self.sp_ended = false;
                    self.sp_base = Duration::from_millis(pos as u64);
                    self.sp_started = Some(Instant::now());
                    self.remote_pending = Some(id);
                }
                SpEvent::Unavailable => {
                    // Often works on the second try: retry once quietly before showing an error.
                    if self.active == Active::Spotify && self.sp_retries == 0 {
                        if let (Some(sp), Some(id)) = (&self.sp, self.sp_track.clone()) {
                            self.sp_retries = 1;
                            if sp.load(&id, self.sp_base, true).is_ok() {
                                self.sp_loading = true;
                                self.sp_started = None;
                                self.sp_loading_since = Some(Instant::now());
                                continue;
                            }
                        }
                    }
                    if self.active == Active::Spotify {
                        self.sp_started = None;
                        self.sp_loading = false;
                        self.sp_loaded = false;
                        let last = spotify::last_log();
                        msgs.push(if last.contains("INVALID_CREDENTIALS") {
                            "Spotify rejected this sign-in for playback. Open the Spotify window and click Sign in again.".to_string()
                        } else if last.contains("Symphonia")
                            || last.contains("Unable to read audio")
                            || last.contains("decryption key")
                            || last.contains("audio key")
                        {
                            format!("Spotify sent audio that litewave could not open. This often means the account is not Spotify Premium (free accounts cannot stream here) or Spotify refused the decryption key. Details: {last}")
                        } else if last.is_empty() {
                            "Spotify could not play this track (Premium is required, and some tracks are region-locked).".to_string()
                        } else {
                            format!("Spotify could not play this track. {last}")
                        });
                    }
                }
                SpEvent::Info(s) => self.sp_info = s,
            }
        }
        msgs
    }

    /// The song the phone started, if there is a new one since the last call.
    pub fn take_remote(&mut self) -> Option<String> {
        self.remote_pending.take()
    }

    /// True when litewave shows up as a device in Spotify.
    pub fn has_connect(&self) -> bool {
        self.sp.as_ref().map_or(false, |s| s.has_connect())
    }

    pub fn load(&mut self, t: &Track, start: Duration, autoplay: bool) -> Result<(), String> {
        self.sp_remote = false;
        match &t.spotify_id {
            Some(id) => {
                let Some(sp) = &self.sp else {
                    return Err("Spotify is not connected".to_string());
                };
                self.local.stop();
                let _ = sp.poll(); // drop events that belong to the previous track
                self.sp_ended = false;
                sp.load(id, start, autoplay)?;
                // Spirc can reset the volume to the phone's value; push ours again.
                sp.set_volume(self.local.volume());
                self.sp_track = Some(id.clone());
                self.sp_retries = 0;
                self.active = Active::Spotify;
                self.sp_loaded = true;
                self.sp_loading = autoplay;
                self.sp_loading_since = Some(Instant::now());
                self.sp_ended = false;
                self.sp_base = start;
                self.sp_started = None;
                Ok(())
            }
            None => {
                if let Some(sp) = &self.sp {
                    sp.stop();
                }
                self.sp_loaded = false;
                self.sp_loading = false;
                self.sp_ended = false;
                self.sp_started = None;
                self.active = Active::Local;
                self.local.load(&t.path, start, autoplay)
            }
        }
    }

    pub fn toggle(&mut self) {
        match self.active {
            Active::Local => self.local.toggle(),
            Active::Spotify => {
                let Some(sp) = &self.sp else { return };
                if self.sp_loading {
                    sp.pause();
                    self.sp_loading = false;
                    self.sp_started = None;
                } else if self.sp_started.is_some() {
                    sp.pause();
                    self.sp_base = self.position();
                    self.sp_started = None;
                } else {
                    sp.resume();
                    self.sp_started = Some(Instant::now());
                }
            }
            Active::None => {}
        }
    }

    pub fn stop(&mut self) {
        self.local.stop();
        if let Some(sp) = &self.sp {
            sp.stop();
        }
        self.sp_loaded = false;
        self.sp_loading = false;
        self.sp_ended = false;
        self.sp_started = None;
        self.sp_base = Duration::ZERO;
        self.active = Active::None;
    }

    pub fn set_volume(&mut self, v: f32) {
        self.local.set_volume(v);
        if let Some(sp) = &self.sp {
            sp.set_volume_local(v); // instant
            self.vol_pending = Some(v); // the phone hears about it a moment later (see poll)
        }
    }

    /// True while a volume change still has to be sent to the phone.
    pub fn volume_pending(&self) -> bool {
        self.vol_pending.is_some()
    }

    pub fn position(&self) -> Duration {
        match self.active {
            Active::Local => self.local.position(),
            Active::Spotify => {
                if self.sp_loading {
                    self.sp_base
                } else {
                    self.sp_base + self.sp_started.map(|s| s.elapsed()).unwrap_or_default()
                }
            }
            Active::None => Duration::ZERO,
        }
    }

    pub fn is_loaded(&self) -> bool {
        match self.active {
            Active::Local => self.local.is_loaded(),
            Active::Spotify => self.sp_loaded,
            Active::None => false,
        }
    }

    pub fn is_playing(&self) -> bool {
        match self.active {
            Active::Local => self.local.is_playing(),
            Active::Spotify => self.sp_started.is_some() && !self.sp_loading,
            Active::None => false,
        }
    }

    pub fn finished(&self) -> bool {
        match self.active {
            Active::Local => self.local.finished(),
            Active::Spotify => self.sp_ended,
            Active::None => false,
        }
    }
}
