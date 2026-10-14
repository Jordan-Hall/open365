//! Pictures, shapes and text boxes on a worksheet: insert at a cell, move, resize, rotate, fill,
//! outline, edit text, reorder and delete, and keep anchored objects where Excel keeps them when the
//! cells beneath change (see [`super::drawing_shift`]).
//!
//! Part names, relationship ids and element shapes follow what Excel writes (`xl/media/image1.png`,
//! `xl/drawings/drawing1.xml`, `rId1`). Every function leaves objects it does not touch byte for byte
//! as they were (the drawing part is parsed to a tree, only the edited anchor changes).

use open365_drawing::catalog;
use open365_drawing::image::{self, ImageFormat};
use open365_opc::{PartName, TargetMode};

use super::super::{MAIN, R_NS, XML_DECL, content_type, rel, relative_target};
use super::drawing_shift::Place;
use super::drawing_xml::{self as dx, A_NS, Body, Names, XDR_NS};
use crate::XlsxError;
use crate::drawing::{Anchor, AnchorMarker, EMU_PER_POINT, EditAs, RectPt};
use crate::geometry::{BASE_DPI, GridMetrics};
use crate::parts;
use crate::refs::{MAX_COL, MAX_ROW};
use crate::workbook::Workbook;
use crate::xml::{self, ParsedXml, RawXml, XmlElement};

const IMAGE_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
/// Width of the PNG made beside an inserted SVG, in pixels along the longer side.
const SVG_FALLBACK_SIDE: u32 = 512;

/// Size of a shape inserted with a click and no drag, in points. Not measured yet: a click needs
/// pointer input the COM oracle cannot send. Kept in one place so a measurement replaces it.
pub const CLICK_SHAPE_SIZE_PT: (f64, f64) = (72.0, 72.0);
/// Size of a text box inserted with a click, in points. Not measured yet (see above).
pub const CLICK_TEXT_BOX_SIZE_PT: (f64, f64) = (72.0, 21.6);

fn bad(message: impl Into<String>) -> XlsxError {
    XlsxError::UnsupportedEdit(message.into())
}

pub(super) fn is_anchor(el: &XmlElement) -> bool {
    matches!(
        el.head.local_name(),
        "twoCellAnchor" | "oneCellAnchor" | "absoluteAnchor"
    )
}

/// `cNvPr` anywhere below `el`, first in document order.
pub(super) fn c_nv_pr(el: &XmlElement) -> Option<&XmlElement> {
    for c in el.elements() {
        if c.head.local_name() == "cNvPr" {
            return Some(c);
        }
        if let Some(found) = c_nv_pr(c) {
            return Some(found);
        }
    }
    None
}

pub(super) fn c_nv_pr_mut(el: &mut XmlElement) -> Option<&mut XmlElement> {
    for c in el.children.iter_mut().filter_map(RawXml::as_element_mut) {
        if c.head.local_name() == "cNvPr" {
            return Some(c);
        }
        if let Some(found) = c_nv_pr_mut(c) {
            return Some(found);
        }
    }
    None
}

pub(super) fn object_id(anchor: &XmlElement) -> Option<u32> {
    c_nv_pr(anchor)?.head.attr("id")?.trim().parse().ok()
}

pub(super) fn max_shape_id(el: &XmlElement) -> u32 {
    let own = (el.head.local_name() == "cNvPr")
        .then(|| {
            el.head
                .attr("id")
                .and_then(|v| v.trim().parse::<u32>().ok())
        })
        .flatten()
        .unwrap_or(0);
    el.elements().map(max_shape_id).fold(own, u32::max)
}

pub(super) fn shape_names(el: &XmlElement, out: &mut Vec<String>) {
    if el.head.local_name() == "cNvPr"
        && let Some(n) = el.head.attr("name")
    {
        out.push(n.to_owned());
    }
    for c in el.elements() {
        shape_names(c, out);
    }
}

/// A name `stem N` unused in the drawing, N starting at `id - 1` as Excel numbers them.
pub(super) fn fresh_name(stem: &str, id: u32, taken: &[String]) -> String {
    (id.saturating_sub(1).max(1)..)
        .map(|n| format!("{stem} {n}"))
        .find(|n| !taken.contains(n))
        .unwrap_or_default()
}

