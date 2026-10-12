//! Text-level edits of worksheet and workbook XML, for features the workbook model keeps as raw
//! elements (sheet protection, outline levels, notes). The workbook is saved, its package patched
//! and the result opened again; the caller keeps a copy for undo.

use open365_opc::{Limits, Package, PartName};
use open365_xlsx::Workbook;

use crate::error::CommandError;

fn opc(e: impl std::fmt::Display) -> CommandError {
    CommandError::Format(e.to_string())
}

/// Saves `wb`, lets `edit` change the package, and opens the result.
pub(crate) fn rewrite(
    wb: &Workbook,
    edit: impl FnOnce(&mut Package) -> Result<(), CommandError>,
) -> Result<Workbook, CommandError> {
    let mut copy = wb.clone();
    let bytes = copy.save()?;
    let limits = Limits::default();
    let mut pkg = Package::read(&bytes, &limits).map_err(opc)?;
    edit(&mut pkg)?;
    let out = pkg.write().map_err(opc)?;
    Ok(Workbook::open(&out, &limits)?)
}

pub(crate) fn part_text(pkg: &Package, name: &str) -> Result<String, CommandError> {
    let part = pkg
        .part(name)
        .ok_or_else(|| CommandError::Format(format!("missing part {name}")))?;
    Ok(String::from_utf8_lossy(part.data()).into_owned())
}

pub(crate) fn set_part_text(
    pkg: &mut Package,
    name: &str,
    text: String,
) -> Result<(), CommandError> {
    pkg.set_part(PartName::new(name).map_err(opc)?, text.into_bytes());
    Ok(())
}

/// The package name of a worksheet.
pub(crate) fn sheet_part_name(wb: &Workbook, sheet: usize) -> Result<String, CommandError> {
    wb.sheet_part(sheet)
        .map(|p| p.as_str().to_owned())
        .ok_or_else(|| CommandError::Format("the sheet has no part".to_owned()))
}

/// The value of attribute `name` in the start tag at the front of `tag`.
pub(crate) fn attr(tag: &str, name: &str) -> Option<String> {
    let head = &tag[..tag.find('>').map_or(tag.len(), |i| i + 1)];
    let key = format!(" {name}=\"");
    let at = head.find(&key)? + key.len();
    let end = head[at..].find('"')?;
    Some(head[at..at + end].to_owned())
}

/// Sets (or, with `None`, removes) attribute `name` in the start tag at the front of `tag`.
pub(crate) fn set_attr(tag: &mut String, name: &str, value: Option<&str>) {
    let head_end = tag.find('>').map_or(tag.len(), |i| i + 1);
    let key = format!(" {name}=\"");
    if let Some(start) = tag[..head_end].find(&key) {
        let vstart = start + key.len();
        if let Some(len) = tag[vstart..head_end].find('"') {
            match value {
                Some(v) => tag.replace_range(vstart..vstart + len, v),
                None => tag.replace_range(start..vstart + len + 1, ""),
            }
        }
    } else if let Some(v) = value {
        let close = if tag[..head_end].ends_with("/>") {
            head_end - 2
        } else {
            head_end - 1
        };
        tag.insert_str(close, &format!(" {name}=\"{v}\""));
    }
}

pub(crate) fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// A worksheet cut at its row elements.
pub(crate) struct Rows {
    pub head: String,
    /// `(row number, element text)` in order.
    pub rows: Vec<(u32, String)>,
    pub tail: String,
}

impl Rows {
    pub fn parse(xml: &str) -> Option<Self> {
        let sd = xml.find("<sheetData")?;
        let open_end = sd + xml[sd..].find('>')? + 1;
        if xml[sd..open_end].ends_with("/>") {
            return Some(Self {
                head: format!("{}<sheetData>", &xml[..sd]),
                rows: Vec::new(),
                tail: format!("</sheetData>{}", &xml[open_end..]),
            });
        }
        let close = xml.find("</sheetData>")?;
        let mut rows = Vec::new();
        let mut at = open_end;
        while let Some(rel) = xml[at..close].find("<row") {
            let start = at + rel;
            let tag_end = start + xml[start..].find('>')? + 1;
            let end = if xml[start..tag_end].ends_with("/>") {
                tag_end
            } else {
                tag_end + xml[tag_end..].find("</row>")? + "</row>".len()
            };
            let text = xml[start..end].to_owned();
            let r = attr(&text, "r")?.parse().ok()?;
            rows.push((r, text));
            at = end;
        }
        Some(Self {
            head: xml[..open_end].to_owned(),
            rows,
            tail: xml[close..].to_owned(),
        })
    }

