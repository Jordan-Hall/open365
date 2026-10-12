//! Sheets pointer buttons, context menus, the Insert, Delete and Paste Special dialogs, the
//! Ctrl+drag fill, the AutoFill Options button and the animated copy outline, driven through the
//! shell as a person would.

#![allow(clippy::unwrap_used)]

use open365_preview::sheet::MemoryClipboard;
use open365_preview::{AppKind, Preview};
use open365_ui_core::geom::Point;
use open365_ui_core::{InputEvent, Key, Modifiers, NodeId, PointerButton, Widget};

fn sheets() -> Preview {
    let mut p = Preview::new(AppKind::Sheets, false, 1600, 1000, 1.25);
    p.shell
        .sheet_mut()
        .set_clipboard(MemoryClipboard::default());
    p
}

fn mods(ctrl: bool, shift: bool, alt: bool) -> Modifiers {
    Modifiers { shift, ctrl, alt }
}

fn chord(p: &mut Preview, key: Key, m: Modifiers) {
    for pressed in [true, false] {
        p.input(InputEvent::Key {
            key,
            pressed,
            modifiers: m,
        });
    }
}

fn right_click(p: &mut Preview, x: f32, y: f32) {
    let s = p.scale();
    let pt = Point::new(x / s, y / s);
    p.input(InputEvent::PointerMoved(pt));
    p.input(InputEvent::PointerDown(PointerButton::Secondary, pt));
    p.input(InputEvent::PointerUp(PointerButton::Secondary, pt));
}

fn node_by_label(p: &Preview, label: &str) -> Option<NodeId> {
    p.ui.tree.paint_order().into_iter().find(|id| {
        p.ui.tree
            .get(*id)
            .is_some_and(|n| n.label == label && p.ui.tree.shown(*id))
    })
}

fn click_label(p: &mut Preview, label: &str) {
    let id = node_by_label(p, label).unwrap_or_else(|| panic!("no node {label}"));
    let c = p.ui.tree.abs_rect(id).center();
    let s = p.scale();
    p.click(c.x * s, c.y * s);
}

fn menu_labels(p: &Preview) -> Vec<String> {
    let id = p.ui.context_menu().expect("a context menu");
    match p.ui.tree.get(id).map(|n| &n.widget) {
        Some(Widget::Menu(m)) if m.open => m.items.iter().map(|i| i.label.clone()).collect(),
        _ => panic!("the context menu is not open"),
    }
}

fn menu_open(p: &Preview) -> bool {
    p.ui.context_menu()
        .and_then(|id| p.ui.tree.get(id))
        .is_some_and(|n| matches!(&n.widget, Widget::Menu(m) if m.open))
}

#[test]
fn a_right_click_selects_the_cell_and_opens_excels_cell_menu() {
    let mut p = sheets();
    let c = p.shell.cell_centre(3, 3);
    right_click(&mut p, c.x, c.y);
    assert_eq!(
        p.shell.sheet().active,
        (3, 3),
        "the cell under the pointer is selected"
    );
    let labels = menu_labels(&p);
    assert_eq!(&labels[..3], ["Cut", "Copy", "Paste Options:"]);
    for l in [
        "Paste Special...",
        "Insert...",
        "Delete...",
        "Clear Contents",
        "Format Cells...",
    ] {
        assert!(labels.iter().any(|x| x == l), "{l} in {labels:?}");
    }
    let pos = |l: &str| labels.iter().position(|x| x == l).unwrap();
    assert!(pos("Insert...") < pos("Delete...") && pos("Delete...") < pos("Clear Contents"));
    assert!(pos("Clear Contents") < pos("Format Cells..."));
    // A right click inside the selection keeps it.
    p.key(Key::Escape);
    assert!(!menu_open(&p));
    {
        let s = p.shell.sheet_mut();
        s.select_cell((1, 1));
        s.extend_to((4, 4));
    }
    p.shell.context_menu_at(&mut p.ui, c.x, c.y);
    assert_eq!(p.shell.sheet().range(), (1, 1, 4, 4));
    assert!(menu_open(&p));
}

