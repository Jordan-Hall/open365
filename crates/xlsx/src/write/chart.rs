//! Inserting a chart: the drawing part, the chart anchor and the chart part.

use open365_opc::{PartName, TargetMode};

use super::{R_NS, XML_DECL, content_type, esc, rel, relative_target};
use crate::XlsxError;
use crate::refs::{MAX_COL, MAX_ROW};
use crate::sheet::Rect;
use crate::workbook::Workbook;
use crate::xml::{self, RawXml, XmlElement};

const XDR_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing";
const A_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const C_NS: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";

/// Where the chart sits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChartAnchor {
    /// The cells the chart covers, `(row1, col1, row2, col2)`: it runs from the top-left corner of
    /// the first cell to the bottom-right corner of the last (a `twoCellAnchor`, as Excel writes).
    pub range: Rect,
    /// The shape name; `Chart N` when absent.
    pub name: Option<String>,
}

/// What [`Workbook::insert_chart`] made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChartInsert {
    pub chart_part: String,
    pub drawing_part: String,
    /// The drawing's id for the new shape (`cNvPr id`).
    pub shape_id: u32,
    pub name: String,
    /// The drawing part was created by this call.
    pub drawing_created: bool,
}

fn max_shape_id(el: &XmlElement) -> u32 {
    let mut best = 0;
    if el.head.local_name() == "cNvPr"
        && let Some(id) = el
            .head
            .attr("id")
            .and_then(|v| v.trim().parse::<u32>().ok())
    {
        best = id;
    }
    for c in el.elements() {
        best = best.max(max_shape_id(c));
    }
    best
}

fn shape_names(el: &XmlElement, out: &mut Vec<String>) {
    if el.head.local_name() == "cNvPr"
        && let Some(n) = el.head.attr("name")
    {
        out.push(n.to_owned());
    }
    for c in el.elements() {
        shape_names(c, out);
    }
}

fn anchor_xml(prefix: &str, range: Rect, id: u32, name: &str, rid: &str) -> String {
    let (r1, c1, r2, c2) = range;
    let p = |s: &str| {
        if prefix.is_empty() {
            s.to_owned()
        } else {
            format!("{prefix}:{s}")
        }
    };
    let corner = |tag: &str, col: u32, row: u32| {
        format!(
            "<{t}><{c}>{col}</{c}><{co}>0</{co}><{r}>{row}</{r}><{ro}>0</{ro}></{t}>",
            t = p(tag),
            c = p("col"),
            co = p("colOff"),
            r = p("row"),
            ro = p("rowOff"),
        )
    };
    format!(
        "<{anchor} xmlns:{xp}=\"{XDR_NS}\" xmlns:a=\"{A_NS}\">{from}{to}<{gf} macro=\"\"><{nv}><{cnv} id=\"{id}\" name=\"{name}\"/><{cnvg}/></{nv}><{xfrm}><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/></{xfrm}><a:graphic><a:graphicData uri=\"{C_NS}\"><c:chart xmlns:c=\"{C_NS}\" xmlns:r=\"{R_NS}\" r:id=\"{rid}\"/></a:graphicData></a:graphic></{gf}><{cd}/></{anchor}>",
        anchor = p("twoCellAnchor"),
        xp = if prefix.is_empty() { "xdr" } else { prefix },
        from = corner("from", c1 - 1, r1 - 1),
        to = corner("to", c2, r2),
        gf = p("graphicFrame"),
        nv = p("nvGraphicFramePr"),
        cnv = p("cNvPr"),
        cnvg = p("cNvGraphicFramePr"),
        xfrm = p("xfrm"),
        cd = p("clientData"),
        name = esc(name),
    )
}

