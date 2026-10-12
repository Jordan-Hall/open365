//! The Review tab's commands for Sheets: Protect Sheet, Protect Workbook and cell notes.
//! Protection and notes live in worksheet and workbook elements the model keeps raw, so the
//! commands patch the package text (see [`super::xmlpatch`]).

use open365_opc::{Relationships, TargetMode, rels_part_name, resolve_target};
use open365_xlsx::Workbook;
use serde::{Deserialize, Serialize};

use super::xmlpatch::{self, AFTER_PROTECTION};
use crate::command::CellRef;
use crate::error::CommandError;
use crate::xlsx_state::XlsxState;

fn bad(msg: impl Into<String>) -> CommandError {
    CommandError::Format(msg.into())
}

/// The Protect Sheet dialog's "Allow all users of this worksheet to" list. `true` allows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SheetProtection {
    pub select_locked_cells: bool,
    pub select_unlocked_cells: bool,
    pub format_cells: bool,
    pub format_columns: bool,
    pub format_rows: bool,
    pub insert_columns: bool,
    pub insert_rows: bool,
    pub insert_hyperlinks: bool,
    pub delete_columns: bool,
    pub delete_rows: bool,
    pub sort: bool,
    pub use_auto_filter: bool,
    pub use_pivot_tables: bool,
    pub edit_objects: bool,
    pub edit_scenarios: bool,
}

impl Default for SheetProtection {
    /// The dialog as Excel opens it: only the two "select" boxes are ticked.
    fn default() -> Self {
        Self {
            select_locked_cells: true,
            select_unlocked_cells: true,
            format_cells: false,
            format_columns: false,
            format_rows: false,
            insert_columns: false,
            insert_rows: false,
            insert_hyperlinks: false,
            delete_columns: false,
            delete_rows: false,
            sort: false,
            use_auto_filter: false,
            use_pivot_tables: false,
            edit_objects: false,
            edit_scenarios: false,
        }
    }
}

/// A cell note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    pub row: u32,
    pub col: u32,
    pub author: String,
    pub text: String,
}

/// A sheet or workbook password. Its `Debug` output is redacted, so a logged or audited command
/// never carries it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Password(String);

impl Password {
    #[must_use]
    pub fn new(text: &str) -> Self {
        Self(text.to_owned())
    }

    /// The text; keep it out of logs and messages.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Password {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Password(<redacted>)")
    }
}

/// Every Review tab edit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ReviewOp {
    ProtectSheet {
        options: SheetProtection,
        #[serde(default)]
        password: Option<Password>,
    },
    UnprotectSheet {
        #[serde(default)]
        password: Option<Password>,
    },
    /// Protect Workbook: Structure (sheets cannot be added, moved, renamed or deleted).
    ProtectWorkbook {
        #[serde(default)]
        password: Option<Password>,
    },
    UnprotectWorkbook {
        #[serde(default)]
        password: Option<Password>,
    },
    /// New Note / Edit Note: sets the note of a cell.
    SetNote(Note),
    DeleteNote {
        row: u32,
        col: u32,
    },
    /// New Comment: a new thread on the cell, or a reply when it has one.
    AddComment {
        row: u32,
        col: u32,
        author: String,
        text: String,
    },
    /// Delete (Comments group): the thread of the cell.
    DeleteComment {
        row: u32,
        col: u32,
    },
}

/// The 16-bit hash Excel stores in `password` / `workbookPassword` (upper-case hex).
#[must_use]
pub fn legacy_password_hash(password: &str) -> String {
    let chars: Vec<u16> = password.encode_utf16().collect();
    let mut hash: u16 = 0;
    for &c in chars.iter().rev() {
        hash = ((hash >> 14) & 1) | ((hash << 1) & 0x7fff);
        hash ^= c;
    }
    hash = ((hash >> 14) & 1) | ((hash << 1) & 0x7fff);
    hash ^= chars.len() as u16;
    hash ^= 0xCE4B;
    format!("{hash:04X}")
}

// ---- reading ----------------------------------------------------------------------------

/// The protection of a sheet, `None` when it is not protected.
#[must_use]
pub fn sheet_protection(wb: &Workbook, sheet: usize) -> Option<SheetProtection> {
    let ws = wb.sheet(sheet)?;
    let el = ws
        .raw_after()
        .iter()
        .filter_map(open365_xlsx::RawXml::as_element)
        .find(|e| e.head.is_main("sheetProtection"))?;
    let on = |name: &str, default: bool| match el.head.attr(name).map(str::trim) {
        Some("1" | "true") => true,
        Some("0" | "false") => false,
        _ => default,
    };
    if !on("sheet", false) {
        return None;
    }
    // In the file `true` means "locked" (not allowed) except for the select boxes' inverse names.
    Some(SheetProtection {
        select_locked_cells: !on("selectLockedCells", false),
        select_unlocked_cells: !on("selectUnlockedCells", false),
        format_cells: !on("formatCells", true),
        format_columns: !on("formatColumns", true),
        format_rows: !on("formatRows", true),
        insert_columns: !on("insertColumns", true),
        insert_rows: !on("insertRows", true),
        insert_hyperlinks: !on("insertHyperlinks", true),
        delete_columns: !on("deleteColumns", true),
        delete_rows: !on("deleteRows", true),
        sort: !on("sort", true),
        use_auto_filter: !on("autoFilter", true),
        use_pivot_tables: !on("pivotTables", true),
        edit_objects: !on("objects", false),
        edit_scenarios: !on("scenarios", false),
    })
}

