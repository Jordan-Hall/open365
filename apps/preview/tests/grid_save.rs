//! Save As: Ctrl+S on an unnamed workbook and the Save As command (F12) ask for a path through
//! the dialog, then save through the command layer. The dialog is replaced by a closure.

#![allow(clippy::unwrap_used)]

use std::cell::RefCell;
use std::rc::Rc;

use open365_preview::sheet::SheetModel;
use open365_preview::{AppKind, Preview};
use open365_ui_core::{InputEvent, Key, Modifiers};

fn sheets() -> Preview {
    Preview::new(AppKind::Sheets, false, 1600, 1000, 1.25)
}

fn ctrl_s(p: &mut Preview) {
    for pressed in [true, false] {
        p.input(InputEvent::Key {
            key: Key::Char('S'),
            pressed,
            modifiers: Modifiers {
                ctrl: true,
                ..Modifiers::default()
            },
        });
    }
}

#[test]
fn ctrl_s_on_an_unnamed_workbook_opens_save_as_and_saves_there() {
    let mut p = sheets();
    let asked = Rc::new(RefCell::new(Vec::new()));
    let target = std::env::temp_dir().join("open365-grid-save-test.xlsx");
    let _ = std::fs::remove_file(&target);
    {
        let (asked, target) = (Rc::clone(&asked), target.clone());
        p.shell.sheet_mut().set_save_chooser(Box::new(move |name| {
            asked.borrow_mut().push(name.to_owned());
            Some(target.clone())
        }));
    }
    p.shell.sheet_mut().enter(1, 1, "42").unwrap();
    assert!(p.shell.sheet().path().is_none());
    ctrl_s(&mut p);
    assert_eq!(*asked.borrow(), ["Book1.xlsx"]);
    assert_eq!(p.shell.sheet().path().as_deref(), Some(target.as_path()));
    assert!(!p.shell.sheet().is_dirty());
    let again = SheetModel::open(&target).unwrap();
    assert_eq!(again.display(1, 1), "42");
    // Now it has a name: Ctrl+S saves in place, without the dialog.
    p.shell.sheet_mut().enter(1, 1, "43").unwrap();
    ctrl_s(&mut p);
    assert_eq!(asked.borrow().len(), 1);
    assert_eq!(SheetModel::open(&target).unwrap().display(1, 1), "43");
    // Save As asks again and suggests the current file name.
    p.shell.save_as();
    assert_eq!(asked.borrow().len(), 2);
    assert_eq!(asked.borrow()[1], "open365-grid-save-test.xlsx");
    let _ = std::fs::remove_file(&target);
}

#[test]
fn cancelling_the_dialog_saves_nothing() {
    let mut p = sheets();
    p.shell.sheet_mut().set_save_chooser(Box::new(|_| None));
    p.shell.sheet_mut().enter(1, 1, "x").unwrap();
    ctrl_s(&mut p);
    assert!(p.shell.sheet().path().is_none());
    assert!(p.shell.sheet().is_dirty());
}

#[test]
fn a_name_without_an_extension_gets_xlsx() {
    let mut p = sheets();
    let base = std::env::temp_dir().join("open365-grid-save-noext");
    let _ = std::fs::remove_file(base.with_extension("xlsx"));
    let chosen = base.clone();
    p.shell
        .sheet_mut()
        .set_save_chooser(Box::new(move |_| Some(chosen.clone())));
    p.shell.save_as();
    assert!(base.with_extension("xlsx").exists());
    let _ = std::fs::remove_file(base.with_extension("xlsx"));
}
