//! Editing the drawing objects of a worksheet: frame, rotation, flips, fill, outline, text, name,
//! placement, order and deletion; and the snapshot and reflow that keep objects where Excel keeps them
//! when rows, columns or cells change (rules in [`super::drawing_shift`]).
//!
//! An object is addressed by its drawing id (`cNvPr id`, see `Workbook::sheet_drawing_objects`).

use open365_opc::{PartName, Relationships, resolve_target};

use super::drawing_ops::{
    Item, Snapshot, child_mut, emu, fragment, is_anchor, shape_xfrm_mut, sp_pr_mut, stored_size,
    write_anchor, write_xfrm,
};
use super::drawing_shift::{AxisShift, Place, reflow};
use super::drawing_xml as dx;
use crate::XlsxError;
use crate::drawing::{Anchor, EditAs, RectPt, parse_anchor};
use crate::structure::{EditAxis, FormulaShifter, StructEdit};
use crate::workbook::Workbook;
use crate::xml::{RawXml, XmlElement};

fn bad(message: impl Into<String>) -> XlsxError {
    XlsxError::UnsupportedEdit(message.into())
}

const GEOMETRY: [&str; 2] = ["prstGeom", "custGeom"];
const FILLS: [&str; 6] = [
    "noFill",
    "solidFill",
    "gradFill",
    "blipFill",
    "pattFill",
    "grpFill",
];

/// Index at which a new fill goes in `spPr`: after the geometry, else after the transform.
fn fill_position(sp_pr: &XmlElement) -> usize {
    let after = |names: &[&str]| {
        sp_pr.children.iter().rposition(|c| {
            c.as_element()
                .is_some_and(|e| names.contains(&e.head.local_name()))
        })
    };
    after(&GEOMETRY)
        .or_else(|| after(&["xfrm"]))
        .map_or(0, |i| i + 1)
}

fn remove_children(el: &mut XmlElement, names: &[&str]) {
    el.children.retain(|c| {
        !c.as_element()
            .is_some_and(|e| names.contains(&e.head.local_name()))
    });
}

