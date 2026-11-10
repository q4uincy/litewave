//! Lyrics: a `.lrc` file next to the song, or the free LRCLIB service (cached on disk).
//!
//! Loading runs on a worker thread so the UI never waits for the network.

use crate::library::Track;
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::mpsc::{channel, Receiver},
    time::Duration,
};

const API: &str = "https://lrclib.net/api";

/// Show a line this many seconds early, so the highlight never feels late.
pub const LEAD: f32 = 0.25;

#[derive(Clone, Default, Serialize, Deserialize, Debug, PartialEq)]
pub struct Lyrics {
    /// (start time in seconds, text). For plain lyrics every time is 0.0.
    pub lines: Vec<(f32, String)>,
    /// True when the lines carry real timestamps.
    pub synced: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    /// Lyrics are switched off, or nothing is playing.
    Idle,
    Loading,
    Ready,
    /// Looked everywhere, found nothing.
    Missing,
    /// Could not reach the lyrics service (offline, timeout...).
    Failed,
}

pub struct LyricsResult {
    pub key: String,
    pub result: Result<Option<Lyrics>, String>,
}

impl Lyrics {
    /// Index of the line that is current at `secs` (synced lyrics only).
    pub fn current(&self, secs: f32) -> Option<usize> {
        if !self.synced {
            return None;
        }
        let n = self.lines.partition_point(|(t, _)| *t <= secs);
        if n == 0 {
            None
        } else {
            Some(n - 1)
        }
    }

    /// (previous, current, next) lines for a small screen. None for plain lyrics.
    pub fn window(&self, secs: f32) -> Option<[String; 3]> {
        if !self.synced || self.lines.is_empty() {
            return None;
        }
        let get = |i: isize| -> String {
            if i < 0 {
                return String::new();
            }
            self.lines
                .get(i as usize)
                .map(|(_, s)| s.clone())
                .unwrap_or_default()
        };
        let cur = self.current(secs).map(|i| i as isize).unwrap_or(-1);
        let mut mid = get(cur);
        if mid.trim().is_empty() {
            mid = "...".to_string();
        }
        Some([get(cur - 1), mid, get(cur + 1)])
    }
}

// ---- parsing -----------------------------------------------------------------

/// "mm:ss.xx" (or "mm:ss") -> seconds. Returns None for metadata tags like "ar:Name".
fn parse_stamp(tag: &str) -> Option<f32> {
    let (m, s) = tag.split_once(':')?;
    let m: u32 = m.trim().parse().ok()?;
    let s: f32 = s.trim().replace(',', ".").parse().ok()?;
    if !s.is_finite() || s < 0.0 {
        return None;
    }
    Some(m as f32 * 60.0 + s)
}

/// Removes enhanced-LRC word timing tags such as `<00:12.34>`.
fn strip_word_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '<' && chars.peek().map_or(false, |n| n.is_ascii_digit()) {
            for n in chars.by_ref() {
                if n == '>' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

pub fn plain_lyrics(text: &str) -> Lyrics {
    let mut lines: Vec<(f32, String)> = text
        .lines()
        .map(|l| (0.0, l.trim().to_string()))
        .collect();
    while lines.last().map_or(false, |(_, s)| s.is_empty()) {
        lines.pop();
    }
    let first = lines.iter().position(|(_, s)| !s.is_empty()).unwrap_or(lines.len());
    lines.drain(..first);
    Lyrics {
        lines,
        synced: false,
    }
}

/// Parses LRC text. Falls back to plain lyrics when there are no timestamps.
pub fn parse_lrc(text: &str) -> Lyrics {
    let mut lines: Vec<(f32, String)> = Vec::new();
    let mut plain: Vec<String> = Vec::new();
    let mut offset = 0.0_f32;

    for raw in text.lines() {
        let raw = raw.trim_start_matches('\u{feff}').trim();
        let mut rest = raw;
        let mut stamps: Vec<f32> = Vec::new();
        let mut had_tag = false;
        while rest.starts_with('[') {
            let Some(end) = rest.find(']') else { break };
            had_tag = true;
            let tag = &rest[1..end];
            if let Some(t) = parse_stamp(tag) {
                stamps.push(t);
            } else if let Some(v) = tag.strip_prefix("offset:") {
                offset = v.trim().parse::<f32>().unwrap_or(0.0) / 1000.0;
            }
            // Other tags ([ar:], [ti:], [al:], [by:]...) are ignored.
            rest = rest[end + 1..].trim_start();
        }
        if stamps.is_empty() {
            if !had_tag && !raw.is_empty() {
                plain.push(raw.to_string());
            }
            continue;
        }
        let body = strip_word_tags(rest).trim().to_string();
        for t in stamps {
            lines.push((t, body.clone()));
        }
    }

    if !lines.is_empty() {
        for (t, _) in lines.iter_mut() {
            *t = (*t - offset).max(0.0);
        }
        lines.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        return Lyrics {
            lines,
            synced: true,
        };
    }
    Lyrics {
        lines: plain.into_iter().map(|l| (0.0, l)).collect(),
        synced: false,
    }
}

// ---- local files -------------------------------------------------------------

fn local_lrc(path: &Path) -> Option<Lyrics> {
    if path.as_os_str().is_empty() {
        return None;
    }
    let p = path.with_extension("lrc");
    let bytes = std::fs::read(p).ok()?;
    let l = parse_lrc(&String::from_utf8_lossy(&bytes));
    if l.lines.is_empty() {
        None
    } else {
        Some(l)
    }
}

// ---- disk cache --------------------------------------------------------------

fn cache_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("litewave").join("lyrics"))
}

