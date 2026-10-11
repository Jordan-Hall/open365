//! DOCX document model on top of [`open365_opc`].
//!
//! [`Document::open`] reads a package, finds the main story through the package
//! relationships and builds a typed model of it. Everything the model does not interpret
//! stays as raw XML in place, so an unedited story serialises back to an
//! infoset-equal part and every other part is written back byte-identical (ADR-001).
//!
//! Paragraph indices count paragraphs in document order, including those inside tables.
//! Character offsets are Unicode scalar values.

mod document;
mod drawing;
mod edit;
mod error;
mod layout_props;
mod model;
mod notes;
mod numbering;
mod ppr;
mod props;
mod ranges;
mod section;
mod settings;
mod styles;
mod table;
mod xml;
mod xmlq;

pub use document::{Document, DocumentKind, DroppedItem, HygieneAction};
pub use drawing::{
    Align, Drawing, EMU_PER_POINT, HorizontalRelativeFrom, ImagePart, Position,
    VerticalRelativeFrom, WrapSide, WrapType,
};
pub use error::DocxError;
pub use layout_props::{
    Ligatures, ParagraphMark, ParagraphProps, RunLayoutProps, TabKind, TabLeader, TabStop,
};
pub use model::{
    Block, Body, Break, BreakKind, Cell, DocChild, DocumentRoot, Group, Inline, Paragraph, Row,
    RowChild, Run, RunContent, Table, TableChild,
};
pub use notes::{Note, NoteKind};
pub use numbering::{
    AbstractNum, IndentOverride, LEVELS, LevelJustification, LevelOverride, LevelRun, LevelSuffix,
    Num, NumLevel, NumPr, Numbering, ParagraphNumbering, ResolvedNum, StyleNumPr,
};
pub use ppr::Alignment;
pub use props::{RunPropItem, RunPropKind, RunProps, RunPropsPatch};
pub use ranges::{DocPos, EditLogEntry, Snapshot};
pub use section::{
    Column, Columns, DocGrid, DocGridType, HeaderFooterRef, HeaderFooterType, Orientation,
    PageMargins, PageSize, Section, SectionType,
};
pub use settings::{DEFAULT_COMPATIBILITY_MODE, Settings};
pub use styles::{
    EffectiveParagraph, EffectiveRun, LineSpacing, StyleInfo, StyleKind, Styles, TableCellFormat,
    ThemeFonts,
};
pub use table::{
    Border, BorderSet, CellMargins, CellPosition, CellProps, CellView, HeightRule, RowProps,
    RowView, Shading, TableJustification, TableLayoutType, TableLook, TableProps,
    TableStyleCondition, TableView, TableWidth, VerticalAlign, VerticalMerge,
};
pub use xml::{
    Attr, ElementHead, MAX_XML_DEPTH, ParsedXml, RawXml, W_NS, W_NS_STRICT, XmlElement,
    parse as parse_xml,
};