/// Adds the namespace declaration of `a:` to the first tag of a snippet, so it parses alone.
pub(super) fn with_a_ns(snippet: &str) -> String {
    let end = snippet.find([' ', '>', '/']).unwrap_or(snippet.len());
    format!("{} xmlns:a=\"{A_NS}\"{}", &snippet[..end], &snippet[end..])
}

pub(super) fn fragment(snippet: &str, root: &xml::ElementHead) -> Result<XmlElement, XlsxError> {
    crate::sheet::parse_fragment(&with_a_ns(snippet), root)
}

/// Writes `value` as the text of the child `local` of a marker element, creating it when missing.
pub(super) fn put_text(el: &mut XmlElement, local: &str, value: i64) {
    if let Some(c) = el
        .children
        .iter_mut()
        .filter_map(RawXml::as_element_mut)
        .find(|c| c.head.local_name() == local)
    {
        c.children = vec![RawXml::Text(value.to_string())];
    } else {
        let mut c = el.sibling(local);
        c.children = vec![RawXml::Text(value.to_string())];
        el.children.push(RawXml::Element(c));
    }
}

pub(super) fn write_marker(el: &mut XmlElement, m: &AnchorMarker) {
    put_text(el, "col", i64::from(m.col));
    put_text(el, "colOff", m.col_off);
    put_text(el, "row", i64::from(m.row));
    put_text(el, "rowOff", m.row_off);
}

pub(super) fn child_mut<'a>(el: &'a mut XmlElement, local: &str) -> Option<&'a mut XmlElement> {
    el.children
        .iter_mut()
        .filter_map(RawXml::as_element_mut)
        .find(|c| c.head.local_name() == local)
}

/// Rewrites the markers or position of an anchor element for `anchor`, in place.
pub(super) fn write_anchor(el: &mut XmlElement, anchor: &Anchor) {
    match *anchor {
        Anchor::TwoCell { from, to, edit_as } => {
            if let Some(f) = child_mut(el, "from") {
                write_marker(f, &from);
            }
            if let Some(t) = child_mut(el, "to") {
                write_marker(t, &to);
            }
            match edit_as {
                EditAs::TwoCell => el.head.remove_attr("editAs"),
                EditAs::OneCell => el.head.set_attr("editAs", "oneCell"),
                EditAs::Absolute => el.head.set_attr("editAs", "absolute"),
            }
        }
        Anchor::OneCell { from, cx, cy } => {
            if let Some(f) = child_mut(el, "from") {
                write_marker(f, &from);
            }
            if let Some(e) = child_mut(el, "ext") {
                e.head.set_attr("cx", &cx.to_string());
                e.head.set_attr("cy", &cy.to_string());
            }
        }
        Anchor::Absolute { x, y, cx, cy } => {
            if let Some(p) = child_mut(el, "pos") {
                p.head.set_attr("x", &x.to_string());
                p.head.set_attr("y", &y.to_string());
            }
            if let Some(e) = child_mut(el, "ext") {
                e.head.set_attr("cx", &cx.to_string());
                e.head.set_attr("cy", &cy.to_string());
            }
        }
    }
}

/// The `a:xfrm` of a shape, picture or connector directly below the anchor.
pub(super) fn shape_xfrm_mut(anchor: &mut XmlElement) -> Option<&mut XmlElement> {
    let obj = anchor
        .children
        .iter_mut()
        .filter_map(RawXml::as_element_mut)
        .find(|c| matches!(c.head.local_name(), "sp" | "pic" | "cxnSp"))?;
    let sp_pr = child_mut(obj, "spPr")?;
    child_mut(sp_pr, "xfrm")
}

pub(super) fn sp_pr_mut(anchor: &mut XmlElement) -> Option<&mut XmlElement> {
    let obj = anchor
        .children
        .iter_mut()
        .filter_map(RawXml::as_element_mut)
        .find(|c| matches!(c.head.local_name(), "sp" | "pic" | "cxnSp"))?;
    child_mut(obj, "spPr")
}

/// Points to EMU.
pub(super) fn emu(pt: f64) -> i64 {
    (pt * EMU_PER_POINT).round() as i64
}

/// Sets `a:off` and `a:ext` of an `a:xfrm` to the rectangle (points).
pub(super) fn write_xfrm(xfrm: &mut XmlElement, r: &RectPt) {
    if let Some(o) = child_mut(xfrm, "off") {
        o.head.set_attr("x", &emu(r.x).to_string());
        o.head.set_attr("y", &emu(r.y).to_string());
    }
    if let Some(e) = child_mut(xfrm, "ext") {
        e.head.set_attr("cx", &emu(r.width).to_string());
        e.head.set_attr("cy", &emu(r.height).to_string());
    }
}