fn cache_key(t: &Track) -> String {
    let s = format!(
        "{}|{}|{}|{}",
        t.title.to_lowercase(),
        t.artist.to_lowercase(),
        t.album.to_lowercase(),
        t.duration.as_secs()
    );
    // FNV-1a, 64 bit: a short stable file name.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Some(Some(l)) = cached lyrics, Some(None) = cached "not found", None = not cached.
fn read_cache(key: &str) -> Option<Option<Lyrics>> {
    let p = cache_dir()?.join(format!("{key}.json"));
    let text = std::fs::read_to_string(&p).ok()?;
    let parsed: Option<Lyrics> = serde_json::from_str(&text).ok()?;
    if parsed.is_none() {
        // "Not found" is only trusted for a week: LRCLIB gains new lyrics all the time.
        let age = std::fs::metadata(&p)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|m| m.elapsed().ok())
            .unwrap_or_default();
        if age > Duration::from_secs(7 * 24 * 3600) {
            return None;
        }
    }
    Some(parsed)
}

fn write_cache(key: &str, value: &Option<Lyrics>) {
    let Some(dir) = cache_dir() else { return };
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(json) = serde_json::to_string(value) {
        let _ = std::fs::write(dir.join(format!("{key}.json")), json);
    }
}

// ---- LRCLIB ------------------------------------------------------------------

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(12))
        .user_agent(concat!(
            "litewave/",
            env!("CARGO_PKG_VERSION"),
            " (desktop music player)"
        ))
        .build()
}

fn from_response(v: &serde_json::Value) -> Option<Lyrics> {
    if v.get("instrumental").and_then(|x| x.as_bool()).unwrap_or(false) {
        return None;
    }
    if let Some(s) = v.get("syncedLyrics").and_then(|x| x.as_str()) {
        let l = parse_lrc(s);
        if l.synced {
            return Some(l);
        }
    }
    let p = v.get("plainLyrics").and_then(|x| x.as_str())?;
    let l = plain_lyrics(p);
    if l.lines.is_empty() {
        None
    } else {
        Some(l)
    }
}

fn describe(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, _) => format!("Lyrics service error ({code})"),
        other => format!("Could not reach the lyrics service: {other}"),
    }
}

fn fetch_lrclib(t: &Track) -> Result<Option<Lyrics>, String> {
    if t.title.trim().is_empty() {
        return Ok(None);
    }
    let ag = agent();
    let unknown_artist = t.artist.trim().is_empty() || t.artist == "Unknown artist";
    let secs = t.duration.as_secs();

    // 1. Exact match (needs the artist).
    if !unknown_artist {
        let mut req = ag
            .get(&format!("{API}/get"))
            .query("track_name", &t.title)
            .query("artist_name", &t.artist);
        if !t.album.trim().is_empty() {
            req = req.query("album_name", &t.album);
        }
        if secs > 0 {
            req = req.query("duration", &secs.to_string());
        }
        match req.call() {
            Ok(resp) => {
                let v: serde_json::Value = resp.into_json().map_err(|e| e.to_string())?;
                if let Some(l) = from_response(&v) {
                    return Ok(Some(l));
                }
            }
            Err(ureq::Error::Status(404, _)) => {}
            Err(e) => return Err(describe(e)),
        }
    }

    // 2. Search, then pick the best result that looks like the same recording.
    let q = if unknown_artist {
        t.title.clone()
    } else {
        format!("{} {}", t.title, t.artist)
    };
    let resp = ag
        .get(&format!("{API}/search"))
        .query("q", &q)
        .call()
        .map_err(describe)?;
    let items: Vec<serde_json::Value> = resp.into_json().map_err(|e| e.to_string())?;

    let want = secs as f64;
    let want_title = t.title.to_lowercase();
    let mut best: Option<(f64, Lyrics)> = None;
    for item in items.iter().take(10) {
        let Some(l) = from_response(item) else { continue };
        let name = item
            .get("trackName")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_lowercase();
        if !name.contains(&want_title) && !want_title.contains(&name) {
            continue;
        }
        let d = item.get("duration").and_then(|x| x.as_f64()).unwrap_or(0.0);
        let diff = if want > 0.0 && d > 0.0 { (d - want).abs() } else { 0.0 };
        if diff > 5.0 {
            continue; // a different version of the song, so the timing would be off
        }
        let score = diff + if l.synced { 0.0 } else { 100.0 };
        if best.as_ref().map_or(true, |(s, _)| score < *s) {
            best = Some((score, l));
        }
    }
    Ok(best.map(|(_, l)| l))
}

