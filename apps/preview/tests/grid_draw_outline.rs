//! The outline gutter: it makes room, the level buttons and the +/- of a group hide and show
//! the detail rows.

#![allow(clippy::unwrap_used)]

mod grid_draw_common;
use grid_draw_common::*;
use open365_preview::doc::sheetchrome;

fn grouped() -> open365_preview::Preview {
    let mut p = sheets();
    {
        let m = p.shell.sheet_mut();
        for r in 1..=5u32 {
            m.enter(r, 1, &r.to_string()).unwrap();
        }
        m.select_cell((2, 1));
        m.extend_to((3, 1));
        m.data_group_rows().unwrap();
        m.select_cell((1, 1));
    }
    let list = p.draw_list();
    drop(list);
    p
}

/// Centre of the row-group band (`level`, 1-based) of the gutter at the height of `row`.
fn band(p: &open365_preview::Preview, level: u32, row: u32) -> (f32, f32) {
    let s = p.scale();
    let marks = p.shell.sheet().grid_marks();
    let (gw, _) = sheetchrome::gutter(&marks, s);
    let a1 = p.shell.cell_centre(row, 1);
    let left_of_cells = a1.x - p.shell.sheet().geometry().col_px(1) as f32 * s / 2.0;
    let grid_x0 = 2.0 * p.shell.row_header_centre(row).x - left_of_cells;
    let x0 = grid_x0 - gw;
    (
        (x0 + (level as f32 - 0.5) * (14.0 * s).round()).round(),
        a1.y,
    )
}

#[test]
fn a_group_makes_a_gutter_and_the_cells_move_right() {
    let mut p = sheets();
    let before = p.shell.cell_centre(1, 1).x;
    p.shell.sheet_mut().select_cell((2, 1));
    p.shell.sheet_mut().extend_to((3, 1));
    p.shell.sheet_mut().data_group_rows().unwrap();
    p.shell.sheet_mut().select_cell((1, 1));
    let _ = p.draw_list();
    let marks = p.shell.sheet().grid_marks();
    assert_eq!(marks.row_levels, 1);
    assert_eq!(marks.row_groups.len(), 1);
    assert_eq!(
        (marks.row_groups[0].first, marks.row_groups[0].last),
        (2, 3)
    );
    assert!(sheetchrome::gutter(&marks, p.scale()).0 > 0.0);
    p.shell
        .sheet_mut()
        .set_clipboard(open365_preview::sheet::MemoryClipboard::default());
    // Rebuild the canvas so the geometry follows (the shell does after each command).
    let after = p.shell.cell_centre(1, 1).x;
    assert!(after > before, "{after} > {before}");
}

#[test]
fn the_summary_button_collapses_and_expands_the_group() {
    let mut p = grouped();
    assert!(!p.shell.sheet().geometry().row_hidden(2));
    let (x, y) = band(&p, 1, 4);
    p.click(x, y);
    assert!(p.shell.sheet().geometry().row_hidden(2) && p.shell.sheet().geometry().row_hidden(3));
    assert_eq!(p.shell.sheet().active, (1, 1), "the selection did not move");
    assert!(p.shell.sheet().grid_marks().row_groups[0].collapsed);
    let (x, y) = band(&p, 1, 4);
    p.click(x, y);
    assert!(!p.shell.sheet().geometry().row_hidden(2));
}

#[test]
fn level_button_one_collapses_everything_and_two_shows_all() {
    let mut p = grouped();
    let s = p.scale();
    let g = (14.0 * s).round();
    let (x, _) = band(&p, 1, 1);
    // The level buttons sit beside the column headings.
    let y = p.shell.col_header_centre(1).y;
    p.click(x, y);
    assert!(p.shell.sheet().geometry().row_hidden(2));
    p.click(x + g, y);
    assert!(!p.shell.sheet().geometry().row_hidden(2));
}
