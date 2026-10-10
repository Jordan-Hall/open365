//! Open365 desktop entry point: `open365-desktop [file.docx|file.xlsx]`.

use std::path::PathBuf;

fn main() -> eframe::Result {
    let initial = std::env::args_os().nth(1).map(PathBuf::from);
    open365_ui_egui::run(initial)
}