fn load(t: &Track) -> Result<Option<Lyrics>, String> {
    if let Some(l) = local_lrc(&t.path) {
        return Ok(Some(l));
    }
    let key = cache_key(t);
    if let Some(cached) = read_cache(&key) {
        return Ok(cached);
    }
    let found = fetch_lrclib(t)?;
    write_cache(&key, &found);
    Ok(found)
}

/// Looks up lyrics for `track` on a worker thread. `key` identifies the song so the app can
/// ignore a result that arrives after the user has already skipped to another track.
pub fn spawn_load(track: Track, key: String, ctx: egui::Context) -> Receiver<LyricsResult> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let result = load(&track);
        let _ = tx.send(LyricsResult { key, result });
        ctx.request_repaint();
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_synced_lines_and_ignores_metadata() {
        let l = parse_lrc("[ar:Someone]\n[ti:Song]\n[00:12.50]Hello\n[01:02.00]World\n");
        assert!(l.synced);
        assert_eq!(l.lines.len(), 2);
        assert_eq!(l.lines[0], (12.5, "Hello".to_string()));
        assert_eq!(l.lines[1], (62.0, "World".to_string()));
    }

    #[test]
    fn repeated_stamps_are_sorted() {
        let l = parse_lrc("[00:30.00][00:10.00]Chorus\n[00:20.00]Verse");
        let times: Vec<f32> = l.lines.iter().map(|(t, _)| *t).collect();
        assert_eq!(times, vec![10.0, 20.0, 30.0]);
        assert_eq!(l.lines[0].1, "Chorus");
    }

    #[test]
    fn offset_shifts_lines_earlier() {
        let l = parse_lrc("[offset:1000]\n[00:05.00]Hi");
        assert_eq!(l.lines[0].0, 4.0);
    }

    #[test]
    fn current_line_follows_position() {
        let l = parse_lrc("[00:05.00]a\n[00:10.00]b\n[00:15.00]c");
        assert_eq!(l.current(1.0), None);
        assert_eq!(l.current(5.0), Some(0));
        assert_eq!(l.current(12.0), Some(1));
        assert_eq!(l.current(99.0), Some(2));
    }

    #[test]
    fn window_has_neighbours() {
        let l = parse_lrc("[00:05.00]a\n[00:10.00]b\n[00:15.00]c");
        let w = l.window(11.0).unwrap();
        assert_eq!(w, ["a".to_string(), "b".to_string(), "c".to_string()]);
        let before = l.window(0.0).unwrap();
        assert_eq!(before[0], "");
        assert_eq!(before[1], "...");
        assert_eq!(before[2], "a");
    }

    #[test]
    fn text_without_stamps_is_plain() {
        let l = parse_lrc("just words\nmore words");
        assert!(!l.synced);
        assert_eq!(l.lines.len(), 2);
        assert_eq!(l.current(5.0), None);
        assert!(l.window(5.0).is_none());
    }

    #[test]
    fn strips_word_timing_tags() {
        let l = parse_lrc("[00:01.00]<00:01.00>Hel<00:01.40>lo");
        assert_eq!(l.lines[0].1, "Hello");
    }

    #[test]
    fn plain_lyrics_trims_blank_edges() {
        let l = plain_lyrics("\n\nline one\n\nline two\n\n");
        assert_eq!(l.lines.len(), 3);
        assert_eq!(l.lines[0].1, "line one");
    }
}
