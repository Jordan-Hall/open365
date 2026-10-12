//! Writer Mailings tab: recipient lists, merge fields, the merge itself, envelopes and labels.
//!
//! A merge works on document bytes: every record opens the main document again, updates its
//! fields against that record (`MERGEFIELD`, `ADDRESSBLOCK`, `GREETINGLINE`, `IF`, `SKIPIF`,
//! `NEXT`...) and the results are joined into one document. The main document is never changed
//! by a merge.

use std::cell::Cell;

use open365_docx::{
    CivilDateTime, DocPos, Document, FieldContext, MergeData, RawXml, XmlElement, escape_xml,
    parse_xml,
};
use open365_opc::Limits;
use serde::{Deserialize, Serialize};

use crate::CommandError;
use crate::command::EffectClass;
use crate::ext::writer_xml::{self, fail};

// ---- recipients -------------------------------------------------------------------------------

/// The columns Word's "Type a New List" starts with.
pub const DEFAULT_COLUMNS: [&str; 13] = [
    "Title",
    "First Name",
    "Last Name",
    "Company Name",
    "Address Line 1",
    "Address Line 2",
    "City",
    "State",
    "ZIP Code",
    "Country or Region",
    "Home Phone",
    "Work Phone",
    "E-mail Address",
];

/// A recipient list: a header row of column names and the records.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Recipients {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    /// Whether each record takes part in the merge (the check boxes of Edit Recipient List).
    pub included: Vec<bool>,
    /// File name or description shown for the data source.
    pub source: String,
}

impl Recipients {
    /// Word's blank list with one empty record.
    #[must_use]
    pub fn blank() -> Self {
        let columns: Vec<String> = DEFAULT_COLUMNS.iter().map(|c| (*c).to_owned()).collect();
        let width = columns.len();
        Self {
            columns,
            rows: vec![vec![String::new(); width]],
            included: vec![true],
            source: "New list".to_owned(),
        }
    }

    /// A list from rows of text; the first row names the columns.
    #[must_use]
    pub fn from_table(mut table: Vec<Vec<String>>, source: &str) -> Self {
        if table.is_empty() {
            return Self {
                source: source.to_owned(),
                ..Self::default()
            };
        }
        let columns: Vec<String> = table
            .remove(0)
            .into_iter()
            .enumerate()
            .map(|(i, c)| {
                if c.trim().is_empty() {
                    format!("Column{}", i + 1)
                } else {
                    c.trim().to_owned()
                }
            })
            .collect();
        let width = columns.len();
        let rows: Vec<Vec<String>> = table
            .into_iter()
            .filter(|r| r.iter().any(|c| !c.trim().is_empty()))
            .map(|mut r| {
                r.resize(width, String::new());
                r
            })
            .collect();
        let included = vec![true; rows.len()];
        Self {
            columns,
            rows,
            included,
            source: source.to_owned(),
        }
    }

