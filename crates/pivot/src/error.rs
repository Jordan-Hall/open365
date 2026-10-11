use std::fmt;

/// Why a pivot part could not be read or a pivot could not be computed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PivotError {
    /// The XML did not parse (malformed, DOCTYPE, too deep).
    Xml(String),
    /// The part is XML but not the expected pivot root element.
    WrongPart(&'static str),
    /// The pivot cache source is not a worksheet range the engine can read (external
    /// connection, consolidation, scenario, data model).
    UnsupportedSource(String),
    /// The cache source names a sheet, range or name that does not exist.
    MissingSource(String),
    /// A pivot field index or item index points outside the cache.
    BadIndex(&'static str),
    /// The pivot uses a feature this engine does not compute (OLAP, calculated items).
    Unsupported(&'static str),
    /// Writing the grid into the workbook failed.
    Write(String),
}

impl fmt::Display for PivotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Xml(e) => write!(f, "pivot part is not valid XML: {e}"),
            Self::WrongPart(what) => write!(f, "part is not a {what}"),
            Self::UnsupportedSource(s) => write!(f, "unsupported pivot cache source: {s}"),
            Self::MissingSource(s) => write!(f, "pivot cache source not found: {s}"),
            Self::BadIndex(what) => write!(f, "pivot index out of range: {what}"),
            Self::Unsupported(what) => write!(f, "pivot feature not supported: {what}"),
            Self::Write(e) => write!(f, "cannot write pivot cells: {e}"),
        }
    }
}

impl std::error::Error for PivotError {}
