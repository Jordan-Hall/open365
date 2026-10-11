//! Table editing against Microsoft Word.
//!
//! `tools/office/table-edit-oracle.ps1` applies each scenario in Word and writes the table
//! properties of the saved file to `fixtures/reference/docx/tbledit-<scenario>/table.tsv`.
//! Here the same edit runs on the same fixture through the model and the two property dumps are
//! compared field by field (grid, `w:tblPr`, `w:trPr`, `w:tcPr`, paragraph properties, text),
//! property by property inside the property elements. A scenario either matches Word exactly or
//! lists the categories where it is known to differ, with the reason, in `known_divergence`.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use common::repo_root;
use open365_docx::{
    AutoFitMode, Border, BorderSet, BorderSide, ColumnAdjust, ColumnSide, DocPos, Document,
    DocxError, HeightRule, RevisionKind, RowSide, Shading, TableJustification, TableLayoutType,
    TableWidth, TextSeparator, TrackChanges,
};
use open365_opc::Limits;

fn base_path(base: &str) -> PathBuf {
    repo_root()
        .join("fixtures")
        .join("docx")
        .join(format!("{base}.docx"))
}

fn open_base(base: &str) -> Document {
    let bytes = fs::read(base_path(base)).unwrap_or_else(|e| panic!("{base}: {e}"));
    Document::open(&bytes, &Limits::default()).expect("base fixture opens")
}

fn track() -> TrackChanges {
    TrackChanges::with_date("Table Tester", "2026-01-02T03:04:00Z")
}

const RED: [u8; 3] = [255, 0, 0];

fn border(style: &str, width: f64, color: Option<[u8; 3]>) -> Border {
    Border {
        style: style.to_owned(),
        width,
        space: 0.0,
        color,
    }
}

fn clear_shading(fill: [u8; 3]) -> Shading {
    Shading {
        pattern: "clear".to_owned(),
        fill: Some(fill),
        color: None,
    }
}

type EditFn = Box<dyn Fn(&mut Document) -> Result<(), DocxError>>;

fn insert(
    rows: usize,
    cols: usize,
    at: DocPos,
    mode: AutoFitMode,
) -> impl Fn(&mut Document) -> Result<(), DocxError> {
    move |d| {
        let width = d.text_width_pt(at.para);
        d.insert_table(at, rows, cols, width, mode).map(|_| ())
    }
}