    /// Reads delimited text. The delimiter (comma, semicolon or tab) comes from the header line;
    /// double quotes enclose fields that hold delimiters, quotes or line breaks.
    #[must_use]
    pub fn from_csv(text: &str, source: &str) -> Self {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let header = text.lines().next().unwrap_or_default();
        let delimiter = [',', ';', '\t']
            .into_iter()
            .max_by_key(|d| header.matches(*d).count())
            .filter(|d| header.contains(*d))
            .unwrap_or(',');
        let mut table = Vec::new();
        let mut row = Vec::new();
        let mut field = String::new();
        let mut quoted = false;
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            if quoted {
                if c == '"' {
                    if chars.peek() == Some(&'"') {
                        chars.next();
                        field.push('"');
                    } else {
                        quoted = false;
                    }
                } else {
                    field.push(c);
                }
            } else if c == '"' && field.is_empty() {
                quoted = true;
            } else if c == delimiter {
                row.push(std::mem::take(&mut field));
            } else if c == '\n' || c == '\r' {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                row.push(std::mem::take(&mut field));
                table.push(std::mem::take(&mut row));
            } else {
                field.push(c);
            }
        }
        if !field.is_empty() || !row.is_empty() {
            row.push(field);
            table.push(row);
        }
        Self::from_table(table, source)
    }

    /// The sheet names of a workbook, for the Select Table dialog.
    ///
    /// # Errors
    /// The bytes are not a workbook.
    pub fn xlsx_sheets(bytes: &[u8]) -> Result<Vec<String>, CommandError> {
        let wb = open365_xlsx::Workbook::open(bytes, &Limits::default())?;
        Ok(wb.sheets().iter().map(|s| s.name.clone()).collect())
    }

    /// Reads sheet `sheet` of a workbook; its first used row holds the column names.
    ///
    /// # Errors
    /// The bytes are not a workbook or the sheet is not a worksheet.
    pub fn from_xlsx(bytes: &[u8], sheet: usize, source: &str) -> Result<Self, CommandError> {
        let wb = open365_xlsx::Workbook::open(bytes, &Limits::default())?;
        let ws = wb
            .sheet(sheet)
            .ok_or_else(|| fail("the sheet is not a worksheet"))?;
        let Some((r1, c1, r2, c2)) = ws.used_range() else {
            return Ok(Self {
                source: source.to_owned(),
                ..Self::default()
            });
        };
        let mut table = Vec::new();
        for r in r1..=r2 {
            table.push(
                (c1..=c2)
                    .map(|c| {
                        wb.cell(sheet, r, c)
                            .map(|cell| cell_text(&cell.value))
                            .unwrap_or_default()
                    })
                    .collect(),
            );
        }
        Ok(Self::from_table(table, source))
    }

    /// Positions of the records that take part, in list order.
    #[must_use]
    pub fn active(&self) -> Vec<usize> {
        (0..self.rows.len())
            .filter(|i| self.included.get(*i).copied().unwrap_or(true))
            .collect()
    }

    /// Index of column `name` (case-insensitive).
    #[must_use]
    pub fn column(&self, name: &str) -> Option<usize> {
        self.columns
            .iter()
            .position(|c| c.eq_ignore_ascii_case(name.trim()))
    }

    /// The text of `column` in record `row`.
    #[must_use]
    pub fn value(&self, row: usize, column: &str) -> Option<&str> {
        let c = self.column(column)?;
        self.rows.get(row)?.get(c).map(String::as_str)
    }

    /// Appends an empty record.
    pub fn add_row(&mut self) {
        self.rows.push(vec![String::new(); self.columns.len()]);
        self.included.push(true);
    }

    /// Removes record `row`.
    pub fn remove_row(&mut self, row: usize) {
        if row < self.rows.len() {
            self.rows.remove(row);
            self.included.remove(row);
        }
    }

    /// Sets one cell.
    pub fn set_cell(&mut self, row: usize, column: usize, text: &str) {
        if let Some(cell) = self.rows.get_mut(row).and_then(|r| r.get_mut(column)) {
            *cell = text.to_owned();
        }
    }

    /// Ticks or clears the check box of record `row`.
    pub fn set_included(&mut self, row: usize, on: bool) {
        if let Some(flag) = self.included.get_mut(row) {
            *flag = on;
        }
    }

    /// Sorts the records by `column` (text compared without case; numbers by value when both
    /// parse). The check boxes travel with their records.
    pub fn sort_by(&mut self, column: usize, ascending: bool) {
        let mut order: Vec<usize> = (0..self.rows.len()).collect();
        order.sort_by(|a, b| {
            let (x, y) = (
                cell_at(&self.rows, *a, column),
                cell_at(&self.rows, *b, column),
            );
            let ord = match (x.trim().parse::<f64>(), y.trim().parse::<f64>()) {
                (Ok(m), Ok(n)) => m.total_cmp(&n),
                _ => x.to_lowercase().cmp(&y.to_lowercase()),
            };
            if ascending { ord } else { ord.reverse() }
        });
        self.rows = order.iter().map(|i| self.rows[*i].clone()).collect();
        self.included = order.iter().map(|i| self.included[*i]).collect();
    }

    /// Filter: keeps ticked only the records for which `keep(value of column)` holds.
    pub fn filter(&mut self, column: usize, keep: impl Fn(&str) -> bool) {
        for i in 0..self.rows.len() {
            let keep_it = keep(cell_at(&self.rows, i, column));
            self.included[i] = keep_it;
        }
    }

    /// The next record at or after `from` (wrapping once) whose `column` (all columns when
    /// `None`) contains `needle`, without regard to case.
    #[must_use]
    pub fn find(&self, needle: &str, column: Option<usize>, from: usize) -> Option<usize> {
        let n = self.rows.len();
        let needle = needle.to_lowercase();
        (0..n).map(|k| (from + k) % n.max(1)).find(|i| {
            let row = &self.rows[*i];
            match column {
                Some(c) => row
                    .get(c)
                    .is_some_and(|v| v.to_lowercase().contains(&needle)),
                None => row.iter().any(|v| v.to_lowercase().contains(&needle)),
            }
        })
    }
}

fn cell_at(rows: &[Vec<String>], row: usize, column: usize) -> &str {
    rows.get(row)
        .and_then(|r| r.get(column))
        .map_or("", String::as_str)
}

fn cell_text(v: &open365_xlsx::CellValue) -> String {
    use open365_xlsx::CellValue as V;
    match v {
        V::Empty => String::new(),
        V::Number(n) => {
            if n.fract() == 0.0 && n.abs() < 1e15 {
                format!("{n:.0}")
            } else {
                n.to_string()
            }
        }
        V::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_owned(),
        V::Error(_) => String::new(),
        other => other.as_text().unwrap_or_default().to_owned(),
    }
}

// ---- field mapping ----------------------------------------------------------------------------

/// The address parts Address Block and Greeting Line use, with the column names Word looks for
/// first (Match Fields starts from these).
pub const MAP_PARTS: [(&str, &str, &[&str]); 11] = [
    ("_TITLE0_", "Courtesy Title", &["Title", "Courtesy Title"]),
    (
        "_FIRST0_",
        "First Name",
        &["First Name", "FirstName", "First"],
    ),
    (
        "_LAST0_",
        "Last Name",
        &["Last Name", "LastName", "Surname", "Last"],
    ),
    ("_SUFFIX0_", "Suffix", &["Suffix"]),
    (
        "_COMPANY_",
        "Company",
        &["Company Name", "Company", "Organisation", "Organization"],
    ),
    (
        "_STREET1_",
        "Address 1",
        &["Address Line 1", "Address 1", "Address", "Street"],
    ),
    ("_STREET2_", "Address 2", &["Address Line 2", "Address 2"]),
    ("_CITY_", "City", &["City", "Town"]),
    ("_STATE_", "State", &["State", "County", "Province"]),
    (
        "_POSTAL_",
        "Postal Code",
        &["ZIP Code", "Postal Code", "Postcode", "Zip"],
    ),
    (
        "_COUNTRY_",
        "Country or Region",
        &["Country or Region", "Country", "Region"],
    ),
];

