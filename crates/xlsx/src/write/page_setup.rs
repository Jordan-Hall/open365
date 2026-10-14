//! Page setup of a worksheet: margins, orientation, paper size, scaling, fit to pages, printing of
//! gridlines and headings, centring, manual page breaks, the print area and the print titles.
//!
//! The first group lives in the worksheet (`sheetPr/pageSetUpPr`, `printOptions`, `pageMargins`,
//! `pageSetup`, `rowBreaks`, `colBreaks`); the print area and titles are the sheet-scoped defined
//! names `_xlnm.Print_Area` and `_xlnm.Print_Titles`, as Excel stores them. Attributes of
//! `pageSetup` this module does not model (printer settings `r:id`, `pageOrder`, `copies`, ...)
//! are kept when the element is edited.

use super::NameSpec;
use crate::XlsxError;
use crate::refs::{MAX_COL, MAX_ROW, col_letters, col_number, parse_cell_ref};
use crate::sheet::Rect;
use crate::workbook::{DefinedName, Workbook};
use crate::xml::{RawXml, XmlElement};

const MAIN: &str = crate::xml::MAIN_NS;

/// Page margins in inches.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageMargins {
    pub left: f64,
    pub right: f64,
    pub top: f64,
    pub bottom: f64,
    pub header: f64,
    pub footer: f64,
}

impl Default for PageMargins {
    /// The Normal margins of a new worksheet.
    fn default() -> Self {
        Self {
            left: 0.7,
            right: 0.7,
            top: 0.75,
            bottom: 0.75,
            header: 0.3,
            footer: 0.3,
        }
    }
}

/// The page setup of one worksheet.
#[derive(Debug, Clone, PartialEq)]
pub struct PageSetup {
    pub margins: PageMargins,
    pub landscape: bool,
    /// `paperSize` code (1 Letter, 9 A4, ...); `None` when the file does not say.
    pub paper_size: Option<u32>,
    /// Scale in percent (10 to 400) used when `fit_to_page` is off.
    pub scale: u32,
    /// Fit to pages (`fitToPage`): `fit_width` and `fit_height` count pages, 0 is Automatic.
    pub fit_to_page: bool,
    pub fit_width: u32,
    pub fit_height: u32,
    pub print_gridlines: bool,
    pub print_headings: bool,
    pub center_horizontally: bool,
    pub center_vertically: bool,
    pub first_page_number: Option<i32>,
    /// A manual page break above each of these rows (the first row of the new page, 2 or more).
    pub row_breaks: Vec<u32>,
    /// A manual page break left of each of these columns (the first column of the new page).
    pub col_breaks: Vec<u32>,
    /// Print area ranges `(row1, col1, row2, col2)`; empty prints the used range.
    pub print_area: Vec<Rect>,
    /// Rows repeated at the top of every page, `(first, last)`.
    pub title_rows: Option<(u32, u32)>,
    /// Columns repeated at the left of every page, `(first, last)`.
    pub title_cols: Option<(u32, u32)>,
}

impl Default for PageSetup {
    fn default() -> Self {
        Self {
            margins: PageMargins::default(),
            landscape: false,
            paper_size: None,
            scale: 100,
            fit_to_page: false,
            fit_width: 0,
            fit_height: 0,
            print_gridlines: false,
            print_headings: false,
            center_horizontally: false,
            center_vertically: false,
            first_page_number: None,
            row_breaks: Vec::new(),
            col_breaks: Vec::new(),
            print_area: Vec::new(),
            title_rows: None,
            title_cols: None,
        }
    }
}

const PRINT_AREA: &str = "_xlnm.Print_Area";
const PRINT_TITLES: &str = "_xlnm.Print_Titles";

fn flag(el: &XmlElement, name: &str) -> bool {
    matches!(el.head.attr(name).map(str::trim), Some("1" | "true"))
}

fn number(el: &XmlElement, name: &str) -> Option<f64> {
    el.head.attr(name)?.trim().parse().ok()
}