fn stored_sheet_hash(wb: &Workbook, sheet: usize) -> Option<String> {
    let ws = wb.sheet(sheet)?;
    let el = ws
        .raw_after()
        .iter()
        .filter_map(open365_xlsx::RawXml::as_element)
        .find(|e| e.head.is_main("sheetProtection"))?;
    el.head
        .attr("password")
        .map(str::to_owned)
        .or_else(|| el.head.attr("hashValue").map(|_| "sha".to_owned()))
}

/// Whether Protect Workbook (structure) is on.
#[must_use]
pub fn workbook_structure_locked(wb: &Workbook) -> bool {
    wb.package()
        .part(wb.workbook_part().as_str())
        .is_some_and(|p| {
            let text = String::from_utf8_lossy(p.data());
            xmlpatch::element_text(&text, "workbookProtection").is_some_and(|e| {
                matches!(
                    xmlpatch::attr(&e, "lockStructure").as_deref(),
                    Some("1" | "true")
                )
            })
        })
}

/// Whether the cell is locked (the default) in its style.
#[must_use]
pub fn cell_locked(wb: &Workbook, sheet: usize, row: u32, col: u32) -> bool {
    let style = wb.cell(sheet, row, col).and_then(|c| c.style).unwrap_or(0) as usize;
    if style == 0 {
        return true;
    }
    let Some(bytes) = wb.serialize_styles() else {
        return true;
    };
    let text = String::from_utf8_lossy(&bytes);
    let Some(at) = xmlpatch::find_open(&text, "cellXfs") else {
        return true;
    };
    let body = &text[at..];
    let end = body.find("</cellXfs>").unwrap_or(body.len());
    let body = &body[..end];
    let mut from = 0;
    let mut index = 0;
    while let Some(rel) = body[from..].find("<xf") {
        let start = from + rel;
        let after = body[start + 3..].chars().next();
        if !matches!(after, Some(' ' | '>' | '/')) {
            from = start + 3;
            continue;
        }
        let tag_end = start + body[start..].find('>').map_or(0, |i| i + 1);
        let stop = if body[start..tag_end].ends_with("/>") {
            tag_end
        } else {
            tag_end + body[tag_end..].find("</xf>").map_or(0, |i| i + 5)
        };
        if index == style {
            let xf = &body[start..stop];
            return !xmlpatch::find_open(xf, "protection").is_some_and(|p| {
                matches!(
                    xmlpatch::attr(&xf[p..], "locked").as_deref(),
                    Some("0" | "false")
                )
            });
        }
        index += 1;
        from = stop.max(start + 3);
    }
    true
}

// ---- notes and threaded comments -------------------------------------------------------

/// One entry of a comment thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadItem {
    pub id: String,
    pub author: String,
    pub text: String,
    /// `dT`, e.g. `2024-05-01T12:00:00.00` (UTC).
    pub time: String,
}

/// A threaded comment: the first comment and its replies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thread {
    pub row: u32,
    pub col: u32,
    pub id: String,
    pub items: Vec<ThreadItem>,
}

/// Everything the Review tab keeps in a sheet's comment parts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Annotations {
    pub notes: Vec<Note>,
    pub threads: Vec<Thread>,
}

const THREADED_PLACEHOLDER: &str = "[Threaded comment]\n\nYour version of Excel allows you to read this threaded comment; however, any edits to it will get removed if the file is opened in a newer version of Excel. Learn more: https://go.microsoft.com/fwlink/?linkid=870924\n\nComment:\n    ";

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn parse_ref(a1: &str) -> Option<(u32, u32)> {
    open365_xlsx::parse_cell_ref(a1)
}

/// The part a part relates to with a relationship type ending in `suffix`.
fn related_part(wb: &Workbook, source: &open365_opc::PartName, suffix: &str) -> Option<String> {
    let rels = wb.package().part(rels_part_name(Some(source)).as_str())?;
    let rels = Relationships::parse(rels.data()).ok()?;
    rels.iter()
        .find(|r| r.target_mode == TargetMode::Internal && r.rel_type.ends_with(suffix))
        .and_then(|r| resolve_target(Some(source), &r.target).ok())
        .map(|n| n.as_str().to_owned())
}

fn comments_part(wb: &Workbook, sheet: usize) -> Option<String> {
    related_part(wb, wb.sheet_part(sheet)?, "/comments")
}

fn threaded_part(wb: &Workbook, sheet: usize) -> Option<String> {
    related_part(wb, wb.sheet_part(sheet)?, "/threadedComment")
}

fn persons_part(wb: &Workbook) -> Option<String> {
    related_part(wb, wb.workbook_part(), "/person")
}

fn part_string(wb: &Workbook, name: &str) -> Option<String> {
    wb.package()
        .part(name)
        .map(|p| String::from_utf8_lossy(p.data()).into_owned())
}

