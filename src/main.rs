#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod art;
mod config;
mod library;
mod lyrics;
mod mediakeys;
mod player;
mod resume;
mod single;
mod spotify;
mod store;
mod theme;
mod update;
mod ytdlp;

/// The release build has no console, so a crash would be silent. Write it to a file and show it.
fn report(msg: &str) {
    if let Some(dir) = dirs::config_dir().map(|d| d.join("litewave")) {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("crash.log"), msg);
    }
    let short: String = msg.chars().take(1200).collect();
    let _ = rfd::MessageDialog::new()
        .set_title("litewave could not run")
        .set_description(short)
        .set_level(rfd::MessageLevel::Error)
        .show();
}

/// The litewave logo as the window / taskbar icon.
fn app_icon() -> Option<eframe::egui::IconData> {
    let img = image::load_from_memory(include_bytes!("../assets/icon.png"))
        .ok()?
        .to_rgba8();
    let (width, height) = img.dimensions();
    Some(eframe::egui::IconData { rgba: img.into_raw(), width, height })
}

fn main() -> eframe::Result<()> {
    std::panic::set_hook(Box::new(|info| {
        if player::EXPECT_PANIC.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let bt = std::backtrace::Backtrace::force_capture();
        report(&format!("litewave crashed:\n{info}\n\n{bt}"));
    }));
    // Only one litewave window: a second start brings the first to the front and exits.
    if single::start().is_err() {
        return Ok(());
    }
    // Try OpenGL first. If the PC has no usable OpenGL (e.g. only the Windows "Basic Display
    // Adapter"), retry with wgpu, which uses DirectX / Vulkan instead.
    let make_options = |renderer: eframe::Renderer| {
        let mut viewport = eframe::egui::ViewportBuilder::default()
            .with_title("litewave")
            .with_inner_size([1000.0, 640.0])
            .with_min_inner_size([420.0, 480.0]);
        if let Some(icon) = app_icon() {
            viewport = viewport.with_icon(icon);
        }
        eframe::NativeOptions {
            viewport,
            renderer,
            ..Default::default()
        }
    };
    let mut res = eframe::run_native(
        "litewave",
        make_options(eframe::Renderer::Glow),
        Box::new(|cc| Ok(Box::new(update::Root::new(cc)))),
    );
    if res.is_err() {
        res = eframe::run_native(
            "litewave",
            make_options(eframe::Renderer::Wgpu),
            Box::new(|cc| Ok(Box::new(update::Root::new(cc)))),
        );
    }
    if let Err(e) = &res {
        report(&format!(
            "litewave could not start its window:\n{e}\n\n\
             Your graphics driver may be missing or too old. Install the latest driver for your \
             graphics card (Intel, NVIDIA or AMD) from your PC maker's website, then try again."
        ));
    }
    res
}
