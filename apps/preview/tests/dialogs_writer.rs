//! The Writer dialogs (Page Setup, Columns, Line Numbers, Hyphenation, Watermark, Page Borders,
//! Zoom, Paragraph): each opens, takes keyboard input and OK applies the effect through the
//! existing `=value` command forms.

#![allow(clippy::unwrap_used)]

mod dialogs_common;

use dialogs_common::*;
use open365_docx::Document;
use open365_opc::Limits;
use open365_preview::Preview;
use open365_ui_core::Key;

fn saved_xml(p: &mut Preview, name: &str) -> String {
    let path = std::env::temp_dir().join(format!("o365-dlg-{}-{name}.docx", std::process::id()));
    assert!(p.shell.writer_mut().unwrap().save_to(path.clone()), "save");
    let bytes = std::fs::read(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    String::from_utf8(
        Document::open(&bytes, &Limits::default())
            .unwrap()
            .serialize_main(),
    )
    .unwrap()
}

#[test]
fn page_setup_margins_take_typed_lengths() {
    let mut p = writer("hello");
    assert!(bare(&mut p, "layout.page-set-up.launcher"));
    assert_eq!(title(&p).as_deref(), Some("Page Setup"));
    // Opens on the tab strip; Tab goes to the Top margin.
    key(&mut p, Key::Tab);
    assert_eq!(focus_label(&p), "Top");
    type_text(&mut p, "2\"");
    key(&mut p, Key::Enter);
    assert!(!is_open(&p), "{:?}", p.shell.writer().unwrap().status);
    let m = p.shell.writer().unwrap().caret_section().unwrap().margins;
    assert_eq!(m.top_twips, 2880);
}

#[test]
fn page_setup_orientation_and_paper() {
    let mut p = writer("hello");
    assert!(bare(&mut p, "layout.page-set-up.margins.custom"));
    alt(&mut p, 't'); // Landscape's access key is D; T is Top
    alt(&mut p, 'd');
    key(&mut p, Key::Enter);
    assert!(!is_open(&p));
    let s = p.shell.writer().unwrap().caret_section().unwrap();
    assert_eq!(s.page.orient, open365_docx::Orientation::Landscape);
}

#[test]
fn columns_dialog_makes_three_columns() {
    let mut p = writer("hello");
    assert!(bare(&mut p, "layout.page-set-up.columns.more"));
    assert_eq!(title(&p).as_deref(), Some("Columns"));
    alt(&mut p, 'n'); // Number of columns
    type_text(&mut p, "3");
    key(&mut p, Key::Enter);
    assert!(!is_open(&p), "{:?}", p.shell.writer().unwrap().status);
    assert_eq!(
        p.shell
            .writer()
            .unwrap()
            .caret_section()
            .unwrap()
            .columns
            .count,
        3
    );
}

#[test]
fn columns_preset_two() {
    let mut p = writer("hello");
    assert!(bare(&mut p, "layout.page-set-up.columns.more"));
    alt(&mut p, 't'); // Two
    key(&mut p, Key::Enter);
    assert_eq!(
        p.shell
            .writer()
            .unwrap()
            .caret_section()
            .unwrap()
            .columns
            .count,
        2
    );
}

#[test]
fn line_numbers_dialog_adds_numbering() {
    let mut p = writer("hello");
    assert!(bare(&mut p, "layout.page-set-up.line-numbers.options"));
    assert_eq!(title(&p).as_deref(), Some("Line Numbers"));
    key(&mut p, Key::Enter);
    assert!(!is_open(&p), "{:?}", p.shell.writer().unwrap().status);
    assert!(saved_xml(&mut p, "ln").contains("lnNumType"));
}

#[test]
fn hyphenation_dialog_turns_automatic_hyphenation_on() {
    let mut p = writer("hello");
    assert!(!p.shell.writer().unwrap().hyphenation_on());
    assert!(bare(&mut p, "layout.page-set-up.hyphenation.options"));
    assert_eq!(title(&p).as_deref(), Some("Hyphenation"));
    alt(&mut p, 'a'); // Automatically hyphenate document
    key(&mut p, Key::Enter);
    assert!(!is_open(&p), "{:?}", p.shell.writer().unwrap().status);
    assert!(p.shell.writer().unwrap().hyphenation_on());
}

#[test]
fn watermark_dialog_sets_and_removes_text() {
    let mut p = writer("hello");
    assert!(bare(&mut p, "design.page-background.watermark.custom"));
    assert_eq!(title(&p).as_deref(), Some("Printed Watermark"));
    alt(&mut p, 't'); // Text watermark
    alt(&mut p, 'x'); // Text
    type_text(&mut p, "TOP SECRET");
    key(&mut p, Key::Enter);
    assert!(!is_open(&p), "{:?}", p.shell.writer().unwrap().status);
    assert_eq!(
        p.shell.writer().unwrap().watermark_text().as_deref(),
        Some("TOP SECRET")
    );

    assert!(bare(&mut p, "design.page-background.watermark.custom"));
    alt(&mut p, 'n'); // No watermark
    key(&mut p, Key::Enter);
    assert!(p.shell.writer().unwrap().watermark_text().is_none());
}

#[test]
fn page_borders_dialog_draws_a_box() {
    let mut p = writer("hello");
    assert!(bare(&mut p, "design.page-background.page-borders"));
    assert_eq!(title(&p).as_deref(), Some("Borders and Shading"));
    alt(&mut p, 'b'); // Box
    key(&mut p, Key::Enter);
    assert!(!is_open(&p), "{:?}", p.shell.writer().unwrap().status);
    assert!(saved_xml(&mut p, "pb").contains("pgBorders"));
}

#[test]
fn zoom_dialog_sets_the_zoom() {
    let mut p = writer("hello");
    assert!(bare(&mut p, "view.zoom.zoom"));
    assert_eq!(title(&p).as_deref(), Some("Zoom"));
    key(&mut p, Key::Up); // 100% -> 200%
    key(&mut p, Key::Enter);
    assert!(!is_open(&p));
    assert!((p.shell.zoom() - 200.0).abs() < 0.5, "{}", p.shell.zoom());
}

#[test]
fn paragraph_dialog_aligns_and_indents() {
    let mut p = writer("hello");
    assert!(bare(&mut p, "home.paragraph.launcher"));
    assert_eq!(title(&p).as_deref(), Some("Paragraph"));
    key(&mut p, Key::Tab); // from the tab strip to Alignment
    key(&mut p, Key::Down); // Centered
    alt(&mut p, 'l'); // Left indent
    type_text(&mut p, "0.5\"");
    key(&mut p, Key::Enter);
    assert!(!is_open(&p), "{:?}", p.shell.writer().unwrap().status);
    assert_eq!(
        p.shell.writer().unwrap().alignment_state(),
        Some(open365_docx::Alignment::Center)
    );
    assert!(saved_xml(&mut p, "para").contains("w:left=\"720\""));
}

#[test]
fn escape_cancels_every_writer_dialog_without_changes() {
    let mut p = writer("hello");
    for id in [
        "layout.page-set-up.launcher",
        "layout.page-set-up.columns.more",
        "layout.page-set-up.line-numbers.options",
        "layout.page-set-up.hyphenation.options",
        "design.page-background.watermark.custom",
        "design.page-background.page-borders",
        "view.zoom.zoom",
        "home.paragraph.launcher",
    ] {
        assert!(bare(&mut p, id), "{id}");
        key(&mut p, Key::Escape);
        assert!(!is_open(&p), "{id}");
    }
    assert!(p.shell.writer().unwrap().watermark_text().is_none());
}

#[test]
fn paragraph_dialog_sets_first_line_and_hanging_indents() {
    let mut p = writer("hello");
    assert!(bare(&mut p, "home.paragraph.launcher"));
    // Special: First line fills By with half an inch, as Word does.
    alt(&mut p, 's');
    key(&mut p, Key::Down);
    assert_eq!(focus_label(&p), "Special");
    key(&mut p, Key::Enter);
    assert!(!is_open(&p), "{:?}", p.shell.writer().unwrap().status);
    let xml = saved_xml(&mut p, "firstline");
    assert!(xml.contains("w:firstLine=\"720\""), "{xml}");

    // Hanging with a 1" left indent: the first line starts half an inch to the left of it.
    assert!(bare(&mut p, "home.paragraph.launcher"));
    alt(&mut p, 'l');
    type_text(&mut p, "1\"");
    alt(&mut p, 's');
    key(&mut p, Key::Down);
    key(&mut p, Key::Enter);
    assert!(!is_open(&p), "{:?}", p.shell.writer().unwrap().status);
    let xml = saved_xml(&mut p, "hanging");
    assert!(xml.contains("w:left=\"1440\""), "{xml}");
    assert!(xml.contains("w:hanging=\"720\""), "{xml}");

    // The box shows what the paragraph has, and none empties By.
    assert!(bare(&mut p, "home.paragraph.launcher"));
    alt(&mut p, 's');
    key(&mut p, Key::Up);
    key(&mut p, Key::Up);
    key(&mut p, Key::Enter);
    assert!(!is_open(&p));
    let xml = saved_xml(&mut p, "none");
    assert!(!xml.contains("w:hanging"), "{xml}");
}

#[test]
fn paragraph_dialog_refuses_a_bad_indent_and_keeps_the_outline_level() {
    let mut p = writer("hello");
    assert!(bare(&mut p, "home.paragraph.launcher"));
    alt(&mut p, 'l');
    type_text(&mut p, "wide");
    key(&mut p, Key::Enter);
    assert!(is_open(&p));
    assert!(dump(&p).contains(&"Enter an indent such as 0.5\" or 1.27 cm.".to_owned()));
    alt(&mut p, 'l');
    type_text(&mut p, "0\"");
    // Outline level is part of the dialog.
    alt(&mut p, 'o');
    key(&mut p, Key::Down);
    key(&mut p, Key::Enter);
    assert!(!is_open(&p), "{:?}", dump(&p));
    let xml = saved_xml(&mut p, "none");
    assert!(xml.contains(r#"<w:outlineLvl w:val="0"/>"#), "{xml}");
    // The dialog opens on the level and Body Text takes it off again; one undo step each.
    assert!(bare(&mut p, "home.paragraph.launcher"));
    alt(&mut p, 'o');
    key(&mut p, Key::Up);
    key(&mut p, Key::Enter);
    assert!(!is_open(&p));
    let xml = saved_xml(&mut p, "none");
    assert!(!xml.contains("outlineLvl"), "{xml}");
}
