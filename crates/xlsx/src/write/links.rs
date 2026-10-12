//! Cell hyperlinks: external URL, place in this workbook, e-mail address.

use open365_opc::TargetMode;

use super::{MAIN, R_NS, esc, rel};
use crate::XlsxError;
use crate::refs::{MAX_COL, MAX_ROW, parse_range};
use crate::sheet::CellValue;
use crate::sheet::Rect;
use crate::structure::range_a1;
use crate::styles_edit::{
    el, fix_count, intern, item, remove_child, section, section_index, set_font_child,
};
use crate::workbook::{CellInput, Workbook};
use crate::xml::{RawXml, XmlElement};

/// Cells that get the Hyperlink style when a link is set (Excel styles every cell of the range).
const STYLE_CELL_LIMIT: u64 = 4096;

/// Where a hyperlink points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HyperlinkTarget {
    /// A web page or file: `https://example.com/page#part`.
    Url(String),
    /// A place in this workbook: `Sheet2!A1` or a defined name.
    Location(String),
    /// An e-mail address, with an optional subject.
    Email {
        address: String,
        subject: Option<String>,
    },
}

/// A hyperlink to set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HyperlinkSpec {
    /// The cell (or cells) that carry the link.
    pub range: Rect,
    pub target: HyperlinkTarget,
    /// The text to show (Excel's "Text to display"): it replaces the content of the first cell and is the
    /// `display` attribute of a link to a place in the workbook. `None` keeps a non-empty cell as it is
    /// and shows the address in an empty one, as Excel does.
    pub display: Option<String>,
    /// The screen tip.
    pub tooltip: Option<String>,
}

