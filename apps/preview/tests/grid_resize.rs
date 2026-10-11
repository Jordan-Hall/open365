//! Resizing columns and rows by dragging a header edge, double-click autofit, and the fill handle.
//! Each change is one command, so each is one undo step.

#![allow(clippy::unwrap_used)]

use open365_preview::{AppKind, Preview};
use open365_ui_core::{InputEvent, Key, Modifiers};

fn sheets() -> Preview {
    Preview::new(AppKind::Sheets, false, 1600, 1000, 1.25)
}

fn undo(p: &mut Preview) {
    for pressed in [true, false] {
        p.input(InputEvent::Key {
            key: Key::Char('Z'),
            pressed,
            modifiers: Modifiers {
                ctrl: true,
                ..Modifiers::default()
            },
        });
    }
}

#[test]
fn dragging_a_column_edge_resizes_the_column() {
    let mut p = sheets();
    let before = p.shell.sheet().geometry().col_px(2);
    assert_eq!(before, 64, "Excel's default column is 64 px at 96 dpi");
    let e = p.shell.col_edge_point(2);
    p.press(e.x, e.y);
    // 50 logical pixels wider: 62.5 device pixels at 125 %.
    p.shell.grid_drag(&mut p.ui, e.x + 62.5, e.y);
    p.release(e.x + 62.5, e.y);
    p.shell.grid_release(&mut p.ui);
    let after = p.shell.sheet().geometry().col_px(2);
    assert!((113..=115).contains(&after), "width {after}");
    // The neighbours kept their size and column C moved right by the difference.
    assert_eq!(p.shell.sheet().geometry().col_px(3), 64);
    undo(&mut p);
    assert_eq!(p.shell.sheet().geometry().col_px(2), 64, "one undo step");
}

#[test]
fn dragging_a_row_edge_resizes_the_row() {
    let mut p = sheets();
    assert_eq!(
        p.shell.sheet().geometry().row_px(3),
        19,
        "the blank workbook default, 14.4 pt"
    );
    let e = p.shell.row_edge_point(3);
    p.press(e.x, e.y);
    p.shell.grid_drag(&mut p.ui, e.x, e.y + 25.0);
    p.release(e.x, e.y + 25.0);
    p.shell.grid_release(&mut p.ui);
    let after = p.shell.sheet().geometry().row_px(3);
    assert!((39..=41).contains(&after), "height {after}");
    // Hit testing follows the new size: row 4 starts lower.
    let c = p.shell.cell_centre(4, 1);
    p.click(c.x, c.y);
    assert_eq!(p.shell.sheet().active, (4, 1));
}

#[test]
fn resizing_a_selected_column_resizes_every_selected_column() {
    let mut p = sheets();
    let h = p.shell.col_header_centre(2);
    p.click(h.x, h.y);
    {
        let s = p.shell.sheet_mut();
        s.select_cols(2, 4, false);
    }
    let e = p.shell.col_edge_point(3);
    p.press(e.x, e.y);
    p.shell.grid_drag(&mut p.ui, e.x - 25.0, e.y);
    p.release(e.x - 25.0, e.y);
    p.shell.grid_release(&mut p.ui);
    let g = p.shell.sheet().geometry();
    assert_eq!(
        (g.col_px(2), g.col_px(3), g.col_px(4)),
        (g.col_px(3), g.col_px(3), g.col_px(3))
    );
    assert!(g.col_px(3) < 64);
    assert_eq!(g.col_px(1), 64);
}

#[test]
fn double_click_on_the_edge_fits_the_widest_text() {
    let mut p = sheets();
    p.shell
        .sheet_mut()
        .enter(
            1,
            2,
            "A long piece of text that overflows the default width",
        )
        .unwrap();
    p.shell.sheet_mut().enter(2, 2, "short").unwrap();
    let e = p.shell.col_edge_point(2);
    p.click(e.x, e.y);
    p.click(e.x, e.y);
    let wide = p.shell.sheet().geometry().col_px(2);
    assert!(wide > 150, "fitted to {wide}");
    // An empty column keeps its width.
    let e = p.shell.col_edge_point(6);
    p.click(e.x, e.y);
    p.click(e.x, e.y);
    assert_eq!(p.shell.sheet().geometry().col_px(6), 64);
    // Double click on a row edge restores the default height.
    let e = p.shell.row_edge_point(2);
    p.press(e.x, e.y);
    p.shell.grid_drag(&mut p.ui, e.x, e.y + 30.0);
    p.release(e.x, e.y + 30.0);
    p.shell.grid_release(&mut p.ui);
    assert!(p.shell.sheet().geometry().row_px(2) > 30);
    let e = p.shell.row_edge_point(2);
    p.click(e.x, e.y);
    p.click(e.x, e.y);
    assert_eq!(p.shell.sheet().geometry().row_px(2), 19);
}

#[test]
fn the_fill_handle_continues_a_number_series() {
    let mut p = sheets();
    {
        let s = p.shell.sheet_mut();
        s.enter(1, 1, "1").unwrap();
        s.enter(2, 1, "3").unwrap();
        s.enter(1, 2, "Item1").unwrap();
        s.enter(1, 3, "=A1*2").unwrap();
        s.select_cell((1, 1));
        s.extend_to((2, 3));
    }
    let h = p.shell.fill_handle_point();
    let to = p.shell.cell_centre(5, 3);
    p.press(h.x, h.y);
    p.shell.grid_drag(&mut p.ui, to.x, to.y);
    p.release(to.x, to.y);
    p.shell.grid_release(&mut p.ui);
    let s = p.shell.sheet();
    assert_eq!(
        (3..=5).map(|r| s.display(r, 1)).collect::<Vec<_>>(),
        ["5", "7", "9"],
        "a step of two continues"
    );
    assert_eq!(s.range(), (1, 1, 5, 3), "the selection grows over the fill");
    assert_eq!(
        s.source(3, 3),
        "=A3*2",
        "formulas fill with shifted references"
    );
    undo(&mut p);
    assert_eq!(p.shell.sheet().display(3, 1), "", "one undo step");
}

#[test]
fn fill_right_counts_up_text_with_a_number() {
    let mut p = sheets();
    {
        let s = p.shell.sheet_mut();
        s.enter(1, 1, "Item1").unwrap();
        s.select_cell((1, 1));
        assert!(s.fill_to((1, 4)).unwrap());
    }
    let s = p.shell.sheet();
    assert_eq!(
        (1..=4).map(|c| s.display(1, c)).collect::<Vec<_>>(),
        ["Item1", "Item2", "Item3", "Item4"]
    );
}
