use crate::library::Track;
use crate::spotify::{self, SpEvent};
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};
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
        let decoder =
            Decoder::new(BufReader::new(file)).map_err(|e| format!("Cannot decode file: {e}"))?;
        let sink = Sink::try_new(&self.handle).map_err(|e| format!("Audio error: {e}"))?;
        sink.set_volume(self.volume);
        if !autoplay {
            sink.pause();
        }
        if start.is_zero() {
            sink.append(decoder);
        } else {
            sink.append(decoder.skip_duration(start));
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
    sp_ended: bool,
    sp_info: String,
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
            sp_ended: false,
            sp_info: String::new(),
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
                        self.sp_ended = true;
                    }
                }
                SpEvent::Unavailable => {
                    if self.active == Active::Spotify {
                        self.sp_started = None;
                        self.sp_loading = false;
                        self.sp_loaded = false;
                        let last = spotify::last_log();
                        msgs.push(if last.is_empty() {
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

    pub fn load(&mut self, t: &Track, start: Duration, autoplay: bool) -> Result<(), String> {
        match &t.spotify_id {
            Some(id) => {
                let Some(sp) = &self.sp else {
                    return Err("Spotify is not connected".to_string());
                };
                self.local.stop();
                let _ = sp.poll(); // drop events that belong to the previous track
                self.sp_ended = false;
                sp.load(id, start, autoplay)?;
                self.active = Active::Spotify;
                self.sp_loaded = true;
                self.sp_loading = autoplay;
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
                if self.sp_started.is_some() {
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
    }

    pub fn position(&self) -> Duration {
        match self.active {
            Active::Local => self.local.position(),
            Active::Spotify => {
                self.sp_base + self.sp_started.map(|s| s.elapsed()).unwrap_or_default()
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
            Active::Spotify => self.sp_started.is_some(),
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
