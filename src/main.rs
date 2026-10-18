#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod config;
mod library;
mod player;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("litewave")
            .with_inner_size([760.0, 560.0])
            .with_min_inner_size([520.0, 380.0]),
        ..Default::default()
    };
    eframe::run_native(
        "litewave",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