impl Workbook {
    /// Inserts a chart (the bytes of a `chartSpace` part) over `anchor.range` of worksheet `sheet`.
    /// The sheet's drawing part is created when it has none, else the chart is added to it. Part names,
    /// content types and relationship ids follow Excel (`xl/charts/chart1.xml`, `xl/drawings/drawing1.xml`).
    ///
    /// # Errors
    /// The sheet is not a worksheet, the range is outside the grid, or the bytes are not a chart part.
    pub fn insert_chart(
        &mut self,
        sheet: usize,
        chart_xml: &[u8],
        anchor: &ChartAnchor,
    ) -> Result<ChartInsert, XlsxError> {
        self.require_transitional()?;
        let (r1, c1, r2, c2) = anchor.range;
        if r1 == 0 || c1 == 0 || r1 > r2 || c1 > c2 || r2 > MAX_ROW || c2 > MAX_COL {
            return Err(XlsxError::UnsupportedEdit(
                "the chart range is outside the worksheet".to_owned(),
            ));
        }
        let parsed = xml::parse(chart_xml)?;
        if parsed.root.head.local_name() != "chartSpace" {
            return Err(XlsxError::UnsupportedEdit(
                "the chart part must be a chartSpace document".to_owned(),
            ));
        }
        let sheet_part = self
            .sheet_part(sheet)
            .cloned()
            .ok_or(XlsxError::NotAWorksheet(sheet))?;
        self.worksheet_mut(sheet)?;
        self.edited()?;

        // The sheet's drawing, when it has one.
        let existing: Option<PartName> = self.sheets[sheet]
            .data
            .as_ref()
            .and_then(|ws| ws.root_element("drawing"))
            .and_then(|d| d.head.attr_r("id").map(str::to_owned))
            .and_then(|id| {
                self.relationships(Some(&sheet_part))
                    .into_iter()
                    .find(|r| r.id == id)
            })
            .and_then(|r| open365_opc::resolve_target(Some(&sheet_part), &r.target).ok());

        let chart_part = self.next_part_name("xl/charts", "chart", "xml");
        let (drawing_name, created) = match &existing {
            Some(p) => (p.clone(), false),
            None => {
                let name = self.next_part_name("xl/drawings", "drawing", "xml");
                let body =
                    format!("{XML_DECL}<xdr:wsDr xmlns:xdr=\"{XDR_NS}\" xmlns:a=\"{A_NS}\"/>");
                let part = self.add_part(&name, content_type::DRAWING, body.into_bytes())?;
                (part, true)
            }
        };

        let mut doc = xml::parse(
            self.part_bytes(drawing_name.as_str())
                .ok_or_else(|| XlsxError::MissingPart(drawing_name.to_string()))?,
        )?;
        let id = max_shape_id(&doc.root).max(1) + 1;
        let mut names = Vec::new();
        shape_names(&doc.root, &mut names);
        let name = anchor.name.clone().unwrap_or_else(|| {
            (1u32..)
                .map(|n| format!("Chart {n}"))
                .find(|n| !names.contains(n))
                .unwrap_or_default()
        });

        self.add_part(&chart_part, content_type::CHART, chart_xml.to_vec())?;
        let rid = self.add_relationship(
            Some(&drawing_name),
            rel::CHART,
            &relative_target(Some(&drawing_name), &chart_part),
            TargetMode::Internal,
        )?;

        let prefix = doc
            .root
            .head
            .name
            .rsplit_once(':')
            .map_or(String::new(), |(p, _)| p.to_owned());
        let frag = crate::sheet::parse_fragment(
            &anchor_xml(&prefix, anchor.range, id, &name, &rid),
            &doc.root.head,
        )?;
        doc.root.children.push(RawXml::Element(frag));
        self.put_part_bytes(drawing_name.as_str(), doc.to_bytes())?;

        if created {
            let drid = self.add_relationship(
                Some(&sheet_part),
                rel::DRAWING,
                &relative_target(Some(&sheet_part), drawing_name.as_str()),
                TargetMode::Internal,
            )?;
            let ws = self.worksheet_mut(sheet)?;
            let el = ws.fragment(&format!(
                "<drawing xmlns=\"{}\" xmlns:r=\"{R_NS}\" r:id=\"{drid}\"/>",
                super::MAIN
            ))?;
            ws.put_root_element("drawing", Some(el));
            self.sheets[sheet].dirty = true;
        }
        Ok(ChartInsert {
            chart_part,
            drawing_part: drawing_name.to_string(),
            shape_id: id,
            name,
            drawing_created: created,
        })
    }
}
