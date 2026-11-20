//! 10YL // TACTICAL DEPLOYMENT — a small launcher for a Project Zomboid mod pack.
//! It reads the pack through pz-mod-manager, checks a release manifest, downloads
//! and installs updates, then starts the client with its own -cachedir.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod config;
mod jobs;
mod pack;
mod theme;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 720.0])
            .with_min_inner_size([1080.0, 640.0])
            .with_title("10YL // TACTICAL DEPLOYMENT"),
        ..Default::default()
    };
    eframe::run_native(
        "10YL // Tactical Deployment",
        options,
        Box::new(|cc| {
            theme::install(&cc.egui_ctx);
            Ok(Box::new(app::Launcher::new(cc)))
        }),
    )
}