fn percent_encode(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn is_blocked_scheme(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    ["javascript:", "vbscript:", "data:"]
        .iter()
        .any(|s| lower.starts_with(s))
}

impl Workbook {
    /// The hyperlinks of a sheet as `(range, target, display, tooltip)`.
    pub fn hyperlinks(&self, sheet: usize) -> Vec<HyperlinkSpec> {
        let Some(ws) = self.sheet(sheet) else {
            return Vec::new();
        };
        let Some(part) = self.sheet_part(sheet) else {
            return Vec::new();
        };
        let rels = self.relationships(Some(part));
        let Some(list) = ws.root_element("hyperlinks") else {
            return Vec::new();
        };
        list.elements()
            .filter(|h| h.head.is_main("hyperlink"))
            .filter_map(|h| {
                let range = h.head.attr("ref").and_then(parse_range)?;
                let target = if let Some(id) = h.head.attr_r("id") {
                    let r = rels.iter().find(|r| r.id == id)?;
                    match r.target.strip_prefix("mailto:") {
                        Some(rest) => {
                            let (address, query) = rest
                                .split_once('?')
                                .map_or((rest, None), |(a, q)| (a, Some(q)));
                            let subject = query
                                .and_then(|q| q.strip_prefix("subject="))
                                .map(percent_decode);
                            HyperlinkTarget::Email {
                                address: address.to_owned(),
                                subject,
                            }
                        }
                        None => HyperlinkTarget::Url(match h.head.attr("location") {
                            Some(frag) => format!("{}#{frag}", r.target),
                            None => r.target.clone(),
                        }),
                    }
                } else {
                    HyperlinkTarget::Location(h.head.attr("location")?.to_owned())
                };
                Some(HyperlinkSpec {
                    range,
                    target,
                    display: h.head.attr("display").map(str::to_owned),
                    tooltip: h.head.attr("tooltip").map(str::to_owned),
                })
            })
            .collect()
    }

    /// Sets the hyperlink on `spec.range`, replacing one with the same range. External targets get a
    /// `hyperlink` relationship of `TargetMode="External"`; a place in the workbook is a `location`.
    ///
    /// # Errors
    /// The sheet is not a worksheet, the range is outside the grid, or the target is empty or uses a
    /// scheme that runs code (`javascript:`, `vbscript:`, `data:`).
    pub fn set_hyperlink(&mut self, sheet: usize, spec: &HyperlinkSpec) -> Result<(), XlsxError> {
        let (r1, c1, r2, c2) = spec.range;
        if r1 == 0 || c1 == 0 || r1 > r2 || c1 > c2 || r2 > MAX_ROW || c2 > MAX_COL {
            return Err(XlsxError::UnsupportedEdit(
                "the hyperlink cells are outside the worksheet".to_owned(),
            ));
        }
        let sheet_part = self
            .sheet_part(sheet)
            .cloned()
            .ok_or(XlsxError::NotAWorksheet(sheet))?;
        let (external, location): (Option<String>, Option<String>) = match &spec.target {
            HyperlinkTarget::Url(u) => {
                let u = u.trim();
                if u.is_empty() || is_blocked_scheme(u) {
                    return Err(XlsxError::UnsupportedEdit(format!(
                        "the address {u:?} cannot be a hyperlink"
                    )));
                }
                match u.split_once('#') {
                    Some((base, frag)) if !base.is_empty() && !frag.is_empty() => {
                        (Some(base.to_owned()), Some(frag.to_owned()))
                    }
                    _ => (Some(u.to_owned()), None),
                }
            }
            HyperlinkTarget::Location(l) => {
                if l.trim().is_empty() {
                    return Err(XlsxError::UnsupportedEdit(
                        "the place in this document is empty".to_owned(),
                    ));
                }
                (None, Some(l.trim().to_owned()))
            }
            HyperlinkTarget::Email { address, subject } => {
                let a = address.trim();
                if !a.contains('@') || a.contains(char::is_whitespace) {
                    return Err(XlsxError::UnsupportedEdit(format!(
                        "{a:?} is not an e-mail address"
                    )));
                }
                let mut t = format!("mailto:{a}");
                if let Some(s) = subject.as_deref().filter(|s| !s.is_empty()) {
                    t.push_str("?subject=");
                    t.push_str(&percent_encode(s));
                }
                (Some(t), None)
            }
        };
        self.worksheet_mut(sheet)?;
        self.edited()?;
        let text_given = spec.display.as_deref().filter(|d| !d.is_empty());
        let shown_default = match &spec.target {
            HyperlinkTarget::Url(u) => u.trim().to_owned(),
            HyperlinkTarget::Location(l) => l.trim().to_owned(),
            HyperlinkTarget::Email { address, .. } => address.trim().to_owned(),
        };
        let was_empty = self
            .sheet(sheet)
            .and_then(|w| w.cell(r1, c1))
            .is_none_or(|c| matches!(c.value, CellValue::Empty) && c.formula.is_none());
        // Excel: the `display` attribute is the text for a place in the workbook; for an address it is
        // written only when the cell already had content and no text was given (then it is the address).
        let display_attr: Option<String> = if external.is_none() {
            Some(text_given.map_or_else(|| shown_default.clone(), str::to_owned))
        } else if text_given.is_none() && !was_empty {
            Some(shown_default.clone())
        } else {
            None
        };
        if let Some(t) = text_given {
            self.set_cell_value(sheet, r1, c1, CellInput::Text(t.to_owned()))?;
        } else if was_empty {
            self.set_cell_value(sheet, r1, c1, CellInput::Text(shown_default.clone()))?;
        }
        self.style_as_hyperlink(sheet, spec.range)?;
        // Drop the link that has the same range.
        self.remove_hyperlink_exact(sheet, spec.range)?;
        let rid = match &external {
            Some(target) => Some(self.add_relationship(
                Some(&sheet_part),
                rel::HYPERLINK,
                target,
                TargetMode::External,
            )?),
            None => None,
        };
        let mut attrs = format!(" ref=\"{}\"", range_a1(spec.range));
        if let Some(rid) = &rid {
            attrs.push_str(&format!(" r:id=\"{rid}\""));
        }
        if let Some(l) = &location {
            attrs.push_str(&format!(" location=\"{}\"", esc(l)));
        }
        if let Some(t) = &spec.tooltip {
            attrs.push_str(&format!(" tooltip=\"{}\"", esc(t)));
        }
        if let Some(d) = &display_attr {
            attrs.push_str(&format!(" display=\"{}\"", esc(d)));
        }
        let ws = self.worksheet_mut(sheet)?;
        let link = ws.fragment(&format!(
            "<hyperlink xmlns=\"{MAIN}\" xmlns:r=\"{R_NS}\"{attrs}/>"
        ))?;
        let mut list = match ws.root_element("hyperlinks").cloned() {
            Some(l) => l,
            None => ws.fragment(&format!("<hyperlinks xmlns=\"{MAIN}\"/>"))?,
        };
        list.children.push(RawXml::Element(link));
        ws.put_root_element("hyperlinks", Some(list));
        self.sheets[sheet].dirty = true;
        Ok(())
    }

    /// Gives the cells of `range` the Hyperlink cell style, as Excel does when it makes a link. A cell with
    /// a format of its own keeps it.
    fn style_as_hyperlink(&mut self, sheet: usize, range: Rect) -> Result<(), XlsxError> {
        let (r1, c1, r2, c2) = range;
        if u64::from(r2 - r1 + 1) * u64::from(c2 - c1 + 1) > STYLE_CELL_LIMIT {
            return Ok(());
        }
        self.ensure_hyperlink_style()?;
        for r in r1..=r2 {
            for c in c1..=c2 {
                if self.cell_xf(sheet, r, c) == 0 {
                    self.apply_cell_style(sheet, (r, c, r, c), "Hyperlink")?;
                }
            }
        }
        Ok(())
    }

    /// Adds the built-in Hyperlink cell style the way Excel writes it (underlined theme colour 10 on the
    /// workbook's Normal font) when the style sheet has none.
    fn ensure_hyperlink_style(&mut self) -> Result<(), XlsxError> {
        let root = self.styles.root_mut()?;
        if section(root, "cellStyles").is_some_and(|s| {
            s.elements().any(|c| {
                c.head
                    .attr("name")
                    .is_some_and(|n| n.eq_ignore_ascii_case("Hyperlink"))
            })
        }) {
            return Ok(());
        }
        let tpl = root.sibling("x");
        let mut font = item(root, "fonts", 0).unwrap_or_else(|| el(&tpl, "font", &[]));
        remove_child(&mut font, "u");
        remove_child(&mut font, "color");
        set_font_child(&mut font, el(&tpl, "u", &[]));
        set_font_child(&mut font, el(&tpl, "color", &[("theme", "10")]));
        let font_id = intern(root, "fonts", font);
        let xf = el(
            &tpl,
            "xf",
            &[
                ("numFmtId", "0"),
                ("fontId", &font_id.to_string()),
                ("fillId", "0"),
                ("borderId", "0"),
                ("applyNumberFormat", "0"),
                ("applyFill", "0"),
                ("applyBorder", "0"),
                ("applyAlignment", "0"),
                ("applyProtection", "0"),
            ],
        );
        let at = section_index(root, "cellStyleXfs");
        let Some(sec) = root.children[at].as_element_mut() else {
            return Ok(());
        };
        let xf_id = sec.elements().count();
        sec.children.push(RawXml::Element(xf));
        fix_count(sec);
        let at = section_index(root, "cellStyles");
        let Some(sec) = root.children[at].as_element_mut() else {
            return Ok(());
        };
        let style: XmlElement = el(
            &tpl,
            "cellStyle",
            &[
                ("name", "Hyperlink"),
                ("xfId", &xf_id.to_string()),
                ("builtinId", "8"),
            ],
        );
        // Excel keeps the list sorted by name.
        let pos = sec
            .children
            .iter()
            .position(|c| {
                c.as_element().is_some_and(|c| {
                    c.head
                        .attr("name")
                        .is_some_and(|n| n.to_ascii_lowercase().as_str() > "hyperlink")
                })
            })
            .unwrap_or(sec.children.len());
        sec.children.insert(pos, RawXml::Element(style));
        fix_count(sec);
        Ok(())
    }

    fn remove_hyperlink_exact(&mut self, sheet: usize, range: Rect) -> Result<bool, XlsxError> {
        let sheet_part = self
            .sheet_part(sheet)
            .cloned()
            .ok_or(XlsxError::NotAWorksheet(sheet))?;
        let text = range_a1(range);
        let ws = self.worksheet_mut(sheet)?;
        let Some(mut list) = ws.root_element("hyperlinks").cloned() else {
            return Ok(false);
        };
        let mut dropped_ids: Vec<String> = Vec::new();
        list.children.retain(|c| match c {
            RawXml::Element(h)
                if h.head.is_main("hyperlink") && h.head.attr("ref") == Some(text.as_str()) =>
            {
                if let Some(id) = h.head.attr_r("id") {
                    dropped_ids.push(id.to_owned());
                }
                false
            }
            _ => true,
        });
        let any = list.elements().next().is_some();
        let removed = !dropped_ids.is_empty() || {
            let before = ws
                .root_element("hyperlinks")
                .map_or(0, |l| l.elements().count());
            before != list.elements().count()
        };
        ws.put_root_element("hyperlinks", any.then_some(list));
        for id in dropped_ids {
            self.remove_relationship(Some(&sheet_part), &id)?;
        }
        self.sheets[sheet].dirty = true;
        Ok(removed)
    }

    /// Removes the hyperlink on the cell at `row`, `col` (a link whose range contains it).
    ///
    /// # Errors
    /// The sheet is not a worksheet.
    pub fn remove_hyperlink(
        &mut self,
        sheet: usize,
        row: u32,
        col: u32,
    ) -> Result<bool, XlsxError> {
        let Some(hit) = self
            .hyperlinks(sheet)
            .into_iter()
            .find(|h| h.range.0 <= row && row <= h.range.2 && h.range.1 <= col && col <= h.range.3)
        else {
            return Ok(false);
        };
        self.edited()?;
        self.remove_hyperlink_exact(sheet, hit.range)
    }
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(v) = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
