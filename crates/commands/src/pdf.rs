//! The `ExportPdf` command: layout with the bundled fonts, then the PDF writer (ADR-004).

use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use open365_docx::Document;
use open365_layout::{LayoutOptions, layout_document_cancellable};
use open365_pdf::{PdfError, PdfMetadata, export_pdf_cancellable};
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
    /// Cooperative cancellation. Layout, the PDF writer and the file write all look at it and
    /// give up with [`CommandError::Cancelled`] when it is set. A file this export created is
    /// removed again; a file that already existed is never touched.
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
    // Without a token the layout still runs through the same entry point, on a flag nobody sets.
    let flag = options.cancel.clone().unwrap_or_default();
    let lay = |fonts: &FontLibrary| {
        layout_document_cancellable(doc, fonts, LayoutOptions::default(), &flag)
            .ok_or(CommandError::Cancelled)
    };
    let mut fonts = FontLibrary::bundled();
    let mut layout = lay(&fonts)?;
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
            layout = lay(&fonts)?;
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
    let out = export_pdf_cancellable(doc, &layout, &fonts, &options.metadata, &flag).map_err(
        |e| match e {
            PdfError::Cancelled => CommandError::Cancelled,
            e => CommandError::Pdf(e.to_string()),
        },
    )?;
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
pub(crate) fn write_pdf(
    path: &Path,
    bytes: &[u8],
    overwrite: bool,
    options: &PdfOptions,
) -> Result<(), CommandError> {
    if overwrite {
        options.check_cancelled()?;
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
    // `create_new` succeeded, so this request made the file and may remove it again.
    let written = options
        .check_cancelled()
        .and_then(|()| {
            file.write_all(bytes)
                .and_then(|()| file.flush())
                .map_err(|e| io("write", e))
        })
        .and_then(|()| options.check_cancelled());
    if written.is_err() {
        drop(file);
        let _ = std::fs::remove_file(path);
    }
    written
}
