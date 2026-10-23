#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use disk_usage_inspector::app::App;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1180.0, 780.0])
            .with_min_inner_size([760.0, 480.0])
            .with_title("Disk Usage Inspector"),
        ..Default::default()
    };
    eframe::run_native(
        "Disk Usage Inspector",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}
