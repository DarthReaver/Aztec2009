//! Native windowed workspace (egui): Maya 2009 style layout with a high resolution software rendered viewport.

mod app;
pub mod icons;
pub mod png;
pub mod theme;
pub mod viewport;

use crate::session::Session;
use eframe::egui;

pub use app::snapshot_png;

/// Open the window and run until it closes.
pub fn run(session: Session) -> Result<(), String> {
    let icon = egui::IconData { rgba: png::app_icon(128), width: 128, height: 128 };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Aztec 2009")
            .with_app_id("aztec2009")
            .with_inner_size([1440.0, 860.0])
            .with_min_inner_size([900.0, 560.0])
            .with_icon(icon),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "Aztec 2009",
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            Ok(Box::new(app::App::new(session)))
        }),
    )
    .map_err(|e| e.to_string())
}
