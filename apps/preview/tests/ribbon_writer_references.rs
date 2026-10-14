//! Writer References tab controls run through `Shell::run_command`.

#![allow(clippy::unwrap_used)]

use open365_docx::DocPos;
use open365_preview::doc::{Format, WriterPane};
use open365_preview::ribbon::Handled;
use open365_preview::{AppKind, Preview};

fn style(pane: &mut WriterPane, id: &str) {
    pane.format(Format::Style(id.to_owned()));
}

/// Heading 1 "Intro", body text, Heading 2 "Details", body text; the caret ends in the last paragraph.
fn rig() -> Preview {
    let mut p = Preview::new(AppKind::Writer, false, 1600, 1000, 1.25);
    let mut pane = WriterPane::blank();
    pane.insert_text("Intro");
    style(&mut pane, "Heading1");
    pane.enter(false);
    style(&mut pane, "Normal");
    pane.insert_text("Some body text about apples.");
    pane.enter(false);
    pane.insert_text("Details");
    style(&mut pane, "Heading2");
    pane.enter(false);
    style(&mut pane, "Normal");
    pane.insert_text("More body text.");
    p.shell.set_writer(&mut p.ui, pane);
    p
}

fn run(p: &mut Preview, id: &str) {
    assert_eq!(p.shell.run_command(&mut p.ui, id), Handled::Yes, "{id}");
}

fn pane(p: &mut Preview) -> &mut WriterPane {
    p.shell.writer_mut().unwrap()
}

fn paragraphs(p: &mut Preview) -> Vec<String> {
    let pane = pane(p);
    (0..pane.paragraph_count())
        .map(|i| pane.paragraph_text(i).unwrap().to_owned())
        .collect()
}

#[test]
fn table_of_contents_lists_the_headings_with_pages_and_undoes() {
    let mut p = rig();
    let before = paragraphs(&mut p);
    run(&mut p, "references.table-of-contents.table-of-contents");
    let after = paragraphs(&mut p);
    assert!(after.len() > before.len(), "{after:?}");
    assert!(after.iter().any(|t| t == "Contents"), "title: {after:?}");
    let entries: Vec<&String> = after
        .iter()
        .filter(|t| t.contains("Intro") || t.contains("Details"))
        .collect();
    // the heading itself and its entry
    assert_eq!(entries.len(), 4, "{after:?}");
    assert!(
        entries
            .iter()
            .any(|t| t.starts_with("Intro") && t.ends_with('1')),
        "entry with page: {after:?}"
    );
    assert!(pane(&mut p).undo());
    assert_eq!(paragraphs(&mut p), before);
}

#[test]
fn remove_table_of_contents_takes_the_table_and_title_out() {
    let mut p = rig();
    let before = paragraphs(&mut p);
    run(&mut p, "references.table-of-contents.table-of-contents");
    let entry = paragraphs(&mut p)
        .iter()
        .rposition(|t| t.starts_with("Intro"))
        .unwrap();
    pane(&mut p).set_caret(DocPos::new(entry, 0));
    run(
        &mut p,
        "references.table-of-contents.table-of-contents.remove",
    );
    assert_eq!(paragraphs(&mut p), before);
}

#[test]
fn update_table_follows_a_new_heading() {
    let mut p = rig();
    run(&mut p, "references.table-of-contents.table-of-contents");
    // Below the last body paragraph, above the table.
    pane(&mut p).set_caret(DocPos::new(3, 15));
    pane(&mut p).enter(false);
    pane(&mut p).insert_text("Appendix");
    style(pane(&mut p), "Heading1");
    assert!(
        !paragraphs(&mut p)
            .iter()
            .any(|t| t.starts_with("Appendix\t") || t.starts_with("Appendix "))
    );
    run(&mut p, "references.table-of-contents.update-table");
    let after = paragraphs(&mut p);
    assert!(
        after.iter().filter(|t| t.contains("Appendix")).count() == 2,
        "entry added: {after:?}"
    );
}