impl Workbook {
    /// Moves and sizes object `id` of `sheet` to the rectangle `(x, y, width, height)` in points from
    /// the corner of A1, keeping its anchor kind and placement mode. The rectangle is the unrotated frame.
    ///
    /// # Errors
    /// The sheet or the object does not exist, or the size is negative.
    pub fn set_drawing_frame(
        &mut self,
        sheet: usize,
        id: u32,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> Result<(), XlsxError> {
        if !(width >= 0.0 && height >= 0.0 && x.is_finite() && y.is_finite()) {
            return Err(bad("the object frame is not valid"));
        }
        self.require_transitional()?;
        let (mut open, index) = self.locate(sheet, id)?;
        self.edited()?;
        let ws = self.sheet(sheet).ok_or(XlsxError::NotAWorksheet(sheet))?;
        let place = Place::new(ws, self.drawing_metrics());
        let el = open.doc.root.children[index]
            .as_element_mut()
            .ok_or_else(|| bad("not an anchor"))?;
        let old = parse_anchor(el).ok_or_else(|| bad("the anchor is not readable"))?;
        let new = match old {
            Anchor::TwoCell { edit_as, .. } => Anchor::TwoCell {
                from: place.marker_at(x, y),
                to: place.marker_at(x + width, y + height),
                edit_as,
            },
            Anchor::OneCell { .. } => Anchor::OneCell {
                from: place.marker_at(x, y),
                cx: emu(width),
                cy: emu(height),
            },
            Anchor::Absolute { .. } => Anchor::Absolute {
                x: emu(x),
                y: emu(y),
                cx: emu(width),
                cy: emu(height),
            },
        };
        write_anchor(el, &new);
        if let Some(xf) = shape_xfrm_mut(el) {
            write_xfrm(
                xf,
                &RectPt {
                    x,
                    y,
                    width,
                    height,
                },
            );
        }
        self.save_drawing(open)
    }

    /// Rotates object `id` to `degrees` clockwise (any real, stored modulo 360).
    ///
    /// # Errors
    /// The object does not exist or has no transform (a chart).
    pub fn set_drawing_rotation(
        &mut self,
        sheet: usize,
        id: u32,
        degrees: f64,
    ) -> Result<(), XlsxError> {
        if !degrees.is_finite() {
            return Err(bad("the rotation is not a number"));
        }
        self.edit_transform(sheet, id, |xf| {
            let units = (degrees.rem_euclid(360.0) * 60_000.0).round() as i64 % 21_600_000;
            if units == 0 {
                xf.head.remove_attr("rot");
            } else {
                xf.head.set_attr("rot", &units.to_string());
            }
        })
    }

    /// Flips object `id` horizontally and/or vertically (absolute, not toggled).
    ///
    /// # Errors
    /// The object does not exist or has no transform.
    pub fn set_drawing_flip(
        &mut self,
        sheet: usize,
        id: u32,
        horizontal: bool,
        vertical: bool,
    ) -> Result<(), XlsxError> {
        self.edit_transform(sheet, id, |xf| {
            for (name, on) in [("flipH", horizontal), ("flipV", vertical)] {
                if on {
                    xf.head.set_attr(name, "1");
                } else {
                    xf.head.remove_attr(name);
                }
            }
        })
    }

    fn edit_transform(
        &mut self,
        sheet: usize,
        id: u32,
        f: impl FnOnce(&mut XmlElement),
    ) -> Result<(), XlsxError> {
        self.require_transitional()?;
        let (mut open, index) = self.locate(sheet, id)?;
        self.edited()?;
        let el = open.doc.root.children[index]
            .as_element_mut()
            .ok_or_else(|| bad("not an anchor"))?;
        let xf = shape_xfrm_mut(el).ok_or_else(|| bad("the object has no transform"))?;
        f(xf);
        self.save_drawing(open)
    }

    /// Sets the fill of a shape, text box or picture frame: `none`, a colour (`RRGGBB`) or a scheme
    /// colour (`accent1`, `accent1:lumMod=60000,lumOff=40000`).
    ///
    /// # Errors
    /// The object does not exist, has no shape properties, or the colour is not valid.
    pub fn set_drawing_fill(&mut self, sheet: usize, id: u32, fill: &str) -> Result<(), XlsxError> {
        let xml = dx::fill_xml(fill)?;
        self.require_transitional()?;
        let (mut open, index) = self.locate(sheet, id)?;
        self.edited()?;
        let head = open.doc.root.head.clone();
        let el = open.doc.root.children[index]
            .as_element_mut()
            .ok_or_else(|| bad("not an anchor"))?;
        let sp_pr = sp_pr_mut(el).ok_or_else(|| bad("the object has no shape properties"))?;
        let new = fragment(&xml, &head)?;
        remove_children(sp_pr, &FILLS);
        let at = fill_position(sp_pr);
        sp_pr.children.insert(at, RawXml::Element(new));
        self.save_drawing(open)
    }

    /// Sets the outline: `color` (`none`, `RRGGBB` or a scheme colour) when given, the width in points,
    /// and the dash style (`solid`, `round-dot`, `square-dot`, `dash`, `dash-dot`, `long-dash`,
    /// `long-dash-dot`, `long-dash-dot-dot`). Parts left `None` keep their value.
    ///
    /// # Errors
    /// The object does not exist, has no shape properties, or a value is not valid.
    pub fn set_drawing_line(
        &mut self,
        sheet: usize,
        id: u32,
        color: Option<&str>,
        width_pt: Option<f64>,
        dash: Option<&str>,
    ) -> Result<(), XlsxError> {
        let fill = color.map(dx::fill_xml).transpose()?;
        let dash = dash.map(dx::dash_value).transpose()?;
        if let Some(w) = width_pt
            && !(0.0..=1584.0).contains(&w)
        {
            return Err(bad("the line width is outside 0 to 1584 points"));
        }
        self.require_transitional()?;
        let (mut open, index) = self.locate(sheet, id)?;
        self.edited()?;
        let head = open.doc.root.head.clone();
        let el = open.doc.root.children[index]
            .as_element_mut()
            .ok_or_else(|| bad("not an anchor"))?;
        let sp_pr = sp_pr_mut(el).ok_or_else(|| bad("the object has no shape properties"))?;
        if !sp_pr
            .children
            .iter()
            .any(|c| c.as_element().is_some_and(|e| e.head.local_name() == "ln"))
        {
            let at = sp_pr
                .children
                .iter()
                .rposition(|c| {
                    c.as_element().is_some_and(|e| {
                        FILLS.contains(&e.head.local_name())
                            || GEOMETRY.contains(&e.head.local_name())
                    })
                })
                .map_or(sp_pr.children.len(), |i| i + 1);
            sp_pr
                .children
                .insert(at, RawXml::Element(fragment("<a:ln/>", &head)?));
        }
        let ln = child_mut(sp_pr, "ln").ok_or_else(|| bad("no outline"))?;
        if let Some(w) = width_pt {
            ln.head.set_attr("w", &emu(w).to_string());
        }
        let mut at = 0;
        if let Some(fill) = fill {
            remove_children(ln, &FILLS);
            ln.children
                .insert(0, RawXml::Element(fragment(&fill, &head)?));
        }
        if ln
            .children
            .first()
            .and_then(RawXml::as_element)
            .is_some_and(|e| FILLS.contains(&e.head.local_name()))
        {
            at = 1;
        }
        if let Some(d) = dash {
            remove_children(ln, &["prstDash", "custDash"]);
            let el = fragment(&format!("<a:prstDash val=\"{d}\"/>"), &head)?;
            ln.children.insert(at, RawXml::Element(el));
        }
        self.save_drawing(open)
    }

    /// Replaces the text of a shape or text box with `text` (lines split at `\n`), keeping the first
    /// paragraph's properties and the run properties of its first run.
    ///
    /// # Errors
    /// The object does not exist or cannot hold text (a picture, a chart).
    pub fn set_drawing_text(&mut self, sheet: usize, id: u32, text: &str) -> Result<(), XlsxError> {
        self.require_transitional()?;
        let (mut open, index) = self.locate(sheet, id)?;
        self.edited()?;
        let head = open.doc.root.head.clone();
        let el = open.doc.root.children[index]
            .as_element_mut()
            .ok_or_else(|| bad("not an anchor"))?;
        let sp = el
            .children
            .iter_mut()
            .filter_map(RawXml::as_element_mut)
            .find(|c| c.head.local_name() == "sp")
            .ok_or_else(|| bad("the object cannot hold text"))?;
        let tx = child_mut(sp, "txBody").ok_or_else(|| bad("the shape has no text body"))?;
        let first = tx
            .children
            .iter()
            .filter_map(RawXml::as_element)
            .find(|c| c.head.local_name() == "p");
        let ppr = first
            .and_then(|p| p.elements().find(|c| c.head.local_name() == "pPr"))
            .map(|p| {
                let mut out = String::new();
                p.write(&mut out);
                out
            })
            .unwrap_or_default();
        // Run properties: the first run's, else the end-of-paragraph ones.
        let rpr = first
            .and_then(|p| {
                p.elements()
                    .find(|c| c.head.local_name() == "r")
                    .and_then(|r| r.elements().find(|c| c.head.local_name() == "rPr"))
                    .or_else(|| p.elements().find(|c| c.head.local_name() == "endParaRPr"))
            })
            .map(|r| {
                r.head
                    .attrs
                    .iter()
                    .filter(|a| a.ns.is_none())
                    .map(|a| format!("{}=\"{}\"", a.name, super::super::esc(&a.value)))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_else(|| "lang=\"en-GB\" sz=\"1100\"".to_owned());
        let paragraphs = dx::paragraphs_xml(text, &ppr, &rpr);
        let wrapper = fragment(&format!("<a:tmp>{paragraphs}</a:tmp>"), &head)?;
        tx.children
            .retain(|c| c.as_element().is_none_or(|e| e.head.local_name() != "p"));
        tx.children.extend(wrapper.children);
        self.save_drawing(open)
    }

    /// Renames object `id`; `alt` (when given) sets its alternative text (`descr`).
    ///
    /// # Errors
    /// The object does not exist or the name is empty.
    pub fn set_drawing_name(
        &mut self,
        sheet: usize,
        id: u32,
        name: &str,
        alt: Option<&str>,
    ) -> Result<(), XlsxError> {
        if name.is_empty() {
            return Err(bad("the object needs a name"));
        }
        self.require_transitional()?;
        let (mut open, index) = self.locate(sheet, id)?;
        self.edited()?;
        let el = open.doc.root.children[index]
            .as_element_mut()
            .ok_or_else(|| bad("not an anchor"))?;
        let c = super::drawing_ops::c_nv_pr_mut(el)
            .ok_or_else(|| bad("the object has no properties"))?;
        c.head.set_attr("name", name);
        if let Some(a) = alt {
            if a.is_empty() {
                c.head.remove_attr("descr");
            } else {
                c.head.set_attr("descr", a);
            }
        }
        self.save_drawing(open)
    }

    /// Format Object > Properties: how the object follows its cells. A one-cell or absolute anchor
    /// element becomes a two-cell anchor with the matching `editAs`.
    ///
    /// # Errors
    /// The object does not exist.
    pub fn set_drawing_placement(
        &mut self,
        sheet: usize,
        id: u32,
        mode: EditAs,
    ) -> Result<(), XlsxError> {
        self.require_transitional()?;
        let (mut open, index) = self.locate(sheet, id)?;
        self.edited()?;
        let ws = self.sheet(sheet).ok_or(XlsxError::NotAWorksheet(sheet))?;
        let place = Place::new(ws, self.drawing_metrics());
        let el = open.doc.root.children[index]
            .as_element_mut()
            .ok_or_else(|| bad("not an anchor"))?;
        let old = parse_anchor(el).ok_or_else(|| bad("the anchor is not readable"))?;
        match old {
            Anchor::TwoCell { from, to, .. } => write_anchor(
                el,
                &Anchor::TwoCell {
                    from,
                    to,
                    edit_as: mode,
                },
            ),
            other => {
                // Rebuild as a two-cell anchor over the same rectangle.
                let r = place.rect(&other);
                let new = Anchor::TwoCell {
                    from: place.marker_at(r.x, r.y),
                    to: place.marker_at(r.x + r.width, r.y + r.height),
                    edit_as: mode,
                };
                let kids: Vec<RawXml> = std::mem::take(&mut el.children)
                    .into_iter()
                    .filter(|c| {
                        !c.as_element()
                            .is_some_and(|e| matches!(e.head.local_name(), "from" | "pos" | "ext"))
                    })
                    .collect();
                let head = open.doc.root.head.clone();
                let prefix = Self::anchor_prefix(&open.doc);
                let n = dx::Names(&prefix);
                let shell = fragment(
                    &format!(
                        "<{a}><{f}/><{t}/></{a}>",
                        a = n.p("twoCellAnchor"),
                        f = n.p("from"),
                        t = n.p("to")
                    ),
                    &head,
                )?;
                let el = open.doc.root.children[index]
                    .as_element_mut()
                    .ok_or_else(|| bad("not an anchor"))?;
                el.head = shell.head;
                el.children = shell.children;
                el.children.extend(kids);
                write_anchor(el, &new);
            }
        }
        self.save_drawing(open)
    }

    /// Arrange > Bring to Front (`front`), Send to Back (`back`), Bring Forward (`forward`) or Send
    /// Backward (`backward`): moves the object's anchor within the drawing's z order.
    ///
    /// # Errors
    /// The object does not exist or `order` is not one of the four names.
    pub fn arrange_drawing(&mut self, sheet: usize, id: u32, order: &str) -> Result<(), XlsxError> {
        if !matches!(order, "front" | "back" | "forward" | "backward") {
            return Err(bad(format!("{order} is not an order")));
        }
        self.require_transitional()?;
        let (mut open, index) = self.locate(sheet, id)?;
        let anchors: Vec<usize> = open
            .doc
            .root
            .children
            .iter()
            .enumerate()
            .filter(|(_, c)| c.as_element().is_some_and(is_anchor))
            .map(|(i, _)| i)
            .collect();
        let pos = anchors.iter().position(|i| *i == index).unwrap_or(0);
        let target = match order {
            "front" => anchors.len() - 1,
            "back" => 0,
            "forward" => (pos + 1).min(anchors.len() - 1),
            _ => pos.saturating_sub(1),
        };
        if target == pos {
            return Ok(());
        }
        self.edited()?;
        let moved = open.doc.root.children.remove(index);
        let at = anchors[target];
        open.doc.root.children.insert(at, moved);
        self.save_drawing(open)
    }

    /// Deletes object `id`: its anchor, and the relationships and parts only it used (picture files).
    ///
    /// # Errors
    /// The object does not exist.
    pub fn delete_drawing(&mut self, sheet: usize, id: u32) -> Result<(), XlsxError> {
        self.require_transitional()?;
        let (mut open, index) = self.locate(sheet, id)?;
        self.edited()?;
        let removed = open.doc.root.children.remove(index);
        let mut rids = Vec::new();
        if let Some(el) = removed.as_element() {
            collect_rids(el, &mut rids);
        }
        let mut still = Vec::new();
        for c in open.doc.root.children.iter().filter_map(RawXml::as_element) {
            collect_rids(c, &mut still);
        }
        let drawing = open.part.clone();
        self.save_drawing(open)?;
        for rid in rids.into_iter().filter(|r| !still.contains(r)) {
            self.release_relationship(&drawing, &rid)?;
        }
        Ok(())
    }

    /// Removes relationship `rid` of `source` and the part it points at when nothing else does.
    fn release_relationship(&mut self, source: &PartName, rid: &str) -> Result<(), XlsxError> {
        let Some(target) = self
            .relationships(Some(source))
            .into_iter()
            .find(|r| r.id == rid)
            .and_then(|r| resolve_target(Some(source), &r.target).ok())
        else {
            return Ok(());
        };
        self.remove_relationship(Some(source), rid)?;
        if !self.part_is_referenced(&target) {
            // A chart part takes the parts only it uses along.
            let children: Vec<PartName> = self
                .relationships(Some(&target))
                .into_iter()
                .filter_map(|r| resolve_target(Some(&target), &r.target).ok())
                .collect();
            self.drop_part(target.as_str());
            for child in children {
                if !self.part_is_referenced(&child) {
                    self.drop_part(child.as_str());
                }
            }
        }
        Ok(())
    }

    /// True when any relationship in the package targets `part`.
    fn part_is_referenced(&self, part: &PartName) -> bool {
        self.package().parts().any(|p| {
            let name = p.name().to_string();
            if !name.ends_with(".rels") {
                return false;
            }
            let Ok(rels) = Relationships::parse(p.data()) else {
                return false;
            };
            // The source of `a/_rels/b.xml.rels` is `a/b.xml`; the package's own rels have none.
            let source = name
                .strip_suffix(".rels")
                .and_then(|n| n.rsplit_once("_rels/"))
                .map(|(dir, file)| format!("{dir}{file}"))
                .and_then(|n| PartName::new(&n).ok());
            rels.iter().any(|r| {
                r.target_mode == open365_opc::TargetMode::Internal
                    && resolve_target(source.as_ref(), &r.target).is_ok_and(|t| {
                        t.as_str().trim_start_matches('/') == part.as_str().trim_start_matches('/')
                    })
            })
        })
    }

    // ---- keeping objects in place when the cells change --------------------------------------------

    /// The anchored objects of `sheet` as they are now; `None` when the sheet has none.
    pub(crate) fn drawing_snapshot(&self, sheet: usize) -> Option<Snapshot> {
        let part = self.existing_drawing(sheet)?;
        let ws = self.sheet(sheet)?;
        let place = Place::new(ws, self.drawing_metrics());
        let open = self.open_drawing(&part).ok()?;
        let items: Vec<Item> = open
            .doc
            .root
            .children
            .iter()
            .enumerate()
            .filter_map(|(index, c)| {
                let el = c.as_element().filter(|e| is_anchor(e))?;
                let anchor = parse_anchor(el)?;
                Some(Item {
                    index,
                    anchor,
                    rect: place.rect(&anchor),
                    size: stored_size(el),
                })
            })
            .collect();
        (!items.is_empty()).then_some(Snapshot { part, items })
    }

    /// Moves the objects of a [`Snapshot`] for the change that was just made to the cells of `sheet`
    /// (`shift` is the rows or columns inserted or deleted, `None` for hide, unhide and resize).
    pub(crate) fn drawing_reflow(
        &mut self,
        sheet: usize,
        snap: Snapshot,
        shift: Option<AxisShift>,
    ) -> Result<(), XlsxError> {
        let ws = self.sheet(sheet).ok_or(XlsxError::NotAWorksheet(sheet))?;
        let place = Place::new(ws, self.drawing_metrics());
        let mut open = self.open_drawing(&snap.part)?;
        let mut gone = Vec::new();
        let mut changed = false;
        for item in &snap.items {
            let new = reflow(&item.anchor, item.rect, item.size, &place, shift.as_ref());
            let Some(new) = new else {
                gone.push(item.index);
                changed = true;
                continue;
            };
            if new == item.anchor {
                continue;
            }
            let Some(el) = open.doc.root.children[item.index].as_element_mut() else {
                continue;
            };
            write_anchor(el, &new);
            let r = place.rect(&new);
            if let Some(xf) = shape_xfrm_mut(el) {
                write_xfrm(xf, &r);
            }
            changed = true;
        }
        // Remove from the back so the indices of the others stay valid.
        for index in gone.into_iter().rev() {
            open.doc.root.children.remove(index);
        }
        if changed {
            self.save_drawing(open)?;
        }
        Ok(())
    }

    /// [`Workbook::edit_structure`] that also moves the drawing objects of the sheet the way Excel does
    /// (insert, delete: move, stretch, shrink or remove them by placement mode).
    ///
    /// # Errors
    /// As [`Workbook::edit_structure`].
    pub fn edit_structure_with_drawings(
        &mut self,
        sheet: usize,
        edit: StructEdit,
        shifter: &dyn FormulaShifter,
    ) -> Result<Vec<(usize, u32, u32)>, XlsxError> {
        let snap = self.drawing_snapshot(sheet);
        let out = self.edit_structure(sheet, edit, shifter)?;
        if let Some(s) = snap {
            let shift = AxisShift {
                rows: edit.axis == EditAxis::Rows,
                band: None,
                at: edit.at,
                count: edit.count,
                delete: edit.delete,
            };
            self.drawing_reflow(sheet, s, Some(shift))?;
        }
        Ok(out)
    }

    /// Runs `change` (hide, unhide or resize rows or columns of `sheet`) and then moves the drawing
    /// objects of the sheet the way Excel does for a geometry change: a move-and-size object stretches
    /// with its cells, a move-only object keeps its size, a free-floating one keeps its place.
    ///
    /// # Errors
    /// Whatever `change` returns, or a drawing part that cannot be written.
    pub fn with_drawings_reflowed<R>(
        &mut self,
        sheet: usize,
        change: impl FnOnce(&mut Self) -> Result<R, XlsxError>,
    ) -> Result<R, XlsxError> {
        let snap = self.drawing_snapshot(sheet);
        let out = change(self)?;
        if let Some(s) = snap {
            self.drawing_reflow(sheet, s, None)?;
        }
        Ok(out)
    }
}

fn collect_rids(el: &XmlElement, out: &mut Vec<String>) {
    for a in &el.head.attrs {
        if a.ns
            .as_deref()
            .is_some_and(|n| n.ends_with("/relationships"))
        {
            out.push(a.value.clone());
        }
    }
    for c in el.elements() {
        collect_rids(c, out);
    }
}
