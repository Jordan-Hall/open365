//! Open365 desktop entry point: `open365-desktop [file.docx|file.xlsx]`.

use std::path::PathBuf;

use open365_ui_egui::App;

fn main() -> eframe::Result {
    let initial = std::env::args_os().nth(1).map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Open365")
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([640.0, 400.0]),
        ..eframe::NativeOptions::default()
    };
    eframe::run_native(
        "Open365",
        options,
        // Installed fonts first, so the on-screen layout is Word's when its fonts are installed;
        // the bundled Carlito and Caladea stand in for the rest.
        Box::new(move |_cc| Ok(Box::new(App::new(initial).with_system_fonts()))),
    )
}
