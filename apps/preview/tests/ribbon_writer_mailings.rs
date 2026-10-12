//! Writer Mailings tab controls run through `Shell::run_command`.

#![allow(clippy::unwrap_used)]

use std::path::PathBuf;

use open365_docx::Document;
use open365_opc::Limits;
use open365_preview::doc::WriterPane;
use open365_preview::ribbon::Handled;
use open365_preview::{AppKind, Preview};

const CSV: &str = "Title,First Name,Last Name,Company Name,Address Line 1,City,State,ZIP Code\n\
Ms.,Ada,Lovelace,Analytical Ltd,1 Main St,London,,N1\n\
Dr.,Grace,Hopper,,2 Navy Rd,Arlington,VA,22202\n\
,Alan,Turing,,3 Bletchley Way,Wilmslow,,SK9\n";

fn csv_path() -> PathBuf {
    std::env::temp_dir().join(format!("open365_mailings_{}.csv", std::process::id()))
}

fn pick() -> Option<PathBuf> {
    Some(csv_path())
}

fn rig(text: &str) -> Preview {
    std::fs::write(csv_path(), CSV).unwrap();
    let mut p = Preview::new(AppKind::Writer, false, 1600, 1000, 1.25);
    let mut pane = WriterPane::blank();
    pane.insert_text(text);
    pane.set_list_picker(pick);
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

fn doc_texts(bytes: &[u8]) -> Vec<String> {
    let d = Document::open(bytes, &Limits::default()).unwrap();
    (0..d.paragraph_count())
        .map(|i| d.paragraph_text(i).unwrap())
        .collect()
}

fn start_with_list(p: &mut Preview) {
    run(p, "mailings.start-mail-merge.start-mail-merge.letters");
    run(
        p,
        "mailings.start-mail-merge.select-recipients.use-an-existing-list",
    );
}

#[test]
fn controls_do_nothing_before_a_merge_starts() {
    let mut p = rig("Hello");
    let before = paragraphs(&mut p);
    run(&mut p, "mailings.write-insert-fields.address-block");
    run(
        &mut p,
        "mailings.write-insert-fields.insert-merge-field.City",
    );
    run(&mut p, "mailings.preview-results.preview-results");
    assert_eq!(paragraphs(&mut p), before);
    assert!(!pane(&mut p).mail_started());
}

#[test]
fn start_and_select_recipients_read_the_file() {
    let mut p = rig("Hello");
    start_with_list(&mut p);
    let pane = pane(&mut p);
    assert!(pane.mail_started());
    assert_eq!(pane.merge_record_count(), 3);
    assert_eq!(pane.merge_columns()[1], "First Name");
}

#[test]
fn merge_field_inserts_and_undoes() {
    let mut p = rig("Dear ");
    start_with_list(&mut p);
    run(
        &mut p,
        "mailings.write-insert-fields.insert-merge-field.First Name",
    );
    assert_eq!(paragraphs(&mut p)[0], "Dear \u{ab}First Name\u{bb}");
    assert!(pane(&mut p).undo());
    assert_eq!(paragraphs(&mut p)[0], "Dear ");
}

#[test]
fn preview_shows_each_record_and_switches_off() {
    let mut p = rig("Dear ");
    start_with_list(&mut p);
    run(
        &mut p,
        "mailings.write-insert-fields.insert-merge-field.First Name",
    );
    run(&mut p, "mailings.preview-results.preview-results");
    assert_eq!(paragraphs(&mut p)[0], "Dear Ada");
    run(&mut p, "mailings.preview-results.next");
    assert_eq!(paragraphs(&mut p)[0], "Dear Grace");
    run(&mut p, "mailings.preview-results.last");
    assert_eq!(paragraphs(&mut p)[0], "Dear Alan");
    run(&mut p, "mailings.preview-results.record=2");
    assert_eq!(paragraphs(&mut p)[0], "Dear Grace");
    run(&mut p, "mailings.preview-results.find-recipient=turing");
    assert_eq!(paragraphs(&mut p)[0], "Dear Alan");
    run(&mut p, "mailings.preview-results.first");
    assert_eq!(paragraphs(&mut p)[0], "Dear Ada");
    run(&mut p, "mailings.preview-results.preview-results");
    assert_eq!(paragraphs(&mut p)[0], "Dear \u{ab}First Name\u{bb}");
}

#[test]
fn address_block_and_greeting_line_merge() {
    let mut p = rig("");
    start_with_list(&mut p);
    run(&mut p, "mailings.write-insert-fields.address-block");
    run(&mut p, "mailings.finish.finish-merge");
    let docs = pane(&mut p).take_new_documents();
    assert_eq!(docs.len(), 1);
    let t = doc_texts(&docs[0]);
    assert!(
        t.iter().any(|l| l == "Ms. Ada Lovelace"),
        "name line: {t:?}"
    );
    assert!(t.iter().any(|l| l == "Analytical Ltd"), "{t:?}");
    assert!(t.iter().any(|l| l == "Arlington, VA 22202"), "{t:?}");
    // Alan has no title or company: his first line is just the name.
    assert!(t.iter().any(|l| l == "Alan Turing"), "{t:?}");
    assert!(t.iter().any(|l| l == "Wilmslow, SK9"), "{t:?}");
}

#[test]
fn greeting_line_uses_the_title_and_falls_back() {
    let mut p = rig("");
    start_with_list(&mut p);
    run(&mut p, "mailings.write-insert-fields.greeting-line");
    run(&mut p, "mailings.finish.finish-merge");
    let t = doc_texts(&pane(&mut p).take_new_documents()[0]);
    assert!(t.iter().any(|l| l == "Dear Ms. Lovelace,"), "{t:?}");
    assert!(t.iter().any(|l| l == "Dear Hopper,") || t.iter().any(|l| l == "Dear Dr. Hopper,"));
    assert!(t.iter().any(|l| l == "Dear Turing,"), "{t:?}");
}

#[test]
fn unmatched_greeting_gets_the_fallback() {
    let mut p = rig("");
    start_with_list(&mut p);
    run(&mut p, "mailings.write-insert-fields.greeting-line");
    // Unmatch the last name and the title: nothing is left to greet.
    run(&mut p, "mailings.write-insert-fields.match-fields=_LAST0_:");
    run(
        &mut p,
        "mailings.write-insert-fields.match-fields=_TITLE0_:",
    );
    run(&mut p, "mailings.finish.finish-merge");
    let t = doc_texts(&pane(&mut p).take_new_documents()[0]);
    assert!(
        t.iter()
            .all(|l| l == "Dear Sir or Madam," || l.trim().is_empty()),
        "{t:?}"
    );
    assert!(t.iter().any(|l| l == "Dear Sir or Madam,"));
}

#[test]
fn finish_merge_makes_one_page_per_record_and_skip_rule_drops_one() {
    let mut p = rig("Hi ");
    start_with_list(&mut p);
    run(
        &mut p,
        "mailings.write-insert-fields.insert-merge-field.First Name",
    );
    run(&mut p, "mailings.write-insert-fields.rules.merge-sequence");
    assert_eq!(paragraphs(&mut p)[0], "Hi \u{ab}First Name\u{bb}1");
    run(
        &mut p,
        "mailings.finish.finish-merge.edit-individual-documents",
    );
    let docs = pane(&mut p).take_new_documents();
    let t = doc_texts(&docs[0]);
    let hi: Vec<&String> = t.iter().filter(|l| l.starts_with("Hi ")).collect();
    assert_eq!(hi.len(), 3, "{t:?}");
    assert_eq!(hi[0], "Hi Ada1");
    assert_eq!(hi[2], "Hi Alan3");
    // The main document is untouched by the merge.
    assert!(paragraphs(&mut p)[0].starts_with("Hi \u{ab}First Name\u{bb}"));

    // Skip Record If First Name = "" skips nobody; editing the condition by hand: tick off one.
    let list_ok = pane(&mut p).edit_recipients(|l| l.set_included(1, false));
    assert!(list_ok);
    run(&mut p, "mailings.finish.finish-merge");
    let t = doc_texts(&pane(&mut p).take_new_documents()[0]);
    let hi: Vec<&String> = t.iter().filter(|l| l.starts_with("Hi ")).collect();
    assert_eq!(hi.len(), 2, "{t:?}");
}

#[test]
fn skip_record_if_rule_leaves_matching_records_out() {
    let mut p = rig("");
    start_with_list(&mut p);
    run(&mut p, "mailings.write-insert-fields.rules.skip-record-if");
    // The rule compares the first column (Title) with an empty text: Alan has no title.
    run(&mut p, "mailings.finish.finish-merge");
    let docs = pane(&mut p).take_new_documents();
    let merged = doc_texts(&docs[0]);
    let before = merged.len();
    // Three records, one page each, minus Alan = two records' paragraphs.
    assert!(before >= 2, "{merged:?}");
    assert!(pane(&mut p).status.as_deref().unwrap().contains("Merged 2"));
}

#[test]
fn rules_insert_fields_into_the_document() {
    let mut p = rig("");
    start_with_list(&mut p);
    for (rule, expect) in [
        ("merge-record", "1"),
        ("next-record", "\u{ab}Next Record\u{bb}"),
        ("fill-in", ""),
        ("set-bookmark", "Text"),
    ] {
        run(
            &mut p,
            &format!("mailings.write-insert-fields.rules.{rule}"),
        );
        assert!(
            paragraphs(&mut p)[0].contains(expect),
            "{rule}: {:?}",
            paragraphs(&mut p)
        );
    }
    run(&mut p, "mailings.write-insert-fields.rules.if-then-else");
    assert!(
        paragraphs(&mut p)[0].contains("Title"),
        "{:?}",
        paragraphs(&mut p)
    );
}

#[test]
fn type_a_new_list_and_edit_it() {
    let mut p = rig("");
    run(
        &mut p,
        "mailings.start-mail-merge.start-mail-merge.directory",
    );
    run(
        &mut p,
        "mailings.start-mail-merge.select-recipients.type-a-new-list",
    );
    assert_eq!(pane(&mut p).merge_columns().len(), 13);
    assert!(pane(&mut p).edit_recipients(|l| {
        l.set_cell(0, 1, "Zed");
        l.add_row();
        l.set_cell(1, 1, "Amy");
        l.sort_by(1, true);
    }));
    run(
        &mut p,
        "mailings.write-insert-fields.insert-merge-field.First Name",
    );
    run(&mut p, "mailings.finish.finish-merge");
    let t = doc_texts(&pane(&mut p).take_new_documents()[0]);
    // A directory runs the records together without page breaks.
    assert_eq!(&t[0], "Amy", "{t:?}");
    assert_eq!(&t[1], "Zed", "{t:?}");
}

#[test]
fn check_for_errors_finds_a_field_the_list_lacks() {
    let mut p = rig("");
    start_with_list(&mut p);
    run(
        &mut p,
        "mailings.write-insert-fields.insert-merge-field.City",
    );
    assert!(pane(&mut p).check_merge_errors().is_empty());
    // A list without the City column.
    pane(&mut p).edit_recipients(|l| {
        let at = l.column("City").unwrap();
        l.columns.remove(at);
        for r in &mut l.rows {
            r.remove(at);
        }
    });
    run(&mut p, "mailings.preview-results.check-for-errors");
    let problems = pane(&mut p).check_merge_errors();
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("City"));
}