/// Part of an address mapped to a column of the list (`None`: "(not matched)").
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FieldMap(pub Vec<(String, Option<String>)>);

impl FieldMap {
    /// The automatic match of Word's Match Fields dialog.
    #[must_use]
    pub fn auto(rec: &Recipients) -> Self {
        Self(
            MAP_PARTS
                .iter()
                .map(|(key, _, names)| {
                    let col = names
                        .iter()
                        .find_map(|n| rec.column(n).map(|i| rec.columns[i].clone()));
                    ((*key).to_owned(), col)
                })
                .collect(),
        )
    }

    /// Maps `key` (`_FIRST0_`...) to `column`; `None` clears it.
    pub fn set(&mut self, key: &str, column: Option<&str>) {
        if let Some(slot) = self.0.iter_mut().find(|(k, _)| k == key) {
            slot.1 = column.map(str::to_owned);
        }
    }

    fn column(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, _)| k == key)
            .and_then(|(_, c)| c.as_deref())
    }
}

// ---- the merge ---------------------------------------------------------------------------------

/// What kind of document a merge builds (Start Mail Merge).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeKind {
    /// A letter for each record, each on a new page.
    #[default]
    Letters,
    /// E-mail messages: one document per record, joined like letters.
    Email,
    /// Envelopes: one per record, each on a new page.
    Envelopes,
    /// Label sheets: the label table holds `NEXT` fields; sheets follow each other.
    Labels,
    /// A catalogue: all records one after the other without page breaks.
    Directory,
    /// Plain document: no merge.
    Normal,
}

impl MergeKind {
    /// Word's name for the type, as the Start Mail Merge menu shows it.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Letters => "Letters",
            Self::Email => "E-mail Messages",
            Self::Envelopes => "Envelopes",
            Self::Labels => "Labels",
            Self::Directory => "Directory",
            Self::Normal => "Normal Document",
        }
    }

    fn page_break_between(self) -> bool {
        !matches!(self, Self::Directory | Self::Normal)
    }
}

/// One record as `MergeData`. `NEXT` moves it to the next record while fields update.
struct RecordView<'a> {
    rec: &'a Recipients,
    map: &'a FieldMap,
    order: &'a [usize],
    cursor: Cell<usize>,
    seq: usize,
    skip: Cell<bool>,
}

impl MergeData for RecordView<'_> {
    fn value(&self, field: &str) -> Option<String> {
        let at = self.cursor.get();
        let Some(row) = self.order.get(at).copied() else {
            // Past the last record (the cells of a label sheet that run out): show nothing.
            return Some(String::new());
        };
        match field {
            "_MERGEREC_" => return Some((row + 1).to_string()),
            "_MERGESEQ_" => return Some(self.seq.to_string()),
            _ => {}
        }
        if field.starts_with('_') && field.ends_with('_') {
            let col = self.map.column(field);
            return Some(
                col.and_then(|c| self.rec.value(row, c))
                    .unwrap_or_default()
                    .to_owned(),
            );
        }
        self.rec.value(row, field).map(str::to_owned)
    }

    fn signal(&self, control: &str) {
        match control {
            "SKIPIF" => self.skip.set(true),
            "NEXT" | "NEXTIF" => self.cursor.set(self.cursor.get() + 1),
            _ => {}
        }
    }
}

/// What a finished merge produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeOutput {
    pub bytes: Vec<u8>,
    /// Records that went into the document.
    pub merged: usize,
    /// Records `SKIPIF` left out.
    pub skipped: usize,
}

/// Merges the document `source` with the records `order` (positions in `rec`, in the order to
/// merge) and returns the merged document.
///
/// # Errors
/// The source is not a document or a field cannot be written.
pub fn merge_documents(
    source: &[u8],
    rec: &Recipients,
    map: &FieldMap,
    order: &[usize],
    kind: MergeKind,
    now: CivilDateTime,
) -> Result<MergeOutput, CommandError> {
    let limits = Limits::default();
    let mut parts: Vec<Vec<u8>> = Vec::new();
    let (mut pos, mut seq, mut skipped) = (0usize, 0usize, 0usize);
    while pos < order.len() {
        let mut doc = Document::open(source, &limits)?;
        let view = RecordView {
            rec,
            map,
            order,
            cursor: Cell::new(pos),
            seq: seq + 1,
            skip: Cell::new(false),
        };
        let mut ctx = FieldContext::new(now);
        ctx.merge = Some(&view);
        doc.update_fields(&ctx)?;
        pos = view.cursor.get() + 1;
        if view.skip.get() {
            skipped += 1;
            continue;
        }
        seq += 1;
        parts.push(doc.save()?);
    }
    let merged = parts.len();
    let bytes = join(source, parts, kind.page_break_between())?;
    Ok(MergeOutput {
        bytes,
        merged,
        skipped,
    })
}

const NS_W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

