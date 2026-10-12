//! Insert tab on the sheet model: tables, PivotTables, charts, links, comments and symbols.
//! Every edit is a `SheetObjects` command (one undo step); the ribbon module
//! `ribbon::sheets::insert` maps control ids to these methods.
//!
//! Defaults follow Excel: a lone cell means its current region, a table guesses its headers, a
//! chart reads the headers and the category column from the cells and puts series in the longer
//! direction, a single-series chart is titled with the series name and has no legend.

use std::fmt::Write as _;

use open365_chart::{BuildKind, BuildSeries, ChartBuilder, LegendPos};
use open365_commands::objects_ops::{HyperlinkRequest, LinkKind, TableRequest};
use open365_commands::{Command, ObjectsOp, RangeRef};
use open365_xlsx::CellValue;

use super::super::{MAX_COL, MAX_ROW, Rect4, SheetModel};
use crate::dialogs::{insert_chart, insert_link};

/// What a cell holds, for the header and category guesses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Empty,
    Number,
    Text,
}

/// Excel's message for a data range without labelled columns.
pub const PIVOT_FIELD_NAME: &str = "The PivotTable field name is not valid. To create a PivotTable report, you must use data that is organized as a list with labeled columns. If you are changing the name of a PivotTable field, you must type a new name for the field.";

/// `$A$1` for a cell (1-based).
#[must_use]
pub fn a1_abs(row: u32, col: u32) -> String {
    format!("${}${row}", col_name(col))
}

/// `A`, `B` ... `AA` for a 1-based column.
#[must_use]
pub fn col_name(col: u32) -> String {
    let mut n = col;
    let mut s = String::new();
    while n > 0 {
        let r = (n - 1) % 26;
        s.insert(0, char::from(b'A' + r as u8));
        n = (n - 1) / 26;
    }
    s
}

fn col_index(name: &str) -> Option<u32> {
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_uppercase()) {
        return None;
    }
    Some(
        name.bytes()
            .fold(0u32, |a, b| a * 26 + u32::from(b - b'A' + 1)),
    )
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn quote_sheet(name: &str) -> String {
    let plain = !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name.chars().all(|c| c.is_alphanumeric() || c == '_');
    if plain {
        name.to_owned()
    } else {
        format!("'{}'", name.replace('\'', "''"))
    }
}

fn range_ref(r: Rect4) -> RangeRef {
    RangeRef {
        first_row: r.0,
        first_col: r.1,
        last_row: r.2,
        last_col: r.3,
    }
}

/// Maps a reference of the chart crate's fixed layout (categories in `A`, series from `B`, names in
/// row 1) to the cells the chart was built from.
fn remap(
    xml: &str,
    plan: &insert_chart::ChartPlan,
    origin: (u32, u32),
    names: Option<&[String]>,
) -> String {
    let cell_of = |k: usize, line: usize| -> (u32, u32) {
        let (k, line) = (k as u32, line as u32);
        if plan.transposed {
            (origin.0 + line, origin.1 + k)
        } else {
            (origin.0 + k, origin.1 + line)
        }
    };
    let map_point = |p: &str| -> Option<String> {
        let rest = p.strip_prefix('$')?;
        let (letters, row) = rest.split_once('$')?;
        let (bcol, brow) = (col_index(letters)? as usize, row.parse::<usize>().ok()?);
        let line = match bcol {
            1 => plan.cat_line?,
            n => *plan.series_lines.get(n - 2)?,
        };
        let k = if brow <= 1 {
            plan.header_k.unwrap_or(0)
        } else {
            plan.data_k0 + brow - 2
        };
        let (r, c) = cell_of(k, line);
        Some(a1_abs(r, c))
    };
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(start) = rest.find("<c:f>") {
        let (before, tail) = rest.split_at(start + 5);
        out.push_str(before);
        let Some(end) = tail.find("</c:f>") else {
            rest = tail;
            break;
        };
        let formula = &tail[..end];
        let mapped = formula.rsplit_once('!').and_then(|(sheet, cells)| {
            let parts: Option<Vec<String>> = cells.split(':').map(map_point).collect();
            Some(format!("{sheet}!{}", parts?.join(":")))
        });
        out.push_str(mapped.as_deref().unwrap_or(formula));
        rest = &tail[end..];
    }
    out.push_str(rest);
    let Some(names) = names else {
        return out;
    };
    let mut series = 0usize;
    // No header row: series are named "Series1" ... as literals, as Excel does.
    let mut result = String::with_capacity(out.len());
    let mut rest = out.as_str();
    while let Some(start) = rest.find("<c:tx><c:strRef>") {
        let Some(len) = rest[start..].find("</c:strRef></c:tx>") else {
            break;
        };
        result.push_str(&rest[..start]);
        let name = names.get(series).map_or("", String::as_str);
        let _ = write!(result, "<c:tx><c:v>{}</c:v></c:tx>", xml_escape(name));
        series += 1;
        rest = &rest[start + len + "</c:strRef></c:tx>".len()..];
    }
    result.push_str(rest);
    result
}

