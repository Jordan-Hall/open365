//! Cell entry in the Sheets preview, driven through simulated keyboard and pointer input only:
//! typing replaces, F2 and double click edit in place, the formula bar edits the active cell,
//! Enter, Tab and the arrows commit and move, Esc cancels, Delete clears, formulas recalculate,
//! IME composition lands in the cell, the AccessKit tree names the grid and the editor, and a
//! saved workbook reopens with its values.

use open365_preview::sheet::SheetModel;
use open365_preview::{AppKind, Preview};
use open365_ui_core::{ImeInput, InputEvent, Key, NodeId, Widget};

fn sheets() -> Preview {
    Preview::new(AppKind::Sheets, true, 1600, 1000, 1.25)
}

fn type_text(p: &mut Preview, s: &str) {
    p.input(InputEvent::Text(s.to_owned()));
}

fn find(p: &Preview, label: &str) -> NodeId {
    p.ui.tree
        .paint_order()
        .into_iter()
        .find(|id| p.ui.tree.get(*id).is_some_and(|n| n.label == label))
        .unwrap_or_else(|| panic!("no node labelled {label}"))
}

#[test]
fn typing_into_a1_and_b1_and_a_formula_in_c1_shows_the_sum() {
    let mut p = sheets();
    type_text(&mut p, "2");
    p.key(Key::Tab);
    type_text(&mut p, "3");
    p.key(Key::Tab);
    type_text(&mut p, "=A1+B1");
    p.key(Key::Enter);
    let s = p.shell.sheet();
    assert_eq!(s.display(1, 1), "2");
    assert_eq!(s.display(1, 2), "3");
    assert_eq!(s.display(1, 3), "5", "C1 shows the recalculated value");
    assert_eq!(s.source(1, 3), "=A1+B1");
    // Enter moved down from C1.
    assert_eq!(s.active, (2, 3));
    // Changing a precedent recalculates the formula.
    p.shell.sheet_mut().enter(1, 1, "10").unwrap();
    assert_eq!(p.shell.sheet().display(1, 3), "13");
}

#[test]
fn typing_replaces_and_arrows_commit_and_move() {
    let mut p = sheets();
    type_text(&mut p, "old");
    p.key(Key::Enter);
    p.key(Key::Up);
    assert_eq!(p.shell.sheet().active, (1, 1));
    type_text(&mut p, "new");
    assert!(p.shell.is_editing());
    p.key(Key::Right);
    assert!(!p.shell.is_editing());
    assert_eq!(p.shell.sheet().display(1, 1), "new");
    assert_eq!(p.shell.sheet().active, (1, 2));
}

#[test]
fn f2_edits_in_place_and_escape_cancels() {
    let mut p = sheets();
    type_text(&mut p, "abc");
    p.key(Key::Enter);
    p.key(Key::Up);
    p.key(Key::F2);
    assert_eq!(p.shell.editor_text(), Some("abc"));
    type_text(&mut p, "d");
    assert_eq!(
        p.shell.formula_text(),
        "abcd",
        "the formula bar follows the editor"
    );
    p.key(Key::Escape);
    assert!(!p.shell.is_editing());
    assert_eq!(p.shell.sheet().display(1, 1), "abc");
    assert_eq!(p.shell.formula_text(), "abc");
    // F2 then Enter commits the edit.
    p.key(Key::F2);
    type_text(&mut p, "!");
    p.key(Key::Enter);
    assert_eq!(p.shell.sheet().display(1, 1), "abc!");
}

#[test]
fn double_click_edits_in_place_and_a_click_selects() {
    let mut p = sheets();
    p.shell.sheet_mut().enter(2, 2, "hello").unwrap();
    let c = p.shell.cell_centre(2, 2);
    p.click(c.x, c.y);
    assert_eq!(p.shell.sheet().active, (2, 2));
    assert!(!p.shell.is_editing());
    p.click(c.x, c.y);
    assert!(p.shell.is_editing());
    assert_eq!(p.shell.editor_text(), Some("hello"));
}

#[test]
fn delete_clears_and_the_formula_bar_edits_the_active_cell() {
    let mut p = sheets();
    type_text(&mut p, "5");
    p.key(Key::Enter);
    p.key(Key::Up);
    p.key(Key::Delete);
    assert_eq!(p.shell.sheet().display(1, 1), "");

    let bar = find(&p, "Formula Bar");
    let r = p.ui.tree.abs_rect(bar);
    let s = p.scale();
    p.click((r.x + 20.0) * s, (r.y + r.h / 2.0) * s);
    type_text(&mut p, "=2*21");
    p.key(Key::Enter);
    assert_eq!(p.shell.sheet().display(1, 1), "42");
    assert_eq!(p.shell.sheet().active, (2, 1));
}