fn body_of(root: &mut XmlElement) -> Option<&mut XmlElement> {
    root.children.iter_mut().find_map(|c| match c {
        RawXml::Element(e) if e.head.is_w("body") => Some(e),
        _ => None,
    })
}

fn is_sect(c: &RawXml) -> bool {
    matches!(c, RawXml::Element(e) if e.head.is_w("sectPr"))
}

/// Joins merged documents: the first one's package, then the body content of the others.
fn join(source: &[u8], parts: Vec<Vec<u8>>, page_break: bool) -> Result<Vec<u8>, CommandError> {
    let limits = Limits::default();
    let mut base = match parts.first() {
        Some(first) => Document::open(first, &limits)?,
        None => {
            // No record: the merge is empty, which Word reports instead; keep a blank result.
            let mut d = Document::open(source, &limits)?;
            writer_xml::rewrite(&mut d, |main| {
                if let Some(body) = body_of(main.root()) {
                    body.children.retain(is_sect);
                }
                Ok(())
            })?;
            return Ok(d.save()?);
        }
    };
    if parts.len() == 1 {
        return Ok(base.save()?);
    }
    let mut later: Vec<Vec<RawXml>> = Vec::new();
    for bytes in &parts[1..] {
        let mut d = Document::open(bytes, &limits)?;
        let mut taken = Vec::new();
        let name = d.main_part().clone();
        let saved = d.save()?;
        let pkg = open365_opc::Package::read(&saved, &limits).map_err(|e| fail(e.to_string()))?;
        let part = pkg
            .part(name.as_str())
            .ok_or_else(|| fail("the document has no main part"))?;
        let mut parsed = parse_xml(part.data())?;
        if let Some(body) = body_of(&mut parsed.root) {
            taken.extend(body.children.drain(..).filter(|c| !is_sect(c)));
        }
        later.push(taken);
    }
    writer_xml::rewrite(&mut base, |main| {
        let like = main.root().head.clone();
        let body = body_of(main.root()).ok_or_else(|| fail("the document has no body"))?;
        let at = body
            .children
            .iter()
            .rposition(is_sect)
            .unwrap_or(body.children.len());
        let tail: Vec<RawXml> = body.children.split_off(at);
        for chunk in later {
            if page_break {
                body.children.push(RawXml::Element(break_paragraph(&like)));
            }
            body.children.extend(chunk);
        }
        body.children.extend(tail);
        Ok(())
    })?;
    Ok(base.save()?)
}

/// A paragraph with a page break, between two merged records.
fn break_paragraph(like: &open365_docx::ElementHead) -> XmlElement {
    let src = format!("<w:p xmlns:w=\"{NS_W}\"><w:r><w:br w:type=\"page\"/></w:r></w:p>");
    let mut p = parse_xml(src.as_bytes())
        .map(|x| x.root)
        .unwrap_or_else(|_| like.sibling("p").into_empty());
    p.head = like.sibling("p");
    p
}

// ---- operations -------------------------------------------------------------------------------

/// A paper size for envelopes, in twentieths of a point (landscape: `width` is the long side).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvelopeSize {
    pub name: String,
    pub width: u32,
    pub height: u32,
}

impl EnvelopeSize {
    /// Word's envelope sizes.
    #[must_use]
    pub fn all() -> Vec<Self> {
        let mm = |m: f64| (m / 25.4 * 1440.0).round() as u32;
        let inch = |i: f64| (i * 1440.0).round() as u32;
        let e = |n: &str, w: u32, h: u32| Self {
            name: n.to_owned(),
            width: w,
            height: h,
        };
        vec![
            e("Size 10", inch(9.5), inch(4.125)),
            e("Size 9", inch(8.875), inch(3.875)),
            e("Size 11", inch(10.375), inch(4.5)),
            e("Size 12", inch(11.0), inch(4.75)),
            e("Monarch", inch(7.5), inch(3.875)),
            e("DL", mm(220.0), mm(110.0)),
            e("C4", mm(324.0), mm(229.0)),
            e("C5", mm(229.0), mm(162.0)),
            e("C6", mm(162.0), mm(114.0)),
            e("B5", mm(250.0), mm(176.0)),
        ]
    }

    /// The size called `name`, Size 10 when unknown.
    #[must_use]
    pub fn named(name: &str) -> Self {
        Self::all()
            .into_iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
            .unwrap_or_else(|| Self::all().remove(0))
    }
}

/// A label product: page and label geometry in twentieths of a point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelDef {
    pub name: String,
    pub page_width: u32,
    pub page_height: u32,
    pub columns: u32,
    pub rows: u32,
    pub label_width: u32,
    pub label_height: u32,
    /// Distance from one label's left edge to the next one's.
    pub pitch_x: u32,
    /// Distance from one label's top edge to the next one's.
    pub pitch_y: u32,
    pub margin_left: u32,
    pub margin_top: u32,
}

