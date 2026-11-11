//! One-click updates from GitHub releases.
//!
//! On start (Windows only) litewave asks GitHub for the latest release. If it is newer than this
//! build and has a `litewave-setup.exe`, a small "Update now" box appears. Clicking it downloads
//! the installer, starts it silently and closes litewave; the installer replaces the files and
//! starts litewave again.

use std::io::Read;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;

use eframe::egui;

const REPO: &str = "q4uincy/litewave";
const ASSET: &str = "litewave-setup.exe";

enum Msg {
    Available { version: String, url: String, notes: String },
    Downloaded(Result<(), String>),
}

enum State {
    Idle,
    Available { version: String, url: String, notes: String },
    Downloading,
    Failed(String),
    Dismissed,
}

/// Wraps the real app and draws the update box on top of it.
pub struct Root {
    app: crate::app::App,
    state: State,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
}

impl Root {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        crate::single::watch(&cc.egui_ctx);
        let (tx, rx) = channel();
        let mut state = State::Dismissed;
        if cfg!(windows) {
            state = State::Idle;
            let tx = tx.clone();
            let ctx = cc.egui_ctx.clone();
            std::thread::spawn(move || {
                if let Some(msg) = check() {
                    let _ = tx.send(msg);
                    ctx.request_repaint();
                }
            });
        }
        Self { app: crate::app::App::new(cc), state, tx, rx }
    }

    fn show_update_box(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Available { version, url, notes } => {
                    if matches!(self.state, State::Idle) {
                        self.state = State::Available { version, url, notes };
                    }
                }
                Msg::Downloaded(Ok(())) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                Msg::Downloaded(Err(e)) => self.state = State::Failed(e),
            }
        }
        if matches!(self.state, State::Idle | State::Dismissed) {
            return;
        }

        let mut start: Option<String> = None;
        let mut dismiss = false;
        let mut skip: Option<String> = None;
        egui::Area::new(egui::Id::new("litewave_update_box"))
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-14.0, -14.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_max_width(300.0);
                    match &self.state {
                        State::Available { version, url, notes } => {
                            ui.label(format!("litewave {version} is available"));
                            if !notes.trim().is_empty() {
                                ui.collapsing("What's new", |ui| {
                                    ui.label(notes.as_str());
                                });
                            }
                            ui.horizontal(|ui| {
                                if ui.button("Update now").clicked() {
                                    start = Some(url.clone());
                                }
                                if ui.button("Later").clicked() {
                                    dismiss = true;
                                }
                                if ui.button("Skip this version").clicked() {
                                    skip = Some(version.clone());
                                }
                            });
                        }
                        State::Downloading => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label("Downloading the update...");
                            });
                        }
                        State::Failed(e) => {
                            ui.label(e.as_str());
                            if ui.button("Dismiss").clicked() {
                                dismiss = true;
                            }
                        }
                        _ => {}
                    }
                });
            });

        if let Some(v) = skip {
            skip_version(&v);
            self.state = State::Dismissed;
        } else if let Some(url) = start {
            self.state = State::Downloading;
            let tx = self.tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let result = download_and_run(&url);
                let _ = tx.send(Msg::Downloaded(result));
                ctx.request_repaint();
            });
        } else if dismiss {
            self.state = State::Dismissed;
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
        self.app.update(ctx, frame);
        self.show_update_box(ctx);
    }
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

fn skip_file() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|d| d.join("litewave").join("skipped_update.txt"))
}

fn skip_version(version: &str) {
    if let Some(f) = skip_file() {
        if let Some(dir) = f.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(f, version);
    }
}

fn check() -> Option<Msg> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .user_agent(concat!("litewave/", env!("CARGO_PKG_VERSION")))
        .build();
    let release: serde_json::Value = agent
        .get(&format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .set("Accept", "application/vnd.github+json")
        .call()
        .ok()?
        .into_json()
        .ok()?;
    let tag = release["tag_name"].as_str()?;
    if !is_newer(tag, env!("CARGO_PKG_VERSION")) {
        return None;
    }
    let version = tag.trim_start_matches('v').to_string();
    let skipped = skip_file().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
    if skipped.trim() == version {
        return None;
    }
    let notes: String = release["body"].as_str().unwrap_or("").chars().take(700).collect();
    let url = release["assets"]
        .as_array()?
        .iter()
        .find(|a| a["name"].as_str() == Some(ASSET))?["browser_download_url"]
        .as_str()?
        .to_string();
    Some(Msg::Available { version, url, notes })
}

fn download_and_run(url: &str) -> Result<(), String> {
    // Only ever run an installer that was downloaded from this project's own releases.
    let allowed = format!("https://github.com/{REPO}/releases/download/");
    if !url.starts_with(&allowed) {
        return Err("The update link is not from the litewave releases page.".into());
    }
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(60))
        .user_agent(concat!("litewave/", env!("CARGO_PKG_VERSION")))
        .build();
    let resp = agent.get(url).call().map_err(|e| format!("Download failed: {e}"))?;
    let path = std::env::temp_dir().join(ASSET);
    let mut file = std::fs::File::create(&path).map_err(|e| format!("Could not save the update: {e}"))?;
    let mut reader = resp.into_reader();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("Download failed: {e}"))?;
        if n == 0 {
            break;
        }
        std::io::Write::write_all(&mut file, &buf[..n]).map_err(|e| format!("Could not save the update: {e}"))?;
    }
    drop(file);
    std::process::Command::new(&path)
        .args(["/SILENT", "/SUPPRESSMSGBOXES", "/CLOSEAPPLICATIONS"])
        .spawn()
        .map_err(|e| format!("Could not start the installer: {e}"))?;
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
}
