//! Spectrum visualizer: reads the latest samples from the audio tap, runs an FFT and turns the result
//! into 48 smoothed bars (low notes on the left, high on the right). Drawn behind the player bar.

use crate::audio_tap;
use eframe::egui::{self, pos2, Color32, Rect};
use std::cell::RefCell;
use std::f32::consts::PI;
use std::time::{Duration, Instant};

pub const BARS: usize = 48;
/// FFT size (about 46 ms of audio at 44.1 kHz).
const N: usize = 2048;

struct State {
    bars: [f32; BARS],
    last: Instant,
    /// Hann window (smooths the edges of the sample block so the FFT is cleaner).
    window: Vec<f32>,
}

thread_local! {
    // Only the UI thread draws, so no lock is needed.
    static STATE: RefCell<State> = RefCell::new(State {
        bars: [0.0; BARS],
        last: Instant::now(),
        window: (0..N).map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / (N as f32 - 1.0)).cos()).collect(),
    });
}

/// In-place radix-2 FFT (the length must be a power of two).
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0usize;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * PI / len as f32;
        let (wr, wi) = (ang.cos(), ang.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let a = start + k;
                let b = a + len / 2;
                let tr = re[b] * cr - im[b] * ci;
                let ti = re[b] * ci + im[b] * cr;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let ncr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = ncr;
            }
        }
        len <<= 1;
    }
}

/// Raw bar heights (0 to 1) for the sound playing right now.
fn spectrum(window: &[f32]) -> [f32; BARS] {
    let samples = audio_tap::latest(N);
    let mut re: Vec<f32> = samples.iter().zip(window).map(|(s, w)| s * w).collect();
    let mut im = vec![0.0f32; N];
    fft(&mut re, &mut im);
    let rate = audio_tap::sample_rate().max(8000) as f32;
    let hz_per_bin = rate / N as f32;
    let lo = 40.0f32;
    let hi = (rate * 0.45).min(16_000.0);
    let mut out = [0.0f32; BARS];
    for b in 0..BARS {
        let f0 = lo * (hi / lo).powf(b as f32 / BARS as f32);
        let f1 = lo * (hi / lo).powf((b + 1) as f32 / BARS as f32);
        let i0 = ((f0 / hz_per_bin).floor() as usize).clamp(1, N / 2 - 2);
        let i1 = ((f1 / hz_per_bin).ceil() as usize).max(i0 + 1).min(N / 2);
        let mut peak = 0.0f32;
        for i in i0..i1 {
            peak = peak.max((re[i] * re[i] + im[i] * im[i]).sqrt());
        }
        // A full-scale sine gives N/4 with this window. Show -60 dB to -6 dB, and lift the highs a
        // little because music has less energy up there.
        let amp = peak / (N as f32 / 4.0);
        let db = 20.0 * amp.max(1e-6).log10() + 10.0 * b as f32 / BARS as f32;
        out[b] = ((db + 60.0) / 54.0).clamp(0.0, 1.0);
    }
    out
}

/// Smoothed bar heights: they jump up quickly and fall slowly.
fn bars() -> [f32; BARS] {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let now = Instant::now();
        let dt = now.duration_since(s.last).as_secs_f32().min(0.1);
        s.last = now;
        let target = if audio_tap::is_active() { spectrum(&s.window) } else { [0.0; BARS] };
        for i in 0..BARS {
            let (t, b) = (target[i], s.bars[i]);
            s.bars[i] = if t > b { b + (t - b) * 0.65 } else { (b - dt * 1.5).max(t) };
        }
        s.bars
    })
}

/// Draws the bars rising from the bottom edge of `rect`. Also asks for repaints while they move.
pub fn paint(p: &egui::Painter, rect: Rect, color: Color32) {
    let bars = bars();
    if bars.iter().any(|v| *v > 0.01) {
        p.ctx().request_repaint_after(Duration::from_millis(33));
    }
    let slot = rect.width() / BARS as f32;
    let gap = (slot * 0.25).min(3.0);
    for (i, v) in bars.iter().enumerate() {
        let h = v * rect.height();
        if h < 1.0 {
            continue;
        }
        let x0 = rect.min.x + i as f32 * slot + gap * 0.5;
        let r = Rect::from_min_max(pos2(x0, rect.max.y - h), pos2(x0 + slot - gap, rect.max.y));
        p.rect_filled(r, 1.5, color.gamma_multiply(0.30 + 0.25 * v));
    }
}
