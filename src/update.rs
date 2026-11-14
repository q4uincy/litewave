//! Updates from GitHub releases, the way Spotify does them: quiet and out of the way.
//!
//! On Windows litewave looks for a newer release when it starts (and again every few hours while
//! it runs). If there is one, the installer is downloaded in the background and nothing pops up.
//! A small "Update ready" button appears in the top bar; clicking it shows what is new and a
//! "Restart to update" button. If the update is put off a few times (3 starts, or a week), it is
//! installed by itself the next time litewave is closed, so people never get stuck on an old
//! version. "Update automatically" in Settings > Updates turns the background download and the
//! automatic install off; the button then asks before it downloads anything.
//!
//! "What's new": the first start after an update opens a "What's new" window with the notes of
//! the version that is now running. Settings > Updates has a button to read them again.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Mutex;
use std::time::Duration;

use eframe::egui;
use eframe::egui::{Color32, RichText};

const REPO: &str = "q4uincy/litewave";
const ASSET: &str = "litewave-setup.exe";
/// An update that was waiting on this many starts installs itself when litewave is closed ...
const AUTO_AFTER_STARTS: u32 = 3;
/// ... or when it has been waiting this long.
const AUTO_AFTER_SECS: u64 = 7 * 24 * 3600;
/// How often a running litewave looks for a new release.
const RECHECK_EVERY: Duration = Duration::from_secs(6 * 3600);

static CHECK_REQUEST: AtomicBool = AtomicBool::new(false);
static WHATS_NEW_REQUEST: AtomicBool = AtomicBool::new(false);
static INSTALL_REQUEST: AtomicBool = AtomicBool::new(false);
static POPUP_OPEN: AtomicBool = AtomicBool::new(false);
static STATUS: Mutex<String> = Mutex::new(String::new());
/// Text of the top bar button, or `None` when there is nothing to show.
static PILL: Mutex<Option<String>> = Mutex::new(None);
/// A downloaded installer that is waiting: (version, path).
static READY: Mutex<Option<(String, PathBuf)>> = Mutex::new(None);

/// Settings > Updates: ask for a check now (handled by `Root` on the next frame).
pub fn request_check() {
    set_status("Checking for updates...");
    CHECK_REQUEST.store(true, Ordering::SeqCst);
}

/// Settings > Updates: show the release notes of the version that is running now.
pub fn request_whats_new() {
    set_status("Loading release notes...");
    WHATS_NEW_REQUEST.store(true, Ordering::SeqCst);
}

/// Settings > Updates: restart into the downloaded update (or download it first).
pub fn request_install() {
    INSTALL_REQUEST.store(true, Ordering::SeqCst);
}

/// An update is downloaded and ready to install.
pub fn update_ready() -> bool {
    READY.lock().map_or(false, |g| g.is_some())
}

/// The text shown under the "Check for updates" button.
pub fn status() -> String {
    STATUS.lock().map(|s| s.clone()).unwrap_or_default()
}

fn set_status(s: &str) {
    if let Ok(mut g) = STATUS.lock() {
        *g = s.to_string();
    }
}

fn set_pill(label: Option<&str>) {
    if let Ok(mut g) = PILL.lock() {
        *g = label.map(String::from);
    }
}

fn pill_label() -> Option<String> {
    PILL.lock().ok().and_then(|g| g.clone())
}

fn set_ready(v: Option<(String, PathBuf)>) {
    if let Ok(mut g) = READY.lock() {
        *g = v;
    }
}

