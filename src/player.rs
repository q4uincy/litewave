use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};
use std::{
    fs::File,
    io::BufReader,
    path::Path,
    time::{Duration, Instant},
};

/// Thin wrapper around rodio. Position is tracked with our own clock so the UI
/// does not depend on sink internals.
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
