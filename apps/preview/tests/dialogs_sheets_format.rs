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
    // Border page: Tab visits the style list, the colour, then the presets; Outline draws the
    // selected (thin) line around the cell.
    ctrl(&mut p, Key::Tab);
    keys(&mut p, &[Key::Tab, Key::Tab, Key::Tab, Key::Tab]);
    assert_eq!(focus_label(&p), "Outline");
    key(&mut p, Key::Enter);
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
    // One OK, one undo step: font, fill and border go together, and come back together.
    assert!(p.shell.sheet_mut().undo());
    let f = p.shell.sheet().home_cell_format(2, 2).unwrap();
    assert!(!f.font.bold);
    assert_eq!(f.fill.pattern, FillPattern::None);
    assert!(!format!("{:?}", f.border.left).contains("Thin"));
    assert!(p.shell.sheet_mut().redo());
    let f = p.shell.sheet().home_cell_format(2, 2).unwrap();
    assert!(f.font.bold && f.fill.pattern == FillPattern::Solid);
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

// ---- Protect, Unprotect and Confirm Password -----------------------------------------------------

/// Presses the OK button (Enter would press the focused push button instead).
fn press_ok(p: &mut open365_preview::Preview) {
    let ok = find(p, "OK").expect("OK button");
    click(p, ok);
}

/// The text a focused field shows (a password field shows bullets only).
fn focused_text(p: &open365_preview::Preview) -> String {
    match p
        .ui
        .focus()
        .and_then(|f| p.ui.tree.get(f))
        .map(|n| &n.widget)
    {
        Some(open365_ui_core::Widget::TextField(e)) => e.text.clone(),
        other => panic!("not a text field: {other:?}"),
    }
}

#[test]
fn protect_sheet_opens_before_it_protects_and_asks_for_the_password_twice() {
    let mut p = sheets();
    assert!(bare(&mut p, "review.protect.protect-sheet"));
    assert_eq!(title(&p).as_deref(), Some("Protect Sheet"));
    assert!(
        p.shell.sheet().review_sheet_protection().is_none(),
        "the dialog comes first"
    );
    type_text(&mut p, "secret");
    assert_eq!(
        focused_text(&p),
        "\u{2022}".repeat(6),
        "the field is masked"
    );
    assert!(!dump(&p).iter().any(|l| l.contains("secret")));
    key(&mut p, Key::Enter);
    assert_eq!(title(&p).as_deref(), Some("Confirm Password"));
    assert!(p.shell.sheet().review_sheet_protection().is_none());

    // A different second entry returns to Protect Sheet with Excel's complaint.
    type_text(&mut p, "other");
    key(&mut p, Key::Enter);
    assert_eq!(title(&p).as_deref(), Some("Protect Sheet"));
    assert!(dump(&p).contains(&"Confirmation password is not identical.".to_owned()));
    assert!(p.shell.sheet().review_sheet_protection().is_none());
    assert_eq!(focused_text(&p), "", "the password was cleared");

    // The same twice protects.
    type_text(&mut p, "secret");
    key(&mut p, Key::Enter);
    type_text(&mut p, "secret");
    key(&mut p, Key::Enter);
    assert!(!is_open(&p), "{:?}", dump(&p));
    assert!(p.shell.sheet().review_sheet_protection().is_some());
    assert!(
        p.shell
            .sheet_mut()
            .review_unprotect_sheet(Some("nope"))
            .is_err()
    );
    assert!(p.shell.sheet().review_sheet_protection().is_some());

    // Escape on Confirm Password leaves Protect Sheet open and nothing changed.
    assert!(
        p.shell
            .sheet_mut()
            .review_unprotect_sheet(Some("secret"))
            .is_ok()
    );
    assert!(bare(&mut p, "review.protect.protect-sheet"));
    type_text(&mut p, "abc");
    key(&mut p, Key::Enter);
    key(&mut p, Key::Escape);
    assert_eq!(title(&p).as_deref(), Some("Protect Sheet"));
    key(&mut p, Key::Escape);
    assert!(!is_open(&p));
    assert!(p.shell.sheet().review_sheet_protection().is_none());
}

