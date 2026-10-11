//! More read-only views: styles, formulas, recalculation. Same rules as `views`: hidden
//! sheets, rows and columns are left out unless `include_hidden`, and every list is capped.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use open365_calc::Value as Calc;
use open365_commands::{CommandError, FormulaEngine};
use open365_docx::{Document, StyleKind};
use open365_xlsx::{CellValue, Rgb, Workbook, cell_ref_string, parse_range};
use serde_json::{Value, json};

use crate::views::{MAX_CELLS, MAX_TEXT_BYTES, visibility_filter};

/// Most styles listed.
pub const MAX_STYLES: usize = 2_000;
/// Most formula cells listed in one call.
pub const MAX_FORMULAS: usize = 5_000;
/// Most cells that `get_styles` describes for a range.
pub const MAX_STYLE_CELLS: u64 = 1_000;
/// Most differences `recalc` lists.
pub const MAX_DIFFS: usize = 200;

fn kind_name(kind: StyleKind) -> &'static str {
    match kind {
        StyleKind::Paragraph => "paragraph",
        StyleKind::Character => "character",
        StyleKind::Table => "table",
        StyleKind::Numbering => "numbering",
    }
}

/// DOCX: the styles of the style sheet with their relations.
pub fn docx_styles(doc: &Document) -> Value {
    let Some(styles) = doc.styles() else {
        return json!({"kind": "docx", "styles": [], "count": 0, "truncated": false});
    };
    let all: Vec<_> = styles.styles().collect();
    let list: Vec<Value> = all
        .iter()
        .take(MAX_STYLES)
        .map(|s| {
            json!({
                "id": s.id,
                "name": s.name,
                "type": kind_name(s.kind),
                "based_on": s.based_on,
                "next": s.next,
                "default": s.is_default,
            })
        })
        .collect();
    json!({
        "kind": "docx",
        "default_paragraph_style": styles.default_paragraph_style(),
        "count": all.len(),
        "styles": list,
        "truncated": all.len() > MAX_STYLES,
    })
}

fn hex(rgb: Rgb) -> String {
    format!("{:02X}{:02X}{:02X}", rgb.r, rgb.g, rgb.b)
}

fn lower_debug(v: &impl std::fmt::Debug) -> String {
    format!("{v:?}").to_ascii_lowercase()
}

/// XLSX: the workbook's style tables and, when `range` is given, the formatting of its cells.
pub fn xlsx_styles(
    wb: &Workbook,
    sheet: usize,
    range: Option<&str>,
    include_hidden: bool,
) -> Result<Value, String> {
    let styles = wb.styles();
    let mut custom: Vec<(u32, String)> = styles
        .custom_formats()
        .iter()
        .map(|(id, code)| (*id, code.clone()))
        .collect();
    custom.sort();
    let mut out = json!({
        "kind": "xlsx",
        "cell_styles": styles.cell_style_names(),
        "cell_format_count": styles.cell_xf_count(),
        "default_font": styles.default_font().map(|(name, size)| json!({"name": name, "size": size})),
        "number_formats": custom.iter().map(|(id, code)| json!({"id": id, "code": code})).collect::<Vec<_>>(),
    });
    let Some(range) = range else {
        return Ok(out);
    };
    let (r1, c1, r2, c2) = parse_range(range).ok_or("range must look like A1 or A1:D20")?;
    let area = u64::from(r2 - r1 + 1) * u64::from(c2 - c1 + 1);
    if area > MAX_STYLE_CELLS {
        return Err(format!(
            "range has {area} cells; the limit is {MAX_STYLE_CELLS}"
        ));
    }
    let keep = visibility_filter(wb, include_hidden);
    let mut cells = Vec::new();
    for row in r1..=r2 {
        for col in c1..=c2 {
            if !keep(sheet, row, col) {
                continue;
            }
            let xf = wb.cell_xf(sheet, row, col);
            let f = wb.cell_format(sheet, row, col);
            cells.push(json!({
                "ref": cell_ref_string(row, col),
                "style_name": styles.cell_style_name(xf),
                "number_format": wb.number_format(sheet, row, col),
                "font": {
                    "name": f.font.name,
                    "size": f.font.size,
                    "bold": f.font.bold,
                    "italic": f.font.italic,
                    "underline": lower_debug(&f.font.underline),
                    "strike": f.font.strike,
                    "color": hex(styles.resolve_color(&f.font.color)),
                },
                "fill": {
                    "pattern": lower_debug(&f.fill.pattern),
                    "color": hex(styles.resolve_color(&f.fill.fg)),
                },
                "alignment": {
                    "horizontal": lower_debug(&f.alignment.horizontal),
                    "vertical": lower_debug(&f.alignment.vertical),
                    "wrap_text": f.alignment.wrap_text,
                    "indent": f.alignment.indent,
                },
            }));
        }
    }
    out["range"] = json!(range);
    out["cells"] = Value::Array(cells);
    Ok(out)
}

fn cell_json(v: &CellValue) -> Value {
    match v {
        CellValue::Empty => Value::Null,
        CellValue::Number(n) => json!(n),
        CellValue::Bool(b) => json!(b),
        CellValue::Error(e) => json!(e.code()),
        other => json!(other.as_text().unwrap_or_default()),
    }
}