#[test]
fn copy_from_the_menu_starts_the_animated_outline_and_esc_stops_it() {
    let mut p = sheets();
    p.shell.sheet_mut().enter(3, 3, "x").unwrap();
    let c = p.shell.cell_centre(3, 3);
    right_click(&mut p, c.x, c.y);
    assert!(!p.ui.animating());
    p.key(Key::Char('C'));
    assert_eq!(p.shell.sheet().copied_range(), Some((3, 3, 3, 3)));
    assert!(
        p.ui.animating(),
        "the frame source runs while the outline marches"
    );
    assert!(p.ui.next_wake().is_some());
    let f = p.ui.anim_frame();
    let t = std::time::Instant::now() + std::time::Duration::from_millis(500);
    p.ui.handle(InputEvent::Tick(t));
    assert!(p.ui.anim_frame() > f);
    p.key(Key::Escape);
    assert_eq!(p.shell.sheet().copied_range(), None);
    assert!(!p.ui.animating(), "no wake-ups once nothing animates");
}

#[test]
fn header_menus_insert_and_delete_whole_rows_and_columns() {
    let mut p = sheets();
    {
        let s = p.shell.sheet_mut();
        s.enter(2, 1, "a").unwrap();
        s.enter(3, 1, "b").unwrap();
    }
    let h = p.shell.row_header_centre(2);
    right_click(&mut p, h.x, h.y);
    assert_eq!(p.shell.sheet().range().0, 2);
    let labels = menu_labels(&p);
    assert!(
        labels.iter().any(|l| l == "Insert") && labels.iter().any(|l| l == "Delete"),
        "{labels:?}"
    );
    p.key(Key::Char('I'));
    assert_eq!(p.shell.sheet().source(2, 1), "");
    assert_eq!(p.shell.sheet().source(3, 1), "a");
    // Column header: Delete removes the column.
    p.shell.sheet_mut().enter(1, 2, "right").unwrap();
    let h = p.shell.col_header_centre(1);
    right_click(&mut p, h.x, h.y);
    p.key(Key::Char('D'));
    assert_eq!(p.shell.sheet().source(1, 1), "right");
}

#[test]
fn insert_dialog_shifts_cells_down_or_right_and_remembers_the_choice() {
    let mut p = sheets();
    {
        let s = p.shell.sheet_mut();
        for (i, v) in ["1", "2", "3"].iter().enumerate() {
            s.enter(i as u32 + 1, 1, v).unwrap();
        }
        s.enter(2, 2, "right").unwrap();
        s.select_cell((2, 1));
    }
    chord(&mut p, Key::Char('+'), mods(true, true, false));
    assert!(p.shell.dialog_open());
    // The default is Shift cells down; Enter is OK.
    p.key(Key::Enter);
    assert!(!p.shell.dialog_open());
    let s = p.shell.sheet();
    assert_eq!(
        (s.source(2, 1), s.source(3, 1), s.source(4, 1)),
        (String::new(), "2".into(), "3".into())
    );
    assert_eq!(s.source(2, 2), "right", "other columns stay");
    // Shift cells right moves the row's cells right.
    p.shell.sheet_mut().select_cell((2, 2));
    chord(&mut p, Key::Char('+'), mods(true, true, false));
    click_label(&mut p, "Shift cells right");
    click_label(&mut p, "OK");
    let s = p.shell.sheet();
    assert_eq!(
        (s.source(2, 2), s.source(2, 3)),
        (String::new(), "right".into())
    );
    // Esc cancels.
    chord(&mut p, Key::Char('+'), mods(true, true, false));
    assert!(p.shell.dialog_open());
    p.key(Key::Escape);
    assert!(!p.shell.dialog_open());
    assert_eq!(p.shell.sheet().source(2, 3), "right");
    // Entire row.
    p.shell.sheet_mut().select_cell((3, 1));
    chord(&mut p, Key::Char('+'), mods(true, true, false));
    click_label(&mut p, "Entire row");
    click_label(&mut p, "OK");
    assert_eq!(p.shell.sheet().source(4, 1), "2", "row 3 moved to 4 whole");
}