/// The small button in the top bar ("Update ready"). Draws nothing when there is no update.
/// Clicking it opens or closes the details popup that `Root` draws right below it.
pub fn pill(ui: &mut egui::Ui, accent: Color32) {
    let Some(label) = pill_label() else { return };
    let resp = ui.add(
        egui::Button::new(RichText::new(format!("   {label}")).size(13.0).strong().color(accent))
            .fill(accent.gamma_multiply(0.16))
            .stroke(egui::Stroke::new(1.0, accent.gamma_multiply(0.55)))
            .rounding(16.0)
            .min_size(egui::vec2(0.0, 30.0)),
    );
    // The dot in front of the text.
    ui.painter().circle_filled(resp.rect.left_center() + egui::vec2(15.0, 0.0), 3.5, accent);
    ui.data_mut(|d| {
        d.insert_temp(egui::Id::new("update_pill_rect"), resp.rect);
        d.insert_temp(egui::Id::new("update_pill_accent"), accent);
    });
    if resp.clicked() {
        POPUP_OPEN.fetch_xor(true, Ordering::SeqCst);
    }
    let _ = resp.on_hover_text("Click for details");
}

/// A release on GitHub that is newer than this build.
#[derive(Clone)]
struct Found {
    version: String,
    url: String,
    notes: String,
    /// Size of the installer in bytes (0 = unknown); used to check the download.
    size: u64,
}

struct Ready {
    version: String,
    notes: String,
    path: PathBuf,
}

enum Msg {
    /// Result of "Check for updates" in Settings.
    Manual(Result<Option<Found>, String>),
    /// The background check found a newer release.
    Found(Found),
    Downloaded { found: Found, result: Result<PathBuf, String> },
    /// Release notes of the running version. `manual` = asked for in Settings.
    WhatsNew { manual: bool, result: Result<(String, String), String> },
}

enum State {
    Idle,
    /// A newer release exists but "Update automatically" is off: nothing downloaded yet.
    Available(Found),
    /// `then_install`: the user already asked for the update, so restart as soon as it is here.
    Downloading { version: String, then_install: bool },
    Ready(Ready),
}

/// Wraps the real app and draws the update popup on top of it.
pub struct Root {
    app: crate::app::App,
    state: State,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    /// (version, notes) of the "What's new" window, if open.
    whats_new: Option<(String, String)>,
    /// Last problem to show inside the popup (download or installer failed).
    error: Option<String>,
    /// The version that was already counted as "seen but not installed" in this run.
    counted: Option<String>,
}

