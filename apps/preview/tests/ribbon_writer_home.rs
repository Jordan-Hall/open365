//! Writer Home tab controls run through `Shell::run_command` and change the document.

#![allow(clippy::unwrap_used)]

use std::sync::atomic::{AtomicUsize, Ordering};

use open365_docx::{Alignment, DocPos, Document};
use open365_opc::Limits;
use open365_preview::doc::{Toggle, WriterPane};
use open365_preview::ribbon::Handled;
use open365_preview::{AppKind, Preview};

fn rig(text: &str) -> Preview {
    let mut p = Preview::new(AppKind::Writer, false, 1600, 1000, 1.25);
    let mut pane = WriterPane::blank();
    pane.insert_text(text);
    p.shell.set_writer(&mut p.ui, pane);
    p
}

fn run(p: &mut Preview, id: &str) {
    assert_eq!(p.shell.run_command(&mut p.ui, id), Handled::Yes, "{id}");
}

fn pane(p: &mut Preview) -> &mut WriterPane {
    p.shell.writer_mut().unwrap()
}

static N: AtomicUsize = AtomicUsize::new(0);

/// The saved document, reopened.
fn saved(p: &mut Preview) -> Document {
    let path = std::env::temp_dir().join(format!(
        "open365-home-{}-{}.docx",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    assert!(pane(p).save_to(path.clone()), "save");
    let bytes = std::fs::read(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    Document::open(&bytes, &Limits::default()).unwrap()
}

fn xml(p: &mut Preview) -> String {
    String::from_utf8(saved(p).serialize_main()).unwrap()
}

fn select(p: &mut Preview, a: (usize, usize), b: (usize, usize)) {
    pane(p).select(DocPos::new(a.0, a.1), DocPos::new(b.0, b.1));
}

fn undo(p: &mut Preview) {
    assert!(pane(p).undo());
}

fn multi(lines: &[&str]) -> Preview {
    let mut p = rig("");
    pane(&mut p).paste_text(&lines.join("\n"));
    p
}

#[test]
fn clipboard_copy_cut_paste() {
    let mut p = rig("hello world");
    select(&mut p, (0, 0), (0, 5));
    run(&mut p, "home.clipboard.copy");
    assert_eq!(pane(&mut p).text(), "hello world");
    run(&mut p, "home.clipboard.cut");
    assert_eq!(pane(&mut p).text(), " world");
    pane(&mut p).set_caret(DocPos::new(0, 6));
    run(&mut p, "home.clipboard.paste");
    assert_eq!(pane(&mut p).text(), " worldhello");
    undo(&mut p);
    assert_eq!(pane(&mut p).text(), " world");
    // The paste menu entries paste text too.
    run(&mut p, "home.clipboard.paste.values");
    assert_eq!(pane(&mut p).text(), " worldhello");
}

#[test]
fn format_painter_copies_run_formatting() {
    let mut p = rig("bold plain");
    select(&mut p, (0, 0), (0, 4));
    run(&mut p, "home.font.font-colour=00AA00");
    run(&mut p, "home.font.bold");
    pane(&mut p).set_caret(DocPos::new(0, 2));
    run(&mut p, "home.clipboard.format-painter");
    assert!(pane(&mut p).home_state().painter.is_some());
    select(&mut p, (0, 5), (0, 10));
    run(&mut p, "home.clipboard.format-painter");
    assert!(pane(&mut p).home_state().painter.is_none());
    let x = xml(&mut p);
    assert_eq!(x.matches("00AA00").count(), 2, "both runs are green: {x}");
    undo(&mut p);
    assert_eq!(xml(&mut p).matches("00AA00").count(), 1);
}

#[test]
fn font_controls_change_runs() {
    let mut p = rig("hello world");
    select(&mut p, (0, 0), (0, 5));
    run(&mut p, "home.font.bold");
    assert!(pane(&mut p).toggle_state(Toggle::Bold));
    run(&mut p, "home.font.italic");
    run(&mut p, "home.font.underline");
    run(&mut p, "home.font.strikethrough");
    run(&mut p, "home.font.subscript");
    let x = xml(&mut p);
    for tag in ["<w:b/>", "<w:i/>", "<w:u ", "<w:strike/>", "w:vertAlign"] {
        assert!(x.contains(tag), "{tag} in {x}");
    }
    run(&mut p, "home.font.superscript");
    assert!(xml(&mut p).contains("superscript"));
    run(&mut p, "home.font.font=Arial");
    run(&mut p, "home.font.font-size=20");
    assert_eq!(pane(&mut p).font_state(), Some(("Arial".to_owned(), 20.0)));
}

#[test]
fn grow_and_shrink_follow_word_sizes() {
    let mut p = rig("hello");
    select(&mut p, (0, 0), (0, 5));
    run(&mut p, "home.font.font-size=11");
    run(&mut p, "home.font.grow-font");
    assert_eq!(pane(&mut p).font_state().unwrap().1, 12.0);
    run(&mut p, "home.font.grow-font");
    assert_eq!(pane(&mut p).font_state().unwrap().1, 14.0);
    run(&mut p, "home.font.shrink-font");
    run(&mut p, "home.font.shrink-font");
    assert_eq!(pane(&mut p).font_state().unwrap().1, 11.0);
    run(&mut p, "home.font.shrink-font");
    assert_eq!(
        pane(&mut p).font_state().unwrap().1,
        10.5,
        "the list has 10.5 between 10 and 11"
    );
}

#[test]
fn colours_and_highlight() {
    let mut p = rig("hello world");
    select(&mut p, (0, 0), (0, 5));
    run(&mut p, "home.font.font-colour");
    assert!(
        xml(&mut p).contains("FF0000"),
        "the button applies red at first"
    );
    run(&mut p, "home.font.font-colour=0070C0");
    assert!(xml(&mut p).contains("0070C0"));
    run(&mut p, "home.font.text-highlight-colour");
    let x = xml(&mut p);
    assert!(x.contains("<w:highlight w:val=\"yellow\"/>"), "{x}");
    run(&mut p, "home.font.text-highlight-colour=none");
    assert!(!xml(&mut p).contains("w:highlight"));
    undo(&mut p);
    assert!(xml(&mut p).contains("w:highlight"));
}

#[test]
fn underline_styles() {
    let mut p = rig("hello");
    select(&mut p, (0, 0), (0, 5));
    run(&mut p, "home.font.underline.double");
    assert!(xml(&mut p).contains("<w:u w:val=\"double\"/>"));
}

#[test]
fn change_case_modes_keep_the_formatting() {
    let mut p = rig("hello world. second one");
    select(&mut p, (0, 0), (0, 5));
    run(&mut p, "home.font.bold");
    select(&mut p, (0, 0), (0, 23));
    run(&mut p, "home.font.change-case.uppercase");
    assert_eq!(pane(&mut p).text(), "HELLO WORLD. SECOND ONE");
    assert!(xml(&mut p).contains("<w:b/>"), "the bold run stays");
    run(&mut p, "home.font.change-case.lowercase");
    assert_eq!(pane(&mut p).text(), "hello world. second one");
    run(&mut p, "home.font.change-case.sentence-case");
    assert_eq!(pane(&mut p).text(), "Hello world. Second one");
    run(&mut p, "home.font.change-case.capitalize-each-word");
    assert_eq!(pane(&mut p).text(), "Hello World. Second One");
    run(&mut p, "home.font.change-case.toggle-case");
    assert_eq!(pane(&mut p).text(), "hELLO wORLD. sECOND oNE");
    undo(&mut p);
    assert_eq!(pane(&mut p).text(), "Hello World. Second One");
}

#[test]
fn change_case_without_selection_takes_the_word() {
    let mut p = rig("one two three");
    pane(&mut p).set_caret(DocPos::new(0, 5));
    run(&mut p, "home.font.change-case=uppercase");
    assert_eq!(pane(&mut p).text(), "one TWO three");
}

#[test]
fn clear_formatting_resets_text_and_paragraph() {
    let mut p = rig("hello world");
    pane(&mut p).select_all();
    run(&mut p, "home.font.bold");
    run(&mut p, "home.styles.heading-1");
    run(&mut p, "home.paragraph.centre");
    run(&mut p, "home.font.clear-formatting");
    let x = xml(&mut p);
    assert!(!x.contains("<w:b/>"), "{x}");
    assert!(!x.contains("Heading1"), "{x}");
    assert!(!x.contains("w:jc"), "{x}");
    assert_eq!(pane(&mut p).text(), "hello world");
    undo(&mut p);
    assert!(xml(&mut p).contains("Heading1"));
}

#[test]
fn bullets_and_numbering_toggle_lists() {
    let mut p = multi(&["one", "two", "three"]);
    pane(&mut p).select_all();
    run(&mut p, "home.paragraph.bullets");
    let x = xml(&mut p);
    assert_eq!(x.matches("<w:numPr>").count(), 3, "{x}");
    assert!(x.contains("ListParagraph"));
    // The same button takes the list off.
    run(&mut p, "home.paragraph.bullets");
    assert!(!xml(&mut p).contains("<w:numPr>"));
    run(&mut p, "home.paragraph.numbering");
    let doc = saved(&mut p);
    let n = doc.numbering().expect("numbering part");
    assert!(n.abstract_nums().count() >= 1);
    assert!(
        String::from_utf8(doc.serialize_main())
            .unwrap()
            .matches("<w:numPr>")
            .count()
            == 3
    );
    run(&mut p, "home.paragraph.multi-level-list");
    assert!(xml(&mut p).contains("<w:numPr>"));
    undo(&mut p);
    assert!(xml(&mut p).contains("<w:numPr>"));
}

#[test]
fn indent_steps_are_half_an_inch() {
    let mut p = rig("hello");
    run(&mut p, "home.paragraph.increase-indent");
    run(&mut p, "home.paragraph.increase-indent");
    assert!(xml(&mut p).contains("w:left=\"1440\""));
    run(&mut p, "home.paragraph.decrease-indent");
    assert!(xml(&mut p).contains("w:left=\"720\""));
    run(&mut p, "home.paragraph.decrease-indent");
    run(&mut p, "home.paragraph.decrease-indent");
    assert!(xml(&mut p).contains("w:left=\"0\""));
}

#[test]
fn alignment_buttons() {
    let mut p = rig("hello");
    for (id, a) in [
        ("home.paragraph.centre", Alignment::Center),
        ("home.paragraph.align-right", Alignment::Right),
        ("home.paragraph.justify", Alignment::Justify),
        ("home.paragraph.align-left", Alignment::Left),
    ] {
        run(&mut p, id);
        assert_eq!(pane(&mut p).alignment_state(), Some(a), "{id}");
    }
}

#[test]
fn line_and_paragraph_spacing() {
    let mut p = rig("hello");
    run(&mut p, "home.paragraph.line-and-paragraph-spacing=1.5");
    let x = xml(&mut p);
    assert!(
        x.contains("w:line=\"360\"") && x.contains("w:lineRule=\"auto\""),
        "{x}"
    );
    run(
        &mut p,
        "home.paragraph.line-and-paragraph-spacing.add-space-before",
    );
    assert!(xml(&mut p).contains("w:before=\"240\""));
    run(
        &mut p,
        "home.paragraph.line-and-paragraph-spacing.remove-space-after",
    );
    let x = xml(&mut p);
    assert!(
        x.contains("w:after=\"0\"") && x.contains("w:line=\"360\""),
        "{x}"
    );
    undo(&mut p);
    assert!(!xml(&mut p).contains("w:after=\"0\""));
}

#[test]
fn shading_and_borders() {
    let mut p = rig("hello");
    run(&mut p, "home.paragraph.shading=FFFF00");
    assert!(xml(&mut p).contains("w:fill=\"FFFF00\""));
    run(&mut p, "home.paragraph.shading=none");
    assert!(!xml(&mut p).contains("<w:shd"));
    run(&mut p, "home.paragraph.borders");
    let x = xml(&mut p);
    assert!(
        x.contains("<w:pBdr><w:bottom "),
        "bottom border by default: {x}"
    );
    run(&mut p, "home.paragraph.borders.bottom");
    assert!(!xml(&mut p).contains("w:pBdr"), "the same side toggles off");
    run(&mut p, "home.paragraph.borders=all");
    let x = xml(&mut p);
    for side in ["<w:top ", "<w:left ", "<w:bottom ", "<w:right "] {
        assert!(x.contains(side), "{side}");
    }
    run(&mut p, "home.paragraph.borders=none");
    assert!(!xml(&mut p).contains("w:pBdr"));
}

#[test]
fn sort_orders_paragraphs() {
    let mut p = multi(&["pear", "Apple", "fig"]);
    pane(&mut p).select_all();
    run(&mut p, "home.paragraph.sort");
    assert_eq!(pane(&mut p).text(), "Apple\nfig\npear");
    run(&mut p, "home.paragraph.sort=descending");
    assert_eq!(pane(&mut p).text(), "pear\nfig\nApple");
    undo(&mut p);
    assert_eq!(pane(&mut p).text(), "Apple\nfig\npear");
}

#[test]
fn show_all_toggles() {
    let mut p = rig("hello");
    run(&mut p, "home.paragraph.show-all");
    assert!(pane(&mut p).home_state().show_all);
    run(&mut p, "home.paragraph.show-all");
    assert!(!pane(&mut p).home_state().show_all);
}

#[test]
fn styles_gallery_and_names() {
    let mut p = rig("hello");
    run(&mut p, "home.styles.heading-2");
    assert_eq!(pane(&mut p).style_state().as_deref(), Some("Heading2"));
    run(&mut p, "home.styles.quickstylesgallery=Title");
    assert_eq!(pane(&mut p).style_state().as_deref(), Some("Title"));
    run(&mut p, "home.styles.normal");
    assert_eq!(pane(&mut p).style_state().as_deref(), Some("Normal"));
}

#[test]
fn editing_find_replace_select() {
    let mut p = rig("hello world");
    run(&mut p, "home.editing.find");
    assert!(pane(&mut p).find_is_open());
    run(&mut p, "home.editing.replace");
    assert!(pane(&mut p).replace_is_open());
    pane(&mut p).set_caret(DocPos::new(0, 0));
    run(&mut p, "home.editing.select.select-all");
    assert!(pane(&mut p).has_selection());
}

#[test]
fn every_wired_home_control_answers() {
    let mut p = rig("hello");
    for id in open365_preview::ribbon::writer::home::IDS {
        assert_eq!(p.shell.run_command(&mut p.ui, id), Handled::Yes, "{id}");
    }
}
