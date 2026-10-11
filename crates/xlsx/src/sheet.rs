//! Worksheet model: typed, sparse `sheetData`, everything else raw and in order.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;

use quick_xml::XmlVersion;
use quick_xml::events::{BytesStart, Event};

use crate::XlsxError;
use crate::refs::{MAX_COL, MAX_ROW, parse_cell_ref, parse_range, shift_formula};
use crate::sst::{decode_xstring, encode_xstring, item_text};
use crate::tables::Table;
use crate::xml::{
    Attr, ElementHead, Parser, RawXml, XmlElement, check_end, escape_text, is_blank, push_entity,
    write_attrs, write_head, xml_err,
};

mod resize;
mod structure;
pub use resize::{ColumnsSnapshot, MAX_COLUMN_WIDTH, MAX_ROW_HEIGHT, RowHeightsSnapshot};

/// A cell error value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CellError {
    Null,
    Div0,
    Value,
    Ref,
    Name,
    Num,
    NA,
    GettingData,
    /// An error literal this model does not know, kept as written.
    Other(Box<str>),
}

impl CellError {
    pub fn parse(code: &str) -> Self {
        match code {
            "#NULL!" => Self::Null,
            "#DIV/0!" => Self::Div0,
            "#VALUE!" => Self::Value,
            "#REF!" => Self::Ref,
            "#NAME?" => Self::Name,
            "#NUM!" => Self::Num,
            "#N/A" => Self::NA,
            "#GETTING_DATA" => Self::GettingData,
            other => Self::Other(other.into()),
        }
    }

    /// The error as written in a cell: `#N/A`.
    pub fn code(&self) -> &str {
        match self {
            Self::Null => "#NULL!",
            Self::Div0 => "#DIV/0!",
            Self::Value => "#VALUE!",
            Self::Ref => "#REF!",
            Self::Name => "#NAME?",
            Self::Num => "#NUM!",
            Self::NA => "#N/A",
            Self::GettingData => "#GETTING_DATA",
            Self::Other(s) => s,
        }
    }

    /// The `CVErr` number Excel reports for the error through `Value2`, when it has one.
    pub fn excel_code(&self) -> Option<i32> {
        Some(match self {
            Self::Null => -2_146_826_288,
            Self::Div0 => -2_146_826_281,
            Self::Value => -2_146_826_273,
            Self::Ref => -2_146_826_265,
            Self::Name => -2_146_826_259,
            Self::Num => -2_146_826_252,
            Self::NA => -2_146_826_246,
            Self::GettingData => -2_146_826_245,
            Self::Other(_) => return None,
        })
    }
}

/// The value a cell holds (for a formula cell: its cached result).
#[derive(Debug, Clone, PartialEq)]
pub enum CellValue {
    Empty,
    Number(f64),
    Bool(bool),
    Error(CellError),
    /// `t="str"`: the string result of a formula.
    Str(Arc<str>),
    /// `t="s"`: index into the shared string table and its resolved plain text.
    SharedString {
        index: u32,
        text: Arc<str>,
    },
    /// `t="inlineStr"`: the plain text of the inline string.
    InlineStr(Arc<str>),
    /// `t="d"`: an ISO 8601 date, as written.
    IsoDate(Arc<str>),
}

impl CellValue {
    /// The text of any string-like value.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Str(s) | Self::InlineStr(s) | Self::IsoDate(s) => Some(s),
            Self::SharedString { text, .. } => Some(text),
            _ => None,
        }
    }

    /// The `t` attribute this value is written with (`None` for numbers and blanks).
    pub fn type_attr(&self) -> Option<&'static str> {
        match self {
            Self::Empty | Self::Number(_) => None,
            Self::Bool(_) => Some("b"),
            Self::Error(_) => Some("e"),
            Self::Str(_) => Some("str"),
            Self::SharedString { .. } => Some("s"),
            Self::InlineStr(_) => Some("inlineStr"),
            Self::IsoDate(_) => Some("d"),
        }
    }
}

/// A legacy array formula: the master cell, its range and the formula text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArrayFormula {
    /// Master cell `(row, col)`, the only cell that stores the text.
    pub master: (u32, u32),
    /// Cells the array covers, including the master.
    pub range: Rect,
    /// Formula text without `=`.
    pub text: String,
    /// True when the master carries a dynamic-array metadata index (`cm`).
    pub dynamic: bool,
}

/// What a cell is within an array formula.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrayRole {
    Master,
    /// Part of the array anchored at `master`.
    Member {
        master: (u32, u32),
    },
}

/// A cell's place in a shared-formula group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedFormulaRef {
    pub si: u32,
    pub master: (u32, u32),
    /// The master's formula text (without `=`), before shifting.
    pub master_text: String,
    /// The `ref` range the master declares, when present.
    pub master_range: Option<Rect>,
    pub is_master: bool,
    /// Offset of this cell from the master (0 for the master).
    pub row_offset: i64,
    pub col_offset: i64,
}

/// How a formula is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormulaKind {
    /// Ordinary formula in this cell.
    Normal,
    /// Member of shared group `si`. The master carries the text and the `ref` range;
    /// dependents have empty text.
    Shared { si: u32, range: Option<String> },
    /// Array formula anchored here, spanning `range`.
    Array { range: Option<String> },
    /// What-if data table; its attributes are in [`Formula::attrs`].
    DataTable,
}

/// The `f` element of a cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Formula {
    /// Formula text without `=`. Empty for a shared-formula dependent.
    pub text: String,
    pub kind: FormulaKind,
    /// Remaining attributes in order (`ca`, `aca`, `bx`, `dt2D`, `r1`, ...).
    pub attrs: Vec<Attr>,
}

impl Formula {
    pub fn normal(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            kind: FormulaKind::Normal,
            attrs: Vec::new(),
        }
    }

    /// True when `ca="1"` (calculate on every load).
    pub fn calc_always(&self) -> bool {
        self.attrs
            .iter()
            .any(|a| a.ns.is_none() && a.name == "ca" && matches!(a.value.as_str(), "1" | "true"))
    }
}

/// Rarely used cell parts, boxed so plain cells stay small.
#[derive(Debug, Clone, PartialEq)]
struct CellExtra {
    /// Attributes other than `r`, `s`, `t`, in order (`cm`, `vm`, `ph`, namespace declarations).
    attrs: Vec<Attr>,
    /// The `v` text when it is not what the value would be written as (`3.1415899999999999`).
    raw_v: Option<Box<str>>,
    /// Attributes of the `v` element (`xml:space="preserve"` on text with edge spaces).
    v_attrs: Vec<Attr>,
    /// A `t` attribute the value would not produce (`n`, or a type on a valueless cell).
    t_override: Option<Box<str>>,
    /// The inline string element, rich text included.
    inline: Option<XmlElement>,
    /// Children after `v`/`is` (`extLst`).
    children: Vec<RawXml>,
}

