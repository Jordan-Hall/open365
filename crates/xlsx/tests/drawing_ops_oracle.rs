//! What happens to worksheet drawing objects when the cells beneath them change, against Excel:
//! `fixtures/reference/xlsx/drawing-ops/ops.json` (written by `tools/office/drawing-ops-oracle.ps1`)
//! holds, per scenario, every shape's `Left/Top/Width/Height`, `TopLeftCell` and `BottomRightCell`
//! after Excel inserted, deleted, hid or resized rows, columns or cells of `drawing-ops.xlsx`.
//!
//! Positions and cells must match; sizes of move-only and free-floating objects within 1.5 points
//! (the file stores their markers for the 96 dpi grid this crate lays out, Excel's run was at 120 dpi).

mod common;

use open365_opc::Limits;
use open365_xlsx::{
    Anchor, EditAxis, FormulaShifter, GridMetrics, Rect, ShiftDirection, StructEdit, Workbook,
    col_letters,
};
use serde_json::Value;

use common::{fixture, repo_root};

struct NoFormulas;

impl FormulaShifter for NoFormulas {
    fn shift(&self, _: &str, _: &str) -> Option<String> {
        None
    }
    fn shift_name(&self, _: &str) -> Option<String> {
        None
    }
    fn shift_anchored(&self, _: &str, _: &str, _: i64) -> Option<String> {
        None
    }
}

fn reference() -> Value {
    let path = repo_root().join("fixtures/reference/xlsx/drawing-ops/ops.json");
    serde_json::from_slice(&std::fs::read(path).expect("ops.json exists")).expect("ops.json parses")
}

fn structure(wb: &mut Workbook, axis: EditAxis, delete: bool, at: u32, count: u32) {
    wb.edit_structure_with_drawings(
        0,
        StructEdit {
            axis,
            delete,
            at,
            count,
        },
        &NoFormulas,
    )
    .expect("structure edit");
}

fn cells(wb: &mut Workbook, rect: Rect, dir: ShiftDirection) {
    wb.shift_cells(0, rect, dir).expect("shift cells");
}