impl Root {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        crate::single::watch(&cc.egui_ctx);
        let (tx, rx) = channel();
        let current = env!("CARGO_PKG_VERSION");
        if cfg!(windows) {
            clean_old_installers(current);
            let tx = tx.clone();
            let ctx = cc.egui_ctx.clone();
            std::thread::spawn(move || {
                loop {
                    if crate::config::Config::load().auto_update_check {
                        if let Some(msg) = check() {
                            if tx.send(msg).is_err() {
                                break;
                            }
                            ctx.request_repaint();
                        }
                    }
                    std::thread::sleep(RECHECK_EVERY);
                }
            });
        }
        // First start after an update: show what changed in this version (once).
        let last = state_file("seen_version.txt")
            .and_then(|f| std::fs::read_to_string(f).ok())
            .unwrap_or_default();
        let last = last.trim();
        if last.is_empty() {
            write_state("seen_version.txt", current); // fresh install: nothing to compare with
        } else if is_newer(current, last) && crate::config::Config::load().auto_update_check {
            let tx = tx.clone();
            let ctx = cc.egui_ctx.clone();
            std::thread::spawn(move || {
                let result = fetch_notes(current);
                let _ = tx.send(Msg::WhatsNew { manual: false, result });
                ctx.request_repaint();
            });
        } else if is_newer(last, current) {
            write_state("seen_version.txt", current); // downgraded: keep in step
        }
        Self {
            app: crate::app::App::new(cc),
            state: State::Idle,
            tx,
            rx,
            whats_new: None,
            error: None,
            counted: None,
        }
    }

    /// Starts the background download of `found`.
    fn start_download(&mut self, ctx: &egui::Context, found: Found, then_install: bool) {
        set_status(&format!("Downloading litewave {} in the background...", found.version));
        self.error = None;
        self.state = State::Downloading { version: found.version.clone(), then_install };
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = download(&found);
            let _ = tx.send(Msg::Downloaded { found, result });
            ctx.request_repaint();
        });
    }

    /// A newer release was found (by the background check or by "Check for updates").
    fn on_found(&mut self, ctx: &egui::Context, found: Found) {
        if matches!(self.state, State::Idle) {
            if self.counted.as_deref() != Some(found.version.as_str()) {
                note_pending(&found.version);
                self.counted = Some(found.version.clone());
            }
            if crate::config::Config::load().auto_update_install {
                self.start_download(ctx, found, false);
            } else {
                set_status(&format!("litewave {} is available.", found.version));
                self.state = State::Available(found);
            }
            return;
        }
        let text = match &self.state {
            State::Ready(r) => format!("litewave {} is ready. Restart to update.", r.version),
            State::Downloading { version, .. } => {
                format!("Downloading litewave {version} in the background...")
            }
            State::Available(f) => format!("litewave {} is available.", f.version),
            State::Idle => return,
        };
        set_status(&text);
    }

    /// "Restart to update" / "Download and restart".
    fn install_clicked(&mut self, ctx: &egui::Context) {
        if matches!(self.state, State::Ready(_)) {
            self.install_now(ctx);
            return;
        }
        let found = if let State::Available(f) = &self.state { Some(f.clone()) } else { None };
        if let Some(f) = found {
            self.start_download(ctx, f, true);
        }
    }

    /// Starts the downloaded installer and closes litewave; the installer starts it again.
    fn install_now(&mut self, ctx: &egui::Context) {
        let (path, version) = match &self.state {
            State::Ready(r) => (r.path.clone(), r.version.clone()),
            _ => return,
        };
        match run_installer(&path, &version, true) {
            Ok(()) => {
                set_ready(None);
                POPUP_OPEN.store(false, Ordering::SeqCst);
                crate::tray::QUITTING.store(true, Ordering::SeqCst);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Err(e) => self.error = Some(e),
        }
    }

    fn show_updates(&mut self, ctx: &egui::Context) {
        if CHECK_REQUEST.swap(false, Ordering::SeqCst) {
            let tx = self.tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(Msg::Manual(fetch(true)));
                ctx.request_repaint();
            });
        }
        if WHATS_NEW_REQUEST.swap(false, Ordering::SeqCst) {
            let tx = self.tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let result = fetch_notes(env!("CARGO_PKG_VERSION"));
                let _ = tx.send(Msg::WhatsNew { manual: true, result });
                ctx.request_repaint();
            });
        }
        if INSTALL_REQUEST.swap(false, Ordering::SeqCst) {
            self.install_clicked(ctx);
        }
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::WhatsNew { manual, result } => match result {
                    Ok((version, notes)) => {
                        write_state("seen_version.txt", env!("CARGO_PKG_VERSION"));
                        if manual {
                            set_status("");
                        }
                        if !notes.trim().is_empty() {
                            self.whats_new = Some((version, notes));
                        } else if manual {
                            set_status("This release has no notes.");
                        }
                    }
                    // Offline etc.: stay silent on start (we try again next time), tell the user otherwise.
                    Err(e) => {
                        if manual {
                            set_status(&e);
                        }
                    }
                },
                Msg::Found(found) => self.on_found(ctx, found),
                Msg::Manual(Ok(Some(found))) => {
                    if cfg!(windows) {
                        self.on_found(ctx, found);
                    } else {
                        set_status(&format!("litewave {} is available. Download it from the releases page.", found.version));
                    }
                }
                Msg::Manual(Ok(None)) => {
                    if matches!(self.state, State::Idle) {
                        set_status("You're up to date.");
                    }
                }
                Msg::Manual(Err(e)) => set_status(&e),
                Msg::Downloaded { found, result } => {
                    let then_install =
                        matches!(self.state, State::Downloading { then_install: true, .. });
                    match result {
                        Ok(path) => {
                            set_ready(Some((found.version.clone(), path.clone())));
                            set_status(&format!("litewave {} is ready. Restart to update.", found.version));
                            self.state = State::Ready(Ready {
                                version: found.version,
                                notes: found.notes,
                                path,
                            });
                            if then_install {
                                self.install_now(ctx);
                            }
                        }
                        Err(e) => {
                            set_status(&e);
                            if then_install {
                                // The user asked for it: keep the button and say what went wrong.
                                self.error = Some(e);
                                self.state = State::Available(found);
                            } else {
                                // Quiet background failure: try again at the next check.
                                self.state = State::Idle;
                            }
                        }
                    }
                }
            }
        }
        self.show_whats_new(ctx);

        let label = match &self.state {
            State::Ready(_) => Some("Update ready"),
            State::Available(_) => Some("Update available"),
            State::Downloading { then_install: true, .. } => Some("Updating..."),
            _ => None,
        };
        set_pill(label);
        if label.is_none() {
            POPUP_OPEN.store(false, Ordering::SeqCst);
        }
        self.show_popup(ctx);
    }

    /// The details popup, right under the top bar button.
    fn show_popup(&mut self, ctx: &egui::Context) {
        if !POPUP_OPEN.load(Ordering::SeqCst) {
            return;
        }
        let (title, sub, notes, button) = match &self.state {
            State::Ready(r) => (
                format!("litewave {} is ready", r.version),
                "It was downloaded in the background. litewave closes and opens again, it only takes a moment.",
                r.notes.clone(),
                Some("Restart to update"),
            ),
            State::Available(f) => (
                format!("litewave {} is available", f.version),
                "Download it now; litewave then closes and opens again.",
                f.notes.clone(),
                Some("Download and restart"),
            ),
            State::Downloading { version, .. } => (
                format!("Updating to litewave {version}"),
                "litewave restarts as soon as the download is done.",
                String::new(),
                None,
            ),
            State::Idle => return,
        };
        let downloading = matches!(self.state, State::Downloading { .. });
        let pill_rect: Option<egui::Rect> = ctx.data(|d| d.get_temp(egui::Id::new("update_pill_rect")));
        let accent: Color32 = ctx
            .data(|d| d.get_temp(egui::Id::new("update_pill_accent")))
            .unwrap_or(Color32::from_rgb(30, 215, 96));
        let on_accent = {
            let lum = 0.299 * accent.r() as f32 + 0.587 * accent.g() as f32 + 0.114 * accent.b() as f32;
            if lum > 150.0 { Color32::BLACK } else { Color32::WHITE }
        };

        let mut area = egui::Area::new(egui::Id::new("litewave_update_popup")).order(egui::Order::Foreground);
        area = match pill_rect {
            Some(r) => area
                .pivot(egui::Align2::RIGHT_TOP)
                .fixed_pos(egui::pos2(r.right(), r.bottom() + 6.0)),
            None => area.anchor(egui::Align2::RIGHT_TOP, egui::vec2(-12.0, 58.0)),
        };
        let mut install = false;
        let mut close = false;
        let shown = area.show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_width(310.0);
                ui.label(RichText::new(&title).strong().size(15.0));
                ui.add_space(2.0);
                ui.label(RichText::new(sub).size(12.5).weak());
                if !notes.trim().is_empty() {
                    ui.add_space(6.0);
                    egui::CollapsingHeader::new("What's new").default_open(true).show(ui, |ui| {
                        egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                            show_notes(ui, &notes);
                        });
                    });
                }
                if let Some(e) = &self.error {
                    ui.add_space(6.0);
                    ui.colored_label(Color32::from_rgb(235, 100, 100), e.as_str());
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if downloading {
                        ui.spinner();
                        ui.label("Downloading...");
                    }
                    if let Some(b) = button {
                        let btn = egui::Button::new(RichText::new(b).strong().color(on_accent))
                            .fill(accent)
                            .rounding(14.0)
                            .min_size(egui::vec2(0.0, 30.0));
                        if ui.add(btn).clicked() {
                            install = true;
                        }
                    }
                    if ui
                        .add(egui::Button::new("Later").rounding(14.0).min_size(egui::vec2(0.0, 30.0)))
                        .clicked()
                    {
                        close = true;
                    }
                });
            });
        });
        // Click anywhere else (not on the button that opened it) or press Esc to close.
        let rect = shown.response.rect;
        if ctx.input(|i| i.pointer.any_pressed()) {
            if let Some(p) = ctx.input(|i| i.pointer.interact_pos()) {
                if !rect.contains(p) && !pill_rect.map_or(false, |r| r.contains(p)) {
                    close = true;
                }
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            close = true;
        }
        if install {
            self.install_clicked(ctx);
        }
        if close {
            POPUP_OPEN.store(false, Ordering::SeqCst);
        }
    }
}