/// One cell. Its row is the enclosing [`Row`]'s `r`.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    /// 1-based column.
    pub col: u32,
    /// Style index (`s`); `None` when the attribute is absent.
    pub style: Option<u32>,
    pub value: CellValue,
    pub formula: Option<Box<Formula>>,
    extra: Option<Box<CellExtra>>,
}

impl Cell {
    fn empty(col: u32) -> Self {
        Self {
            col,
            style: None,
            value: CellValue::Empty,
            formula: None,
            extra: None,
        }
    }

    /// The `t` attribute as written.
    pub fn type_attr(&self) -> Option<&str> {
        self.extra
            .as_ref()
            .and_then(|e| e.t_override.as_deref())
            .or_else(|| self.value.type_attr())
    }

    fn has_content(&self) -> bool {
        self.style.is_some()
            || self.formula.is_some()
            || self.extra.is_some()
            || !matches!(self.value, CellValue::Empty)
    }
}

/// A `row` element: its attributes (kept verbatim) and cells in column order.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// 1-based row number.
    pub r: u32,
    head: ElementHead,
    pub cells: Vec<Cell>,
    /// Children other than `c` (`extLst`).
    extras: Vec<RawXml>,
}

impl Row {
    pub(crate) fn new(r: u32, head: ElementHead) -> Self {
        Self {
            r,
            head,
            cells: Vec::new(),
            extras: Vec::new(),
        }
    }

    /// Row attributes in source order (`spans`, `ht`, `hidden`, ...), including `r`.
    pub fn attrs(&self) -> &[Attr] {
        &self.head.attrs
    }
}

/// Inclusive cell rectangle `(row1, col1, row2, col2)`.
pub type Rect = (u32, u32, u32, u32);

/// A worksheet part.
#[derive(Debug, Clone)]
pub struct Worksheet {
    prolog: String,
    epilogue: String,
    root_head: ElementHead,
    /// Root children before `sheetData`: `sheetPr`, `dimension`, `sheetViews`, `cols`, ...
    before: Vec<RawXml>,
    sheet_data_head: ElementHead,
    rows: Vec<Row>,
    /// Root children after `sheetData`: `mergeCells`, `conditionalFormatting`, `drawing`, ...
    after: Vec<RawXml>,
    /// Shared group `si` to the master cell `(row, col)`.
    shared_masters: HashMap<u32, (u32, u32)>,
    /// Array formula and data table ranges: cells inside cannot be edited one by one.
    locked: Vec<Rect>,
    /// Legacy array formulas (master cell has text), in row-major order.
    arrays: Vec<ArrayFormula>,
    /// Tables of the sheet, set by the workbook from the sheet relationships.
    tables: Vec<Table>,
    /// Descriptions of content the reader could not represent.
    dropped: Vec<String>,
}

struct Names {
    row: String,
    c: String,
    f: String,
    v: String,
    is: String,
    t: String,
}