/// Applies the scenario Excel ran. `None` for a scenario this test does not model.
fn apply(wb: &mut Workbook, name: &str) -> bool {
    use EditAxis::{Cols, Rows};
    // Column width Excel's `ColumnWidth = 20` gives at the 120 dpi run (187 pixels).
    let wide = 187.0 / 9.0;
    match name {
        "insert-rows-above" => structure(wb, Rows, false, 3, 2),
        "insert-row-inside" => structure(wb, Rows, false, 6, 1),
        "insert-row-at-top" => structure(wb, Rows, false, 5, 1),
        "insert-rows-below" => structure(wb, Rows, false, 12, 2),
        "delete-rows-above" => structure(wb, Rows, true, 2, 2),
        "delete-row-inside" => structure(wb, Rows, true, 6, 1),
        "delete-rows-partial" => structure(wb, Rows, true, 3, 3),
        "delete-rows-over" => structure(wb, Rows, true, 4, 6),
        "insert-cols-left" => structure(wb, Cols, false, 2, 1),
        "insert-col-inside" => structure(wb, Cols, false, 5, 1),
        "insert-col-between" => structure(wb, Cols, false, 7, 1),
        "delete-col-left" => structure(wb, Cols, true, 1, 1),
        "delete-cols-inside" => structure(wb, Cols, true, 4, 2),
        "delete-cols-over" => structure(wb, Cols, true, 8, 4),
        "insert-row-at-bottom-edge" => structure(wb, Rows, false, 22, 1),
        "insert-row-at-top-edge" => structure(wb, Rows, false, 20, 1),
        "insert-row-mid-edge" => structure(wb, Rows, false, 21, 1),
        "delete-row-below-edge" => structure(wb, Rows, true, 22, 1),
        "delete-row-last-inside" => structure(wb, Rows, true, 21, 1),
        "delete-rows-exact" => structure(wb, Rows, true, 20, 2),
        "insert-col-bottom-edge" => structure(wb, Cols, false, 6, 1),
        "insert-col-left-edge" => structure(wb, Cols, false, 3, 1),
        "delete-cols-exact" => structure(wb, Cols, true, 3, 3),
        "hide-rows-inside" => reflow(wb, |w| w.hide_rows(0, 6, 6).map(drop)),
        "hide-rows-over" => reflow(wb, |w| w.hide_rows(0, 5, 8).map(drop)),
        "hide-rows-above" => reflow(wb, |w| w.hide_rows(0, 2, 3).map(drop)),
        "hide-col-inside" => reflow(wb, |w| w.hide_columns(0, 4, 4).map(drop)),
        "hide-cols-over" => reflow(wb, |w| w.hide_columns(0, 3, 6).map(drop)),
        "hide-cols-left" => reflow(wb, |w| w.hide_columns(0, 1, 1).map(drop)),
        "hide-cols-p4" => reflow(wb, |w| w.hide_columns(0, 4, 4).map(drop)),
        "hide-row-last" => reflow(wb, |w| w.hide_rows(0, 21, 21).map(drop)),
        "row-height-inside" => reflow(wb, |w| w.set_row_height(0, 6, 6, 40.0).map(drop)),
        "row-height-above" => reflow(wb, |w| w.set_row_height(0, 2, 2, 40.0).map(drop)),
        "row-height-bottom-edge" => reflow(wb, |w| w.set_row_height(0, 22, 22, 40.0).map(drop)),
        "col-width-inside" => reflow(wb, |w| w.set_column_width(0, 4, 4, wide).map(drop)),
        "col-width-left" => reflow(wb, |w| w.set_column_width(0, 1, 1, wide).map(drop)),
        "hide-then-unhide" => {
            reflow(wb, |w| w.hide_rows(0, 6, 6).map(drop));
            reflow(wb, |w| w.unhide_rows(0, 6, 6).map(drop));
        }
        "cells-insert-down" => cells(wb, (6, 3, 6, 6), ShiftDirection::Down),
        "cells-insert-down-b" => cells(wb, (3, 8, 4, 11), ShiftDirection::Down),
        "cells-insert-right" => cells(wb, (5, 3, 8, 4), ShiftDirection::Right),
        "cells-delete-up" => cells(wb, (3, 8, 4, 11), ShiftDirection::Up),
        "cells-delete-left" => cells(wb, (5, 1, 8, 2), ShiftDirection::Left),
        "cells-down-at-from-row" => cells(wb, (5, 3, 5, 6), ShiftDirection::Down),
        "cells-down-narrow-band" => cells(wb, (6, 4, 6, 5), ShiftDirection::Down),
        "cells-down-edge-band" => cells(wb, (20, 4, 20, 5), ShiftDirection::Down),
        "cells-right-wide-band" => cells(wb, (5, 3, 12, 4), ShiftDirection::Right),
        "cells-right-at-inside" => cells(wb, (5, 4, 8, 4), ShiftDirection::Right),
        "cells-right-at-from" => cells(wb, (5, 3, 8, 3), ShiftDirection::Right),
        "cells-right-exact-rows" => cells(wb, (5, 3, 10, 4), ShiftDirection::Right),
        "cells-right-fromrow-only" => cells(wb, (5, 3, 5, 4), ShiftDirection::Right),
        "cells-left-wide-band" => cells(wb, (5, 3, 12, 4), ShiftDirection::Left),
        "cells-up-inside" => cells(wb, (6, 3, 6, 6), ShiftDirection::Up),
        "cells-up-at-from" => cells(wb, (5, 3, 5, 6), ShiftDirection::Up),
        "cells-up-narrow" => cells(wb, (6, 4, 6, 5), ShiftDirection::Up),
        "row-insert-then-delete" => {
            structure(wb, Rows, false, 6, 1);
            structure(wb, Rows, true, 6, 1);
        }
        _ => return false,
    }
    true
}

