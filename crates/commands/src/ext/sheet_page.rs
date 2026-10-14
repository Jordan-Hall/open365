//! Sheets Page Layout, Review and View operations that write worksheet structures: the page setup
//! (margins, orientation, size, scaling, print options, breaks, print area and titles), Allow Edit
//! Ranges (`protectedRanges`), Notes > Convert to Comments, and Custom Views
//! (`customWorkbookViews` and `customSheetViews`). Each runs as one undo step through
//! [`crate::Command::SheetPage`].

use open365_xlsx::{PageMargins, PageSetup, Workbook};
use serde::{Deserialize, Serialize};

use super::review::{self, ReviewOp};
use super::xmlpatch::{self, xml_escape};
use crate::error::CommandError;
use crate::xlsx_state::XlsxState;

fn bad(text: &str) -> CommandError {
    CommandError::Format(text.to_owned())
}

fn d100() -> u32 {
    100
}

/// Page margins in inches.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MarginsRequest {
    pub left: f64,
    pub right: f64,
    pub top: f64,
    pub bottom: f64,
    pub header: f64,
    pub footer: f64,
}

/// A worksheet's page setup. Ranges are `(row1, col1, row2, col2)`; a break at `n` puts row (or
/// column) `n` on a new page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageSetupRequest {
    pub margins: MarginsRequest,
    #[serde(default)]
    pub landscape: bool,
    /// The `paperSize` code (1 Letter, 9 A4, ...).
    #[serde(default)]
    pub paper_size: Option<u32>,
    #[serde(default = "d100")]
    pub scale: u32,
    #[serde(default)]
    pub fit_to_page: bool,
    #[serde(default)]
    pub fit_width: u32,
    #[serde(default)]
    pub fit_height: u32,
    #[serde(default)]
    pub print_gridlines: bool,
    #[serde(default)]
    pub print_headings: bool,
    #[serde(default)]
    pub center_horizontally: bool,
    #[serde(default)]
    pub center_vertically: bool,
    #[serde(default)]
    pub first_page_number: Option<i32>,
    #[serde(default)]
    pub row_breaks: Vec<u32>,
    #[serde(default)]
    pub col_breaks: Vec<u32>,
    #[serde(default)]
    pub print_area: Vec<(u32, u32, u32, u32)>,
    #[serde(default)]
    pub title_rows: Option<(u32, u32)>,
    #[serde(default)]
    pub title_cols: Option<(u32, u32)>,
}

impl From<&PageSetup> for PageSetupRequest {
    fn from(p: &PageSetup) -> Self {
        let m = p.margins;
        Self {
            margins: MarginsRequest {
                left: m.left,
                right: m.right,
                top: m.top,
                bottom: m.bottom,
                header: m.header,
                footer: m.footer,
            },
            landscape: p.landscape,
            paper_size: p.paper_size,
            scale: p.scale,
            fit_to_page: p.fit_to_page,
            fit_width: p.fit_width,
            fit_height: p.fit_height,
            print_gridlines: p.print_gridlines,
            print_headings: p.print_headings,
            center_horizontally: p.center_horizontally,
            center_vertically: p.center_vertically,
            first_page_number: p.first_page_number,
            row_breaks: p.row_breaks.clone(),
            col_breaks: p.col_breaks.clone(),
            print_area: p.print_area.clone(),
            title_rows: p.title_rows,
            title_cols: p.title_cols,
        }
    }
}

impl From<&PageSetupRequest> for PageSetup {
    fn from(r: &PageSetupRequest) -> Self {
        let m = r.margins;
        Self {
            margins: PageMargins {
                left: m.left,
                right: m.right,
                top: m.top,
                bottom: m.bottom,
                header: m.header,
                footer: m.footer,
            },
            landscape: r.landscape,
            paper_size: r.paper_size,
            scale: r.scale.clamp(10, 400),
            fit_to_page: r.fit_to_page,
            fit_width: r.fit_width,
            fit_height: r.fit_height,
            print_gridlines: r.print_gridlines,
            print_headings: r.print_headings,
            center_horizontally: r.center_horizontally,
            center_vertically: r.center_vertically,
            first_page_number: r.first_page_number,
            row_breaks: r.row_breaks.clone(),
            col_breaks: r.col_breaks.clone(),
            print_area: r.print_area.clone(),
            title_rows: r.title_rows,
            title_cols: r.title_cols,
        }
    }
}

