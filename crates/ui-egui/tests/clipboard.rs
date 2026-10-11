//! Copy, cut and paste in both applications.

use std::path::PathBuf;

use egui::{Event, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use open365_ui_egui::{App, ClipboardContents, ClipboardState, MemoryClipboard};

fn harness(rel: &str) -> (Harness<'static, App>, MemoryClipboard) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(rel);
    let mut app = App::new(Some(path));
    let board = MemoryClipboard::default();
    app.clipboard = ClipboardState::new(Box::new(board.clone()));
    let mut h = Harness::builder()
        .with_size(vec2(1100.0, 800.0))
        .build_ui_state(|ui, app: &mut App| app.ui(ui), app);
    h.run();
    (h, board)
}

fn contents(board: &MemoryClipboard) -> ClipboardContents {
    board.0.borrow().clone()
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

fn focus_document(h: &mut Harness<'_, App>) {
    h.get_by_role(egui::accesskit::Role::MultilineTextInput)
        .focus();
    h.run();
}

#[test]
fn word_copy_gives_text_and_html_and_cut_removes_the_selection() {
    let (mut h, board) = harness("docx/find-runs.docx");
    focus_document(&mut h);
    h.state_mut().doc.select(
        open365_ui_egui::DocPos::new(0, 4),
        open365_ui_egui::DocPos::new(0, 7),
    );
    h.event(Event::Copy);
    h.run();
    let c = contents(&board);
    assert_eq!(c.text, "Cat");
    // "at" is bold in the fixture.
    assert!(
        c.html.contains("font-weight:bold") && c.html.contains(">C</span>"),
        "{}",
        c.html
    );
    h.event(Event::Cut);
    h.run();
    assert_eq!(paragraph(&h, 0), "The  sat on the cat mat. CAT cat.");
    h.event(Event::Paste("Dog".to_owned()));
    h.run();
    assert_eq!(paragraph(&h, 0), "The Dog sat on the cat mat. CAT cat.");
}

#[test]
fn excel_copy_gives_tsv_and_html_and_paste_shifts_formulas() {
    let (mut h, board) = harness("xlsx/find-cells.xlsx");
    // B2 holds =SUM(A2:A3); select B2:C2 and copy.
    h.state_mut().grid.select_range((2, 2), (2, 3));
    h.run();
    h.event(Event::Copy);
    h.run();
    let c = contents(&board);
    assert_eq!(c.text, "36\tbanana\n");
    assert!(
        c.html.contains("<table>") && c.html.contains("<td>banana</td>"),
        "{}",
        c.html
    );
    // Paste four rows down: the relative references of the formula move with it.
    h.state_mut().grid.set_active(6, 2);
    h.run();
    h.event(Event::Paste(c.text.clone()));
    h.run();
    assert_eq!(source(&h, 6, 2), "=SUM(A6:A7)");
    assert_eq!(source(&h, 6, 3), "banana");
    // One undo reverts the whole paste.
    h.state_mut().undo();
    h.run();
    assert_eq!(source(&h, 6, 2), "");
    assert_eq!(source(&h, 6, 3), "");
}

#[test]
fn excel_paste_of_foreign_tsv_is_entered_as_typed_and_cut_clears() {
    let (mut h, _board) = harness("xlsx/find-cells.xlsx");
    h.state_mut().grid.set_active(10, 1);
    h.run();
    h.event(Event::Paste("1\t=A10+1\r\n\"two\tlines\"\t".to_owned()));
    h.run();
    assert_eq!(source(&h, 10, 1), "1");
    assert_eq!(source(&h, 10, 2), "=A10+1");
    assert_eq!(source(&h, 11, 1), "two\tlines");
    h.state_mut().grid.select_range((10, 1), (11, 2));
    h.run();
    h.event(Event::Cut);
    h.run();
    assert_eq!(source(&h, 10, 1), "");
    assert_eq!(source(&h, 11, 1), "");
    h.state_mut().undo();
    h.run();
    assert_eq!(source(&h, 10, 2), "=A10+1");
}