impl Names {
    fn new(sheet_data: &ElementHead) -> Self {
        let n = |l: &str| crate::xml::sibling_name(&sheet_data.name, l);
        Self {
            row: n("row"),
            c: n("c"),
            f: n("f"),
            v: n("v"),
            is: n("is"),
            t: n("t"),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum TAttr {
    None,
    N,
    S,
    Str,
    InlineStr,
    B,
    E,
    D,
}

impl TAttr {
    pub(crate) fn parse(s: &str) -> Result<Self, XlsxError> {
        Ok(match s {
            "n" => Self::N,
            "s" => Self::S,
            "str" => Self::Str,
            "inlineStr" => Self::InlineStr,
            "b" => Self::B,
            "e" => Self::E,
            "d" => Self::D,
            other => return Err(malformed(format!("unknown cell type t={other:?}"))),
        })
    }

    fn as_str(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::N => Some("n"),
            Self::S => Some("s"),
            Self::Str => Some("str"),
            Self::InlineStr => Some("inlineStr"),
            Self::B => Some("b"),
            Self::E => Some("e"),
            Self::D => Some("d"),
        }
    }
}

fn malformed(reason: String) -> XlsxError {
    XlsxError::Malformed {
        part: "worksheet".to_owned(),
        reason,
    }
}

/// `v` text for a number: the shortest form that reads back as the same value.
pub fn format_number(v: f64, out: &mut String) {
    let a = v.abs();
    if v != 0.0 && !(1e-5..1e16).contains(&a) {
        let _ = write!(out, "{v:e}");
    } else {
        let _ = write!(out, "{v}");
    }
}

struct Ctx<'s> {
    sst: &'s [Arc<str>],
    text: String,
    fmt: String,
    dropped: Vec<String>,
}

/// Reads text-only content up to the end tag of the element just opened.
fn read_text(p: &mut Parser<'_>, out: &mut String) -> Result<(), XlsxError> {
    loop {
        match p.next_event()? {
            Event::Text(t) => out.push_str(&t.xml10_content()),
            Event::CData(c) => out.push_str(&c.into_inner()),
            Event::GeneralRef(r) => push_entity(&r, out)?,
            Event::End(_) => return Ok(()),
            Event::Comment(_) | Event::PI(_) | Event::Decl(_) => {}
            Event::Start(_) | Event::Empty(_) => {
                return Err(malformed("unexpected element inside f or v".to_owned()));
            }
            Event::Eof => return Err(xml_err("unexpected end of document")),
            Event::DocType(_) => return Err(XlsxError::DoctypeForbidden),
        }
    }
}

pub(crate) fn build_formula(head: ElementHead, text: String) -> Result<Formula, XlsxError> {
    let get = |name: &str| head.attr(name).map(str::to_owned);
    let (kind, skip): (FormulaKind, &[&str]) = match head.attr("t") {
        Some("shared") => {
            let si = head
                .attr("si")
                .and_then(|v| v.trim().parse::<u32>().ok())
                .ok_or_else(|| malformed("shared formula without a valid si".to_owned()))?;
            (
                FormulaKind::Shared {
                    si,
                    range: get("ref"),
                },
                &["t", "si", "ref"],
            )
        }
        Some("array") => (FormulaKind::Array { range: get("ref") }, &["t", "ref"]),
        Some("dataTable") => (FormulaKind::DataTable, &["t"]),
        _ => (FormulaKind::Normal, &[]),
    };
    let attrs = head
        .attrs
        .into_iter()
        .filter(|a| !(a.ns.is_none() && skip.contains(&a.name.as_str())))
        .collect();
    Ok(Formula { text, kind, attrs })
}

fn parse_cell(
    p: &mut Parser<'_>,
    e: &BytesStart<'_>,
    empty: bool,
    row: u32,
    prev_col: u32,
    cx: &mut Ctx<'_>,
) -> Result<Cell, XlsxError> {
    p.push_scope()?;
    let mut col: Option<u32> = None;
    let mut style: Option<u32> = None;
    let mut t_src = TAttr::None;
    let mut other: Vec<(String, String)> = Vec::new();
    for attr in e.attributes() {
        let attr = attr.map_err(xml_err)?;
        let key = attr.key.as_ref();
        let value = attr
            .normalized_value(XmlVersion::Explicit1_0)
            .map_err(xml_err)?;
        match key {
            "r" => {
                let (r, c) = parse_cell_ref(&value)
                    .ok_or_else(|| XlsxError::InvalidCellRef(value.to_string()))?;
                if r != row {
                    return Err(malformed(format!("cell {value} sits in row {row}")));
                }
                col = Some(c);
            }
            "s" => {
                style = Some(
                    value
                        .trim()
                        .parse()
                        .map_err(|_| malformed(format!("bad style index {value:?}")))?,
                );
            }
            "t" => t_src = TAttr::parse(&value)?,
            _ => {
                p.declare(key, &value);
                other.push((key.to_owned(), value.into_owned()));
            }
        }
    }
    let col = col.unwrap_or(prev_col + 1);
    if col <= prev_col || col > MAX_COL {
        return Err(malformed(format!(
            "cells out of order or duplicated at row {row}, column {col}"
        )));
    }

    let mut formula: Option<Formula> = None;
    let mut has_v = false;
    let mut v_attrs: Vec<Attr> = Vec::new();
    let mut inline: Option<XmlElement> = None;
    let mut children: Vec<RawXml> = Vec::new();
    cx.text.clear();
    if !empty {
        loop {
            let (ch, child_empty) = match p.next_event()? {
                Event::Start(c) => (c, false),
                Event::Empty(c) => (c, true),
                Event::End(_) => break,
                Event::Text(t) => {
                    if !is_blank(&t.xml10_content()) {
                        return Err(malformed("text directly inside a cell".to_owned()));
                    }
                    continue;
                }
                Event::Comment(_) | Event::PI(_) | Event::Decl(_) => continue,
                Event::CData(_) | Event::GeneralRef(_) => {
                    return Err(malformed("text directly inside a cell".to_owned()));
                }
                Event::Eof => return Err(xml_err("unexpected end of document")),
                Event::DocType(_) => return Err(XlsxError::DoctypeForbidden),
            };
            let name = ch.name();
            let local = ch.local_name();
            let is_main = p.is_main_element(name.as_ref());
            match local.as_ref() {
                "f" if is_main => {
                    let head = p.enter(&ch)?;
                    let mut text = String::new();
                    if !child_empty {
                        read_text(p, &mut text)?;
                    }
                    p.leave();
                    if formula.replace(build_formula(head, text)?).is_some() {
                        return Err(malformed("cell with two f elements".to_owned()));
                    }
                }
                "v" if is_main => {
                    p.push_scope()?;
                    let mut raw_attrs: Vec<(String, String)> = Vec::new();
                    for attr in ch.attributes() {
                        let attr = attr.map_err(xml_err)?;
                        let key = attr.key.as_ref();
                        let value = attr
                            .normalized_value(XmlVersion::Explicit1_0)
                            .map_err(xml_err)?;
                        p.declare(key, &value);
                        raw_attrs.push((key.to_owned(), value.into_owned()));
                    }
                    v_attrs = raw_attrs
                        .into_iter()
                        .map(|(name, value)| Attr {
                            ns: p.resolve_attr(&name),
                            name,
                            value,
                        })
                        .collect();
                    if !child_empty {
                        read_text(p, &mut cx.text)?;
                    }
                    p.leave();
                    has_v = true;
                }
                "is" if is_main => inline = Some(p.read_element(&ch, child_empty)?),
                _ => children.push(RawXml::Element(p.read_element(&ch, child_empty)?)),
            }
        }
    }
    let attrs: Vec<Attr> = other
        .into_iter()
        .map(|(name, value)| Attr {
            ns: p.resolve_attr(&name),
            name,
            value,
        })
        .collect();
    p.leave();

    finish_cell(
        CellParts {
            col,
            style,
            t_src,
            attrs,
            v_attrs,
            formula,
            has_v,
            inline,
            children,
        },
        &cx.text,
        cx.sst,
        &mut cx.fmt,
    )
}

/// Everything a cell element contributes, before its value is interpreted.
pub(crate) struct CellParts {
    pub(crate) col: u32,
    pub(crate) style: Option<u32>,
    pub(crate) t_src: TAttr,
    pub(crate) attrs: Vec<Attr>,
    pub(crate) v_attrs: Vec<Attr>,
    pub(crate) formula: Option<Formula>,
    pub(crate) has_v: bool,
    pub(crate) inline: Option<XmlElement>,
    pub(crate) children: Vec<RawXml>,
}

/// True when writing `n` would produce exactly `text`. Plain decimals of at most 15
/// significant digits always do (they survive a round trip through a double), which
/// spares the formatting for nearly every cell.
fn number_is_canonical(n: f64, text: &str, fmt: &mut String) -> bool {
    let positional = n == 0.0 || (1e-5..1e16).contains(&n.abs());
    if positional && is_short_plain_decimal(text) {
        return true;
    }
    fmt.clear();
    format_number(n, fmt);
    fmt == text
}

/// `-?(0|[1-9][0-9]*)(\.[0-9]*[1-9])?` with at most 15 significant digits.
fn is_short_plain_decimal(text: &str) -> bool {
    let b = text.as_bytes();
    let b = b.strip_prefix(b"-").unwrap_or(b);
    let (int, frac) = match b.iter().position(|&c| c == b'.') {
        Some(dot) => (&b[..dot], Some(&b[dot + 1..])),
        None => (b, None),
    };
    if int.is_empty() || !int.iter().all(u8::is_ascii_digit) || (int.len() > 1 && int[0] == b'0') {
        return false;
    }
    let mut digits = int.len();
    if int == b"0" {
        digits = 0;
    }
    if let Some(frac) = frac {
        if frac.is_empty() || !frac.iter().all(u8::is_ascii_digit) || frac[frac.len() - 1] == b'0' {
            return false;
        }
        if digits == 0 {
            // Leading zeros after "0." are not significant.
            digits = frac.len() - frac.iter().take_while(|&&c| c == b'0').count();
        } else {
            digits += frac.len();
        }
    }
    digits <= 15
}

pub(crate) fn finish_cell(
    parts: CellParts,
    vt: &str,
    sst: &[Arc<str>],
    fmt: &mut String,
) -> Result<Cell, XlsxError> {
    let CellParts {
        col,
        style,
        t_src,
        attrs,
        v_attrs,
        formula,
        has_v,
        inline,
        children,
    } = parts;
    let mut raw_v: Option<Box<str>> = None;
    let value = match t_src {
        TAttr::None | TAttr::N => {
            if !has_v {
                CellValue::Empty
            } else if vt.trim().is_empty() {
                raw_v = Some(vt.into());
                CellValue::Empty
            } else {
                let n: f64 = vt
                    .trim()
                    .parse()
                    .map_err(|_| malformed(format!("bad number {vt:?}")))?;
                if !n.is_finite() {
                    return Err(malformed(format!("non-finite number {vt:?}")));
                }
                if !number_is_canonical(n, vt, fmt) {
                    raw_v = Some(vt.into());
                }
                CellValue::Number(n)
            }
        }
        TAttr::S => {
            if !has_v {
                CellValue::Empty
            } else {
                let index: u32 = vt
                    .trim()
                    .parse()
                    .map_err(|_| malformed(format!("bad shared string index {vt:?}")))?;
                let text = usize::try_from(index)
                    .ok()
                    .and_then(|i| sst.get(i))
                    .ok_or_else(|| malformed(format!("shared string {index} does not exist")))?;
                let canonical =
                    vt.bytes().all(|b| b.is_ascii_digit()) && (vt == "0" || !vt.starts_with('0'));
                if !canonical {
                    raw_v = Some(vt.into());
                }
                CellValue::SharedString {
                    index,
                    text: Arc::clone(text),
                }
            }
        }
        TAttr::Str => {
            if !has_v {
                CellValue::Empty
            } else {
                let decoded = decode_xstring(vt);
                if encode_xstring(&decoded) != vt {
                    raw_v = Some(vt.into());
                }
                CellValue::Str(decoded.into())
            }
        }
        TAttr::InlineStr => match &inline {
            Some(is) => CellValue::InlineStr(item_text(is).into()),
            None => CellValue::Empty,
        },
        TAttr::B => {
            if !has_v {
                CellValue::Empty
            } else {
                let b = match vt.trim() {
                    "1" | "true" => true,
                    "0" | "false" => false,
                    other => return Err(malformed(format!("bad boolean {other:?}"))),
                };
                if vt != if b { "1" } else { "0" } {
                    raw_v = Some(vt.into());
                }
                CellValue::Bool(b)
            }
        }
        TAttr::E => {
            if !has_v {
                CellValue::Empty
            } else {
                let err = CellError::parse(vt.trim());
                if err.code() != vt {
                    raw_v = Some(vt.into());
                }
                CellValue::Error(err)
            }
        }
        TAttr::D => {
            if !has_v {
                CellValue::Empty
            } else {
                let decoded = decode_xstring(vt);
                if encode_xstring(&decoded) != vt {
                    raw_v = Some(vt.into());
                }
                CellValue::IsoDate(decoded.into())
            }
        }
    };
    let t_override = match (t_src.as_str(), value.type_attr()) {
        (src, derived) if src == derived => None,
        (Some(src), _) => Some(Box::<str>::from(src)),
        (None, _) => None,
    };
    let extra = if attrs.is_empty()
        && v_attrs.is_empty()
        && raw_v.is_none()
        && t_override.is_none()
        && inline.is_none()
        && children.is_empty()
    {
        None
    } else {
        Some(Box::new(CellExtra {
            attrs,
            raw_v,
            v_attrs,
            t_override,
            inline,
            children,
        }))
    };
    Ok(Cell {
        col,
        style,
        value,
        formula: formula.map(Box::new),
        extra,
    })
}

fn parse_row(
    p: &mut Parser<'_>,
    head: ElementHead,
    empty: bool,
    prev_row: u32,
    cx: &mut Ctx<'_>,
) -> Result<Row, XlsxError> {
    let mut head = head;
    let r = match head.attr("r") {
        Some(v) => {
            let r: u32 = v
                .trim()
                .parse()
                .map_err(|_| malformed(format!("bad row number {v:?}")))?;
            if !(1..=MAX_ROW).contains(&r) {
                return Err(XlsxError::CellOutOfRange { row: r, col: 1 });
            }
            r
        }
        None => {
            let r = prev_row + 1;
            head.attrs.push(Attr::plain("r", &r.to_string()));
            r
        }
    };
    if r <= prev_row {
        return Err(malformed(format!("rows out of order or duplicated at {r}")));
    }
    let mut row = Row {
        r,
        head,
        cells: Vec::new(),
        extras: Vec::new(),
    };
    if empty {
        p.leave();
        return Ok(row);
    }
    let mut prev_col = 0u32;
    loop {
        let (ch, child_empty) = match p.next_event()? {
            Event::Start(c) => (c, false),
            Event::Empty(c) => (c, true),
            Event::End(_) => {
                p.leave();
                return Ok(row);
            }
            Event::Text(t) => {
                if !is_blank(&t.xml10_content()) {
                    return Err(malformed("text directly inside a row".to_owned()));
                }
                continue;
            }
            Event::Comment(c) => {
                row.extras.push(RawXml::Comment(c.to_string()));
                continue;
            }
            Event::PI(_) | Event::Decl(_) => continue,
            Event::CData(_) | Event::GeneralRef(_) => {
                return Err(malformed("text directly inside a row".to_owned()));
            }
            Event::Eof => return Err(xml_err("unexpected end of document")),
            Event::DocType(_) => return Err(XlsxError::DoctypeForbidden),
        };
        let name = ch.name();
        if ch.local_name().as_ref() == "c" && p.is_main_element(name.as_ref()) {
            let cell = parse_cell(p, &ch, child_empty, r, prev_col, cx)?;
            prev_col = cell.col;
            row.cells.push(cell);
        } else {
            row.extras
                .push(RawXml::Element(p.read_element(&ch, child_empty)?));
        }
    }
}

fn parse_sheet_data(
    p: &mut Parser<'_>,
    cx: &mut Ctx<'_>,
    empty: bool,
) -> Result<Vec<Row>, XlsxError> {
    let mut rows: Vec<Row> = Vec::new();
    if empty {
        p.leave();
        return Ok(rows);
    }
    let mut prev_row = 0u32;
    loop {
        let (ch, child_empty) = match p.next_event()? {
            Event::Start(c) => (c, false),
            Event::Empty(c) => (c, true),
            Event::End(_) => {
                p.leave();
                return Ok(rows);
            }
            Event::Text(t) => {
                if !is_blank(&t.xml10_content()) {
                    return Err(malformed("text directly inside sheetData".to_owned()));
                }
                continue;
            }
            Event::Comment(_) | Event::PI(_) | Event::Decl(_) => continue,
            Event::CData(_) | Event::GeneralRef(_) => {
                return Err(malformed("text directly inside sheetData".to_owned()));
            }
            Event::Eof => return Err(xml_err("unexpected end of document")),
            Event::DocType(_) => return Err(XlsxError::DoctypeForbidden),
        };
        let head = p.enter(&ch)?;
        if head.is_main("row") {
            let row = parse_row(p, head, child_empty, prev_row, cx)?;
            prev_row = row.r;
            rows.push(row);
        } else {
            // Not a row: nothing in the model can hold it.
            cx.dropped.push(format!("sheetData child <{}>", head.name));
            let mut el = XmlElement {
                head,
                children: Vec::new(),
            };
            if child_empty {
                p.leave();
            } else {
                p.read_content(&mut el)?;
            }
        }
    }
}

fn push_col_letters(col: u32, out: &mut String) {
    let mut buf = [0u8; 4];
    let mut n = col;
    let mut i = buf.len();
    while n > 0 && i > 0 {
        i -= 1;
        buf[i] = b'A' + u8::try_from((n - 1) % 26).unwrap_or(0);
        n = (n - 1) / 26;
    }
    for b in &buf[i..] {
        out.push(char::from(*b));
    }
}

fn write_formula(f: &Formula, name: &str, out: &mut String) {
    out.push('<');
    out.push_str(name);
    match &f.kind {
        FormulaKind::Normal => {}
        FormulaKind::Shared { si, range } => {
            out.push_str(" t=\"shared\"");
            if let Some(range) = range {
                out.push_str(" ref=\"");
                crate::xml::escape_attr(range, out);
                out.push('"');
            }
            let _ = write!(out, " si=\"{si}\"");
        }
        FormulaKind::Array { range } => {
            out.push_str(" t=\"array\"");
            if let Some(range) = range {
                out.push_str(" ref=\"");
                crate::xml::escape_attr(range, out);
                out.push('"');
            }
        }
        FormulaKind::DataTable => out.push_str(" t=\"dataTable\""),
    }
    write_attrs(&f.attrs, out);
    if f.text.is_empty() {
        out.push_str("/>");
    } else {
        out.push('>');
        escape_text(&f.text, out);
        out.push_str("</");
        out.push_str(name);
        out.push('>');
    }
}

fn write_cell(cell: &Cell, row: u32, names: &Names, out: &mut String) {
    let extra = cell.extra.as_deref();
    out.push('<');
    out.push_str(&names.c);
    out.push_str(" r=\"");
    push_col_letters(cell.col, out);
    let _ = write!(out, "{row}\"");
    if let Some(s) = cell.style {
        let _ = write!(out, " s=\"{s}\"");
    }
    if let Some(t) = cell.type_attr() {
        let _ = write!(out, " t=\"{t}\"");
    }
    if let Some(extra) = extra {
        write_attrs(&extra.attrs, out);
    }
    let raw_v = extra.and_then(|e| e.raw_v.as_deref());
    let has_v =
        raw_v.is_some() || !matches!(cell.value, CellValue::Empty | CellValue::InlineStr(_));
    let inline = extra.and_then(|e| e.inline.as_ref());
    let wants_inline = matches!(cell.value, CellValue::InlineStr(_)) || inline.is_some();
    let ext_children = extra.map_or(&[][..], |e| e.children.as_slice());
    if cell.formula.is_none() && !has_v && !wants_inline && ext_children.is_empty() {
        out.push_str("/>");
        return;
    }
    out.push('>');
    if let Some(f) = &cell.formula {
        write_formula(f, &names.f, out);
    }
    if has_v {
        out.push('<');
        out.push_str(&names.v);
        if let Some(extra) = extra {
            write_attrs(&extra.v_attrs, out);
        }
        let before = out.len();
        out.push('>');
        if let Some(raw) = raw_v {
            escape_text(raw, out);
        } else {
            match &cell.value {
                CellValue::Number(n) => format_number(*n, out),
                CellValue::SharedString { index, .. } => {
                    let _ = write!(out, "{index}");
                }
                CellValue::Bool(b) => out.push(if *b { '1' } else { '0' }),
                CellValue::Error(e) => escape_text(e.code(), out),
                CellValue::Str(s) | CellValue::IsoDate(s) => {
                    escape_text(&encode_xstring(s), out);
                }
                CellValue::Empty | CellValue::InlineStr(_) => {}
            }
        }
        if out.len() == before + 1 {
            // Nothing between the tags: write `<v/>`.
            out.truncate(before);
            out.push_str("/>");
        } else {
            out.push_str("</");
            out.push_str(&names.v);
            out.push('>');
        }
    }
    if let Some(is) = inline {
        is.write(out);
    } else if let CellValue::InlineStr(text) = &cell.value {
        let _ = write!(out, "<{}><{}", names.is, names.t);
        if text.starts_with(char::is_whitespace)
            || text.ends_with(char::is_whitespace)
            || text.contains(['\n', '\t'])
        {
            out.push_str(" xml:space=\"preserve\"");
        }
        out.push('>');
        escape_text(&encode_xstring(text), out);
        let _ = write!(out, "</{}></{}>", names.t, names.is);
    }
    for child in ext_children {
        child.write(out);
    }
    out.push_str("</");
    out.push_str(&names.c);
    out.push('>');
}

impl Worksheet {
    pub(crate) fn parse(bytes: &[u8], sst: &[Arc<str>]) -> Result<Self, XlsxError> {
        Self::parse_with(bytes, sst, true)
    }

