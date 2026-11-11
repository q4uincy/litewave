//! Only one litewave window at a time. A second start passes its files (if any) to the first
//! window, brings it to the front (over a localhost port) and then exits.
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use eframe::egui;

const ADDR: &str = "127.0.0.1:47862";
static LISTENER: Mutex<Option<TcpListener>> = Mutex::new(None);
/// Files waiting to be played (from "Open with litewave" or the command line).
static OPEN: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Files that arrived since the last call.
pub fn take_opened() -> Vec<PathBuf> {
    OPEN.lock().map(|mut v| std::mem::take(&mut *v)).unwrap_or_default()
}

fn args_as_paths() -> Vec<PathBuf> {
    std::env::args_os().skip(1).map(PathBuf::from).collect()
}

/// Err(()) = another litewave is already running and was told what to do: the caller must exit.
pub fn start() -> Result<(), ()> {
    let files = args_as_paths();
    match TcpListener::bind(ADDR) {
        Ok(l) => {
            if let Ok(mut slot) = LISTENER.lock() {
                *slot = Some(l);
            }
            if let Ok(mut q) = OPEN.lock() {
                q.extend(files);
            }
            Ok(())
        }
        Err(_) => {
            if ask_first(&files) {
                Err(())
            } else {
                // The port is used by something else: just start normally.
                if let Ok(mut q) = OPEN.lock() {
                    q.extend(files);
                }
                Ok(())
            }
        }
    }
}

fn ask_first(files: &[PathBuf]) -> bool {
    let Ok(addr) = ADDR.parse::<SocketAddr>() else { return false };
    let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(400)) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_millis(1500)));
    let mut msg = String::from(if files.is_empty() { "litewave-show" } else { "litewave-open" });
    for f in files {
        msg.push('\n');
        msg.push_str(&f.to_string_lossy());
    }
    if s.write_all(msg.as_bytes()).is_err() {
        return false;
    }
    let _ = s.shutdown(Shutdown::Write);
    let mut buf = [0u8; 16];
    let n = s.read(&mut buf).unwrap_or(0);
    &buf[..n] == b"litewave-ok"
}

/// Called once the window exists: answers requests from later starts.
pub fn watch(ctx: &egui::Context) {
    let Some(listener) = LISTENER.lock().ok().and_then(|mut l| l.take()) else { return };
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let _ = s.set_read_timeout(Some(Duration::from_millis(1000)));
            let mut data = Vec::new();
            let _ = Read::by_ref(&mut s).take(64 * 1024).read_to_end(&mut data);
            let text = String::from_utf8_lossy(&data).into_owned();
            let mut lines = text.lines();
            let first = lines.next().unwrap_or("");
            if first == "litewave-show" || first == "litewave-open" {
                let _ = s.write_all(b"litewave-ok");
                if first == "litewave-open" {
                    if let Ok(mut q) = OPEN.lock() {
                        q.extend(lines.filter(|l| !l.is_empty()).map(PathBuf::from));
                    }
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                ctx.request_repaint();
            }
        }
    });
}
