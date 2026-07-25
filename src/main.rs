//! Noter — A pure, reliable, cross-platform plain text editor.
//!
//! This is currently a planning skeleton. See README.md, REQUIREMENTS.md,
//! DESIGN.md, and ROADMAP.md for the full vision, architecture, and phased
//! implementation plan with strict quality gates.
//!
//! Philosophy (short version):
//! - Classic Notepad spirit: open file, edit text, save file, get out of the way.
//! - Zero telemetry, zero bloat, zero "smart" rewriting of user content.
//! - System light/dark theme + optional Markdown preview as the only 2026 QOL additions.
//! - Reliability (atomic saves, recovery, line-ending fidelity) is the #1 feature.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release

pub mod core;
pub mod error;
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