    /// `fast` allows the quick scanner for plain `sheetData`; tests turn it off to compare.
    pub(crate) fn parse_with(
        bytes: &[u8],
        sst: &[Arc<str>],
        fast: bool,
    ) -> Result<Self, XlsxError> {
        let mut p = Parser::new(bytes)?;
        let start = p.start_document()?;
        if !start.head.is_main("worksheet") {
            return Err(XlsxError::NotSpreadsheet(
                "sheet part is not a worksheet".to_owned(),
            ));
        }
        let mut cx = Ctx {
            sst,
            text: String::new(),
            fmt: String::new(),
            dropped: Vec::new(),
        };
        let mut before: Vec<RawXml> = Vec::new();
        let mut after: Vec<RawXml> = Vec::new();
        let mut sheet_data: Option<(ElementHead, Vec<Row>)> = None;
        let mut pending = String::new();
        if start.empty {
            return Err(malformed("worksheet has no sheetData".to_owned()));
        }
        loop {
            let target = if sheet_data.is_some() {
                &mut after
            } else {
                &mut before
            };
            match p.next_event()? {
                Event::Start(e) => {
                    flush(&mut pending, target);
                    let head = p.enter(&e)?;
                    if head.is_main("sheetData") {
                        if sheet_data.is_some() {
                            return Err(malformed("two sheetData elements".to_owned()));
                        }
                        let scanned = if fast && head.name == "sheetData" {
                            crate::fast::scan_sheet_data(
                                &p,
                                p.position()?,
                                &head.ns,
                                cx.sst,
                                &mut cx.fmt,
                                &mut cx.text,
                            )
                        } else {
                            None
                        };
                        let rows = if let Some(scanned) = scanned {
                            p.fast_forward(scanned.end)?;
                            p.leave();
                            scanned.rows
                        } else {
                            parse_sheet_data(&mut p, &mut cx, false)?
                        };
                        sheet_data = Some((head, rows));
                    } else {
                        let mut el = XmlElement {
                            head,
                            children: Vec::new(),
                        };
                        p.read_content(&mut el)?;
                        target.push(RawXml::Element(el));
                    }
                }
                Event::Empty(e) => {
                    flush(&mut pending, target);
                    let head = p.enter(&e)?;
                    if head.is_main("sheetData") {
                        if sheet_data.is_some() {
                            return Err(malformed("two sheetData elements".to_owned()));
                        }
                        let rows = parse_sheet_data(&mut p, &mut cx, true)?;
                        sheet_data = Some((head, rows));
                    } else {
                        p.leave();
                        target.push(RawXml::Element(XmlElement {
                            head,
                            children: Vec::new(),
                        }));
                    }
                }
                Event::End(e) => {
                    check_end(e.name().as_ref(), &start.head.name)?;
                    flush(&mut pending, target);
                    p.leave();
                    break;
                }
                Event::Text(t) => pending.push_str(&t.xml10_content()),
                Event::CData(c) => pending.push_str(&c.into_inner()),
                Event::GeneralRef(r) => push_entity(&r, &mut pending)?,
                Event::Comment(c) => {
                    flush(&mut pending, target);
                    target.push(RawXml::Comment(c.to_string()));
                }
                Event::PI(pi) => {
                    flush(&mut pending, target);
                    target.push(RawXml::ProcessingInstruction(pi.to_string()));
                }
                Event::Decl(_) => {}
                Event::Eof => return Err(xml_err("unexpected end of document")),
                Event::DocType(_) => return Err(XlsxError::DoctypeForbidden),
            }
        }
        let epilogue = p.finish_document(false)?;
        let (sheet_data_head, rows) =
            sheet_data.ok_or_else(|| malformed("worksheet has no sheetData".to_owned()))?;
        let mut ws = Self {
            prolog: start.prolog,
            epilogue,
            root_head: start.head,
            before,
            sheet_data_head,
            rows,
            after,
            shared_masters: HashMap::new(),
            locked: Vec::new(),
            arrays: Vec::new(),
            tables: Vec::new(),
            dropped: cx.dropped,
        };
        ws.index_formulas();
        Ok(ws)
    }