#[test]
fn add_text_sets_the_heading_level() {
    let mut p = rig();
    pane(&mut p).set_caret(DocPos::new(1, 3));
    run(&mut p, "references.table-of-contents.add-text.level-2");
    assert_eq!(pane(&mut p).style_state().as_deref(), Some("Heading2"));
    run(&mut p, "references.table-of-contents.add-text.do-not-show");
    assert_ne!(pane(&mut p).style_state().as_deref(), Some("Heading2"));
}

#[test]
fn footnote_and_endnote_marks_navigate() {
    let mut p = rig();
    pane(&mut p).set_caret(DocPos::new(1, 4));
    run(&mut p, "references.footnotes.insert-footnote");
    pane(&mut p).set_caret(DocPos::new(3, 4));
    run(&mut p, "references.footnotes.insert-endnote");
    let bytes = pane(&mut p).docx_bytes().unwrap();
    let doc = open365_docx::Document::open(&bytes, &open365_opc::Limits::default()).unwrap();
    let marks = doc.note_references();
    assert_eq!(marks.len(), 2, "{marks:?}");
    assert!(!marks[0].endnote && marks[1].endnote);
    assert_eq!(marks[0].pos, DocPos::new(1, 4));
    assert!(
        doc.footnotes().iter().any(|n| n.id == 1),
        "the footnote part holds the note"
    );
    assert!(doc.endnotes().iter().any(|n| n.id == 1));
    // Next Footnote from the start of the document goes to the footnote mark.
    pane(&mut p).set_caret(DocPos::new(0, 0));
    run(&mut p, "references.footnotes.next-footnote");
    assert_eq!(pane(&mut p).caret(), DocPos::new(1, 4));
    assert!(pane(&mut p).undo());
    assert!(pane(&mut p).undo());
}

#[test]
fn caption_is_numbered_and_renumbered() {
    let mut p = rig();
    pane(&mut p).set_caret(DocPos::new(1, 2));
    run(&mut p, "references.captions.insert-caption");
    let t = paragraphs(&mut p);
    assert_eq!(t[2], "Figure 1", "{t:?}");
    // A second caption above the first one makes the first Figure 2.
    pane(&mut p).set_caret(DocPos::new(0, 0));
    run(&mut p, "references.captions.insert-caption");
    let t = paragraphs(&mut p);
    assert_eq!(t[1], "Figure 1", "{t:?}");
    assert!(t.iter().any(|x| x == "Figure 2"), "{t:?}");
}

#[test]
fn table_of_figures_lists_captions() {
    let mut p = rig();
    pane(&mut p).set_caret(DocPos::new(1, 2));
    run(&mut p, "references.captions.insert-caption");
    let end = pane(&mut p).paragraph_count() - 1;
    let len = pane(&mut p).paragraph_text(end).unwrap().chars().count();
    pane(&mut p).set_caret(DocPos::new(end, len));
    run(&mut p, "references.captions.insert-table-of-figures");
    let t = paragraphs(&mut p);
    assert_eq!(
        t.iter().filter(|x| x.starts_with("Figure 1")).count(),
        2,
        "{t:?}"
    );
    run(&mut p, "references.captions.update-table");
}

#[test]
fn index_collects_marked_entries() {
    let mut p = rig();
    // "apples" in the body paragraph.
    pane(&mut p).select(DocPos::new(1, 21), DocPos::new(1, 27));
    run(&mut p, "references.index.mark-entry");
    let end = pane(&mut p).paragraph_count() - 1;
    let len = pane(&mut p).paragraph_text(end).unwrap().chars().count();
    pane(&mut p).set_caret(DocPos::new(end, len));
    run(&mut p, "references.index.insert-index");
    let t = paragraphs(&mut p);
    assert!(t.iter().any(|x| x == "A"), "heading letter: {t:?}");
    assert!(t.iter().any(|x| x == "apples, 1"), "entry: {t:?}");
    // Mark a second entry and update.
    pane(&mut p).select(DocPos::new(0, 0), DocPos::new(0, 5));
    run(&mut p, "references.index.mark-entry");
    run(&mut p, "references.index.update-index");
    let t = paragraphs(&mut p);
    assert!(t.iter().any(|x| x == "Intro, 1"), "new entry: {t:?}");
    assert!(t.iter().any(|x| x == "I"), "{t:?}");
}