/// The elements `<name ...>...</name>` (or self-closing) of `xml`, as text.
fn elements<'a>(xml: &'a str, name: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(i) = xmlpatch::find_open(&xml[at..], name) {
        let start = at + i;
        let Some(open_end) = xml[start..].find('>').map(|n| start + n + 1) else {
            break;
        };
        let end = if xml[start..open_end].ends_with("/>") {
            open_end
        } else {
            match xml[open_end..].find(&format!("</{name}>")) {
                Some(n) => open_end + n + name.len() + 3,
                None => break,
            }
        };
        out.push(&xml[start..end]);
        at = end;
    }
    out
}

fn inner(element: &str, name: &str) -> String {
    if element.ends_with("/>") {
        return String::new();
    }
    let open = element.find('>').map_or(0, |i| i + 1);
    let close = element.len().saturating_sub(name.len() + 3);
    unescape(element.get(open..close).unwrap_or_default())
}

fn parse_persons(xml: &str) -> Vec<(String, String)> {
    elements(xml, "person")
        .into_iter()
        .filter_map(|p| {
            Some((
                xmlpatch::attr(p, "id")?,
                unescape(&xmlpatch::attr(p, "displayName").unwrap_or_default()),
            ))
        })
        .collect()
}

fn parse_threads(xml: &str, persons: &[(String, String)]) -> Vec<Thread> {
    let mut threads: Vec<Thread> = Vec::new();
    for el in elements(xml, "threadedComment") {
        let Some((row, col)) = xmlpatch::attr(el, "ref").and_then(|r| parse_ref(&r)) else {
            continue;
        };
        let id = xmlpatch::attr(el, "id").unwrap_or_default();
        let author = xmlpatch::attr(el, "personId")
            .and_then(|p| {
                persons
                    .iter()
                    .find(|(i, _)| *i == p)
                    .map(|(_, n)| n.clone())
            })
            .unwrap_or_default();
        let text = elements(el, "text")
            .first()
            .map(|t| inner(t, "text"))
            .unwrap_or_default();
        let item = ThreadItem {
            id: id.clone(),
            author,
            text,
            time: xmlpatch::attr(el, "dT").unwrap_or_default(),
        };
        match xmlpatch::attr(el, "parentId") {
            Some(parent) => {
                if let Some(t) = threads.iter_mut().find(|t| t.id == parent) {
                    t.items.push(item);
                }
            }
            None => threads.push(Thread {
                row,
                col,
                id,
                items: vec![item],
            }),
        }
    }
    threads
}

fn parse_comments(xml: &str) -> Vec<Note> {
    let authors: Vec<String> = xmlpatch::element_text(xml, "authors")
        .map(|list| {
            elements(&list, "author")
                .into_iter()
                .map(|a| inner(a, "author"))
                .collect()
        })
        .unwrap_or_default();
    let mut out = Vec::new();
    for el in elements(xml, "comment") {
        let Some((row, col)) = xmlpatch::attr(el, "ref").and_then(|r| parse_ref(&r)) else {
            continue;
        };
        let author = xmlpatch::attr(el, "authorId")
            .and_then(|a| a.parse::<usize>().ok())
            .and_then(|a| authors.get(a).cloned())
            .unwrap_or_default();
        // The text: every <t> outside the run properties.
        let mut text: String = elements(el, "t")
            .into_iter()
            .map(|t| inner(t, "t"))
            .collect();
        let prefix = format!("{author}:\n");
        if let Some(rest) = text.strip_prefix(&prefix) {
            text = rest.to_owned();
        }
        out.push(Note {
            row,
            col,
            author,
            text,
        });
    }
    out
}

/// The notes and threaded comments of a sheet.
#[must_use]
pub fn annotations(wb: &Workbook, sheet: usize) -> Annotations {
    let mut all = Annotations::default();
    if let Some(xml) = comments_part(wb, sheet).and_then(|n| part_string(wb, &n)) {
        all.notes = parse_comments(&xml)
            .into_iter()
            .filter(|n| !n.author.starts_with("tc="))
            .collect();
    }
    if let Some(xml) = threaded_part(wb, sheet).and_then(|n| part_string(wb, &n)) {
        let persons = persons_part(wb)
            .and_then(|n| part_string(wb, &n))
            .map(|x| parse_persons(&x))
            .unwrap_or_default();
        all.threads = parse_threads(&xml, &persons);
    }
    all
}

/// The notes of a sheet.
#[must_use]
pub fn notes(wb: &Workbook, sheet: usize) -> Vec<Note> {
    annotations(wb, sheet).notes
}

/// The threaded comments of a sheet.
#[must_use]
pub fn threads(wb: &Workbook, sheet: usize) -> Vec<Thread> {
    annotations(wb, sheet).threads
}