impl SheetModel {
    // ---- selection ---------------------------------------------------------------------

    /// What Insert works on: the selection, or the current region of a lone cell.
    #[must_use]
    pub fn insert_data_range(&self) -> Rect4 {
        if self.is_range() {
            self.range()
        } else {
            self.home_current_region(self.active)
        }
    }

    fn kind_at(&self, row: u32, col: u32) -> Kind {
        match self
            .target
            .xlsx()
            .and_then(|wb| wb.cell(self.sheet, row, col))
            .map(|c| &c.value)
        {
            None | Some(CellValue::Empty) => Kind::Empty,
            Some(CellValue::Number(_)) => Kind::Number,
            Some(_) => Kind::Text,
        }
    }

    /// The kinds of the cells of `r`, row by row.
    #[must_use]
    pub fn insert_kinds(&self, r: Rect4) -> Vec<Vec<Kind>> {
        (r.0..=r.2)
            .map(|row| (r.1..=r.3).map(|col| self.kind_at(row, col)).collect())
            .collect()
    }

    fn number_at(&self, row: u32, col: u32) -> f64 {
        match self
            .target
            .xlsx()
            .and_then(|wb| wb.cell(self.sheet, row, col))
            .map(|c| &c.value)
        {
            Some(CellValue::Number(n)) => *n,
            _ => 0.0,
        }
    }

    fn text_at(&self, row: u32, col: u32) -> String {
        self.target
            .xlsx()
            .map(|wb| open365_commands::cell_display_text(wb, self.sheet, row, col))
            .unwrap_or_default()
    }

    fn sheet_name(&self) -> String {
        self.target
            .xlsx()
            .and_then(|wb| wb.sheets().get(self.sheet))
            .map(|s| s.name.clone())
            .unwrap_or_default()
    }

    fn objects(&mut self, sheet: usize, op: ObjectsOp) -> Result<(), String> {
        self.apply(Command::SheetObjects { sheet, op })
    }

    // ---- tables ------------------------------------------------------------------------

    /// Insert > Table: "My table has headers" is `headers`, Excel's guess when `None`.
    pub fn insert_table(
        &mut self,
        headers: Option<bool>,
        name: Option<String>,
    ) -> Result<(), String> {
        let r = self.insert_data_range();
        self.objects(
            self.sheet,
            ObjectsOp::CreateTable(TableRequest {
                range: range_ref(r),
                name,
                has_headers: headers,
                style: None,
                totals_row: false,
                filter_buttons: true,
                first_column: false,
                last_column: false,
                banded_rows: true,
                banded_columns: false,
            }),
        )?;
        self.anchor = (r.0, r.1);
        self.focus = (r.2, r.3);
        Ok(())
    }

    // ---- pivot tables ------------------------------------------------------------------