#[test]
fn unprotect_sheet_prompts_for_the_password_and_answers_a_wrong_one_with_a_message() {
    let mut p = sheets();
    p.shell
        .sheet_mut()
        .review_protect_sheet(Default::default(), Some("secret"))
        .unwrap();
    assert!(bare(&mut p, "review.protect.protect-sheet"));
    assert_eq!(title(&p).as_deref(), Some("Unprotect Sheet"));
    type_text(&mut p, "nope");
    key(&mut p, Key::Enter);
    // Excel's message box over the prompt, which stays.
    assert_eq!(title(&p).as_deref(), Some("Open365"));
    assert!(
        dump(&p)
            .iter()
            .any(|l| l.starts_with("The password you supplied is not correct."))
    );
    key(&mut p, Key::Enter);
    assert_eq!(title(&p).as_deref(), Some("Unprotect Sheet"));
    assert!(p.shell.sheet().review_sheet_protection().is_some());
    for _ in 0..4 {
        key(&mut p, Key::Backspace);
    }
    type_text(&mut p, "secret");
    key(&mut p, Key::Enter);
    assert!(!is_open(&p));
    assert!(p.shell.sheet().review_sheet_protection().is_none());

    // No password: the button unprotects at once.
    p.shell
        .sheet_mut()
        .review_protect_sheet(Default::default(), None)
        .unwrap();
    assert!(!bare(&mut p, "review.protect.protect-sheet"));
    assert!(p.shell.sheet().review_sheet_protection().is_none());
}

#[test]
fn protect_workbook_with_a_password_confirms_it_and_unprotect_asks_for_it() {
    let mut p = sheets();
    assert!(bare(&mut p, "review.protect.protect-workbook"));
    assert!(!p.shell.sheet().review_workbook_protected());
    type_text(&mut p, "pw");
    key(&mut p, Key::Enter);
    assert_eq!(title(&p).as_deref(), Some("Confirm Password"));
    type_text(&mut p, "pw");
    key(&mut p, Key::Enter);
    assert!(!is_open(&p));
    assert!(p.shell.sheet().review_workbook_protected());
    assert!(bare(&mut p, "review.protect.protect-workbook"));
    assert_eq!(title(&p).as_deref(), Some("Unprotect Workbook"));
    type_text(&mut p, "pw");
    key(&mut p, Key::Enter);
    assert!(!is_open(&p));
    assert!(!p.shell.sheet().review_workbook_protected());
}

// ---- Format Cells: angle, strikethrough, shrink to fit and borders ---------------------------------

#[test]
fn format_cells_takes_any_text_angle_and_shrink_to_fit() {
    let mut p = sheets();
    p.shell.sheet_mut().select_cell((2, 2));
    assert!(p.shell.open_dialog(
        &mut p.ui,
        DialogRequest::FormatCells {
            tab: FormatTab::Alignment
        }
    ));
    alt(&mut p, 'k'); // Shrink to fit
    alt(&mut p, 'e'); // Degrees
    type_text(&mut p, "-37");
    key(&mut p, Key::Enter);
    assert!(!is_open(&p), "{:?}", dump(&p));
    let f = p.shell.sheet().home_cell_format(2, 2).unwrap();
    assert!(f.alignment.shrink_to_fit);
    assert_eq!(f.alignment.text_rotation, 127, "37 degrees clockwise");

    // Wrap text and shrink to fit exclude each other.
    assert!(p.shell.open_dialog(
        &mut p.ui,
        DialogRequest::FormatCells {
            tab: FormatTab::Alignment
        }
    ));
    alt(&mut p, 'w');
    alt(&mut p, 'e');
    type_text(&mut p, "95");
    key(&mut p, Key::Enter);
    assert!(
        is_open(&p),
        "an angle outside -90..90 keeps the dialog open"
    );
    assert!(dump(&p).contains(&"The number must be between -90 and 90.".to_owned()));
    alt(&mut p, 'e');
    type_text(&mut p, "90");
    key(&mut p, Key::Enter);
    assert!(!is_open(&p));
    let f = p.shell.sheet().home_cell_format(2, 2).unwrap();
    assert!(f.alignment.wrap_text && !f.alignment.shrink_to_fit);
    assert_eq!(f.alignment.text_rotation, 90);
}

