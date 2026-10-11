//! Read-only access to the chart objects a worksheet's drawing part anchors.
//!
//! Nothing here edits a part: the drawing and chart parts stay byte-identical in the package
//! (ADR-001). [`Workbook::sheet_charts`] lists, in drawing order, every `graphicFrame` that
//! points at a chart part, with the anchor that fixes its position and size on the sheet.
//! [`Anchor::rect_pt`] turns an anchor into a rectangle in points, taking column widths and row
//! heights from the sheet the way Excel does (`Shape.Width` of a move-and-size chart).

use open365_opc::{PartName, Relationships, rels_part_name, resolve_target};

use crate::geometry::GridMetrics;
use crate::sheet::Worksheet;
use crate::workbook::Workbook;
use crate::xml::{self, XmlElement};

const XDR_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing";
const CHART_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";
const DRAWING_REL: &str = "/relationships/drawing";
const CHART_REL: &str = "/relationships/chart";

/// English Metric Units per point.
pub const EMU_PER_POINT: f64 = 12_700.0;

/// A cell corner plus an offset into the cell (`xdr:from` / `xdr:to`). Column and row are
/// 0-based as stored; offsets are in EMU.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AnchorMarker {
    pub col: u32,
    pub col_off: i64,
    pub row: u32,
    pub row_off: i64,
}

/// How an anchored object follows its cells (`editAs` of a two-cell anchor).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EditAs {
    /// Moves and resizes with the cells (the default).
    #[default]
    TwoCell,
    OneCell,
    Absolute,
}

/// The anchor of a drawing object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    TwoCell {
        from: AnchorMarker,
        to: AnchorMarker,
        edit_as: EditAs,
    },
    /// Top-left cell plus an extent in EMU.
    OneCell {
        from: AnchorMarker,
        cx: i64,
        cy: i64,
    },
    /// Position and extent in EMU from the sheet origin.
    Absolute { x: i64, y: i64, cx: i64, cy: i64 },
}

/// A rectangle in points from the sheet origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RectPt {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// One chart object of a worksheet's drawing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrawingChart {
    /// `xdr:cNvPr/@name`, empty when absent.
    pub name: String,
    pub anchor: Anchor,
    /// The chart part (`xl/charts/chartN.xml`).
    pub chart_part: PartName,
    pub drawing_part: PartName,
}

fn emu(text: Option<&str>) -> i64 {
    text.and_then(|t| t.trim().parse().ok()).unwrap_or(0)
}

fn is_xdr(e: &XmlElement, local: &str) -> bool {
    e.head.ns.as_deref() == Some(XDR_NS) && e.head.local_name() == local
}

fn xdr_child<'a>(e: &'a XmlElement, local: &str) -> Option<&'a XmlElement> {
    e.elements().find(|c| is_xdr(c, local))
}

fn marker(e: &XmlElement) -> AnchorMarker {
    let num = |local: &str| xdr_child(e, local).map(XmlElement::own_text);
    let uint = |local: &str| {
        num(local)
            .and_then(|t| t.trim().parse::<u32>().ok())
            .unwrap_or(0)
    };
    let off = |local: &str| emu(num(local).as_deref());
    AnchorMarker {
        col: uint("col"),
        col_off: off("colOff"),
        row: uint("row"),
        row_off: off("rowOff"),
    }
}

fn ext(e: &XmlElement) -> (i64, i64) {
    e.elements()
        .find(|c| c.head.local_name() == "ext")
        .map_or((0, 0), |x| (emu(x.head.attr("cx")), emu(x.head.attr("cy"))))
}

/// The chart relationship id a `graphicFrame` refers to, found anywhere below it.
fn chart_rid(e: &XmlElement) -> Option<&str> {
    for child in e.elements() {
        if child.head.ns.as_deref() == Some(CHART_NS)
            && child.head.local_name() == "chart"
            && let Some(id) = child.head.attr_r("id")
        {
            return Some(id);
        }
        if let Some(id) = chart_rid(child) {
            return Some(id);
        }
    }
    None
}

fn frame_name(e: &XmlElement) -> String {
    fn find(e: &XmlElement) -> Option<String> {
        for child in e.elements() {
            if child.head.local_name() == "cNvPr" {
                return Some(child.head.attr("name").unwrap_or_default().to_owned());
            }
            if child.head.local_name() == "nvGraphicFramePr"
                && let Some(n) = find(child)
            {
                return Some(n);
            }
        }
        None
    }
    find(e).unwrap_or_default()
}

fn parse_anchor(e: &XmlElement) -> Option<Anchor> {
    match e.head.local_name() {
        "twoCellAnchor" => {
            let edit_as = match e.head.attr("editAs") {
                Some("oneCell") => EditAs::OneCell,
                Some("absolute") => EditAs::Absolute,
                _ => EditAs::TwoCell,
            };
            Some(Anchor::TwoCell {
                from: marker(xdr_child(e, "from")?),
                to: marker(xdr_child(e, "to")?),
                edit_as,
            })
        }
        "oneCellAnchor" => {
            let (cx, cy) = ext(e);
            Some(Anchor::OneCell {
                from: marker(xdr_child(e, "from")?),
                cx,
                cy,
            })
        }
        "absoluteAnchor" => {
            let pos = xdr_child(e, "pos")?;
            let (cx, cy) = ext(e);
            Some(Anchor::Absolute {
                x: emu(pos.head.attr("x")),
                y: emu(pos.head.attr("y")),
                cx,
                cy,
            })
        }
        _ => None,
    }
}

