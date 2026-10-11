//! Read-only views over an open document. Hidden content (`w:vanish` runs, hidden sheets, rows
//! and columns) is left out unless the server runs with `--include-hidden`.

use open365_docx::{Document, Inline, Paragraph, RawXml, Run, RunPropItem};
use open365_xlsx::{CellValue, SheetKind, SheetState, Workbook, cell_ref_string, parse_range};
use serde_json::{Value, json};

/// Largest range `read_cells` serves in one call.
pub const MAX_CELLS: u64 = 10_000;
/// Most paragraphs one `read_text` call returns.
pub const MAX_PARAGRAPHS: usize = 1_000;
/// Budget for the text of one response.
pub const MAX_TEXT_BYTES: usize = 256 * 1024;

fn run_hidden(run: &Run) -> bool {
    run.props.items().iter().any(|item| match item {
        RunPropItem::Known(_, e) | RunPropItem::Raw(RawXml::Element(e)) => {
            e.head.is_w("vanish") && !matches!(e.head.attr_w("val"), Some("0" | "false" | "off"))
        }
        RunPropItem::Raw(_) => false,
    })
}

fn collect(content: &[Inline], include_hidden: bool, out: &mut String, omitted: &mut usize) {
    for inline in content {
        match inline {
            Inline::Run(r) => {
                if !include_hidden && run_hidden(r) {
                    *omitted += 1;
                } else {
                    out.push_str(&r.text());
                }
            }
            Inline::Group(g) => collect(&g.content, include_hidden, out, omitted),
            Inline::Raw(_) => {}
        }
    }
}

/// Paragraph text without hidden runs, and how many hidden runs were left out.
pub fn visible_text(para: &Paragraph, include_hidden: bool) -> (String, usize) {
    let mut out = String::new();
    let mut omitted = 0;
    collect(&para.content, include_hidden, &mut out, &mut omitted);
    (out, omitted)
}

fn heading_level(doc: &Document, para: &Paragraph) -> Option<u32> {
    let id = para.style_id()?;
    let name = doc
        .styles()
        .and_then(|s| s.style(id))
        .map_or_else(|| id.to_owned(), |s| s.name.clone())
        .to_ascii_lowercase();
    if name == "title" {
        return Some(0);
    }
    let rest = name.strip_prefix("heading")?.trim();
    rest.parse::<u32>().ok().filter(|n| (1..=9).contains(n))
}

pub fn docx_outline(doc: &Document, include_hidden: bool) -> Value {
    let mut items = Vec::new();
    for i in 0..doc.paragraph_count() {
        let Some(p) = doc.paragraph(i) else { continue };
        let Some(level) = heading_level(doc, p) else {
            continue;
        };
        let (text, _) = visible_text(p, include_hidden);
        if text.trim().is_empty() {
            continue;
        }
        items.push(json!({"paragraph": i, "level": level, "text": text}));
        if items.len() >= MAX_PARAGRAPHS {
            break;
        }
    }
    json!({"kind": "docx", "paragraph_count": doc.paragraph_count(), "headings": items})
}

/// Cuts `text` to the response budget at a character boundary.
fn clip(mut text: String) -> (String, bool) {
    if text.len() <= MAX_TEXT_BYTES {
        return (text, false);
    }
    let mut end = MAX_TEXT_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    (text, true)
}

pub fn docx_text(doc: &Document, include_hidden: bool, start: usize, count: usize) -> Value {
    let total = doc.paragraph_count();
    let count = count.clamp(1, MAX_PARAGRAPHS);
    let mut paragraphs = Vec::new();
    let mut bytes = 0;
    let mut next = start;
    while next < total && paragraphs.len() < count {
        let Some(p) = doc.paragraph(next) else { break };
        let (text, omitted) = visible_text(p, include_hidden);
        if bytes + text.len() > MAX_TEXT_BYTES && !paragraphs.is_empty() {
            break;
        }
        let (text, cut) = clip(text);
        bytes += text.len();
        let mut entry = json!({"paragraph": next, "text": text});
        if cut {
            entry["text_truncated"] = json!(true);
        }
        if omitted > 0 {
            // Command offsets count the hidden text too, so they would not match this text.
            entry["offsets_match_text"] = json!(false);
        }
        paragraphs.push(entry);
        next += 1;
    }
    json!({
        "paragraph_count": total,
        "paragraphs": paragraphs,
        "truncated": next < total,
        "next_paragraph": (next < total).then_some(next),
    })
}

fn sheet_visible(wb: &Workbook, index: usize, include_hidden: bool) -> bool {
    wb.sheets()
        .get(index)
        .is_some_and(|s| include_hidden || s.state == SheetState::Visible)
}

