//! Sheets grid colours and selection, driven through the headless harness: Excel's white cells
//! and light-grey grid lines, range selection by dragging, Shift+click, Shift+arrows,
//! Ctrl+arrows, Ctrl+A and the row and column headers.

#![allow(clippy::unwrap_used)]

use open365_gfx::Color;
use open365_preview::{AppKind, Palette, Preview};
use open365_ui_core::{InputEvent, Key, Modifiers};

fn sheets() -> Preview {
    Preview::new(AppKind::Sheets, false, 1600, 1000, 1.25)
}

fn chord(p: &mut Preview, key: Key, ctrl: bool, shift: bool) {
    let modifiers = Modifiers {
        ctrl,
        shift,
        alt: false,
    };
    for pressed in [true, false] {
        p.input(InputEvent::Key {
            key,
            pressed,
            modifiers,
        });
    }
}

fn hold_shift(p: &mut Preview, down: bool) {
    p.input(InputEvent::Key {
        key: Key::Shift,
        pressed: down,
        modifiers: Modifiers {
            shift: down,
            ..Modifiers::default()
        },
    });
}

#[test]
fn cells_are_white_and_grid_lines_are_excels_light_grey_in_both_themes() {
    for dark in [false, true] {
        let pal = Palette::new(AppKind::Sheets, dark);
        assert_eq!(pal.cell_bg, Color::rgb(255, 255, 255), "dark {dark}");
        assert_eq!(pal.grid_line, Color::rgb(0xe0, 0xe0, 0xe0), "dark {dark}");
        assert_eq!(pal.cell_text, Color::rgb(0, 0, 0), "dark {dark}");
    }
    let mut p = sheets();
    if !p.has_gpu() {
        return;
    }
    let c = p.shell.cell_centre(8, 4);
    let snap = p.render().expect("a frame");
    assert_eq!(snap.pixel(c.x as u32, c.y as u32), [255, 255, 255, 255]);
}

#[test]
fn dragging_with_the_mouse_selects_a_range() {
    let mut p = sheets();
    let (a, b) = (p.shell.cell_centre(2, 2), p.shell.cell_centre(4, 3));
    p.move_to(a.x, a.y);
    p.press(a.x, a.y);
    p.shell.grid_drag(&mut p.ui, b.x, b.y);
    p.release(b.x, b.y);
    p.shell.grid_release(&mut p.ui);
    assert_eq!(p.shell.sheet().range(), (2, 2, 4, 3));
    assert_eq!(p.shell.sheet().active, (2, 2), "the anchor stays active");
    assert_eq!(p.shell.name_box_text(), "B2:C4");
}

#[test]
fn shift_click_extends_from_the_active_cell() {
    let mut p = sheets();
    let (a, b) = (p.shell.cell_centre(1, 1), p.shell.cell_centre(3, 2));
    p.click(a.x, a.y);
    hold_shift(&mut p, true);
    p.click(b.x, b.y);
    hold_shift(&mut p, false);
    assert_eq!(p.shell.sheet().range(), (1, 1, 3, 2));
}

#[test]
fn shift_arrows_grow_and_shrink_the_range() {
    let mut p = sheets();
    chord(&mut p, Key::Right, false, true);
    chord(&mut p, Key::Down, false, true);
    chord(&mut p, Key::Down, false, true);
    assert_eq!(p.shell.sheet().range(), (1, 1, 3, 2));
    chord(&mut p, Key::Up, false, true);
    assert_eq!(p.shell.sheet().range(), (1, 1, 2, 2));
    assert_eq!(p.shell.sheet().active, (1, 1));
    // A plain arrow collapses the selection.
    p.key(Key::Right);
    assert!(!p.shell.sheet().is_range());
}

#[test]
fn ctrl_arrows_jump_over_blocks_of_data() {
    let mut p = sheets();
    {
        let s = p.shell.sheet_mut();
        for r in 1..=3 {
            s.enter(r, 1, "x").unwrap();
        }
        s.enter(6, 1, "y").unwrap();
        s.enter(7, 1, "y").unwrap();
    }
    chord(&mut p, Key::Down, true, false);
    assert_eq!(p.shell.sheet().active, (3, 1), "to the end of the block");
    chord(&mut p, Key::Down, true, false);
    assert_eq!(p.shell.sheet().active, (6, 1), "to the next block");
    chord(&mut p, Key::Down, true, false);
    assert_eq!(p.shell.sheet().active, (7, 1));
    chord(&mut p, Key::Down, true, false);
    assert_eq!(p.shell.sheet().active.0, 1_048_576, "to the last row");
    chord(&mut p, Key::Up, true, false);
    assert_eq!(p.shell.sheet().active, (7, 1));
    // Ctrl+Shift extends instead of moving.
    chord(&mut p, Key::Up, true, true);
    assert_eq!(p.shell.sheet().range(), (6, 1, 7, 1));
    chord(&mut p, Key::Up, true, true);
    assert_eq!(p.shell.sheet().range(), (3, 1, 7, 1));
    chord(&mut p, Key::Right, true, false);
    assert_eq!(p.shell.sheet().active.1, 16_384);
}

#[test]
fn ctrl_a_selects_every_cell() {
    let mut p = sheets();
    chord(&mut p, Key::Char('A'), true, false);
    assert_eq!(p.shell.sheet().range(), (1, 1, 1_048_576, 16_384));
    assert_eq!(p.shell.sheet().active, (1, 1));
}

#[test]
fn clicking_headers_selects_whole_columns_and_rows() {
    let mut p = sheets();
    let h = p.shell.col_header_centre(3);
    p.click(h.x, h.y);
    assert_eq!(p.shell.sheet().range(), (1, 3, 1_048_576, 3));
    assert_eq!(p.shell.name_box_text(), "C1:C1048576");
    // Dragging across headers widens the selection.
    let (a, b) = (p.shell.col_header_centre(2), p.shell.col_header_centre(4));
    p.press(a.x, a.y);
    p.shell.grid_drag(&mut p.ui, b.x, b.y);
    p.release(b.x, b.y);
    p.shell.grid_release(&mut p.ui);
    assert_eq!(p.shell.sheet().range(), (1, 2, 1_048_576, 4));

    let h = p.shell.row_header_centre(5);
    p.click(h.x, h.y);
    assert_eq!(p.shell.sheet().range(), (5, 1, 5, 16_384));
    // Shift+click on another row header extends.
    hold_shift(&mut p, true);
    let h = p.shell.row_header_centre(7);
    p.click(h.x, h.y);
    hold_shift(&mut p, false);
    assert_eq!(p.shell.sheet().range(), (5, 1, 7, 16_384));
    // The corner box selects everything.
    let g = p.shell.cell_centre(1, 1);
    p.click(g.x - 60.0, g.y - 30.0);
    assert_eq!(p.shell.sheet().range(), (1, 1, 1_048_576, 16_384));
}
