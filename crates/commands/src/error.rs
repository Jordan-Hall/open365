use open365_docx::DocxError;
use open365_xlsx::XlsxError;

/// Why a command was refused or failed. A failed command changes nothing.
#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error("{command} does not apply to a {target} document")]
    WrongTarget {
        command: &'static str,
        target: &'static str,
    },
    #[error("DOCX error: {0}")]
    Docx(#[from] DocxError),
    #[error("XLSX error: {0}")]
    Xlsx(#[from] XlsxError),
    #[error("invalid formula: {0}")]
    Formula(String),
    #[error("{op} {path}: {source}")]
    Io {
        op: &'static str,
        path: String,
        source: std::io::Error,
    },
    #[error("PDF export failed: {0}")]
    Pdf(String),
    #[error("{path} exists; pass overwrite to replace it")]
    Exists { path: String },
    #[error("the PDF is larger than {limit} bytes")]
    TooLarge { limit: usize },
    #[error("the operation was cancelled")]
    Cancelled,
    #[error("nothing to undo")]
    NothingToUndo,
    #[error("nothing to redo")]
    NothingToRedo,
    #[error("the last edit cannot be undone (its previous content is not restorable)")]
    NotUndoable,
    #[error("find and replace: {0}")]
    Find(#[from] open365_docx::FindError),
    #[error("paste: {0}")]
    Paste(String),
    #[error("invalid command log: {0}")]
    Log(String),
    #[error("cell format: {0}")]
    Format(String),
}
