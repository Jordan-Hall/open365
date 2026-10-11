//! Scrolling the Sheets grid: the mouse wheel, the scroll bars (drag and wheel over them) and the
//! frozen row and column headers, which stay put while the cells move.

#![allow(clippy::unwrap_used)]

use open365_preview::{AppKind, Preview};
use open365_ui_core::{InputEvent, NodeId};

fn sheets() -> Preview {
    Preview::new(AppKind::Sheets, false, 1600, 1000, 1.25)
}

fn bar(p: &Preview, label: &str) -> NodeId {
    p.ui.tree
        .paint_order()
        .into_iter()
        .find(|id| p.ui.tree.get(*id).is_some_and(|n| n.label == label))
        .unwrap_or_else(|| panic!("no node labelled {label}"))
}

#[test]
fn the_wheel_scrolls_three_rows_a_notch_and_shift_scrolls_sideways() {
    let mut p = sheets();
    p.shell.grid_wheel(&mut p.ui, -1.0, false);
    assert_eq!(p.shell.sheet().top_left, (4, 1));
    p.shell.grid_wheel(&mut p.ui, -2.0, false);
    assert_eq!(p.shell.sheet().top_left, (10, 1));
    p.shell.grid_wheel(&mut p.ui, 1.0, false);
    assert_eq!(p.shell.sheet().top_left, (7, 1));
    p.shell.grid_wheel(&mut p.ui, 100.0, false);
    assert_eq!(p.shell.sheet().top_left, (1, 1), "stops at the first row");
    p.shell.grid_wheel(&mut p.ui, -1.0, true);
    assert_eq!(p.shell.sheet().top_left, (1, 4));
    // Scrolling does not move the active cell.
    assert_eq!(p.shell.sheet().active, (1, 1));
}

#[test]
fn headers_stay_fixed_while_the_cells_scroll() {
    let mut p = sheets();
    let before = (p.shell.cell_centre(1, 1), p.shell.col_header_centre(1));
    p.shell.grid_wheel(&mut p.ui, -5.0, false);
    p.shell.grid_wheel(&mut p.ui, -2.0, true);
    let top_left = p.shell.sheet().top_left;
    assert_eq!(top_left, (16, 7));
    // The first visible cell sits where cell A1 sat, under the same headers.
    let after = (
        p.shell.cell_centre(top_left.0, top_left.1),
        p.shell.col_header_centre(top_left.1),
    );
    assert_eq!(before, after);
    // Cell A1 is now off-screen above and left of the headers.
    assert!(p.shell.cell_centre(1, 1).x < before.0.x);
    assert!(p.shell.cell_centre(1, 1).y < before.0.y);
}

#[test]
fn dragging_the_scroll_bar_thumb_scrolls_the_grid() {
    let mut p = sheets();
    let v = bar(&p, "Vertical scroll bar");
    let r = p.ui.tree.abs_rect(v);
    let s = p.scale();
    let x = (r.x + r.w / 2.0) * s;
    // The thumb starts at the top of the track.
    let (y0, y1) = ((r.y + 4.0) * s, (r.y + 120.0) * s);
    p.move_to(x, y0);
    p.press(x, y0);
    p.move_to(x, y1);
    p.release(x, y1);
    assert!(
        p.shell.sheet().top_left.0 > 1,
        "scrolled to {:?}",
        p.shell.sheet().top_left
    );
    assert_eq!(p.shell.sheet().top_left.1, 1);
}

#[test]
fn the_wheel_over_a_scroll_bar_moves_it() {
    let mut p = sheets();
    let h = bar(&p, "Horizontal scroll bar");
    let r = p.ui.tree.abs_rect(h);
    let s = p.scale();
    p.move_to((r.x + r.w / 2.0) * s, (r.y + r.h / 2.0) * s);
    p.input(InputEvent::Wheel { delta_y: -1.0 });
    assert!(p.shell.sheet().top_left.1 > 1);
}