    /// Insert > PivotTable > From Table/Range, to a new sheet: an empty PivotTable over `source`
    /// (the selection or its current region) that refreshes on load. The new sheet becomes the
    /// active one with A3 selected, as in Excel.
    pub fn insert_pivot_table(&mut self, source: Option<Rect4>) -> Result<(), String> {
        let (r1, c1, r2, c2) = source.unwrap_or_else(|| self.insert_data_range());
        if r2 <= r1 || c2 < c1 {
            return Err(
                "The PivotTable report needs a header row and at least one row of data.".to_owned(),
            );
        }
        let names: Vec<String> = (c1..=c2).map(|c| self.text_at(r1, c)).collect();
        if names.iter().any(|n| n.trim().is_empty()) {
            return Err(PIVOT_FIELD_NAME.to_owned());
        }
        let source_sheet = self.sheet_name();
        let reference = format!("{}{}:{}{}", col_name(c1), r1, col_name(c2), r2);
        let n = u32::try_from(names.len()).unwrap_or(0);
        let mut number = 1;
        while self.target.xlsx().is_some_and(|wb| {
            wb.package()
                .part(&format!("xl/pivotTables/pivotTable{number}.xml"))
                .is_some()
        }) {
            number += 1;
        }
        let ns = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
        let mut cache = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<pivotCacheDefinition xmlns=\"{ns}\" refreshOnLoad=\"1\" refreshedBy=\"Open365\" refreshedDate=\"0\" createdVersion=\"6\" refreshedVersion=\"6\" minRefreshableVersion=\"3\" recordCount=\"{}\" saveData=\"0\"><cacheSource type=\"worksheet\"><worksheetSource ref=\"{reference}\" sheet=\"{}\"/></cacheSource><cacheFields count=\"{n}\">",
            r2 - r1,
            xml_escape(&source_sheet)
        );
        for name in &names {
            let _ = write!(
                cache,
                "<cacheField name=\"{}\" numFmtId=\"0\"><sharedItems/></cacheField>",
                xml_escape(name)
            );
        }
        cache.push_str("</cacheFields></pivotCacheDefinition>");
        let mut table = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<pivotTableDefinition xmlns=\"{ns}\" name=\"PivotTable{number}\" cacheId=\"0\" applyNumberFormats=\"0\" applyBorderFormats=\"0\" applyFontFormats=\"0\" applyPatternFormats=\"0\" applyAlignmentFormats=\"0\" applyWidthHeightFormats=\"1\" dataCaption=\"Values\" updatedVersion=\"6\" minRefreshableVersion=\"3\" useAutoFormatting=\"1\" itemPrintTitles=\"1\" createdVersion=\"6\" indent=\"0\" outline=\"1\" outlineData=\"1\" multipleFieldFilters=\"0\"><location ref=\"A3:C20\" firstHeaderRow=\"1\" firstDataRow=\"1\" firstDataCol=\"1\"/><pivotFields count=\"{n}\">"
        );
        for _ in &names {
            table.push_str("<pivotField showAll=\"0\"/>");
        }
        table.push_str("</pivotFields><pivotTableStyleInfo name=\"PivotStyleLight16\" showRowHeaders=\"1\" showColHeaders=\"1\" showRowStripes=\"0\" showColStripes=\"0\" showLastColumn=\"1\"/></pivotTableDefinition>");

        let before = self.sheet;
        self.objects(
            before,
            ObjectsOp::AddSheet {
                at: None,
                name: None,
            },
        )?;
        let new = self
            .target
            .xlsx()
            .map_or(before, |wb| wb.sheets().len().saturating_sub(1));
        let added = self.objects(
            new,
            ObjectsOp::AddPivotTable {
                cache_definition: cache.into_bytes(),
                cache_records: None,
                table_definition: table.into_bytes(),
            },
        );
        if let Err(e) = added {
            self.undo();
            self.sheet = before;
            self.status = e.clone();
            return Err(e);
        }
        self.sheet = new;
        self.select_cell((3, 1));
        Ok(())
    }

    // ---- charts ------------------------------------------------------------------------

    /// Insert > Charts: a chart of the selection (or its current region) with Excel's defaults,
    /// placed to the right of the data.
    pub fn insert_chart(&mut self, kind: BuildKind) -> Result<(), String> {
        let r = self.insert_data_range();
        let kinds = self.insert_kinds(r);
        let scatter = kind == BuildKind::Scatter;
        let Some(plan) = insert_chart::plan(&kinds, scatter) else {
            return Err("Select cells that contain numbers to chart.".to_owned());
        };
        let origin = (r.0, r.1);
        let cell_of = |k: usize, line: usize| -> (u32, u32) {
            let (k, line) = (k as u32, line as u32);
            if plan.transposed {
                (origin.0 + line, origin.1 + k)
            } else {
                (origin.0 + k, origin.1 + line)
            }
        };
        let mut b = ChartBuilder::new(kind);
        b.sheet = quote_sheet(&self.sheet_name());
        let positions = plan.data_k0..plan.data_k0 + plan.count;
        let mut xs = Vec::new();
        if let Some(line) = plan.cat_line {
            for k in positions.clone() {
                let (row, col) = cell_of(k, line);
                if scatter {
                    xs.push(self.number_at(row, col));
                } else {
                    b.categories.push(self.text_at(row, col));
                }
            }
        }
        let mut lines = plan.series_lines.clone();
        if matches!(kind, BuildKind::Pie | BuildKind::Doughnut) {
            lines.truncate(1);
        }
        let mut fallback = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            let name = match plan.header_k {
                Some(k) => {
                    let (row, col) = cell_of(k, *line);
                    self.text_at(row, col)
                }
                None => format!("Series{}", i + 1),
            };
            fallback.push(format!("Series{}", i + 1));
            let values = positions
                .clone()
                .map(|k| {
                    let (row, col) = cell_of(k, *line);
                    self.number_at(row, col)
                })
                .collect();
            b.series.push(BuildSeries {
                name,
                values,
                xs: xs.clone(),
                sizes: Vec::new(),
            });
        }
        let single = b.series.len() == 1;
        b.title = Some(if single && !matches!(kind, BuildKind::Scatter) {
            b.series[0].name.clone()
        } else {
            "Chart Title".to_owned()
        });
        b.legend = if single && !matches!(kind, BuildKind::Pie | BuildKind::Doughnut) {
            None
        } else {
            Some(LegendPos::Bottom)
        };
        let xml = remap(
            &b.to_xml(),
            &plan,
            origin,
            plan.header_k.is_none().then_some(fallback.as_slice()),
        );
        let first_col = (r.3 + 2).min(MAX_COL - 8);
        let first_row = r.0.min(MAX_ROW - 15);
        self.objects(
            self.sheet,
            ObjectsOp::InsertChart {
                chart_xml: xml,
                range: RangeRef {
                    first_row,
                    first_col,
                    last_row: first_row + 14,
                    last_col: first_col + 7,
                },
                name: None,
            },
        )
    }

    // ---- links, comments, symbols ------------------------------------------------------

    /// Insert > Link (Ctrl+K) on the selection. `text` replaces the cell text; an empty cell shows
    /// the address when no text is given.
    pub fn insert_link(
        &mut self,
        target: LinkKind,
        text: Option<String>,
        screen_tip: Option<String>,
    ) -> Result<(), String> {
        let (row, col) = self.active;
        let text = text.or_else(|| {
            (self.kind_at(row, col) == Kind::Empty).then(|| insert_link::default_text(&target))
        });
        let range = range_ref(self.range());
        self.objects(
            self.sheet,
            ObjectsOp::SetHyperlink(HyperlinkRequest {
                range,
                target,
                text,
                screen_tip,
            }),
        )
    }

    /// Remove Hyperlink on the active cell.
    pub fn insert_remove_link(&mut self) -> Result<(), String> {
        let (row, col) = self.active;
        self.objects(self.sheet, ObjectsOp::RemoveHyperlink { row, col })
    }

    /// Insert > Symbol: the symbol goes at the end of the active cell's text, as when the cell is
    /// being edited with the caret at the end.
    pub fn insert_symbol(&mut self, symbol: &str) -> Result<(), String> {
        let (row, col) = self.active;
        let text = format!("{}{symbol}", self.source(row, col));
        self.review_enter(row, col, &text)
    }
}