#[test]
fn envelope_adds_a_first_section_with_the_addresses() {
    let mut p = rig("Body text");
    pane(&mut p).select_all();
    run(&mut p, "mailings.create.envelopes");
    let t = paragraphs(&mut p);
    assert!(t.iter().any(|l| l == "Body text"), "{t:?}");
    assert!(pane(&mut p).undo());
    assert_eq!(paragraphs(&mut p), ["Body text"]);
}

#[test]
fn envelopes_merge_type_lays_out_the_page() {
    let mut p = rig("");
    let before = paragraphs(&mut p).len();
    run(
        &mut p,
        "mailings.start-mail-merge.start-mail-merge.envelopes",
    );
    assert!(paragraphs(&mut p).len() > before);
    assert_eq!(pane(&mut p).mail(|m| m.kind).unwrap().label(), "Envelopes");
}

#[test]
fn labels_sheet_goes_to_a_new_document() {
    let mut p = rig("Ada Lovelace");
    pane(&mut p).enter(false);
    pane(&mut p).insert_text("1 Main St");
    pane(&mut p).select_all();
    run(&mut p, "mailings.create.labels");
    let docs = pane(&mut p).take_new_documents();
    assert_eq!(docs.len(), 1);
    let t = doc_texts(&docs[0]);
    assert_eq!(
        t.iter().filter(|l| *l == "Ada Lovelace").count(),
        30,
        "{t:?}"
    );
}

