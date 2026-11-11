//! Only one litewave window at a time. A second start asks the first one to come to the front
//! (over a localhost port) and then exits.
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Mutex;
use std::time::Duration;

use eframe::egui;

const ADDR: &str = "127.0.0.1:47862";
static LISTENER: Mutex<Option<TcpListener>> = Mutex::new(None);

/// Err(()) = another litewave is already running and was told to show itself: the caller must exit.
pub fn start() -> Result<(), ()> {
    match TcpListener::bind(ADDR) {
        Ok(l) => {
            if let Ok(mut slot) = LISTENER.lock() {
                *slot = Some(l);
            }
            Ok(())
        }
        Err(_) => {
            if ask_first_to_show() {
                Err(())
            } else {
                Ok(()) // the port is used by something else: just start normally
            }
        }
    }
}

fn ask_first_to_show() -> bool {
    let Ok(addr) = ADDR.parse::<SocketAddr>() else { return false };
    let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(400)) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_millis(800)));
    if s.write_all(b"litewave-show").is_err() {
        return false;
    }
    let mut buf = [0u8; 16];
    let n = s.read(&mut buf).unwrap_or(0);
    &buf[..n] == b"litewave-ok"
}

/// Called once the window exists: answers "show yourself" requests from later starts.
pub fn watch(ctx: &egui::Context) {
    let Some(listener) = LISTENER.lock().ok().and_then(|mut l| l.take()) else { return };
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
            let mut buf = [0u8; 32];
            let n = s.read(&mut buf).unwrap_or(0);
            if buf[..n].starts_with(b"litewave-show") {
                let _ = s.write_all(b"litewave-ok");
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                ctx.request_repaint();
            }
        }
    });
}