    /// Rebuilds the shared-group and array-range indexes from the cells.
    fn index_formulas(&mut self) {
        self.shared_masters.clear();
        self.locked.clear();
        self.arrays.clear();
        for row in &self.rows {
            for cell in &row.cells {
                let Some(f) = &cell.formula else { continue };
                match &f.kind {
                    FormulaKind::Shared { si, .. } if !f.text.is_empty() => {
                        self.shared_masters.entry(*si).or_insert((row.r, cell.col));
                    }
                    FormulaKind::Array { range } => {
                        let rect = range
                            .as_deref()
                            .and_then(parse_range)
                            .unwrap_or((row.r, cell.col, row.r, cell.col));
                        self.locked.push(rect);
                        if !f.text.is_empty() {
                            self.arrays.push(ArrayFormula {
                                master: (row.r, cell.col),
                                range: rect,
                                text: f.text.clone(),
                                dynamic: f.attrs.iter().any(|a| a.ns.is_none() && a.name == "cm"),
                            });
                        }
                    }
                    FormulaKind::DataTable => {
                        let rect = f
                            .attrs
                            .iter()
                            .find(|a| a.ns.is_none() && a.name == "ref")
                            .and_then(|a| parse_range(&a.value))
                            .unwrap_or((row.r, cell.col, row.r, cell.col));
                        self.locked.push(rect);
                    }
                    _ => {}
                }
            }
        }
    }

