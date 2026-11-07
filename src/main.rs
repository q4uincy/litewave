#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod config;
mod library;
mod player;
mod spotify;
mod theme;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("litewave")
            .with_inner_size([1000.0, 640.0])
            .with_min_inner_size([420.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(
        "litewave",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