impl LabelDef {
    /// Common sheets with the sizes their makers publish. Letter and A4 pages.
    #[must_use]
    pub fn all() -> Vec<Self> {
        let inch = |i: f64| (i * 1440.0).round() as u32;
        let mm = |m: f64| (m / 25.4 * 1440.0).round() as u32;
        let us = |n: &str, c, r, lw: f64, lh: f64, px: f64, py: f64, ml: f64, mt: f64| Self {
            name: n.to_owned(),
            page_width: inch(8.5),
            page_height: inch(11.0),
            columns: c,
            rows: r,
            label_width: inch(lw),
            label_height: inch(lh),
            pitch_x: inch(px),
            pitch_y: inch(py),
            margin_left: inch(ml),
            margin_top: inch(mt),
        };
        let a4 = |n: &str, c, r, lw: f64, lh: f64, px: f64, py: f64, ml: f64, mt: f64| Self {
            name: n.to_owned(),
            page_width: mm(210.0),
            page_height: mm(297.0),
            columns: c,
            rows: r,
            label_width: mm(lw),
            label_height: mm(lh),
            pitch_x: mm(px),
            pitch_y: mm(py),
            margin_left: mm(ml),
            margin_top: mm(mt),
        };
        vec![
            us(
                "30 per page (Avery 5160)",
                3,
                10,
                2.625,
                1.0,
                2.75,
                1.0,
                0.1875,
                0.5,
            ),
            us(
                "20 per page (Avery 5161)",
                2,
                10,
                4.0,
                1.0,
                4.1875,
                1.0,
                0.15625,
                0.5,
            ),
            us(
                "14 per page (Avery 5162)",
                2,
                7,
                4.0,
                1.33,
                4.1875,
                1.33,
                0.15625,
                0.875,
            ),
            us(
                "10 per page (Avery 5163)",
                2,
                5,
                4.0,
                2.0,
                4.1875,
                2.0,
                0.15625,
                0.5,
            ),
            us(
                "6 per page (Avery 5164)",
                2,
                3,
                4.0,
                3.33,
                4.1875,
                3.33,
                0.15625,
                0.5,
            ),
            us(
                "80 per page (Avery 5167)",
                4,
                20,
                1.75,
                0.5,
                2.0625,
                0.5,
                0.3,
                0.5,
            ),
            a4(
                "21 per page (Avery L7160)",
                3,
                7,
                63.5,
                38.1,
                66.0,
                38.1,
                7.2,
                15.15,
            ),
            a4(
                "14 per page (Avery L7163)",
                2,
                7,
                99.1,
                38.1,
                101.6,
                38.1,
                4.65,
                15.15,
            ),
            a4(
                "24 per page (Avery L7159)",
                3,
                8,
                63.5,
                33.9,
                66.0,
                33.9,
                7.2,
                12.9,
            ),
        ]
    }

    /// The product called `name` (a prefix is enough), the first one when unknown.
    #[must_use]
    pub fn named(name: &str) -> Self {
        let low = name.to_lowercase();
        Self::all()
            .into_iter()
            .find(|d| d.name.to_lowercase().contains(&low))
            .unwrap_or_else(|| Self::all().remove(0))
    }

    /// Labels per sheet.
    #[must_use]
    pub fn per_page(&self) -> u32 {
        self.columns * self.rows
    }
}

/// One Mailings-tab edit of the document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MailingsOp {
    /// A field at `at` whose result shows `placeholder` (merge fields, rules, address block...).
    InsertField {
        at: DocPos,
        instruction: String,
        placeholder: String,
    },
    /// Run-level XML (a field built with [`nested_field`]) at `at`.
    InsertRuns { at: DocPos, xml: String },
    /// Add to Document: a new first section of the envelope's size with the addresses in frames.
    Envelope {
        delivery: String,
        return_address: String,
        size: EnvelopeSize,
    },
    /// Replaces the document body with a label sheet. `content` is the run-level XML of the
    /// first label's paragraphs (`<w:p>` elements); with `merge` the other labels start with a
    /// `NEXT` field and hold a copy of it.
    LabelSheet {
        def: LabelDef,
        content: Vec<String>,
        merge: bool,
    },
    /// Update Labels: copies the first label into every other label of the first table.
    UpdateLabels,
}

impl MailingsOp {
    #[must_use]
    pub fn effect(&self) -> EffectClass {
        EffectClass::Edit
    }
}

/// Runs `op`.
///
/// # Errors
/// The operation does not fit the document.
pub fn apply(doc: &mut Document, op: &MailingsOp) -> Result<(), CommandError> {
    match op {
        MailingsOp::InsertField {
            at,
            instruction,
            placeholder,
        } => {
            doc.insert_inline_xml(*at, &complex_field(instruction, placeholder))?;
            Ok(())
        }
        MailingsOp::InsertRuns { at, xml } => {
            doc.insert_inline_xml(*at, xml)?;
            Ok(())
        }
        MailingsOp::Envelope {
            delivery,
            return_address,
            size,
        } => {
            doc.insert_blocks_xml(0, &envelope_xml(delivery, return_address, size))?;
            Ok(())
        }
        MailingsOp::LabelSheet {
            def,
            content,
            merge,
        } => label_sheet(doc, def, content, *merge),
        MailingsOp::UpdateLabels => update_labels(doc),
    }
}

/// Run-level XML of a complex field with a text result.
#[must_use]
pub fn complex_field(instruction: &str, result: &str) -> String {
    let marker = |kind: &str| format!("<w:r><w:fldChar w:fldCharType=\"{kind}\"/></w:r>");
    format!(
        "{}<w:r><w:instrText xml:space=\"preserve\"> {} </w:instrText></w:r>{}{}{}",
        marker("begin"),
        escape_xml(instruction),
        marker("separate"),
        text_run(result),
        marker("end"),
    )
}

