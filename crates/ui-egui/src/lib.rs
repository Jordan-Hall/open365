//! Thin egui/eframe shell for Open365 (ADR-004).
//!
//! The crate holds no document logic. It reads documents through `open365-commands`, sends
//! every edit as a command, and draws two custom views (a paged document and a spreadsheet
//! grid) that publish their own AccessKit trees for screen readers.

mod a11y;
mod app;
mod doc_view;
mod docmodel;
mod fonts;
mod grid_text;
mod grid_view;
mod parts;

use std::path::PathBuf;

pub use app::{App, print_command};
pub use doc_view::{DocPos, DocView, Toggle};
pub use grid_view::{GridView, PaintedCell, Resize};

/// Opens the main window and runs until it closes. `initial` is a file to open at start-up.
pub fn run(initial: Option<PathBuf>) -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Open365")
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([640.0, 400.0]),
        ..eframe::NativeOptions::default()
    };
    eframe::run_native(
        "Open365",
        options,
        Box::new(move |_cc| Ok(Box::new(App::new(initial)))),
    )
}