/// The edit that reproduces the Word call of scenario `name`; `None` when the scenario is a
/// Word behaviour this model does not offer (see `skipped`).
#[allow(clippy::too_many_lines)]
fn scenario(name: &str) -> Option<EditFn> {
    let fixed = AutoFitMode::Fixed;
    let p = DocPos::new(1, 0);
    let edit: EditFn = match name {
        "insert-3x3" => Box::new(insert(3, 3, p, fixed)),
        "insert-2x5" => Box::new(insert(2, 5, p, fixed)),
        "insert-1x1" => Box::new(insert(1, 1, p, fixed)),
        "insert-3x3-content" => Box::new(insert(3, 3, p, AutoFitMode::Content)),
        "insert-3x3-window" => Box::new(insert(3, 3, p, AutoFitMode::Window)),
        "insert-3x3-a4" => Box::new(insert(3, 3, p, fixed)),
        "insert-3x3-odd" => Box::new(insert(3, 3, p, fixed)),
        "insert-7x2-odd" => Box::new(insert(2, 7, p, fixed)),
        "insert-4x4-a4" => Box::new(insert(4, 4, p, fixed)),
        "insert-at-start" => Box::new(insert(2, 2, DocPos::new(1, 0), fixed)),
        "insert-at-middle" => Box::new(insert(2, 2, DocPos::new(1, 5), fixed)),
        "insert-at-end" => Box::new(insert(2, 2, DocPos::new(1, 11), fixed)),
        "insert-3x3-tracked" => Box::new(|d| {
            d.set_track_changes(Some(track()));
            insert(3, 3, DocPos::new(1, 0), AutoFitMode::Fixed)(d)
        }),
        "row-above-first" => Box::new(|d| d.insert_rows(0, 0, RowSide::Above, 1).map(|_| ())),
        "row-above-mid" => Box::new(|d| d.insert_rows(0, 1, RowSide::Above, 1).map(|_| ())),
        "row-below-mid" => Box::new(|d| d.insert_rows(0, 1, RowSide::Below, 1).map(|_| ())),
        "row-below-last" => Box::new(|d| d.insert_rows(0, 2, RowSide::Below, 1).map(|_| ())),
        "row-above-two" => Box::new(|d| d.insert_rows(0, 1, RowSide::Above, 2).map(|_| ())),
        // Word's selection of a vertically merged cell covers all its rows.
        "row-below-merged" => Box::new(|d| d.insert_rows(0, 2, RowSide::Below, 1).map(|_| ())),
        "row-above-merged" => Box::new(|d| d.insert_rows(0, 2, RowSide::Above, 1).map(|_| ())),
        "row-below-spanned" => Box::new(|d| d.insert_rows(0, 0, RowSide::Below, 1).map(|_| ())),
        "row-below-merged-mid" => Box::new(|d| d.insert_rows(0, 1, RowSide::Below, 1).map(|_| ())),
        "row-above-restart" => Box::new(|d| d.insert_rows(0, 1, RowSide::Above, 1).map(|_| ())),
        "row-below-last-merged" => Box::new(|d| d.insert_rows(0, 2, RowSide::Below, 1).map(|_| ())),
        "row-below-styled" => Box::new(|d| d.insert_rows(0, 1, RowSide::Below, 1).map(|_| ())),
        "row-above-styled-col1" => Box::new(|d| d.insert_rows(0, 1, RowSide::Above, 1).map(|_| ())),
        "row-below-header" => Box::new(|d| d.insert_rows(0, 0, RowSide::Below, 1).map(|_| ())),
        "row-above-header" => Box::new(|d| d.insert_rows(0, 0, RowSide::Above, 1).map(|_| ())),
        "row-below-mid-tracked" => Box::new(|d| {
            d.set_track_changes(Some(track()));
            d.insert_rows(0, 1, RowSide::Below, 1).map(|_| ())
        }),
        "row-above-mid-tracked" => Box::new(|d| {
            d.set_track_changes(Some(track()));
            d.insert_rows(0, 1, RowSide::Above, 1).map(|_| ())
        }),
        "col-left-first" => Box::new(|d| d.insert_columns(0, 0, ColumnSide::Left, 1)),
        "col-right-mid" => Box::new(|d| d.insert_columns(0, 1, ColumnSide::Right, 1)),
        "col-right-last" => Box::new(|d| d.insert_columns(0, 2, ColumnSide::Right, 1)),
        "col-left-mid" => Box::new(|d| d.insert_columns(0, 1, ColumnSide::Left, 1)),
        "col-right-narrow" => Box::new(|d| d.insert_columns(0, 1, ColumnSide::Right, 1)),
        "col-left-narrow" => Box::new(|d| d.insert_columns(0, 0, ColumnSide::Left, 1)),
        "col-right-merged" => Box::new(|d| d.insert_columns(0, 0, ColumnSide::Right, 1)),
        "col-left-merged" => Box::new(|d| d.insert_columns(0, 1, ColumnSide::Left, 1)),
        "col-right-window" => Box::new(|d| d.insert_columns(0, 1, ColumnSide::Right, 1)),
        "col-right-content" => Box::new(|d| d.insert_columns(0, 1, ColumnSide::Right, 1)),
        "col-right-styled" => Box::new(|d| d.insert_columns(0, 1, ColumnSide::Right, 1)),
        "col-left-styled" => Box::new(|d| d.insert_columns(0, 0, ColumnSide::Left, 1)),
        "col-right-auto-narrow" => Box::new(|d| d.insert_columns(0, 1, ColumnSide::Right, 1)),
        "delete-row-mid" => Box::new(|d| d.delete_rows(0, 1..2)),
        "delete-row-first" => Box::new(|d| d.delete_rows(0, 0..1)),
        "delete-row-last" => Box::new(|d| d.delete_rows(0, 2..3)),
        "delete-row-merged" => Box::new(|d| d.delete_rows(0, 1..2)),
        "delete-row-merged-cont" => Box::new(|d| d.delete_rows(0, 2..3)),
        "delete-row-spanned" => Box::new(|d| d.delete_rows(0, 0..1)),
        "delete-row-styled" => Box::new(|d| d.delete_rows(0, 0..1)),
        "delete-row-mid-tracked" => Box::new(|d| {
            d.set_track_changes(Some(track()));
            d.delete_rows(0, 1..2)
        }),
        "delete-row-first-tracked" => Box::new(|d| {
            d.set_track_changes(Some(track()));
            d.delete_rows(0, 0..1)
        }),
        "delete-col-mid" => Box::new(|d| d.delete_columns(0, 1..2)),
        "delete-col-first" => Box::new(|d| d.delete_columns(0, 0..1)),
        "delete-col-last" => Box::new(|d| d.delete_columns(0, 2..3)),
        "delete-col-narrow" => Box::new(|d| d.delete_columns(0, 1..2)),
        "delete-col-window" => Box::new(|d| d.delete_columns(0, 1..2)),
        "delete-col-auto-narrow" => Box::new(|d| d.delete_columns(0, 1..2)),
        "delete-col-merged" => Box::new(|d| d.delete_columns(0, 1..2)),
        // Word does not track column deletion through the object model.
        "delete-col-mid-tracked" => Box::new(|d| {
            d.set_track_changes(Some(track()));
            d.delete_columns(0, 1..2)
        }),
        "delete-table" => Box::new(|d| d.delete_table(0)),
        "delete-table-tracked" => Box::new(|d| {
            d.set_track_changes(Some(track()));
            d.delete_table(0)
        }),
        "merge-h" => Box::new(|d| d.merge_cells(0, (0, 0), (0, 1))),
        "merge-h-row" => Box::new(|d| d.merge_cells(0, (1, 0), (1, 2))),
        "merge-v" => Box::new(|d| d.merge_cells(0, (0, 0), (1, 0))),
        "merge-v-col" => Box::new(|d| d.merge_cells(0, (0, 1), (2, 1))),
        "merge-block" => Box::new(|d| d.merge_cells(0, (0, 0), (1, 1))),
        "merge-all" => Box::new(|d| d.merge_cells(0, (0, 0), (2, 2))),
        "merge-h-beside-vmerge" => Box::new(|d| d.merge_cells(0, (1, 1), (1, 2))),
        "merge-v-col3" => Box::new(|d| d.merge_cells(0, (0, 2), (1, 2))),
        "merge-styled" => Box::new(|d| d.merge_cells(0, (1, 0), (1, 1))),
        "split-1x2" => Box::new(|d| d.split_cell(0, 1, 1, 1, 2)),
        "split-2x1" => Box::new(|d| d.split_cell(0, 1, 1, 2, 1)),
        "split-2x2" => Box::new(|d| d.split_cell(0, 1, 1, 2, 2)),
        "split-1x3-first" => Box::new(|d| d.split_cell(0, 0, 0, 1, 3)),
        "split-span" => Box::new(|d| d.split_cell(0, 0, 0, 1, 2)),
        "split-span-3" => Box::new(|d| d.split_cell(0, 0, 0, 1, 3)),
        "split-vmerge" => Box::new(|d| d.split_cell(0, 1, 0, 2, 1)),
        "split-styled" => Box::new(|d| d.split_cell(0, 1, 1, 1, 2)),
        "col-width-none" => Box::new(|d| d.set_column_width(0, 1, 100.0, ColumnAdjust::None)),
        "col-width-prop" => {
            Box::new(|d| d.set_column_width(0, 1, 100.0, ColumnAdjust::Proportional))
        }
        "col-width-same" => Box::new(|d| d.set_column_width(0, 1, 100.0, ColumnAdjust::Equal)),
        "col-width-first" => {
            Box::new(|d| d.set_column_width(0, 0, 100.0, ColumnAdjust::NextColumn))
        }
        "col-width-narrow" => Box::new(|d| d.set_column_width(0, 1, 150.0, ColumnAdjust::None)),
        "col-width-narrow-shrink" => {
            Box::new(|d| d.set_column_width(0, 1, 50.0, ColumnAdjust::None))
        }
        "col-width-narrow-prop" => {
            Box::new(|d| d.set_column_width(0, 0, 150.0, ColumnAdjust::Proportional))
        }
        "col-width-last" => Box::new(|d| d.set_column_width(0, 2, 200.0, ColumnAdjust::None)),
        "col-width-window" => Box::new(|d| d.set_column_width(0, 1, 100.0, ColumnAdjust::None)),
        "col-width-auto-narrow-grow" => {
            Box::new(|d| d.set_column_width(0, 1, 150.0, ColumnAdjust::None))
        }
        "col-width-auto-narrow-over" => {
            Box::new(|d| d.set_column_width(0, 1, 400.0, ColumnAdjust::None))
        }
        "row-height-atleast" => {
            Box::new(|d| d.set_row_height(0, 1, Some((40.0, HeightRule::AtLeast))))
        }
        "row-height-exact" => Box::new(|d| d.set_row_height(0, 1, Some((40.0, HeightRule::Exact)))),
        "row-height-auto" => Box::new(|d| {
            d.set_row_height(0, 1, Some((40.0, HeightRule::AtLeast)))?;
            d.set_row_height(0, 1, Some((0.0, HeightRule::Auto)))
        }),
        "row-header" => Box::new(|d| d.set_row_header(0, 0, true)),
        "row-cantsplit" => Box::new(|d| d.set_row_cant_split(0, 1, true)),
        "cell-shading" => {
            Box::new(|d| d.set_cell_shading(0, 0, 0, Some(clear_shading([255, 255, 0]))))
        }
        "cell-shading-pattern" => Box::new(|d| {
            d.set_cell_shading(
                0,
                0,
                1,
                Some(Shading {
                    pattern: "pct25".to_owned(),
                    fill: Some([255, 255, 0]),
                    color: Some(RED),
                }),
            )
        }),
        "row-shading" => Box::new(|d| {
            for c in 0..3 {
                d.set_cell_shading(0, 1, c, Some(clear_shading([0, 255, 0])))?;
            }
            Ok(())
        }),
        "cell-border-top" => Box::new(|d| {
            d.set_cell_border(
                0,
                1,
                1,
                BorderSide::Top,
                Some(&border("single", 1.5, Some(RED))),
            )
        }),
        "cell-border-all" => Box::new(|d| {
            for side in [
                BorderSide::Top,
                BorderSide::Left,
                BorderSide::Bottom,
                BorderSide::Right,
            ] {
                d.set_cell_border(0, 1, 1, side, Some(&border("double", 0.75, None)))?;
            }
            Ok(())
        }),
        "cell-border-none" => Box::new(|d| {
            d.set_cell_border(0, 1, 1, BorderSide::Left, Some(&border("nil", 0.0, None)))
        }),
        "table-borders-none" => Box::new(|d| {
            let none = border("none", 0.0, None);
            d.set_table_borders(
                0,
                &BorderSet {
                    top: Some(none.clone()),
                    left: Some(none.clone()),
                    bottom: Some(none.clone()),
                    right: Some(none.clone()),
                    inside_h: Some(none.clone()),
                    inside_v: Some(none),
                },
            )
        }),
        "table-borders-outside" => Box::new(|d| {
            let double = border("double", 0.5, None);
            d.set_table_borders(
                0,
                &BorderSet {
                    top: Some(double.clone()),
                    left: Some(double.clone()),
                    bottom: Some(double.clone()),
                    right: Some(double),
                    inside_h: None,
                    inside_v: None,
                },
            )
        }),
        "table-align-center" => {
            Box::new(|d| d.set_table_alignment(0, Some(TableJustification::Center)))
        }
        "table-align-right" => {
            Box::new(|d| d.set_table_alignment(0, Some(TableJustification::Right)))
        }
        "table-align-left-narrow" => {
            Box::new(|d| d.set_table_alignment(0, Some(TableJustification::Left)))
        }
        "table-indent" => Box::new(|d| d.set_table_indent(0, Some(36.0))),
        "table-pref-width" => Box::new(|d| d.set_table_width(0, TableWidth::Points(300.0))),
        "table-pref-percent" => Box::new(|d| d.set_table_width(0, TableWidth::Percent(80.0))),
        "autofit-fixed" | "autofit-fixed-narrow" => {
            Box::new(|d| d.set_table_autofit(0, AutoFitMode::Fixed))
        }
        "autofit-content" | "autofit-content-narrow" => {
            Box::new(|d| d.set_table_autofit(0, AutoFitMode::Content))
        }
        "autofit-window" | "autofit-window-narrow" => {
            Box::new(|d| d.set_table_autofit(0, AutoFitMode::Window))
        }
        "allow-autofit-off" => Box::new(|d| d.set_table_layout(0, TableLayoutType::Fixed)),
        "text-to-table-tab" => Box::new(|d| {
            let w = d.text_width_pt(1);
            d.convert_text_to_table(1, 3, TextSeparator::Tab, None, w, AutoFitMode::Fixed)
                .map(|_| ())
        }),
        "text-to-table-comma" => Box::new(|d| {
            let w = d.text_width_pt(1);
            d.convert_text_to_table(1, 2, TextSeparator::Comma, None, w, AutoFitMode::Fixed)
                .map(|_| ())
        }),
        "text-to-table-para" => Box::new(|d| {
            let w = d.text_width_pt(1);
            d.convert_text_to_table(
                1,
                4,
                TextSeparator::Paragraph,
                Some(2),
                w,
                AutoFitMode::Fixed,
            )
            .map(|_| ())
        }),
        "text-to-table-para-1col" => Box::new(|d| {
            let w = d.text_width_pt(1);
            d.convert_text_to_table(
                1,
                4,
                TextSeparator::Paragraph,
                Some(1),
                w,
                AutoFitMode::Fixed,
            )
            .map(|_| ())
        }),
        "table-to-text-tab" => Box::new(|d| d.convert_table_to_text(0, TextSeparator::Tab)),
        "table-to-text-para" => Box::new(|d| d.convert_table_to_text(0, TextSeparator::Paragraph)),
        "table-to-text-comma" => Box::new(|d| d.convert_table_to_text(0, TextSeparator::Comma)),
        "table-to-text-merged" => Box::new(|d| d.convert_table_to_text(0, TextSeparator::Tab)),
        "table-to-text-multipara" => Box::new(|d| {
            d.merge_cells(0, (0, 0), (0, 1))?;
            d.convert_table_to_text(0, TextSeparator::Tab)
        }),
        _ => return None,
    };
    Some(edit)
}

