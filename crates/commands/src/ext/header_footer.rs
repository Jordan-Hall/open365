//! Page Setup > Header/Footer: the worksheet's `headerFooter` element. Each text is the string
//! Excel stores, `&L` left, `&C` centre and `&R` right sections with `&P`, `&N`, ... codes.

use open365_xlsx::Workbook;
use serde::{Deserialize, Serialize};

use super::xmlpatch::{self, xml_escape};
use crate::error::CommandError;
use crate::xlsx_state::XlsxState;

fn yes() -> bool {
    true
}

/// A sheet's headers and footers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeaderFooterRequest {
    #[serde(default)]
    pub odd_header: String,
    #[serde(default)]
    pub odd_footer: String,
    #[serde(default)]
    pub even_header: String,
    #[serde(default)]
    pub even_footer: String,
    #[serde(default)]
    pub first_header: String,
    #[serde(default)]
    pub first_footer: String,
    #[serde(default)]
    pub different_odd_even: bool,
    #[serde(default)]
    pub different_first: bool,
    /// "Scale with document" (on by default).
    #[serde(default = "yes")]
    pub scale_with_doc: bool,
    /// "Align with page margins" (on by default).
    #[serde(default = "yes")]
    pub align_with_margins: bool,
}

impl Default for HeaderFooterRequest {
    fn default() -> Self {
        Self {
            odd_header: String::new(),
            odd_footer: String::new(),
            even_header: String::new(),
            even_footer: String::new(),
            first_header: String::new(),
            first_footer: String::new(),
            different_odd_even: false,
            different_first: false,
            scale_with_doc: true,
            align_with_margins: true,
        }
    }
}

/// Elements that follow `headerFooter` in the worksheet schema.
const AFTER_HEADER_FOOTER: &[&str] = &[
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

/// The `headerFooter` element of `h`; `None` when it holds nothing (Excel then omits it).
fn element(h: &HeaderFooterRequest) -> Option<String> {
    if *h == HeaderFooterRequest::default() {
        return None;
    }
    let mut el = String::from("<headerFooter");
    if h.different_odd_even {
        el.push_str(" differentOddEven=\"1\"");
    }
    if h.different_first {
        el.push_str(" differentFirst=\"1\"");
    }
    if !h.scale_with_doc {
        el.push_str(" scaleWithDoc=\"0\"");
    }
    if !h.align_with_margins {
        el.push_str(" alignWithMargins=\"0\"");
    }
    el.push('>');
    for (tag, text) in [
        ("oddHeader", &h.odd_header),
        ("oddFooter", &h.odd_footer),
        ("evenHeader", &h.even_header),
        ("evenFooter", &h.even_footer),
        ("firstHeader", &h.first_header),
        ("firstFooter", &h.first_footer),
    ] {
        if !text.is_empty() {
            el.push_str(&format!("<{tag}>{}</{tag}>", xml_escape(text)));
        }
    }
    el.push_str("</headerFooter>");
    Some(el)
}

/// The header and footer of `sheet`; the defaults (all empty) for a sheet without any.
#[must_use]
pub fn header_footer(wb: &Workbook, sheet: usize) -> HeaderFooterRequest {
    let mut h = HeaderFooterRequest::default();
    let Some(el) = wb.sheet(sheet).and_then(|ws| {
        ws.raw_after()
            .iter()
            .filter_map(open365_xlsx::RawXml::as_element)
            .find(|e| e.head.is_main("headerFooter"))
    }) else {
        return h;
    };
    let on = |name: &str, default: bool| match el.head.attr(name).map(str::trim) {
        Some("1" | "true") => true,
        Some("0" | "false") => false,
        _ => default,
    };
    h.different_odd_even = on("differentOddEven", false);
    h.different_first = on("differentFirst", false);
    h.scale_with_doc = on("scaleWithDoc", true);
    h.align_with_margins = on("alignWithMargins", true);
    let text = |name: &str| {
        el.child_main(name)
            .map(|c| c.own_text())
            .unwrap_or_default()
    };
    h.odd_header = text("oddHeader");
    h.odd_footer = text("oddFooter");
    h.even_header = text("evenHeader");
    h.even_footer = text("evenFooter");
    h.first_header = text("firstHeader");
    h.first_footer = text("firstFooter");
    h
}

/// Writes `h` as the `headerFooter` of `sheet`, replacing the one there.
pub(crate) fn set(
    x: &mut XlsxState,
    sheet: usize,
    h: &HeaderFooterRequest,
) -> Result<(), CommandError> {
    let part = xmlpatch::sheet_part_name(&x.workbook, sheet)?;
    let wb = xmlpatch::rewrite(&x.workbook, |pkg| {
        let mut xml = xmlpatch::part_text(pkg, &part)?;
        xmlpatch::remove_element(&mut xml, "headerFooter");
        if let Some(el) = element(h) {
            xmlpatch::insert_before_any(&mut xml, &el, AFTER_HEADER_FOOTER);
        }
        xmlpatch::set_part_text(pkg, &part, xml)
    })?;
    x.restore_workbook(wb);
    Ok(())
}
