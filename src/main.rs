//! Noter, a focused cross-platform plain-text editor.
//!
//! The current M0 prototype is not ready for daily use. See `README.md` and
//! `docs/ROADMAP.md` for the verified status and execution plan.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

mod app;

use app::NoterApp;

fn main() -> eframe::Result {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([900.0, 700.0])
            .with_min_inner_size([300.0, 200.0])
            .with_transparent(false), // Disable transparency to prevent DWM choppiness
        ..Default::default()
    };

    eframe::run_native(
        "Noter",
        options,
        Box::new(|cc| Ok(Box::new(NoterApp::new(cc)))),
    )
}