fn comments_xml(notes: &[Note]) -> String {
    let mut authors: Vec<&str> = Vec::new();
    for n in notes {
        if !authors.contains(&n.author.as_str()) {
            authors.push(&n.author);
        }
    }
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<comments xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><authors>",
    );
    for a in &authors {
        out.push_str(&format!("<author>{}</author>", xmlpatch::xml_escape(a)));
    }
    out.push_str("</authors><commentList>");
    let mut sorted: Vec<&Note> = notes.iter().collect();
    sorted.sort_by_key(|n| (n.row, n.col));
    for n in sorted {
        let id = authors.iter().position(|a| *a == n.author).unwrap_or(0);
        let r = open365_xlsx::cell_ref_string(n.row, n.col);
        if n.author.starts_with("tc=") {
            out.push_str(&format!(
                "<comment ref=\"{r}\" authorId=\"{id}\" shapeId=\"0\"><text><t xml:space=\"preserve\">{}</t></text></comment>",
                xmlpatch::xml_escape(&n.text),
            ));
            continue;
        }
        out.push_str(&format!(
            "<comment ref=\"{r}\" authorId=\"{id}\" shapeId=\"0\"><text><r><rPr><b/><sz val=\"9\"/><color indexed=\"81\"/><rFont val=\"Tahoma\"/><family val=\"2\"/></rPr><t xml:space=\"preserve\">{}:\n</t></r><r><rPr><sz val=\"9\"/><color indexed=\"81\"/><rFont val=\"Tahoma\"/><family val=\"2\"/></rPr><t xml:space=\"preserve\">{}</t></r></text></comment>",
            xmlpatch::xml_escape(&n.author),
            xmlpatch::xml_escape(&n.text),
        ));
    }
    out.push_str("</commentList></comments>");
    out
}

fn vml_xml(notes: &[Note], shape_base: u32) -> String {
    let mut out = String::from(
        "<xml xmlns:v=\"urn:schemas-microsoft-com:vml\" xmlns:o=\"urn:schemas-microsoft-com:office:office\" xmlns:x=\"urn:schemas-microsoft-com:office:excel\">\n <o:shapelayout v:ext=\"edit\"><o:idmap v:ext=\"edit\" data=\"1\"/></o:shapelayout>\n <v:shapetype id=\"_x0000_t202\" coordsize=\"21600,21600\" o:spt=\"202\" path=\"m,l,21600r21600,l21600,xe\"><v:stroke joinstyle=\"miter\"/><v:path gradientshapeok=\"t\" o:connecttype=\"rect\"/></v:shapetype>\n",
    );
    for (i, n) in notes.iter().enumerate() {
        let (r0, c0) = (n.row - 1, n.col - 1);
        out.push_str(&format!(
            " <v:shape id=\"_x0000_s{}\" type=\"#_x0000_t202\" style=\"position:absolute;margin-left:80pt;margin-top:2pt;width:108pt;height:60pt;z-index:{};visibility:hidden\" fillcolor=\"#ffffe1\" o:insetmode=\"auto\"><v:fill color2=\"#ffffe1\"/><v:shadow on=\"t\" color=\"black\" obscured=\"t\"/><v:path o:connecttype=\"none\"/><v:textbox style=\"mso-direction-alt:auto\"><div style=\"text-align:left\"></div></v:textbox><x:ClientData ObjectType=\"Note\"><x:MoveWithCells/><x:SizeWithCells/><x:Anchor>{}, 15, {}, 2, {}, 15, {}, 16</x:Anchor><x:AutoFill>False</x:AutoFill><x:Row>{r0}</x:Row><x:Column>{c0}</x:Column></x:ClientData></v:shape>\n",
            shape_base + i as u32,
            i + 1,
            c0 + 1,
            r0.saturating_sub(1),
            c0 + 3,
            r0 + 3,
        ));
    }
    out.push_str("</xml>");
    out
}

fn add_override(ct: &mut String, part: &str, kind: &str) {
    let entry = format!("<Override PartName=\"/{part}\" ContentType=\"{kind}\"/>");
    if !ct.contains(&format!("PartName=\"/{part}\""))
        && let Some(at) = ct.rfind("</Types>")
    {
        ct.insert_str(at, &entry);
    }
}

fn remove_override(ct: &mut String, part: &str) {
    let key = format!("PartName=\"/{part}\"");
    if let Some(k) = ct.find(&key)
        && let Some(start) = ct[..k].rfind("<Override")
        && let Some(end) = ct[k..].find("/>").map(|n| k + n + 2)
    {
        ct.replace_range(start..end, "");
    }
}

fn add_default_vml(ct: &mut String) {
    if !ct.contains("Extension=\"vml\"") {
        let default = "<Default Extension=\"vml\" ContentType=\"application/vnd.openxmlformats-officedocument.vmlDrawing\"/>";
        if let Some(at) = ct.find("<Override").or_else(|| ct.rfind("</Types>")) {
            ct.insert_str(at, default);
        }
    }
}

const REL_COMMENTS: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments";
const REL_VML: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/vmlDrawing";
const REL_THREADED: &str =
    "http://schemas.microsoft.com/office/2017/10/relationships/threadedComment";
const REL_PERSON: &str = "http://schemas.microsoft.com/office/2017/10/relationships/person";
const CT_COMMENTS: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml";
const CT_THREADED: &str = "application/vnd.ms-excel.threadedcomments+xml";
const CT_PERSON: &str = "application/vnd.ms-excel.person+xml";
const NS_THREADED: &str = "http://schemas.microsoft.com/office/spreadsheetml/2018/threadedcomments";