    pub fn render(&self) -> String {
        let mut out = self.head.clone();
        for (_, t) in &self.rows {
            out.push_str(t);
        }
        out.push_str(&self.tail);
        out
    }

    /// The element of row `r`, created empty (in order) when missing.
    pub fn row_mut(&mut self, r: u32) -> &mut String {
        let pos = match self.rows.binary_search_by_key(&r, |(n, _)| *n) {
            Ok(i) => i,
            Err(i) => {
                self.rows.insert(i, (r, format!("<row r=\"{r}\"/>")));
                i
            }
        };
        &mut self.rows[pos].1
    }

    pub fn level(&self, r: u32) -> u32 {
        self.rows
            .binary_search_by_key(&r, |(n, _)| *n)
            .ok()
            .and_then(|i| attr(&self.rows[i].1, "outlineLevel"))
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    }

    pub fn max_level(&self) -> u32 {
        self.rows
            .iter()
            .filter_map(|(_, t)| attr(t, "outlineLevel")?.parse().ok())
            .max()
            .unwrap_or(0)
    }
}

/// Writes `outlineLevelRow` on `sheetFormatPr` (adding the element when needed).
pub(crate) fn set_outline_level_row(xml: &mut String, level: u32) {
    set_outline_level(xml, "outlineLevelRow", level);
}

/// Writes `outlineLevelRow` or `outlineLevelCol` on `sheetFormatPr`.
pub(crate) fn set_outline_level(xml: &mut String, name: &str, level: u32) {
    let value = (level > 0).then(|| level.to_string());
    if let Some(at) = xml.find("<sheetFormatPr") {
        let end = at + xml[at..].find('>').map_or(0, |i| i + 1);
        let mut tag = xml[at..end].to_owned();
        set_attr(&mut tag, name, value.as_deref());
        xml.replace_range(at..end, &tag);
    } else if let Some(v) = value {
        let element = format!("<sheetFormatPr defaultRowHeight=\"15\" {name}=\"{v}\"/>");
        let at = ["<cols", "<sheetData"]
            .iter()
            .filter_map(|k| xml.find(k))
            .min();
        if let Some(at) = at {
            xml.insert_str(at, &element);
        }
    }
}

/// Start of element `<name` followed by a space, `>` or `/`.
pub(crate) fn find_open(xml: &str, name: &str) -> Option<usize> {
    let pat = format!("<{name}");
    let mut from = 0;
    while let Some(rel) = xml[from..].find(&pat) {
        let i = from + rel;
        match xml[i + pat.len()..].chars().next() {
            Some(' ' | '>' | '/') => return Some(i),
            _ => from = i + pat.len(),
        }
    }
    None
}

/// Inserts `element` before the first present element of `later` that follows `sheetData`,
/// else before the closing worksheet tag.
pub(crate) fn insert_before_any(xml: &mut String, element: &str, later: &[&str]) {
    let sheet_data_end = xml.find("</sheetData>").unwrap_or(0);
    let at = later
        .iter()
        .filter_map(|n| find_open(xml, n))
        .filter(|&i| i > sheet_data_end)
        .min()
        .or_else(|| xml.rfind("</worksheet>"));
    if let Some(at) = at {
        xml.insert_str(at, element);
    }
}

/// Removes element `name` (self-closing or with content) when present; true when it was.
pub(crate) fn remove_element(xml: &mut String, name: &str) -> bool {
    let Some(i) = find_open(xml, name) else {
        return false;
    };
    let Some(open_end) = xml[i..].find('>').map(|n| i + n + 1) else {
        return false;
    };
    let end = if xml[i..open_end].ends_with("/>") {
        open_end
    } else {
        let close = format!("</{name}>");
        match xml[open_end..].find(&close) {
            Some(n) => open_end + n + close.len(),
            None => return false,
        }
    };
    xml.replace_range(i..end, "");
    true
}

/// The text of element `name` from its start tag to its end, when present.
pub(crate) fn element_text(xml: &str, name: &str) -> Option<String> {
    let i = find_open(xml, name)?;
    let open_end = i + xml[i..].find('>')? + 1;
    if xml[i..open_end].ends_with("/>") {
        return Some(xml[i..open_end].to_owned());
    }
    let close = format!("</{name}>");
    let n = xml[open_end..].find(&close)?;
    Some(xml[i..open_end + n + close.len()].to_owned())
}