impl Root {
    /// The "What's new in this version" window.
    fn show_whats_new(&mut self, ctx: &egui::Context) {
        let Some((version, notes)) = &self.whats_new else { return };
        let mut close = false;
        egui::Window::new(format!("What's new in litewave {version}"))
            .collapsible(false)
            .resizable(true)
            .default_width(380.0)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().max_height(340.0).show(ui, |ui| {
                    show_notes(ui, notes);
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Got it").clicked() {
                        close = true;
                    }
                    ui.hyperlink_to("All releases", format!("https://github.com/{REPO}/releases"));
                });
            });
        if close {
            self.whats_new = None;
        }
    }
}

impl eframe::App for Root {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Files opened with litewave (Open with, command line) or dropped onto the window.
        let mut files = crate::single::take_opened();
        files.extend(ctx.input(|i| {
            i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect::<Vec<_>>()
        }));
        if !files.is_empty() {
            self.app.open_paths(files);
        }
        crate::diag::frame();
        // One frame a second at least, so a window that stops drawing can be told from an idle one.
        ctx.request_repaint_after(Duration::from_secs(1));
        self.app.update(ctx, frame);
        self.show_updates(ctx);
    }
}

/// Called by `main` after the window has closed: if an update has been waiting for a few starts
/// (or a week), install it now, without opening litewave again. Does nothing otherwise.
pub fn install_on_exit() {
    if !cfg!(windows) {
        return;
    }
    let Some((version, path)) = READY.lock().ok().and_then(|g| g.clone()) else { return };
    if !crate::config::Config::load().auto_update_install || !auto_due(&version) {
        return;
    }
    let _ = run_installer(&path, &version, false);
}