#[test]
fn delete_dialog_closes_the_gap_left_or_up() {
    let mut p = sheets();
    {
        let s = p.shell.sheet_mut();
        for (i, v) in ["a", "b", "c", "d"].iter().enumerate() {
            s.enter(i as u32 + 1, 1, v).unwrap();
        }
        s.select_cell((2, 1));
    }
    chord(&mut p, Key::Char('-'), mods(true, false, false));
    assert!(p.shell.dialog_open());
    click_label(&mut p, "Shift cells up");
    click_label(&mut p, "OK");
    let s = p.shell.sheet();
    assert_eq!(
        (s.source(2, 1), s.source(3, 1), s.source(4, 1)),
        ("c".into(), "d".into(), String::new())
    );
    // Whole rows selected: no question.
    p.shell.sheet_mut().select_rows(1, 1, false);
    chord(&mut p, Key::Char('-'), mods(true, false, false));
    assert!(!p.shell.dialog_open());
    assert_eq!(p.shell.sheet().source(1, 1), "c");
}

#[test]
fn paste_special_dialog_pastes_values_skipping_blanks() {
    let mut p = sheets();
    {
        let s = p.shell.sheet_mut();
        s.enter(1, 1, "x").unwrap();
        s.enter(3, 1, "z").unwrap();
        s.enter(2, 3, "keep").unwrap();
        s.enter(1, 2, "=1+1").unwrap();
        s.select_cell((1, 1));
        s.extend_to((3, 1));
    }
    chord(&mut p, Key::Char('C'), mods(true, false, false));
    assert!(p.shell.sheet().has_copied());
    p.shell.sheet_mut().select_cell((1, 3));
    chord(&mut p, Key::Char('V'), mods(true, false, true));
    assert!(p.shell.dialog_open(), "Ctrl+Alt+V opens Paste Special");
    click_label(&mut p, "Skip blanks");
    click_label(&mut p, "Values");
    click_label(&mut p, "OK");
    let s = p.shell.sheet();
    assert_eq!(
        (s.source(1, 3), s.source(2, 3), s.source(3, 3)),
        ("x".into(), "keep".into(), "z".into())
    );
}

#[test]
fn paste_special_without_a_copy_does_not_open() {
    let mut p = sheets();
    chord(&mut p, Key::Char('V'), mods(true, false, true));
    assert!(!p.shell.dialog_open());
}

#[test]
fn ctrl_drag_of_the_fill_handle_counts_a_lone_number_and_options_redo_the_fill() {
    let mut p = sheets();
    p.shell.sheet_mut().enter(1, 1, "5").unwrap();
    p.shell.sheet_mut().select_cell((1, 1));
    let h = p.shell.fill_handle_point();
    let to = p.shell.cell_centre(3, 1);
    p.input(InputEvent::Key {
        key: Key::Ctrl,
        pressed: true,
        modifiers: mods(true, false, false),
    });
    p.press(h.x, h.y);
    p.move_to(to.x, to.y);
    p.release(to.x, to.y);
    p.input(InputEvent::Key {
        key: Key::Ctrl,
        pressed: false,
        modifiers: Modifiers::default(),
    });
    let s = p.shell.sheet();
    assert_eq!(
        (s.source(2, 1), s.source(3, 1)),
        ("6".into(), "7".into()),
        "Ctrl makes it count"
    );
    // The AutoFill Options button follows; Copy Cells redoes the fill.
    assert!(node_by_label(&p, "AutoFill Options").is_some());
    click_label(&mut p, "AutoFill Options");
    assert!(menu_open(&p));
    assert_eq!(menu_labels(&p)[0], "Copy Cells");
    p.key(Key::Char('C'));
    let s = p.shell.sheet();
    assert_eq!((s.source(2, 1), s.source(3, 1)), ("5".into(), "5".into()));
    // Moving the selection takes the button away.
    p.shell.sheet_mut().select_cell((9, 9));
    p.key(Key::Down);
    assert!(node_by_label(&p, "AutoFill Options").is_none());
}

#[test]
fn shift_f10_opens_the_cell_menu_and_the_middle_button_does_nothing() {
    let mut p = sheets();
    chord(&mut p, Key::F10, mods(false, true, false));
    assert!(menu_open(&p));
    p.key(Key::Escape);
    assert!(!menu_open(&p));
    let c = p.shell.cell_centre(2, 2);
    let s = p.scale();
    let pt = Point::new(c.x / s, c.y / s);
    p.input(InputEvent::PointerDown(PointerButton::Middle, pt));
    p.input(InputEvent::PointerUp(PointerButton::Middle, pt));
    assert!(!menu_open(&p));
    assert_eq!(
        p.shell.sheet().active,
        (1, 1),
        "a middle press does not select"
    );
}
