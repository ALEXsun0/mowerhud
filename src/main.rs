#![windows_subsystem = "windows"]

mod api;
mod app;
mod config;
mod model;
mod notify;
mod ui;

use app::MowerHUD;
use eframe::egui;

#[tokio::main]
async fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([520.0, 80.0])
            .with_min_inner_size([340.0, 80.0])
            .with_max_inner_size([1000.0, 360.0])
            .with_resizable(false)
            .with_maximized(false)
            .with_always_on_top()
            .with_decorations(false), // 隐藏标题栏
        ..Default::default()
    };

    eframe::run_native(
        "MowerHUD",
        options,
        Box::new(|cc| Ok(Box::new(MowerHUD::new(cc)))),
    )
}
