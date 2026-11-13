//! System tray icon (Windows). Left click shows / hides litewave; the right-click menu has
//! Show, Play / Pause, Next, Previous and Quit. With Settings > "Close button hides to tray" the
//! window's X keeps the music playing in the background instead of quitting.
//! On other systems this module is an empty stub.

use std::sync::atomic::AtomicBool;

/// Set right before litewave really quits, so the close button is not turned into "hide".
pub static QUITTING: AtomicBool = AtomicBool::new(false);

/// What the tray asked for.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Cmd {
    /// Left click: show the window, or hide it when it is already shown.
    ShowHide,
    Show,
    PlayPause,
    Next,
    Prev,
    Quit,
}

#[cfg(windows)]
pub struct Tray {
    icon: tray_icon::TrayIcon,
    rx: std::sync::mpsc::Receiver<Cmd>,
    tip: String,
}

#[cfg(windows)]
impl Tray {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Option<Self> {
        use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
        use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

        let img = image::load_from_memory(include_bytes!("../assets/icon.png")).ok()?.to_rgba8();
        let (w, h) = img.dimensions();
        let icon = Icon::from_rgba(img.into_raw(), w, h).ok()?;

        let menu = Menu::new();
        menu.append(&MenuItem::with_id("show", "Show litewave", true, None)).ok()?;
        menu.append(&PredefinedMenuItem::separator()).ok()?;
        menu.append(&MenuItem::with_id("toggle", "Play / Pause", true, None)).ok()?;
        menu.append(&MenuItem::with_id("next", "Next", true, None)).ok()?;
        menu.append(&MenuItem::with_id("prev", "Previous", true, None)).ok()?;
        menu.append(&PredefinedMenuItem::separator()).ok()?;
        menu.append(&MenuItem::with_id("quit", "Quit", true, None)).ok()?;

        let (tx, rx) = std::sync::mpsc::channel();
        let (tx2, ctx, ctx2) = (tx.clone(), cc.egui_ctx.clone(), cc.egui_ctx.clone());
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let cmd = match e.id.0.as_str() {
                "show" => Cmd::Show,
                "toggle" => Cmd::PlayPause,
                "next" => Cmd::Next,
                "prev" => Cmd::Prev,
                "quit" => Cmd::Quit,
                _ => return,
            };
            let _ = tx.send(cmd);
            ctx.request_repaint();
        }));
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = e
            {
                let _ = tx2.send(Cmd::ShowHide);
                ctx2.request_repaint();
            }
        }));

        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_tooltip("litewave")
            .with_icon(icon)
            .build()
            .ok()?;
        Some(Self { icon, rx, tip: "litewave".into() })
    }

    pub fn poll(&mut self) -> Vec<Cmd> {
        self.rx.try_iter().collect()
    }

    /// Hover text of the tray icon (the song that is playing).
    pub fn set_tooltip(&mut self, text: &str) {
        if self.tip != text {
            self.tip = text.to_string();
            let _ = self.icon.set_tooltip(Some(text));
        }
    }
}

#[cfg(not(windows))]
pub struct Tray;

#[cfg(not(windows))]
impl Tray {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Option<Self> {
        None
    }
    pub fn poll(&mut self) -> Vec<Cmd> {
        Vec::new()
    }
    pub fn set_tooltip(&mut self, _text: &str) {}
}