static GUID_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// A fresh identifier in Excel's `{XXXXXXXX-XXXX-4XXX-8XXX-XXXXXXXXXXXX}` form.
fn new_guid() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    let mut z =
        nanos ^ GUID_COUNTER.fetch_add(0x9E37_79B9_7F4A_7C15, std::sync::atomic::Ordering::Relaxed);
    let mut next = || {
        z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut x = z;
        x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        x ^ (x >> 31)
    };
    let (a, b) = (next(), next());
    format!(
        "{{{:08X}-{:04X}-4{:03X}-8{:03X}-{:012X}}}",
        a >> 32,
        (a >> 16) & 0xFFFF,
        a & 0xFFF,
        (b >> 52) & 0xFFF,
        b & 0xFFFF_FFFF_FFFF
    )
}

/// The current UTC time as a `dT` value.
fn now_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // Civil date from days since 1970-01-01.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.00",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

fn thread_placeholder(t: &Thread) -> String {
    let mut s = String::from(THREADED_PLACEHOLDER);
    for (i, item) in t.items.iter().enumerate() {
        if i > 0 {
            s.push_str("\nReply:\n    ");
        }
        s.push_str(&item.text);
    }
    s
}

fn threaded_xml(threads: &[Thread], person_of: &dyn Fn(&str) -> String) -> String {
    let mut out = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<ThreadedComments xmlns=\"{NS_THREADED}\" xmlns:x=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">"
    );
    for t in threads {
        let r = open365_xlsx::cell_ref_string(t.row, t.col);
        for (i, item) in t.items.iter().enumerate() {
            let parent = if i == 0 {
                String::new()
            } else {
                format!(" parentId=\"{}\"", t.id)
            };
            out.push_str(&format!(
                "<threadedComment ref=\"{r}\" dT=\"{}\" personId=\"{}\" id=\"{}\"{parent}><text>{}</text></threadedComment>",
                item.time,
                person_of(&item.author),
                item.id,
                xmlpatch::xml_escape(&item.text)
            ));
        }
    }
    out.push_str("</ThreadedComments>");
    out
}

fn ensure_r_namespace(xml: &mut String) {
    if let Some(at) = xml.find("<worksheet") {
        let end = at + xml[at..].find('>').map_or(0, |i| i);
        if !xml[at..end].contains("xmlns:r=") {
            xml.insert_str(
                at + "<worksheet".len(),
                " xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"",
            );
        }
    }
}

fn next_rid(rels: &str) -> String {
    let n = (1..)
        .find(|n| !rels.contains(&format!("Id=\"rId{n}\"")))
        .unwrap_or(1);
    format!("rId{n}")
}

fn insert_relationship(rels: &mut String, rel: &str) {
    if let Some(at) = rels.rfind("</Relationships>") {
        rels.insert_str(at, rel);
    }
}

fn strip_relationships(rels: &mut String, types: &[&str]) {
    let mut from = 0;
    while let Some(i) = rels[from..].find("<Relationship ") {
        let start = from + i;
        let Some(end) = rels[start..].find("/>").map(|n| start + n + 2) else {
            break;
        };
        if types
            .iter()
            .any(|t| rels[start..end].contains(&format!("Type=\"{t}\"")))
        {
            rels.replace_range(start..end, "");
            from = start;
        } else {
            from = end;
        }
    }
}

const EMPTY_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"></Relationships>";

/// The package name a relationship of type `ty` in `rels` points at, resolved against the
/// directory of the source part `source`.
fn rel_target(rels: &str, ty: &str, source: &str) -> Option<String> {
    let at = rels.find(&format!("Type=\"{ty}\""))?;
    let start = rels[..at].rfind("<Relationship ")?;
    let end = at + rels[at..].find("/>")?;
    let target = xmlpatch::attr(&rels[start..end], "Target")?;
    if let Some(t) = target.strip_prefix('/') {
        return Some(t.to_owned());
    }
    let mut parts: Vec<&str> = source
        .rsplit_once('/')
        .map_or("", |x| x.0)
        .split('/')
        .collect();
    for seg in target.split('/') {
        match seg {
            ".." => {
                parts.pop();
            }
            "." | "" => {}
            s => parts.push(s),
        }
    }
    Some(parts.join("/"))
}

fn book_rels_name(book_part: &str) -> String {
    let (dir, file) = book_part.rsplit_once('/').unwrap_or(("", book_part));
    if dir.is_empty() {
        format!("_rels/{file}.rels")
    } else {
        format!("{dir}/_rels/{file}.rels")
    }
}

/// The persons part: its name, and whether the workbook still lacks the relationship to it.
fn persons_part_name(pkg: &open365_opc::Package, book_part: &str) -> (String, bool) {
    let rels = pkg
        .part(&book_rels_name(book_part))
        .map(|p| String::from_utf8_lossy(p.data()).into_owned())
        .unwrap_or_default();
    match rel_target(&rels, REL_PERSON, book_part) {
        Some(n) => (n, false),
        None => ("xl/persons/person.xml".to_owned(), true),
    }
}

