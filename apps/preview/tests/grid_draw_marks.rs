//! Note and comment markers, their hover popup, trace arrows, Show Formulas and AutoSum keys.

#![allow(clippy::unwrap_used)]

mod grid_draw_common;
use grid_draw_common::*;
use open365_ui_core::Key;

const RED: [u8; 3] = [0xff, 0x00, 0x00];
const PURPLE: [u8; 3] = [0x87, 0x64, 0xb8];
const NOTE_YELLOW: [u8; 3] = [0xff, 0xff, 0xe1];
const TRACE_BLUE: [u8; 3] = [0x1e, 0x5f, 0xcc];
const TRACE_RED: [u8; 3] = [0xe0, 0x1b, 0x1b];

#[test]
fn a_note_draws_a_red_marker_and_a_comment_a_purple_one() {
    let mut p = sheets();
    let blank = p.draw_list();
    assert_eq!(count(&blank, RED), 0);
    assert_eq!(count(&blank, PURPLE), 0);
    p.shell.sheet_mut().select_cell((2, 2));
    p.shell.sheet_mut().review_set_note("check this").unwrap();
    p.shell.sheet_mut().select_cell((4, 3));
    p.shell.sheet_mut().review_add_comment("why?").unwrap();
    let list = p.draw_list();
    assert!(count(&list, RED) > 0, "the note marker is red");
    assert!(count(&list, PURPLE) > 0, "the comment marker is purple");
}

#[test]
fn a_marker_from_a_file_draws_and_hovering_the_cell_shows_the_note() {
    let mut p = open_fixture("comments-notes.xlsx");
    assert!(!p.shell.sheet().grid_marks().notes.is_empty());
    assert!(count(&p.draw_list(), RED) > 0);
    assert_eq!(
        count(&p.draw_list(), NOTE_YELLOW),
        0,
        "no popup until the pointer is over the cell"
    );
    let c = p.shell.cell_centre(1, 1);
    p.move_to(c.x, c.y);
    assert!(
        count(&p.draw_list(), NOTE_YELLOW) > 0,
        "the yellow note popup"
    );
    let far = p.shell.cell_centre(8, 6);
    p.move_to(far.x, far.y);
    assert_eq!(count(&p.draw_list(), NOTE_YELLOW), 0);
}

#[test]
fn trace_arrows_are_blue_and_an_error_source_makes_them_red() {
    let mut p = sheets();
    {
        let m = p.shell.sheet_mut();
        m.enter(1, 1, "1").unwrap();
        m.enter(1, 2, "=A1+1").unwrap();
        m.select_cell((1, 2));
        m.trace_precedents();
    }
    assert_eq!(p.shell.sheet().ext_formulas.arrows.len(), 1);
    let list = p.draw_list();
    assert!(count(&list, TRACE_BLUE) > 0);
    assert_eq!(count(&list, TRACE_RED), 0);
    {
        let m = p.shell.sheet_mut();
        m.remove_arrows();
        m.enter(3, 1, "=1/0").unwrap();
        m.enter(3, 2, "=A3+1").unwrap();
        m.select_cell((3, 2));
        m.trace_precedents();
    }
    let list = p.draw_list();
    assert!(count(&list, TRACE_RED) > 0, "the arrow from #DIV/0! is red");
}

#[test]
fn an_off_sheet_arrow_is_dashed_black_with_a_sheet_icon() {
    let mut p = open_fixture("cross-sheet.xlsx");
    let m = p.shell.sheet_mut();
    let cell = (1..12u32)
        .flat_map(|r| (1..6u32).map(move |c| (r, c)))
        .find(|&(r, c)| m.source(r, c).contains('!'));
    let (r, c) = cell.expect("a cross-sheet formula in the fixture");
    m.select_cell((r, c));
    m.trace_precedents();
    assert!(m.ext_formulas.arrows.iter().any(|a| a.off_sheet));
    let list = p.draw_list();
    assert!(count(&list, [0, 0, 0]) > 0);
}

#[test]
fn ctrl_backtick_shows_formulas_in_the_cells() {
    let mut p = sheets();
    p.shell.sheet_mut().enter(1, 1, "2").unwrap();
    p.shell.sheet_mut().enter(1, 2, "=A1*3").unwrap();
    assert_eq!(p.shell.sheet().shown_for_view(1, 2).unwrap().0, "6");
    chord(&mut p, Key::Char('`'), mods(true, false, false));
    assert!(p.shell.sheet().ext_formulas.show_formulas);
    assert_eq!(p.shell.sheet().shown_for_view(1, 2).unwrap().0, "=A1*3");
    let _ = p.draw_list();
    chord(&mut p, Key::Char('`'), mods(true, false, false));
    assert_eq!(p.shell.sheet().shown_for_view(1, 2).unwrap().0, "6");
}

#[test]
fn alt_equals_sums_the_cells_above() {
    let mut p = sheets();
    p.shell.sheet_mut().enter(1, 1, "1").unwrap();
    p.shell.sheet_mut().enter(2, 1, "2").unwrap();
    p.shell.sheet_mut().select_cell((3, 1));
    // The Alt+= chord reaches the grid as this call (ui-core hands Alt+letter to access keys).
    p.shell.auto_sum(&mut p.ui);
    assert_eq!(p.shell.sheet().source(3, 1), "=SUM(A1:A2)");
    assert_eq!(p.shell.sheet().display(3, 1), "3");
}
