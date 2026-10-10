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
mod edit;
mod error;
mod model;
mod ppr;
mod props;
mod ranges;
mod styles;
mod xml;

pub use document::{Document, DroppedItem, HygieneAction};
pub use error::DocxError;
pub use model::{
    Block, Body, Break, BreakKind, Cell, DocChild, DocumentRoot, Group, Inline, Paragraph, Row,
    RowChild, Run, RunContent, Table, TableChild,
};
pub use ppr::Alignment;
pub use props::{RunPropItem, RunPropKind, RunProps, RunPropsPatch};
pub use ranges::{DocPos, EditLogEntry, Snapshot};
pub use styles::{
    EffectiveParagraph, EffectiveRun, LineSpacing, StyleInfo, StyleKind, Styles, ThemeFonts,
};
pub use xml::{
    Attr, ElementHead, MAX_XML_DEPTH, ParsedXml, RawXml, W_NS, W_NS_STRICT, XmlElement,
    parse as parse_xml,
};
