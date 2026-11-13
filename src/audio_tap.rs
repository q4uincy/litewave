//! Audio tap: a small shared window of the most recent samples that are being played.
//!
//! Both players (local files in `player.rs` and Spotify in `spotify/real.rs`) copy what they play
//! into this window. The equalizer, the play-bar visualizer and MilkDrop read from it, so they work
//! the same for local songs and Spotify. Nothing here changes the sound.
#![allow(dead_code)] // the readers (EQ, visualizer) are added in the next steps

use rodio::Source;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// How many mono samples are kept (about 0.19 s at 44.1 kHz: enough for an FFT of 4096-8192).
const CAP: usize = 8192;
/// Samples collected by a local source before they are copied in one go (keeps locking cheap).
const CHUNK: usize = 1024;

struct Ring {
    buf: Vec<f32>,
    /// Next write position.
    pos: usize,
    last: Option<Instant>,
}

fn ring() -> &'static Mutex<Ring> {
    static RING: OnceLock<Mutex<Ring>> = OnceLock::new();
    RING.get_or_init(|| Mutex::new(Ring { buf: vec![0.0; CAP], pos: 0, last: None }))
}

/// Adds interleaved samples (any channel count) to the window, mixed down to mono.
pub fn push_interleaved(samples: &[f32], channels: u16) {
    let ch = channels.max(1) as usize;
    let Ok(mut r) = ring().lock() else { return };
    for frame in samples.chunks(ch) {
        let m = frame.iter().sum::<f32>() / frame.len() as f32;
        let p = r.pos;
        r.buf[p] = m;
        r.pos = (p + 1) % CAP;
    }
    r.last = Some(Instant::now());
}

/// The latest `n` mono samples, oldest first (`n` is capped at the window size).
pub fn latest(n: usize) -> Vec<f32> {
    let n = n.min(CAP);
    let Ok(r) = ring().lock() else { return vec![0.0; n] };
    (0..n).map(|i| r.buf[(r.pos + CAP - n + i) % CAP]).collect()
}

/// True while audio was received a moment ago (false when paused or stopped).
pub fn is_active() -> bool {
    ring()
        .lock()
        .ok()
        .and_then(|r| r.last)
        .map_or(false, |t| t.elapsed() < Duration::from_millis(250))
}

/// Wraps a rodio source so that everything it plays is also copied into the tap.
pub fn tap<S: Source<Item = i16>>(inner: S) -> TapSource<S> {
    let channels = inner.channels();
    TapSource { inner, channels, chunk: Vec::with_capacity(CHUNK) }
}

pub struct TapSource<S: Source<Item = i16>> {
    inner: S,
    channels: u16,
    chunk: Vec<f32>,
}

impl<S: Source<Item = i16>> TapSource<S> {
    fn flush(&mut self) {
        if !self.chunk.is_empty() {
            push_interleaved(&self.chunk, self.channels);
            self.chunk.clear();
        }
    }
}

impl<S: Source<Item = i16>> Iterator for TapSource<S> {
    type Item = i16;

    fn next(&mut self) -> Option<i16> {
        match self.inner.next() {
            Some(v) => {
                self.chunk.push(v as f32 / 32768.0);
                if self.chunk.len() >= CHUNK {
                    self.flush();
                }
                Some(v)
            }
            None => {
                self.flush();
                None
            }
        }
    }
}

impl<S: Source<Item = i16>> Source for TapSource<S> {
    fn current_frame_len(&self) -> Option<usize> {
        self.inner.current_frame_len()
    }
    fn channels(&self) -> u16 {
        self.inner.channels()
    }
    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }
    fn try_seek(&mut self, pos: Duration) -> Result<(), rodio::source::SeekError> {
        self.chunk.clear();
        self.inner.try_seek(pos)
    }
}