/// Turns a GitHub release body into plain, readable lines: bullets instead of `*`, no
/// "by @user in <link>" tails, no bold markers, no "Full Changelog" link line.
fn clean_notes(body: &str) -> String {
    let mut out = Vec::new();
    for line in body.lines() {
        let t = line.trim();
        if t.starts_with("**Full Changelog**") || t.starts_with("Full Changelog") {
            continue;
        }
        let mut t = t.replace("**", "").replace('`', "");
        if let Some(rest) = t.strip_prefix("* ").or_else(|| t.strip_prefix("- ")) {
            let mut item = rest.to_string();
            if let Some(i) = item.find(" by @") {
                item.truncate(i);
            }
            t = format!("\u{2022} {item}");
        }
        out.push(t);
    }
    let joined = out.join("\n");
    joined.trim().chars().take(4000).collect()
}

/// Draws cleaned release notes: `#` lines are headings, everything else wraps as text.
fn show_notes(ui: &mut egui::Ui, notes: &str) {
    for line in notes.lines() {
        if line.trim().is_empty() {
            ui.add_space(4.0);
        } else if let Some(h) = line.trim_start().strip_prefix('#') {
            ui.label(egui::RichText::new(h.trim_start_matches('#').trim()).strong());
        } else {
            ui.label(line);
        }
    }
}

