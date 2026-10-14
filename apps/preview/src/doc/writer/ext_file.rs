//! File tab support for the Writer pane: replacing the open document (New, Open), exporting a
//! PDF and the dirty check that guards closing.

use std::path::PathBuf;

use open365_clipboard::MemoryPayload;
use open365_commands::Command;
use open365_ui_core::Ui;

use super::WriterPane;

impl WriterPane {
    /// Asks for a `.docx` to open (the `rfd` dialog).
    #[must_use]
    pub fn choose_open_path() -> Option<PathBuf> {
        rfd::FileDialog::new()
            .add_filter(open365_branding::FORMAT_DOCX, &["docx"])
            .set_title("Open")
            .pick_file()
    }

    /// Swaps the open document for `new`, keeping the clipboard, the path chooser, the font
    /// choice, the view area and the zoom, and moving the canvas node to the new pane.
    pub fn replace_document(&mut self, ui: &mut Ui, mut new: WriterPane) {
        let parent = self
            .canvas
            .and_then(|c| ui.tree.get(c).and_then(|n| n.parent));
        self.release(ui);
        new.clipboard = std::mem::replace(&mut self.clipboard, Box::new(MemoryPayload::default()));
        new.chooser = self.chooser;
        if self.system_fonts {
            new.use_system_fonts();
        }
        let (area, scale, zoom) = (self.area, self.scale, self.zoom);
        *self = new;
        self.set_view(area, scale, zoom);
        if let Some(p) = parent {
            self.build(ui, p, area, scale, zoom);
        }
    }

    /// Exports the document as a PDF to `path` (default: the document's path with `.pdf`; a
    /// document without a path asks with the path chooser). The document is not changed.
    pub fn export_pdf(&mut self, path: Option<PathBuf>) -> bool {
        let path = path
            .or_else(|| self.path().map(|p| p.with_extension("pdf")))
            .or_else(|| (self.chooser)());
        let Some(mut path) = path else { return false };
        path.set_extension("pdf");
        match self.run(Command::ExportPdf {
            path: path.clone(),
            overwrite: true,
        }) {
            Some(out) => {
                self.status = Some(format!(
                    "Exported {} ({} bytes)",
                    path.display(),
                    out.bytes_written
                ));
                true
            }
            None => false,
        }
    }
}