/// Elements that follow `sheetProtection` in the worksheet schema.
pub(crate) const AFTER_PROTECTION: &[&str] = &[
    "protectedRanges",
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

/// One `col` element.
pub(crate) struct ColEntry {
    pub min: u32,
    pub max: u32,
    pub tag: String,
}

/// The `cols` element of a worksheet: its byte range and entries.
pub(crate) fn parse_cols(xml: &str) -> (Option<(usize, usize)>, Vec<ColEntry>) {
    let Some(start) = find_open(xml, "cols") else {
        return (None, Vec::new());
    };
    let Some(open_end) = xml[start..].find('>').map(|n| start + n + 1) else {
        return (None, Vec::new());
    };
    if xml[start..open_end].ends_with("/>") {
        return (Some((start, open_end)), Vec::new());
    }
    let Some(close) = xml[open_end..].find("</cols>").map(|n| open_end + n) else {
        return (None, Vec::new());
    };
    let mut entries = Vec::new();
    let mut at = open_end;
    while let Some(rel) = xml[at..close].find("<col") {
        let s0 = at + rel;
        let Some(e0) = xml[s0..].find('>').map(|n| s0 + n + 1) else {
            break;
        };
        let tag = xml[s0..e0].to_owned();
        if let (Some(min), Some(max)) = (
            attr(&tag, "min").and_then(|v| v.parse().ok()),
            attr(&tag, "max").and_then(|v| v.parse().ok()),
        ) {
            entries.push(ColEntry { min, max, tag });
        }
        at = e0;
    }
    (Some((start, close + "</cols>".len())), entries)
}

/// Writes the entries back as the `cols` element (before `sheetData` when it was absent).
pub(crate) fn write_cols(xml: &mut String, range: Option<(usize, usize)>, entries: &[ColEntry]) {
    let mut text = String::from("<cols>");
    for e in entries {
        text.push_str(&e.tag);
    }
    text.push_str("</cols>");
    match range {
        Some((a, b)) => xml.replace_range(a..b, &text),
        None => {
            if let Some(at) = xml.find("<sheetData") {
                xml.insert_str(at, &text);
            }
        }
    }
}

/// Applies `f` to the start tags of the columns `first..=last`, splitting entries at the edges
/// and creating default-width entries for columns that had none.
pub(crate) fn adjust_cols(
    entries: &mut Vec<ColEntry>,
    first: u32,
    last: u32,
    f: &dyn Fn(&mut String),
) {
    let mut out: Vec<ColEntry> = Vec::new();
    let mut covered: Vec<(u32, u32)> = Vec::new();
    for e in entries.drain(..) {
        if e.max < first || e.min > last {
            out.push(e);
            continue;
        }
        let (lo, hi) = (e.min.max(first), e.max.min(last));
        let piece = |min: u32, max: u32, apply: bool| {
            let mut tag = e.tag.clone();
            set_attr(&mut tag, "min", Some(&min.to_string()));
            set_attr(&mut tag, "max", Some(&max.to_string()));
            if apply {
                f(&mut tag);
            }
            ColEntry { min, max, tag }
        };
        if e.min < lo {
            out.push(piece(e.min, lo - 1, false));
        }
        out.push(piece(lo, hi, true));
        if hi < e.max {
            out.push(piece(hi + 1, e.max, false));
        }
        covered.push((lo, hi));
    }
    covered.sort_unstable();
    let mut next = first;
    for (lo, hi) in covered {
        if lo > next {
            out.push(default_col(next, lo - 1, f));
        }
        next = next.max(hi + 1);
    }
    if next <= last {
        out.push(default_col(next, last, f));
    }
    out.sort_by_key(|e| e.min);
    *entries = out;
}

fn default_col(min: u32, max: u32, f: &dyn Fn(&mut String)) -> ColEntry {
    let mut tag = format!("<col min=\"{min}\" max=\"{max}\" width=\"9.140625\"/>");
    f(&mut tag);
    ColEntry { min, max, tag }
}

/// The highest `outlineLevel` of the entries.
pub(crate) fn max_col_level(entries: &[ColEntry]) -> u32 {
    entries
        .iter()
        .filter_map(|e| attr(&e.tag, "outlineLevel")?.parse().ok())
        .max()
        .unwrap_or(0)
}