#[test]
fn table_of_authorities_lists_citations() {
    let mut p = rig();
    pane(&mut p).select(DocPos::new(1, 0), DocPos::new(1, 4));
    run(&mut p, "references.table-of-authorities.mark-citation");
    let end = pane(&mut p).paragraph_count() - 1;
    let len = pane(&mut p).paragraph_text(end).unwrap().chars().count();
    pane(&mut p).set_caret(DocPos::new(end, len));
    run(
        &mut p,
        "references.table-of-authorities.insert-table-of-authorities",
    );
    let t = paragraphs(&mut p);
    assert!(t.iter().any(|x| x == "Cases"), "{t:?}");
    assert!(
        t.iter()
            .any(|x| x.starts_with("Some") && x.ends_with('1') && x.len() > 4),
        "{t:?}"
    );
    run(&mut p, "references.table-of-authorities.update-table");
}

#[test]
fn controls_answer_without_a_document() {
    let mut p = Preview::new(AppKind::Writer, false, 1600, 1000, 1.25);
    for id in open365_preview::ribbon::writer::references::IDS {
        assert_eq!(p.shell.run_command(&mut p.ui, id), Handled::Yes, "{id}");
    }
}

#[test]
fn every_control_runs_on_a_document() {
    let mut p = rig();
    for id in open365_preview::ribbon::writer::references::IDS {
        for i in [(*id).to_owned(), format!("{id}=12")] {
            assert_eq!(p.shell.run_command(&mut p.ui, &i), Handled::Yes, "{i}");
        }
    }
}

// ---- citations, bibliography, cross-references --------------------------------------------

use open365_docx::{Author, Document, Source, SourceKind};
use open365_opc::Limits;
use std::sync::{Mutex, MutexGuard, PoisonError};

/// The source list and the style are per person (process wide), so these tests take turns.
static TURN: Mutex<()> = Mutex::new(());

fn turn() -> MutexGuard<'static, ()> {
    TURN.lock().unwrap_or_else(PoisonError::into_inner)
}

fn book() -> Source {
    let mut s = Source::new(
        "Doe2020",
        SourceKind::Book,
        vec![Author::new("Doe", "Jane"), Author::new("Roe", "Richard")],
        "Learning Rust",
        "2020",
    );
    s.publisher = "Acme Press".into();
    s
}

fn main_xml(p: &mut Preview) -> String {
    let bytes = pane(p).docx_bytes().unwrap();
    let doc = Document::open(&bytes, &Limits::default()).unwrap();
    String::from_utf8(doc.serialize_main()).unwrap()
}

#[test]
fn insert_citation_adds_a_field_with_the_style_text_and_undoes() {
    let _t = turn();
    let mut p = rig();
    pane(&mut p).add_source(book());
    run(&mut p, "references.citations-bibliography.style=APA");
    let before = paragraphs(&mut p);
    run(
        &mut p,
        "references.citations-bibliography.insert-citation.Doe2020",
    );
    let after = paragraphs(&mut p);
    assert!(
        after.last().unwrap().ends_with("(Doe & Roe, 2020)"),
        "{after:?}"
    );
    assert!(main_xml(&mut p).contains("CITATION Doe2020"));
    assert_eq!(pane(&mut p).cited_sources().len(), 1);
    assert!(pane(&mut p).undo());
    assert_eq!(paragraphs(&mut p), before);
}

#[test]
fn style_box_changes_the_next_citation() {
    let _t = turn();
    let mut p = rig();
    pane(&mut p).add_source(book());
    run(&mut p, "references.citations-bibliography.style=MLA");
    run(
        &mut p,
        "references.citations-bibliography.insert-citation.Doe2020",
    );
    assert!(
        paragraphs(&mut p)
            .last()
            .unwrap()
            .ends_with("(Doe and Roe)"),
        "{:?}",
        paragraphs(&mut p)
    );
    run(&mut p, "references.citations-bibliography.style.chicago");
    assert_eq!(pane(&mut p).bib_style().label(), "Chicago");
    run(&mut p, "references.citations-bibliography.style=APA");
}

