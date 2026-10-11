//! Worksheet side of merged cells: the `mergeCells` element and the cells a merge empties.

use super::{CellValue, Rect, Worksheet};
use crate::structure::range_a1;
use crate::xml::{RawXml, XmlElement};

impl Worksheet {
    /// Replaces the `mergeCells` element with `ranges` (removed when none are left).
    pub(crate) fn put_merges(&mut self, ranges: &[Rect]) {
        let element = (!ranges.is_empty()).then(|| {
            let mut el = XmlElement {
                head: self.sheet_data_head.sibling("mergeCells"),
                children: Vec::new(),
            };
            el.head.set_attr("count", &ranges.len().to_string());
            for &r in ranges {
                let mut m = XmlElement {
                    head: self.sheet_data_head.sibling("mergeCell"),
                    children: Vec::new(),
                };
                m.head.set_attr("ref", &range_a1(r));
                el.children.push(RawXml::Element(m));
            }
            el
        });
        self.put_root_element("mergeCells", element);
    }

    /// Cells of the merge areas other than their upper-left cell that hold a value or a formula.
    pub(crate) fn merge_discards(&self, areas: &[Rect]) -> Vec<(u32, u32)> {
        let mut out = Vec::new();
        for &area in areas {
            for (r, c, cell) in self.iter_cells() {
                let inside = r >= area.0 && r <= area.2 && c >= area.1 && c <= area.3;
                if inside
                    && (r, c) != (area.0, area.1)
                    && (!matches!(cell.value, CellValue::Empty) || cell.formula.is_some())
                {
                    out.push((r, c));
                }
            }
        }
        out
    }
}