/// `MERGEFIELD "column"` as Word writes it (quoted when the name has blanks).
#[must_use]
pub fn merge_field_instruction(column: &str) -> String {
    let name = column.replace('"', "'");
    if name.contains(' ') {
        format!("MERGEFIELD \"{name}\"")
    } else {
        format!("MERGEFIELD {name}")
    }
}

/// A field whose instruction holds a merge field: `head {MERGEFIELD column} tail`. Rules such as
/// `IF`, `SKIPIF` and `NEXTIF` compare a merge field this way.
#[must_use]
pub fn nested_field(head: &str, column: &str, tail: &str, result: &str) -> String {
    let marker = |kind: &str| format!("<w:r><w:fldChar w:fldCharType=\"{kind}\"/></w:r>");
    let instr = |t: &str| {
        format!(
            "<w:r><w:instrText xml:space=\"preserve\"> {} </w:instrText></w:r>",
            escape_xml(t)
        )
    };
    format!(
        "{}{}{}{}{}{}{}",
        marker("begin"),
        instr(head),
        complex_field(
            &merge_field_instruction(column),
            &format!("\u{ab}{column}\u{bb}")
        ),
        instr(tail),
        marker("separate"),
        text_run(result),
        marker("end"),
    )
}

fn text_run(text: &str) -> String {
    format!(
        "<w:r><w:t xml:space=\"preserve\">{}</w:t></w:r>",
        escape_xml(text)
    )
}

/// One paragraph per line of `text`.
fn paragraphs(text: &str, ppr: &str) -> String {
    text.lines()
        .map(|l| format!("<w:p>{ppr}{}</w:p>", text_run(l)))
        .collect()
}

fn frame_ppr(x: u32, y: u32, w: u32, h: u32) -> String {
    format!(
        "<w:pPr><w:framePr w:w=\"{w}\" w:h=\"{h}\" w:hRule=\"exact\" w:hSpace=\"180\" w:wrap=\"auto\" w:vAnchor=\"page\" w:hAnchor=\"page\" w:x=\"{x}\" w:y=\"{y}\"/><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr>"
    )
}

/// The addresses of an envelope and the paragraph that ends its section. The delivery address
/// sits right of centre and just above the middle; the return address in the top-left corner.
fn envelope_xml(delivery: &str, return_address: &str, size: &EnvelopeSize) -> String {
    let (w, h) = (size.width.max(size.height), size.width.min(size.height));
    let mut xml = String::new();
    if !return_address.trim().is_empty() {
        xml.push_str(&paragraphs(
            return_address,
            &frame_ppr(360, 360, (w / 3).max(2880), 1440),
        ));
    }
    let dx = w / 2 - 720;
    let dy = h / 2 - 360;
    if delivery.trim().is_empty() {
        xml.push_str(&format!(
            "<w:p>{}</w:p>",
            frame_ppr(dx, dy, (w - dx) - 720, 1800)
        ));
    } else {
        xml.push_str(&paragraphs(
            delivery,
            &frame_ppr(dx, dy, (w - dx) - 720, 1800),
        ));
    }
    xml.push_str(&format!(
        "<w:p><w:pPr><w:sectPr><w:pgSz w:w=\"{w}\" w:h=\"{h}\" w:orient=\"landscape\"/><w:pgMar w:top=\"360\" w:right=\"360\" w:bottom=\"360\" w:left=\"360\" w:header=\"0\" w:footer=\"0\" w:gutter=\"0\"/></w:sectPr></w:pPr></w:p>"
    ));
    xml
}

fn parse_children(fragment: &str) -> Result<Vec<RawXml>, CommandError> {
    let src = format!("<w:body xmlns:w=\"{NS_W}\">{fragment}</w:body>");
    let mut parsed = parse_xml(src.as_bytes())?;
    Ok(std::mem::take(&mut parsed.root.children))
}

fn next_field_paragraph(content: &str) -> String {
    // The NEXT field goes into the first paragraph, before its text.
    let field = complex_field("NEXT", "\u{ab}Next Record\u{bb}");
    match content.find("</w:pPr>") {
        Some(i) => {
            let at = i + "</w:pPr>".len();
            format!("{}{field}{}", &content[..at], &content[at..])
        }
        None => match content.find('>') {
            Some(i) => format!("{}{field}{}", &content[..=i], &content[i + 1..]),
            None => content.to_owned(),
        },
    }
}

fn label_cell(width: u32, paras: &str) -> String {
    format!("<w:tc><w:tcPr><w:tcW w:w=\"{width}\" w:type=\"dxa\"/></w:tcPr>{paras}</w:tc>")
}