/// Rewrites a sheet's comment parts to hold exactly `all`.
fn write_annotations(
    wb: &Workbook,
    sheet: usize,
    all: &Annotations,
) -> Result<Workbook, CommandError> {
    let sheet_part = xmlpatch::sheet_part_name(wb, sheet)?;
    let book_part = wb.workbook_part().as_str().to_owned();
    let existing_persons = persons_part(wb)
        .and_then(|n| part_string(wb, &n))
        .map(|x| parse_persons(&x))
        .unwrap_or_default();
    xmlpatch::rewrite(wb, |pkg| {
        let file = sheet_part
            .rsplit('/')
            .next()
            .unwrap_or(&sheet_part)
            .to_owned();
        let rels_name = format!("xl/worksheets/_rels/{file}.rels");
        let mut rels =
            xmlpatch::part_text(pkg, &rels_name).unwrap_or_else(|_| EMPTY_RELS.to_owned());
        let mut sheet_xml = xmlpatch::part_text(pkg, &sheet_part)?;
        let mut ct = xmlpatch::part_text(pkg, "[Content_Types].xml")?;
        let comments_name = rel_target(&rels, REL_COMMENTS, &sheet_part);
        let vml_name = rel_target(&rels, REL_VML, &sheet_part);
        let threaded_name = rel_target(&rels, REL_THREADED, &sheet_part);

        // Threaded comments and the people they name.
        if all.threads.is_empty() {
            if let Some(t) = &threaded_name {
                pkg.remove_part(t);
                remove_override(&mut ct, t);
            }
            strip_relationships(&mut rels, &[REL_THREADED]);
        } else {
            let mut persons = existing_persons.clone();
            let mut ids: Vec<(String, String)> = Vec::new();
            for author in all
                .threads
                .iter()
                .flat_map(|t| t.items.iter().map(|i| &i.author))
            {
                let id = match persons.iter().find(|(_, n)| n == author) {
                    Some((id, _)) => id.clone(),
                    None => {
                        let id = new_guid();
                        persons.push((id.clone(), author.clone()));
                        id
                    }
                };
                ids.push((author.clone(), id));
            }
            let lookup = |name: &str| -> String {
                ids.iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, id)| id.clone())
                    .unwrap_or_default()
            };
            let name = threaded_name.clone().unwrap_or_else(|| {
                let n = (1..)
                    .find(|n| {
                        pkg.part(&format!("xl/threadedComments/threadedComment{n}.xml"))
                            .is_none()
                    })
                    .unwrap_or(1);
                format!("xl/threadedComments/threadedComment{n}.xml")
            });
            xmlpatch::set_part_text(pkg, &name, threaded_xml(&all.threads, &lookup))?;
            add_override(&mut ct, &name, CT_THREADED);
            if threaded_name.is_none() {
                let rid = next_rid(&rels);
                insert_relationship(
                    &mut rels,
                    &format!(
                        "<Relationship Id=\"{rid}\" Type=\"{REL_THREADED}\" Target=\"../{}\"/>",
                        name.trim_start_matches("xl/")
                    ),
                );
            }
            let (pname, needs_rel) = persons_part_name(pkg, &book_part);
            let mut px = format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<personList xmlns=\"{NS_THREADED}\" xmlns:x=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">"
            );
            for (id, display) in &persons {
                let d = xmlpatch::xml_escape(display);
                px.push_str(&format!(
                    "<person displayName=\"{d}\" id=\"{id}\" userId=\"{d}\" providerId=\"None\"/>"
                ));
            }
            px.push_str("</personList>");
            xmlpatch::set_part_text(pkg, &pname, px)?;
            add_override(&mut ct, &pname, CT_PERSON);
            if needs_rel {
                let brels_name = book_rels_name(&book_part);
                let mut brels =
                    xmlpatch::part_text(pkg, &brels_name).unwrap_or_else(|_| EMPTY_RELS.to_owned());
                let rid = next_rid(&brels);
                let dir = book_part.rsplit_once('/').map_or("", |x| x.0);
                insert_relationship(
                    &mut brels,
                    &format!(
                        "<Relationship Id=\"{rid}\" Type=\"{REL_PERSON}\" Target=\"{}\"/>",
                        pname.strip_prefix(&format!("{dir}/")).unwrap_or(&pname)
                    ),
                );
                xmlpatch::set_part_text(pkg, &brels_name, brels)?;
            }
        }

        // The legacy comments part and its drawing hold the notes and a placeholder per thread.
        let mut legacy: Vec<Note> = all.notes.clone();
        for t in &all.threads {
            legacy.push(Note {
                row: t.row,
                col: t.col,
                author: format!("tc={}", t.id),
                text: thread_placeholder(t),
            });
        }
        legacy.sort_by_key(|n| (n.row, n.col));
        if legacy.is_empty() {
            if let Some(c) = &comments_name {
                pkg.remove_part_and_override(c);
            }
            if let Some(v) = &vml_name {
                pkg.remove_part(v);
            }
            strip_relationships(&mut rels, &[REL_COMMENTS, REL_VML]);
            xmlpatch::remove_element(&mut sheet_xml, "legacyDrawing");
        } else {
            let cname = comments_name.clone().unwrap_or_else(|| {
                let n = (1..)
                    .find(|n| pkg.part(&format!("xl/comments{n}.xml")).is_none())
                    .unwrap_or(1);
                format!("xl/comments{n}.xml")
            });
            xmlpatch::set_part_text(pkg, &cname, comments_xml(&legacy))?;
            let vname = vml_name.clone().unwrap_or_else(|| {
                let v = (1..)
                    .find(|v| {
                        pkg.part(&format!("xl/drawings/vmlDrawing{v}.vml"))
                            .is_none()
                    })
                    .unwrap_or(1);
                format!("xl/drawings/vmlDrawing{v}.vml")
            });
            let base = 1025 + 1024 * u32::try_from(sheet).unwrap_or(0);
            xmlpatch::set_part_text(pkg, &vname, vml_xml(&legacy, base))?;
            add_override(&mut ct, &cname, CT_COMMENTS);
            add_default_vml(&mut ct);
            if comments_name.is_none() {
                let rid = next_rid(&rels);
                insert_relationship(
                    &mut rels,
                    &format!(
                        "<Relationship Id=\"{rid}\" Type=\"{REL_COMMENTS}\" Target=\"../{}\"/>",
                        cname.trim_start_matches("xl/")
                    ),
                );
            }
            if vml_name.is_none() {
                let rid = next_rid(&rels);
                insert_relationship(
                    &mut rels,
                    &format!(
                        "<Relationship Id=\"{rid}\" Type=\"{REL_VML}\" Target=\"../{}\"/>",
                        vname.trim_start_matches("xl/")
                    ),
                );
                if xmlpatch::find_open(&sheet_xml, "legacyDrawing").is_none() {
                    xmlpatch::insert_before_any(
                        &mut sheet_xml,
                        &format!("<legacyDrawing r:id=\"{rid}\"/>"),
                        &[
                            "legacyDrawingHF",
                            "drawingHF",
                            "picture",
                            "oleObjects",
                            "controls",
                            "webPublishItems",
                            "tableParts",
                            "extLst",
                        ],
                    );
                    ensure_r_namespace(&mut sheet_xml);
                }
            }
        }
        xmlpatch::set_part_text(pkg, &rels_name, rels)?;
        xmlpatch::set_part_text(pkg, &sheet_part, sheet_xml)?;
        xmlpatch::set_part_text(pkg, "[Content_Types].xml", ct)
    })
}

