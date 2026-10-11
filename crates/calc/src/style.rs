//! Cell presentation that worksheet functions can observe (`CELL`): number format, horizontal alignment,
//! the locked flag and column widths. The engine stores them; nothing here affects values.

/// Horizontal alignment of a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HAlign {
    /// Text left, numbers right.
    #[default]
    General,
    /// Left.
    Left,
    /// Right.
    Right,
    /// Centred.
    Center,
    /// Repeat to fill the cell.
    Fill,
    /// Justified.
    Justify,
    /// Centred across the selection.
    CenterAcrossSelection,
    /// Distributed.
    Distributed,
}

/// What `CELL` can ask a cell about its appearance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellStyle {
    /// Number format code (en-US syntax); `None` is `General`.
    pub number_format: Option<String>,
    /// Horizontal alignment.
    pub halign: HAlign,
    /// Protection flag: cells are locked unless cleared.
    pub locked: bool,
}

impl Default for CellStyle {
    fn default() -> Self {
        CellStyle {
            number_format: None,
            halign: HAlign::General,
            locked: true,
        }
    }
}

/// Excel's default column width in characters, before it is rounded for `CELL("width")`.
pub const STANDARD_COLUMN_WIDTH: f64 = 8.43;