#[test]
fn format_cells_font_tab_sets_strikethrough() {
    let mut p = sheets();
    p.shell.sheet_mut().select_cell((2, 2));
    assert!(p.shell.open_dialog(
        &mut p.ui,
        DialogRequest::FormatCells {
            tab: FormatTab::Font
        }
    ));
    alt(&mut p, 'k');
    key(&mut p, Key::Enter);
    assert!(!is_open(&p));
    assert!(p.shell.sheet().home_cell_format(2, 2).unwrap().font.strike);
}

#[test]
fn format_cells_border_tab_draws_chosen_styles_and_colours_on_the_clicked_edges() {
    use open365_xlsx::BorderStyle;
    let mut p = sheets();
    p.shell.sheet_mut().select_cell((2, 2));
    assert!(p.shell.open_dialog(
        &mut p.ui,
        DialogRequest::FormatCells {
            tab: FormatTab::Border
        }
    ));
    key(&mut p, Key::Tab); // the style list, on Thin
    assert_eq!(focus_label(&p), "Thin");
    for _ in 0..6 {
        key(&mut p, Key::Down); // Thin -> Thick
    }
    assert_eq!(focus_label(&p), "Thick");
    key(&mut p, Key::Tab); // colour
    keys(&mut p, &[Key::Down, Key::Down, Key::Down]); // Automatic, Black, White, Red
    alt(&mut p, 'o'); // Outline
    let diag = find(&p, "\\").expect("diagonal button");
    click(&mut p, diag);
    // Pressing the top edge again removes the line the outline drew.
    let top = find(&p, "Top").expect("top button");
    click(&mut p, top);
    press_ok(&mut p);
    assert!(!is_open(&p), "{:?}", dump(&p));
    let b = p.shell.sheet().home_cell_format(2, 2).unwrap().border;
    assert_eq!(b.left.style, BorderStyle::Thick);
    assert_eq!(b.right.style, BorderStyle::Thick);
    assert_eq!(b.bottom.style, BorderStyle::Thick);
    assert_eq!(
        b.top.style,
        BorderStyle::None,
        "the second press removed it"
    );
    assert!(matches!(
        b.left.color.source,
        ColorSource::Rgb(c) if (c.r, c.g, c.b) == (255, 0, 0)
    ));
    assert!(b.diagonal_down && b.diagonal_down_edge().style == BorderStyle::Thick);
}

#[test]
fn format_cells_border_tab_none_preset_clears_and_inside_needs_a_range() {
    use open365_xlsx::BorderStyle;
    let mut p = sheets();
    p.shell.sheet_mut().select_cell((2, 2));
    p.shell.run_command(&mut p.ui, "home.font.borders.all");
    assert!(p.shell.open_dialog(
        &mut p.ui,
        DialogRequest::FormatCells {
            tab: FormatTab::Border
        }
    ));
    let enabled = |p: &open365_preview::Preview, label: &str| -> bool {
        let id = find(p, label).expect(label);
        p.ui.tree.get(id).unwrap().enabled
    };
    assert!(
        !enabled(&p, "Inside Horizontal Border"),
        "one cell has no inside lines"
    );
    alt(&mut p, 'n');
    press_ok(&mut p);
    assert!(!is_open(&p));
    let b = p.shell.sheet().home_cell_format(2, 2).unwrap().border;
    assert_eq!(b.left.style, BorderStyle::None);
    assert_eq!(b.top.style, BorderStyle::None);

    // Over a range the inside lines can be drawn.
    p.shell.sheet_mut().select_cell((2, 2));
    p.shell.sheet_mut().extend_to((3, 3));
    assert!(p.shell.open_dialog(
        &mut p.ui,
        DialogRequest::FormatCells {
            tab: FormatTab::Border
        }
    ));
    assert!(enabled(&p, "Inside Horizontal Border"));
    alt(&mut p, 'i');
    press_ok(&mut p);
    assert!(!is_open(&p));
    let inner = p.shell.sheet().home_cell_format(2, 2).unwrap().border;
    assert_eq!(inner.bottom.style, BorderStyle::Thin);
    assert_eq!(inner.right.style, BorderStyle::Thin);
}

