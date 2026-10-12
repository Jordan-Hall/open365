//! Creating a table (Insert > Table, Home > Format as Table).

use open365_opc::TargetMode;

use super::shift::ShiftDirection;
use super::{MAIN, XML_DECL, content_type, esc, names, rel, relative_target};
use crate::XlsxError;
use crate::refs::{MAX_COL, MAX_ROW};
use crate::sheet::{CellValue, Rect};
use crate::structure::range_a1;
use crate::tables::Table;
use crate::workbook::{CellInput, Workbook};

/// Whether the first row of the range is the header row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HeaderChoice {
    Yes,
    No,
    /// Excel's "My table has headers" guess: on when every cell of the first row is text.
    #[default]
    Guess,
}

/// The table style.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TableStyleChoice {
    /// `TableStyleMedium2`, what Insert > Table applies.
    #[default]
    Default,
    Named(String),
    /// Format as Table > Clear: no style.
    None,
}

/// A table to create.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableSpec {
    /// The cells the table covers, `(row1, col1, row2, col2)`.
    pub range: Rect,
    /// The table name; `Table1`, `Table2` ... when absent.
    pub name: Option<String>,
    pub headers: HeaderChoice,
    pub style: TableStyleChoice,
    /// Add a totals row below the range.
    pub totals_row: bool,
    /// Filter buttons in the header row (on in Excel by default).
    pub auto_filter: bool,
    pub first_column: bool,
    pub last_column: bool,
    pub row_stripes: bool,
    pub column_stripes: bool,
}

impl Default for TableSpec {
    fn default() -> Self {
        Self {
            range: (1, 1, 1, 1),
            name: None,
            headers: HeaderChoice::Guess,
            style: TableStyleChoice::Default,
            totals_row: false,
            auto_filter: true,
            first_column: false,
            last_column: false,
            row_stripes: true,
            column_stripes: false,
        }
    }
}

/// What [`Workbook::create_table`] made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableInfo {
    pub part: String,
    pub id: u32,
    pub name: String,
    /// The table range including the header and totals rows.
    pub range: Rect,
    /// A header row was inserted because the data had none.
    pub header_inserted: bool,
    /// Formula cells that moved or changed and need a recalculation (`sheet, row, col`), plus the
    /// totals-row formulas.
    pub recalc: Vec<(usize, u32, u32)>,
}

fn flag(b: bool) -> &'static str {
    if b { "1" } else { "0" }
}

/// A column name as it appears inside a structured reference.
fn structured_column(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if matches!(c, '[' | ']' | '#' | '\'') {
            out.push('\'');
        }
        out.push(c);
    }
    out
}

impl Workbook {
    fn cell_text(&self, sheet: usize, row: u32, col: u32) -> Option<String> {
        match &self.cell(sheet, row, col)?.value {
            CellValue::Str(t) | CellValue::InlineStr(t) => Some(t.to_string()),
            CellValue::SharedString { text, .. } => Some(text.to_string()),
            CellValue::Number(n) => Some(format!("{n}")),
            CellValue::Bool(b) => Some(if *b { "TRUE" } else { "FALSE" }.to_owned()),
            _ => None,
        }
    }

    /// Excel's guess whether the first row of `rect` is a header: every cell is non-empty text.
    pub fn guess_table_headers(&self, sheet: usize, rect: Rect) -> bool {
        let (r1, c1, r2, c2) = rect;
        if r2 <= r1 {
            return false;
        }
        (c1..=c2).all(|c| {
            self.cell(sheet, r1, c).is_some_and(|cell| {
                cell.formula.is_none()
                    && match &cell.value {
                        CellValue::Str(t) | CellValue::InlineStr(t) => !t.trim().is_empty(),
                        CellValue::SharedString { text, .. } => !text.trim().is_empty(),
                        _ => false,
                    }
            })
        })
    }

    fn next_table_id(&self) -> u32 {
        self.tables().map(|(_, t)| t.id).max().unwrap_or(0) + 1
    }

    fn default_table_name(&self) -> String {
        (1u32..)
            .map(|n| format!("Table{n}"))
            .find(|n| !self.table_name_taken(n))
            .unwrap_or_default()
    }

    fn table_name_taken(&self, name: &str) -> bool {
        self.tables().any(|(_, t)| {
            t.name.eq_ignore_ascii_case(name) || t.display_name.eq_ignore_ascii_case(name)
        }) || self
            .defined_names
            .iter()
            .any(|n| n.name.eq_ignore_ascii_case(name))
    }

