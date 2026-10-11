//! The Find and Replace dialog in both applications: Ctrl+F / Ctrl+H, find next, and Replace All
//! with a count, undone in one step.

use std::path::PathBuf;

use egui::{Event, Key, Modifiers, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use open365_ui_egui::App;

fn harness(rel: &str) -> Harness<'static, App> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(rel);
    let app = App::new(Some(path));
    let mut h = Harness::builder()
        .with_size(vec2(1100.0, 800.0))
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    h.run();
    h
}

fn ctrl(h: &mut Harness<'_, App>, key: Key) {
    h.event(Event::Key {
        key,
        physical_key: Some(key),
        pressed: true,
        repeat: false,
        modifiers: Modifiers::COMMAND,
    });
    h.run();
}

fn paragraph(h: &Harness<'_, App>, i: usize) -> String {
    h.state()
        .target()
        .and_then(|t| t.docx())
        .and_then(|d| d.paragraph_text(i))
        .unwrap_or_default()
}

fn source(h: &Harness<'_, App>, row: u32, col: u32) -> String {
    let wb = h.state().target().and_then(|t| t.xlsx()).expect("workbook");
    open365_commands::cell_source_text(wb, 0, row, col)
}

#[test]
fn ctrl_h_replaces_all_in_a_word_document_and_undo_restores_it() {
    let mut h = harness("docx/find-runs.docx");
    assert!(!h.state().find.open);
    ctrl(&mut h, Key::H);
    assert!(h.state().find.open && h.state().find.replace_mode);
    h.state_mut().find.find = "cat".to_owned();
    h.state_mut().find.replace = "dog".to_owned();
    h.run();
    h.get_by_label("Replace all").click();
    h.run();
    assert_eq!(h.state().status(), "13 replacements made");
    // Word matches the capitals of the found text: Cat becomes Dog, CAT becomes DOG.
    assert_eq!(paragraph(&h, 0), "The Dog sat on the dog mat. DOG dog.");
    h.state_mut().undo();
    h.run();
    assert_eq!(paragraph(&h, 0), "The Cat sat on the cat mat. CAT cat.");
}

#[test]
fn word_options_change_what_is_replaced() {
    let mut h = harness("docx/find-runs.docx");
    ctrl(&mut h, Key::H);
    {
        let f = &mut h.state_mut().find;
        f.find = "cat".to_owned();
        f.replace = "dog".to_owned();
        f.match_case = true;
        f.whole_word = true;
    }
    h.run();
    h.get_by_label("Replace all").click();
    h.run();
    assert_eq!(h.state().status(), "4 replacements made");
    assert_eq!(paragraph(&h, 0), "The Cat sat on the dog mat. CAT dog.");
}

#[test]
fn find_next_selects_matches_and_wraps() {
    let mut h = harness("docx/find-runs.docx");
    ctrl(&mut h, Key::F);
    assert!(h.state().find.open && !h.state().find.replace_mode);
    h.state_mut().find.find = "cat".to_owned();
    h.run();
    h.get_by_label("Find next").click();
    h.run();
    let (a, b) = h.state().doc.selection();
    assert_eq!((a.para, a.offset, b.offset), (0, 4, 7));
    h.get_by_label("Find next").click();
    h.run();
    let (a, _) = h.state().doc.selection();
    assert_eq!((a.para, a.offset), (0, 19));
    h.get_by_label("Find previous").click();
    h.run();
    let (a, _) = h.state().doc.selection();
    assert_eq!((a.para, a.offset), (0, 4));
}

#[test]
fn excel_replace_all_counts_cells_and_undoes() {
    let mut h = harness("xlsx/find-cells.xlsx");
    ctrl(&mut h, Key::H);
    {
        let f = &mut h.state_mut().find;
        f.find = "apple".to_owned();
        f.replace = "pear".to_owned();
        f.match_case = true;
    }
    h.run();
    h.get_by_label("Replace all").click();
    h.run();
    let status = h.state().status().to_owned();
    assert!(status.contains("replacements made in"), "{status}");
    assert_eq!(source(&h, 1, 2), "pear pie");
    h.state_mut().undo();
    h.run();
    assert_eq!(source(&h, 1, 2), "apple pie");
}

#[test]
fn excel_find_next_moves_the_active_cell() {
    let mut h = harness("xlsx/find-cells.xlsx");
    ctrl(&mut h, Key::F);
    h.state_mut().find.find = "banana".to_owned();
    h.run();
    h.get_by_label("Find next").click();
    h.run();
    assert_eq!(h.state().grid.active(), (2, 3));
}