/// A drawing part open for editing.
pub(super) struct Open {
    pub(super) part: PartName,
    pub(super) doc: ParsedXml,
}

/// What a snapshot keeps of one anchored object before the cells change.
#[derive(Debug, Clone)]
pub(crate) struct Item {
    /// Index among the children of `wsDr`.
    pub(super) index: usize,
    pub(super) anchor: Anchor,
    pub(super) rect: RectPt,
    /// The exact size in points when the object stores one (`a:xfrm/a:ext`).
    pub(super) size: Option<(f64, f64)>,
}

/// The anchored objects of a sheet before a change, see [`Workbook::drawing_snapshot`].
#[derive(Debug, Clone)]
pub(crate) struct Snapshot {
    pub(super) part: PartName,
    pub(super) items: Vec<Item>,
}

pub(super) fn stored_size(anchor: &XmlElement) -> Option<(f64, f64)> {
    let obj = anchor
        .elements()
        .find(|c| matches!(c.head.local_name(), "sp" | "pic" | "cxnSp"))?;
    let ext = obj
        .elements()
        .find(|c| c.head.local_name() == "spPr")?
        .elements()
        .find(|c| c.head.local_name() == "xfrm")?
        .elements()
        .find(|c| c.head.local_name() == "ext")?;
    let cx: i64 = ext.head.attr("cx")?.trim().parse().ok()?;
    let cy: i64 = ext.head.attr("cy")?.trim().parse().ok()?;
    (cx > 0 || cy > 0).then_some((cx as f64 / EMU_PER_POINT, cy as f64 / EMU_PER_POINT))
}

impl Workbook {
    pub(super) fn drawing_metrics(&self) -> GridMetrics {
        GridMetrics::for_styles(self.styles(), BASE_DPI)
    }

    /// The sheet's drawing part, when it has one.
    pub(super) fn existing_drawing(&self, sheet: usize) -> Option<PartName> {
        self.sheet_drawing_parts(sheet).into_iter().next()
    }

    /// The sheet's drawing part, made when the sheet has none.
    pub(super) fn ensure_drawing(&mut self, sheet: usize) -> Result<(PartName, bool), XlsxError> {
        if let Some(p) = self.existing_drawing(sheet) {
            return Ok((p, false));
        }
        let sheet_part = self
            .sheet_part(sheet)
            .cloned()
            .ok_or(XlsxError::NotAWorksheet(sheet))?;
        let name = self.next_part_name("xl/drawings", "drawing", "xml");
        let body = format!("{XML_DECL}<xdr:wsDr xmlns:xdr=\"{XDR_NS}\" xmlns:a=\"{A_NS}\"/>");
        let part = self.add_part(&name, content_type::DRAWING, body.into_bytes())?;
        let drid = self.add_relationship(
            Some(&sheet_part),
            rel::DRAWING,
            &relative_target(Some(&sheet_part), part.as_str()),
            TargetMode::Internal,
        )?;
        let ws = self.worksheet_mut(sheet)?;
        let el = ws.fragment(&format!(
            "<drawing xmlns=\"{MAIN}\" xmlns:r=\"{R_NS}\" r:id=\"{drid}\"/>"
        ))?;
        ws.put_root_element("drawing", Some(el));
        self.sheets[sheet].dirty = true;
        Ok((part, true))
    }

    pub(super) fn open_drawing(&self, part: &PartName) -> Result<Open, XlsxError> {
        let bytes = self
            .part_bytes(part.as_str())
            .ok_or_else(|| XlsxError::MissingPart(part.to_string()))?;
        Ok(Open {
            part: part.clone(),
            doc: xml::parse(bytes)?,
        })
    }

    pub(super) fn save_drawing(&mut self, open: Open) -> Result<(), XlsxError> {
        self.put_part_bytes(open.part.as_str(), open.doc.to_bytes())
    }