// ---- running ----------------------------------------------------------------------------

fn check_password(stored: Option<&str>, given: Option<&str>) -> Result<(), CommandError> {
    match stored {
        None => Ok(()),
        Some("sha") => Err(bad(
            "This file's password uses a hash this version cannot check.",
        )),
        Some(hash) => {
            if given.is_some_and(|g| legacy_password_hash(g).eq_ignore_ascii_case(hash)) {
                Ok(())
            } else {
                Err(bad("The password you supplied is not correct."))
            }
        }
    }
}

pub(crate) fn run(
    x: &mut XlsxState,
    sheet: usize,
    op: &ReviewOp,
    notes_out: &mut Vec<String>,
) -> Result<Vec<CellRef>, CommandError> {
    match op {
        ReviewOp::ProtectSheet { options, password } => {
            if sheet_protection(&x.workbook, sheet).is_some() {
                return Err(bad("The sheet is already protected."));
            }
            let part = xmlpatch::sheet_part_name(&x.workbook, sheet)?;
            let element = protection_element(options, password.as_ref().map(Password::expose));
            let wb = xmlpatch::rewrite(&x.workbook, |pkg| {
                let mut xml = xmlpatch::part_text(pkg, &part)?;
                xmlpatch::remove_element(&mut xml, "sheetProtection");
                xmlpatch::insert_before_any(&mut xml, &element, AFTER_PROTECTION);
                xmlpatch::set_part_text(pkg, &part, xml)
            })?;
            x.restore_workbook(wb);
            Ok(Vec::new())
        }
        ReviewOp::UnprotectSheet { password } => {
            if sheet_protection(&x.workbook, sheet).is_none() {
                return Err(bad("The sheet is not protected."));
            }
            check_password(
                stored_sheet_hash(&x.workbook, sheet).as_deref(),
                password.as_ref().map(Password::expose),
            )?;
            let part = xmlpatch::sheet_part_name(&x.workbook, sheet)?;
            let wb = xmlpatch::rewrite(&x.workbook, |pkg| {
                let mut xml = xmlpatch::part_text(pkg, &part)?;
                xmlpatch::remove_element(&mut xml, "sheetProtection");
                xmlpatch::set_part_text(pkg, &part, xml)
            })?;
            x.restore_workbook(wb);
            Ok(Vec::new())
        }
        ReviewOp::ProtectWorkbook { password } => {
            if workbook_structure_locked(&x.workbook) {
                return Err(bad("The workbook structure is already protected."));
            }
            let part = x.workbook.workbook_part().as_str().to_owned();
            let mut el = String::from("<workbookProtection");
            if let Some(p) = password
                .as_ref()
                .map(Password::expose)
                .filter(|p| !p.is_empty())
            {
                el.push_str(&format!(
                    " workbookPassword=\"{}\"",
                    legacy_password_hash(p)
                ));
            }
            el.push_str(" lockStructure=\"1\"/>");
            let wb = xmlpatch::rewrite(&x.workbook, |pkg| {
                let mut xml = xmlpatch::part_text(pkg, &part)?;
                xmlpatch::remove_element(&mut xml, "workbookProtection");
                let at = ["bookViews", "sheets"]
                    .iter()
                    .filter_map(|n| xmlpatch::find_open(&xml, n))
                    .min()
                    .ok_or_else(|| bad("the workbook part has no sheets element"))?;
                xml.insert_str(at, &el);
                xmlpatch::set_part_text(pkg, &part, xml)
            })?;
            x.restore_workbook(wb);
            Ok(Vec::new())
        }
        ReviewOp::UnprotectWorkbook { password } => {
            if !workbook_structure_locked(&x.workbook) {
                return Err(bad("The workbook structure is not protected."));
            }
            let part = x.workbook.workbook_part().as_str().to_owned();
            let stored = x
                .workbook
                .package()
                .part(&part)
                .and_then(|p| {
                    let t = String::from_utf8_lossy(p.data()).into_owned();
                    xmlpatch::element_text(&t, "workbookProtection")
                })
                .and_then(|e| xmlpatch::attr(&e, "workbookPassword"));
            check_password(stored.as_deref(), password.as_ref().map(Password::expose))?;
            let wb = xmlpatch::rewrite(&x.workbook, |pkg| {
                let mut xml = xmlpatch::part_text(pkg, &part)?;
                xmlpatch::remove_element(&mut xml, "workbookProtection");
                xmlpatch::set_part_text(pkg, &part, xml)
            })?;
            x.restore_workbook(wb);
            Ok(Vec::new())
        }
        ReviewOp::SetNote(note) => {
            let mut all = annotations(&x.workbook, sheet);
            all.notes.retain(|n| (n.row, n.col) != (note.row, note.col));
            all.notes.push(note.clone());
            let wb = write_annotations(&x.workbook, sheet, &all)?;
            x.restore_workbook(wb);
            Ok(vec![CellRef {
                sheet,
                row: note.row,
                col: note.col,
            }])
        }
        ReviewOp::DeleteNote { row, col } => {
            let mut all = annotations(&x.workbook, sheet);
            let before = all.notes.len();
            all.notes.retain(|n| (n.row, n.col) != (*row, *col));
            if all.notes.len() == before {
                notes_out.push("The cell has no note.".to_owned());
                return Ok(Vec::new());
            }
            let wb = write_annotations(&x.workbook, sheet, &all)?;
            x.restore_workbook(wb);
            Ok(vec![CellRef {
                sheet,
                row: *row,
                col: *col,
            }])
        }
        ReviewOp::AddComment {
            row,
            col,
            author,
            text,
        } => {
            let mut all = annotations(&x.workbook, sheet);
            if all.notes.iter().any(|n| (n.row, n.col) == (*row, *col)) {
                return Err(bad(
                    "The cell has a note; delete it before adding a comment.",
                ));
            }
            let item = ThreadItem {
                id: new_guid(),
                author: author.clone(),
                text: text.clone(),
                time: now_stamp(),
            };
            match all
                .threads
                .iter_mut()
                .find(|t| (t.row, t.col) == (*row, *col))
            {
                Some(t) => t.items.push(item),
                None => all.threads.push(Thread {
                    row: *row,
                    col: *col,
                    id: item.id.clone(),
                    items: vec![item],
                }),
            }
            all.threads.sort_by_key(|t| (t.row, t.col));
            let wb = write_annotations(&x.workbook, sheet, &all)?;
            x.restore_workbook(wb);
            Ok(vec![CellRef {
                sheet,
                row: *row,
                col: *col,
            }])
        }
        ReviewOp::DeleteComment { row, col } => {
            let mut all = annotations(&x.workbook, sheet);
            let before = all.threads.len();
            all.threads.retain(|t| (t.row, t.col) != (*row, *col));
            if all.threads.len() == before {
                notes_out.push("The cell has no comment.".to_owned());
                return Ok(Vec::new());
            }
            let wb = write_annotations(&x.workbook, sheet, &all)?;
            x.restore_workbook(wb);
            Ok(vec![CellRef {
                sheet,
                row: *row,
                col: *col,
            }])
        }
    }
}

