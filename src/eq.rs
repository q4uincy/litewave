//! 10-band equalizer. The settings are shared (the UI writes them, the players read them), and each
//! audio stream owns an `Eq` that holds its filter memory. Local songs go through `eq::apply(source)`
//! in `player.rs`; Spotify audio goes through an `Eq` inside its sink in `spotify/real.rs`.
//!
//! Each band is a peaking filter (a "bell") of about one octave. To avoid clipping when bands are
//! boosted, the volume is lowered automatically by the biggest boost (a simple auto preamp).
#![allow(dead_code)] // the settings UI is added in the next step

use rodio::Source;
use std::f32::consts::PI;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::time::Duration;

pub const BANDS: usize = 10;
/// Centre frequency of each band in Hz.
pub const FREQS: [f32; BANDS] = [31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];
/// Largest boost or cut in dB.
pub const MAX_DB: f32 = 12.0;
/// Width of each bell (1.41 is about one octave).
const Q: f32 = 1.41;

static ENABLED: AtomicBool = AtomicBool::new(false);
/// Gain of each band in tenths of a dB.
static GAINS: [AtomicI32; BANDS] = [const { AtomicI32::new(0) }; BANDS];
/// Goes up on every change, so the audio threads know to recompute their filters.
static VERSION: AtomicU32 = AtomicU32::new(1);

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
    VERSION.fetch_add(1, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn set_band(i: usize, db: f32) {
    if i < BANDS {
        GAINS[i].store((db.clamp(-MAX_DB, MAX_DB) * 10.0).round() as i32, Ordering::Relaxed);
        VERSION.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn band(i: usize) -> f32 {
    GAINS.get(i).map_or(0.0, |g| g.load(Ordering::Relaxed) as f32 / 10.0)
}

pub fn set_all(db: &[f32; BANDS]) {
    for (i, v) in db.iter().enumerate() {
        GAINS[i].store((v.clamp(-MAX_DB, MAX_DB) * 10.0).round() as i32, Ordering::Relaxed);
    }
    VERSION.fetch_add(1, Ordering::Relaxed);
}

pub fn all() -> [f32; BANDS] {
    std::array::from_fn(band)
}

/// Built-in presets (name, gains in dB for the 10 bands).
pub const PRESETS: [(&str, [f32; BANDS]); 6] = [
    ("Flat", [0.0; BANDS]),
    ("Bass boost", [6.0, 5.0, 4.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("Treble boost", [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 4.0, 5.0, 6.0]),
    ("Vocal", [-2.0, -1.0, 0.0, 2.0, 4.0, 4.0, 3.0, 1.0, 0.0, -1.0]),
    ("Rock", [5.0, 4.0, 2.0, -1.0, -2.0, -1.0, 2.0, 4.0, 5.0, 5.0]),
    ("Electronic", [5.0, 4.0, 1.0, 0.0, -2.0, 2.0, 1.0, 3.0, 5.0, 4.0]),
];

#[derive(Clone, Copy, Default)]
struct Coef {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

/// Peaking EQ filter (RBJ audio-EQ-cookbook formulas).
fn peaking(f0: f32, db: f32, rate: f32) -> Coef {
    let a = 10f32.powf(db / 40.0);
    let w = 2.0 * PI * f0 / rate;
    let alpha = w.sin() / (2.0 * Q);
    let cw = w.cos();
    let a0 = 1.0 + alpha / a;
    Coef {
        b0: (1.0 + alpha * a) / a0,
        b1: (-2.0 * cw) / a0,
        b2: (1.0 - alpha * a) / a0,
        a1: (-2.0 * cw) / a0,
        a2: (1.0 - alpha / a) / a0,
    }
}

/// One stream's equalizer: the current filters and their memory.
pub struct Eq {
    version: u32,
    rate: u32,
    active: bool,
    preamp: f32,
    coefs: [Coef; BANDS],
    on: [bool; BANDS],
    /// Filter memory, per channel and band (two values each).
    z: Vec<[[f32; 2]; BANDS]>,
}

impl Eq {
    pub fn new() -> Self {
        Eq {
            version: 0, // never equal to VERSION, so the first refresh computes everything
            rate: 0,
            active: false,
            preamp: 1.0,
            coefs: [Coef::default(); BANDS],
            on: [false; BANDS],
            z: Vec::new(),
        }
    }

    /// True when the equalizer changes the sound right now (on, and at least one band is not flat).
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Clears the filter memory (after a seek or a new song) so old sound does not leak in.
    pub fn reset(&mut self) {
        for c in self.z.iter_mut() {
            *c = [[0.0; 2]; BANDS];
        }
    }

    /// Call every now and then (every ~1000 samples is plenty). Picks up setting changes,
    /// a new sample rate, or a new channel count.
    pub fn refresh(&mut self, rate: u32, channels: usize) {
        let ch = channels.max(1);
        if self.z.len() != ch {
            self.z = vec![[[0.0; 2]; BANDS]; ch];
        }
        let v = VERSION.load(Ordering::Relaxed);
        if v == self.version && rate == self.rate {
            return;
        }
        self.version = v;
        self.rate = rate;
        let was_active = self.active;
        let gains = all();
        let mut any = false;
        let mut max_boost = 0.0f32;
        for i in 0..BANDS {
            // Skip flat bands, and bands too close to the top of the spectrum for this sample rate.
            let usable = gains[i].abs() >= 0.05 && FREQS[i] < rate as f32 * 0.45;
            self.on[i] = usable;
            if usable {
                any = true;
                max_boost = max_boost.max(gains[i]);
                self.coefs[i] = peaking(FREQS[i], gains[i], rate as f32);
            }
        }
        self.active = enabled() && any && rate > 0;
        self.preamp = 10f32.powf(-max_boost / 20.0);
        if self.active && !was_active {
            self.reset();
        }
    }

    /// Filters one sample of channel `ch`. Does nothing while the equalizer is not active.
    #[inline]
    pub fn sample(&mut self, x: f32, ch: usize) -> f32 {
        if !self.active {
            return x;
        }
        let Some(mem) = self.z.get_mut(ch) else { return x };
        let mut y = x * self.preamp;
        for i in 0..BANDS {
            if !self.on[i] {
                continue;
            }
            let c = self.coefs[i];
            let z = &mut mem[i];
            let out = c.b0 * y + z[0];
            z[0] = c.b1 * y - c.a1 * out + z[1];
            z[1] = c.b2 * y - c.a2 * out;
            y = out;
        }
        y
    }
}

/// Wraps a rodio source (16-bit) so that the equalizer is applied to what it plays.
pub fn apply<S: Source<Item = i16>>(inner: S) -> EqSource<S> {
    EqSource { inner, eq: Eq::new(), ch: 0, count: 0 }
}

pub struct EqSource<S: Source<Item = i16>> {
    inner: S,
    eq: Eq,
    /// Channel of the next sample.
    ch: usize,
    count: u32,
}

impl<S: Source<Item = i16>> Iterator for EqSource<S> {
    type Item = i16;

    fn next(&mut self) -> Option<i16> {
        let v = self.inner.next()?;
        if self.count % 1024 == 0 {
            self.eq.refresh(self.inner.sample_rate(), self.inner.channels() as usize);
        }
        self.count = self.count.wrapping_add(1);
        let nch = self.inner.channels().max(1) as usize;
        let ch = self.ch;
        self.ch = (ch + 1) % nch;
        if !self.eq.is_active() {
            return Some(v);
        }
        let y = self.eq.sample(v as f32 / 32768.0, ch);
        Some((y * 32768.0).round().clamp(-32768.0, 32767.0) as i16)
    }
}

impl<S: Source<Item = i16>> Source for EqSource<S> {
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
        self.eq.reset();
        self.ch = 0;
        self.inner.try_seek(pos)
    }
}
