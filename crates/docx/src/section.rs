//! Section properties (`w:sectPr`, ECMA-376 Part 1 §17.6.17) as typed values.
//!
//! A section's `w:sectPr` sits in the `w:pPr` of the paragraph that ends it; the last
//! section's `w:sectPr` is the final child of `w:body`. Each `w:sectPr` is read on its own:
//! an absent child gets the default stated for it here, not the previous section's value.

use crate::xml::{RawXml, XmlElement};

/// Page orientation (`w:pgSz` `w:orient`, §17.6.13). Default portrait. The width and height
/// are the physical page size; they are not swapped by the orientation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    Portrait,
    Landscape,
}

/// Page size (`w:pgSz`, §17.6.13). Absent `w:w`/`w:h` default to 12240 and 15840 twips
/// (US Letter), and an absent `w:pgSz` element means the same.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageSize {
    pub width_twips: u32,
    pub height_twips: u32,
    pub orient: Orientation,
    /// `w:code`: printer paper code, when present.
    pub code: Option<u32>,
}

impl PageSize {
    pub fn width_pt(&self) -> f32 {
        self.width_twips as f32 / 20.0
    }

    pub fn height_pt(&self) -> f32 {
        self.height_twips as f32 / 20.0
    }
}

impl Default for PageSize {
    fn default() -> Self {
        Self {
            width_twips: 12240,
            height_twips: 15840,
            orient: Orientation::Portrait,
            code: None,
        }
    }
}

/// Page margins (`w:pgMar`, §17.6.11), in twips. ECMA-376 makes every attribute required
/// and defines no value for an absent one; absent values use the margins of Word's blank
/// document: 1 inch for the four sides, 0.5 inch for header and footer, no gutter. Top and
/// bottom may be negative (the page then keeps the text area fixed regardless of header).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageMargins {
    pub top_twips: i32,
    pub right_twips: i32,
    pub bottom_twips: i32,
    pub left_twips: i32,
    /// Distance from the page top to the header (`w:header`).
    pub header_twips: i32,
    /// Distance from the page bottom to the footer (`w:footer`).
    pub footer_twips: i32,
    pub gutter_twips: i32,
}

impl Default for PageMargins {
    fn default() -> Self {
        Self {
            top_twips: 1440,
            right_twips: 1440,
            bottom_twips: 1440,
            left_twips: 1440,
            header_twips: 720,
            footer_twips: 720,
            gutter_twips: 0,
        }
    }
}

/// One explicit column (`w:col`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Column {
    pub width_twips: u32,
    /// Space after this column, when given.
    pub space_twips: Option<u32>,
}

/// Columns (`w:cols`, §17.6.4). Defaults: one column, 720 twips between columns, no
/// separator line, equal widths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Columns {
    /// `w:num`, default 1.
    pub count: u32,
    /// `w:space`, default 720 twips.
    pub space_twips: u32,
    /// `w:sep`, default false.
    pub separator: bool,
    /// `w:equalWidth`, default true.
    pub equal_width: bool,
    /// Explicit columns, used when `equal_width` is false.
    pub columns: Vec<Column>,
}

impl Default for Columns {
    fn default() -> Self {
        Self {
            count: 1,
            space_twips: 720,
            separator: false,
            equal_width: true,
            columns: Vec::new(),
        }
    }
}

/// How a section starts (`w:type`, §17.6.22). Default next page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionType {
    NextPage,
    Continuous,
    NextColumn,
    EvenPage,
    OddPage,
}

/// Document grid type (`w:docGrid` `w:type`, §17.6.5). Default no grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocGridType {
    Default,
    Lines,
    LinesAndChars,
    SnapToChars,
}

/// Document grid (`w:docGrid`, §17.6.5). Absent element: no grid, no pitches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DocGrid {
    pub kind: DocGridType,
    /// `w:linePitch`, twips.
    pub line_pitch_twips: Option<u32>,
    /// `w:charSpace`, default 0.
    pub char_space: i32,
}

impl Default for DocGrid {
    fn default() -> Self {
        Self {
            kind: DocGridType::Default,
            line_pitch_twips: None,
            char_space: 0,
        }
    }
}

/// One section of the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// Zero-based section number in document order.
    pub index: usize,
    /// Index (document paragraph order, as [`crate::Document::paragraph`]) of the paragraph
    /// whose `w:pPr` carries this section's `w:sectPr`. `None` for the last section, which
    /// ends with the body.
    pub end_paragraph: Option<usize>,
    pub page: PageSize,
    pub margins: PageMargins,
    pub columns: Columns,
    /// `w:titlePg` (§17.10.6): the first page uses its own header and footer. Default false.
    pub title_pg: bool,
    pub kind: SectionType,
    pub doc_grid: DocGrid,
}

