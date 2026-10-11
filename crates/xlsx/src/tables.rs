//! Read-only model of table parts (`xl/tables/tableN.xml`). The part stays untouched in
//! the package, so tables round-trip byte-identical (ADR-001).

use crate::XlsxError;
use crate::refs::parse_range;
use crate::sheet::Rect;
use crate::xml::{self, XmlElement};

/// One table of a worksheet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    /// Part name, for example `/xl/tables/table1.xml`.
    pub part: String,
    pub id: u32,
    pub name: String,
    pub display_name: String,
    /// The `ref` attribute as written (`A1:D10`), including header and totals rows.
    pub range_text: String,
    /// `range_text` parsed to `(row1, col1, row2, col2)`.
    pub rect: Option<Rect>,
    /// `headerRowCount`; 1 when absent.
    pub header_row_count: u32,
    /// `totalsRowCount`; 0 when absent.
    pub totals_row_count: u32,
    /// `tableType` when not the default worksheet table.
    pub table_type: Option<String>,
    pub comment: Option<String>,
    pub columns: Vec<TableColumn>,
    pub style: Option<TableStyle>,
}

/// One `tableColumn`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableColumn {
    pub id: u32,
    pub name: String,
    /// `totalsRowFunction` (`sum`, `average`, `custom`, ...).
    pub totals_row_function: Option<String>,
    pub totals_row_label: Option<String>,
    /// `totalsRowFormula` text (used with the `custom` function), without `=`.
    pub totals_row_formula: Option<String>,
    /// `calculatedColumnFormula` text, without `=`. May use structured references.
    pub calculated_column_formula: Option<String>,
    /// True when the calculated column formula is an array formula.
    pub calculated_column_array: bool,
}

/// `tableStyleInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableStyle {
    pub name: Option<String>,
    pub show_first_column: bool,
    pub show_last_column: bool,
    pub show_row_stripes: bool,
    pub show_column_stripes: bool,
}

impl Table {
    /// Number of data rows (header and totals excluded); `None` when `ref` is unparsable.
    pub fn data_row_count(&self) -> Option<u32> {
        let (r1, _, r2, _) = self.rect?;
        (r2 - r1 + 1).checked_sub(self.header_row_count + self.totals_row_count)
    }

    /// The data body range, without header and totals rows.
    pub fn data_rect(&self) -> Option<Rect> {
        let (r1, c1, r2, c2) = self.rect?;
        let top = r1 + self.header_row_count;
        let bottom = r2.checked_sub(self.totals_row_count)?;
        (top <= bottom).then_some((top, c1, bottom, c2))
    }

    /// The column whose name is `name` (case-insensitive) with its 1-based index.
    pub fn column(&self, name: &str) -> Option<(usize, &TableColumn)> {
        self.columns
            .iter()
            .enumerate()
            .find(|(_, c)| c.name.eq_ignore_ascii_case(name))
            .map(|(i, c)| (i + 1, c))
    }

    /// Parses a table part. Returns `None` when it is not a well-formed table.
    pub(crate) fn parse(part: &str, bytes: &[u8]) -> Option<Self> {
        let parsed = xml::parse(bytes).ok()?;
        let root = &parsed.root;
        if !root.head.is_main("table") {
            return None;
        }
        let num = |name: &str, default: u32| {
            root.head
                .attr(name)
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(default)
        };
        let name = root.head.attr("name").unwrap_or_default().to_owned();
        let display_name = root
            .head
            .attr("displayName")
            .map_or_else(|| name.clone(), str::to_owned);
        let range_text = root.head.attr("ref")?.to_owned();
        let columns = root
            .child_main("tableColumns")
            .map(|cols| {
                cols.elements()
                    .filter(|e| e.head.is_main("tableColumn"))
                    .map(parse_column)
                    .collect()
            })
            .unwrap_or_default();
        let style = root.child_main("tableStyleInfo").map(|e| {
            let flag = |n: &str| {
                e.head
                    .attr(n)
                    .is_some_and(|v| matches!(v.trim(), "1" | "true"))
            };
            TableStyle {
                name: e.head.attr("name").map(str::to_owned),
                show_first_column: flag("showFirstColumn"),
                show_last_column: flag("showLastColumn"),
                show_row_stripes: flag("showRowStripes"),
                show_column_stripes: flag("showColumnStripes"),
            }
        });
        Some(Self {
            part: part.to_owned(),
            id: num("id", 0),
            name,
            display_name,
            rect: parse_range(&range_text),
            range_text,
            header_row_count: num("headerRowCount", 1),
            totals_row_count: num("totalsRowCount", 0),
            table_type: root.head.attr("tableType").map(str::to_owned),
            comment: root.head.attr("comment").map(str::to_owned),
            columns,
            style,
        })
    }
}

fn parse_column(e: &XmlElement) -> TableColumn {
    let formula = |local: &str| e.child_main(local).map(XmlElement::own_text);
    TableColumn {
        id: e
            .head
            .attr("id")
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0),
        name: e.head.attr("name").unwrap_or_default().to_owned(),
        totals_row_function: e.head.attr("totalsRowFunction").map(str::to_owned),
        totals_row_label: e.head.attr("totalsRowLabel").map(str::to_owned),
        totals_row_formula: formula("totalsRowFormula"),
        calculated_column_formula: formula("calculatedColumnFormula"),
        calculated_column_array: e
            .child_main("calculatedColumnFormula")
            .and_then(|f| f.head.attr("array"))
            .is_some_and(|v| matches!(v.trim(), "1" | "true")),
    }
}

/// What a structural edit did to a table part.
#[derive(Debug)]
pub(crate) enum Restructured {
    /// The edit deleted every cell of the table.
    Removed,
    /// The part's new bytes, and the header cells of columns the edit added (`(row, col, name)`).
    Updated {
        bytes: Vec<u8>,
        new_headers: Vec<(u32, u32, String)>,
    },
}

