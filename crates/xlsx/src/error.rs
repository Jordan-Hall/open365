use open365_opc::OpcError;

/// Errors from opening, editing or saving an XLSX workbook.
#[derive(Debug, thiserror::Error)]
pub enum XlsxError {
    #[error("package error: {0}")]
    Opc(#[from] OpcError),
    #[error("XML error: {0}")]
    Xml(String),
    #[error("DOCTYPE/DTD is not allowed in workbook XML")]
    DoctypeForbidden,
    #[error("XML nesting deeper than {0} levels")]
    DepthExceeded(usize),
    #[error("part is not UTF-8 XML (UTF-16 and other encodings are unsupported)")]
    NotUtf8,
    #[error("package has no officeDocument relationship")]
    NoWorkbook,
    #[error("part {0:?} is missing from the package")]
    MissingPart(String),
    #[error("not SpreadsheetML: {0}")]
    NotSpreadsheet(String),
    #[error("malformed {part}: {reason}")]
    Malformed { part: String, reason: String },
    #[error("sheet index {index} out of range (workbook has {count})")]
    SheetOutOfRange { index: usize, count: usize },
    #[error("sheet {0} is not a worksheet and has no cells")]
    NotAWorksheet(usize),
    #[error("invalid cell reference {0:?}")]
    InvalidCellRef(String),
    #[error("cell (row {row}, column {col}) is outside the 1048576 x 16384 grid")]
    CellOutOfRange { row: u32, col: u32 },
    #[error("number {0} cannot be stored (NaN and infinities are not valid cell numbers)")]
    InvalidNumber(f64),
    #[error("text contains {0:?}, which XML 1.0 cannot represent")]
    InvalidText(char),
    #[error("unsupported edit: {0}")]
    UnsupportedEdit(String),
    #[error("{0} item(s) in the dropped-content log; refusing to save silently")]
    DroppedContent(usize),
}
