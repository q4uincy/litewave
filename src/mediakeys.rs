//! Media keys and the operating system's media controls (Windows overlay, Linux MPRIS).
//! Built only with `--features mediakeys`; otherwise this module is an empty stub.

use std::time::Duration;

/// What a media key asked for.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Cmd {
    Toggle,
    Play,
    Pause,
    Next,
    Prev,
}

#[cfg(feature = "mediakeys")]
pub struct MediaKeys {
    controls: souvlaki::MediaControls,
    rx: std::sync::mpsc::Receiver<Cmd>,
    shown: String,
    playing: Option<bool>,
}

#[cfg(feature = "mediakeys")]
impl MediaKeys {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Option<Self> {
        #[cfg(windows)]
        let hwnd = {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            match cc.window_handle().ok()?.as_raw() {
                RawWindowHandle::Win32(h) => Some(h.hwnd.get() as *mut std::ffi::c_void),
                _ => None,
            }
        };
        #[cfg(not(windows))]
        let hwnd = None;

        let config = souvlaki::PlatformConfig {
            dbus_name: "litewave",
            display_name: "litewave",
            hwnd,
        };
        let mut controls = souvlaki::MediaControls::new(config).ok()?;
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = cc.egui_ctx.clone();
        controls
            .attach(move |event: souvlaki::MediaControlEvent| {
                let cmd = match event {
                    souvlaki::MediaControlEvent::Play => Cmd::Play,
                    souvlaki::MediaControlEvent::Pause => Cmd::Pause,
                    souvlaki::MediaControlEvent::Toggle => Cmd::Toggle,
                    souvlaki::MediaControlEvent::Next => Cmd::Next,
                    souvlaki::MediaControlEvent::Previous => Cmd::Prev,
                    _ => return,
                };
                let _ = tx.send(cmd);
                ctx.request_repaint();
            })
            .ok()?;
        Some(Self { controls, rx, shown: String::new(), playing: None })
    }

    /// Keys pressed since the last call.
    pub fn poll(&mut self) -> Vec<Cmd> {
        self.rx.try_iter().collect()
    }

    /// Tells the system which song plays and whether it is paused. Cheap: only sends changes.
    pub fn update(&mut self, now: Option<(&str, &str, &str, Duration)>, playing: bool) {
        let key = match now {
            Some((t, a, _, _)) => format!("{t}\u{1}{a}"),
            None => String::new(),
        };
        if key != self.shown {
            self.shown = key;
            let meta = match now {
                Some((title, artist, album, dur)) => souvlaki::MediaMetadata {
                    title: Some(title),
                    artist: Some(artist),
                    album: Some(album),
                    cover_url: None,
                    duration: if dur.is_zero() { None } else { Some(dur) },
                },
                None => souvlaki::MediaMetadata::default(),
            };
            let _ = self.controls.set_metadata(meta);
        }
        let state = now.is_some() && playing;
        if self.playing != Some(state) {
            self.playing = Some(state);
            let pb = if now.is_none() {
                souvlaki::MediaPlayback::Stopped
            } else if playing {
                souvlaki::MediaPlayback::Playing { progress: None }
            } else {
                souvlaki::MediaPlayback::Paused { progress: None }
            };
            let _ = self.controls.set_playback(pb);
        }
    }
}

#[cfg(not(feature = "mediakeys"))]
pub struct MediaKeys;

#[cfg(not(feature = "mediakeys"))]
impl MediaKeys {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Option<Self> {
        None
    }
    pub fn poll(&mut self) -> Vec<Cmd> {
        Vec::new()
    }
    pub fn update(&mut self, _now: Option<(&str, &str, &str, Duration)>, _playing: bool) {}
}