    /// Creates a table over `spec.range` of worksheet `sheet`, writing `xl/tables/tableN.xml` as Excel
    /// does and wiring `tableParts`, the sheet relationship and the content type.
    ///
    /// Without a header row (`HeaderChoice::No`, or a guess of "no") Excel inserts one above the data,
    /// shifting the cells below down; this does the same and names the headers `Column1`, `Column2` ...
    /// With `totals_row` the totals row goes under the range (cells below shift down when occupied),
    /// labelled `Total` on the first column and summing (numbers) or counting (anything else) the last.
    /// Header cells that are not text become text and duplicates are numbered (`A`, `A2`).
    ///
    /// # Errors
    /// The sheet is not a worksheet, the range is outside the grid, overlaps a table, a merge or an array
    /// formula, or the name is illegal or taken.
    pub fn create_table(&mut self, sheet: usize, spec: &TableSpec) -> Result<TableInfo, XlsxError> {
        self.require_transitional()?;
        let (r1, c1, mut r2, c2) = spec.range;
        if r1 == 0 || c1 == 0 || r1 > r2 || c1 > c2 || r2 > MAX_ROW || c2 > MAX_COL {
            return Err(XlsxError::UnsupportedEdit(
                "the table range is outside the worksheet".to_owned(),
            ));
        }
        let sheet_part = self
            .sheet_part(sheet)
            .cloned()
            .ok_or(XlsxError::NotAWorksheet(sheet))?;
        {
            let ws = self.worksheet_mut(sheet)?;
            let hits = |a: Rect, b: Rect| a.0 <= b.2 && b.0 <= a.2 && a.1 <= b.3 && b.1 <= a.3;
            if ws
                .tables()
                .iter()
                .any(|t| t.rect.is_some_and(|r| hits(r, spec.range)))
            {
                return Err(XlsxError::UnsupportedEdit(
                    "a table cannot overlap another table".to_owned(),
                ));
            }
            if let Some(m) = ws.root_element("mergeCells")
                && m.elements()
                    .filter_map(|e| e.head.attr("ref").and_then(crate::refs::parse_range))
                    .any(|r| hits(r, spec.range))
            {
                return Err(XlsxError::UnsupportedEdit(
                    "a table cannot contain merged cells".to_owned(),
                ));
            }
            if ws
                .array_formulas()
                .iter()
                .any(|a| hits(a.range, spec.range))
            {
                return Err(XlsxError::UnsupportedEdit(
                    "a table cannot contain an array formula".to_owned(),
                ));
            }
        }
        let name = match &spec.name {
            Some(n) => {
                names::validate_defined_name(n)?;
                if self.table_name_taken(n) {
                    return Err(XlsxError::UnsupportedEdit(format!(
                        "the name {n:?} is already used by a table or a name"
                    )));
                }
                n.clone()
            }
            None => self.default_table_name(),
        };
        let has_headers = match spec.headers {
            HeaderChoice::Yes => true,
            HeaderChoice::No => false,
            HeaderChoice::Guess => self.guess_table_headers(sheet, spec.range),
        };
        self.edited()?;
        let mut recalc = Vec::new();
        let mut header_inserted = false;
        let mut top = r1;
        if has_headers {
            if r2 == r1 {
                // A table needs a data row: take the blank row under the header.
                r2 += 1;
            }
        } else {
            recalc.extend(self.shift_cells(sheet, (r1, c1, r1, c2), ShiftDirection::Down)?);
            r2 += 1;
            header_inserted = true;
        }
        // Header names.
        let mut headers: Vec<String> = Vec::new();
        for (i, c) in (c1..=c2).enumerate() {
            let base = if header_inserted {
                None
            } else {
                self.cell_text(sheet, top, c).filter(|t| !t.is_empty())
            };
            let mut candidate = base.unwrap_or_else(|| format!("Column{}", i + 1));
            if headers.iter().any(|h| h.eq_ignore_ascii_case(&candidate)) {
                let root = candidate.clone();
                candidate = (2u32..)
                    .map(|n| format!("{root}{n}"))
                    .find(|n| !headers.iter().any(|h| h.eq_ignore_ascii_case(n)))
                    .unwrap_or(root);
            }
            headers.push(candidate);
        }
        for (i, c) in (c1..=c2).enumerate() {
            let existing = self.cell_text(sheet, top, c);
            let is_text = self.cell(sheet, top, c).is_some_and(|x| {
                x.formula.is_none()
                    && matches!(
                        x.value,
                        CellValue::Str(_)
                            | CellValue::InlineStr(_)
                            | CellValue::SharedString { .. }
                    )
            });
            if existing.as_deref() != Some(headers[i].as_str()) || !is_text {
                self.set_cell_value(sheet, top, c, CellInput::Text(headers[i].clone()))?;
            }
        }

        // Totals row.
        let mut last_row = r2;
        let mut last_function = "sum";
        if spec.totals_row {
            let below = (r2 + 1, c1, r2 + 1, c2);
            if r2 + 1 > MAX_ROW {
                return Err(XlsxError::UnsupportedEdit(
                    "no room for a totals row".to_owned(),
                ));
            }
            if !self.sheets[sheet]
                .data
                .as_ref()
                .is_some_and(|ws| ws.rect_is_blank(below))
            {
                recalc.extend(self.shift_cells(sheet, below, ShiftDirection::Down)?);
            }
            last_row = r2 + 1;
            let numeric = (top + 1..=r2).all(|r| {
                matches!(
                    self.cell(sheet, r, c2).map(|x| &x.value),
                    Some(CellValue::Number(_) | CellValue::Empty) | None
                )
            }) && (top + 1..=r2).any(|r| self.cell(sheet, r, c2).is_some());
            if !numeric {
                last_function = "count";
            }
        }
        top = top.min(r1);
        let full: Rect = (top, c1, last_row, c2);
        let id = self.next_table_id();

        // The table part.
        let mut x = String::from(XML_DECL);
        x.push_str(&format!(
            "<table xmlns=\"{MAIN}\" id=\"{id}\" name=\"{n}\" displayName=\"{n}\" ref=\"{r}\"",
            n = esc(&name),
            r = range_a1(full)
        ));
        if spec.totals_row {
            x.push_str(" totalsRowCount=\"1\"");
        } else {
            x.push_str(" totalsRowShown=\"0\"");
        }
        x.push('>');
        if spec.auto_filter {
            x.push_str(&format!(
                "<autoFilter ref=\"{}\"/>",
                range_a1((top, c1, r2, c2))
            ));
        }
        x.push_str(&format!("<tableColumns count=\"{}\">", headers.len()));
        let width = headers.len();
        for (i, h) in headers.iter().enumerate() {
            x.push_str(&format!(
                "<tableColumn id=\"{}\" name=\"{}\"",
                i + 1,
                esc(h)
            ));
            if spec.totals_row {
                if i + 1 == width {
                    x.push_str(&format!(" totalsRowFunction=\"{last_function}\""));
                } else if i == 0 {
                    x.push_str(" totalsRowLabel=\"Total\"");
                }
            }
            x.push_str("/>");
        }
        x.push_str("</tableColumns>");
        let style_name = match &spec.style {
            TableStyleChoice::Default => Some("TableStyleMedium2".to_owned()),
            TableStyleChoice::Named(n) => Some(n.clone()),
            TableStyleChoice::None => None,
        };
        x.push_str("<tableStyleInfo");
        if let Some(s) = &style_name {
            x.push_str(&format!(" name=\"{}\"", esc(s)));
        }
        x.push_str(&format!(
            " showFirstColumn=\"{}\" showLastColumn=\"{}\" showRowStripes=\"{}\" showColumnStripes=\"{}\"/></table>",
            flag(spec.first_column),
            flag(spec.last_column),
            flag(spec.row_stripes),
            flag(spec.column_stripes)
        ));
        let part_name = self.next_part_name("xl/tables", "table", "xml");
        self.add_part(&part_name, content_type::TABLE, x.clone().into_bytes())?;
        let rid = self.add_relationship(
            Some(&sheet_part),
            rel::TABLE,
            &relative_target(Some(&sheet_part), &part_name),
            TargetMode::Internal,
        )?;

        // `tableParts` on the sheet.
        {
            let ws = self.worksheet_mut(sheet)?;
            let part_el = ws.fragment(&format!(
                "<tablePart xmlns=\"{MAIN}\" xmlns:r=\"{}\" r:id=\"{rid}\"/>",
                super::R_NS
            ))?;
            let el = match ws.root_element("tableParts").cloned() {
                Some(mut e) => {
                    e.children.push(crate::xml::RawXml::Element(part_el));
                    let n = e.elements().count();
                    e.head.set_attr("count", &n.to_string());
                    e
                }
                None => {
                    let mut e =
                        ws.fragment(&format!("<tableParts xmlns=\"{MAIN}\" count=\"1\"/>"))?;
                    e.children.push(crate::xml::RawXml::Element(part_el));
                    e
                }
            };
            ws.put_root_element("tableParts", Some(el));
            if let Some(model) = Table::parse(&part_name, x.as_bytes()) {
                ws.push_table(model);
            }
        }
        self.sheets[sheet].dirty = true;

        // Totals-row cells.
        if spec.totals_row {
            self.set_cell_value(sheet, last_row, c1, CellInput::Text("Total".to_owned()))?;
            let code = if last_function == "sum" { 109 } else { 103 };
            let col_name = structured_column(&headers[width - 1]);
            let formula = format!("SUBTOTAL({code},{name}[{col_name}])");
            if width == 1 {
                // A single column carries the function, not the label.
                self.set_cell_formula(sheet, last_row, c1, &formula, None)?;
            } else {
                self.set_cell_formula(sheet, last_row, c2, &formula, None)?;
            }
            recalc.push((sheet, last_row, c2));
        }
        // Drop the attribute helper import warnings for unused types.
        recalc.sort_unstable();
        recalc.dedup();
        Ok(TableInfo {
            part: part_name,
            id,
            name,
            range: full,
            header_inserted,
            recalc,
        })
    }
}