fn label_sheet(
    doc: &mut Document,
    def: &LabelDef,
    content: &[String],
    merge: bool,
) -> Result<(), CommandError> {
    if def.columns == 0 || def.rows == 0 {
        return Err(fail("the label sheet has no labels"));
    }
    let first: String = if content.is_empty() {
        "<w:p/>".to_owned()
    } else {
        content.concat()
    };
    let gap = def.pitch_x.saturating_sub(def.label_width);
    let mut grid = String::new();
    let mut total = 0;
    for c in 0..def.columns {
        grid.push_str(&format!("<w:gridCol w:w=\"{}\"/>", def.label_width));
        total += def.label_width;
        if c + 1 < def.columns {
            grid.push_str(&format!("<w:gridCol w:w=\"{gap}\"/>"));
            total += gap;
        }
    }
    let mut rows = String::new();
    let mut index = 0;
    for _ in 0..def.rows {
        let mut cells = String::new();
        for c in 0..def.columns {
            let body = if index == 0 {
                first.clone()
            } else if merge {
                let paras = next_field_paragraph(content.first().map_or("<w:p/>", String::as_str));
                let rest: String = content.iter().skip(1).map(String::as_str).collect();
                format!("{paras}{rest}")
            } else {
                first.clone()
            };
            cells.push_str(&label_cell(def.label_width, &body));
            if c + 1 < def.columns {
                cells.push_str(&label_cell(gap, "<w:p/>"));
            }
            index += 1;
        }
        rows.push_str(&format!(
            "<w:tr><w:trPr><w:cantSplit/><w:trHeight w:val=\"{}\" w:hRule=\"exact\"/></w:trPr>{cells}</w:tr>",
            def.pitch_y
        ));
    }
    let table = format!(
        "<w:tbl><w:tblPr><w:tblW w:w=\"{total}\" w:type=\"dxa\"/><w:tblLayout w:type=\"fixed\"/><w:tblCellMar><w:left w:w=\"115\" w:type=\"dxa\"/><w:right w:w=\"115\" w:type=\"dxa\"/></w:tblCellMar></w:tblPr><w:tblGrid>{grid}</w:tblGrid>{rows}</w:tbl><w:p><w:pPr><w:spacing w:after=\"0\" w:line=\"20\" w:lineRule=\"exact\"/></w:pPr></w:p>"
    );
    let sect = format!(
        "<w:sectPr><w:pgSz w:w=\"{}\" w:h=\"{}\"/><w:pgMar w:top=\"{}\" w:right=\"0\" w:bottom=\"0\" w:left=\"{}\" w:header=\"0\" w:footer=\"0\" w:gutter=\"0\"/></w:sectPr>",
        def.page_width, def.page_height, def.margin_top, def.margin_left
    );
    let new_children = parse_children(&format!("{table}{sect}"))?;
    writer_xml::rewrite(doc, |main| {
        let body = body_of(main.root()).ok_or_else(|| fail("the document has no body"))?;
        body.children = new_children;
        Ok(())
    })
}

fn is_el(c: &RawXml, local: &str) -> bool {
    matches!(c, RawXml::Element(e) if e.head.is_w(local))
}

