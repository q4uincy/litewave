//! A plain log file for finding freezes: `%APPDATA%\litewave\litewave.log`.
//!
//! `note()` writes one line (seconds since start + the text). A small watchdog thread checks once a
//! second that the window is still drawing frames; if it is not for 5 seconds, it writes which
//! step was the last one noted, so a "Not Responding" window leaves a trail. Never pass tokens.

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

static START: OnceLock<Instant> = OnceLock::new();
static LAST_FRAME_MS: AtomicU64 = AtomicU64::new(0);
static LAST_NOTE: Mutex<String> = Mutex::new(String::new());
static FILE_LOCK: Mutex<()> = Mutex::new(());

fn elapsed_ms() -> u64 {
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

pub fn path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("litewave").join("litewave.log"))
}

fn write_line(line: &str) {
    let _guard = FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Some(p) = path() else { return };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut f = match std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        Ok(f) => f,
        Err(_) => return,
    };
    let _ = writeln!(f, "{line}");
}

/// One progress line.
pub fn note(msg: &str) {
    let ms = elapsed_ms();
    if let Ok(mut g) = LAST_NOTE.lock() {
        *g = msg.to_string();
    }
    write_line(&format!("[{:>7.2}s] {msg}", ms as f64 / 1000.0));
}

/// Called once per drawn frame.
pub fn frame() {
    LAST_FRAME_MS.store(elapsed_ms().max(1), Ordering::Relaxed);
}

/// Starts the log (keeps it small) and the freeze watchdog.
pub fn start() {
    let _ = START.get_or_init(Instant::now);
    if let Some(p) = path() {
        if std::fs::metadata(&p).map_or(false, |m| m.len() > 200_000) {
            let _ = std::fs::remove_file(&p);
        }
    }
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    write_line(&format!("---- litewave {} started (unix time {secs}) ----", env!("CARGO_PKG_VERSION")));
    std::thread::spawn(|| {
        let mut frozen = false;
        loop {
            std::thread::sleep(Duration::from_secs(1));
            let last = LAST_FRAME_MS.load(Ordering::Relaxed);
            if last == 0 {
                continue; // no frame drawn yet
            }
            let gap = elapsed_ms().saturating_sub(last);
            if gap >= 5000 && !frozen {
                frozen = true;
                let step = LAST_NOTE.lock().map(|g| g.clone()).unwrap_or_default();
                write_line(&format!(
                    "[{:>7.2}s] WINDOW STOPPED DRAWING for {}s (minimized windows also do this). Last step: {step}",
                    elapsed_ms() as f64 / 1000.0,
                    gap / 1000
                ));
            } else if gap < 2000 && frozen {
                frozen = false;
                write_line(&format!("[{:>7.2}s] window is drawing again", elapsed_ms() as f64 / 1000.0));
            }
        }
    });
}