/// The page setup of `sheet` as the file holds it.
#[must_use]
pub fn page_setup(wb: &Workbook, sheet: usize) -> PageSetupRequest {
    PageSetupRequest::from(&wb.page_setup(sheet))
}

/// One range of Allow Edit Ranges.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditRange {
    pub title: String,
    /// The cells, as `A1:B2` (several areas are separated by a space).
    pub refers_to: String,
    /// The legacy password hash (`password` attribute); `None` lets everybody edit.
    #[serde(default)]
    pub password_hash: Option<String>,
}

/// One worksheet's part of a custom view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomViewSheet {
    pub zoom: u32,
    /// `normal`, `pageBreakPreview` or `pageLayout`.
    pub view: String,
    pub gridlines: bool,
    pub headings: bool,
    pub frozen_rows: u32,
    pub frozen_cols: u32,
    pub top_left: (u32, u32),
    pub active: (u32, u32),
}

/// A named view of the whole workbook (View > Custom Views). One entry per worksheet, in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomView {
    pub name: String,
    /// The view's GUID; empty for a view not saved yet (derived from the name when saved).
    #[serde(default)]
    pub guid: String,
    pub formula_bar: bool,
    pub active_sheet: usize,
    pub sheets: Vec<CustomViewSheet>,
}

/// What a [`crate::Command::SheetPage`] does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum SheetPageOp {
    /// Page Layout: writes the whole page setup of the sheet.
    SetPageSetup(PageSetupRequest),
    /// Review > Allow Edit Ranges: replaces the ranges of the sheet.
    SetEditRanges(Vec<EditRange>),
    /// Review > Notes > Convert to Comments: every note of the workbook becomes a thread.
    ConvertNotesToComments,
    /// View > Custom Views > Add: a new view, or the one of the same name replaced.
    SaveCustomView(CustomView),
    DeleteCustomView {
        name: String,
    },
}

impl SheetPageOp {
    pub(crate) fn destroys(&self) -> bool {
        matches!(self, Self::DeleteCustomView { .. })
    }
}

pub(crate) fn run(
    x: &mut XlsxState,
    sheet: usize,
    op: &SheetPageOp,
    notes_out: &mut Vec<String>,
) -> Result<Vec<crate::command::CellRef>, CommandError> {
    match op {
        SheetPageOp::SetPageSetup(r) => {
            x.workbook.set_page_setup(sheet, &PageSetup::from(r))?;
            let wb = x.workbook.clone();
            x.restore_workbook(wb);
        }
        SheetPageOp::SetEditRanges(list) => set_edit_ranges(x, sheet, list)?,
        SheetPageOp::ConvertNotesToComments => convert_notes(x, notes_out)?,
        SheetPageOp::SaveCustomView(v) => save_view(x, v)?,
        SheetPageOp::DeleteCustomView { name } => delete_view(x, name)?,
    }
    Ok(Vec::new())
}

// ---- Allow Edit Ranges ----------------------------------------------------------------------

/// Elements that follow `protectedRanges` in the worksheet schema.
const AFTER_PROTECTED: &[&str] = &[
    "scenarios",
    "autoFilter",
    "sortState",
    "dataConsolidate",
    "customSheetViews",
    "mergeCells",
    "phoneticPr",
    "conditionalFormatting",
    "dataValidations",
    "hyperlinks",
    "printOptions",
    "pageMargins",
    "pageSetup",
    "headerFooter",
    "rowBreaks",
    "colBreaks",
    "customProperties",
    "cellWatches",
    "ignoredErrors",
    "smartTags",
    "drawing",
    "legacyDrawing",
    "legacyDrawingHF",
    "drawingHF",
    "picture",
    "oleObjects",
    "controls",
    "webPublishItems",
    "tableParts",
    "extLst",
];

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// The Allow Edit Ranges of `sheet`.
#[must_use]
pub fn edit_ranges(wb: &Workbook, sheet: usize) -> Vec<EditRange> {
    let Some(ws) = wb.sheet(sheet) else {
        return Vec::new();
    };
    ws.raw_after()
        .iter()
        .filter_map(open365_xlsx::RawXml::as_element)
        .filter(|e| e.head.is_main("protectedRanges"))
        .flat_map(open365_xlsx::XmlElement::elements)
        .filter(|e| e.head.is_main("protectedRange"))
        .map(|e| EditRange {
            title: e.head.attr("name").unwrap_or_default().to_owned(),
            refers_to: e.head.attr("sqref").unwrap_or_default().to_owned(),
            password_hash: e.head.attr("password").map(str::to_owned),
        })
        .collect()
}

