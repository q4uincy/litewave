//! Optional YouTube / SoundCloud / Bandcamp source through the external `yt-dlp` program.
//!
//! litewave never bundles yt-dlp. It looks for the program (a path from the settings, or the
//! PATH) and, when it is missing, the feature simply stays off. Songs are downloaded to a small
//! cache folder as m4a and then played by the normal local player, so seeking works.

use crate::library::Track;
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Mutex,
    time::SystemTime,
};

/// The yt-dlp that was found last time, so the (slow) `--version` check runs once, not per song.
static FOUND: Mutex<Option<(String, PathBuf)>> = Mutex::new(None);
/// The browser whose cookies worked last time, tried first so a song does not wait on failures.
static GOOD_BROWSER: Mutex<Option<String>> = Mutex::new(None);

/// How many finished downloads are kept in the cache folder.
const CACHE_KEEP: usize = 30;

/// Builds a `Command` that does not flash a console window on Windows.
fn command(bin: &Path) -> Command {
    let mut c = Command::new(bin);
    c.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c
}

/// Finds a working yt-dlp: the configured path first, then `yt-dlp` on the PATH.
pub fn find(configured: &str) -> Option<PathBuf> {
    if let Ok(g) = FOUND.lock() {
        if let Some((key, path)) = g.as_ref() {
            if key == configured {
                return Some(path.clone());
            }
        }
    }
    let found = find_uncached(configured)?;
    if let Ok(mut g) = FOUND.lock() {
        *g = Some((configured.to_string(), found.clone()));
    }
    Some(found)
}

fn find_uncached(configured: &str) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if !configured.trim().is_empty() {
        candidates.push(PathBuf::from(configured.trim()));
    }
    candidates.push(PathBuf::from("yt-dlp"));
    if cfg!(windows) {
        candidates.push(PathBuf::from("yt-dlp.exe"));
    }
    candidates.into_iter().find(|bin| {
        command(bin)
            .arg("--version")
            .output()
            .map_or(false, |o| o.status.success())
    })
}

/// Where downloaded audio is kept.
pub fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("litewave")
        .join("yt")
}

/// YouTube video ids are letters, digits, `-` and `_`. Anything else is refused, so an id can
/// never smuggle in a path or extra text (it is also always passed after `--` inside a URL).
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Turns the JSON that `yt-dlp --flat-playlist -J` prints into tracks.
pub fn parse_results(json: &str) -> Result<Vec<Track>, String> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("Unexpected yt-dlp output: {e}"))?;
    let entries = v
        .get("entries")
        .and_then(|e| e.as_array())
        .ok_or_else(|| "yt-dlp returned no results".to_string())?;
    let mut out = Vec::new();
    for e in entries {
        let Some(id) = e.get("id").and_then(|x| x.as_str()) else { continue };
        if !valid_id(id) {
            continue;
        }
        let title = e.get("title").and_then(|x| x.as_str()).unwrap_or("Untitled");
        let artist = e
            .get("channel")
            .and_then(|x| x.as_str())
            .or_else(|| e.get("uploader").and_then(|x| x.as_str()))
            .unwrap_or("YouTube");
        let secs = e.get("duration").and_then(|x| x.as_f64()).unwrap_or(0.0);
        out.push(Track::youtube(id, title, artist, (secs * 1000.0) as u64));
    }
    Ok(out)
}

/// Lets yt-dlp use the YouTube login of a browser (fixes "Sign in to confirm you're not a bot").
fn cookie_args(c: &mut Command, browser: &str) {
    let b = browser.trim().to_lowercase();
    if matches!(
        b.as_str(),
        "chrome" | "edge" | "firefox" | "brave" | "opera" | "vivaldi" | "chromium"
    ) {
        c.arg("--cookies-from-browser").arg(b);
    }
}

/// Browsers to try. With none chosen, plain yt-dlp goes first and the installed browsers
/// are used one after another only when YouTube asks for a sign-in.
fn browsers_to_try(browser: &str) -> Vec<String> {
    let b = browser.trim().to_lowercase();
    if b.is_empty() {
        let mut v: Vec<String> =
            ["", "firefox", "edge", "chrome", "brave"].iter().map(|x| x.to_string()).collect();
        // Start with whatever worked last time.
        if let Some(good) = GOOD_BROWSER.lock().ok().and_then(|g| g.clone()) {
            v.retain(|x| *x != good);
            v.insert(0, good);
        }
        v
    } else {
        vec![b]
    }
}

fn remember_browser(b: &str) {
    if let Ok(mut g) = GOOD_BROWSER.lock() {
        *g = Some(b.to_string());
    }
}

/// True when the failure is about the YouTube sign-in or a browser's cookies (worth trying another browser).
fn sign_in_problem(msg: &str) -> bool {
    let m = msg.to_lowercase();
    m.contains("not a bot") || m.contains("sign in to confirm") || m.contains("cookie")
}

/// Turns yt-dlp's sign-in complaint into something a person can act on.
fn friendly(msg: String) -> String {
    if sign_in_problem(&msg) {
        format!("{} Details: {}", "YouTube wants a sign-in. litewave has no YouTube login of its own, it borrows one from a browser. Sign in to youtube.com in your browser with a spare Google account (not your main one), close that browser completely (end all its background processes), pick it under \"Sign-in browser\" on the YouTube page, then search again. Firefox is the most reliable; Chrome, Edge and Brave can fail on Windows.", msg.trim())
    } else {
        msg
    }
}

