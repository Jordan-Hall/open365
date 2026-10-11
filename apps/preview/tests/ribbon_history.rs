//! The Undo and Redo buttons of the quick access toolbar follow the command history.

#![allow(clippy::unwrap_used)]

use open365_preview::doc::WriterPane;
use open365_preview::{AppKind, Preview};
use open365_ui_core::NodeId;

fn node(p: &Preview, label: &str) -> NodeId {
    p.ui.tree
        .paint_order()
        .into_iter()
        .find(|id| p.ui.tree.get(*id).is_some_and(|n| n.label == label))
        .unwrap_or_else(|| panic!("no node labelled {label}"))
}

fn enabled(p: &Preview, label: &str) -> bool {
    p.ui.tree.get(node(p, label)).unwrap().enabled
}

fn click(p: &mut Preview, label: &str) {
    let r = p.ui.tree.abs_rect(node(p, label));
    let s = p.scale();
    p.click((r.x + 4.0) * s, (r.y + r.h / 2.0) * s);
}

#[test]
fn sheets_undo_and_redo_enable_with_the_history() {
    let mut p = Preview::new(AppKind::Sheets, false, 1600, 1000, 1.25);
    assert!(!enabled(&p, "Undo") && !enabled(&p, "Redo"));
    p.shell.sheet_mut().enter(1, 1, "7").unwrap();
    p.shell.sync_history(&mut p.ui);
    assert!(enabled(&p, "Undo") && !enabled(&p, "Redo"));
    click(&mut p, "Undo");
    assert_eq!(p.shell.sheet().source(1, 1), "");
    assert!(!enabled(&p, "Undo") && enabled(&p, "Redo"));
    click(&mut p, "Redo");
    assert_eq!(p.shell.sheet().source(1, 1), "7");
    assert!(enabled(&p, "Undo") && !enabled(&p, "Redo"));
}

#[test]
fn writer_undo_enables_after_an_edit_and_redo_after_an_undo() {
    let mut p = Preview::new(AppKind::Writer, false, 1600, 1000, 1.25);
    let mut pane = WriterPane::blank();
    pane.insert_text("hello");
    p.shell.set_writer(&mut p.ui, pane);
    p.shell.sync_history(&mut p.ui);
    assert!(enabled(&p, "Undo"));
    click(&mut p, "Undo");
    assert_eq!(p.shell.writer().unwrap().text(), "");
    assert!(!enabled(&p, "Undo"));
    let (_, redo) = p.shell.history_state();
    assert!(redo);
}

#[test]
fn writer_redo_is_exact_and_an_edit_clears_it() {
    let mut pane = WriterPane::blank();
    assert!(!pane.can_redo());
    pane.insert_text("a");
    pane.insert_text("b");
    assert!(pane.undo() && pane.undo());
    assert!(pane.can_redo());
    assert!(pane.redo());
    assert!(pane.can_redo(), "one redo step is left");
    assert!(pane.redo());
    assert!(!pane.can_redo(), "no steps left");
    assert!(pane.undo());
    pane.insert_text("c");
    assert!(!pane.can_redo(), "a new edit drops the redo stack");
}