fn set_edit_ranges(
    x: &mut XlsxState,
    sheet: usize,
    list: &[EditRange],
) -> Result<(), CommandError> {
    if review::sheet_protection(&x.workbook, sheet).is_some() {
        return Err(bad(
            "The sheet is protected; unprotect it to change the ranges users may edit.",
        ));
    }
    let mut seen: Vec<String> = Vec::new();
    for r in list {
        if r.title.trim().is_empty() {
            return Err(bad("A range needs a title."));
        }
        if r.refers_to.trim().is_empty() {
            return Err(bad("A range needs cells to refer to."));
        }
        let key = r.title.to_lowercase();
        if seen.contains(&key) {
            return Err(bad("Two ranges have the same title."));
        }
        seen.push(key);
    }
    let part = xmlpatch::sheet_part_name(&x.workbook, sheet)?;
    let element = if list.is_empty() {
        None
    } else {
        let mut el = String::from("<protectedRanges>");
        for r in list {
            el.push_str(&format!(
                "<protectedRange{} sqref=\"{}\" name=\"{}\"/>",
                r.password_hash
                    .as_ref()
                    .map_or_else(String::new, |h| format!(" password=\"{}\"", xml_escape(h))),
                xml_escape(r.refers_to.trim()),
                xml_escape(r.title.trim()),
            ));
        }
        el.push_str("</protectedRanges>");
        Some(el)
    };
    let wb = xmlpatch::rewrite(&x.workbook, |pkg| {
        let mut xml = xmlpatch::part_text(pkg, &part)?;
        xmlpatch::remove_element(&mut xml, "protectedRanges");
        if let Some(el) = &element {
            xmlpatch::insert_before_any(&mut xml, el, AFTER_PROTECTED);
        }
        xmlpatch::set_part_text(pkg, &part, xml)
    })?;
    x.restore_workbook(wb);
    Ok(())
}

/// The legacy hash of an Allow Edit Ranges password.
#[must_use]
pub fn range_password_hash(password: &str) -> String {
    review::legacy_password_hash(password)
}

// ---- Notes to comments ----------------------------------------------------------------------

fn convert_notes(x: &mut XlsxState, notes_out: &mut Vec<String>) -> Result<(), CommandError> {
    let mut converted = 0usize;
    for sheet in 0..x.workbook.sheets().len() {
        for note in review::notes(&x.workbook, sheet) {
            let mut sink = Vec::new();
            // Excel refuses a comment on a cell with a note, so the note goes first.
            review::run(
                x,
                sheet,
                &ReviewOp::DeleteNote {
                    row: note.row,
                    col: note.col,
                },
                &mut sink,
            )?;
            review::run(
                x,
                sheet,
                &ReviewOp::AddComment {
                    row: note.row,
                    col: note.col,
                    author: note.author.clone(),
                    text: note.text.clone(),
                },
                &mut sink,
            )?;
            converted += 1;
        }
    }
    if converted == 0 {
        return Err(bad("There are no notes in this workbook."));
    }
    notes_out.push(format!("{converted} notes converted to comments."));
    Ok(())
}

// ---- Custom views ---------------------------------------------------------------------------

const AFTER_CUSTOM_BOOK_VIEWS: &[&str] = &[
    "pivotCaches",
    "smartTagPr",
    "smartTagTypes",
    "webPublishing",
    "fileRecoveryPr",
    "webPublishObjects",
    "extLst",
];