    /// Content the reader could not represent.
    pub(crate) fn dropped(&self) -> &[String] {
        &self.dropped
    }

    /// Rows with at least a row element, in order.
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    fn find_row(&self, row: u32) -> Result<usize, usize> {
        self.rows.binary_search_by_key(&row, |r| r.r)
    }

    /// The cell at 1-based `(row, col)`.
    pub fn cell(&self, row: u32, col: u32) -> Option<&Cell> {
        let row = self.rows.get(self.find_row(row).ok()?)?;
        let i = row.cells.binary_search_by_key(&col, |c| c.col).ok()?;
        row.cells.get(i)
    }

    /// All cells in row-major order as `(row, col, cell)`.
    pub fn iter_cells(&self) -> impl Iterator<Item = (u32, u32, &Cell)> {
        self.rows
            .iter()
            .flat_map(|r| r.cells.iter().map(move |c| (r.r, c.col, c)))
    }

    /// Bounding box of the cells that exist, `(row1, col1, row2, col2)`. Computed from the
    /// cells; the `dimension` element is advisory and never trusted.
    pub fn used_range(&self) -> Option<Rect> {
        let mut rect: Option<Rect> = None;
        for row in self.rows.iter().filter(|r| !r.cells.is_empty()) {
            let (first, last) = (row.cells.first()?.col, row.cells.last()?.col);
            rect = Some(match rect {
                None => (row.r, first, row.r, last),
                Some((r1, c1, _, c2)) => (r1, c1.min(first), row.r, c2.max(last)),
            });
        }
        rect
    }