impl Default for Section {
    fn default() -> Self {
        Self {
            index: 0,
            end_paragraph: None,
            page: PageSize::default(),
            margins: PageMargins::default(),
            columns: Columns::default(),
            title_pg: false,
            kind: SectionType::NextPage,
            doc_grid: DocGrid::default(),
        }
    }
}

fn int(e: &XmlElement, attr: &str) -> Option<i32> {
    e.head
        .attr_w(attr)
        .and_then(|v| v.trim().parse::<i32>().ok())
}

fn uint(e: &XmlElement, attr: &str) -> Option<u32> {
    e.head
        .attr_w(attr)
        .and_then(|v| v.trim().parse::<u32>().ok())
}

fn on(e: &XmlElement, attr: &str) -> Option<bool> {
    e.head
        .attr_w(attr)
        .map(|v| !matches!(v, "0" | "false" | "off"))
}

impl Section {
    /// Reads one `w:sectPr` element.
    pub fn from_sect_pr(sect: &XmlElement, index: usize, end_paragraph: Option<usize>) -> Self {
        let mut out = Self {
            index,
            end_paragraph,
            ..Self::default()
        };
        for c in sect.children.iter().filter_map(RawXml::as_element) {
            match c.head.local_name() {
                "pgSz" if c.head.is_w("pgSz") => {
                    out.page.width_twips = uint(c, "w").unwrap_or(out.page.width_twips);
                    out.page.height_twips = uint(c, "h").unwrap_or(out.page.height_twips);
                    out.page.orient = if c.head.attr_w("orient") == Some("landscape") {
                        Orientation::Landscape
                    } else {
                        Orientation::Portrait
                    };
                    out.page.code = uint(c, "code");
                }
                "pgMar" if c.head.is_w("pgMar") => {
                    let m = &mut out.margins;
                    m.top_twips = int(c, "top").unwrap_or(m.top_twips);
                    m.right_twips = int(c, "right").unwrap_or(m.right_twips);
                    m.bottom_twips = int(c, "bottom").unwrap_or(m.bottom_twips);
                    m.left_twips = int(c, "left").unwrap_or(m.left_twips);
                    m.header_twips = int(c, "header").unwrap_or(m.header_twips);
                    m.footer_twips = int(c, "footer").unwrap_or(m.footer_twips);
                    m.gutter_twips = int(c, "gutter").unwrap_or(m.gutter_twips);
                }
                "cols" if c.head.is_w("cols") => {
                    let cols = &mut out.columns;
                    cols.count = uint(c, "num").filter(|n| *n > 0).unwrap_or(cols.count);
                    cols.space_twips = uint(c, "space").unwrap_or(cols.space_twips);
                    cols.separator = on(c, "sep").unwrap_or(cols.separator);
                    cols.equal_width = on(c, "equalWidth").unwrap_or(cols.equal_width);
                    cols.columns = c
                        .children
                        .iter()
                        .filter_map(RawXml::as_element)
                        .filter(|e| e.head.is_w("col"))
                        .filter_map(|e| {
                            Some(Column {
                                width_twips: uint(e, "w")?,
                                space_twips: uint(e, "space"),
                            })
                        })
                        .collect();
                }
                "titlePg" if c.head.is_w("titlePg") => {
                    out.title_pg = on(c, "val").unwrap_or(true);
                }
                "type" if c.head.is_w("type") => {
                    out.kind = match c.head.attr_w("val") {
                        Some("continuous") => SectionType::Continuous,
                        Some("nextColumn") => SectionType::NextColumn,
                        Some("evenPage") => SectionType::EvenPage,
                        Some("oddPage") => SectionType::OddPage,
                        _ => SectionType::NextPage,
                    };
                }
                "docGrid" if c.head.is_w("docGrid") => {
                    out.doc_grid = DocGrid {
                        kind: match c.head.attr_w("type") {
                            Some("lines") => DocGridType::Lines,
                            Some("linesAndChars") => DocGridType::LinesAndChars,
                            Some("snapToChars") => DocGridType::SnapToChars,
                            _ => DocGridType::Default,
                        },
                        line_pitch_twips: uint(c, "linePitch"),
                        char_space: int(c, "charSpace").unwrap_or(0),
                    };
                }
                _ => {}
            }
        }
        out
    }
}