#[test]
fn bibliography_lists_cited_sources_in_the_style_and_undoes() {
    let _t = turn();
    let mut p = rig();
    pane(&mut p).add_source(book());
    run(&mut p, "references.citations-bibliography.style=MLA");
    run(
        &mut p,
        "references.citations-bibliography.insert-citation.Doe2020",
    );
    pane(&mut p).enter(false);
    let before = paragraphs(&mut p);
    run(
        &mut p,
        "references.citations-bibliography.bibliography.works-cited",
    );
    let after = paragraphs(&mut p);
    assert!(after.iter().any(|t| t == "Works Cited"), "{after:?}");
    assert!(
        after
            .iter()
            .any(|t| t.starts_with("Doe, Jane, and Richard Roe. Learning Rust. Acme Press, 2020.")),
        "{after:?}"
    );
    assert!(main_xml(&mut p).contains("BIBLIOGRAPHY"));
    assert!(pane(&mut p).undo());
    assert_eq!(paragraphs(&mut p), before);
    run(&mut p, "references.citations-bibliography.style=APA");
}

#[test]
fn bibliography_without_citations_says_so() {
    let _t = turn();
    let mut p = rig();
    let before = paragraphs(&mut p);
    run(&mut p, "references.citations-bibliography.bibliography");
    assert_eq!(paragraphs(&mut p), before);
    assert!(
        pane(&mut p)
            .status
            .as_deref()
            .unwrap()
            .contains("no source")
    );
}

#[test]
fn cross_reference_points_at_the_caption() {
    let _t = turn();
    let mut p = rig();
    run(&mut p, "references.captions.insert-caption");
    let cap = paragraphs(&mut p)
        .iter()
        .position(|t| t.starts_with("Figure 1"))
        .unwrap();
    let last = paragraphs(&mut p).len() - 2;
    pane(&mut p).set_caret(DocPos::new(last, 15));
    let before = paragraphs(&mut p);
    run(&mut p, "references.captions.cross-reference.figure");
    let after = paragraphs(&mut p);
    assert_eq!(
        after[last],
        format!("{}Figure 1", before[last]),
        "{after:?}"
    );
    let xml = main_xml(&mut p);
    assert!(xml.contains("REF _Ref"), "{xml}");
    assert!(xml.contains("w:bookmarkStart"));
    assert!(xml.contains("\\h"));
    assert_eq!(cap, last + 1);
    assert!(pane(&mut p).undo());
    assert_eq!(paragraphs(&mut p), before);
}

#[test]
fn cross_reference_page_and_entire_caption() {
    let _t = turn();
    let mut p = rig();
    run(&mut p, "references.captions.insert-caption");
    let last = paragraphs(&mut p).len() - 2;
    pane(&mut p).set_caret(DocPos::new(last, 15));
    run(&mut p, "references.captions.cross-reference.figure.page");
    assert!(main_xml(&mut p).contains("PAGEREF _Ref"));
    assert!(paragraphs(&mut p)[last].ends_with('1'));
    let end = paragraphs(&mut p)[last].chars().count();
    pane(&mut p).set_caret(DocPos::new(last, end));
    run(&mut p, "references.captions.cross-reference.figure.entire");
    assert!(paragraphs(&mut p)[last].ends_with("Figure 1"));
}

#[test]
fn cross_reference_without_captions_changes_nothing() {
    let _t = turn();
    let mut p = rig();
    let before = paragraphs(&mut p);
    run(&mut p, "references.captions.cross-reference.table");
    assert_eq!(paragraphs(&mut p), before);
    assert!(
        pane(&mut p)
            .status
            .as_deref()
            .unwrap()
            .contains("Table captions")
    );
}

#[test]
fn manage_sources_reports_the_list() {
    let _t = turn();
    let mut p = rig();
    pane(&mut p).add_source(book());
    run(&mut p, "references.citations-bibliography.manage-sources");
    assert!(
        pane(&mut p)
            .status
            .as_deref()
            .unwrap()
            .starts_with("Sources: ")
    );
}

#[test]
fn change_provider_and_show_notes_stay_unhandled() {
    let mut p = rig();
    for id in [
        "references.citations-bibliography.change-provider",
        "references.footnotes.show-notes",
    ] {
        assert_eq!(p.shell.run_command(&mut p.ui, id), Handled::No, "{id}");
    }
}