    /// The master cell `(row, col)` of shared group `si`.
    pub fn shared_master(&self, si: u32) -> Option<(u32, u32)> {
        self.shared_masters.get(&si).copied()
    }

    /// Formula text of a cell, without `=`. A shared-formula dependent is expanded by
    /// shifting the master's relative references.
    ///
    /// A non-master cell of a legacy array formula (stored with an empty `f`) has no
    /// formula of its own: it returns `None`; see [`Worksheet::array_role`].
    pub fn formula_text(&self, row: u32, col: u32) -> Option<String> {
        let f = self.cell(row, col)?.formula.as_deref()?;
        if f.text.is_empty()
            && !matches!(f.kind, FormulaKind::Shared { .. })
            && matches!(self.array_role(row, col), Some(ArrayRole::Member { .. }))
        {
            return None;
        }
        if let FormulaKind::Shared { si, .. } = &f.kind
            && f.text.is_empty()
        {
            let (mr, mc) = self.shared_master(*si)?;
            let master = self.cell(mr, mc)?.formula.as_deref()?;
            return Some(shift_formula(
                &master.text,
                i64::from(row) - i64::from(mr),
                i64::from(col) - i64::from(mc),
            ));
        }
        Some(f.text.clone())
    }

    /// Legacy array formulas (`t="array"` with a master that has text), row-major.
    pub fn array_formulas(&self) -> &[ArrayFormula] {
        &self.arrays
    }

    /// The array formula that covers `(row, col)`, if any.
    pub fn array_formula_at(&self, row: u32, col: u32) -> Option<&ArrayFormula> {
        self.arrays.iter().find(|a| {
            let (r1, c1, r2, c2) = a.range;
            (r1..=r2).contains(&row) && (c1..=c2).contains(&col)
        })
    }

    /// Whether `(row, col)` is an array formula's master, or "part of array X".
    pub fn array_role(&self, row: u32, col: u32) -> Option<ArrayRole> {
        let a = self.array_formula_at(row, col)?;
        Some(if a.master == (row, col) {
            ArrayRole::Master
        } else {
            ArrayRole::Member { master: a.master }
        })
    }

    /// How `(row, col)` relates to its shared-formula group: the master, or a dependent
    /// with its offset from the master.
    pub fn shared_formula(&self, row: u32, col: u32) -> Option<SharedFormulaRef> {
        let f = self.cell(row, col)?.formula.as_deref()?;
        let FormulaKind::Shared { si, .. } = &f.kind else {
            return None;
        };
        let (mr, mc) = self.shared_master(*si)?;
        let master = self.cell(mr, mc)?.formula.as_deref()?;
        let master_range = match &master.kind {
            FormulaKind::Shared { range, .. } => range.as_deref().and_then(parse_range),
            _ => None,
        };
        Some(SharedFormulaRef {
            si: *si,
            master: (mr, mc),
            master_text: master.text.clone(),
            master_range,
            is_master: (mr, mc) == (row, col),
            row_offset: i64::from(row) - i64::from(mr),
            col_offset: i64::from(col) - i64::from(mc),
        })
    }

    /// Tables (`xl/tables/tableN.xml`) the sheet relates to, in relationship order.
    pub fn tables(&self) -> &[Table] {
        &self.tables
    }

    /// The table whose range contains `(row, col)`.
    pub fn table_at(&self, row: u32, col: u32) -> Option<&Table> {
        self.tables.iter().find(|t| {
            t.rect.is_some_and(|(r1, c1, r2, c2)| {
                (r1..=r2).contains(&row) && (c1..=c2).contains(&col)
            })
        })
    }

    pub(crate) fn set_tables(&mut self, tables: Vec<Table>) {
        self.tables = tables;
    }

    /// Root children before `sheetData`, in order.
    pub fn raw_before(&self) -> &[RawXml] {
        &self.before
    }

    /// Root children after `sheetData`, in order.
    pub fn raw_after(&self) -> &[RawXml] {
        &self.after
    }

    /// The worksheet part serialised from the model.
    pub fn to_bytes(&self) -> Vec<u8> {
        let names = Names::new(&self.sheet_data_head);
        let mut out = String::with_capacity(self.rows.len() * 160 + 4096);
        out.push_str(&self.prolog);
        write_head(&self.root_head, false, &mut out);
        for node in &self.before {
            node.write(&mut out);
        }
        write_head(&self.sheet_data_head, self.rows.is_empty(), &mut out);
        if !self.rows.is_empty() {
            for row in &self.rows {
                let empty = row.cells.is_empty() && row.extras.is_empty();
                write_head(&row.head, empty, &mut out);
                if empty {
                    continue;
                }
                for cell in &row.cells {
                    write_cell(cell, row.r, &names, &mut out);
                }
                for extra in &row.extras {
                    extra.write(&mut out);
                }
                out.push_str("</");
                out.push_str(&names.row);
                out.push('>');
            }
            out.push_str("</");
            out.push_str(&self.sheet_data_head.name);
            out.push('>');
        }
        for node in &self.after {
            node.write(&mut out);
        }
        out.push_str("</");
        out.push_str(&self.root_head.name);
        out.push('>');
        out.push_str(&self.epilogue);
        out.into_bytes()
    }

    // ---- editing -------------------------------------------------------------------

    /// Fails when `(row, col)` lies inside an array formula or data table.
    pub(crate) fn check_editable(&self, row: u32, col: u32) -> Result<(), XlsxError> {
        if self
            .locked
            .iter()
            .any(|&(r1, c1, r2, c2)| (r1..=r2).contains(&row) && (c1..=c2).contains(&col))
        {
            return Err(XlsxError::UnsupportedEdit(format!(
                "cell {} belongs to an array formula or data table",
                crate::refs::cell_ref_string(row, col)
            )));
        }
        Ok(())
    }

