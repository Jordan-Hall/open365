use open365_opc::OpcError;

/// Errors from opening, editing or saving a DOCX document.
#[derive(Debug, thiserror::Error)]
pub enum DocxError {
    #[error("package error: {0}")]
    Opc(#[from] OpcError),
    #[error("XML error: {0}")]
    Xml(String),
    #[error("DOCTYPE/DTD is not allowed in document XML")]
    DoctypeForbidden,
    #[error("XML nesting deeper than {0} levels")]
    DepthExceeded(usize),
    #[error("part is not UTF-8 XML (UTF-16 and other encodings are unsupported)")]
    NotUtf8,
    #[error("package has no officeDocument relationship")]
    NoMainDocument,
    #[error("main document part {0:?} is missing from the package")]
    MissingMainPart(String),
    #[error("main document is not WordprocessingML: {0}")]
    NotWordprocessing(String),
    #[error("paragraph index {index} out of range (document has {count})")]
    ParagraphOutOfRange { index: usize, count: usize },
    #[error("character range {start}..{end} out of range for a paragraph of {len} characters")]
    RangeOutOfBounds {
        start: usize,
        end: usize,
        len: usize,
    },
    #[error("text contains {0:?}, which XML 1.0 cannot represent")]
    InvalidText(char),
    #[error("cannot edit this range: {0}")]
    UnsupportedRange(String),
    #[error("the deletion would cut a field in two; select the whole field or only its result")]
    FieldCut,
    #[error("style {0:?} is not defined in the document")]
    UnknownStyle(String),
    #[error("revision {index} out of range (document has {count})")]
    NoSuchRevision { index: usize, count: usize },
    #[error("comment {0} does not exist")]
    NoSuchComment(u32),
    #[error("comments cannot be edited: {0}")]
    CommentsUnavailable(String),
    #[error("{0} item(s) in the dropped-content log; refusing to save silently")]
    DroppedContent(usize),
}