/// A stable GUID for a view name, in the `{8-4-4-4-12}` form Excel writes.
fn guid_of(name: &str) -> String {
    let mut h: [u64; 2] = [0xcbf2_9ce4_8422_2325, 0x8422_2325_cbf2_9ce4];
    for (i, b) in name.to_lowercase().bytes().enumerate() {
        let k = i % 2;
        h[k] ^= u64::from(b);
        h[k] = h[k].wrapping_mul(0x0000_0100_0000_01b3);
    }
    let hex = format!("{:016X}{:016X}", h[0], h[1]);
    format!(
        "{{{}-{}-{}-{}-{}}}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// Every `<tag ...>...</tag>` or `<tag .../>` of `xml`, as text.
fn split_elements(xml: &str, tag: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(i) = xmlpatch::find_open(rest, tag) {
        let Some(gt) = rest[i..].find('>') else { break };
        let open_end = i + gt + 1;
        let end = if rest[i..open_end].ends_with("/>") {
            open_end
        } else {
            let close = format!("</{tag}>");
            match rest[open_end..].find(&close) {
                Some(n) => open_end + n + close.len(),
                None => break,
            }
        };
        out.push(rest[i..end].to_owned());
        rest = &rest[end..];
    }
    out
}

fn view_of_sheet(el: &str) -> CustomViewSheet {
    let attr = |n: &str| xmlpatch::attr(el, n);
    let flag = |n: &str| attr(n).as_deref() != Some("0");
    let cell = |t: &str| {
        open365_xlsx::parse_range(t.split_whitespace().next().unwrap_or(t))
            .map(|(r, c, _, _)| (r, c))
    };
    let pane = split_elements(el, "pane").into_iter().next();
    let (fr, fc) = pane.as_deref().map_or((0, 0), |p| {
        let n = |k: &str| xmlpatch::attr(p, k).and_then(|v| v.parse::<f64>().ok());
        let frozen =
            xmlpatch::attr(p, "state").is_some_and(|s| s == "frozen" || s == "frozenSplit");
        if frozen {
            (
                n("ySplit").unwrap_or(0.0) as u32,
                n("xSplit").unwrap_or(0.0) as u32,
            )
        } else {
            (0, 0)
        }
    });
    let top_left = pane
        .as_deref()
        .and_then(|p| xmlpatch::attr(p, "topLeftCell"))
        .or_else(|| attr("topLeftCell"))
        .and_then(|t| cell(&t))
        .unwrap_or((1, 1));
    let active = split_elements(el, "selection")
        .last()
        .and_then(|s| xmlpatch::attr(s, "activeCell"))
        .and_then(|t| cell(&t))
        .unwrap_or((1, 1));
    CustomViewSheet {
        zoom: attr("scale").and_then(|v| v.parse().ok()).unwrap_or(100),
        view: attr("view").unwrap_or_else(|| "normal".to_owned()),
        gridlines: flag("showGridLines"),
        headings: flag("showRowCol"),
        frozen_rows: fr,
        frozen_cols: fc,
        top_left,
        active,
    }
}

fn sheet_views_xml(wb: &Workbook, sheet: usize) -> String {
    let Some(ws) = wb.sheet(sheet) else {
        return String::new();
    };
    let mut out = String::new();
    for node in ws.raw_after() {
        if node
            .as_element()
            .is_some_and(|e| e.head.is_main("customSheetViews"))
        {
            node.write(&mut out);
        }
    }
    out
}

/// The workbook's custom views, in file order.
#[must_use]
pub fn custom_views(wb: &Workbook) -> Vec<CustomView> {
    let Some(book) = wb
        .package()
        .part(wb.workbook_part().as_str())
        .map(|p| String::from_utf8_lossy(p.data()).into_owned())
    else {
        return Vec::new();
    };
    let per_sheet: Vec<String> = (0..wb.sheets().len())
        .map(|i| sheet_views_xml(wb, i))
        .collect();
    split_elements(&book, "customWorkbookView")
        .iter()
        .filter_map(|tag| {
            let guid = xmlpatch::attr(tag, "guid")?;
            let name = unescape(&xmlpatch::attr(tag, "name")?);
            let sheets = per_sheet
                .iter()
                .map(|xml| {
                    split_elements(xml, "customSheetView")
                        .into_iter()
                        .find(|v| xmlpatch::attr(v, "guid").as_deref() == Some(guid.as_str()))
                        .map(|v| view_of_sheet(&v))
                        .unwrap_or_else(|| CustomViewSheet {
                            zoom: 100,
                            view: "normal".to_owned(),
                            gridlines: true,
                            headings: true,
                            frozen_rows: 0,
                            frozen_cols: 0,
                            top_left: (1, 1),
                            active: (1, 1),
                        })
                })
                .collect();
            let active_id =
                xmlpatch::attr(tag, "activeSheetId").and_then(|v| v.parse::<u32>().ok());
            Some(CustomView {
                name,
                guid: guid.clone(),
                formula_bar: xmlpatch::attr(tag, "showFormulaBar").as_deref() != Some("0"),
                active_sheet: wb
                    .sheets()
                    .iter()
                    .position(|s| Some(s.sheet_id) == active_id)
                    .unwrap_or(0),
                sheets,
            })
        })
        .collect()
}

fn sheet_view_element(guid: &str, v: &CustomViewSheet) -> String {
    let cell = |(r, c): (u32, u32)| open365_xlsx::cell_ref_string(r, c);
    let mut el = format!("<customSheetView guid=\"{guid}\"");
    if v.zoom != 100 {
        el.push_str(&format!(" scale=\"{}\"", v.zoom));
    }
    if !v.gridlines {
        el.push_str(" showGridLines=\"0\"");
    }
    if !v.headings {
        el.push_str(" showRowCol=\"0\"");
    }
    if v.view != "normal" && !v.view.is_empty() {
        el.push_str(&format!(" view=\"{}\"", xml_escape(&v.view)));
    }
    let frozen = v.frozen_rows > 0 || v.frozen_cols > 0;
    if !frozen && v.top_left != (1, 1) {
        el.push_str(&format!(" topLeftCell=\"{}\"", cell(v.top_left)));
    }
    el.push('>');
    let selection_pane = if frozen {
        let pane = match (v.frozen_rows > 0, v.frozen_cols > 0) {
            (true, true) => "bottomRight",
            (true, false) => "bottomLeft",
            _ => "topRight",
        };
        let tl = (
            v.top_left.0.max(v.frozen_rows + 1),
            v.top_left.1.max(v.frozen_cols + 1),
        );
        el.push_str("<pane");
        if v.frozen_cols > 0 {
            el.push_str(&format!(" xSplit=\"{}\"", v.frozen_cols));
        }
        if v.frozen_rows > 0 {
            el.push_str(&format!(" ySplit=\"{}\"", v.frozen_rows));
        }
        el.push_str(&format!(
            " topLeftCell=\"{}\" activePane=\"{pane}\" state=\"frozen\"/>",
            cell(tl)
        ));
        format!(" pane=\"{pane}\"")
    } else {
        String::new()
    };
    el.push_str(&format!(
        "<selection{selection_pane} activeCell=\"{a}\" sqref=\"{a}\"/>",
        a = cell(v.active)
    ));
    el.push_str("</customSheetView>");
    el
}

fn book_view_element(wb: &Workbook, guid: &str, v: &CustomView) -> String {
    let id = wb
        .sheets()
        .get(v.active_sheet)
        .or_else(|| wb.sheets().first())
        .map_or(1, |s| s.sheet_id);
    format!(
        "<customWorkbookView name=\"{}\" guid=\"{guid}\" includePrintSettings=\"0\" includeHiddenRowCol=\"0\" maximized=\"1\" windowWidth=\"28800\" windowHeight=\"15600\" activeSheetId=\"{id}\"{}/>",
        xml_escape(&v.name),
        if v.formula_bar {
            ""
        } else {
            " showFormulaBar=\"0\""
        },
    )
}

/// Replaces every custom view with `views` (the GUIDs of views that stay keep their raw
/// `customSheetView` text, so print settings a file brought along survive).
fn write_views(
    x: &mut XlsxState,
    views: &[(String, CustomView, bool)],
) -> Result<(), CommandError> {
    let wb0 = &x.workbook;
    let book_part = wb0.workbook_part().as_str().to_owned();
    let sheet_count = wb0.sheets().len();
    let parts: Vec<String> = (0..sheet_count)
        .map(|i| xmlpatch::sheet_part_name(wb0, i))
        .collect::<Result<_, _>>()?;
    let old_sheet_views: Vec<String> = (0..sheet_count).map(|i| sheet_views_xml(wb0, i)).collect();
    let book_elements: Vec<String> = views
        .iter()
        .map(|(guid, v, _)| book_view_element(wb0, guid, v))
        .collect();
    let wb = xmlpatch::rewrite(wb0, |pkg| {
        let mut book = xmlpatch::part_text(pkg, &book_part)?;
        xmlpatch::remove_element(&mut book, "customWorkbookViews");
        if !book_elements.is_empty() {
            let el = format!(
                "<customWorkbookViews>{}</customWorkbookViews>",
                book_elements.concat()
            );
            // Insert before the first later element, else before the closing tag.
            let at = AFTER_CUSTOM_BOOK_VIEWS
                .iter()
                .filter_map(|n| xmlpatch::find_open(&book, n))
                .min()
                .or_else(|| book.rfind("</workbook>"));
            if let Some(at) = at {
                book.insert_str(at, &el);
            }
        }
        xmlpatch::set_part_text(pkg, &book_part, book)?;
        for (i, part) in parts.iter().enumerate() {
            let mut xml = xmlpatch::part_text(pkg, part)?;
            xmlpatch::remove_element(&mut xml, "customSheetViews");
            let mut inner = String::new();
            for (guid, v, fresh) in views {
                let kept = (!*fresh)
                    .then(|| {
                        split_elements(&old_sheet_views[i], "customSheetView")
                            .into_iter()
                            .find(|e| xmlpatch::attr(e, "guid").as_deref() == Some(guid.as_str()))
                    })
                    .flatten();
                match (kept, v.sheets.get(i)) {
                    (Some(raw), _) => inner.push_str(&raw),
                    (None, Some(s)) => inner.push_str(&sheet_view_element(guid, s)),
                    (None, None) => {}
                }
            }
            if !inner.is_empty() {
                let el = format!("<customSheetViews>{inner}</customSheetViews>");
                xmlpatch::insert_before_any(&mut xml, &el, AFTER_CUSTOM_SHEET_VIEWS);
            }
            xmlpatch::set_part_text(pkg, part, xml)?;
        }
        Ok(())
    })?;
    x.restore_workbook(wb);
    Ok(())
}

const AFTER_CUSTOM_SHEET_VIEWS: &[&str] = &[
    "mergeCells",
    "phoneticPr",
    "conditionalFormatting",
    "dataValidations",
    "hyperlinks",
    "printOptions",
    "pageMargins",
    "pageSetup",
    "headerFooter",
    "rowBreaks",
    "colBreaks",
    "customProperties",
    "cellWatches",
    "ignoredErrors",
    "smartTags",
    "drawing",
    "legacyDrawing",
    "legacyDrawingHF",
    "drawingHF",
    "picture",
    "oleObjects",
    "controls",
    "webPublishItems",
    "tableParts",
    "extLst",
];

fn save_view(x: &mut XlsxState, view: &CustomView) -> Result<(), CommandError> {
    let name = view.name.trim();
    if name.is_empty() {
        return Err(bad("A view needs a name."));
    }
    if name.chars().count() > 255 {
        return Err(bad("The view name is too long."));
    }
    let mut all: Vec<(String, CustomView, bool)> = custom_views(&x.workbook)
        .into_iter()
        .map(|v| (v.guid.clone(), v, false))
        .collect();
    let mut fresh = view.clone();
    fresh.name = name.to_owned();
    let guid = if view.guid.is_empty() {
        guid_of(name)
    } else {
        view.guid.clone()
    };
    fresh.guid.clone_from(&guid);
    match all
        .iter_mut()
        .find(|(_, v, _)| v.name.eq_ignore_ascii_case(name))
    {
        Some(slot) => *slot = (guid, fresh, true),
        None => all.push((guid, fresh, true)),
    }
    write_views(x, &all)
}

fn delete_view(x: &mut XlsxState, name: &str) -> Result<(), CommandError> {
    let all: Vec<(String, CustomView, bool)> = custom_views(&x.workbook)
        .into_iter()
        .map(|v| (v.guid.clone(), v, false))
        .collect();
    if !all
        .iter()
        .any(|(_, v, _)| v.name.eq_ignore_ascii_case(name))
    {
        return Err(bad("There is no view with that name."));
    }
    let rest: Vec<_> = all
        .into_iter()
        .filter(|(_, v, _)| !v.name.eq_ignore_ascii_case(name))
        .collect();
    write_views(x, &rest)
}