    /// When `(row, col)` is the master of a shared formula, turns every dependent into a
    /// normal formula with its expanded text so nothing is lost when the master changes.
    /// Returns the number of cells materialised.
    pub(crate) fn materialise_if_master(&mut self, row: u32, col: u32) -> usize {
        let si = match self
            .cell(row, col)
            .and_then(|c| c.formula.as_deref())
            .map(|f| (&f.kind, !f.text.is_empty()))
        {
            Some((FormulaKind::Shared { si, .. }, true)) => *si,
            _ => return 0,
        };
        if self.shared_masters.get(&si) != Some(&(row, col)) {
            return 0;
        }
        let Some(master_text) = self
            .cell(row, col)
            .and_then(|c| c.formula.as_deref())
            .map(|f| f.text.clone())
        else {
            return 0;
        };
        let mut count = 0;
        for r in &mut self.rows {
            for cell in &mut r.cells {
                let Some(f) = cell.formula.as_deref_mut() else {
                    continue;
                };
                if matches!(&f.kind, FormulaKind::Shared { si: s, .. } if *s == si)
                    && f.text.is_empty()
                {
                    f.text = shift_formula(
                        &master_text,
                        i64::from(r.r) - i64::from(row),
                        i64::from(cell.col) - i64::from(col),
                    );
                    f.kind = FormulaKind::Normal;
                    count += 1;
                }
            }
        }
        self.shared_masters.remove(&si);
        count
    }

    /// Grows the advisory `dimension` to cover `(row, col)`.
    fn grow_dimension(&mut self, row: u32, col: u32) {
        let Some(dim) = self
            .before
            .iter_mut()
            .filter_map(RawXml::as_element_mut)
            .find(|e| e.head.is_main("dimension"))
        else {
            return;
        };
        let new = match dim.head.attr("ref").and_then(parse_range) {
            Some((r1, c1, r2, c2)) => (r1.min(row), c1.min(col), r2.max(row), c2.max(col)),
            None => (row, col, row, col),
        };
        let text = if new.0 == new.2 && new.1 == new.3 {
            crate::refs::cell_ref_string(new.0, new.1)
        } else {
            format!(
                "{}:{}",
                crate::refs::cell_ref_string(new.0, new.1),
                crate::refs::cell_ref_string(new.2, new.3)
            )
        };
        dim.head.set_attr("ref", &text);
    }

    /// The cell at `(row, col)`, created blank (with its row) when missing.
    pub(crate) fn cell_slot(&mut self, row: u32, col: u32) -> &mut Cell {
        let ri = match self.find_row(row) {
            Ok(i) => i,
            Err(i) => {
                let mut head = self.sheet_data_head.sibling("row");
                head.attrs.push(Attr::plain("r", &row.to_string()));
                self.rows.insert(
                    i,
                    Row {
                        r: row,
                        head,
                        cells: Vec::new(),
                        extras: Vec::new(),
                    },
                );
                i
            }
        };
        let ci = match self.rows[ri].cells.binary_search_by_key(&col, |c| c.col) {
            Ok(i) => i,
            Err(i) => {
                self.rows[ri].cells.insert(i, Cell::empty(col));
                // `spans` is an optimisation hint; drop it when the new cell is outside it.
                let head = &mut self.rows[ri].head;
                if let Some(spans) = head.attr("spans")
                    && !spans.split_whitespace().any(|s| {
                        s.split_once(':')
                            .and_then(|(a, b)| {
                                Some((a.parse::<u32>().ok()?, b.parse::<u32>().ok()?))
                            })
                            .is_some_and(|(a, b)| (a..=b).contains(&col))
                    })
                {
                    head.remove_attr("spans");
                }
                i
            }
        };
        self.grow_dimension(row, col);
        &mut self.rows[ri].cells[ci]
    }

    /// Sets a cell's value and formula, dropping representation details of the old
    /// content (raw number text, inline string, type override). Style and unknown
    /// attributes stay. Removes the cell when it ends up with no content at all.
    pub(crate) fn set_content(
        &mut self,
        row: u32,
        col: u32,
        value: CellValue,
        formula: Option<Formula>,
    ) {
        let blank = matches!(value, CellValue::Empty) && formula.is_none();
        let cell = self.cell_slot(row, col);
        let formula_changed = cell.formula.is_some() || formula.is_some();
        cell.value = value;
        cell.formula = formula.map(Box::new);
        if let Some(extra) = cell.extra.as_deref_mut() {
            extra.raw_v = None;
            extra.v_attrs.clear();
            extra.t_override = None;
            extra.inline = None;
        }
        if cell.extra.as_deref().is_some_and(|e| {
            e.attrs.is_empty()
                && e.children.is_empty()
                && e.raw_v.is_none()
                && e.v_attrs.is_empty()
                && e.inline.is_none()
        }) {
            cell.extra = None;
        }
        if blank && !cell.has_content() {
            self.remove_cell(row, col);
        }
        if formula_changed {
            self.index_formulas();
        }
    }

    fn remove_cell(&mut self, row: u32, col: u32) {
        if let Ok(ri) = self.find_row(row)
            && let Ok(ci) = self.rows[ri].cells.binary_search_by_key(&col, |c| c.col)
        {
            self.rows[ri].cells.remove(ci);
        }
    }
}

fn flush(pending: &mut String, target: &mut Vec<RawXml>) {
    if !pending.is_empty() {
        target.push(RawXml::Text(std::mem::take(pending)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_sheets() -> Vec<(String, Vec<u8>)> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/xlsx");
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir).expect("fixtures") {
            let path = entry.expect("entry").path();
            if path.extension().is_some_and(|e| e == "xlsx") {
                let bytes = std::fs::read(&path).expect("read");
                let package = open365_opc::Package::read(&bytes, &open365_opc::Limits::default())
                    .expect("zip");
                for part in package.parts() {
                    if part.name().as_str().starts_with("xl/worksheets/")
                        && part.name().as_str().ends_with(".xml")
                    {
                        out.push((
                            format!("{}:{}", path.display(), part.name()),
                            part.data().to_vec(),
                        ));
                    }
                }
            }
        }
        out
    }

    #[test]
    fn quick_scanner_and_general_reader_build_the_same_model() {
        let sheets = fixture_sheets();
        assert!(sheets.len() >= 16);
        let mut quick = 0;
        for (name, bytes) in sheets {
            // Shared strings are only looked up by index; a long stand-in table is enough.
            let sst: Vec<Arc<str>> = (0..64).map(|i| Arc::<str>::from(format!("s{i}"))).collect();
            let sst = &sst[..];
            let slow = Worksheet::parse_with(&bytes, sst, false);
            let fast = Worksheet::parse_with(&bytes, sst, true);
            match (slow, fast) {
                (Ok(slow), Ok(fast)) => {
                    assert!(slow.rows == fast.rows, "{name}: rows differ");
                    assert_eq!(slow.to_bytes(), fast.to_bytes(), "{name}");
                    quick += 1;
                }
                (Err(a), Err(b)) => assert_eq!(a.to_string(), b.to_string(), "{name}"),
                (a, b) => panic!("{name}: {:?} vs {:?}", a.map(|_| "ok"), b.map(|_| "ok")),
            }
        }
        assert!(quick >= 16);
    }
}
