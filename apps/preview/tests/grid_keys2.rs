//! Sheets keyboard: Tab then Enter returns to the start column, Enter and Tab walk a selection,
//! Ctrl+Space and Shift+Space select columns and rows, Ctrl+; enters the date, F2 flips the
//! editor between Enter and Edit mode, Esc drops the copy marker.

#![allow(clippy::unwrap_used)]

use open365_preview::sheet::MemoryClipboard;
use open365_preview::{AppKind, Preview};
use open365_ui_core::{InputEvent, Key, Modifiers};

fn sheets() -> Preview {
    let mut p = Preview::new(AppKind::Sheets, false, 1600, 1000, 1.25);
    p.shell
        .sheet_mut()
        .set_clipboard(MemoryClipboard::default());
    p
}

fn chord(p: &mut Preview, key: Key, ctrl: bool, shift: bool) {
    for pressed in [true, false] {
        p.input(InputEvent::Key {
            key,
            pressed,
            modifiers: Modifiers {
                ctrl,
                shift,
                alt: false,
            },
        });
    }
}

#[test]
fn enter_after_tabs_returns_to_the_column_the_tabs_began_in() {
    let mut p = sheets();
    p.shell.sheet_mut().select_cell((2, 2));
    p.key(Key::Tab);
    p.key(Key::Tab);
    assert_eq!(p.shell.sheet().active, (2, 4));
    p.key(Key::Enter);
    assert_eq!(p.shell.sheet().active, (3, 2));
    // A plain Enter goes straight down.
    p.key(Key::Enter);
    assert_eq!(p.shell.sheet().active, (4, 2));
    // An arrow in between cancels the return.
    p.key(Key::Tab);
    p.key(Key::Right);
    p.key(Key::Enter);
    assert_eq!(p.shell.sheet().active, (5, 4));
}

#[test]
fn tab_and_enter_walk_inside_a_selection() {
    let mut p = sheets();
    {
        let s = p.shell.sheet_mut();
        s.select_cell((1, 1));
        s.extend_to((2, 2));
    }
    p.key(Key::Tab);
    assert_eq!(p.shell.sheet().active, (1, 2));
    p.key(Key::Tab);
    assert_eq!(
        p.shell.sheet().active,
        (2, 1),
        "wraps to the next row of the selection"
    );
    // Enter walks down a column, then to the top of the next one, then round.
    p.key(Key::Enter);
    assert_eq!(p.shell.sheet().active, (1, 2));
    p.key(Key::Enter);
    assert_eq!(p.shell.sheet().active, (2, 2));
    p.key(Key::Enter);
    assert_eq!(p.shell.sheet().active, (1, 1), "round the selection");
    assert_eq!(p.shell.sheet().range(), (1, 1, 2, 2), "the selection stays");
}

#[test]
fn ctrl_space_and_shift_space_select_columns_and_rows() {
    let mut p = sheets();
    p.shell.sheet_mut().select_cell((3, 2));
    chord(&mut p, Key::Space, true, false);
    assert_eq!(p.shell.sheet().range(), (1, 2, 1_048_576, 2));
    p.shell.sheet_mut().select_cell((3, 2));
    chord(&mut p, Key::Space, false, true);
    assert_eq!(p.shell.sheet().range(), (3, 1, 3, 16_384));
    assert!(!p.shell.is_editing());
}

#[test]
fn ctrl_semicolon_enters_the_date_and_the_time() {
    let mut p = sheets();
    p.shell.sheet_mut().select_cell((1, 1));
    chord(&mut p, Key::Char(';'), true, false);
    assert!(!p.shell.sheet().source(1, 1).is_empty());
    p.shell.sheet_mut().select_cell((2, 1));
    chord(&mut p, Key::Char(';'), true, true);
    assert!(!p.shell.sheet().source(2, 1).is_empty());
}

#[test]
fn typing_is_enter_mode_and_f2_flips_to_edit_mode() {
    let mut p = sheets();
    p.shell.sheet_mut().enter(1, 1, "abc").unwrap();
    p.shell.sheet_mut().select_cell((1, 1));
    p.key(Key::F2);
    assert_eq!(p.shell.editor_text(), Some("abc"));
    // Edit mode: the arrow moves the caret and does not commit.
    p.key(Key::Down);
    assert!(p.shell.is_editing());
    // F2 again is Enter mode: the arrow commits and moves.
    p.key(Key::F2);
    p.key(Key::Down);
    assert!(!p.shell.is_editing());
    assert_eq!(p.shell.sheet().active, (2, 1));
}

#[test]
fn escape_drops_the_copy_marker() {
    let mut p = sheets();
    p.shell.sheet_mut().enter(1, 1, "x").unwrap();
    p.shell.sheet_mut().select_cell((1, 1));
    chord(&mut p, Key::Char('C'), true, false);
    assert_eq!(p.shell.sheet().copied_range(), Some((1, 1, 1, 1)));
    p.key(Key::Escape);
    assert_eq!(p.shell.sheet().copied_range(), None);
}