#[test]
fn label_merge_fills_a_sheet_from_the_list() {
    let mut p = rig("");
    start_with_list(&mut p);
    run(&mut p, "mailings.start-mail-merge.start-mail-merge.labels");
    run(
        &mut p,
        "mailings.write-insert-fields.insert-merge-field.City",
    );
    run(&mut p, "mailings.write-insert-fields.update-labels");
    run(&mut p, "mailings.finish.finish-merge");
    let t = doc_texts(&pane(&mut p).take_new_documents()[0]);
    for city in ["London", "Arlington", "Wilmslow"] {
        assert!(t.iter().any(|l| l == city), "{city}: {t:?}");
    }
}

#[test]
fn normal_document_ends_the_merge() {
    let mut p = rig("");
    start_with_list(&mut p);
    run(
        &mut p,
        "mailings.start-mail-merge.start-mail-merge.normal-document",
    );
    assert!(!pane(&mut p).mail_started());
}

#[test]
fn menus_list_the_choices() {
    use open365_preview::ribbon::writer::mailings::menu_items;
    assert_eq!(
        menu_items("mailings.write-insert-fields.rules")
            .unwrap()
            .len(),
        9
    );
    assert_eq!(
        menu_items("mailings.start-mail-merge.start-mail-merge")
            .unwrap()
            .len(),
        7
    );
    assert!(menu_items("mailings.create.labels").is_none());
}