/// Release notes of one version (the tag `v<version>`): `(version, notes)`.
fn fetch_notes(version: &str) -> Result<(String, String), String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .user_agent(concat!("litewave/", env!("CARGO_PKG_VERSION")))
        .build();
    let release: serde_json::Value = agent
        .get(&format!("https://api.github.com/repos/{REPO}/releases/tags/v{version}"))
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("Could not load the release notes: {e}"))?
        .into_json()
        .map_err(|e| e.to_string())?;
    let notes = clean_notes(release["body"].as_str().unwrap_or(""));
    Ok((version.to_string(), notes))
}

/// "v0.6.10" -> [0, 6, 10]
fn parse_version(v: &str) -> Vec<u64> {
    v.trim()
        .trim_start_matches('v')
        .split('.')
        .map(|part| {
            part.chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse()
                .unwrap_or(0)
        })
        .collect()
}

fn is_newer(latest: &str, current: &str) -> bool {
    parse_version(latest) > parse_version(current)
}

fn state_file(name: &str) -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("litewave").join(name))
}

fn write_state(name: &str, text: &str) {
    if let Some(f) = state_file(name) {
        if let Some(dir) = f.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(f, text);
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// (starts, first seen) for the update that is waiting; (0, 0) if it is a different version.
fn read_pending(version: &str) -> (u32, u64) {
    let text = state_file("update_pending.txt")
        .and_then(|f| std::fs::read_to_string(f).ok())
        .unwrap_or_default();
    let mut lines = text.lines();
    if lines.next().unwrap_or("").trim() != version {
        return (0, 0);
    }
    let starts = lines.next().and_then(|x| x.trim().parse().ok()).unwrap_or(0);
    let first = lines.next().and_then(|x| x.trim().parse().ok()).unwrap_or(0);
    (starts, first)
}

/// Counts one more start in which this update was waiting and not installed.
fn note_pending(version: &str) {
    let (starts, first) = read_pending(version);
    let first = if starts == 0 { now_secs() } else { first };
    write_state("update_pending.txt", &format!("{version}\n{}\n{first}", starts + 1));
}

/// Has the update waited long enough to install itself?
fn auto_due(version: &str) -> bool {
    let (starts, first) = read_pending(version);
    starts >= AUTO_AFTER_STARTS
        || (starts > 0 && now_secs().saturating_sub(first) >= AUTO_AFTER_SECS)
}

fn update_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("litewave").join("update"))
}

fn installer_path(version: &str) -> Option<PathBuf> {
    update_dir().map(|d| d.join(format!("litewave-setup-{version}.exe")))
}