    /// Opens the drawing holding object `id` of `sheet` and the index of its anchor.
    pub(super) fn locate(&self, sheet: usize, id: u32) -> Result<(Open, usize), XlsxError> {
        self.worksheet_checked(sheet)?;
        let part = self
            .existing_drawing(sheet)
            .ok_or_else(|| bad("the sheet has no drawing objects"))?;
        let open = self.open_drawing(&part)?;
        let index = open
            .doc
            .root
            .children
            .iter()
            .position(|c| {
                c.as_element()
                    .is_some_and(|e| is_anchor(e) && object_id(e) == Some(id))
            })
            .ok_or_else(|| bad(format!("the sheet has no object with id {id}")))?;
        Ok((open, index))
    }

    pub(super) fn worksheet_checked(&self, sheet: usize) -> Result<(), XlsxError> {
        self.sheet(sheet)
            .map(|_| ())
            .ok_or(XlsxError::NotAWorksheet(sheet))
    }

    pub(super) fn anchor_prefix(doc: &ParsedXml) -> String {
        doc.root
            .head
            .name
            .rsplit_once(':')
            .map_or(String::new(), |(p, _)| p.to_owned())
    }

    /// The part name `xl/media/imageN.ext` with the lowest N unused by any extension.
    fn next_media_name(&self, ext: &str) -> String {
        (1u32..)
            .map(|n| (n, format!("xl/media/image{n}.{ext}")))
            .find(|(n, _)| {
                !self.part_names().iter().any(|p| {
                    p.trim_start_matches('/')
                        .starts_with(&format!("xl/media/image{n}."))
                })
            })
            .map(|(_, name)| name)
            .unwrap_or_default()
    }

    pub(super) fn add_media(
        &mut self,
        ext: &str,
        content_type: &str,
        bytes: Vec<u8>,
    ) -> Result<String, XlsxError> {
        let name = self.next_media_name(ext);
        self.package.set_part(PartName::new(&name)?, bytes);
        parts::add_default(&mut self.package, ext, content_type)?;
        self.parts_dirty = true;
        Ok(name)
    }

    fn check_cell(row: u32, col: u32) -> Result<(), XlsxError> {
        if row == 0 || col == 0 || row > MAX_ROW || col > MAX_COL {
            return Err(bad("the cell is outside the worksheet"));
        }
        Ok(())
    }
}

/// How a new object is placed: markers and absolute position for the cell and size.
pub(super) fn placement(
    place: &Place,
    row: u32,
    col: u32,
    ext: (i64, i64),
) -> (AnchorMarker, AnchorMarker, (i64, i64)) {
    let from = AnchorMarker {
        col: col - 1,
        col_off: 0,
        row: row - 1,
        row_off: 0,
    };
    let (x, y) = (place.x_of(&from), place.y_of(&from));
    let to = place.marker_at(
        x + ext.0 as f64 / EMU_PER_POINT,
        y + ext.1 as f64 / EMU_PER_POINT,
    );
    (from, to, (emu(x), emu(y)))
}

impl Workbook {
    /// Insert > Pictures: places a picture file (PNG, JPEG, GIF, BMP or SVG) with its top-left corner
    /// at cell `(row, col)` (1-based), at the natural size Excel gives it (pixels over the file's
    /// resolution). The anchor is a move-but-do-not-size two-cell anchor, as Excel writes for a picture.
    /// An SVG is stored with a PNG fallback beside it. Returns the object id (`cNvPr id`).
    ///
    /// # Errors
    /// The sheet is not a worksheet, the cell is outside the grid, or the bytes are not a picture.
    pub fn insert_picture(
        &mut self,
        sheet: usize,
        row: u32,
        col: u32,
        data: &[u8],
        name: Option<&str>,
    ) -> Result<u32, XlsxError> {
        self.require_transitional()?;
        Self::check_cell(row, col)?;
        self.worksheet_checked(sheet)?;
        let info = image::sniff(data).map_err(|e| bad(e.to_string()))?;
        let ext = info.natural_emu();
        let (raster, raster_ext, svg) = if info.format == ImageFormat::Svg {
            let png = image::svg_to_png(data, SVG_FALLBACK_SIDE).map_err(|e| bad(e.to_string()))?;
            (png, ImageFormat::Png, Some(data))
        } else {
            (data.to_vec(), info.format, None)
        };
        self.edited()?;
        let (drawing, _) = self.ensure_drawing(sheet)?;
        let mut open = self.open_drawing(&drawing)?;
        let prefix = Self::anchor_prefix(&open.doc);
        let id = max_shape_id(&open.doc.root).max(1) + 1;
        let mut taken = Vec::new();
        shape_names(&open.doc.root, &mut taken);
        let stem = if svg.is_some() { "Graphic" } else { "Picture" };
        let name = name.map_or_else(|| fresh_name(stem, id, &taken), str::to_owned);

        let media = self.add_media(raster_ext.extension(), raster_ext.content_type(), raster)?;
        let embed = self.add_relationship(
            Some(&drawing),
            IMAGE_REL,
            &relative_target(Some(&drawing), &media),
            TargetMode::Internal,
        )?;
        let svg_rid = match svg {
            Some(bytes) => {
                let part =
                    self.add_media("svg", ImageFormat::Svg.content_type(), bytes.to_vec())?;
                Some(self.add_relationship(
                    Some(&drawing),
                    IMAGE_REL,
                    &relative_target(Some(&drawing), &part),
                    TargetMode::Internal,
                )?)
            }
            None => None,
        };

        let ws = self.sheet(sheet).ok_or(XlsxError::NotAWorksheet(sheet))?;
        let place = Place::new(ws, self.drawing_metrics());
        let (from, to, off) = placement(&place, row, col, ext);
        let names = Names(&prefix);
        let text = dx::anchor_xml(
            &names,
            &from,
            &to,
            off,
            &Body::Picture {
                id,
                name: &name,
                embed: &embed,
                svg: svg_rid.as_deref(),
                ext,
            },
        );
        let frag = crate::sheet::parse_fragment(&text, &open.doc.root.head)?;
        open.doc.root.children.push(RawXml::Element(frag));
        self.save_drawing(open)?;
        Ok(id)
    }