/// Word behaviours the model does not offer, and why.
fn skipped(name: &str) -> Option<&'static str> {
    Some(match name {
        "insert-3x3-default" | "insert-3x3-word8" => {
            "Word 97 table behaviour (fixed layout, empty tblLook, row-level cell margins); the UI does not produce it"
        }
        "col-right-mid-tracked" => {
            "Word tracks a column insertion as tblPrChange/tcPrChange on every cell; not modelled"
        }
        _ => return None,
    })
}

/// Categories (`kind:field`) where our output is known to differ from Word.
///
/// * Grid widths Word recomputes from measured content or its own layout pass: auto-fit to
///   contents (`autofit-content*`, `col-right-content`), preferred table width
///   (`table-pref-*`), border changes (`table-borders-outside`), rounding in percent tables
///   (`col-width-window`, `autofit-window-narrow`).
/// * Row heights Word writes from the measured layout (`merge-all`, `split-2x*`).
/// * Border materialisation: Word writes the style's bottom border on the cells above when a
///   row is shaded or a border is switched off (`row-shading`, `cell-border-none`).
/// * Columns through merged cells (`col-*-merged`, `delete-col-merged`): Word works cell by
///   cell and keeps ragged rows with `gridAfter`; the model keeps the grid rectangular and
///   widens or narrows the spanning cell instead.
fn known_divergence(name: &str) -> &'static [&'static str] {
    match name {
        "autofit-content"
        | "autofit-content-narrow"
        | "col-right-content"
        | "table-pref-width"
        | "table-pref-percent"
        | "table-borders-outside"
        | "col-width-window" => &["tbl:grid"],
        "autofit-window-narrow" => &["tbl:grid"],
        "merge-all" | "split-2x1" | "split-2x2" => &["row:pr"],
        "row-shading" | "cell-border-none" => &["cell:pr"],
        "col-left-merged" | "col-right-merged" | "delete-col-merged" => &[
            "structure",
            "tbl:grid",
            "tbl:pr",
            "row:pr",
            "cell:pr",
            "cell:txt",
        ],
        _ => &[],
    }
}

