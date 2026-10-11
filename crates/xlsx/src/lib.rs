//! XLSX workbook model on top of [`open365_opc`].
//!
//! [`Workbook::open`] reads a package, finds the workbook through the package
//! relationships, then models the worksheets, shared strings and number-format tables.
//! `sheetData` is typed (sparse rows and cells, formulas, cached values); everything else
//! in a worksheet, and the workbook and style parts, stays as raw XML in place. An
//! unedited part serialises back to an infoset-equal tree and every part the model did
//! not change is written back byte-identical (ADR-001).
//!
//! Rows and columns are 1-based throughout (`A1` is row 1, column 1); sheets are indexed
//! in workbook order from 0.
//!
//! Known representation limits: whitespace between the elements of `sheetData` is not
//! kept (it carries no meaning there); a cell written without `r` gets one on save; the
//! `t` attribute of a valueless cell is only kept when it is not the type its value
//! implies.

mod builtin_numfmt;
mod builtin_numfmt_nls;
mod conditional;
mod drawing;
mod error;
mod fast;
mod format;
mod geometry;
mod parts;
mod refs;
mod sheet;
mod sst;
mod structure;
mod styles;
mod styles_builtin;
mod styles_edit;
mod styles_named;
mod styles_numfmt;
mod tables;
mod workbook;
mod xml;

pub use builtin_numfmt::{
    builtin_number_format_in, builtin_number_format_lcids, nls_pattern_to_excel,
    resolve_number_format_lcid,
};
pub use conditional::{
    CellIsOp, CfBlock, CfKind, CfRule, Cfvo, CfvoKind, DataBarAxis, DataBarDirection, DataBarExt,
    Dxf, DxfBorder, DxfFill, DxfFont, TimePeriod,
};
pub use drawing::{Anchor, AnchorMarker, DrawingChart, EMU_PER_POINT, EditAs, RectPt};
pub use error::XlsxError;
pub use format::{
    Alignment, Border, BorderEdge, BorderStyle, CellFormat, Color, ColorSource, Fill, FillPattern,
    Font, HorizontalAlignment, Rgb, Theme, Underline, VerticalAlignment, apply_tint, shared_edge,
};
pub use geometry::{
    Axis, BASE_DPI, ColumnSpec, FrozenPanes, GridMetrics, Pane, PaneState, SheetFormat,
    SheetGeometry, default_font,
};
pub use parts::HygieneAction;
pub use refs::{
    MAX_COL, MAX_ROW, cell_ref_string, col_letters, col_number, parse_cell_ref, parse_range,
    shift_formula,
};
pub use sheet::{
    ArrayFormula, ArrayRole, AutoFilter, Cell, CellError, CellValue, ColorFilter, ColorKey,
    ColumnsSnapshot, CustomFilter, CustomFilters, DataValidation, DataValidations, DateGroup,
    DateTimeGrouping, DynamicFilter, DynamicKind, ErrorStyle, FilterColumn, FilterContext,
    FilterCriteria, FilterOperator, FilterOutcome, Formula, FormulaKind, HeaderMode, IconFilter,
    MAX_COLUMN_WIDTH, MAX_ROW_HEIGHT, MAX_SORT_KEYS, Rect, Row, RowHeightsSnapshot,
    SharedFormulaRef, SortBy, SortCondition, SortKey, SortMethod, SortOn, SortOrientation,
    SortOutcome, SortSpec, SortState, TextKey, TextOrder, Top10, ValidationContext,
    ValidationOperator, ValidationType, Validity, ValuesFilter, WorkbookCells, Worksheet,
    format_number,
};
pub use sst::{SharedStrings, decode_xstring, encode_xstring};
pub use structure::{EditAxis, FormulaShifter, StructEdit};
pub use styles::{Styles, builtin_number_format};
pub use styles_edit::{
    AlignmentPatch, BorderPatch, FillSpec, FontPatch, FormatPatch, NumberFormat, ProtectionPatch,
    Rotation,
};
pub use tables::{Table, TableColumn, TableStyle};
pub use workbook::{
    BorderSpec, CellInput, DefinedName, DroppedItem, FormatSnapshot, SheetInfo, SheetKind,
    SheetState, Workbook, WorkbookKind,
};
pub use xml::{
    Attr, ElementHead, MAIN_NS, MAIN_NS_STRICT, MAX_XML_DEPTH, ParsedXml, RawXml, XmlElement,
    parse as parse_xml,
};