    /// Insert > Shapes and Insert > Text Box: a shape of the gallery preset `preset` (an
    /// `ST_ShapeType` name from [`catalog`]) or a text box, with its top-left corner at cell
    /// `(row, col)` and `size_pt` (width, height) points, or the click default. The style is the one
    /// Excel gives a new shape (accent 1 fill and outline, light text) or text box (white fill, thin
    /// grey outline, dark text). Returns the object id.
    ///
    /// # Errors
    /// The sheet is not a worksheet, the cell is outside the grid, the preset is not in the gallery
    /// or the size is not positive.
    pub fn insert_shape(
        &mut self,
        sheet: usize,
        row: u32,
        col: u32,
        preset: &str,
        text_box: bool,
        size_pt: Option<(f64, f64)>,
    ) -> Result<u32, XlsxError> {
        self.require_transitional()?;
        Self::check_cell(row, col)?;
        self.worksheet_checked(sheet)?;
        let preset = if text_box { "rect" } else { preset };
        if catalog::label_of(preset).is_none() {
            return Err(bad(format!("{preset} is not a shape in the gallery")));
        }
        let (w, h) = size_pt.unwrap_or(if text_box {
            CLICK_TEXT_BOX_SIZE_PT
        } else {
            CLICK_SHAPE_SIZE_PT
        });
        let connector = catalog::is_connector(preset);
        if !(w >= 0.0 && h >= 0.0 && w.is_finite() && h.is_finite()) || (w == 0.0 && h == 0.0) {
            return Err(bad("the shape needs a size"));
        }
        let ext = (emu(w), emu(h));
        self.edited()?;
        let (drawing, _) = self.ensure_drawing(sheet)?;
        let mut open = self.open_drawing(&drawing)?;
        let prefix = Self::anchor_prefix(&open.doc);
        let id = max_shape_id(&open.doc.root).max(1) + 1;
        let mut taken = Vec::new();
        shape_names(&open.doc.root, &mut taken);
        let stem = if text_box {
            "TextBox"
        } else {
            catalog::default_name_stem(preset)
        };
        let name = fresh_name(stem, id, &taken);

        let ws = self.sheet(sheet).ok_or(XlsxError::NotAWorksheet(sheet))?;
        let place = Place::new(ws, self.drawing_metrics());
        let (from, to, off) = placement(&place, row, col, ext);
        let text = dx::anchor_xml(
            &Names(&prefix),
            &from,
            &to,
            off,
            &Body::Shape {
                id,
                name: &name,
                preset,
                text_box,
                connector,
                ext,
            },
        );
        let frag = crate::sheet::parse_fragment(&text, &open.doc.root.head)?;
        open.doc.root.children.push(RawXml::Element(frag));
        self.save_drawing(open)?;
        Ok(id)
    }
}