fn update_labels(doc: &mut Document) -> Result<(), CommandError> {
    writer_xml::rewrite(doc, |main| {
        let body = body_of(main.root()).ok_or_else(|| fail("the document has no body"))?;
        let table = body
            .children
            .iter_mut()
            .find_map(|c| match c {
                RawXml::Element(e) if e.head.is_w("tbl") => Some(e),
                _ => None,
            })
            .ok_or_else(|| fail("there is no label table"))?;
        let mut first: Option<Vec<RawXml>> = None;
        for row in table.children.iter_mut().filter(|c| is_el(c, "tr")) {
            let RawXml::Element(row) = row else { continue };
            let mut label_index = 0usize;
            for cell in row.children.iter_mut().filter(|c| is_el(c, "tc")) {
                let RawXml::Element(cell) = cell else {
                    continue;
                };
                let is_label = label_index.is_multiple_of(2);
                label_index += 1;
                if !is_label {
                    continue;
                }
                let paras: Vec<RawXml> = cell
                    .children
                    .iter()
                    .filter(|c| is_el(c, "p"))
                    .cloned()
                    .collect();
                match &first {
                    None => first = Some(paras),
                    Some(src) => {
                        let props: Vec<RawXml> = cell
                            .children
                            .iter()
                            .filter(|c| is_el(c, "tcPr"))
                            .cloned()
                            .collect();
                        let mut copy = src.clone();
                        copy.retain(|p| {
                            // Drop a NEXT field a former update left in the copy.
                            !matches!(p, RawXml::Element(e) if writer_xml::para_text(e).contains("\u{ab}Next Record\u{bb}") && writer_xml::para_text(e).trim() == "\u{ab}Next Record\u{bb}")
                        });
                        let mut out = props;
                        out.extend(copy);
                        cell.children = out;
                        if let Some(RawXml::Element(p)) =
                            cell.children.iter_mut().find(|c| is_el(c, "p"))
                        {
                            let nodes = parse_children(&format!(
                                "<w:p>{}</w:p>",
                                complex_field("NEXT", "\u{ab}Next Record\u{bb}")
                            ))?;
                            if let Some(RawXml::Element(np)) = nodes.into_iter().next() {
                                let at = p
                                    .children
                                    .iter()
                                    .position(|c| !is_el(c, "pPr"))
                                    .unwrap_or(p.children.len());
                                for (k, n) in np.children.into_iter().enumerate() {
                                    p.children.insert(at + k, n);
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const CT: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#;
    const RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;
    const DOC_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"/>"#;

    fn docx(body: &str) -> Vec<u8> {
        let document = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><w:document xmlns:w=\"{NS_W}\"><w:body>{body}<w:sectPr/></w:body></w:document>"
        );
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default();
        for (name, data) in [
            ("[Content_Types].xml", CT),
            ("_rels/.rels", RELS),
            ("word/document.xml", document.as_str()),
            ("word/_rels/document.xml.rels", DOC_RELS),
        ] {
            zip.start_file(name, opts).unwrap();
            zip.write_all(data.as_bytes()).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    fn texts(bytes: &[u8]) -> Vec<String> {
        let d = Document::open(bytes, &Limits::default()).unwrap();
        (0..d.paragraph_count())
            .map(|i| d.paragraph_text(i).unwrap())
            .collect()
    }

    fn now() -> CivilDateTime {
        CivilDateTime::new(2026, 1, 2, 3, 4, 5).unwrap()
    }

    fn list() -> Recipients {
        Recipients::from_csv(
            "First Name,Last Name,City\nAda,Lovelace,London\n\"Grace, H\",Hopper,\"New\nYork\"\nAlan,Turing,Wilmslow\n",
            "people.csv",
        )
    }

    #[test]
    fn csv_handles_quotes_and_line_breaks() {
        let l = list();
        assert_eq!(l.columns, ["First Name", "Last Name", "City"]);
        assert_eq!(l.rows.len(), 3);
        assert_eq!(l.rows[1][0], "Grace, H");
        assert_eq!(l.rows[1][2], "New\nYork");
        assert_eq!(Recipients::from_csv("a;b\n1;2", "x").rows[0], ["1", "2"]);
    }

    #[test]
    fn merge_fills_one_page_per_record() {
        let src = docx(&format!(
            "<w:p>{}<w:r><w:t xml:space=\"preserve\"> - </w:t></w:r>{}</w:p>",
            complex_field("MERGEFIELD \"First Name\"", "\u{ab}First Name\u{bb}"),
            complex_field("MERGEFIELD City", "\u{ab}City\u{bb}")
        ));
        let rec = list();
        let map = FieldMap::auto(&rec);
        let out =
            merge_documents(&src, &rec, &map, &rec.active(), MergeKind::Letters, now()).unwrap();
        assert_eq!(out.merged, 3);
        let t = texts(&out.bytes);
        assert!(t.contains(&"Ada - London".to_owned()), "{t:?}");
        assert!(t.contains(&"Alan - Wilmslow".to_owned()), "{t:?}");
    }

    #[test]
    fn skipif_leaves_a_record_out_and_next_consumes_records() {
        let skip = docx(&format!(
            "<w:p>{}{}</w:p>",
            complex_field("SKIPIF \"Ada\" = \"Ada\"", ""),
            complex_field("MERGEFIELD \"First Name\"", "x")
        ));
        let rec = list();
        let map = FieldMap::auto(&rec);
        let out =
            merge_documents(&skip, &rec, &map, &rec.active(), MergeKind::Letters, now()).unwrap();
        assert_eq!((out.merged, out.skipped), (0, 3));
        let two = docx(&format!(
            "<w:p>{}</w:p><w:p>{}{}</w:p>",
            complex_field("MERGEFIELD \"First Name\"", "x"),
            complex_field("NEXT", ""),
            complex_field("MERGEFIELD \"First Name\"", "x")
        ));
        let out =
            merge_documents(&two, &rec, &map, &rec.active(), MergeKind::Directory, now()).unwrap();
        let t = texts(&out.bytes);
        assert_eq!(&t[..2], ["Ada", "Grace, H"], "{t:?}");
        assert_eq!(out.merged, 2);
    }

    #[test]
    fn counters_follow_the_records() {
        let src = docx(&format!(
            "<w:p>{} {}</w:p>",
            complex_field("MERGESEQ", "0"),
            complex_field("MERGEREC", "0")
        ));
        let rec = list();
        let map = FieldMap::auto(&rec);
        let out = merge_documents(&src, &rec, &map, &[1, 2], MergeKind::Directory, now()).unwrap();
        assert_eq!(texts(&out.bytes)[..2], ["12", "23"]);
    }

    #[test]
    fn address_block_uses_the_matched_columns() {
        let src = docx(&format!(
            "<w:p>{}</w:p>",
            complex_field("ADDRESSBLOCK", "\u{ab}AddressBlock\u{bb}")
        ));
        let rec = list();
        let map = FieldMap::auto(&rec);
        let out = merge_documents(&src, &rec, &map, &[0], MergeKind::Letters, now()).unwrap();
        let t = texts(&out.bytes);
        assert_eq!(t[0], "Ada Lovelace");
        assert_eq!(t[1], "London");
    }

    #[test]
    fn label_sheet_and_update_labels() {
        let mut d = Document::open(&docx("<w:p/>"), &Limits::default()).unwrap();
        let def = LabelDef::named("5160");
        assert_eq!(def.per_page(), 30);
        apply(
            &mut d,
            &MailingsOp::LabelSheet {
                def: def.clone(),
                content: vec![format!(
                    "<w:p>{}</w:p>",
                    complex_field("MERGEFIELD City", "\u{ab}City\u{bb}")
                )],
                merge: false,
            },
        )
        .unwrap();
        apply(&mut d, &MailingsOp::UpdateLabels).unwrap();
        let bytes = d.save().unwrap();
        let src = bytes;
        let rec = list();
        let map = FieldMap::auto(&rec);
        let out =
            merge_documents(&src, &rec, &map, &rec.active(), MergeKind::Labels, now()).unwrap();
        let t = texts(&out.bytes);
        assert!(t.contains(&"London".to_owned()), "{t:?}");
        assert!(t.contains(&"Wilmslow".to_owned()), "{t:?}");
    }
}
