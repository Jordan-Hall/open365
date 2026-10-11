//! DOCX document model on top of [`open365_opc`].
//!
//! [`Document::open`] reads a package, finds the main story through the package
//! relationships and builds a typed model of it. Everything the model does not interpret
//! stays as raw XML in place, so an unedited story serialises back to an
//! infoset-equal part and every other part is written back byte-identical (ADR-001).
//!
//! Paragraph indices count paragraphs in document order, including those inside tables.
//! Character offsets are Unicode scalar values.

mod comments;
mod document;
mod drawing;
mod edit;
mod error;
mod fields;
mod fields_date;
mod fields_eval;
mod fields_formula;
mod fields_locale;
mod fields_locale_data;
mod fields_number;
mod fields_props;
mod fields_refs;
mod fields_scan;
mod fields_snap;
mod fields_toc;
mod fields_toc_styles;
mod fields_update;
mod flatten;
mod layout_props;
mod model;
mod notes;
mod numbering;
mod ppr;
mod props;
mod ranges;
mod review;
mod search;
mod section;
mod settings;
mod styles;
mod table;
mod table_edit;
mod track;
mod xml;
mod xmlq;

pub use comments::{Comment, CommentAnchor, NewComment};
pub use document::{Document, DocumentKind, DroppedItem, HygieneAction};
pub use drawing::{
    Align, Drawing, EMU_PER_POINT, HorizontalRelativeFrom, ImagePart, Position, TextBox,
    TextBoxAnchor, VerticalRelativeFrom, WrapSide, WrapType,
};
pub use error::DocxError;
pub use fields::{
    Arg, DateNames, Field, FieldContext, FieldKind, FieldUpdateReport, GeneralFormat, Instruction,
    MergeData, NoPages, PageInfo, PageProvider, SkipReason, SkippedField, Switch,
};
pub use fields_date::{CivilDateTime, format_date_picture};
pub use fields_formula::{FormulaEnv, Num as FormulaNum, evaluate as evaluate_formula};
pub use fields_locale::LocaleNames;
pub use fields_number::{NumberSymbols, format_picture as format_number_picture};
pub use flatten::{FlatBlocks, SDT_END, SDT_START, flatten_content_controls, is_sdt_marker};
pub use layout_props::{
    Ligatures, ParagraphBorders, ParagraphMark, ParagraphProps, RunLayoutProps, TabKind, TabLeader,
    TabStop,
};
pub use model::{
    Block, Body, Break, BreakKind, Cell, DocChild, DocumentRoot, Group, Inline, Paragraph, Row,
    RowChild, Run, RunContent, Sdt, SdtChild, SdtContent, Table, TableChild,
};
pub use notes::{Note, NoteKind};
pub use numbering::{
    AbstractNum, IndentOverride, LEVELS, LevelJustification, LevelOverride, LevelRun, LevelSuffix,
    Num, NumLevel, NumPr, Numbering, ParagraphNumbering, ResolvedNum, StyleNumPr,
};
pub use ppr::Alignment;
pub use props::{RunPropItem, RunPropKind, RunProps, RunPropsPatch};
pub use ranges::{DocPos, EditLogEntry, Snapshot};
pub use review::{
    FinalParagraph, Revision, RevisionKind, SpanMark, TextSpan, TextView, TrackChanges,
};
pub use search::{DocMatch, FindError, FindOptions, find_in_text};
pub use section::{
    Column, Columns, DocGrid, DocGridType, HeaderFooterRef, HeaderFooterType, Orientation,
    PageMargins, PageSize, Section, SectionType,
};
pub use settings::{DEFAULT_COMPATIBILITY_MODE, Settings};
pub use styles::{
    EffectiveParagraph, EffectiveRun, LineSpacing, StyleInfo, StyleKind, Styles, TableCellFormat,
    ThemeFonts,
};
pub use table::AutoFitMode;
pub use table::{
    Border, BorderSet, CellMargins, CellPosition, CellProps, CellView, HeightRule, RowProps,
    RowView, Shading, TableJustification, TableLayoutType, TableLook, TableProps,
    TableStyleCondition, TableView, TableWidth, VerticalAlign, VerticalMerge,
};
pub use table_edit::{BorderSide, ColumnAdjust, ColumnSide, RowSide, TextSeparator};
pub use xml::{
    Attr, ElementHead, MAX_XML_DEPTH, ParsedXml, RawXml, W_NS, W_NS_STRICT, XmlElement,
    parse as parse_xml,
};