fn drop<T>(_: T) {}

fn reflow(wb: &mut Workbook, f: impl FnOnce(&mut Workbook) -> Result<(), open365_xlsx::XlsxError>) {
    wb.with_drawings_reflowed(0, f).expect("geometry change");
}

#[test]
fn objects_follow_the_cells_as_in_excel() {
    let json = reference();
    let dpi = json["dpi"].as_u64().expect("dpi") as u32;
    let mut modelled = 0;
    let mut failures = Vec::new();
    for sc in json["scenarios"].as_array().expect("scenarios") {
        let name = sc["scenario"].as_str().expect("name");
        if name == "initial" {
            continue;
        }
        let mut wb = Workbook::open(&fixture("drawing-ops"), &Limits::default()).expect("opens");
        if !apply(&mut wb, name) {
            eprintln!("not modelled: {name}");
            continue;
        }
        modelled += 1;
        let metrics = GridMetrics::for_styles(wb.styles(), dpi);
        let ws = wb.sheet(0).expect("sheet");
        let objects = wb.sheet_drawing_objects(0);
        for shape in sc["shapes"].as_array().expect("shapes") {
            let n = shape["name"].as_str().expect("shape name");
            let ours = objects.iter().find(|o| o.object.name == n);
            if shape.get("deleted").is_some() {
                if ours.is_some() {
                    failures.push(format!("{name} {n}: Excel deleted it, we kept it"));
                }
                continue;
            }
            let Some(obj) = ours else {
                failures.push(format!("{name} {n}: we deleted it, Excel kept it"));
                continue;
            };
            let placement = shape["placement"].as_i64().expect("placement");
            // A free-floating object keeps its place in points on whatever grid is laid out; the
            // others are compared on Excel's 120 dpi grid.
            let m96 = GridMetrics::for_styles(wb.styles(), open365_xlsx::BASE_DPI);
            let r = if placement == 3 {
                obj.anchor.rect_px_pt(ws, m96)
            } else {
                obj.anchor.rect_px_pt(ws, metrics)
            };
            let f = |k: &str| shape[k].as_f64().expect(k);
            // Two-cell objects are exact; the others keep a stored size laid out on another grid.
            let size_tol = if placement == 1 { 0.61 } else { 1.5 };
            let pos_tol = 0.61;
            let d = [
                r.x - f("left"),
                r.y - f("top"),
                r.width - f("width"),
                r.height - f("height"),
            ];
            if d[0].abs() > pos_tol
                || d[1].abs() > pos_tol
                || d[2].abs() > size_tol
                || d[3].abs() > size_tol
            {
                failures.push(format!(
                    "{name} {n}: dx {:.2} dy {:.2} dw {:.2} dh {:.2}",
                    d[0], d[1], d[2], d[3]
                ));
            }
            // The cells of the first and last marker (Excel's TopLeftCell and BottomRightCell).
            let a1 = |col0: u32, row0: u32| format!("{}{}", col_letters(col0 + 1), row0 + 1);
            let (tl, br) = match obj.anchor {
                Anchor::TwoCell { from, to, .. } => (a1(from.col, from.row), a1(to.col, to.row)),
                Anchor::OneCell { from, .. } => (a1(from.col, from.row), String::new()),
                Anchor::Absolute { .. } => (String::new(), String::new()),
            };
            let want_tl = shape["topLeftCell"].as_str().expect("tl");
            let want_br = shape["bottomRightCell"].as_str().expect("br");
            if (placement != 3 && tl != want_tl) || (placement == 1 && br != want_br) {
                failures.push(format!(
                    "{name} {n}: cells {tl}:{br}, Excel {want_tl}:{want_br}"
                ));
            }
        }
    }
    assert!(modelled >= 50, "only {modelled} scenarios ran");
    assert!(
        failures.is_empty(),
        "{} differences:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
