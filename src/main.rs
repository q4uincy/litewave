#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod config;
mod library;
mod player;
mod spotify;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("litewave")
            .with_inner_size([760.0, 600.0])
            .with_min_inner_size([360.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native(
        "litewave",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