fn protection_element(o: &SheetProtection, password: Option<&str>) -> String {
    let mut el = String::from("<sheetProtection");
    if let Some(p) = password.filter(|p| !p.is_empty()) {
        el.push_str(&format!(" password=\"{}\"", legacy_password_hash(p)));
    }
    el.push_str(" sheet=\"1\"");
    // `objects` and `scenarios` are 1 when protected.
    let mut flag =
        |name: &str, value: bool| el.push_str(&format!(" {name}=\"{}\"", u8::from(value)));
    flag("objects", !o.edit_objects);
    flag("scenarios", !o.edit_scenarios);
    // The permission attributes are 1 when the action is locked.
    for (name, allowed) in [
        ("formatCells", o.format_cells),
        ("formatColumns", o.format_columns),
        ("formatRows", o.format_rows),
        ("insertColumns", o.insert_columns),
        ("insertRows", o.insert_rows),
        ("insertHyperlinks", o.insert_hyperlinks),
        ("deleteColumns", o.delete_columns),
        ("deleteRows", o.delete_rows),
    ] {
        if allowed {
            flag(name, false);
        }
    }
    if !o.select_locked_cells {
        flag("selectLockedCells", true);
    }
    for (name, allowed) in [
        ("sort", o.sort),
        ("autoFilter", o.use_auto_filter),
        ("pivotTables", o.use_pivot_tables),
    ] {
        if allowed {
            flag(name, false);
        }
    }
    if !o.select_unlocked_cells {
        flag("selectUnlockedCells", true);
    }
    el.push_str("/>");
    el
}
