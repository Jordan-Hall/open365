//! New documents from templates (`.dotx`, `.dotm`, `.xltx`, `.xltm`).
//!
//! The template's content type becomes the plain (or macro-enabled) document type; the VBA
//! project, when present, is kept byte-identical and never run. The result is an unsaved
//! [`Target`] with no path, so saving it is an `Export` (the caller's `Save` command).

use open365_docx::Document;
use open365_opc::Limits;
use open365_xlsx::Workbook;

use crate::error::CommandError;
use crate::target::Target;

/// The extension of the document made from a template with extension `template_ext`
/// (case-insensitive, no dot): `dotx` -> `docx`, `dotm` -> `docm`, `xltx` -> `xlsx`,
/// `xltm` -> `xlsm`. `None` for anything else.
pub fn template_output_extension(template_ext: &str) -> Option<&'static str> {
    match template_ext.to_ascii_lowercase().as_str() {
        "dotx" => Some("docx"),
        "dotm" => Some("docm"),
        "xltx" => Some("xlsx"),
        "xltm" => Some("xlsm"),
        _ => None,
    }
}

fn not_a_template(ext: &str) -> CommandError {
    CommandError::Log(format!("the file is not a template (.{ext} content type)"))
}

impl Target {
    /// Opens template bytes as a new, unsaved document. `ext` is the template's extension.
    ///
    /// # Errors
    /// An unsupported extension, a package whose main part is not a template type, or an
    /// error opening or saving the package.
    pub fn open_template(bytes: &[u8], ext: &str) -> Result<Self, CommandError> {
        let ext = ext.to_ascii_lowercase();
        match ext.as_str() {
            "dotx" | "dotm" => {
                let mut doc = Document::open(bytes, &Limits::default())?;
                if !doc.instantiate_template()? {
                    return Err(not_a_template(&ext));
                }
                // Reopen the saved bytes so undo replays onto the instantiated document.
                let saved = doc.save()?;
                Self::open_docx(&saved)
            }
            "xltx" | "xltm" => {
                let mut wb = Workbook::open(bytes, &Limits::default())?;
                if !wb.instantiate_template()? {
                    return Err(not_a_template(&ext));
                }
                let saved = wb.save()?;
                Self::open_xlsx(&saved)
            }
            _ => Err(CommandError::Log(format!(
                "unsupported template extension .{ext}"
            ))),
        }
    }
}