#[test]
fn the_name_box_selects_a_cell() {
    let mut p = sheets();
    p.shell.sheet_mut().enter(3, 2, "x").unwrap();
    let nb = find(&p, "Name Box");
    let r = p.ui.tree.abs_rect(nb);
    let s = p.scale();
    p.click((r.x + 10.0) * s, (r.y + r.h / 2.0) * s);
    p.input(InputEvent::Key {
        key: Key::Char('A'),
        pressed: true,
        modifiers: open365_ui_core::Modifiers {
            ctrl: true,
            ..Default::default()
        },
    });
    type_text(&mut p, "B3");
    p.key(Key::Enter);
    assert_eq!(p.shell.sheet().active, (3, 2));
    assert_eq!(p.shell.formula_text(), "x");
}

#[test]
fn ime_composition_enters_the_cell() {
    let mut p = sheets();
    assert!(p.ui.wants_ime(), "the focused grid allows the IME");
    p.input(InputEvent::Ime(ImeInput::Enabled));
    p.input(InputEvent::Ime(ImeInput::Preedit {
        text: "ni".into(),
        cursor: None,
    }));
    assert!(p.shell.is_editing());
    p.input(InputEvent::Ime(ImeInput::Commit("\u{4f60}\u{597d}".into())));
    p.key(Key::Enter);
    assert_eq!(p.shell.sheet().display(1, 1), "\u{4f60}\u{597d}");
}

#[test]
fn the_accessibility_tree_names_the_grid_and_the_editor() {
    let mut p = sheets();
    type_text(&mut p, "7");
    let tree = p.ui.accessibility_tree();
    let grid = tree
        .nodes
        .iter()
        .find(|(_, n)| n.role() == accesskit::Role::Grid)
        .expect("a Grid node");
    assert_eq!(grid.1.label(), Some("Sheet1 grid, cell A1"));
    let editor = tree
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Cell editor"))
        .expect("the editor node");
    assert_eq!(editor.1.value(), Some("7"));
    assert!(matches!(
        p.ui.tree.get(find(&p, "Cell editor")).map(|n| &n.widget),
        Some(Widget::TextField(_))
    ));
}

#[test]
fn a_saved_workbook_reopens_with_its_values() {
    let mut p = sheets();
    type_text(&mut p, "2");
    p.key(Key::Tab);
    type_text(&mut p, "3");
    p.key(Key::Tab);
    type_text(&mut p, "=A1+B1");
    p.key(Key::Enter);
    let path = std::env::temp_dir().join("open365-preview-cells-test.xlsx");
    p.shell.sheet_mut().save(&path).unwrap();
    let again = SheetModel::open(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!(again.display(1, 1), "2");
    assert_eq!(again.display(1, 2), "3");
    assert_eq!(again.display(1, 3), "5");
    assert_eq!(again.source(1, 3), "=A1+B1");
}

#[test]
fn screen_reader_actions_activate_widgets() {
    use accesskit::{Action, ActionData, ActionRequest};
    let mut p = sheets();
    let req = |p: &Preview, label: &str, action, data| ActionRequest {
        action,
        target_tree: accesskit::TreeId::ROOT,
        target_node: open365_ui_core::access::ak_id(find(p, label)),
        data,
    };
    // Click on the File button opens Backstage.
    let r = req(&p, "File", Action::Click, None);
    assert!(p.accessibility_action(&r));
    assert!(p.shell.backstage_open());
    p.shell.build_all(&mut p.ui);
    // SetValue on the Name Box selects a cell (Backstage closed first).
    let back = req(&p, "Back", Action::Click, None);
    assert!(p.accessibility_action(&back));
    assert!(!p.shell.backstage_open());
    let r = req(
        &p,
        "Name Box",
        Action::SetValue,
        Some(ActionData::Value("C4".into())),
    );
    assert!(p.accessibility_action(&r));
    assert_eq!(p.shell.sheet().active, (4, 3));
    // Focus on the grid, and SetValue on the formula bar edits the active cell.
    let r = req(
        &p,
        "Formula Bar",
        Action::SetValue,
        Some(ActionData::Value("=1+2".into())),
    );
    assert!(p.accessibility_action(&r));
    assert_eq!(p.shell.sheet().display(4, 3), "3");
    // An unsupported action on a widget is refused.
    let r = req(&p, "Formula Bar", Action::Increment, None);
    assert!(!p.accessibility_action(&r));
}