fn fmt_inches(v: f64) -> String {
    let s = format!("{v:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() {
        "0".to_owned()
    } else {
        s.to_owned()
    }
}

fn quote_sheet(name: &str) -> String {
    let plain = !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name.chars().all(|c| c.is_alphanumeric() || c == '_')
        && parse_cell_ref(name).is_none();
    if plain {
        name.to_owned()
    } else {
        format!("'{}'", name.replace('\'', "''"))
    }
}

fn rect_text(r: Rect) -> String {
    let (r1, c1, r2, c2) = r;
    if c1 == 1 && c2 == MAX_COL && !(r1 == 1 && r2 == MAX_ROW) {
        format!("${r1}:${r2}")
    } else if r1 == 1 && r2 == MAX_ROW {
        format!("${}:${}", col_letters(c1), col_letters(c2))
    } else if (r1, c1) == (r2, c2) {
        format!("${}${r1}", col_letters(c1))
    } else {
        format!("${}${r1}:${}${r2}", col_letters(c1), col_letters(c2))
    }
}

/// Splits a defined-name formula at the commas outside quotes.
fn split_union(formula: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut start, mut quoted) = (0, false);
    for (i, ch) in formula.char_indices() {
        match ch {
            '\'' => quoted = !quoted,
            ',' if !quoted => {
                parts.push(&formula[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&formula[start..]);
    parts
}

/// The reference after the sheet prefix (`Sheet1!$A$1:$B$2` gives `$A$1:$B$2`).
fn after_sheet(part: &str) -> &str {
    let part = part.trim();
    let mut quoted = false;
    let mut at = None;
    for (i, ch) in part.char_indices() {
        match ch {
            '\'' => quoted = !quoted,
            '!' if !quoted => at = Some(i),
            _ => {}
        }
    }
    at.map_or(part, |i| &part[i + 1..])
}

/// `$A$1:$C$5`, `$A$1`, `$1:$3` or `$A:$C` as a rectangle.
fn parse_area(text: &str) -> Option<Rect> {
    let t = text.replace('$', "");
    let (a, b) = t.split_once(':').unwrap_or((&t, &t));
    if let (Ok(x), Ok(y)) = (a.parse::<u32>(), b.parse::<u32>()) {
        let (x, y) = (x.min(y), x.max(y));
        return (x >= 1 && y <= MAX_ROW).then_some((x, 1, y, MAX_COL));
    }
    if let (Some(x), Some(y)) = (col_number(a), col_number(b)) {
        let (x, y) = (x.min(y), x.max(y));
        return Some((1, x, MAX_ROW, y));
    }
    let (r1, c1) = parse_cell_ref(a)?;
    let (r2, c2) = parse_cell_ref(b)?;
    Some((r1.min(r2), c1.min(c2), r1.max(r2), c1.max(c2)))
}

fn breaks_of(el: Option<&XmlElement>) -> Vec<u32> {
    let mut out: Vec<u32> = el
        .into_iter()
        .flat_map(XmlElement::elements)
        .filter(|b| b.head.is_main("brk"))
        .filter_map(|b| b.head.attr("id")?.trim().parse::<u32>().ok())
        // `id` is the row (column) before the break; the model keeps the first one after it.
        .map(|id| id + 1)
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

impl Workbook {
    /// The page setup of `sheet`; the defaults for a sheet that has none.
    #[must_use]
    pub fn page_setup(&self, sheet: usize) -> PageSetup {
        let mut p = PageSetup::default();
        let Some(ws) = self.sheet(sheet) else {
            return p;
        };
        let find = |name: &str| {
            ws.raw_after()
                .iter()
                .filter_map(RawXml::as_element)
                .find(|e| e.head.is_main(name))
        };
        if let Some(el) = find("pageMargins") {
            let m = &mut p.margins;
            for (slot, name) in [
                (&mut m.left, "left"),
                (&mut m.right, "right"),
                (&mut m.top, "top"),
                (&mut m.bottom, "bottom"),
                (&mut m.header, "header"),
                (&mut m.footer, "footer"),
            ] {
                if let Some(v) = number(el, name) {
                    *slot = v;
                }
            }
        }
        if let Some(el) = find("printOptions") {
            p.print_gridlines = flag(el, "gridLines");
            p.print_headings = flag(el, "headings");
            p.center_horizontally = flag(el, "horizontalCentered");
            p.center_vertically = flag(el, "verticalCentered");
        }
        p.fit_to_page = ws
            .raw_before()
            .iter()
            .filter_map(RawXml::as_element)
            .find(|e| e.head.is_main("sheetPr"))
            .and_then(|e| e.child_main("pageSetUpPr"))
            .is_some_and(|e| flag(e, "fitToPage"));
        if let Some(el) = find("pageSetup") {
            p.landscape = el.head.attr("orientation") == Some("landscape");
            p.paper_size = el
                .head
                .attr("paperSize")
                .and_then(|v| v.trim().parse().ok());
            p.scale = el
                .head
                .attr("scale")
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(100);
            p.first_page_number = el
                .head
                .attr("firstPageNumber")
                .and_then(|v| v.trim().parse().ok());
            if p.fit_to_page {
                let pages = |name: &str| {
                    el.head
                        .attr(name)
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(1)
                };
                p.fit_width = pages("fitToWidth");
                p.fit_height = pages("fitToHeight");
            }
        } else if p.fit_to_page {
            (p.fit_width, p.fit_height) = (1, 1);
        }
        p.row_breaks = breaks_of(find("rowBreaks"));
        p.col_breaks = breaks_of(find("colBreaks"));
        for n in self.sheet_defined_names(sheet) {
            if n.name.eq_ignore_ascii_case(PRINT_AREA) {
                p.print_area = split_union(&n.formula)
                    .into_iter()
                    .filter_map(|s| parse_area(after_sheet(s)))
                    .collect();
            } else if n.name.eq_ignore_ascii_case(PRINT_TITLES) {
                for rect in split_union(&n.formula)
                    .into_iter()
                    .filter_map(|s| parse_area(after_sheet(s)))
                {
                    if rect.1 == 1 && rect.3 == MAX_COL {
                        p.title_rows = Some((rect.0, rect.2));
                    } else if rect.0 == 1 && rect.2 == MAX_ROW {
                        p.title_cols = Some((rect.1, rect.3));
                    }
                }
            }
        }
        p
    }

    /// Writes `p` as the page setup of `sheet`.
    ///
    /// # Errors
    /// The sheet does not exist or is not a worksheet.
    pub fn set_page_setup(&mut self, sheet: usize, p: &PageSetup) -> Result<(), XlsxError> {
        self.edited()?;
        let sheet_name =
            self.sheets()
                .get(sheet)
                .map(|s| s.name.clone())
                .ok_or(XlsxError::SheetOutOfRange {
                    index: sheet,
                    count: self.sheets().len(),
                })?;
        {
            let ws = self.worksheet_mut(sheet)?;
            put_fit_to_page(ws, p.fit_to_page)?;
            put_print_options(ws, p)?;
            let margins = p.margins;
            let el = ws.fragment(&format!(
                "<pageMargins xmlns=\"{MAIN}\" left=\"{}\" right=\"{}\" top=\"{}\" bottom=\"{}\" header=\"{}\" footer=\"{}\"/>",
                fmt_inches(margins.left),
                fmt_inches(margins.right),
                fmt_inches(margins.top),
                fmt_inches(margins.bottom),
                fmt_inches(margins.header),
                fmt_inches(margins.footer),
            ))?;
            ws.put_root_element("pageMargins", Some(el));
            put_page_setup(ws, p)?;
            put_breaks(ws, "rowBreaks", &p.row_breaks, u64::from(MAX_COL - 1))?;
            put_breaks(ws, "colBreaks", &p.col_breaks, u64::from(MAX_ROW - 1))?;
        }
        self.sheets[sheet].dirty = true;
        let q = quote_sheet(&sheet_name);
        let area = p
            .print_area
            .iter()
            .map(|r| format!("{q}!{}", rect_text(*r)))
            .collect::<Vec<_>>()
            .join(",");
        self.put_print_name(sheet, PRINT_AREA, (!area.is_empty()).then_some(area))?;
        let mut titles = Vec::new();
        if let Some((a, b)) = p.title_cols {
            titles.push(format!("{q}!{}", rect_text((1, a, MAX_ROW, b))));
        }
        if let Some((a, b)) = p.title_rows {
            titles.push(format!("{q}!{}", rect_text((a, 1, b, MAX_COL))));
        }
        let titles = titles.join(",");
        self.put_print_name(sheet, PRINT_TITLES, (!titles.is_empty()).then_some(titles))
    }

    /// Sets, replaces or removes the sheet-scoped built-in name `name`. The formula is stored as
    /// given: a union of areas (`Sheet1!$A$1:$B$2,Sheet1!$D$1`) is what Excel writes for a print
    /// area, but the defined-name editor only takes formulas it can parse, so the list is edited
    /// directly and the names element is rewritten through a throw-away define and delete.
    fn put_print_name(
        &mut self,
        sheet: usize,
        name: &str,
        formula: Option<String>,
    ) -> Result<(), XlsxError> {
        let scope = u32::try_from(sheet).ok();
        let at = self
            .defined_names
            .iter()
            .position(|n| n.local_sheet_id == scope && n.name.eq_ignore_ascii_case(name));
        match (at, formula) {
            (Some(i), Some(f)) => {
                if self.defined_names[i].formula == f {
                    return Ok(());
                }
                self.defined_names[i].formula = f;
            }
            (Some(i), None) => {
                self.defined_names.remove(i);
            }
            (None, Some(f)) => self.defined_names.push(DefinedName {
                name: name.to_owned(),
                local_sheet_id: scope,
                hidden: false,
                formula: f,
                comment: None,
            }),
            (None, None) => return Ok(()),
        }
        let tmp = NameSpec {
            name: "_xlnm.Sync_Names".to_owned(),
            scope: None,
            formula: "1".to_owned(),
            comment: None,
            hidden: false,
        };
        self.define_name(&tmp)?;
        self.delete_defined_name(&tmp.name, None)?;
        Ok(())
    }
}

fn put_fit_to_page(ws: &mut crate::sheet::Worksheet, on: bool) -> Result<(), XlsxError> {
    let existing = ws
        .raw_before()
        .iter()
        .filter_map(RawXml::as_element)
        .find(|e| e.head.is_main("sheetPr"))
        .cloned();
    let mut sheet_pr = match (existing, on) {
        (Some(e), _) => e,
        (None, true) => ws.fragment(&format!("<sheetPr xmlns=\"{MAIN}\"/>"))?,
        (None, false) => return Ok(()),
    };
    let at = sheet_pr.children.iter().position(|c| {
        c.as_element()
            .is_some_and(|e| e.head.is_main("pageSetUpPr"))
    });
    match (at, on) {
        (Some(i), true) => {
            if let Some(e) = sheet_pr.children[i].as_element_mut() {
                e.head.set_attr("fitToPage", "1");
            }
        }
        (Some(i), false) => {
            if let Some(e) = sheet_pr.children[i].as_element_mut() {
                e.head.remove_attr("fitToPage");
                if e.head.attrs.is_empty() {
                    sheet_pr.children.remove(i);
                }
            }
        }
        (None, true) => {
            let child = ws.fragment(&format!("<pageSetUpPr xmlns=\"{MAIN}\" fitToPage=\"1\"/>"))?;
            sheet_pr.children.push(RawXml::Element(child));
        }
        (None, false) => {}
    }
    let empty = sheet_pr.children.is_empty()
        && sheet_pr
            .head
            .attrs
            .iter()
            .all(|a| a.ns.is_none() && a.name.starts_with("xmlns"));
    ws.put_before_element("sheetPr", (!empty).then_some(sheet_pr));
    Ok(())
}

fn put_print_options(ws: &mut crate::sheet::Worksheet, p: &PageSetup) -> Result<(), XlsxError> {
    let mut attrs = String::new();
    for (on, name) in [
        (p.center_horizontally, "horizontalCentered"),
        (p.center_vertically, "verticalCentered"),
        (p.print_headings, "headings"),
        (p.print_gridlines, "gridLines"),
    ] {
        if on {
            attrs.push_str(&format!(" {name}=\"1\""));
        }
    }
    let kept_grid_set = ws
        .raw_after()
        .iter()
        .filter_map(RawXml::as_element)
        .find(|e| e.head.is_main("printOptions"))
        .and_then(|e| e.head.attr("gridLinesSet").map(str::to_owned));
    if let Some(v) = kept_grid_set {
        attrs.push_str(&format!(" gridLinesSet=\"{v}\""));
    }
    let el = if attrs.is_empty() {
        None
    } else {
        Some(ws.fragment(&format!("<printOptions xmlns=\"{MAIN}\"{attrs}/>"))?)
    };
    ws.put_root_element("printOptions", el);
    Ok(())
}

fn put_page_setup(ws: &mut crate::sheet::Worksheet, p: &PageSetup) -> Result<(), XlsxError> {
    let existing = ws
        .raw_after()
        .iter()
        .filter_map(RawXml::as_element)
        .find(|e| e.head.is_main("pageSetup"))
        .cloned();
    let mut el = match existing {
        Some(e) => e,
        None => ws.fragment(&format!("<pageSetup xmlns=\"{MAIN}\"/>"))?,
    };
    let set = |el: &mut XmlElement, name: &str, value: Option<String>| match value {
        Some(v) => el.head.set_attr(name, &v),
        None => el.head.remove_attr(name),
    };
    set(&mut el, "paperSize", p.paper_size.map(|v| v.to_string()));
    set(
        &mut el,
        "scale",
        (p.scale != 100).then(|| p.scale.to_string()),
    );
    set(
        &mut el,
        "firstPageNumber",
        p.first_page_number.map(|v| v.to_string()),
    );
    if p.first_page_number.is_some() {
        el.head.set_attr("useFirstPageNumber", "1");
    } else {
        el.head.remove_attr("useFirstPageNumber");
    }
    set(
        &mut el,
        "fitToWidth",
        (p.fit_to_page && p.fit_width != 1).then(|| p.fit_width.to_string()),
    );
    set(
        &mut el,
        "fitToHeight",
        (p.fit_to_page && p.fit_height != 1).then(|| p.fit_height.to_string()),
    );
    set(
        &mut el,
        "orientation",
        Some(if p.landscape { "landscape" } else { "portrait" }.to_owned()),
    );
    ws.put_root_element("pageSetup", Some(el));
    Ok(())
}

fn put_breaks(
    ws: &mut crate::sheet::Worksheet,
    name: &str,
    breaks: &[u32],
    max: u64,
) -> Result<(), XlsxError> {
    let mut ids: Vec<u32> = breaks.iter().filter(|b| **b >= 2).map(|b| b - 1).collect();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        ws.put_root_element(name, None);
        return Ok(());
    }
    let list: String = ids
        .iter()
        .map(|id| format!("<brk id=\"{id}\" max=\"{max}\" man=\"1\"/>"))
        .collect();
    let el = ws.fragment(&format!(
        "<{name} xmlns=\"{MAIN}\" count=\"{n}\" manualBreakCount=\"{n}\">{list}</{name}>",
        n = ids.len()
    ))?;
    ws.put_root_element(name, Some(el));
    Ok(())
}
