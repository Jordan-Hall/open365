//! Format Cells, Row Height, Column Width, Page Setup, Zoom, Protect, the Merge question and the
//! validation alerts: each opens, takes keyboard input and OK applies the effect.

#![allow(clippy::unwrap_used)]

mod dialogs_common;

use dialogs_common::*;
use open365_preview::dialogs::request::{DialogRequest, FormatTab};
use open365_preview::sheet_ext::data::Rejected;
use open365_ui_core::Key;
use open365_xlsx::{ColorSource, FillPattern, HorizontalAlignment};

#[test]
fn format_cells_number_tab_sets_percentage_with_the_arrow_keys() {
    let mut p = sheets();
    p.shell.sheet_mut().enter(1, 1, "0.5").unwrap();
    p.shell.sheet_mut().select_cell((1, 1));
    assert!(bare(&mut p, "home.number.launcher"));
    assert_eq!(title(&p).as_deref(), Some("Format Cells"));
    // Office opens on the tab strip; Tab goes into the page, to the category list.
    key(&mut p, Key::Tab);
    assert_eq!(
        focus_label(&p),
        "General",
        "the selected row of the Category list"
    );
    for _ in 0..6 {
        key(&mut p, Key::Down); // General -> Percentage
    }
    key(&mut p, Key::Enter);
    assert!(!is_open(&p));
    assert_eq!(p.shell.sheet().home_format_code(1, 1), "0.00%");
}

#[test]
fn format_cells_tabs_switch_with_arrows_and_ctrl_tab() {
    let mut p = sheets();
    p.shell.sheet_mut().select_cell((1, 1));
    assert!(p.shell.open_dialog(
        &mut p.ui,
        DialogRequest::FormatCells {
            tab: FormatTab::Alignment
        }
    ));
    // The Alignment page is showing: its Horizontal box is in the tab order.
    key(&mut p, Key::Tab);
    assert_eq!(focus_label(&p), "Horizontal");
    key(&mut p, Key::Down); // Left
    ctrl(&mut p, Key::Tab); // Font
    key(&mut p, Key::Tab);
    assert_eq!(focus_label(&p), "Font", "the Font page is showing");
    key(&mut p, Key::Enter);
    assert!(!is_open(&p));
    let f = p.shell.sheet().home_cell_format(1, 1).unwrap();
    assert_eq!(f.alignment.horizontal, HorizontalAlignment::Left);
}

#[test]
fn format_cells_font_fill_and_border_tabs() {
    let mut p = sheets();
    p.shell.sheet_mut().select_cell((2, 2));
    assert!(p.shell.open_dialog(
        &mut p.ui,
        DialogRequest::FormatCells {
            tab: FormatTab::Font
        }
    ));
    key(&mut p, Key::Tab); // Font name
    key(&mut p, Key::Tab); // Font style
    keys(&mut p, &[Key::Down, Key::Down]); // Regular, Italic, Bold
    // Border page: Tab goes to the presets (none checked yet), Down checks Outline.
    ctrl(&mut p, Key::Tab);
    key(&mut p, Key::Tab);
    key(&mut p, Key::Down);
    // Fill page.
    ctrl(&mut p, Key::Tab);
    key(&mut p, Key::Tab);
    for _ in 0..5 {
        key(&mut p, Key::Down); // No Color -> Yellow
    }
    key(&mut p, Key::Enter);
    assert!(!is_open(&p), "{}", p.shell.sheet().status);
    let f = p.shell.sheet().home_cell_format(2, 2).unwrap();
    assert!(f.font.bold);
    assert_eq!(f.fill.pattern, FillPattern::Solid);
    assert!(matches!(f.fill.fg.source, ColorSource::Rgb(c) if (c.r, c.g, c.b) == (255, 255, 0)));
    assert!(
        format!("{:?}", f.border.left).contains("Thin"),
        "{:?}",
        f.border.left
    );
}

#[test]
fn row_height_and_column_width_take_typed_numbers() {
    let mut p = sheets();
    p.shell.sheet_mut().select_cell((3, 2));
    assert!(bare(&mut p, "home.cells.format.row-height"));
    assert_eq!(title(&p).as_deref(), Some("Row Height"));
    type_text(&mut p, "30");
    key(&mut p, Key::Enter);
    assert!(!is_open(&p));
    assert_eq!(p.shell.sheet().home_row_state(3).0, Some(30.0));

    assert!(bare(&mut p, "home.cells.format.column-width"));
    assert_eq!(title(&p).as_deref(), Some("Column Width"));
    type_text(&mut p, "20");
    key(&mut p, Key::Enter);
    assert!(!is_open(&p));
    let (w, _) = p.shell.sheet().home_col_state(2).unwrap();
    assert!((w - 20.7).abs() < 0.5, "{w}");

    // Nonsense stays open with the complaint, Escape leaves.
    assert!(bare(&mut p, "home.cells.format.row-height"));
    type_text(&mut p, "9999");
    key(&mut p, Key::Enter);
    assert!(is_open(&p));
    key(&mut p, Key::Escape);
    assert!(!is_open(&p));
}