/// Searches YouTube. Blocking: call it from a thread.
pub fn search(bin: &Path, query: &str, count: usize, browser: &str) -> Result<Vec<Track>, String> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let mut last = "yt-dlp search failed".to_string();
    for b in browsers_to_try(browser) {
        let mut c = command(bin);
        c.args(["--flat-playlist", "-J", "--no-warnings"]);
        cookie_args(&mut c, &b);
        let out = c
            .arg("--")
            .arg(format!("ytsearch{}:{q}", count.clamp(1, 25)))
            .output()
            .map_err(|e| format!("Could not run yt-dlp: {e}"))?;
        if out.status.success() {
            remember_browser(&b);
            return parse_results(&String::from_utf8_lossy(&out.stdout));
        }
        last = last_line(&out.stderr, "yt-dlp search failed");
        if !sign_in_problem(&last) {
            break;
        }
    }
    Err(friendly(last))
}

/// Downloads the audio of one video into the cache and returns the file. Blocking.
/// Asks for m4a (format 140) because the built-in decoder cannot read Opus/WebM.
pub fn fetch_audio(bin: &Path, id: &str, browser: &str) -> Result<PathBuf, String> {
    if !valid_id(id) {
        return Err("Invalid video id".to_string());
    }
    let dir = cache_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create cache folder: {e}"))?;
    if let Some(hit) = cached(&dir, id) {
        return Ok(hit);
    }
    let mut last = "yt-dlp could not download this video".to_string();
    let mut done = false;
    for b in browsers_to_try(browser) {
        let mut c = command(bin);
        c.args([
            "-f",
            "140/bestaudio[ext=m4a]/bestaudio",
            "--no-playlist",
            "--no-warnings",
            "--no-part",
            "--no-mtime",
            "--concurrent-fragments",
            "4",
            "--socket-timeout",
            "15",
        ]);
        cookie_args(&mut c, &b);
        let out = c
            .arg("-o")
            .arg(dir.join(format!("{id}.%(ext)s")))
            .arg("--")
            .arg(format!("https://www.youtube.com/watch?v={id}"))
            .output()
            .map_err(|e| format!("Could not run yt-dlp: {e}"))?;
        if out.status.success() {
            remember_browser(&b);
            done = true;
            break;
        }
        last = last_line(&out.stderr, "yt-dlp could not download this video");
        if !sign_in_problem(&last) {
            break;
        }
    }
    if !done {
        return Err(friendly(last));
    }
    let path = cached(&dir, id).ok_or_else(|| "yt-dlp finished but left no file".to_string())?;
    prune(&dir, CACHE_KEEP, &path);
    Ok(path)
}

/// An already downloaded file for `id`, if there is one.
pub fn cached(dir: &Path, id: &str) -> Option<PathBuf> {
    let prefix = format!("{id}.");
    std::fs::read_dir(dir).ok()?.filter_map(Result::ok).find_map(|e| {
        let name = e.file_name().to_string_lossy().into_owned();
        let ok = name.starts_with(&prefix)
            && !name.ends_with(".part")
            && !name.ends_with(".ytdl")
            && e.metadata().map_or(false, |m| m.len() > 0);
        ok.then(|| e.path())
    })
}

/// Deletes the oldest files so at most `keep` remain. `protect` is never deleted.
fn prune(dir: &Path, keep: usize, protect: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<(SystemTime, PathBuf)> = rd
        .filter_map(Result::ok)
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            m.is_file().then(|| (m.modified().unwrap_or(SystemTime::UNIX_EPOCH), e.path()))
        })
        .collect();
    if files.len() <= keep {
        return;
    }
    files.sort_by_key(|(t, _)| *t);
    let excess = files.len() - keep;
    for (_, p) in files.into_iter().take(excess) {
        if p != protect {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// The last non-empty line of yt-dlp's error output, which is where it puts the reason.
fn last_line(stderr: &[u8], fallback: &str) -> String {
    String::from_utf8_lossy(stderr)
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map_or_else(|| fallback.to_string(), |l| l.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flat_playlist_json() {
        let json = r#"{"entries":[
            {"id":"dQw4w9WgXcQ","title":"Song","channel":"Band","duration":212.0},
            {"id":"bad id!","title":"x"},
            {"id":"abc_DEF-123","title":"Other","uploader":"Someone"}
        ]}"#;
        let t = parse_results(json).unwrap();
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].yt_id.as_deref(), Some("dQw4w9WgXcQ"));
        assert_eq!(t[0].duration.as_secs(), 212);
        assert_eq!(t[1].artist, "Someone");
    }

    #[test]
    fn rejects_odd_ids() {
        assert!(!valid_id("../x"));
        assert!(!valid_id("a b"));
        assert!(!valid_id(""));
        assert!(valid_id("abc_DEF-123"));
    }

    #[test]
    fn missing_entries_is_an_error() {
        assert!(parse_results("{}").is_err());
    }
}
