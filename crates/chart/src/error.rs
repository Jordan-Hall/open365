use std::fmt;

/// Why a chart part could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChartError {
    /// The XML did not parse (malformed, DOCTYPE, too deep).
    Xml(String),
    /// The part is XML but not a `c:chartSpace` with a `c:chart`.
    NotAChart,
}

impl fmt::Display for ChartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Xml(e) => write!(f, "chart part is not valid XML: {e}"),
            Self::NotAChart => f.write_str("part is not a DrawingML chart (c:chartSpace/c:chart)"),
        }
    }
}

impl std::error::Error for ChartError {}
