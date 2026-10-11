//! The `ExportPdf` command: layout with the bundled fonts, then the PDF writer (ADR-004).

use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use open365_docx::Document;
use open365_layout::{LayoutOptions, layout_document};
use open365_pdf::{PdfMetadata, export_pdf};
use open365_text::{FontLibrary, FontMatch};

use crate::error::CommandError;
use crate::target::write_atomic;

/// What `ExportPdf` needs besides the document. Not part of the command: it is policy of the
/// caller (a CLI flag, a server limit), never document state.
#[derive(Debug, Clone, Default)]
pub struct PdfOptions {
    pub metadata: PdfMetadata,
    /// Use fonts installed on this machine for families the bundled fonts cannot stand in for.
    /// The result then depends on the machine.
    pub system_fonts: bool,
    /// Refuse a PDF larger than this many bytes (nothing is written).
    pub max_bytes: Option<usize>,
    /// Cooperative cancellation. The export looks at it before laying out, before writing the
    /// PDF and before creating the file, and gives up with [`CommandError::Cancelled`] when it
    /// is set. The layout and the PDF writer themselves cannot be interrupted.
    pub cancel: Option<Arc<AtomicBool>>,
}

impl PdfOptions {
    pub(crate) fn check_cancelled(&self) -> Result<(), CommandError> {
        match &self.cancel {
            Some(flag) if flag.load(Ordering::Relaxed) => Err(CommandError::Cancelled),
            _ => Ok(()),
        }
    }
}

pub(crate) struct Rendered {
    pub bytes: Vec<u8>,
    pub pages: usize,
    /// Missing fonts and PDF writer warnings.
    pub warnings: Vec<String>,
}

pub(crate) fn render(doc: &Document, options: &PdfOptions) -> Result<Rendered, CommandError> {
    options.check_cancelled()?;
    let mut fonts = FontLibrary::bundled();
    let mut layout = layout_document(doc, &fonts, LayoutOptions::default());
    if options.system_fonts {
        let missing: Vec<String> = layout
            .fonts
            .iter()
            .filter(|f| f.matched == FontMatch::Fallback)
            .map(|f| f.requested.clone())
            .collect();
        if !missing.is_empty() {
            let names: Vec<&str> = missing.iter().map(String::as_str).collect();
            fonts.add_system_families(&names, &[]);
            layout = layout_document(doc, &fonts, LayoutOptions::default());
        }
    }
    options.check_cancelled()?;
    let mut warnings: Vec<String> = layout
        .fonts
        .iter()
        .filter(|f| f.matched == FontMatch::Fallback)
        .map(|f| {
            format!(
                "font \"{}\" is not available; the default font is used",
                f.requested
            )
        })
        .collect();
    let out = export_pdf(doc, &layout, &fonts, &options.metadata)
        .map_err(|e| CommandError::Pdf(e.to_string()))?;
    if let Some(limit) = options.max_bytes
        && out.bytes.len() > limit
    {
        return Err(CommandError::TooLarge { limit });
    }
    options.check_cancelled()?;
    warnings.extend(out.warnings);
    Ok(Rendered {
        bytes: out.bytes,
        pages: layout.pages.len(),
        warnings,
    })
}

/// Writes the PDF. Without `overwrite` the file is created exclusively (`create_new`), so an
/// existing file or a planted link is never followed or replaced; with it the bytes go to a
/// sibling temporary file that replaces the target.
pub(crate) fn write_pdf(path: &Path, bytes: &[u8], overwrite: bool) -> Result<(), CommandError> {
    if overwrite {
        return write_atomic(path, bytes);
    }
    let io = |op: &'static str, source| CommandError::Io {
        op,
        path: path.display().to_string(),
        source,
    };
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                CommandError::Exists {
                    path: path.display().to_string(),
                }
            } else {
                io("create", e)
            }
        })?;
    if let Err(e) = file.write_all(bytes).and_then(|()| file.flush()) {
        let _ = std::fs::remove_file(path);
        return Err(io("write", e));
    }
    Ok(())
}
