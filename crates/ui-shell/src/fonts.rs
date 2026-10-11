//! UI font: Segoe UI when the operating system has it (never bundled), else bundled Carlito.

use std::sync::Arc;

use egui::{Context, FontData, FontDefinitions, FontFamily};

const FALLBACK: &[u8] = include_bytes!("../../../assets/fonts/Carlito-Regular.ttf");

fn system_segoe() -> Option<Vec<u8>> {
    let windir = std::env::var_os("WINDIR")?;
    std::fs::read(
        std::path::Path::new(&windir)
            .join("Fonts")
            .join("segoeui.ttf"),
    )
    .ok()
}

/// The operating system's UI font as (name, TrueType bytes), when it is installed (never bundled).
#[must_use]
pub fn system_ui_font() -> Option<(&'static str, Vec<u8>)> {
    system_segoe().map(|b| ("Segoe UI", b))
}

/// Installs the UI font as the proportional and monospace family. Call once at start-up, only
/// when the application does not share egui's font table with another component.
///
/// Returns the name of the font installed.
pub fn install(ctx: &Context) -> &'static str {
    let (name, data): (&'static str, Vec<u8>) = match system_segoe() {
        Some(bytes) => ("Segoe UI", bytes),
        None => ("Carlito", FALLBACK.to_vec()),
    };
    let mut defs = FontDefinitions::empty();
    defs.font_data
        .insert(name.to_owned(), Arc::new(FontData::from_owned(data)));
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        defs.families.insert(family, vec![name.to_owned()]);
    }
    ctx.set_fonts(defs);
    name
}