pub fn xlsx_outline(wb: &Workbook, include_hidden: bool) -> Value {
    let sheets: Vec<Value> = wb
        .sheets()
        .iter()
        .enumerate()
        .filter(|(i, _)| sheet_visible(wb, *i, include_hidden))
        .map(|(i, info)| {
            let used = wb
                .sheet(i)
                .and_then(|ws| ws.used_range())
                .map(|(r1, c1, r2, c2)| {
                    format!("{}:{}", cell_ref_string(r1, c1), cell_ref_string(r2, c2))
                });
            json!({
                "sheet": i,
                "name": info.name,
                "state": format!("{:?}", info.state).to_ascii_lowercase(),
                "worksheet": info.kind == SheetKind::Worksheet,
                "used_range": used,
            })
        })
        .collect();
    json!({"kind": "xlsx", "sheets": sheets})
}

fn value_json(v: &CellValue) -> Value {
    match v {
        CellValue::Empty => Value::Null,
        CellValue::Number(n) => json!(n),
        CellValue::Bool(b) => json!(b),
        CellValue::Error(e) => json!(e.code()),
        other => json!(other.as_text().unwrap_or_default()),
    }
}

/// `sheet` is an index or a name. Hidden sheets are refused unless `include_hidden`.
pub fn resolve_sheet(wb: &Workbook, sheet: &Value, include_hidden: bool) -> Result<usize, String> {
    let index = match sheet {
        Value::Number(n) => n.as_u64().and_then(|n| usize::try_from(n).ok()),
        Value::String(name) => wb.sheet_index(name),
        _ => None,
    };
    let index = index
        .filter(|i| *i < wb.sheets().len())
        .ok_or_else(|| "no such sheet".to_owned())?;
    // Same message as an unknown sheet, so a hidden sheet's existence is not confirmed.
    if !sheet_visible(wb, index, include_hidden) {
        return Err("no such sheet".to_owned());
    }
    Ok(index)
}

pub fn xlsx_cells(
    wb: &Workbook,
    sheet: usize,
    range: &str,
    include_hidden: bool,
) -> Result<Value, String> {
    let (r1, c1, r2, c2) = parse_range(range).ok_or("range must look like A1 or A1:D20")?;
    let area = u64::from(r2 - r1 + 1) * u64::from(c2 - c1 + 1);
    if area > MAX_CELLS {
        return Err(format!("range has {area} cells; the limit is {MAX_CELLS}"));
    }
    let ws = wb.sheet(sheet).ok_or("sheet has no cells")?;
    let hidden_rows: Vec<u32> = ws
        .rows()
        .iter()
        .filter(|r| r.hidden())
        .map(|r| r.r)
        .collect();
    let hidden_cols = ws.columns();
    let col_hidden = |c: u32| {
        hidden_cols
            .iter()
            .any(|s| s.hidden && s.min <= c && c <= s.max)
    };
    let mut cells = Vec::new();
    let mut bytes = 0usize;
    let mut truncated = false;
    'rows: for row in r1..=r2 {
        for col in c1..=c2 {
            let Some(cell) = ws.cell(row, col) else {
                continue;
            };
            if !include_hidden && (hidden_rows.contains(&row) || col_hidden(col)) {
                continue;
            }
            let mut entry = json!({
                "ref": cell_ref_string(row, col),
                "value": value_json(&cell.value),
            });
            if let Some(f) = wb.formula_text(sheet, row, col) {
                entry["formula"] = json!(f);
            }
            bytes += entry.to_string().len();
            if bytes > MAX_TEXT_BYTES {
                truncated = true;
                break 'rows;
            }
            cells.push(entry);
        }
    }
    Ok(json!({
        "range": range,
        "cells": cells,
        "truncated": truncated,
    }))
}

/// Which cells a formula may see: with `include_hidden` all of them, otherwise only those on
/// visible sheets outside hidden rows and columns. `keep(sheet, 0, 0)` asks about the sheet.
pub fn visibility_filter(wb: &Workbook, include_hidden: bool) -> impl Fn(usize, u32, u32) -> bool {
    struct Hidden {
        visible: bool,
        rows: std::collections::HashSet<u32>,
        cols: Vec<(u32, u32)>,
    }
    let sheets: Vec<Hidden> = (0..wb.sheets().len())
        .map(|i| {
            let ws = wb.sheet(i);
            Hidden {
                visible: sheet_visible(wb, i, include_hidden),
                rows: ws
                    .map(|w| {
                        w.rows()
                            .iter()
                            .filter(|r| r.hidden())
                            .map(|r| r.r)
                            .collect()
                    })
                    .unwrap_or_default(),
                cols: ws
                    .map(|w| {
                        w.columns()
                            .iter()
                            .filter(|c| c.hidden)
                            .map(|c| (c.min, c.max))
                            .collect()
                    })
                    .unwrap_or_default(),
            }
        })
        .collect();
    move |sheet, row, col| {
        let Some(h) = sheets.get(sheet) else {
            return false;
        };
        if !h.visible {
            return false;
        }
        include_hidden
            || !(h.rows.contains(&row) || h.cols.iter().any(|&(lo, hi)| lo <= col && col <= hi))
    }
}