/// XLSX: the formula cells of a range (the used range when none is given) with their formula
/// text and stored value.
pub fn xlsx_formulas(
    wb: &Workbook,
    sheet: usize,
    range: Option<&str>,
    include_hidden: bool,
) -> Result<Value, String> {
    let ws = wb.sheet(sheet).ok_or("sheet has no cells")?;
    let (r1, c1, r2, c2) = match range {
        Some(r) => {
            let b = parse_range(r).ok_or("range must look like A1 or A1:D20")?;
            let area = u64::from(b.2 - b.0 + 1) * u64::from(b.3 - b.1 + 1);
            if area > MAX_CELLS * 100 {
                return Err(format!("range has {area} cells; narrow it"));
            }
            b
        }
        None => ws.used_range().unwrap_or((1, 1, 1, 1)),
    };
    let keep = visibility_filter(wb, include_hidden);
    let mut formulas = Vec::new();
    let mut bytes = 0usize;
    let mut truncated = false;
    'rows: for row in ws.rows() {
        if row.r < r1 || row.r > r2 {
            continue;
        }
        for cell in &row.cells {
            if cell.col < c1 || cell.col > c2 || cell.formula.is_none() {
                continue;
            }
            if !keep(sheet, row.r, cell.col) {
                continue;
            }
            let entry = json!({
                "ref": cell_ref_string(row.r, cell.col),
                "formula": wb.formula_text(sheet, row.r, cell.col),
                "value": cell_json(&cell.value),
            });
            bytes += entry.to_string().len();
            if formulas.len() >= MAX_FORMULAS || bytes > MAX_TEXT_BYTES {
                truncated = true;
                break 'rows;
            }
            formulas.push(entry);
        }
    }
    Ok(json!({
        "range": format!("{}:{}", cell_ref_string(r1, c1), cell_ref_string(r2, c2)),
        "count": formulas.len(),
        "formulas": formulas,
        "truncated": truncated,
    }))
}

fn same(fresh: &Calc, cached: &CellValue) -> bool {
    match (fresh, cached) {
        (Calc::Empty, CellValue::Empty) => true,
        (Calc::Number(a), CellValue::Number(b)) => (a - b).abs() <= 1e-9 * a.abs().max(1.0),
        (Calc::Bool(a), CellValue::Bool(b)) => a == b,
        (Calc::Error(a), CellValue::Error(b)) => a.to_string() == b.code(),
        (Calc::Text(a), other) => other.as_text() == Some(a.as_str()),
        // A formula that returns an array shows its top-left element, which the engine reports.
        _ => false,
    }
}

fn calc_json(v: &Calc) -> Value {
    match v {
        Calc::Empty => Value::Null,
        Calc::Number(n) => json!(n),
        Calc::Bool(b) => json!(b),
        Calc::Text(t) => json!(t),
        Calc::Error(e) => json!(e.to_string()),
        Calc::Array(_) => json!("(array)"),
    }
}

/// XLSX: recalculates every formula in the range from scratch in a scratch engine and compares
/// the results with the values stored in the file. Nothing is changed or saved.
///
/// # Errors
/// [`CommandError::Cancelled`] when `cancel` is set; [`CommandError::Formula`] for a bad range.
pub fn xlsx_recalc(
    wb: &Workbook,
    sheet: usize,
    range: Option<&str>,
    include_hidden: bool,
    cancel: &Arc<AtomicBool>,
) -> Result<Value, CommandError> {
    let ws = wb
        .sheet(sheet)
        .ok_or_else(|| CommandError::Formula("sheet has no cells".to_owned()))?;
    let name = wb
        .sheets()
        .get(sheet)
        .map(|s| s.name.clone())
        .unwrap_or_default();
    let (r1, c1, r2, c2) = match range {
        Some(r) => parse_range(r)
            .ok_or_else(|| CommandError::Formula("range must look like A1 or A1:D20".to_owned()))?,
        None => ws.used_range().unwrap_or((1, 1, 1, 1)),
    };
    let keep = visibility_filter(wb, include_hidden);
    let engine = FormulaEngine::build_cancellable(wb, &keep, cancel)?;
    let mut checked = 0usize;
    let mut stale = 0usize;
    let mut diffs = Vec::new();
    for row in ws.rows() {
        if row.r < r1 || row.r > r2 {
            continue;
        }
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(CommandError::Cancelled);
        }
        for cell in &row.cells {
            if cell.col < c1 || cell.col > c2 || cell.formula.is_none() {
                continue;
            }
            if !keep(sheet, row.r, cell.col) {
                continue;
            }
            checked += 1;
            let fresh = engine.value(&name, row.r, cell.col);
            if !same(&fresh, &cell.value) {
                stale += 1;
                if diffs.len() < MAX_DIFFS {
                    diffs.push(json!({
                        "ref": cell_ref_string(row.r, cell.col),
                        "formula": wb.formula_text(sheet, row.r, cell.col),
                        "stored": cell_json(&cell.value),
                        "recalculated": calc_json(&fresh),
                    }));
                }
            }
        }
    }
    Ok(json!({
        "sheet": sheet,
        "range": format!("{}:{}", cell_ref_string(r1, c1), cell_ref_string(r2, c2)),
        "formulas_checked": checked,
        "stale": stale,
        "differences": diffs,
        "truncated": stale > diffs.len(),
        "saved": false,
    }))
}
