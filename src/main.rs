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

fn main() {
    // In Phase 0 this is intentionally just a marker.
    // Real implementation will use eframe::run_native(...) with a NoterApp.
    //
    // Planned structure (see DESIGN.md):
    //   - src/app.rs          → NoterApp implementing eframe::App
    //   - src/core/*          → Document, Editor, Undo, Recovery (zero egui knowledge)
    //   - src/ui/*            → egui widgets, menu, find bar, status, markdown preview
    //   - src/platform/*      → theme detection, shortcut mapping
    //
    // Every commit that touches main must keep `cargo fmt -- --check` and
    // `cargo clippy --all-targets -- -D warnings` completely clean.

    println!("Noter (planning skeleton)");
    println!("Version: {}", env!("CARGO_PKG_VERSION"));
    println!("See README.md for build instructions and current status.");
    println!("All planning documents live in the repo root and are part of the product.");
}