#[test]
fn zoom_dialog_sets_the_percentage() {
    let mut p = sheets();
    assert!(bare(&mut p, "view.zoom.zoom"));
    assert_eq!(title(&p).as_deref(), Some("Zoom"));
    key(&mut p, Key::Down); // 100% -> 75%
    key(&mut p, Key::Enter);
    assert!(!is_open(&p));
    assert_eq!(p.shell.sheet().ext_view.zoom, 75);
}

#[test]
fn page_setup_changes_orientation_margins_and_titles() {
    let mut p = sheets();
    assert!(bare(&mut p, "page-layout.page-set-up.launcher"));
    assert_eq!(title(&p).as_deref(), Some("Page Setup"));
    key(&mut p, Key::Tab); // portrait (the checked radio)
    key(&mut p, Key::Down); // landscape
    ctrl(&mut p, Key::Tab); // Margins page
    key(&mut p, Key::Tab);
    assert_eq!(focus_label(&p), "Top");
    type_text(&mut p, "2");
    key(&mut p, Key::Enter);
    assert!(!is_open(&p), "{}", p.shell.sheet().status);
    let page = p.shell.sheet().page();
    assert!(page.landscape);
    assert!((page.margins.top - 2.0).abs() < 1e-9, "{:?}", page.margins);
}

#[test]
fn merge_asks_before_it_discards_values() {
    let mut p = sheets();
    p.shell.sheet_mut().enter(1, 1, "x").unwrap();
    p.shell.sheet_mut().enter(1, 2, "y").unwrap();
    p.shell.sheet_mut().select_cell((1, 1));
    p.shell.sheet_mut().extend_to((1, 2));
    assert!(
        p.shell
            .try_open_dialog(&mut p.ui, "home.alignment.merge-centre")
    );
    key(&mut p, Key::Escape);
    assert!(
        p.shell.sheet().home_merged().is_empty(),
        "Cancel merged nothing"
    );
    assert!(
        p.shell
            .try_open_dialog(&mut p.ui, "home.alignment.merge-centre")
    );
    key(&mut p, Key::Enter);
    assert_eq!(p.shell.sheet().home_merged().len(), 1);
    // Nothing to discard: no question.
    p.shell.sheet_mut().select_cell((5, 1));
    p.shell.sheet_mut().extend_to((5, 2));
    assert!(
        !p.shell
            .try_open_dialog(&mut p.ui, "home.alignment.merge-centre")
    );
}

#[test]
fn validation_alerts_let_an_entry_through_on_yes() {
    let mut p = sheets();
    let rejected = Rejected {
        style: "warning",
        title: "Check".to_owned(),
        message: "That value is outside the list.".to_owned(),
    };
    assert!(
        p.shell
            .open_validation_alert(&mut p.ui, (2, 2), "zzz", &rejected)
    );
    assert_eq!(title(&p).as_deref(), Some("Check"));
    alt(&mut p, 'n'); // No
    assert!(!is_open(&p));
    assert_eq!(p.shell.sheet().source(2, 2), "");
    assert!(
        p.shell
            .open_validation_alert(&mut p.ui, (2, 2), "zzz", &rejected)
    );
    key(&mut p, Key::Enter); // Yes
    assert_eq!(p.shell.sheet().source(2, 2), "zzz");
    let info = Rejected {
        style: "information",
        title: "Note".to_owned(),
        message: "Heads up.".to_owned(),
    };
    assert!(p.shell.open_validation_alert(&mut p.ui, (3, 2), "q", &info));
    key(&mut p, Key::Enter); // OK
    assert_eq!(p.shell.sheet().source(3, 2), "q");
}

#[test]
fn protect_sheet_dialog_applies_options_and_cancel_lifts_the_default() {
    let mut p = sheets();
    assert!(bare(&mut p, "review.protect.protect-sheet"));
    assert_eq!(title(&p).as_deref(), Some("Protect Sheet"));
    alt(&mut p, 'f'); // Format cells
    key(&mut p, Key::Enter);
    assert!(!is_open(&p));
    let prot = p
        .shell
        .sheet()
        .review_sheet_protection()
        .expect("protected");
    assert!(prot.format_cells);

    // Unprotect, then ask again and cancel: the sheet ends unprotected.
    p.shell
        .run_command(&mut p.ui, "review.protect.protect-sheet");
    assert!(p.shell.sheet().review_sheet_protection().is_none());
    assert!(bare(&mut p, "review.protect.protect-sheet"));
    key(&mut p, Key::Escape);
    assert!(p.shell.sheet().review_sheet_protection().is_none());
}

#[test]
fn protect_workbook_dialog_protects_the_structure() {
    let mut p = sheets();
    assert!(bare(&mut p, "review.protect.protect-workbook"));
    assert_eq!(title(&p).as_deref(), Some("Protect Structure and Windows"));
    key(&mut p, Key::Enter);
    assert!(!is_open(&p));
    assert!(p.shell.sheet().review_workbook_protected());
}