// ---------------------------------------------------------------------------
// Comparison
// ---------------------------------------------------------------------------

struct Reference {
    base: String,
    lines: Vec<String>,
}

fn reference(name: &str) -> Reference {
    let path = repo_root()
        .join("fixtures")
        .join("reference")
        .join("docx")
        .join(format!("tbledit-{name}"))
        .join("table.tsv");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut base = String::new();
    let mut lines = Vec::new();
    for line in text.lines() {
        if let Some(b) = line.strip_prefix("# base ") {
            base = b.to_owned();
        } else if !line.starts_with('#') {
            lines.push(line.to_owned());
        }
    }
    Reference { base, lines }
}

/// Top-level children of a property string `name{a(..) b(..)}`.
fn tokens(pr: &str) -> Vec<String> {
    let Some(open) = pr.find('{') else {
        return Vec::new();
    };
    let inner = &pr[open + 1..pr.len().saturating_sub(1)];
    let mut out = Vec::new();
    let mut depth = 0;
    let mut cur = String::new();
    for ch in inner.chars() {
        match ch {
            '{' | '(' => {
                depth += 1;
                cur.push(ch);
            }
            '}' | ')' => {
                depth -= 1;
                cur.push(ch);
            }
            ' ' if depth == 0 => out.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[derive(Default)]
struct Score {
    matched: usize,
    total: usize,
    differing: BTreeSet<String>,
    notes: Vec<String>,
}

/// Field names of each line kind after the leading indices.
fn field_names(kind: &str) -> &'static [&'static str] {
    match kind {
        "p" => &["p", "text"],
        "tbl" => &["tbl", "index", "grid", "rows", "pr"],
        "row" => &["row", "index", "pr", "ex"],
        "cell" => &["cell", "row", "column", "pr", "ppr", "txt"],
        _ => &[],
    }
}

fn compare(expected: &[String], actual: &[String]) -> Score {
    let mut score = Score::default();
    if expected.len() != actual.len() {
        score.differing.insert("structure".to_owned());
        score.notes.push(format!(
            "line count {} (Word) vs {} (ours)",
            expected.len(),
            actual.len()
        ));
    }
    for (i, exp) in expected.iter().enumerate() {
        let Some(act) = actual.get(i) else {
            score.total += 1;
            continue;
        };
        let ef: Vec<&str> = exp.split('\t').collect();
        let af: Vec<&str> = act.split('\t').collect();
        let kind = ef[0];
        if kind != af[0] {
            score.total += 1;
            score.differing.insert("structure".to_owned());
            score.notes.push(format!("line {i}: {kind} vs {}", af[0]));
            continue;
        }
        // Leading index fields (row/cell numbers) are structure.
        let skip = match kind {
            "p" => 1,
            "tbl" => 2,
            "row" => 2,
            "cell" => 3,
            _ => 1,
        };
        let names = field_names(kind);
        for (k, e) in ef.iter().enumerate().skip(1) {
            let a = af.get(k).copied().unwrap_or("");
            if k < skip {
                score.total += 1;
                if *e == a {
                    score.matched += 1;
                } else {
                    score.differing.insert("structure".to_owned());
                    score.notes.push(format!("line {i}: index {e} vs {a}"));
                }
                continue;
            }
            let field = e.split('=').next().unwrap_or("");
            let category = format!("{kind}:{}", names.get(k).copied().unwrap_or(field));
            if matches!(field, "pr" | "ex" | "ppr") && e.contains('{') {
                let (et, mut at) = (tokens(e), tokens(a));
                for t in &et {
                    score.total += 1;
                    if let Some(pos) = at.iter().position(|x| x == t) {
                        at.remove(pos);
                        score.matched += 1;
                    } else {
                        score.differing.insert(category.clone());
                        score
                            .notes
                            .push(format!("line {i} {category}: Word has {t}"));
                    }
                }
                for extra in at {
                    score.total += 1;
                    score.differing.insert(category.clone());
                    score
                        .notes
                        .push(format!("line {i} {category}: we add {extra}"));
                }
            } else {
                score.total += 1;
                if e == &a {
                    score.matched += 1;
                } else {
                    score.differing.insert(category.clone());
                    score
                        .notes
                        .push(format!("line {i} {category}: Word {e} / ours {a}"));
                }
            }
        }
    }
    score
}

fn scenario_names() -> Vec<String> {
    let dir = repo_root().join("fixtures").join("reference").join("docx");
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("reference dir")
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().to_str().map(str::to_owned))
        .filter_map(|n| n.strip_prefix("tbledit-").map(str::to_owned))
        .collect();
    names.sort();
    names
}

fn run(name: &str) -> Option<(Document, Reference, Score)> {
    let edit = scenario(name)?;
    let reference = reference(name);
    let mut doc = open_base(&reference.base);
    edit(&mut doc).unwrap_or_else(|e| panic!("{name}: {e}"));
    let dump: Vec<String> = doc.table_dump().lines().map(str::to_owned).collect();
    let score = compare(&reference.lines, &dump);
    Some((doc, reference, score))
}

#[test]
fn every_reference_scenario_is_modelled_or_skipped_with_a_reason() {
    for name in scenario_names() {
        assert!(
            scenario(&name).is_some() || skipped(&name).is_some(),
            "{name}: no edit and no skip reason"
        );
    }
}

#[test]
fn edits_match_word_property_by_property() {
    let mut matched = 0;
    let mut total = 0;
    let mut exact = 0;
    let mut modelled = 0;
    let mut failures = Vec::new();
    let mut by_category: BTreeMap<String, usize> = BTreeMap::new();
    for name in scenario_names() {
        let Some((_doc, _reference, score)) = run(&name) else {
            continue;
        };
        modelled += 1;
        matched += score.matched;
        total += score.total;
        if score.differing.is_empty() {
            exact += 1;
        }
        let known: BTreeSet<&str> = known_divergence(&name).iter().copied().collect();
        let unexpected: Vec<&String> = score
            .differing
            .iter()
            .filter(|c| !known.contains(c.as_str()))
            .collect();
        for c in &score.differing {
            *by_category.entry(c.clone()).or_default() += 1;
        }
        if !unexpected.is_empty() {
            failures.push(format!(
                "{name}: {}/{} fields; unexpected differences in {unexpected:?}\n    {}",
                score.matched,
                score.total,
                score.notes.join("\n    ")
            ));
        }
    }
    eprintln!(
        "table edits vs Word: {exact}/{modelled} scenarios identical, {matched}/{total} fields match"
    );
    eprintln!("scenarios differing, by category: {by_category:?}");
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn print_scores() {
    if std::env::var_os("TABLE_EDIT_VERBOSE").is_none() {
        return;
    }
    for name in scenario_names() {
        if let Some((_d, _r, s)) = run(&name) {
            eprintln!("{name}: {}/{} {:?}", s.matched, s.total, s.differing);
            for n in &s.notes {
                eprintln!("    {n}");
            }
        }
    }
}

#[test]
fn tracked_rows_are_revisions_and_reject_restores_the_table() {
    let mut doc = open_base("tbledit-base-grid3x3");
    let before = doc.table_dump();
    doc.set_track_changes(Some(track()));
    doc.insert_rows(0, 1, RowSide::Below, 1).expect("insert");
    doc.delete_rows(0, 0..1).expect("delete");
    let kinds: Vec<RevisionKind> = doc.revisions().iter().map(|r| r.kind).collect();
    assert!(kinds.contains(&RevisionKind::RowInserted), "{kinds:?}");
    assert!(kinds.contains(&RevisionKind::RowDeleted), "{kinds:?}");
    doc.reject_all_revisions();
    // Rejecting leaves empty `w:trPr` / `w:rPr` containers behind (review.rs); they carry nothing.
    let after = doc
        .table_dump()
        .replace("pr=trPr\t", "pr=-\t")
        .replace("ppr=pPr{rPr}", "ppr=-");
    assert_eq!(after, before);
}

#[test]
fn a_failed_edit_changes_nothing_and_saves_reopen_identically() {
    let mut doc = open_base("tbledit-base-merged");
    let before = doc.serialize_main();
    assert!(doc.split_cell(0, 1, 0, 3, 1).is_err());
    assert!(doc.delete_rows(0, 5..6).is_err());
    assert!(doc.insert_columns(0, 9, ColumnSide::Left, 1).is_err());
    assert_eq!(doc.serialize_main(), before);
    assert!(!doc.is_dirty());

    doc.merge_cells(0, (1, 1), (2, 2)).expect("merge");
    doc.insert_columns(0, 2, ColumnSide::Right, 1)
        .expect("column");
    let dump = doc.table_dump();
    let bytes = doc.save().expect("saves");
    let reopened = Document::open(&bytes, &Limits::default()).expect("reopens");
    assert_eq!(reopened.table_dump(), dump);
}

#[test]
fn text_survives_a_round_trip_through_a_table() {
    let mut doc = open_base("tbledit-base-lines-tab");
    let before: Vec<String> = (0..doc.paragraph_count())
        .map(|i| doc.paragraph_text(i).unwrap_or_default())
        .collect();
    let width = doc.text_width_pt(1);
    doc.convert_text_to_table(1, 3, TextSeparator::Tab, None, width, AutoFitMode::Fixed)
        .expect("to table");
    doc.convert_table_to_text(0, TextSeparator::Tab)
        .expect("to text");
    let after: Vec<String> = (0..doc.paragraph_count())
        .map(|i| doc.paragraph_text(i).unwrap_or_default())
        .collect();
    assert_eq!(after, before);
}

#[test]
fn autofit_mode_reads_back() {
    let mut doc = open_base("tbledit-base-grid3x3");
    assert_eq!(
        doc.table(0).map(|t| t.autofit_mode()),
        Some(AutoFitMode::Fixed)
    );
    for mode in [
        AutoFitMode::Content,
        AutoFitMode::Window,
        AutoFitMode::Fixed,
    ] {
        doc.set_table_autofit(0, mode).expect("autofit");
        assert_eq!(doc.table(0).map(|t| t.autofit_mode()), Some(mode));
    }
}
