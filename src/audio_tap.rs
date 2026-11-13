//! Audio tap: a small shared window of the most recent samples that are being played.
//!
//! Both players (local files in `player.rs` and Spotify in `spotify/real.rs`) copy what they play
//! into this window. The play-bar visualizer and MilkDrop read from it, so they work
//! the same for local songs and Spotify. Nothing here changes the sound. Every sample in the tap
//! has the volume applied, and `sample_rate()` says how fast they were played.
#![allow(dead_code)] // the readers (EQ, visualizer) are added in the next steps

use rodio::Source;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Volume of the local player (0.0 to 1.0) as f32 bits. rodio applies the sink volume *after* the
/// tap, so the tap scales local samples itself and the visualizer looks the same at any volume
/// for local songs and for Spotify (whose samples are tapped after its own volume).
static LOCAL_VOLUME: AtomicU32 = AtomicU32::new(0x3f80_0000); // 1.0
/// Sample rate of the audio that was pushed last, in Hz (44100 until something plays).
static SAMPLE_RATE: AtomicU32 = AtomicU32::new(44_100);

/// Tells the tap the local player's volume (call whenever it changes).
pub fn set_local_volume(v: f32) {
    LOCAL_VOLUME.store(v.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
}

/// Sample rate in Hz of the audio now playing, for labelling FFT frequencies
/// (bin `i` of an `n`-point FFT is `i * sample_rate() / n` Hz).
pub fn sample_rate() -> u32 {
    SAMPLE_RATE.load(Ordering::Relaxed)
}

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
/// `rate` is their sample rate in Hz. The samples must already have the volume applied.
pub fn push_interleaved(samples: &[f32], channels: u16, rate: u32) {
    if rate > 0 {
        SAMPLE_RATE.store(rate, Ordering::Relaxed);
    }
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
    let rate = inner.sample_rate();
    TapSource { inner, channels, rate, chunk: Vec::with_capacity(CHUNK) }
}

pub struct TapSource<S: Source<Item = i16>> {
    inner: S,
    channels: u16,
    rate: u32,
    chunk: Vec<f32>,
}

impl<S: Source<Item = i16>> TapSource<S> {
    fn flush(&mut self) {
        if !self.chunk.is_empty() {
            let vol = f32::from_bits(LOCAL_VOLUME.load(Ordering::Relaxed));
            for v in self.chunk.iter_mut() {
                *v *= vol;
            }
            push_interleaved(&self.chunk, self.channels, self.rate);
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