/// Deletes downloaded installers that are not newer than the running version, and half-finished
/// downloads.
fn clean_old_installers(current: &str) {
    let Some(dir) = update_dir() else { return };
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let keep = name
            .strip_prefix("litewave-setup-")
            .and_then(|n| n.strip_suffix(".exe"))
            .map_or(false, |v| is_newer(v, current));
        if !keep {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// The installer for this version was already started. If the app still reports an older version
/// (for example a build from source), do not ask again and again; only a newer release asks.
fn already_installed(version: &str) -> bool {
    state_file("installed_update.txt")
        .and_then(|f| std::fs::read_to_string(f).ok())
        .map_or(false, |t| t.trim() == version)
}

fn check() -> Option<Msg> {
    fetch(false).ok().flatten().map(Msg::Found)
}

/// Asks GitHub for the latest release. `Ok(None)` = nothing newer (or already installed, unless `manual`).
fn fetch(manual: bool) -> Result<Option<Found>, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .user_agent(concat!("litewave/", env!("CARGO_PKG_VERSION")))
        .build();
    let release: serde_json::Value = agent
        .get(&format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("Could not reach GitHub: {e}"))?
        .into_json()
        .map_err(|e| e.to_string())?;
    let tag = release["tag_name"].as_str().ok_or("No release found.")?;
    if !is_newer(tag, env!("CARGO_PKG_VERSION")) {
        return Ok(None);
    }
    let version = tag.trim_start_matches('v').to_string();
    if !manual && already_installed(&version) {
        return Ok(None);
    }
    let notes = clean_notes(release["body"].as_str().unwrap_or(""));
    let asset = release["assets"]
        .as_array()
        .and_then(|a| a.iter().find(|a| a["name"].as_str() == Some(ASSET)));
    let url = asset.and_then(|a| a["browser_download_url"].as_str()).map(String::from);
    let size = asset.and_then(|a| a["size"].as_u64()).unwrap_or(0);
    match url {
        Some(url) => Ok(Some(Found { version, url, notes, size })),
        None => Err(format!("litewave {version} is out, but its installer is not uploaded yet.")),
    }
}

/// Downloads the installer into litewave's update folder. An installer that was already
/// downloaded earlier (same size) is reused.
fn download(found: &Found) -> Result<PathBuf, String> {
    // Only ever run an installer that was downloaded from this project's own releases.
    let allowed = format!("https://github.com/{REPO}/releases/download/");
    if !found.url.starts_with(&allowed) {
        return Err("The update link is not from the litewave releases page.".into());
    }
    let dest = installer_path(&found.version).ok_or("There is no place to save the update.")?;
    if found.size > 0 && std::fs::metadata(&dest).map_or(false, |m| m.len() == found.size) {
        return Ok(dest);
    }
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("Could not save the update: {e}"))?;
    }
    let part = dest.with_extension("part");
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(60))
        .user_agent(concat!("litewave/", env!("CARGO_PKG_VERSION")))
        .build();
    let resp = agent.get(&found.url).call().map_err(|e| format!("Download failed: {e}"))?;
    let mut file = std::fs::File::create(&part).map_err(|e| format!("Could not save the update: {e}"))?;
    let mut reader = resp.into_reader();
    let mut buf = [0u8; 64 * 1024];
    let mut total: u64 = 0;
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("Download failed: {e}"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| format!("Could not save the update: {e}"))?;
        total += n as u64;
    }
    file.flush().map_err(|e| format!("Could not save the update: {e}"))?;
    drop(file);
    // A cut-off download must never be run.
    if (found.size > 0 && total != found.size) || total < 100_000 {
        let _ = std::fs::remove_file(&part);
        return Err("The download was incomplete. litewave will try again later.".into());
    }
    std::fs::rename(&part, &dest).map_err(|e| format!("Could not save the update: {e}"))?;
    Ok(dest)
}

/// Starts the installer. `relaunch`: the installer opens litewave again when it is done
/// (restart now); without it the update happens quietly after litewave was closed.
fn run_installer(path: &std::path::Path, version: &str, relaunch: bool) -> Result<(), String> {
    let mut cmd = std::process::Command::new(path);
    cmd.args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/CLOSEAPPLICATIONS", "/NORESTART"]);
    if !relaunch {
        cmd.arg("/NOLAUNCH=1");
    }
    cmd.spawn().map_err(|e| format!("Could not start the installer: {e}"))?;
    write_state("installed_update.txt", version);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(is_newer("v0.6.3", "0.6.0"));
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(!is_newer("v0.6.0", "0.6.0"));
        assert!(!is_newer("v0.5.9", "0.6.0"));
    }

    #[test]
    fn cleans_release_notes() {
        let body = "## What's Changed\n* Add lyrics by @q4uincy in https://github.com/x/y/pull/1\n\n**Full Changelog**: https://github.com/x/y/compare/a...b";
        assert_eq!(clean_notes(body), "## What's Changed\n\u{2022} Add lyrics");
    }
}