/// Parses a drawing part and resolves its chart relationships. `rels` are the drawing's own.
fn charts_in_drawing(drawing: &PartName, bytes: &[u8], rels: &Relationships) -> Vec<DrawingChart> {
    let Ok(doc) = xml::parse(bytes) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for anchor_el in doc.root.elements() {
        let Some(anchor) = parse_anchor(anchor_el) else {
            continue;
        };
        for frame in anchor_el.elements().filter(|c| is_xdr(c, "graphicFrame")) {
            let Some(rid) = chart_rid(frame) else {
                continue;
            };
            let Some(rel) = rels.get(rid) else { continue };
            if !rel.rel_type.ends_with(CHART_REL) {
                continue;
            }
            let Ok(part) = resolve_target(Some(drawing), &rel.target) else {
                continue;
            };
            out.push(DrawingChart {
                name: frame_name(frame),
                anchor,
                chart_part: part,
                drawing_part: drawing.clone(),
            });
        }
    }
    out
}

impl Workbook {
    /// The charts anchored on worksheet `sheet`, in drawing order. Empty for a sheet without
    /// a drawing, a chart sheet, or a drawing that does not parse.
    pub fn sheet_charts(&self, sheet: usize) -> Vec<DrawingChart> {
        let Some(sheet_part) = self.sheet_part(sheet) else {
            return Vec::new();
        };
        let package = self.package();
        let Some(rels) = package
            .part(rels_part_name(Some(sheet_part)).as_str())
            .and_then(|p| Relationships::parse(p.data()).ok())
        else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for rel in rels.iter().filter(|r| r.rel_type.ends_with(DRAWING_REL)) {
            let Ok(drawing) = resolve_target(Some(sheet_part), &rel.target) else {
                continue;
            };
            let Some(part) = package.part(drawing.as_str()) else {
                continue;
            };
            let drawing_rels = package
                .part(rels_part_name(Some(&drawing)).as_str())
                .and_then(|p| Relationships::parse(p.data()).ok())
                .unwrap_or_default();
            out.extend(charts_in_drawing(&drawing, part.data(), &drawing_rels));
        }
        out
    }
}

/// Top edge of 0-based `row` from the sheet origin in points: the default height for every
/// row, corrected for the rows that carry their own height or are hidden.
fn row_top_pt(ws: &Worksheet, row: u32) -> f64 {
    let format = ws.sheet_format();
    let default = if format.zero_height {
        0.0
    } else {
        format.default_row_height
    };
    let mut top = f64::from(row) * default;
    for r in ws.rows().iter().take_while(|r| r.r <= row) {
        let own = if r.hidden() {
            0.0
        } else {
            r.height().unwrap_or(if format.zero_height {
                format.default_row_height
            } else {
                default
            })
        };
        top += own - default;
    }
    top
}

impl Anchor {
    /// The anchored rectangle in points. Columns follow Excel's pixel quantisation (`metrics`
    /// at 96 dpi gives a default column of 48 pt); rows use their height in points.
    pub fn rect_pt(&self, ws: &Worksheet, metrics: GridMetrics) -> RectPt {
        let geometry = ws.geometry(metrics);
        let col_x = |m: &AnchorMarker| {
            metrics.px_to_pt(geometry.col_x(m.col + 1) as u32) + m.col_off as f64 / EMU_PER_POINT
        };
        let row_y = |m: &AnchorMarker| row_top_pt(ws, m.row) + m.row_off as f64 / EMU_PER_POINT;
        match *self {
            Self::TwoCell { from, to, .. } => {
                let (x0, y0) = (col_x(&from), row_y(&from));
                RectPt {
                    x: x0,
                    y: y0,
                    width: (col_x(&to) - x0).max(0.0),
                    height: (row_y(&to) - y0).max(0.0),
                }
            }
            Self::OneCell { from, cx, cy } => RectPt {
                x: col_x(&from),
                y: row_y(&from),
                width: cx as f64 / EMU_PER_POINT,
                height: cy as f64 / EMU_PER_POINT,
            },
            Self::Absolute { x, y, cx, cy } => RectPt {
                x: x as f64 / EMU_PER_POINT,
                y: y as f64 / EMU_PER_POINT,
                width: cx as f64 / EMU_PER_POINT,
                height: cy as f64 / EMU_PER_POINT,
            },
        }
    }

    /// Width and height in points.
    pub fn size_pt(&self, ws: &Worksheet, metrics: GridMetrics) -> (f64, f64) {
        let r = self.rect_pt(ws, metrics);
        (r.width, r.height)
    }
}