fn set_element_text(el: &mut XmlElement, text: &str) {
    el.children = vec![crate::xml::RawXml::Text(text.to_owned())];
}

/// Rewrites a table part for an insert or delete of rows or columns on its sheet (`edit`, when that sheet is
/// the one edited) and for references into the edited sheet inside its formulas (`shifter`, with `home`
/// the sheet the table is on).
pub(crate) fn restructure(
    part: &str,
    bytes: &[u8],
    edit: Option<&crate::structure::StructEdit>,
    shifter: &dyn crate::structure::FormulaShifter,
    home: &str,
) -> Result<Restructured, XlsxError> {
    use crate::structure::{EditAxis, shift_range_text};
    use crate::xml::{Attr, RawXml};

    let mut parsed = xml::parse(bytes)?;
    let Some(table) = Table::parse(part, bytes) else {
        return Ok(Restructured::Updated {
            bytes: bytes.to_vec(),
            new_headers: Vec::new(),
        });
    };
    let mut new_headers = Vec::new();
    if let (Some(edit), Some(rect)) = (edit, table.rect) {
        let Some(mut new_rect) = edit.map_rect(rect) else {
            return Ok(Restructured::Removed);
        };
        let (r1, c1, r2, c2) = rect;
        if edit.axis == EditAxis::Rows && edit.delete {
            // A table keeps one data row, whatever is deleted.
            let min_rows = table.header_row_count + table.totals_row_count + 1;
            if new_rect.2 + 1 < new_rect.0 + min_rows {
                new_rect.2 = new_rect.0 + min_rows - 1;
            }
        }
        let columns = parsed.root.children.iter_mut().find_map(|c| match c {
            RawXml::Element(e) if e.head.is_main("tableColumns") => Some(e),
            _ => None,
        });
        if let Some(columns) = columns {
            let (at, end) = edit.span();
            if edit.axis == EditAxis::Cols && edit.delete {
                let mut index = 0u32;
                columns.children.retain(|child| {
                    let Some(e) = child.as_element() else {
                        return true;
                    };
                    if !e.head.is_main("tableColumn") {
                        return true;
                    }
                    let col = c1 + index;
                    index += 1;
                    !(at..=end).contains(&col)
                });
            } else if edit.axis == EditAxis::Cols && c1 < at && at <= c2 {
                let mut taken: Vec<String> = columns
                    .elements()
                    .filter_map(|e| e.head.attr("name").map(str::to_lowercase))
                    .collect();
                let first_id = columns
                    .elements()
                    .filter_map(|e| e.head.attr("id").and_then(|v| v.parse::<u32>().ok()))
                    .max()
                    .unwrap_or(0)
                    + 1;
                let position = columns
                    .children
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| {
                        c.as_element()
                            .is_some_and(|e| e.head.is_main("tableColumn"))
                    })
                    .map(|(i, _)| i)
                    .nth((at - c1) as usize)
                    .unwrap_or(columns.children.len());
                // Excel names a new column after the one on its left: `Col1` gives `Col12`, `Col13`.
                let left = columns
                    .elements()
                    .filter(|e| e.head.is_main("tableColumn"))
                    .nth((at - c1 - 1) as usize)
                    .and_then(|e| e.head.attr("name"))
                    .unwrap_or("Column")
                    .to_owned();
                let mut n = 2u32;
                for k in 0..edit.count {
                    let name = loop {
                        let candidate = format!("{left}{n}");
                        n += 1;
                        if !taken.contains(&candidate.to_lowercase()) {
                            break candidate;
                        }
                    };
                    taken.push(name.to_lowercase());
                    let mut el = columns.sibling("tableColumn");
                    el.head.attrs = vec![
                        Attr::plain("id", &(first_id + k).to_string()),
                        Attr::plain("name", &name),
                    ];
                    columns
                        .children
                        .insert(position + k as usize, RawXml::Element(el));
                    if table.header_row_count > 0 {
                        new_headers.push((r1, at + k, name));
                    }
                }
            }
            let n = columns
                .elements()
                .filter(|e| e.head.is_main("tableColumn"))
                .count();
            columns.head.set_attr("count", &n.to_string());
        }
        let _ = r2;
        let text = crate::structure::range_a1(new_rect);
        parsed.root.head.set_attr("ref", &text);
        for child in &mut parsed.root.children {
            if let RawXml::Element(e) = child
                && (e.head.is_main("autoFilter") || e.head.is_main("sortState"))
                && let Some(new) = e.head.attr("ref").and_then(|r| shift_range_text(r, edit))
            {
                e.head.set_attr("ref", &new);
                for s in &mut e.children {
                    if let RawXml::Element(sc) = s
                        && sc.head.is_main("sortCondition")
                        && let Some(new) =
                            sc.head.attr("ref").and_then(|r| shift_range_text(r, edit))
                    {
                        sc.head.set_attr("ref", &new);
                    }
                }
            }
        }
    }
    // Formulas of calculated columns and totals rows.
    fn formulas(el: &mut XmlElement, shifter: &dyn crate::structure::FormulaShifter, home: &str) {
        let local = el.head.local_name();
        if local == "calculatedColumnFormula" || local == "totalsRowFormula" {
            let text = el.own_text();
            if let Some(new) = shifter.shift(text.trim_start_matches('='), home) {
                set_element_text(el, &new);
            }
            return;
        }
        for child in &mut el.children {
            if let RawXml::Element(c) = child {
                formulas(c, shifter, home);
            }
        }
    }
    formulas(&mut parsed.root, shifter, home);
    Ok(Restructured::Updated {
        bytes: parsed.to_bytes(),
        new_headers,
    })
}