// ---- Page Setup: Header/Footer --------------------------------------------------------------------

#[test]
fn page_setup_header_footer_tab_picks_presets_and_edits_custom_sections() {
    use open365_preview::dialogs::request::PageTab;
    let mut p = sheets();
    assert!(p.shell.open_dialog(
        &mut p.ui,
        DialogRequest::PageSetup {
            tab: PageTab::HeaderFooter
        }
    ));
    key(&mut p, Key::Tab); // Header list
    keys(&mut p, &[Key::Down, Key::Down]); // (none), Page 1, Page 1 of ?
    assert!(
        dump(&p).contains(&"Page 1 of 1".to_owned()),
        "{:?}",
        dump(&p)
    );
    // Custom Header opens over Page Setup, on the centre section.
    alt(&mut p, 'u');
    assert_eq!(title(&p).as_deref(), Some("Header"));
    key(&mut p, Key::End);
    type_text(&mut p, " draft ");
    let button = find(&p, "Sheet Name").expect("insert button");
    click(&mut p, button);
    key(&mut p, Key::Enter);
    assert_eq!(title(&p).as_deref(), Some("Page Setup"));
    // The edited text is a custom entry now, shown in the preview.
    assert!(
        dump(&p).iter().any(|l| l.contains("draft Sheet1")),
        "{:?}",
        dump(&p)
    );
    // The footer takes a preset too.
    alt(&mut p, 'f');
    key(&mut p, Key::Down);
    key(&mut p, Key::Enter);
    assert!(!is_open(&p));
    let saved = p.shell.sheet().page().header_footer;
    assert_eq!(saved.header[1], "Page &P of &N draft &A");
    assert_eq!(saved.footer[1], "&P");
    assert!(saved.scale_with_document && saved.align_with_margins);
}

#[test]
fn header_footer_is_written_to_the_file_printed_and_undone() {
    use open365_preview::dialogs::header_footer::HeaderFooter;
    use open365_preview::sheet::SheetModel;
    let mut p = sheets();
    p.shell.sheet_mut().enter(1, 1, "x").unwrap();
    let mut hf = HeaderFooter {
        header: ["Left".into(), String::new(), "Right && co".into()],
        ..HeaderFooter::default()
    };
    hf.footer[1] = "Page &P of &N".into();
    hf.first_footer[1] = "Cover".into();
    hf.align_with_margins = false;
    p.shell.sheet_mut().set_header_footer(0, &hf).unwrap();
    assert_eq!(p.shell.sheet().page().header_footer, hf);
    // The PDF prints the footer with the page numbers filled in, and the header.
    let pdf = String::from_utf8_lossy(&p.shell.sheet().pdf_bytes()).into_owned();
    assert!(pdf.contains("(Page 1 of 1)"), "{pdf}");
    assert!(pdf.contains("(Right & co)"));
    assert!(!pdf.contains("(Cover)"));
    // "Different first page" prints the first-page footer on page 1 instead.
    hf.different_first = true;
    p.shell.sheet_mut().set_header_footer(0, &hf).unwrap();
    let pdf = String::from_utf8_lossy(&p.shell.sheet().pdf_bytes()).into_owned();
    assert!(pdf.contains("(Cover)") && !pdf.contains("(Page 1 of 1)"));
    // It survives save and reopen.
    let path = std::env::temp_dir().join(format!("o365-hf-{}.xlsx", std::process::id()));
    p.shell.sheet_mut().save(&path).unwrap();
    let back = SheetModel::open(&path).unwrap();
    std::fs::remove_file(&path).ok();
    assert_eq!(back.page().header_footer, hf);
    // One undo step each.
    assert!(p.shell.sheet_mut().undo());
    assert!(!p.shell.sheet().page().header_footer.different_first);
    assert!(p.shell.sheet_mut().undo());
    assert_eq!(
        p.shell.sheet().page().header_footer,
        HeaderFooter::default()
    );
}
